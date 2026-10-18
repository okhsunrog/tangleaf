use super::*;
use notes_core::{
    AttachmentOwner, Connection, Hlc, Op, OpKind, Origin, PageCreate, PageDelete, PageKind,
    PageLayout, db,
};
use std::future::Future;
use std::sync::mpsc;
use std::task::{Context as TaskContext, Waker};
use std::time::Duration;

struct Fixture {
    directory: tempfile::TempDir,
    store: NotesStore,
    blobs: BlobStore,
}

async fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let store = NotesStore::open(directory.path().join("notes.db"))
        .await
        .unwrap();
    let blobs = BlobStore::new(directory.path());
    Fixture {
        directory,
        store,
        blobs,
    }
}

fn install(store: &BlobStore, bytes: &[u8]) -> BlobHash {
    let hash = BlobHash::digest(bytes);
    store
        .install_reader(bytes, hash, bytes.len() as u64)
        .unwrap();
    hash
}

async fn attach(fixture: &Fixture, page: uuid::Uuid, bytes: &[u8]) -> notes_core::db::Attachment {
    let hash = install(&fixture.blobs, bytes);
    fixture
        .store
        .create_attachment(
            AttachmentOwner::Page(page),
            hash,
            "image.png".into(),
            "image/png".into(),
            bytes.len() as u64,
        )
        .await
        .unwrap()
        .value
}

async fn remote(store: &NotesStore, index: u64, kind: OpKind) {
    let device_id = uuid::Uuid::from_u128(0xD371CE);
    let operation = Op {
        op_id: uuid::Uuid::from_u128(0x100_0000 + u128::from(index)),
        workspace_uuid: db::workspace_uuid(store.read_connection()).await.unwrap(),
        device_id,
        hlc: Hlc::new(index * 10, 0, device_id),
        format_version: notes_core::operation::FORMAT_VERSION,
        kind,
    };
    store
        .apply_batch(&[operation], Origin::Remote)
        .await
        .unwrap();
}

fn remote_page(uuid: uuid::Uuid) -> OpKind {
    OpKind::PageCreate(PageCreate {
        uuid,
        kind: PageKind::Note,
        title: Some("Delayed remote owner".into()),
        layout: PageLayout::Outline,
        created_at: 1,
    })
}

#[tokio::test]
async fn unmaterialized_present_intents_require_bodies_and_recover_after_owner_creation() {
    let fixture = fixture().await;
    let page = uuid::Uuid::new_v4();
    let owner = AttachmentOwner::Page(page);
    let bytes = b"received before its remote owner";
    let hash = install(&fixture.blobs, bytes);
    remote(
        &fixture.store,
        1,
        OpKind::AttachmentAdd(notes_core::AttachmentAdd {
            owner,
            blob_hash: hash,
            filename: "delayed.png".into(),
            mime: "image/png".into(),
            size: bytes.len() as u64,
        }),
    )
    .await;
    assert!(
        db::list_attachments(fixture.store.read_connection(), owner)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        required_attachment_blobs(fixture.store.read_connection())
            .await
            .unwrap(),
        BTreeMap::from([(hash, bytes.len() as u64)])
    );
    assert!(fixture.blobs.remove_verified(hash, 1024).unwrap());
    let destination = fixture.directory.path().join("delayed.recovery");
    assert!(
        create_recovery_backup(&fixture.store, &fixture.blobs, &destination)
            .await
            .is_err()
    );
    assert!(!destination.exists());
    assert_no_partial(fixture.directory.path());
    install(&fixture.blobs, bytes);
    create_recovery_backup(&fixture.store, &fixture.blobs, &destination)
        .await
        .unwrap();
    let verified = prepare_recovery_backup(&destination, fixture.directory.path())
        .await
        .unwrap();
    let recovered = tempfile::tempdir().unwrap();
    let recovered_blobs = BlobStore::new(recovered.path());
    verified.install_blobs(&recovered_blobs).unwrap();
    let path = recovered.path().join("notes.db");
    fs::copy(verified.database_path(), &path).unwrap();
    let reopened = NotesStore::open(path).await.unwrap();
    remote(&reopened, 2, remote_page(page)).await;
    let attachments = db::list_attachments(reopened.read_connection(), owner)
        .await
        .unwrap();
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].blob_hash, hash);
    assert_eq!(recovered_blobs.read_verified(hash, 1024).unwrap(), bytes);
    // Owner deletion only suppresses materialization; the present intent must
    // remain restorable when a newer remote create revives the owner.
    remote(&reopened, 3, OpKind::PageDelete(PageDelete { uuid: page })).await;
    assert!(
        db::list_attachments(reopened.read_connection(), owner)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        required_attachment_blobs(reopened.read_connection())
            .await
            .unwrap(),
        BTreeMap::from([(hash, bytes.len() as u64)])
    );
    remote(&reopened, 4, remote_page(page)).await;
    let attachments = db::list_attachments(reopened.read_connection(), owner)
        .await
        .unwrap();
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].blob_hash, hash);
    reopened.close().await.unwrap();
    fixture.store.close().await.unwrap();
}

