use notes_core::db::{self, DataArchive};
use notes_core::ink::{InkBackground, InkDraftPatch, InkPoint, InkStroke, Store as InkStore};
use notes_core::store::{CommitNotice, NotesStore};
use notes_core::{Connection, ReplicaRole};
use rusqlite::types::Value;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
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

async fn revision(connection: &Connection) -> (i64, i64) {
    connection
        .call(|database| {
            database.query_row(
                "SELECT version, workspace_epoch FROM store_state WHERE singleton=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
        })
        .await
        .unwrap()
}

async fn pins(connection: &Connection) -> i64 {
    connection
        .call(|database| {
            database.query_row("SELECT COUNT(*) FROM history_ink_snapshots", [], |row| {
                row.get(0)
            })
        })
        .await
        .unwrap()
}

async fn raw_state(connection: &Connection) -> BTreeMap<String, Vec<Vec<Value>>> {
    connection.call(|database| {
        let tx = database.transaction()?;
        let tables = tx.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")?
            .query_map([], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut state = BTreeMap::new();
        for table in tables {
            let sql = format!("SELECT * FROM \"{}\"", table.replace('"', "\"\""));
            let mut statement = tx.prepare(&sql)?;
            let columns = statement.column_count();
            let mut rows = statement.query_map([], |row| (0..columns).map(|column| row.get::<_, Value>(column)).collect::<rusqlite::Result<Vec<_>>>())?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows.sort_by_cached_key(|row| format!("{row:?}"));
            state.insert(table, rows);
        }
        tx.commit()?;
        Ok(state)
    }).await.unwrap()
}

fn stroke(id: u128) -> InkStroke {
    InkStroke {
        id: uuid::Uuid::from_u128(id),
        width: 2.0,
        points: vec![InkPoint {
            x: id as f64,
            y: 2.0,
            pressure: 0.7,
            tilt_x: 0.0,
            tilt_y: 0.0,
            time: 1.0,
        }],
    }
}

fn draw(ink: &InkStore, ids: &[u128]) {
    let revision = ink.read().unwrap().revision;
    ink.patch(
        InkDraftPatch {
            order: ids.iter().copied().map(uuid::Uuid::from_u128).collect(),
            upserts: ids.iter().copied().map(stroke).collect(),
            background: InkBackground::Grid,
        },
        revision,
    )
    .unwrap();
}

struct RestoreFixture {
    fixture: Fixture,
    archive: DataArchive,
    text_uuid: uuid::Uuid,
    ink: InkStore,
    expected_drawing: serde_json::Value,
    previous_head: uuid::Uuid,
}

async fn restore_fixture() -> RestoreFixture {
    let fixture = fixture().await;
    notes_core::configure_sync(&fixture.connection, "https://sync.example")
        .await
        .unwrap();
    let text = fixture
        .store
        .create_page_with_markdown("Archive text".into(), "archived paragraph".into())
        .await
        .unwrap()
        .value;
    let drawing = fixture
        .store
        .create_handwritten_note(Some("Archive drawing".into()))
        .await
        .unwrap()
        .value;
    let ink = InkStore::new(fixture._directory.path().join("notes.db"), drawing.uuid);
    draw(&ink, &[1]);
    ink.publish("Before archive".into()).unwrap().unwrap();
    let expected_drawing = serde_json::to_value(ink.read().unwrap().draft).unwrap();
    let archive = db::export_archive(&fixture.connection).await.unwrap();
    fixture
        .store
        .create_page_with_markdown("Archive text".into(), "later paragraph".into())
        .await
        .unwrap();
    draw(&ink, &[1, 2]);
    let previous_head = ink
        .publish("After archive".into())
        .unwrap()
        .unwrap()
        .version_uuid;
    let deleted = fixture
        .store
        .create_handwritten_note(Some("Deleted drawing".into()))
        .await
        .unwrap()
        .value;
    let deleted_ink = InkStore::new(fixture._directory.path().join("notes.db"), deleted.uuid);
    draw(&deleted_ink, &[9]);
    fixture.store.delete_page(deleted.uuid).await.unwrap();
    assert_eq!(pins(&fixture.connection).await, 1);
    fixture.connection.call(|database| database.execute(
        "INSERT INTO sync_meta(key,value) VALUES('last_server_seq','41') ON CONFLICT(key) DO UPDATE SET value=excluded.value", []
    )).await.unwrap();
    fixture.notices.lock().unwrap().clear();
    RestoreFixture {
        fixture,
        archive,
        text_uuid: text.uuid,
        ink,
        expected_drawing,
        previous_head,
    }
}

#[tokio::test]
async fn mixed_archive_restore_publishes_one_replacement_and_preserves_replication_progress() {
    let restore = restore_fixture().await;
    let before = revision(&restore.fixture.connection).await;
    let pending = notes_core::pending_outbox(&restore.fixture.connection, 100)
        .await
        .unwrap();
    let installed = Arc::new(AtomicBool::new(false));
    let callback_installed = installed.clone();
    let callback_notices = restore.fixture.notices.clone();
    let committed = restore
        .fixture
        .store
        .import_archive_with_precommit(restore.archive, move || {
            assert!(
                callback_notices.lock().unwrap().is_empty(),
                "notification cannot precede blob publication"
            );
            callback_installed.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap();
    assert!(installed.load(Ordering::SeqCst));
    assert_eq!(committed.commit_version, Some(before.0 + 1));
    assert_eq!(committed.workspace_epoch, before.1 + 1);
    assert_eq!(committed.value.applied_operations, 8);
    assert!(committed.changes.is_empty());
    let notices = restore.fixture.notices.lock().unwrap().clone();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].workspace_replaced);
    assert_eq!(notices[0].workspace_epoch, before.1 + 1);
    assert_eq!(notices[0].commit_version, before.0 + 1);
    assert_eq!(
        notes_core::sync_cursor(&restore.fixture.connection)
            .await
            .unwrap(),
        41
    );
    let after = notes_core::pending_outbox(&restore.fixture.connection, 100)
        .await
        .unwrap();
    assert_eq!(
        after.len(),
        pending.len() + committed.value.applied_operations
    );
    for operation in pending {
        assert_eq!(
            after
                .iter()
                .find(|candidate| candidate.op_id == operation.op_id),
            Some(&operation)
        );
    }
    assert_eq!(
        db::history_status(&restore.fixture.connection)
            .await
            .unwrap()
            .undo_count,
        0
    );
    assert_eq!(
        db::history_status(&restore.fixture.connection)
            .await
            .unwrap()
            .redo_count,
        0
    );
    assert_eq!(pins(&restore.fixture.connection).await, 0);
    let blocks = db::list_block_children(&restore.fixture.connection, restore.text_uuid, None)
        .await
        .unwrap();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].markdown, "archived paragraph");
    assert_eq!(
        serde_json::to_value(restore.ink.read().unwrap().draft).unwrap(),
        restore.expected_drawing
    );
    let versions = restore.ink.versions().unwrap();
    assert_eq!(versions.len(), 1);
    assert_ne!(versions[0].publication.version_uuid, restore.previous_head);
    assert_eq!(versions[0].publication.parents, vec![restore.previous_head]);
}

