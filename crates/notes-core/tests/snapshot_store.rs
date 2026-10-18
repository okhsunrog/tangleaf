use notes_blob::BlobHash;
use notes_core::db;
use notes_core::store::{CommitNotice, NotesStore};
use notes_core::{AttachmentOwner, BlockStyle, Connection, CoreError, Hlc, SyncSnapshot};
use std::future::Future;
use std::sync::{Arc, Mutex, mpsc};
use std::task::{Poll, Waker};
use std::time::Duration;

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

async fn clock(connection: &Connection) -> (i64, i64) {
    connection
        .call(|db| {
            db.query_row(
                "SELECT version, workspace_epoch FROM store_state WHERE singleton=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
        })
        .await
        .unwrap()
}

async fn durable_state(connection: &Connection) -> serde_json::Value {
    let snapshot = notes_core::export_sync_snapshot(connection, 0)
        .await
        .unwrap();
    let outbox = notes_core::pending_outbox(connection, u32::MAX)
        .await
        .unwrap();
    let metadata = connection
        .call(|db| {
            db.prepare("SELECT key, value FROM sync_meta ORDER BY key")?
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .await
        .unwrap();
    let history = connection.call(|db| {
        db.prepare(
            "SELECT 'undo', id, action_uuid, action, forward_json, inverse_json FROM history_undo
             UNION ALL SELECT 'redo', id, action_uuid, action, forward_json, inverse_json FROM history_redo
             ORDER BY 1,2",
        )?.query_map([], |row| Ok((
            row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, uuid::Uuid>(2)?,
            row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?,
        )))?.collect::<rusqlite::Result<Vec<_>>>()
    }).await.unwrap();
    let applied = connection
        .call(|db| {
            db.query_row("SELECT COUNT(*) FROM applied_ops", [], |row| {
                row.get::<_, i64>(0)
            })
        })
        .await
        .unwrap();
    serde_json::json!([
        snapshot,
        outbox,
        metadata,
        history,
        applied,
        clock(connection).await
    ])
}

async fn source_snapshot() -> (Fixture, SyncSnapshot) {
    let source = fixture().await;
    let page = db::create_page(&source.connection, "Source page".into())
        .await
        .unwrap();
    let root = db::create_block(
        &source.connection,
        page.uuid,
        None,
        None,
        BlockStyle::Paragraph,
        "A [[linked page]] with a child".into(),
    )
    .await
    .unwrap();
    db::create_block(
        &source.connection,
        page.uuid,
        Some(root.uuid),
        None,
        BlockStyle::Bullet,
        "Child".into(),
    )
    .await
    .unwrap();
    db::create_attachment(
        &source.connection,
        AttachmentOwner::Block(root.uuid),
        BlobHash::digest(b"image bytes"),
        "image.png".into(),
        "image/png".into(),
        11,
    )
    .await
    .unwrap();
    let snapshot = notes_core::export_sync_snapshot(&source.connection, 37)
        .await
        .unwrap();
    (source, snapshot)
}

#[tokio::test]
async fn replacement_preserves_source_envelopes_cursor_and_clock_and_publishes_one_epoch() {
    let (_source, mut snapshot) = source_snapshot().await;
    let future = Hlc::new(
        chrono::Utc::now().timestamp_millis() as u64 + 86_400_000,
        40,
        uuid::Uuid::from_u128(0xF0),
    );
    snapshot.pages[0].title_hlc = Some(future.clone());
    snapshot.pages[0].existence_hlc = future.clone();
    let destination = fixture().await;
    notes_core::configure_sync(&destination.connection, "https://sync.example")
        .await
        .unwrap();
    destination.notices.lock().unwrap().clear();
    let before = clock(&destination.connection).await;
    let committed = destination
        .store
        .import_sync_snapshot(snapshot.clone())
        .await
        .unwrap();
    assert_eq!(committed.commit_version, Some(before.0 + 1));
    assert_eq!(committed.workspace_epoch, before.1 + 1);
    assert!(committed.changes.is_empty());
    assert_eq!(
        notes_core::export_sync_snapshot(&destination.connection, 37)
            .await
            .unwrap(),
        snapshot
    );
    assert_eq!(
        notes_core::sync_cursor(&destination.connection)
            .await
            .unwrap(),
        37
    );
    assert!(
        notes_core::pending_outbox(&destination.connection, u32::MAX)
            .await
            .unwrap()
            .is_empty()
    );
    let notices = destination.notices.lock().unwrap().clone();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].workspace_replaced);
    assert_eq!(notices[0].workspace_epoch, committed.workspace_epoch);
    assert_eq!(notices[0].commit_version, committed.commit_version.unwrap());
    db::rename_page(
        &destination.connection,
        snapshot.pages[0].uuid,
        Some("Newer local title".into()),
    )
    .await
    .unwrap();
    let after = notes_core::export_sync_snapshot(&destination.connection, 37)
        .await
        .unwrap();
    assert!(after.pages[0].title_hlc.as_ref().unwrap() > &future);
}

#[tokio::test]
async fn sql_failure_after_deletion_rolls_back_content_history_outbox_cursor_clock_and_notice() {
    let (_source, snapshot) = source_snapshot().await;
    let destination = fixture().await;
    destination
        .store
        .import_sync_snapshot(snapshot.clone())
        .await
        .unwrap();
    notes_core::configure_sync(&destination.connection, "https://sync.example")
        .await
        .unwrap();
    let local = db::create_note(&destination.connection, Some("Local work".into()))
        .await
        .unwrap();
    let page_uuid = snapshot.pages[0].uuid;
    destination
        .connection
        .call(move |db| {
            db.execute_batch(&format!(
                "CREATE TRIGGER fail_snapshot_insert BEFORE INSERT ON pages
             WHEN NEW.uuid=x'{}' BEGIN SELECT RAISE(ABORT, 'injected snapshot failure'); END;",
                page_uuid.simple(),
            ))?;
            Ok(())
        })
        .await
        .unwrap();
    destination.notices.lock().unwrap().clear();
    let before = durable_state(&destination.connection).await;
    let error = destination
        .store
        .import_sync_snapshot(snapshot)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("injected snapshot failure"),
        "{error}"
    );
    assert_eq!(durable_state(&destination.connection).await, before);
    assert!(
        db::get_page(
            &destination.connection,
            local.into_created().unwrap().page.uuid
        )
        .await
        .unwrap()
        .is_some()
    );
    assert!(destination.notices.lock().unwrap().is_empty());
}