#[tokio::test]
async fn malformed_present_intent_metadata_is_rejected_without_a_materialized_owner() {
    let fixture = fixture().await;
    let hash = install(&fixture.blobs, b"strict intent metadata");
    remote(
        &fixture.store,
        1,
        OpKind::AttachmentAdd(notes_core::AttachmentAdd {
            owner: AttachmentOwner::Page(uuid::Uuid::new_v4()),
            blob_hash: hash,
            filename: "delayed.png".into(),
            mime: "image/png".into(),
            size: 22,
        }),
    )
    .await;
    for update in [
        "UPDATE attachment_lww SET size=-1",
        "UPDATE attachment_lww SET size=NULL",
        "PRAGMA ignore_check_constraints=ON; UPDATE attachment_lww SET size=22,blob_hash=zeroblob(31)",
    ] {
        fixture
            .store
            .write_connection()
            .call(move |db| db.execute_batch(update))
            .await
            .unwrap();
        assert!(
            required_attachment_blobs(fixture.store.read_connection())
                .await
                .is_err()
        );
    }
    fixture.store.close().await.unwrap();
}

async fn fingerprint(connection: &Connection) -> serde_json::Value {
    let snapshot = notes_core::export_sync_snapshot(
        connection,
        notes_core::sync_cursor(connection).await.unwrap(),
    )
    .await
    .unwrap();
    let outbox = notes_core::pending_outbox(connection, u32::MAX)
        .await
        .unwrap();
    let quarantined = notes_core::quarantined_outbox(connection, u32::MAX)
        .await
        .unwrap();
    let history = connection.call(|db| {
        db.prepare("SELECT 'undo',id,forward_json,inverse_json FROM history_undo UNION ALL SELECT 'redo',id,forward_json,inverse_json FROM history_redo ORDER BY 1,2")?
            .query_map([], |r| Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?)))?.collect::<rusqlite::Result<Vec<_>>>()
    }).await.unwrap();
    serde_json::json!([
        snapshot,
        outbox,
        quarantined
            .iter()
            .map(|q| (&q.op, &q.reason))
            .collect::<Vec<_>>(),
        history
    ])
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let output = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &output);
        } else {
            fs::copy(entry.path(), output).unwrap();
        }
    }
}

fn assert_no_partial(directory: &Path) {
    assert!(!fs::read_dir(directory).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".tangleaf-recovery-")
    }));
}

