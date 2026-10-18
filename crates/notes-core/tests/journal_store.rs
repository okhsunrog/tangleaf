use notes_core::db::{self, BlockContent};
use notes_core::store::{CommitNotice, NotesStore};
use notes_core::{BlockStyle, Connection, CoreError, JournalDate, journal_page_uuid};
use std::sync::{Arc, Mutex};

struct Fixture {
    _directory: tempfile::TempDir,
    connection: Connection,
    store: NotesStore,
    notices: Arc<Mutex<Vec<CommitNotice>>>,
    initial_version: i64,
}

async fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(directory.path().join("notes.db")).await.unwrap();
    notes_core::configure_sync(&connection, "https://sync.example")
        .await
        .unwrap();
    let initial_version = counters(&connection).await.5;
    let notices = Arc::new(Mutex::new(Vec::new()));
    let observed = notices.clone();
    let store = NotesStore::new(connection.clone()).with_commit_observer(move |notice| {
        observed.lock().unwrap().push(notice.clone());
    });
    Fixture {
        _directory: directory,
        connection,
        store,
        notices,
        initial_version,
    }
}

fn date() -> JournalDate {
    "2026-10-18".parse().unwrap()
}
fn content(markdown: &str) -> BlockContent {
    BlockContent {
        markdown: markdown.into(),
    }
}

async fn counters(connection: &Connection) -> (i64, i64, i64, i64, i64, i64) {
    connection
        .call(|database| {
            database.query_row(
                "SELECT (SELECT COUNT(*) FROM pages), (SELECT COUNT(*) FROM blocks),
                (SELECT COUNT(*) FROM applied_ops), (SELECT COUNT(*) FROM sync_outbox),
                (SELECT COUNT(*) FROM history_undo),
                (SELECT version FROM store_state WHERE singleton = 1)",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn preview_and_blank_capture_do_not_persist_a_day_but_explicit_ensure_does() {
    let fixture = fixture().await;
    assert!(
        db::get_journal(&fixture.connection, date())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db::list_journals(&fixture.connection, None, Default::default())
            .await
            .unwrap()
            .is_empty()
    );
    let error = fixture
        .store
        .append_to_journal(date(), content(" \t\n"), BlockStyle::Paragraph)
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<CoreError>(),
        Some(CoreError::InvalidInput(_))
    ));
    assert_eq!(
        counters(&fixture.connection).await,
        (0, 0, 0, 0, 0, fixture.initial_version)
    );
    assert!(fixture.notices.lock().unwrap().is_empty());

    let ensured = fixture.store.ensure_journal(date()).await.unwrap();
    let expected_uuid = journal_page_uuid(
        db::workspace_uuid(&fixture.connection).await.unwrap(),
        &date(),
    );
    assert_eq!(ensured.value.uuid, expected_uuid);
    assert!(ensured.value.title.is_none());
    assert_eq!(ensured.changes.changed_pages, vec![expected_uuid]);
    assert!(ensured.changes.blocks.is_empty());
    assert!(ensured.changes.history_changed);
    assert_eq!(
        counters(&fixture.connection).await,
        (1, 0, 1, 1, 1, fixture.initial_version + 1)
    );
    let again = fixture.store.ensure_journal(date()).await.unwrap();
    assert_eq!(again.value, ensured.value);
    assert!(again.changes.is_empty());
    assert_eq!(again.commit_version, None);
    assert_eq!(fixture.notices.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn concurrent_ensure_calls_publish_only_one_journal_creation() {
    let fixture = fixture().await;
    let (first, second) = tokio::join!(
        fixture.store.ensure_journal(date()),
        fixture.store.ensure_journal(date())
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(first.value, second.value);
    assert_eq!(
        usize::from(first.commit_version.is_some()) + usize::from(second.commit_version.is_some()),
        1
    );
    assert_eq!(
        counters(&fixture.connection).await,
        (1, 0, 1, 1, 1, fixture.initial_version + 1)
    );
    assert_eq!(fixture.notices.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn concurrent_capture_uses_one_page_ordered_roots_and_one_history_action_per_entry() {
    let fixture = fixture().await;
    let (first, second) = tokio::join!(
        fixture
            .store
            .append_to_journal(date(), content("First entry"), BlockStyle::Paragraph),
        fixture
            .store
            .append_to_journal(date(), content("Second entry"), BlockStyle::Bullet),
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert_ne!(first.value.uuid, second.value.uuid);
    assert_eq!(first.value.page_uuid, second.value.page_uuid);
    assert_ne!(first.value.order_key, second.value.order_key);
    let mut entries = vec![first.value.clone(), second.value.clone()];
    entries.sort_by(|left, right| left.order_key.cmp(&right.order_key));
    assert_eq!(
        db::list_block_children(&fixture.connection, first.value.page_uuid, None)
            .await
            .unwrap(),
        entries
    );
    assert_eq!(
        counters(&fixture.connection).await,
        (1, 2, 3, 3, 2, fixture.initial_version + 2)
    );
    assert!(first.changes.history_changed && second.changes.history_changed);
    assert_eq!(first.changes.blocks.len(), 1);
    assert_eq!(second.changes.blocks.len(), 1);
    assert_eq!(
        first.changes.changed_pages.len() + second.changes.changed_pages.len(),
        1
    );
    assert_eq!(fixture.notices.lock().unwrap().len(), 2);

    let last = entries.pop().unwrap();
    let undone = fixture.store.undo().await.unwrap();
    assert!(undone.changes.deleted_pages.is_empty());
    assert!(
        db::get_block(&fixture.connection, last.uuid)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db::get_journal(&fixture.connection, date())
            .await
            .unwrap()
            .is_some()
    );
    fixture.store.undo().await.unwrap();
    assert!(
        db::get_journal(&fixture.connection, date())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn failures_roll_back_ensure_initial_capture_and_existing_capture_before_notice() {
    for operation in ["ensure", "initial capture", "existing capture"] {
        let fixture = fixture().await;
        if operation == "existing capture" {
            fixture
                .store
                .append_to_journal(date(), content("Keep this"), BlockStyle::Paragraph)
                .await
                .unwrap();
        }
        fixture.notices.lock().unwrap().clear();
        let before_counts = counters(&fixture.connection).await;
        let before = serde_json::to_value(
            notes_core::export_sync_snapshot(&fixture.connection, 0)
                .await
                .unwrap(),
        )
        .unwrap();
        fixture
            .connection
            .call(|database| {
                database.execute_batch(
            "CREATE TEMP TRIGGER reject_journal_commit BEFORE UPDATE OF version ON store_state
             BEGIN SELECT RAISE(ABORT, 'injected journal failure'); END;",
        )
            })
            .await
            .unwrap();
        let error = if operation == "ensure" {
            fixture.store.ensure_journal(date()).await.unwrap_err()
        } else {
            fixture
                .store
                .append_to_journal(date(), content("Reject this"), BlockStyle::Paragraph)
                .await
                .unwrap_err()
        };
        assert!(error.to_string().contains("injected journal failure"));
        assert_eq!(counters(&fixture.connection).await, before_counts);
        assert_eq!(
            serde_json::to_value(
                notes_core::export_sync_snapshot(&fixture.connection, 0)
                    .await
                    .unwrap()
            )
            .unwrap(),
            before
        );
        assert!(fixture.notices.lock().unwrap().is_empty());
    }
}
