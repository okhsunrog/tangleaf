use notes_core::db;
use notes_core::store::{CommitNotice, NotesStore};
use notes_core::{Connection, CoreError, ReplicaRole};
use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::sync::{Arc, Mutex, mpsc};
use std::task::Poll;
use uuid::Uuid;

struct Fixture {
    directory: tempfile::TempDir,
    connection: Connection,
    workspace: Uuid,
    store: NotesStore,
    notices: Arc<Mutex<Vec<CommitNotice>>>,
}

async fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(directory.path().join("notes.db")).await.unwrap();
    let workspace = db::workspace_uuid(&connection).await.unwrap();
    let notices = Arc::new(Mutex::new(Vec::new()));
    let observed = notices.clone();
    let store = NotesStore::new(connection.clone()).with_commit_observer(move |notice| {
        observed.lock().unwrap().push(notice.clone());
    });
    Fixture {
        directory,
        connection,
        workspace,
        store,
        notices,
    }
}

async fn value(connection: &Connection, key: &'static str) -> Option<String> {
    connection
        .call(move |database| {
            use rusqlite::OptionalExtension;
            database
                .query_row("SELECT value FROM sync_meta WHERE key = ?1", [key], |row| {
                    row.get(0)
                })
                .optional()
        })
        .await
        .unwrap()
}

async fn changes(connection: &Connection) -> u64 {
    connection
        .call(|database| Ok(database.total_changes()))
        .await
        .unwrap()
}