#[tokio::test]
async fn bootstrap_rechecks_emptiness_after_a_local_edit_queued_during_network_work() {
    let (_source, snapshot) = source_snapshot().await;
    let destination = fixture().await;
    let inspected = notes_core::export_sync_snapshot(&destination.connection, 0)
        .await
        .unwrap();
    assert!(inspected.pages.is_empty());
    let before = clock(&destination.connection).await;
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut blocker = Box::pin(destination.connection.call(move |_| {
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        Ok(())
    }));
    let mut context = std::task::Context::from_waker(Waker::noop());
    assert!(blocker.as_mut().poll(&mut context).is_pending());
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let mut edit = Box::pin(
        destination
            .store
            .create_note(Some("Written while downloading".into())),
    );
    assert!(edit.as_mut().poll(&mut context).is_pending());
    let mut bootstrap = Box::pin(destination.store.bootstrap_sync_snapshot(snapshot));
    assert!(bootstrap.as_mut().poll(&mut context).is_pending());
    release_tx.send(()).unwrap();
    blocker.await.unwrap();
    let local = edit.await.unwrap();
    let error = bootstrap.await.unwrap_err();
    assert!(matches!(
        error.downcast_ref::<CoreError>(),
        Some(CoreError::Conflict(_))
    ));
    assert_eq!(
        db::workspace_uuid(&destination.connection).await.unwrap(),
        inspected.workspace_uuid
    );
    assert!(
        db::get_page(
            &destination.connection,
            local.value.into_created().unwrap().page.uuid
        )
        .await
        .unwrap()
        .is_some()
    );
    assert_eq!(
        notes_core::sync_cursor(&destination.connection)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        clock(&destination.connection).await,
        (before.0 + 1, before.1)
    );
    let notices = destination.notices.lock().unwrap();
    assert_eq!(notices.len(), 1);
    assert!(!notices[0].workspace_replaced);
}