#[tokio::test]
async fn full_backup_preserves_history_only_and_quarantined_outbox_only_bodies_and_reopens() {
    let fixture = fixture().await;
    let history_page = fixture
        .store
        .create_page("Retained history".into())
        .await
        .unwrap()
        .value;
    let history_blob = attach(&fixture, history_page.uuid, b"history-only body").await;
    fixture.store.delete_page(history_page.uuid).await.unwrap();
    assert!(
        notes_core::pending_outbox(fixture.store.read_connection(), u32::MAX)
            .await
            .unwrap()
            .is_empty()
    );
    notes_core::configure_sync(fixture.store.write_connection(), "https://sync.example")
        .await
        .unwrap();
    let outbox_page = fixture
        .store
        .create_page("Pending publication".into())
        .await
        .unwrap()
        .value;
    let outbox_blob = attach(&fixture, outbox_page.uuid, b"outbox-only body").await;
    let operation = notes_core::pending_outbox(fixture.store.read_connection(), u32::MAX)
        .await
        .unwrap()
        .into_iter()
        .find(|operation| matches!(operation.kind, OpKind::AttachmentAdd(_)))
        .unwrap();
    notes_core::quarantine_outbox_op(
        fixture.store.write_connection(),
        operation.op_id,
        "held publication".into(),
        123,
    )
    .await
    .unwrap();
    fixture
        .store
        .delete_attachment(outbox_blob.uuid)
        .await
        .unwrap();
    let orphan = install(&fixture.blobs, b"conservatively retained orphan");
    let required = required_attachment_blobs(fixture.store.read_connection())
        .await
        .unwrap();
    assert_eq!(required.len(), 2);
    assert_eq!(required[&history_blob.blob_hash], history_blob.size);
    assert_eq!(required[&outbox_blob.blob_hash], outbox_blob.size);
    let before = fingerprint(fixture.store.read_connection()).await;
    let destination = fixture.directory.path().join("complete.recovery");
    create_recovery_backup(&fixture.store, &fixture.blobs, &destination)
        .await
        .unwrap();
    let verified = prepare_recovery_backup(&destination, fixture.directory.path())
        .await
        .unwrap();
    assert_eq!(verified.manifest.blobs.len(), 3);
    assert!(
        verified
            .manifest
            .blobs
            .iter()
            .any(|blob| blob.hash == orphan)
    );
    let recovered = tempfile::tempdir().unwrap();
    let recovered_blobs = BlobStore::new(recovered.path());
    verified.install_blobs(&recovered_blobs).unwrap();
    assert_eq!(
        recovered_blobs
            .read_verified(history_blob.blob_hash, 1024)
            .unwrap(),
        b"history-only body"
    );
    assert_eq!(
        recovered_blobs
            .read_verified(outbox_blob.blob_hash, 1024)
            .unwrap(),
        b"outbox-only body"
    );
    let path = recovered.path().join("notes.db");
    fs::copy(verified.database_path(), &path).unwrap();
    let reopened = NotesStore::open(path).await.unwrap();
    assert_eq!(fingerprint(reopened.read_connection()).await, before);
    reopened.close().await.unwrap();
    fixture.store.close().await.unwrap();
    let original = NotesStore::open(fixture.directory.path().join("notes.db"))
        .await
        .unwrap();
    assert_eq!(fingerprint(original.read_connection()).await, before);
    original.close().await.unwrap();
    assert_no_partial(fixture.directory.path());
}

#[tokio::test]
async fn lease_covers_cleanup_and_an_attachment_committed_after_backup_starts() {
    let fixture = fixture().await;
    let page = fixture
        .store
        .create_page("Late attachment".into())
        .await
        .unwrap()
        .value;
    let orphan = install(&fixture.blobs, b"leased orphan");
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut blocker = Box::pin(fixture.store.read_connection().call(move |_| {
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        Ok(())
    }));
    let mut context = TaskContext::from_waker(Waker::noop());
    assert!(blocker.as_mut().poll(&mut context).is_pending());
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let destination = fixture.directory.path().join("late.recovery");
    let mut backup = Box::pin(create_recovery_backup(
        &fixture.store,
        &fixture.blobs,
        &destination,
    ));
    assert!(backup.as_mut().poll(&mut context).is_pending());
    let late = attach(&fixture, page.uuid, b"committed during capture").await;
    let cleanup = fixture.blobs.clone();
    db::cleanup_unreferenced_attachment_blobs(
        fixture.store.write_connection(),
        vec![orphan],
        move |hash| {
            assert!(!cleanup.remove_verified(hash, 1024)?);
            Ok(())
        },
    )
    .await
    .unwrap();
    assert!(fixture.blobs.verify(orphan).is_ok());
    release_tx.send(()).unwrap();
    blocker.await.unwrap();
    backup.await.unwrap();
    assert!(fixture.blobs.remove_verified(orphan, 1024).unwrap());
    let verified = prepare_recovery_backup(destination, fixture.directory.path())
        .await
        .unwrap();
    assert_eq!(verified.manifest.blobs.len(), 2);
    let captured = Connection::open_read_only(verified.database_path())
        .await
        .unwrap();
    assert_eq!(
        required_attachment_blobs(&captured).await.unwrap()[&late.blob_hash],
        late.size
    );
    captured.close().await.unwrap();
    fixture.store.close().await.unwrap();
}

