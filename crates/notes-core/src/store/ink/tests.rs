use super::*;
use crate::db;
use crate::ink::{InkBackground, InkDraftPatch, InkPoint, InkStroke, Publish, Store, transfer};
use crate::store::CommitNotice;
use crate::{ChangeSet, Connection, Op, Origin, SyncSnapshot};
use std::future::Future;
use std::sync::{Arc, Mutex, mpsc};
use std::task::Waker;
use std::time::Duration;
use uuid::Uuid;

struct Source {
    _directory: tempfile::TempDir,
    page: Uuid,
    snapshot: SyncSnapshot,
    operations: Vec<Op>,
    versions: Vec<(Publish, Blobs)>,
}

fn patch(count: u128) -> InkDraftPatch {
    let strokes = (1..=count)
        .map(|id| InkStroke {
            id: Uuid::from_u128(id),
            width: 2.0,
            points: vec![InkPoint {
                x: id as f64,
                y: 2.0,
                pressure: 0.7,
                tilt_x: 0.0,
                tilt_y: 0.0,
                time: 1.0,
            }],
        })
        .collect::<Vec<_>>();
    InkDraftPatch {
        order: strokes.iter().map(|stroke| stroke.id).collect(),
        upserts: strokes,
        background: InkBackground::Grid,
    }
}

async fn source(count: u128) -> Source {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.db");
    let connection = db::open(&path).await.unwrap();
    crate::configure_sync(&connection, "https://sync.example")
        .await
        .unwrap();
    let page = db::create_handwritten_note_with_ops(&connection, Some("Drawing".into()))
        .await
        .unwrap()
        .value;
    let ink = Store::new(&path, page.uuid);
    let mut versions = Vec::new();
    for n in 1..=count {
        ink.patch(patch(n), ink.read().unwrap().revision).unwrap();
        let publication = ink.publish("Source".into()).unwrap().unwrap();
        let blobs = transfer::export_graph(&connection, publication.root_hash)
            .await
            .unwrap();
        versions.push((publication, blobs));
    }
    let operations = crate::pending_outbox(&connection, 100).await.unwrap();
    let snapshot = crate::export_sync_snapshot(&connection, 42).await.unwrap();
    connection.close().await.unwrap();
    Source {
        _directory: directory,
        page: page.uuid,
        snapshot,
        operations,
        versions,
    }
}

struct Destination {
    directory: tempfile::TempDir,
    connection: Connection,
    store: NotesStore,
    notices: Arc<Mutex<Vec<CommitNotice>>>,
}

impl Destination {
    fn ink(&self, page: Uuid) -> Store {
        Store::new(self.directory.path().join("destination.db"), page)
    }
}

async fn destination(source: &Source, first_available: bool) -> Destination {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(directory.path().join("destination.db"))
        .await
        .unwrap();
    if first_available {
        let (publication, blobs) = &source.versions[0];
        transfer::stage_graph(&connection, publication.root_hash, blobs.clone())
            .await
            .unwrap();
    }
    let notices = Arc::new(Mutex::new(Vec::new()));
    let observed = notices.clone();
    let store = NotesStore::new(connection.clone())
        .with_commit_observer(move |notice| observed.lock().unwrap().push(notice.clone()));
    store
        .import_sync_snapshot(source.snapshot.clone())
        .await
        .unwrap();
    // Record the already-known operation IDs, so the later retry is a true
    // duplicate. Snapshot metadata itself deliberately contains no bodies.
    store
        .apply_batch(&source.operations, Origin::Remote)
        .await
        .unwrap();
    notices.lock().unwrap().clear();
    Destination {
        directory,
        connection,
        store,
        notices,
    }
}

fn expected(page: Uuid) -> ChangeSet {
    ChangeSet {
        affected_pages: vec![page],
        handwriting_pages: vec![page],
        ..Default::default()
    }
}