#[tokio::test]
async fn archive_workspace_pinning_matches_authored_operation_publication_policy() {
    let source = fixture().await;
    source.store.create_page("Incoming".into()).await.unwrap();
    let archive = db::export_archive(&source.connection).await.unwrap();
    for role in [ReplicaRole::Client, ReplicaRole::Server] {
        for url in [None, Some(" \t "), Some("https://sync.example")] {
            let target = fixture().await;
            notes_core::set_replica_role(&target.connection, role)
                .await
                .unwrap();
            if let Some(url) = url {
                notes_core::configure_sync(&target.connection, url)
                    .await
                    .unwrap();
            }
            let before = revision(&target.connection).await;
            let workspace = db::workspace_uuid(&target.connection).await.unwrap();
            let result = target.store.import_archive(archive.clone()).await;
            let pinned =
                role == ReplicaRole::Server || url.is_some_and(|url| !url.trim().is_empty());
            if pinned {
                let error = result.unwrap_err();
                assert!(
                    error.to_string().contains("different workspace"),
                    "{role:?} {url:?}: {error:#}"
                );
                assert_eq!(
                    db::workspace_uuid(&target.connection).await.unwrap(),
                    workspace
                );
                assert_eq!(revision(&target.connection).await, before);
                assert!(target.notices.lock().unwrap().is_empty());
            } else {
                let result = result.unwrap();
                assert_eq!(
                    db::workspace_uuid(&target.connection).await.unwrap(),
                    archive.workspace_uuid
                );
                assert_eq!(result.commit_version, Some(before.0 + 1));
                assert_eq!(result.workspace_epoch, before.1 + 1);
                assert_eq!(target.notices.lock().unwrap().len(), 1);
            }
        }
    }
}

#[tokio::test]
async fn precommit_failure_restores_all_text_ink_history_pins_and_replication_rows() {
    let restore = restore_fixture().await;
    let before = raw_state(&restore.fixture.connection).await;
    let callback_ran = Arc::new(AtomicBool::new(false));
    let flag = callback_ran.clone();
    let error = restore
        .fixture
        .store
        .import_archive_with_precommit(restore.archive, move || {
            flag.store(true, Ordering::SeqCst);
            Err(anyhow::anyhow!("archive publication failpoint"))
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("archive publication failpoint"));
    assert!(callback_ran.load(Ordering::SeqCst));
    assert_eq!(raw_state(&restore.fixture.connection).await, before);
    assert!(restore.fixture.notices.lock().unwrap().is_empty());
}

#[tokio::test]
async fn final_epoch_failure_rolls_back_the_archive_even_after_blob_publication() {
    let restore = restore_fixture().await;
    let before = raw_state(&restore.fixture.connection).await;
    restore
        .fixture
        .connection
        .call(|database| {
            database.execute_batch(
        "CREATE TEMP TRIGGER reject_archive_epoch BEFORE UPDATE OF workspace_epoch ON store_state
         BEGIN SELECT RAISE(ABORT, 'archive epoch failpoint'); END;"
    )
        })
        .await
        .unwrap();
    let callback_ran = Arc::new(AtomicBool::new(false));
    let flag = callback_ran.clone();
    let error = restore
        .fixture
        .store
        .import_archive_with_precommit(restore.archive, move || {
            flag.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("archive epoch failpoint"));
    assert!(callback_ran.load(Ordering::SeqCst));
    assert_eq!(raw_state(&restore.fixture.connection).await, before);
    assert!(restore.fixture.notices.lock().unwrap().is_empty());
}