async fn state(connection: &Connection) -> serde_json::Value {
    let content = notes_core::export_sync_snapshot(connection, 0)
        .await
        .unwrap();
    let metadata = connection.call(|database| {
        let metadata = database.prepare("SELECT key, value FROM sync_meta ORDER BY key")?
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let outbox = database.prepare("SELECT rowid, op_id, envelope, quarantined_at, quarantine_reason FROM sync_outbox ORDER BY rowid")?
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Uuid>(1)?, row.get::<_, String>(2)?, row.get::<_, Option<i64>>(3)?, row.get::<_, Option<String>>(4)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let applied = database.prepare("SELECT op_id, seq FROM applied_ops ORDER BY op_id")?
            .query_map([], |row| Ok((row.get::<_, Uuid>(0)?, row.get::<_, Option<i64>>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let counts = database.query_row("SELECT (SELECT version FROM store_state), (SELECT COUNT(*) FROM history_undo), (SELECT COUNT(*) FROM history_redo)",
            [], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)))?;
        Ok((metadata, outbox, applied, counts))
    }).await.unwrap();
    serde_json::json!([content, metadata])
}

async fn queued_operations(f: &Fixture, count: usize) -> Vec<notes_core::Op> {
    notes_core::configure_sync(&f.connection, "https://sync.example")
        .await
        .unwrap();
    for index in 0..count {
        db::create_page(&f.connection, format!("Queued {index}"))
            .await
            .unwrap();
    }
    notes_core::pending_outbox(&f.connection, 100)
        .await
        .unwrap()
}

fn assert_quiet_notices(f: &Fixture, expected: usize) {
    let notices = f.notices.lock().unwrap();
    assert_eq!(notices.len(), expected);
    assert!(notices.iter().all(|notice| notice.changes.is_empty()));
}

#[tokio::test]
async fn role_and_configuration_changes_publish_progress_while_exact_repeats_make_zero_sql_writes()
{
    let f = fixture().await;
    let role = f.store.set_replica_role(ReplicaRole::Server).await.unwrap();
    let configured = f
        .store
        .configure_sync(" https://sync.example/ \n")
        .await
        .unwrap();
    assert!(role.commit_version.is_some());
    assert_eq!(
        configured.commit_version,
        role.commit_version.map(|version| version + 1)
    );
    assert_eq!(
        value(&f.connection, "replica_role").await.as_deref(),
        Some("server")
    );
    assert_eq!(
        value(&f.connection, "server_url").await.as_deref(),
        Some("https://sync.example/")
    );
    assert_quiet_notices(&f, 2);
    let before = state(&f.connection).await;
    let sql_changes = changes(&f.connection).await;
    for repeated in [
        f.store.set_replica_role(ReplicaRole::Server).await.unwrap(),
        f.store
            .configure_sync("\n https://sync.example/ ")
            .await
            .unwrap(),
    ] {
        assert_eq!(repeated.commit_version, None);
        assert!(repeated.changes.is_empty());
    }
    assert_eq!(changes(&f.connection).await, sql_changes);
    assert_eq!(state(&f.connection).await, before);
    assert_quiet_notices(&f, 2);
}

#[tokio::test]
async fn changing_server_clears_only_binding_and_preserves_cursor_pending_content_and_clock() {
    let f = fixture().await;
    f.store
        .configure_sync("https://first.example")
        .await
        .unwrap();
    f.store
        .bind_sync_workspace("https://first.example", f.workspace, 7)
        .await
        .unwrap();
    db::create_page(&f.connection, "Pending".into())
        .await
        .unwrap();
    let pending = notes_core::pending_outbox(&f.connection, 100)
        .await
        .unwrap();
    let clock = value(&f.connection, "last_hlc").await;
    let unchanged = f
        .store
        .configure_sync(" https://first.example \n")
        .await
        .unwrap();
    assert_eq!(unchanged.commit_version, None);
    assert_eq!(
        notes_core::sync_bound_workspace(&f.connection)
            .await
            .unwrap(),
        Some(f.workspace)
    );
    let changed = f
        .store
        .configure_sync("https://second.example")
        .await
        .unwrap();
    assert!(changed.commit_version.is_some());
    assert_eq!(
        notes_core::sync_bound_workspace(&f.connection)
            .await
            .unwrap(),
        None
    );
    assert_eq!(notes_core::sync_cursor(&f.connection).await.unwrap(), 7);
    assert_eq!(
        notes_core::pending_outbox(&f.connection, 100)
            .await
            .unwrap(),
        pending
    );
    assert_eq!(value(&f.connection, "last_hlc").await, clock);
    assert_quiet_notices(&f, 3);
    let before = state(&f.connection).await;
    for (url, workspace, seq, invalid) in [
        ("https://first.example", f.workspace, 3, false),
        ("https://second.example", Uuid::now_v7(), 3, false),
        (
            "https://second.example",
            f.workspace,
            i64::MAX as u64 + 1,
            true,
        ),
        ("https://second.example", f.workspace, u64::MAX, true),
    ] {
        let error = f
            .store
            .bind_sync_workspace(url, workspace, seq)
            .await
            .unwrap_err();
        assert!(matches!(
            (error.downcast_ref::<CoreError>(), invalid),
            (Some(CoreError::Conflict(_)), false) | (Some(CoreError::InvalidInput(_)), true)
        ));
        assert_eq!(state(&f.connection).await, before);
    }
    // Binding assigns the bootstrap cursor; it intentionally is not a gapless
    // pull operation and can select an earlier canonical server snapshot.
    let bound = f
        .store
        .bind_sync_workspace(" https://second.example ", f.workspace, 3)
        .await
        .unwrap();
    assert!(bound.commit_version.is_some());
    assert_eq!(notes_core::sync_cursor(&f.connection).await.unwrap(), 3);
    let writes = changes(&f.connection).await;
    assert_eq!(
        f.store
            .bind_sync_workspace("https://second.example", f.workspace, 3)
            .await
            .unwrap()
            .commit_version,
        None
    );
    assert_eq!(changes(&f.connection).await, writes);
    f.connection
        .call(|database| {
            database
                .execute(
                    "UPDATE sync_meta SET value = '0003' WHERE key = 'last_server_seq'",
                    [],
                )
                .map(|_| ())
        })
        .await
        .unwrap();
    let canonical = f
        .store
        .bind_sync_workspace("https://second.example", f.workspace, 3)
        .await
        .unwrap();
    assert_eq!(
        canonical.commit_version,
        bound.commit_version.map(|version| version + 1)
    );
    assert_eq!(
        value(&f.connection, "last_server_seq").await.as_deref(),
        Some("3")
    );
    assert_quiet_notices(&f, 5);
}

#[tokio::test]
async fn empty_url_remains_a_supported_configuration_and_does_not_drop_pending_changes() {
    let f = fixture().await;
    let operations = queued_operations(&f, 1).await;
    f.store
        .bind_sync_workspace("https://sync.example", f.workspace, 4)
        .await
        .unwrap();
    let disabled = f.store.configure_sync(" \n ").await.unwrap();
    assert!(disabled.commit_version.is_some());
    assert_eq!(
        value(&f.connection, "server_url").await.as_deref(),
        Some("")
    );
    assert_eq!(
        notes_core::sync_bound_workspace(&f.connection)
            .await
            .unwrap(),
        None
    );
    assert_eq!(notes_core::sync_cursor(&f.connection).await.unwrap(), 4);
    assert_eq!(
        notes_core::pending_outbox(&f.connection, 100)
            .await
            .unwrap(),
        operations
    );
    let writes = changes(&f.connection).await;
    assert_eq!(
        f.store.configure_sync("").await.unwrap().commit_version,
        None
    );
    assert_eq!(changes(&f.connection).await, writes);
    assert_quiet_notices(&f, 2);
}

#[tokio::test]
async fn late_configuration_and_binding_failures_restore_every_prior_value_and_publish_nothing() {
    for command in ["configuration", "binding"] {
        let f = fixture().await;
        f.store
            .configure_sync("https://first.example")
            .await
            .unwrap();
        if command == "configuration" {
            f.store
                .bind_sync_workspace("https://first.example", f.workspace, 7)
                .await
                .unwrap();
        }
        f.notices.lock().unwrap().clear();
        let reject_key = if command == "configuration" {
            "server_url"
        } else {
            "last_server_seq"
        };
        f.connection
            .call(move |database| {
                database.execute_batch(&format!(
                    "CREATE TRIGGER reject_sync_metadata BEFORE INSERT ON sync_meta
             WHEN NEW.key = '{reject_key}' BEGIN SELECT RAISE(ABORT, 'metadata failpoint'); END;",
                ))
            })
            .await
            .unwrap();
        let before = state(&f.connection).await;
        let error = if command == "configuration" {
            f.store
                .configure_sync("https://second.example")
                .await
                .unwrap_err()
        } else {
            f.store
                .bind_sync_workspace("https://first.example", f.workspace, 7)
                .await
                .unwrap_err()
        };
        assert!(error.to_string().contains("metadata failpoint"));
        assert_eq!(state(&f.connection).await, before, "{command}");
        assert_quiet_notices(&f, 0);
    }
}

async fn block_worker(connection: &Connection) -> (mpsc::Sender<()>, tokio::task::JoinHandle<()>) {
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let connection = connection.clone();
    let blocker = tokio::spawn(async move {
        connection
            .call(move |_| {
                let _ = entered_tx.send(());
                let _ = release_rx.recv();
                Ok(())
            })
            .await
            .unwrap();
    });
    entered_rx.await.unwrap();
    (release_tx, blocker)
}

async fn enqueue<F: Future>(mut future: Pin<&mut F>) {
    poll_fn(|context| {
        assert!(future.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
}

#[tokio::test]
async fn acknowledgement_and_quarantine_races_follow_worker_order_without_resurrecting_outbox_rows()
{
    for acknowledgement_first in [true, false] {
        let f = fixture().await;
        let operation = queued_operations(&f, 1).await.pop().unwrap();
        let (release, blocker) = block_worker(&f.connection).await;
        let mut ack = std::pin::pin!(f.store.acknowledge_server_ops(vec![(operation.op_id, 1)]));
        let mut quarantine = std::pin::pin!(f.store.quarantine_outbox_op(
            operation.op_id,
            "Rejected".into(),
            10
        ));
        if acknowledgement_first {
            enqueue(ack.as_mut()).await;
            enqueue(quarantine.as_mut()).await;
        } else {
            enqueue(quarantine.as_mut()).await;
            enqueue(ack.as_mut()).await;
        }
        release.send(()).unwrap();
        blocker.await.unwrap();
        let (ack, quarantine) = tokio::join!(ack, quarantine);
        let ack = ack.unwrap();
        let quarantine = quarantine.unwrap();
        assert!(ack.commit_version.is_some());
        assert_eq!(quarantine.value, !acknowledgement_first);
        if acknowledgement_first {
            assert_eq!(quarantine.commit_version, None);
        } else {
            assert_eq!(
                ack.commit_version,
                quarantine.commit_version.map(|version| version + 1)
            );
        }
        assert!(
            notes_core::pending_outbox(&f.connection, 100)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            notes_core::quarantined_outbox(&f.connection, 100)
                .await
                .unwrap()
                .is_empty()
        );
        assert_quiet_notices(&f, if acknowledgement_first { 1 } else { 2 });
    }
}

#[tokio::test]
async fn repeated_quarantine_preserves_first_reason_and_release_restores_original_queue_order() {
    let f = fixture().await;
    let operations = queued_operations(&f, 3).await;
    for (index, operation) in operations.iter().take(2).enumerate() {
        assert!(
            f.store
                .quarantine_outbox_op(
                    operation.op_id,
                    format!("Reason {index}"),
                    10 + index as i64
                )
                .await
                .unwrap()
                .value
        );
    }
    let before = state(&f.connection).await;
    let writes = changes(&f.connection).await;
    let duplicate = f
        .store
        .quarantine_outbox_op(operations[0].op_id, "Replacement reason".into(), 999)
        .await
        .unwrap();
    assert!(!duplicate.value);
    assert_eq!(duplicate.commit_version, None);
    let missing = f
        .store
        .quarantine_outbox_op(Uuid::now_v7(), "Missing".into(), 999)
        .await
        .unwrap();
    assert!(!missing.value);
    assert_eq!(missing.commit_version, None);
    assert_eq!(state(&f.connection).await, before);
    assert_eq!(changes(&f.connection).await, writes);
    assert_eq!(
        notes_core::quarantined_outbox(&f.connection, 100)
            .await
            .unwrap()[0]
            .reason,
        "Reason 0"
    );
    assert_eq!(
        notes_core::pending_outbox(&f.connection, 100)
            .await
            .unwrap(),
        operations[2..]
    );
    let released = f.store.release_quarantined_outbox().await.unwrap();
    assert_eq!(released.value, 2);
    assert!(released.commit_version.is_some());
    assert_eq!(
        notes_core::pending_outbox(&f.connection, 100)
            .await
            .unwrap(),
        operations
    );
    let writes = changes(&f.connection).await;
    let empty = f.store.release_quarantined_outbox().await.unwrap();
    assert_eq!(empty.value, 0);
    assert_eq!(empty.commit_version, None);
    assert_eq!(changes(&f.connection).await, writes);
    assert_quiet_notices(&f, 3);
}

#[tokio::test]
async fn release_failure_keeps_every_held_operation_and_does_not_advance_the_commit() {
    let f = fixture().await;
    let operations = queued_operations(&f, 2).await;
    for operation in &operations {
        f.store
            .quarantine_outbox_op(operation.op_id, "Held".into(), 10)
            .await
            .unwrap();
    }
    f.notices.lock().unwrap().clear();
    f.connection
        .call(|database| {
            database.execute_batch(
                "CREATE TRIGGER reject_release BEFORE UPDATE ON sync_outbox
         WHEN NEW.quarantined_at IS NULL AND OLD.rowid = (SELECT MAX(rowid) FROM sync_outbox)
         BEGIN SELECT RAISE(ABORT, 'release failpoint'); END;",
            )
        })
        .await
        .unwrap();
    let before = state(&f.connection).await;
    let error = f.store.release_quarantined_outbox().await.unwrap_err();
    assert!(error.to_string().contains("release failpoint"));
    assert_eq!(state(&f.connection).await, before);
    assert_quiet_notices(&f, 0);
}

#[tokio::test]
async fn metadata_observer_sees_committed_configuration_and_binding_on_another_connection() {
    let f = fixture().await;
    let path = f.directory.path().join("notes.db");
    let observed = Arc::new(Mutex::new(Vec::new()));
    let rows = observed.clone();
    let store = NotesStore::new(f.connection.clone()).with_commit_observer(move |notice| {
        let reader = rusqlite::Connection::open(&path).unwrap();
        let row = reader
            .query_row(
                "SELECT (SELECT value FROM sync_meta WHERE key='server_url'),
            (SELECT value FROM sync_meta WHERE key='bound_workspace_uuid'),
            (SELECT value FROM sync_meta WHERE key='last_server_seq'),
            (SELECT version FROM store_state)",
                [],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .unwrap();
        rows.lock().unwrap().push((notice.clone(), row));
    });
    let configured = store.configure_sync("https://sync.example").await.unwrap();
    let bound = store
        .bind_sync_workspace("https://sync.example", f.workspace, 9)
        .await
        .unwrap();
    let observed = observed.lock().unwrap();
    assert_eq!(observed.len(), 2);
    assert_eq!(
        observed[0].1,
        (
            Some("https://sync.example".into()),
            None,
            None,
            configured.commit_version.unwrap()
        )
    );
    assert_eq!(
        observed[1].1,
        (
            Some("https://sync.example".into()),
            Some(f.workspace.to_string()),
            Some("9".into()),
            bound.commit_version.unwrap()
        )
    );
    assert!(
        observed
            .iter()
            .all(|(notice, row)| notice.changes.is_empty() && notice.commit_version == row.3)
    );
}

#[tokio::test]
async fn legacy_metadata_adapters_preserve_results_without_attaching_an_observer() {
    let f = fixture().await;
    notes_core::configure_sync(&f.connection, "https://sync.example")
        .await
        .unwrap();
    notes_core::set_replica_role(&f.connection, ReplicaRole::Server)
        .await
        .unwrap();
    notes_core::bind_sync_workspace(&f.connection, "https://sync.example", f.workspace, 5)
        .await
        .unwrap();
    db::create_page(&f.connection, "Queued".into())
        .await
        .unwrap();
    let operation = notes_core::pending_outbox(&f.connection, 10)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert!(
        notes_core::quarantine_outbox_op(&f.connection, operation.op_id, "Held".into(), 10)
            .await
            .unwrap()
    );
    assert!(
        !notes_core::quarantine_outbox_op(&f.connection, operation.op_id, "Other".into(), 20)
            .await
            .unwrap()
    );
    assert_eq!(
        notes_core::release_quarantined_outbox(&f.connection)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        notes_core::release_quarantined_outbox(&f.connection)
            .await
            .unwrap(),
        0
    );
    assert_eq!(notes_core::sync_cursor(&f.connection).await.unwrap(), 5);
    assert_eq!(
        value(&f.connection, "replica_role").await.as_deref(),
        Some("server")
    );
    assert_quiet_notices(&f, 0);
}
