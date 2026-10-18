use super::*;
use notes_core::{BlockStyle, db};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

struct Workspace {
    directory: tempfile::TempDir,
    store: NotesStore,
    blobs: BlobStore,
    lock: Option<crate::app_lock::ApplicationLock>,
}

impl Workspace {
    async fn new(title: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let lock = crate::app_lock::ApplicationLock::acquire(directory.path())
            .await
            .unwrap();
        let store = NotesStore::open(directory.path().join(DATABASE))
            .await
            .unwrap();
        let blobs = BlobStore::new(directory.path());
        let page = store.create_page(title.into()).await.unwrap().value;
        store
            .create_block(
                page.uuid,
                None,
                None,
                BlockStyle::Bullet,
                format!("Body of {title}"),
            )
            .await
            .unwrap();
        Self {
            directory,
            store,
            blobs,
            lock: Some(lock),
        }
    }

    async fn backup(&self, destination: &Path) {
        create_recovery_backup(&self.store, &self.blobs, destination)
            .await
            .unwrap();
    }

    async fn close(&mut self) -> Arc<ClosedWorkspace> {
        ClosedWorkspace::close_test_workspace(
            &self.store,
            self.blobs.clone(),
            self.lock.take().unwrap(),
        )
        .await
        .unwrap()
    }
}

async fn titles(path: &Path) -> Vec<String> {
    let reader = notes_core::Connection::open_read_only(path).await.unwrap();
    let mut titles = db::list_pages(&reader, u32::MAX)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|page| page.title)
        .collect::<Vec<_>>();
    titles.sort();
    reader.close().await.unwrap();
    titles
}

async fn incoming(parent: &Path) -> (Workspace, PathBuf) {
    let workspace = Workspace::new("Restored note").await;
    let backup = parent.join("incoming.recovery");
    workspace.backup(&backup).await;
    (workspace, backup)
}

#[tokio::test]
async fn preparation_validates_a_private_current_version_database_without_closing_the_current_workspace()
 {
    let mut current = Workspace::new("Current note").await;
    let (incoming, backup) = incoming(current.directory.path()).await;
    let shutdown = WorkspaceShutdown::default();
    let runtime = RecoveryRuntime::new(current.directory.path().to_owned(), shutdown);
    let preview = runtime.prepare(backup, &current.store).await.unwrap();
    assert_eq!((preview.page_count, preview.block_count), (1, 1));
    assert!(preview.workspace_differs);
    assert_ne!(preview.current_workspace_uuid, preview.workspace_uuid);
    assert_eq!(preview.blob_count, 0);
    assert!(runtime.discard(uuid::Uuid::new_v4()).is_err());
    assert!(
        runtime
            .accept_restore(uuid::Uuid::new_v4(), |_| async {
                panic!("invalid token executed")
            })
            .is_err()
    );
    current
        .store
        .create_page("Still writable after preview".into())
        .await
        .unwrap();
    runtime.discard(preview.token).unwrap();
    assert!(
        runtime
            .accept_restore(preview.token, |_| async {
                panic!("discarded token executed")
            })
            .is_err()
    );
    current.close().await;
    incoming.store.close().await.unwrap();
}