#[tokio::test]
async fn empty_bootstrap_adopts_the_canonical_server_workspace_and_notifies() {
    let source = fixture().await;
    let snapshot = notes_core::export_sync_snapshot(&source.connection, 12)
        .await
        .unwrap();
    let destination = fixture().await;
    assert_ne!(
        db::workspace_uuid(&destination.connection).await.unwrap(),
        snapshot.workspace_uuid
    );
    let committed = destination
        .store
        .bootstrap_sync_snapshot(snapshot.clone())
        .await
        .unwrap();
    assert_eq!(
        notes_core::export_sync_snapshot(&destination.connection, 12)
            .await
            .unwrap(),
        snapshot
    );
    assert_eq!(
        notes_core::sync_cursor(&destination.connection)
            .await
            .unwrap(),
        12
    );
    assert_eq!(committed.workspace_epoch, 1);
    assert!(destination.notices.lock().unwrap()[0].workspace_replaced);
}

#[tokio::test]
async fn guarded_bootstrap_accepts_a_downloaded_ink_graph_and_opens_its_published_drawing() {
    use notes_core::ink::{InkBackground, InkDraftPatch, InkPoint, InkStroke, Store, transfer};

    let source = fixture().await;
    let page = source
        .store
        .create_handwritten_note(Some("Downloaded drawing".into()))
        .await
        .unwrap()
        .value;
    let source_ink = Store::new(source._directory.path().join("notes.db"), page.uuid);
    let stroke = InkStroke {
        id: uuid::Uuid::now_v7(),
        width: 2.0,
        points: (0..4)
            .map(|index| InkPoint {
                x: f64::from(index * 10),
                y: f64::from(index * 20),
                pressure: 0.5,
                tilt_x: 0.0,
                tilt_y: 0.0,
                time: f64::from(index),
            })
            .collect(),
    };
    source_ink
        .patch(
            InkDraftPatch {
                order: vec![stroke.id],
                upserts: vec![stroke],
                background: InkBackground::Grid,
            },
            None,
        )
        .unwrap();
    let publication = source_ink.publish("Source device".into()).unwrap().unwrap();
    let snapshot = notes_core::export_sync_snapshot(&source.connection, 42)
        .await
        .unwrap();
    let blobs = transfer::export_graph(&source.connection, publication.root_hash)
        .await
        .unwrap();
    let destination = fixture().await;
    // initialize_replica downloads this graph before importing the snapshot.
    // Immutable cached bodies and staged roots are not local source edits.
    transfer::stage_graph(&destination.connection, publication.root_hash, blobs)
        .await
        .unwrap();
    let staged = destination
        .connection
        .call(|db| {
            db.query_row(
                "SELECT (SELECT COUNT(*) FROM ink_staged_roots),
                 (SELECT COUNT(*) FROM ink_records), (SELECT COUNT(*) FROM ink_chunks),
                 (SELECT COUNT(*) FROM pages), (SELECT COUNT(*) FROM page_identities),
                 (SELECT COUNT(*) FROM ink_documents), (SELECT COUNT(*) FROM ink_versions)",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                    ))
                },
            )
        })
        .await
        .unwrap();
    assert_eq!(staged.0, 1);
    assert!(staged.1 > 0);
    assert!(staged.2 > 0);
    assert_eq!((staged.3, staged.4, staged.5, staged.6), (0, 0, 0, 0));
    let committed = destination
        .store
        .bootstrap_sync_snapshot(snapshot.clone())
        .await
        .unwrap();
    assert_eq!(committed.workspace_epoch, 1);
    assert_eq!(destination.notices.lock().unwrap().len(), 1);
    assert_eq!(
        notes_core::export_sync_snapshot(&destination.connection, 42)
            .await
            .unwrap(),
        snapshot
    );
    let downloaded = Store::new(destination._directory.path().join("notes.db"), page.uuid);
    let opened = downloaded.open_editor(true).unwrap();
    assert_eq!(
        serde_json::to_value(opened.snapshot.draft).unwrap(),
        serde_json::to_value(source_ink.read().unwrap().draft).unwrap(),
    );
    downloaded.close_editor().unwrap();
}