async fn state(connection: &Connection) -> Vec<i64> {
    connection
        .call(|database| {
            database.query_row(
                "SELECT (SELECT version FROM store_state),
                    (SELECT workspace_epoch FROM store_state),
                    (SELECT COUNT(*) FROM ink_records), (SELECT COUNT(*) FROM ink_chunks),
                    (SELECT COUNT(*) FROM ink_staged_roots),
                    (SELECT COUNT(*) FROM ink_versions WHERE root_id IS NOT NULL),
                    (SELECT COUNT(*) FROM ink_history), (SELECT COUNT(*) FROM applied_ops),
                    (SELECT COUNT(*) FROM sync_outbox), (SELECT COUNT(*) FROM history_undo),
                    (SELECT COUNT(*) FROM history_redo), total_changes()",
                [],
                |row| (0..12).map(|column| row.get(column)).collect(),
            )
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn late_head_download_publishes_actual_adoption_and_duplicate_apply_is_quiet() {
    let source = source(1).await;
    let target = destination(&source, false).await;
    let ink = target.ink(source.page);
    let before_status = ink.status().unwrap();
    assert!(!before_status.heads[0].available);
    assert!(before_status.revision.is_none());
    let before = state(&target.connection).await;
    let (publication, blobs) = &source.versions[0];
    let result = target
        .store
        .stage_ink_graph(publication.root_hash, blobs.clone())
        .await
        .unwrap();
    assert_eq!(result.changes, expected(source.page));
    assert_eq!(result.commit_version, Some(before[0] + 1));
    assert_eq!(result.workspace_epoch, before[1]);
    let status = ink.status().unwrap();
    assert!(status.heads[0].available);
    assert_eq!(status.base_version, Some(publication.version_uuid));
    assert!(status.revision.is_some());
    assert_eq!(ink.read().unwrap().draft.strokes.len(), 1);
    assert_eq!(target.notices.lock().unwrap().len(), 1);
    let after = state(&target.connection).await;
    assert_eq!(after[7..11], before[7..11]);
    let duplicate = target
        .store
        .apply_batch(&source.operations, Origin::Remote)
        .await
        .unwrap();
    assert!(duplicate.changes.is_empty());
    assert!(duplicate.commit_version.is_none());
    let staged_again = target
        .store
        .stage_ink_graph(publication.root_hash, blobs.clone())
        .await
        .unwrap();
    assert!(staged_again.commit_version.is_none());
    assert!(staged_again.changes.is_empty());
    assert_eq!(state(&target.connection).await, after);
    assert_eq!(ink.status().unwrap(), status);
    assert_eq!(target.notices.lock().unwrap().len(), 1);
    target.connection.close().await.unwrap();
    let reopened = db::open(target.directory.path().join("destination.db"))
        .await
        .unwrap();
    assert_eq!(ink.status().unwrap(), status);
    assert_eq!(ink.read().unwrap().draft.strokes.len(), 1);
    reopened.close().await.unwrap();
}

#[tokio::test]
async fn late_non_head_download_enables_only_that_pages_preview() {
    let source = source(2).await;
    let target = destination(&source, false).await;
    let ink = target.ink(source.page);
    let status = ink.status().unwrap();
    let (publication, blobs) = &source.versions[0];
    let result = target
        .store
        .stage_ink_graph(publication.root_hash, blobs.clone())
        .await
        .unwrap();
    assert_eq!(result.changes, expected(source.page));
    assert_eq!(ink.status().unwrap(), status);
    assert_eq!(
        ink.preview(publication.version_uuid).unwrap().strokes.len(),
        1
    );
    assert!(ink.read().is_err());
    assert_eq!(target.notices.lock().unwrap().len(), 1);
    target.connection.close().await.unwrap();
}

#[tokio::test]
async fn late_graph_preserves_dirty_and_open_editor_drafts() {
    let source = source(2).await;
    for dirty in [false, true] {
        let target = destination(&source, true).await;
        let ink = target.ink(source.page);
        if dirty {
            ink.patch(patch(3), ink.read().unwrap().revision).unwrap();
        } else {
            ink.open_editor(true).unwrap();
        }
        let draft = serde_json::to_value(ink.read().unwrap()).unwrap();
        let status = ink.status().unwrap();
        let (publication, blobs) = &source.versions[1];
        let result = target
            .store
            .stage_ink_graph(publication.root_hash, blobs.clone())
            .await
            .unwrap();
        assert_eq!(result.changes, expected(source.page));
        assert_eq!(serde_json::to_value(ink.read().unwrap()).unwrap(), draft);
        let after = ink.status().unwrap();
        assert!(after.heads[0].available);
        assert_eq!(after.revision, status.revision);
        assert_eq!(after.base_version, status.base_version);
        assert_eq!(after.unpublished_changes, dirty);
        target.connection.close().await.unwrap();
    }
}

#[tokio::test]
async fn failed_installation_rolls_back_bodies_availability_clock_and_notice() {
    let source = source(1).await;
    let target = destination(&source, false).await;
    target
        .connection
        .call(|database| {
            database.execute_batch(
                "CREATE TEMP TRIGGER fail_graph_install BEFORE UPDATE OF root_id ON ink_versions
         BEGIN SELECT RAISE(ABORT, 'injected late graph failure'); END;",
            )
        })
        .await
        .unwrap();
    let before = state(&target.connection).await;
    let (publication, blobs) = &source.versions[0];
    let failure = target
        .store
        .stage_ink_graph(publication.root_hash, blobs.clone())
        .await
        .unwrap_err();
    assert!(format!("{failure:#}").contains("injected late graph failure"));
    // total_changes counts rolled-back statements; all persisted state must
    // match, and the enclosing executor must reject a caught stage error too.
    let after = state(&target.connection).await;
    assert_eq!(&after[..11], &before[..11]);
    let prepared = PreparedInkGraph::prepare(publication.root_hash, blobs.clone()).unwrap();
    let caught = target
        .store
        .write(move |write| -> CoreResult<()> {
            assert!(write.stage_ink_graph(prepared).is_err());
            Ok(())
        })
        .await;
    assert!(caught.is_err());
    assert_eq!(&state(&target.connection).await[..11], &before[..11]);
    assert!(target.notices.lock().unwrap().is_empty());
    assert!(!target.ink(source.page).status().unwrap().heads[0].available);
    target.connection.close().await.unwrap();
}

#[tokio::test]
async fn staging_before_metadata_owns_a_commit_but_does_not_invent_public_changes() {
    let source = source(1).await;
    let connection = db::open_in_memory().await.unwrap();
    let store = NotesStore::new(connection.clone());
    let (publication, blobs) = &source.versions[0];
    let result = store
        .stage_ink_graph(publication.root_hash, blobs.clone())
        .await
        .unwrap();
    assert!(result.commit_version.is_some());
    assert_eq!(result.changes, ChangeSet::default());
    let before = state(&connection).await;
    assert_eq!(before[4], 1);
    let repeat = store
        .stage_ink_graph(publication.root_hash, blobs.clone())
        .await
        .unwrap();
    assert!(repeat.commit_version.is_none());
    assert_eq!(state(&connection).await, before);
    connection.close().await.unwrap();
}

#[tokio::test]
async fn cancellation_after_admission_keeps_installation_and_observer_together() {
    let source = source(1).await;
    let target = destination(&source, false).await;
    let (publication, blobs) = &source.versions[0];
    let prepared = PreparedInkGraph::prepare(publication.root_hash, blobs.clone()).unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut blocker = Box::pin(target.connection.call(move |_| {
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        Ok(())
    }));
    let mut context = std::task::Context::from_waker(Waker::noop());
    assert!(blocker.as_mut().poll(&mut context).is_pending());
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let mut accepted = Box::pin(target.store.stage_prepared_ink_graph(prepared));
    assert!(accepted.as_mut().poll(&mut context).is_pending());
    drop(accepted);
    release_tx.send(()).unwrap();
    blocker.await.unwrap();
    // This read is behind the accepted installation and its synchronous notice.
    assert_eq!(state(&target.connection).await[5], 1);
    assert_eq!(
        target.ink(source.page).read().unwrap().draft.strokes.len(),
        1
    );
    let notices = target.notices.lock().unwrap();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].changes, expected(source.page));
    drop(notices);
    target.connection.close().await.unwrap();
}