#[tokio::test]
async fn incompatible_primary_rows_are_rejected_before_current_shutdown() {
    let mut current = Workspace::new("Current note").await;
    let source = Workspace::new("Unreadable incoming note").await;
    source
        .store
        .write_connection()
        .call(|database| {
            database.execute("UPDATE blocks SET markdown_hlc='malformed revision'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let backup = current.directory.path().join("invalid.recovery");
    source.backup(&backup).await;
    let runtime = RecoveryRuntime::new(
        current.directory.path().to_owned(),
        WorkspaceShutdown::default(),
    );
    assert!(runtime.prepare(backup, &current.store).await.is_err());
    current
        .store
        .create_page("Current still writable".into())
        .await
        .unwrap();
    current.close().await;
    source.store.close().await.unwrap();
}

#[tokio::test]
async fn accepted_restore_survives_waiter_cancellation_and_exit_waits_for_publication() {
    let mut current = Workspace::new("Original note").await;
    let (source, backup) = incoming(current.directory.path()).await;
    let runtime = RecoveryRuntime::new(
        current.directory.path().to_owned(),
        WorkspaceShutdown::default(),
    );
    let preview = runtime.prepare(backup, &current.store).await.unwrap();
    // Preserve a real WAL-only final commit after the preview was accepted.
    let last_reader = rusqlite::Connection::open_with_flags(
        current.directory.path().join(DATABASE),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    last_reader
        .query_row("SELECT count(*) FROM pages", [], |row| row.get::<_, i64>(0))
        .unwrap();
    current
        .store
        .create_page("Last accepted write after preview".into())
        .await
        .unwrap();
    let closed = current.close().await;
    last_reader.close().unwrap();
    assert!(
        current
            .directory
            .path()
            .join("notes.db-wal")
            .metadata()
            .unwrap()
            .len()
            > 0
    );
    let (release, released) = tokio::sync::oneshot::channel();
    let executions = Arc::new(AtomicUsize::new(0));
    let actual = executions.clone();
    let completion = runtime
        .accept_restore(preview.token, move |prepared| async move {
            actual.fetch_add(1, Ordering::SeqCst);
            released.await.unwrap();
            restore_closed_workspace(closed, prepared).await
        })
        .unwrap();
    let waiter = tokio::spawn(wait_restore(completion));
    tokio::task::yield_now().await;
    waiter.abort();
    let exit = tokio::spawn(RecoveryRuntime::wait_before_exit(runtime.begin_exit()));
    tokio::task::yield_now().await;
    assert!(!exit.is_finished());
    assert!(
        runtime
            .accept_restore(preview.token, |_| async {
                panic!("second restore executed")
            })
            .is_err()
    );
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), exit)
        .await
        .unwrap()
        .unwrap();
    let completed = runtime.0.state.lock().unwrap().restoring.clone().unwrap();
    let result = wait_restore(completed).await.unwrap();
    assert_eq!(result.status, RecoveryRestoreStatus::Restored);
    assert!(result.restart_required);
    assert!(result.error.is_none());
    assert_eq!(executions.load(Ordering::SeqCst), 1);
    let before = prepare_recovery_backup(
        result.before_restore_backup_path.unwrap(),
        current.directory.path(),
    )
    .await
    .unwrap();
    assert_eq!(
        titles(&before.database_path()).await,
        ["Last accepted write after preview", "Original note"]
    );
    assert_eq!(
        titles(&current.directory.path().join(DATABASE)).await,
        ["Restored note"]
    );
    assert!(!current.directory.path().join("notes.db-wal").exists());
    assert!(!current.directory.path().join("notes.db-shm").exists());
    let reopened = NotesStore::open(current.directory.path().join(DATABASE))
        .await
        .unwrap();
    reopened
        .create_page("Fresh runtime works".into())
        .await
        .unwrap();
    reopened.close().await.unwrap();
    source.store.close().await.unwrap();
}

#[tokio::test]
async fn safety_backup_failure_keeps_current_content_and_forces_restart_after_drain() {
    let mut current = Workspace::new("Do not replace me").await;
    let (source, backup) = incoming(current.directory.path()).await;
    let prepared = prepare_ready_recovery(&backup, current.directory.path(), uuid::Uuid::nil())
        .await
        .unwrap();
    let bytes = b"required original body";
    let hash = BlobHash::digest(bytes);
    current
        .blobs
        .install_reader(bytes.as_slice(), hash, bytes.len() as u64)
        .unwrap();
    let page = db::list_pages(current.store.read_connection(), 1)
        .await
        .unwrap()[0]
        .uuid;
    current
        .store
        .create_attachment(
            notes_core::AttachmentOwner::Page(page),
            hash,
            "missing.png".into(),
            "image/png".into(),
            bytes.len() as u64,
        )
        .await
        .unwrap();
    assert!(current.blobs.remove_verified(hash, 1024).unwrap());
    let closed = current.close().await;
    let result = restore_closed_workspace(closed, prepared).await;
    assert_eq!(result.status, RecoveryRestoreStatus::Unchanged);
    assert!(result.restart_required);
    assert!(result.before_restore_backup_path.is_none());
    assert!(
        result
            .error
            .unwrap()
            .contains("Before-restore backup failed")
    );
    assert_eq!(
        titles(&current.directory.path().join(DATABASE)).await,
        ["Do not replace me"]
    );
    source.store.close().await.unwrap();
}

#[tokio::test]
async fn a_changed_prepared_file_fails_after_the_safety_backup_without_publishing() {
    let mut current = Workspace::new("Keep original").await;
    let (source, backup) = incoming(current.directory.path()).await;
    let prepared = prepare_ready_recovery(&backup, current.directory.path(), uuid::Uuid::nil())
        .await
        .unwrap();
    OpenOptions::new()
        .append(true)
        .open(&prepared.database.path)
        .unwrap()
        .write_all(b"changed after preparation")
        .unwrap();
    let result = restore_closed_workspace(current.close().await, prepared).await;
    assert_eq!(result.status, RecoveryRestoreStatus::Unchanged);
    assert!(result.restart_required);
    assert!(result.before_restore_backup_path.is_some());
    assert!(result.error.unwrap().contains("size changed"));
    assert_eq!(
        titles(&current.directory.path().join(DATABASE)).await,
        ["Keep original"]
    );
    source.store.close().await.unwrap();
}

#[tokio::test]
async fn a_busy_current_checkpoint_cannot_replace_the_database_or_retire_wal() {
    let mut current = Workspace::new("Original before read mark").await;
    let (source, backup) = incoming(current.directory.path()).await;
    let prepared = prepare_ready_recovery(&backup, current.directory.path(), uuid::Uuid::nil())
        .await
        .unwrap();
    let reader = rusqlite::Connection::open_with_flags(
        current.directory.path().join(DATABASE),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    reader
        .execute_batch("BEGIN; SELECT count(*) FROM pages;")
        .unwrap();
    current
        .store
        .create_page("Original after read mark".into())
        .await
        .unwrap();
    let result = restore_closed_workspace(current.close().await, prepared).await;
    assert_eq!(result.status, RecoveryRestoreStatus::Unchanged);
    assert!(result.before_restore_backup_path.is_some());
    assert!(result.error.unwrap().contains("checkpoint is busy"));
    assert!(
        current
            .directory
            .path()
            .join("notes.db-wal")
            .metadata()
            .unwrap()
            .len()
            > 0
    );
    reader.execute_batch("ROLLBACK").unwrap();
    reader.close().unwrap();
    assert_eq!(
        titles(&current.directory.path().join(DATABASE)).await,
        ["Original after read mark", "Original before read mark"]
    );
    source.store.close().await.unwrap();
}

#[tokio::test]
async fn directory_sync_failure_is_reported_after_atomic_publication_not_as_unchanged() {
    fn fail_sync(path: &Path) -> std::result::Result<(), notes_blob::BlobStoreError> {
        Err(notes_blob::BlobStoreError::Io {
            operation: "injected parent sync",
            path: path.to_owned(),
            source: std::io::Error::other("injected directory sync failure"),
        })
    }
    let mut current = Workspace::new("Original").await;
    let (source, backup) = incoming(current.directory.path()).await;
    let prepared = prepare_ready_recovery(&backup, current.directory.path(), uuid::Uuid::nil())
        .await
        .unwrap();
    let closed = current.close().await;
    let result = restore_closed_workspace_with_sync(closed, prepared, fail_sync).await;
    assert_eq!(
        result.status,
        RecoveryRestoreStatus::PublishedDurabilityUncertain
    );
    assert!(result.restart_required);
    assert!(result.before_restore_backup_path.is_some());
    assert!(result.error.unwrap().contains("was published"));
    assert_eq!(
        titles(&current.directory.path().join(DATABASE)).await,
        ["Restored note"]
    );
    source.store.close().await.unwrap();
}

#[tokio::test]
async fn panic_after_rename_reports_unknown_and_retains_the_before_restore_backup() {
    fn panic_after_publish(_: &Path) -> std::result::Result<(), notes_blob::BlobStoreError> {
        panic!("injected panic after atomic database publication");
    }
    let mut current = Workspace::new("Original before panic").await;
    let (source, backup) = incoming(current.directory.path()).await;
    let prepared = prepare_ready_recovery(&backup, current.directory.path(), uuid::Uuid::nil())
        .await
        .unwrap();
    let result =
        restore_closed_workspace_with_sync(current.close().await, prepared, panic_after_publish)
            .await;
    assert_eq!(result.status, RecoveryRestoreStatus::Unknown);
    assert!(result.restart_required);
    let before = prepare_recovery_backup(
        result.before_restore_backup_path.unwrap(),
        current.directory.path(),
    )
    .await
    .unwrap();
    assert_eq!(
        titles(&before.database_path()).await,
        ["Original before panic"]
    );
    // Actual bytes establish why an unclassified panic cannot say unchanged.
    assert_eq!(
        titles(&current.directory.path().join(DATABASE)).await,
        ["Restored note"]
    );
    source.store.close().await.unwrap();
}

#[tokio::test]
async fn a_panicked_owned_restore_finishes_exit_waiters_with_an_unknown_terminal_result() {
    let mut current = Workspace::new("Original").await;
    let (source, backup) = incoming(current.directory.path()).await;
    let runtime = RecoveryRuntime::new(
        current.directory.path().to_owned(),
        WorkspaceShutdown::default(),
    );
    let preview = runtime.prepare(backup, &current.store).await.unwrap();
    let completion = runtime
        .accept_restore(preview.token, |prepared| async move {
            let _plan = prepared;
            panic!("injected owned task panic");
        })
        .unwrap();
    let exit = runtime.begin_exit();
    let result = tokio::time::timeout(Duration::from_secs(5), wait_restore(completion))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.status, RecoveryRestoreStatus::Unknown);
    assert!(result.restart_required);
    tokio::time::timeout(
        Duration::from_secs(5),
        RecoveryRuntime::wait_before_exit(exit),
    )
    .await
    .unwrap();
    current.close().await;
    source.store.close().await.unwrap();
}

#[tokio::test]
async fn trusted_backup_listing_omits_staging_and_android_rejects_outside_paths() {
    let mut current = Workspace::new("Original").await;
    let directory = current.directory.path().join("backups");
    fs::create_dir(&directory).unwrap();
    let backup = directory.join("saved.recovery");
    current.backup(&backup).await;
    fs::create_dir(directory.join(".tangleaf-recovery-private")).unwrap();
    fs::create_dir(directory.join("incomplete")).unwrap();
    let entries = list_backups(&directory).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, backup.to_string_lossy());
    require_local_backup(&backup, &directory).unwrap();
    let outside = tempfile::tempdir().unwrap();
    assert!(require_local_backup(outside.path(), &directory).is_err());
    let runtime = RecoveryRuntime::new(
        current.directory.path().to_owned(),
        WorkspaceShutdown::default(),
    );
    runtime.begin_exit();
    assert!(runtime.prepare(backup, &current.store).await.is_err());
    current.close().await;
}

#[cfg(unix)]
#[test]
fn sidecar_retirement_rejects_dangling_links_and_nonempty_wal_then_removes_only_regular_stale_files()
 {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join(DATABASE);
    let wal = directory.path().join("notes.db-wal");
    let shm = directory.path().join("notes.db-shm");
    fs::write(&database, b"checkpointed standalone database").unwrap();
    std::os::unix::fs::symlink(directory.path().join("missing-target"), &wal).unwrap();
    assert!(
        !wal.try_exists().unwrap(),
        "a dangling symlink looks absent to try_exists"
    );
    assert!(retire_old_sidecars(&database).is_err());
    assert!(fs::symlink_metadata(&wal).unwrap().file_type().is_symlink());
    fs::remove_file(&wal).unwrap();
    fs::write(&wal, b"unexpected committed WAL frames").unwrap();
    assert!(retire_old_sidecars(&database).is_err());
    assert_eq!(fs::read(&wal).unwrap(), b"unexpected committed WAL frames");
    fs::write(&wal, b"").unwrap();
    fs::write(&shm, b"old shared memory").unwrap();
    retire_old_sidecars(&database).unwrap();
    assert!(!wal.exists());
    assert!(!shm.exists());
    assert_eq!(
        fs::read(&database).unwrap(),
        b"checkpointed standalone database"
    );
}
