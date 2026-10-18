use notes_core::db::{self, CreateNoteResult};
use notes_core::store::{CommitNotice, NotesStore};
use notes_core::{
    Connection, CoreError, Hlc, Op, OpKind, Origin, PageAlias, PageAliasSet, PageKind,
};
use std::sync::{Arc, Mutex};

struct Fixture {
    _directory: tempfile::TempDir,
    connection: Connection,
    store: NotesStore,
    notices: Arc<Mutex<Vec<CommitNotice>>>,
}

async fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(directory.path().join("notes.db")).await.unwrap();
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
        store,
        notices,
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
async fn empty_named_titles_are_rejected_without_writes_or_notices() {
    let fixture = fixture().await;
    for error in [
        fixture.store.create_page("  ".into()).await.unwrap_err(),
        fixture
            .store
            .get_or_create_page_by_title("\t\n".into())
            .await
            .unwrap_err(),
    ] {
        assert!(matches!(
            error.downcast_ref::<CoreError>(),
            Some(CoreError::InvalidInput(_))
        ));
    }
    assert_eq!(counters(&fixture.connection).await, (0, 0, 0, 0, 0, 0));
    assert!(fixture.notices.lock().unwrap().is_empty());
}

#[tokio::test]
async fn concurrent_named_creation_returns_one_page_and_publishes_one_commit() {
    let fixture = fixture().await;
    let (first, second, third) = tokio::join!(
        fixture.store.create_page(" Named page ".into()),
        fixture
            .store
            .get_or_create_page_by_title("NAMED PAGE".into()),
        fixture.store.create_page("named page".into()),
    );
    let first = first.unwrap();
    let second = second.unwrap();
    let third = third.unwrap();
    assert_eq!(first.value, second.value);
    assert_eq!(first.value, third.value);
    let committed = [&first, &second, &third]
        .into_iter()
        .filter(|result| result.commit_version.is_some())
        .collect::<Vec<_>>();
    assert_eq!(committed.len(), 1);
    let committed = committed[0];
    assert_eq!(committed.changes.changed_pages, vec![first.value.uuid]);
    assert!(committed.changes.history_changed);
    assert!(committed.changes.blocks.is_empty());
    assert_eq!(fixture.notices.lock().unwrap().len(), 1);
    assert_eq!(counters(&fixture.connection).await, (1, 0, 1, 1, 1, 1));
}

#[tokio::test]
async fn concurrent_named_notes_create_one_initial_block_and_one_history_action() {
    let fixture = fixture().await;
    let (first, second) = tokio::join!(
        fixture.store.create_note(Some(" Named note ".into())),
        fixture.store.create_note(Some("NAMED NOTE".into())),
    );
    let (created, existing) = match (first.unwrap(), second.unwrap()) {
        (created, existing) if matches!(created.value, CreateNoteResult::Created { .. }) => {
            (created, existing)
        }
        (existing, created) => (created, existing),
    };
    let CreateNoteResult::Created { note } = created.value else {
        panic!("one note must be created")
    };
    let CreateNoteResult::Existing { page } = existing.value else {
        panic!("the other call must return the same note")
    };
    assert_eq!(note.page, page);
    assert_eq!(note.initial_block.page_uuid, page.uuid);
    assert!(note.initial_block.markdown.is_empty());
    assert_eq!(created.changes.changed_pages, vec![page.uuid]);
    assert_eq!(created.changes.blocks.len(), 1);
    assert_eq!(created.changes.blocks[0].uuid, note.initial_block.uuid);
    assert!(created.changes.blocks[0].before.is_none());
    assert!(created.changes.history_changed);
    assert!(existing.changes.is_empty());
    assert_eq!(existing.commit_version, None);
    assert_eq!(fixture.notices.lock().unwrap().len(), 1);
    assert_eq!(counters(&fixture.connection).await, (1, 1, 2, 2, 1, 1));
}

