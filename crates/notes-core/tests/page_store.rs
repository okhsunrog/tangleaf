use notes_core::db::{self, Page};
use notes_core::store::{CommitNotice, NotesStore};
use notes_core::{BlockStyle, Connection, CoreError, PageLayout};
use std::sync::{Arc, Mutex};

struct Fixture {
    _directory: tempfile::TempDir,
    connection: Connection,
    page: Page,
    store: NotesStore,
    notices: Arc<Mutex<Vec<CommitNotice>>>,
}

async fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(directory.path().join("notes.db")).await.unwrap();
    let page = db::create_page(&connection, "Original".into())
        .await
        .unwrap();
    notes_core::configure_sync(&connection, "https://sync.example")
        .await
        .unwrap();
    let notices = Arc::new(Mutex::new(Vec::new()));
    let observed = notices.clone();
    let store = NotesStore::new(connection.clone()).with_commit_observer(move |notice| {
        observed.lock().unwrap().push(notice.clone());
    });
    Fixture {
        _directory: directory,
        connection,
        page,
        store,
        notices,
    }
}

async fn counters(connection: &Connection) -> (i64, i64, i64, i64, i64) {
    connection
        .call(|database| {
            database.query_row(
                "SELECT (SELECT COUNT(*) FROM applied_ops),
                (SELECT COUNT(*) FROM sync_outbox),
                (SELECT COUNT(*) FROM history_undo),
                (SELECT COUNT(*) FROM history_redo),
                (SELECT version FROM store_state WHERE singleton = 1)",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn rename_and_layout_publish_exact_commits_without_changing_history_policy() {
    let fixture = fixture().await;
    db::create_block(
        &fixture.connection,
        fixture.page.uuid,
        None,
        None,
        BlockStyle::Paragraph,
        "Temporary".into(),
    )
    .await
    .unwrap();
    fixture.store.undo().await.unwrap();
    fixture.notices.lock().unwrap().clear();
    let before = counters(&fixture.connection).await;
    assert!(before.3 > 0, "fixture retains an existing redo entry");

    let rename = fixture
        .store
        .rename_page_if_revision(
            fixture.page.uuid,
            Some("  Renamed  ".into()),
            fixture.page.title_revision.clone(),
        )
        .await
        .unwrap();
    assert_eq!(rename.value.title.as_deref(), Some("Renamed"));
    assert_eq!(rename.changes.changed_pages, vec![fixture.page.uuid]);
    assert_eq!(rename.changes.affected_pages, vec![fixture.page.uuid]);
    assert_eq!(
        rename.changes.graph_changed_content,
        vec![fixture.page.uuid]
    );
    assert!(!rename.changes.history_changed);
    assert!(rename.changes.blocks.is_empty());
    assert_eq!(rename.commit_version, Some(before.4 + 1));

    let layout = fixture
        .store
        .set_page_layout(fixture.page.uuid, PageLayout::Document)
        .await
        .unwrap();
    assert_eq!(layout.value.layout, PageLayout::Document);
    assert_eq!(layout.value.title_revision, rename.value.title_revision);
    assert_eq!(layout.changes.changed_pages, vec![fixture.page.uuid]);
    assert_eq!(layout.changes.affected_pages, vec![fixture.page.uuid]);
    assert!(layout.changes.graph_changed_content.is_empty());
    assert!(!layout.changes.history_changed);
    assert_eq!(layout.commit_version, Some(before.4 + 2));
    let after = counters(&fixture.connection).await;
    assert_eq!((after.0, after.1), (before.0 + 2, before.1 + 2));
    assert_eq!((after.2, after.3), (before.2, before.3));
    let notices = fixture.notices.lock().unwrap();
    assert_eq!(notices.len(), 2);
    assert_eq!(notices[0].changes, rename.changes);
    assert_eq!(notices[1].changes, layout.changes);
}

#[tokio::test]
async fn equal_title_and_layout_are_quiet_even_with_an_old_title_revision() {
    let fixture = fixture().await;
    let renamed = fixture
        .store
        .rename_page_if_revision(
            fixture.page.uuid,
            Some("Updated".into()),
            fixture.page.title_revision.clone(),
        )
        .await
        .unwrap();
    fixture.notices.lock().unwrap().clear();
    let before = counters(&fixture.connection).await;

    let equal = fixture
        .store
        .rename_page_if_revision(
            fixture.page.uuid,
            Some(" Updated ".into()),
            fixture.page.title_revision,
        )
        .await
        .unwrap();
    assert_eq!(equal.value, renamed.value);
    assert!(equal.changes.is_empty());
    assert_eq!(equal.commit_version, None);
    let layout = fixture
        .store
        .set_page_layout(fixture.page.uuid, renamed.value.layout)
        .await
        .unwrap();
    assert_eq!(layout.value, renamed.value);
    assert!(layout.changes.is_empty());
    assert_eq!(layout.commit_version, None);
    assert_eq!(counters(&fixture.connection).await, before);
    assert!(fixture.notices.lock().unwrap().is_empty());
}

#[tokio::test]
async fn competing_title_edits_compare_against_the_transaction_snapshot() {
    let fixture = fixture().await;
    let before = counters(&fixture.connection).await;
    let (first, second) = tokio::join!(
        fixture.store.rename_page_if_revision(
            fixture.page.uuid,
            Some("First".into()),
            fixture.page.title_revision.clone()
        ),
        fixture.store.rename_page_if_revision(
            fixture.page.uuid,
            Some("Second".into()),
            fixture.page.title_revision
        ),
    );
    let (winner, rejected) = match (first, second) {
        (Ok(winner), Err(rejected)) | (Err(rejected), Ok(winner)) => (winner, rejected),
        outcomes => panic!("expected one committed edit and one conflict, got {outcomes:?}"),
    };
    assert!(matches!(
        rejected.downcast_ref::<CoreError>(),
        Some(CoreError::Conflict(_))
    ));
    assert_eq!(
        db::get_page(&fixture.connection, fixture.page.uuid)
            .await
            .unwrap(),
        Some(winner.value)
    );
    let after = counters(&fixture.connection).await;
    assert_eq!(
        (after.0, after.1, after.4),
        (before.0 + 1, before.1 + 1, before.4 + 1)
    );
    assert_eq!((after.2, after.3), (before.2, before.3));
    assert_eq!(fixture.notices.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn commit_failure_rolls_back_title_layout_aliases_operations_and_notifications() {
    for rename in [true, false] {
        let fixture = fixture().await;
        let before = serde_json::to_value(
            notes_core::export_sync_snapshot(&fixture.connection, 0)
                .await
                .unwrap(),
        )
        .unwrap();
        let before_counts = counters(&fixture.connection).await;
        fixture
            .connection
            .call(|database| {
                database.execute_batch(
                    "CREATE TEMP TRIGGER reject_page_commit BEFORE UPDATE OF version ON store_state
             BEGIN SELECT RAISE(ABORT, 'injected commit failure'); END;",
                )
            })
            .await
            .unwrap();
        let result = if rename {
            fixture
                .store
                .rename_page_if_revision(
                    fixture.page.uuid,
                    Some("Rejected".into()),
                    fixture.page.title_revision.clone(),
                )
                .await
        } else {
            fixture
                .store
                .set_page_layout(fixture.page.uuid, PageLayout::Document)
                .await
        };
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("injected commit failure")
        );
        let after = serde_json::to_value(
            notes_core::export_sync_snapshot(&fixture.connection, 0)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(after, before);
        assert_eq!(counters(&fixture.connection).await, before_counts);
        assert_eq!(
            db::get_page(&fixture.connection, fixture.page.uuid)
                .await
                .unwrap(),
            Some(fixture.page.clone())
        );
        assert_eq!(
            db::get_page_by_title(&fixture.connection, "Original".into())
                .await
                .unwrap(),
            Some(fixture.page)
        );
        assert!(
            db::get_page_by_title(&fixture.connection, "Rejected".into())
                .await
                .unwrap()
                .is_none()
        );
        assert!(fixture.notices.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn title_conflicts_missing_pages_and_journal_titles_remain_typed_and_quiet() {
    let fixture = fixture().await;
    db::create_page(&fixture.connection, "Reserved".into())
        .await
        .unwrap();
    let journal = db::ensure_journal(&fixture.connection, "2026-10-18".parse().unwrap())
        .await
        .unwrap();
    let before = counters(&fixture.connection).await;
    let collision = fixture
        .store
        .rename_page_if_revision(
            fixture.page.uuid,
            Some("Reserved".into()),
            fixture.page.title_revision.clone(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        collision.downcast_ref::<CoreError>(),
        Some(CoreError::Conflict(_))
    ));
    let invalid = fixture
        .store
        .rename_page_if_revision(journal.uuid, journal.title.clone(), journal.title_revision)
        .await
        .unwrap_err();
    assert!(matches!(
        invalid.downcast_ref::<CoreError>(),
        Some(CoreError::InvalidInput(_))
    ));
    let missing = fixture
        .store
        .set_page_layout(uuid::Uuid::now_v7(), PageLayout::Document)
        .await
        .unwrap_err();
    assert!(matches!(
        missing.downcast_ref::<CoreError>(),
        Some(CoreError::NotFound(_))
    ));
    assert_eq!(counters(&fixture.connection).await, before);
    assert!(fixture.notices.lock().unwrap().is_empty());
}