#[tokio::test]
async fn a_nonempty_foreign_workspace_and_invalid_payload_cannot_publish_or_advance_epoch() {
    let (_source, snapshot) = source_snapshot().await;
    let destination = fixture().await;
    destination.store.create_note(None).await.unwrap();
    destination.notices.lock().unwrap().clear();
    let before = durable_state(&destination.connection).await;
    let error = destination
        .store
        .import_sync_snapshot(snapshot.clone())
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<CoreError>(),
        Some(CoreError::Conflict(_))
    ));
    let mut invalid = snapshot;
    invalid.workspace_uuid = uuid::Uuid::nil();
    let error = destination
        .store
        .import_sync_snapshot(invalid)
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<CoreError>(),
        Some(CoreError::InvalidInput(_))
    ));
    assert_eq!(durable_state(&destination.connection).await, before);
    assert!(destination.notices.lock().unwrap().is_empty());
}

#[tokio::test]
async fn sequence_boundary_is_supported_but_overflow_is_rejected_before_worker_admission() {
    let (_source, mut snapshot) = source_snapshot().await;
    snapshot.seq = i64::MAX as u64;
    let destination = fixture().await;
    destination
        .store
        .import_sync_snapshot(snapshot.clone())
        .await
        .unwrap();
    assert_eq!(
        notes_core::sync_cursor(&destination.connection)
            .await
            .unwrap(),
        i64::MAX as u64
    );
    destination.notices.lock().unwrap().clear();
    let before = durable_state(&destination.connection).await;
    snapshot.seq += 1;
    snapshot.pages[0].title = Some("Must not replace the saved title".into());

    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut blocker = Box::pin(destination.connection.call(move |_| {
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        Ok(())
    }));
    let mut context = std::task::Context::from_waker(Waker::noop());
    assert!(blocker.as_mut().poll(&mut context).is_pending());
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let mut import = Box::pin(destination.store.import_sync_snapshot(snapshot.clone()));
    // The worker is blocked, so immediate failure proves pure validation did
    // not admit a replacement job or wait for any database access.
    let Poll::Ready(Err(error)) = import.as_mut().poll(&mut context) else {
        panic!("overflow must be rejected before worker admission");
    };
    assert!(matches!(
        error.downcast_ref::<CoreError>(),
        Some(CoreError::InvalidInput(_))
    ));
    drop(import);
    release_tx.send(()).unwrap();
    blocker.await.unwrap();

    let error = destination
        .store
        .bootstrap_sync_snapshot(snapshot)
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<CoreError>(),
        Some(CoreError::InvalidInput(_))
    ));
    assert_eq!(durable_state(&destination.connection).await, before);
    assert!(destination.notices.lock().unwrap().is_empty());
}

#[tokio::test]
async fn replacement_clears_local_pending_and_history_state_after_a_successful_same_workspace_import()
 {
    let (_source, snapshot) = source_snapshot().await;
    let destination = fixture().await;
    destination
        .store
        .import_sync_snapshot(snapshot.clone())
        .await
        .unwrap();
    notes_core::configure_sync(&destination.connection, "https://sync.example")
        .await
        .unwrap();
    db::create_note(
        &destination.connection,
        Some("Discarded by authoritative replacement".into()),
    )
    .await
    .unwrap();
    assert!(
        !notes_core::pending_outbox(&destination.connection, u32::MAX)
            .await
            .unwrap()
            .is_empty()
    );
    destination.notices.lock().unwrap().clear();
    destination
        .store
        .import_sync_snapshot(snapshot.clone())
        .await
        .unwrap();
    assert_eq!(
        notes_core::export_sync_snapshot(&destination.connection, 37)
            .await
            .unwrap(),
        snapshot
    );
    let counts = destination
        .connection
        .call(|db| {
            db.query_row(
                "SELECT (SELECT COUNT(*) FROM applied_ops), (SELECT COUNT(*) FROM sync_outbox),
         (SELECT COUNT(*) FROM history_undo), (SELECT COUNT(*) FROM history_redo)",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
        })
        .await
        .unwrap();
    assert_eq!(counts, (0, 0, 0, 0));
    assert_eq!(destination.notices.lock().unwrap().len(), 1);
}