#[tokio::test]
async fn untitled_notes_are_distinct_and_each_undo_removes_its_page_and_initial_block() {
    let fixture = fixture().await;
    let first = fixture
        .store
        .create_note(None)
        .await
        .unwrap()
        .value
        .into_created()
        .unwrap();
    let second = fixture
        .store
        .create_note(Some("   ".into()))
        .await
        .unwrap()
        .value
        .into_created()
        .unwrap();
    assert_ne!(first.page.uuid, second.page.uuid);
    assert_ne!(first.initial_block.uuid, second.initial_block.uuid);
    assert!(first.page.title.is_none());
    assert!(second.page.title.is_none());
    assert_eq!(counters(&fixture.connection).await, (2, 2, 4, 4, 2, 2));

    for note in [second, first] {
        let undone = fixture.store.undo().await.unwrap();
        assert_eq!(undone.changes.deleted_pages, vec![note.page.uuid]);
        assert!(
            undone
                .changes
                .blocks
                .iter()
                .any(|block| block.uuid == note.initial_block.uuid && block.after.is_none())
        );
        assert!(
            db::get_page(&fixture.connection, note.page.uuid)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            db::get_block(&fixture.connection, note.initial_block.uuid)
                .await
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test]
async fn handwritten_creation_keeps_primary_identity_without_a_text_block_or_text_history() {
    let fixture = fixture().await;
    let created = fixture
        .store
        .create_handwritten_note(Some("  Handwritten  ".into()))
        .await
        .unwrap();
    assert_eq!(created.value.kind, PageKind::Handwriting);
    assert_eq!(created.value.title.as_deref(), Some("Handwritten"));
    assert_eq!(created.changes.changed_pages, vec![created.value.uuid]);
    assert!(created.changes.blocks.is_empty());
    assert!(!created.changes.history_changed);
    assert_eq!(created.commit_version, Some(1));
    assert_eq!(
        db::get_page(&fixture.connection, created.value.uuid)
            .await
            .unwrap(),
        Some(created.value)
    );
    assert_eq!(counters(&fixture.connection).await, (1, 0, 1, 1, 0, 1));
    assert_eq!(fixture.notices.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn every_named_creation_entry_resolves_an_explicit_alias_without_writes() {
    let fixture = fixture().await;
    let owner = fixture
        .store
        .create_page("Current title".into())
        .await
        .unwrap()
        .value;
    let workspace_uuid = db::workspace_uuid(&fixture.connection).await.unwrap();
    let device_id = uuid::Uuid::from_u128(0xa11a5);
    notes_core::apply(
        &fixture.connection,
        &Op {
            op_id: uuid::Uuid::now_v7(),
            workspace_uuid,
            device_id,
            hlc: Hlc::new(1, 0, device_id),
            format_version: notes_core::operation::FORMAT_VERSION,
            kind: OpKind::PageAliasSet(PageAliasSet {
                uuid: owner.uuid,
                alias: PageAlias::new("Former title").unwrap(),
                present: true,
            }),
        },
        Origin::Remote,
    )
    .await
    .unwrap();
    fixture.notices.lock().unwrap().clear();
    let before = counters(&fixture.connection).await;
    let page = fixture
        .store
        .create_page(" FORMER TITLE ".into())
        .await
        .unwrap();
    let found = fixture
        .store
        .get_or_create_page_by_title("former title".into())
        .await
        .unwrap();
    let note = fixture
        .store
        .create_note(Some("Former Title".into()))
        .await
        .unwrap();
    assert_eq!(page.value, owner);
    assert_eq!(found.value, owner);
    assert!(matches!(note.value, CreateNoteResult::Existing { page } if page == owner));
    assert!(page.changes.is_empty() && found.changes.is_empty() && note.changes.is_empty());
    assert!(
        page.commit_version.is_none()
            && found.commit_version.is_none()
            && note.commit_version.is_none()
    );
    assert_eq!(counters(&fixture.connection).await, before);
    assert!(fixture.notices.lock().unwrap().is_empty());
}

#[tokio::test]
async fn failing_creation_rolls_back_the_page_block_history_outbox_and_notice() {
    for kind in ["page", "note", "handwriting"] {
        let fixture = fixture().await;
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
                    "CREATE TEMP TRIGGER reject_creation BEFORE UPDATE OF version ON store_state
             BEGIN SELECT RAISE(ABORT, 'injected creation failure'); END;",
                )
            })
            .await
            .unwrap();
        let error = match kind {
            "page" => fixture
                .store
                .create_page("Rejected".into())
                .await
                .unwrap_err(),
            "note" => fixture
                .store
                .create_note(Some("Rejected".into()))
                .await
                .unwrap_err(),
            _ => fixture
                .store
                .create_handwritten_note(Some("Rejected".into()))
                .await
                .unwrap_err(),
        };
        assert!(error.to_string().contains("injected creation failure"));
        assert_eq!(
            serde_json::to_value(
                notes_core::export_sync_snapshot(&fixture.connection, 0)
                    .await
                    .unwrap()
            )
            .unwrap(),
            before
        );
        assert_eq!(counters(&fixture.connection).await, (0, 0, 0, 0, 0, 0));
        assert!(fixture.notices.lock().unwrap().is_empty());
        assert!(
            db::get_page_by_title(&fixture.connection, "Rejected".into())
                .await
                .unwrap()
                .is_none()
        );
    }
}