#[tokio::test]
async fn missing_required_bytes_and_conflicting_metadata_cannot_publish_complete_backups() {
    let fixture = fixture().await;
    let page = fixture
        .store
        .create_page("Missing body".into())
        .await
        .unwrap()
        .value;
    let attachment = attach(&fixture, page.uuid, b"required bytes").await;
    fs::remove_file(fixture.blobs.path_for(attachment.blob_hash)).unwrap();
    let destination = fixture.directory.path().join("missing.recovery");
    assert!(
        create_recovery_backup(&fixture.store, &fixture.blobs, &destination)
            .await
            .is_err()
    );
    assert!(!destination.exists());
    assert_no_partial(fixture.directory.path());
    install(&fixture.blobs, b"required bytes");
    notes_core::configure_sync(fixture.store.write_connection(), "https://sync.example")
        .await
        .unwrap();
    let second = attach(&fixture, page.uuid, b"another body").await;
    fixture
        .store
        .write_connection()
        .call(move |db| {
            db.execute(
                "UPDATE attachments SET size=size+1 WHERE uuid=?1",
                [second.uuid],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(
        create_recovery_backup(&fixture.store, &fixture.blobs, &destination)
            .await
            .is_err()
    );
    assert!(!destination.exists());
    assert_no_partial(fixture.directory.path());
    fixture.store.close().await.unwrap();
}

#[tokio::test]
async fn corrupt_truncated_missing_and_malformed_recovery_inputs_are_rejected() {
    let fixture = fixture().await;
    let page = fixture
        .store
        .create_page("Validation".into())
        .await
        .unwrap()
        .value;
    let attachment = attach(&fixture, page.uuid, b"verified body").await;
    let source = fixture.directory.path().join("valid.recovery");
    create_recovery_backup(&fixture.store, &fixture.blobs, &source)
        .await
        .unwrap();
    for variant in [
        "missing",
        "truncated",
        "corrupt",
        "database",
        "database-hash",
        "version",
        "path",
        "schema",
        "metadata",
        "unknown",
    ] {
        let changed = fixture.directory.path().join(format!("invalid-{variant}"));
        copy_tree(&source, &changed);
        let body = BlobStore::new(&changed).path_for(attachment.blob_hash);
        match variant {
            "missing" => fs::remove_file(body).unwrap(),
            "truncated" => fs::write(body, b"x").unwrap(),
            "corrupt" => fs::write(body, b"xxxxxxxxxxxxx").unwrap(),
            "database" => OpenOptions::new()
                .write(true)
                .open(changed.join(DATABASE))
                .unwrap()
                .set_len(100)
                .unwrap(),
            "database-hash" => {
                let mut file = OpenOptions::new()
                    .write(true)
                    .open(changed.join(DATABASE))
                    .unwrap();
                file.write_all(b"corrupt header").unwrap();
            }
            _ => {
                let mut manifest: serde_json::Value =
                    serde_json::from_slice(&fs::read(changed.join(MANIFEST)).unwrap()).unwrap();
                match variant {
                    "version" => manifest["version"] = 99.into(),
                    "path" => manifest["database"]["file"] = "../notes.db".into(),
                    "schema" => {
                        manifest["database"]["schemaVersion"] =
                            (notes_core::db::supported_schema_version() + 1).into()
                    }
                    "metadata" => manifest["database"]["commitVersion"] = 999_999.into(),
                    "unknown" => manifest["unknown"] = true.into(),
                    _ => unreachable!(),
                }
                fs::write(
                    changed.join(MANIFEST),
                    serde_json::to_vec(&manifest).unwrap(),
                )
                .unwrap();
            }
        }
        assert!(
            prepare_recovery_backup(changed, fixture.directory.path())
                .await
                .is_err(),
            "{variant}"
        );
    }
    fixture.store.close().await.unwrap();
}

#[tokio::test]
async fn existing_destinations_and_corrupt_install_targets_are_preserved() {
    let fixture = fixture().await;
    let page = fixture
        .store
        .create_page("Preservation".into())
        .await
        .unwrap()
        .value;
    let attachment = attach(&fixture, page.uuid, b"immutable body").await;
    let destination = fixture.directory.path().join("existing.recovery");
    create_recovery_backup(&fixture.store, &fixture.blobs, &destination)
        .await
        .unwrap();
    let manifest = fs::read(destination.join(MANIFEST)).unwrap();
    assert!(
        create_recovery_backup(&fixture.store, &fixture.blobs, &destination)
            .await
            .is_err()
    );
    assert_eq!(fs::read(destination.join(MANIFEST)).unwrap(), manifest);
    let verified = prepare_recovery_backup(destination, fixture.directory.path())
        .await
        .unwrap();
    let target = tempfile::tempdir().unwrap();
    let target_blobs = BlobStore::new(target.path());
    install(&target_blobs, b"immutable body");
    fs::write(
        target_blobs.path_for(attachment.blob_hash),
        b"corrupt current bytes",
    )
    .unwrap();
    assert!(verified.install_blobs(&target_blobs).is_err());
    assert_eq!(
        fs::read(target_blobs.path_for(attachment.blob_hash)).unwrap(),
        b"corrupt current bytes"
    );
    assert_no_partial(fixture.directory.path());
    fixture.store.close().await.unwrap();
}

#[test]
fn atomic_directory_publication_does_not_replace_an_existing_empty_directory() {
    let parent = tempfile::tempdir().unwrap();
    let source = parent.path().join("source");
    let destination = parent.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("prepared"), b"prepared").unwrap();
    fs::create_dir(&destination).unwrap();
    assert!(publish_directory(&source, &destination).is_err());
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
    assert_eq!(fs::read(source.join("prepared")).unwrap(), b"prepared");
}

#[tokio::test]
async fn dropping_capture_does_not_publish_a_partial_container_or_leave_staging() {
    let fixture = fixture().await;
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut blocker = Box::pin(fixture.store.read_connection().call(move |_| {
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        Ok(())
    }));
    let mut context = TaskContext::from_waker(Waker::noop());
    assert!(blocker.as_mut().poll(&mut context).is_pending());
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let destination = fixture.directory.path().join("canceled.recovery");
    let mut backup = Box::pin(create_recovery_backup(
        &fixture.store,
        &fixture.blobs,
        &destination,
    ));
    assert!(backup.as_mut().poll(&mut context).is_pending());
    drop(backup);
    release_tx.send(()).unwrap();
    blocker.await.unwrap();
    fixture.store.close().await.unwrap();
    assert!(!destination.exists());
    assert_no_partial(fixture.directory.path());
}

#[cfg(unix)]
#[tokio::test]
async fn unsafe_links_are_rejected_and_only_regular_incoming_temporaries_are_omitted() {
    use std::os::unix::fs::symlink;
    let fixture = fixture().await;
    let hash = install(&fixture.blobs, b"body");
    let body = fixture.blobs.path_for(hash);
    fs::write(
        body.parent().unwrap().join(".incoming-test"),
        b"in progress",
    )
    .unwrap();
    let good = fixture.directory.path().join("incoming.recovery");
    create_recovery_backup(&fixture.store, &fixture.blobs, &good)
        .await
        .unwrap();
    let verified = prepare_recovery_backup(&good, fixture.directory.path())
        .await
        .unwrap();
    assert_eq!(verified.manifest.blobs.len(), 1);
    let outside = fixture.directory.path().join("outside");
    fs::write(&outside, b"body").unwrap();
    fs::remove_file(&body).unwrap();
    symlink(&outside, &body).unwrap();
    let bad = fixture.directory.path().join("unsafe.recovery");
    assert!(
        create_recovery_backup(&fixture.store, &fixture.blobs, &bad)
            .await
            .is_err()
    );
    assert!(!bad.exists());
    let alias = fixture.directory.path().join("recovery-link");
    symlink(&good, &alias).unwrap();
    assert!(
        prepare_recovery_backup(alias, fixture.directory.path())
            .await
            .is_err()
    );
    fixture.store.close().await.unwrap();
}
