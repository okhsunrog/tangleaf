//! Prepared, one-shot physical replacement after the shared close barrier.

use super::*;
use crate::workspace::{ClosedWorkspace, WorkspaceShutdown};
use futures::FutureExt;
use notes_core::store::{DatabaseBackup, validate_recovery_content};
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryBackupEntry {
    pub path: String,
    pub created_at: i64,
    pub database_bytes: u64,
    pub blob_count: u32,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryRestorePreview {
    pub token: uuid::Uuid,
    pub created_at: i64,
    pub workspace_uuid: uuid::Uuid,
    pub current_workspace_uuid: uuid::Uuid,
    pub workspace_differs: bool,
    pub commit_version: i64,
    pub schema_version: i64,
    pub database_bytes: u64,
    pub blob_count: u32,
    pub blob_bytes: u64,
    pub page_count: u64,
    pub block_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryRestoreStatus {
    Unchanged,
    Restored,
    PublishedDurabilityUncertain,
    Unknown,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryRestoreResult {
    pub status: RecoveryRestoreStatus,
    pub restart_required: bool,
    pub before_restore_backup_path: Option<String>,
    pub error: Option<String>,
    pub warning: Option<String>,
}

struct PreparedRecovery {
    verified: VerifiedRecoveryBackup,
    _ready_directory: tempfile::TempDir,
    database: DatabaseBackup,
    preview: RecoveryRestorePreview,
    before_backup: Arc<Mutex<Option<PathBuf>>>,
}

type RestoreCompletion = watch::Receiver<Option<RecoveryRestoreResult>>;

struct RuntimeState {
    exiting: bool,
    preparing: bool,
    prepared: Option<PreparedRecovery>,
    restoring: Option<RestoreCompletion>,
}

struct RuntimeInner {
    directory: PathBuf,
    shutdown: WorkspaceShutdown,
    state: Mutex<RuntimeState>,
}

#[derive(Clone)]
pub(crate) struct RecoveryRuntime(Arc<RuntimeInner>);

impl RecoveryRuntime {
    pub(crate) fn new(directory: PathBuf, shutdown: WorkspaceShutdown) -> Self {
        Self(Arc::new(RuntimeInner {
            directory,
            shutdown,
            state: Mutex::new(RuntimeState {
                exiting: false,
                preparing: false,
                prepared: None,
                restoring: None,
            }),
        }))
    }

    pub(crate) async fn list_backups(&self) -> Result<Vec<RecoveryBackupEntry>> {
        let directory = self.0.directory.join("backups");
        tokio::task::spawn_blocking(move || list_backups(&directory)).await?
    }

    pub(crate) async fn prepare(
        &self,
        source: PathBuf,
        store: &NotesStore,
    ) -> Result<RecoveryRestorePreview> {
        {
            let mut state = self
                .0
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            ensure!(
                !state.exiting && state.restoring.is_none(),
                "workspace shutdown or restore has already started"
            );
            ensure!(!state.preparing, "recovery preparation is already running");
            state.preparing = true;
            state.prepared = None;
        }
        let owner = self.clone();
        let store = store.clone();
        tokio::spawn(async move {
            let work = async {
                #[cfg(target_os = "android")]
                require_local_backup(&source, &owner.0.directory.join("backups"))?;
                let current = notes_core::db::workspace_uuid(store.read_connection()).await?;
                prepare_ready_recovery(&source, &owner.0.directory, current).await
            };
            let prepared = AssertUnwindSafe(work)
                .catch_unwind()
                .await
                .map_err(|_| anyhow::anyhow!("recovery preparation panicked"))
                .and_then(|result| result);
            let mut state = owner
                .0
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.preparing = false;
            let prepared = prepared?;
            ensure!(
                !state.exiting && state.restoring.is_none(),
                "workspace closed while recovery was preparing"
            );
            let preview = prepared.preview.clone();
            state.prepared = Some(prepared);
            Ok(preview)
        })
        .await?
    }

    pub(crate) fn discard(&self, token: uuid::Uuid) -> Result<()> {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        ensure!(
            state
                .prepared
                .as_ref()
                .is_some_and(|prepared| prepared.preview.token == token),
            "recovery token is missing or no longer current"
        );
        state.prepared = None;
        Ok(())
    }

    pub(crate) async fn commit(
        &self,
        token: uuid::Uuid,
        app: &tauri::AppHandle,
    ) -> Result<RecoveryRestoreResult> {
        let owner = self.clone();
        let app = app.clone();
        let completion = self.accept_restore(token, move |prepared| async move {
            owner.0.shutdown.begin(app);
            match owner.0.shutdown.wait().await {
                Ok(Some(closed)) => restore_closed_workspace(closed, prepared).await,
                Ok(None) => unchanged(
                    "No initialized workspace is available to restore",
                    None,
                    true,
                ),
                Err(error) => unchanged(format!("Workspace drain failed: {error}"), None, true),
            }
        })?;
        wait_restore(completion).await
    }

    fn accept_restore<F, W>(&self, token: uuid::Uuid, work: F) -> Result<RestoreCompletion>
    where
        F: FnOnce(PreparedRecovery) -> W + Send + 'static,
        W: std::future::Future<Output = RecoveryRestoreResult> + Send + 'static,
    {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        ensure!(
            !state.exiting && state.restoring.is_none(),
            "workspace shutdown or restore has already started"
        );
        ensure!(!state.preparing, "recovery preparation is still running");
        ensure!(
            state
                .prepared
                .as_ref()
                .is_some_and(|prepared| prepared.preview.token == token),
            "recovery token is missing, consumed, or no longer current"
        );
        let prepared = state.prepared.take().unwrap();
        let before_backup = prepared.before_backup.clone();
        let (finished, completion) = watch::channel(None);
        state.restoring = Some(completion.clone());
        // The accepted job owns staging and completion even if IPC disappears.
        tokio::spawn(async move {
            let outcome = AssertUnwindSafe(async move { work(prepared).await })
                .catch_unwind()
                .await
                .unwrap_or_else(|_| {
                    unknown(
                        "Recovery task panicked; publication state is unknown",
                        before_backup
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .clone(),
                    )
                });
            finished.send_replace(Some(outcome));
        });
        Ok(completion)
    }

    /// The same gate decides whether Exit or restore was accepted first. Exit
    /// waits an accepted restore through backup/rename/directory sync, so it
    /// cannot terminate the process while the closed token is still in use.
    pub(crate) fn begin_exit(&self) -> Option<RestoreCompletion> {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.exiting = true;
        state.prepared = None;
        state.restoring.clone()
    }

    pub(crate) async fn wait_before_exit(completion: Option<RestoreCompletion>) {
        if let Some(completion) = completion
            && let Err(error) = wait_restore(completion).await
        {
            tracing::error!(?error, "Recovery completion unavailable before exit");
        }
    }
}

async fn wait_restore(mut completion: RestoreCompletion) -> Result<RecoveryRestoreResult> {
    loop {
        if let Some(outcome) = completion.borrow_and_update().clone() {
            return Ok(outcome);
        }
        completion
            .changed()
            .await
            .context("recovery completion channel closed")?;
    }
}

async fn prepare_ready_recovery(
    source: &Path,
    parent: &Path,
    current: uuid::Uuid,
) -> Result<PreparedRecovery> {
    let verified = prepare_recovery_backup(source, parent).await?;
    let ready_directory = tempfile::Builder::new()
        .prefix(".tangleaf-ready-recovery-")
        .tempdir_in(parent)?;
    let source = verified.database_path();
    let expected = verified.manifest.database.clone();
    let path = ready_directory.path().join("initialized.db");
    let working = path.clone();
    tokio::task::spawn_blocking(move || copy_database(&source, &working, &expected)).await??;
    let store = NotesStore::open(&path)
        .await
        .context("opening recovery with the current application")?;
    let validated = async {
        let counts = validate_recovery_content(store.read_connection()).await?;
        validate_references(
            &required_attachment_blobs(store.read_connection()).await?,
            &validate_manifest(&verified.manifest)?,
        )?;
        Ok::<_, anyhow::Error>(counts)
    }
    .await;
    let close = store.close().await;
    let counts = validated?;
    close.context("closing initialized recovery database")?;
    let database = backup_closed_database(path, ready_directory.path().join(DATABASE)).await?;
    let manifest = &verified.manifest;
    let preview = RecoveryRestorePreview {
        token: uuid::Uuid::new_v4(),
        created_at: manifest.created_at,
        workspace_uuid: database.workspace_uuid,
        current_workspace_uuid: current,
        workspace_differs: database.workspace_uuid != current,
        commit_version: database.commit_version,
        schema_version: database.schema_version,
        database_bytes: database.size,
        blob_count: u32::try_from(manifest.blobs.len())?,
        blob_bytes: manifest.blobs.iter().try_fold(0_u64, |total, blob| {
            total
                .checked_add(blob.size)
                .context("recovery blob size overflow")
        })?,
        page_count: counts.pages,
        block_count: counts.blocks,
    };
    Ok(PreparedRecovery {
        verified,
        _ready_directory: ready_directory,
        database,
        preview,
        before_backup: Arc::new(Mutex::new(None)),
    })
}

fn list_backups(directory: &Path) -> Result<Vec<RecoveryBackupEntry>> {
    if !directory.try_exists()? {
        return Ok(Vec::new());
    }
    require_directory(directory)?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if !entry.file_type()?.is_dir() || entry.file_type()?.is_symlink() {
            continue;
        }
        let read = || -> Result<_> {
            let path = entry.path();
            let mut file = open_regular(&path.join(MANIFEST))?;
            ensure!(
                file.metadata()?.len() <= MAX_MANIFEST_BYTES,
                "recovery manifest exceeds size limit"
            );
            let mut encoded = Vec::new();
            Read::by_ref(&mut file)
                .take(MAX_MANIFEST_BYTES + 1)
                .read_to_end(&mut encoded)?;
            let manifest: RecoveryManifest = serde_json::from_slice(&encoded)?;
            validate_manifest(&manifest)?;
            Ok(RecoveryBackupEntry {
                path: path.to_string_lossy().into_owned(),
                created_at: manifest.created_at,
                database_bytes: manifest.database.size,
                blob_count: u32::try_from(manifest.blobs.len())?,
            })
        };
        if let Ok(entry) = read() {
            entries.push(entry);
        }
    }
    entries.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(entries)
}

#[cfg(any(target_os = "android", test))]
fn require_local_backup(source: &Path, directory: &Path) -> Result<()> {
    require_directory(source)?;
    let parent = directory.canonicalize()?;
    ensure!(
        source.canonicalize()?.parent() == Some(parent.as_path()),
        "this platform can restore only backups in the application backup directory"
    );
    Ok(())
}

async fn restore_closed_workspace(
    closed: Arc<ClosedWorkspace>,
    prepared: PreparedRecovery,
) -> RecoveryRestoreResult {
    restore_closed_workspace_with_sync(closed, prepared, notes_blob::sync_directory).await
}

async fn restore_closed_workspace_with_sync(
    closed: Arc<ClosedWorkspace>,
    prepared: PreparedRecovery,
    sync_parent: fn(&Path) -> std::result::Result<(), notes_blob::BlobStoreError>,
) -> RecoveryRestoreResult {
    let directory = closed
        .database_path()
        .parent()
        .unwrap_or(Path::new("."))
        .join("backups");
    let destination = directory.join(format!(
        "before-restore-{}-{}.recovery",
        chrono::Utc::now().format("%Y%m%d-%H%M%S"),
        uuid::Uuid::new_v4()
    ));
    let lease = closed.blobs().lease_deletions();
    let capture = async {
        fs::create_dir_all(&directory)?;
        require_directory(&directory)?;
        create_recovery_backup_offline(closed.database_path(), closed.blobs(), &destination).await
    }
    .await;
    let backup = match capture {
        Ok(path) => path,
        Err(error) => {
            let mut result = unchanged(
                format!("Before-restore backup failed: {error:#}"),
                destination
                    .try_exists()
                    .ok()
                    .filter(|exists| *exists)
                    .map(|_| destination),
                true,
            );
            result.warning = closed.warning.clone();
            return result;
        }
    };
    let before = backup.to_string_lossy().into_owned();
    *prepared
        .before_backup
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = Some(backup.clone());
    let warning = closed.warning.clone();
    let publication = tokio::task::spawn_blocking(move || {
        let _lease = lease;
        prepared.verified.install_blobs(closed.blobs())?;
        replace_database_with_sync(&closed, &prepared.database, sync_parent)
    })
    .await;
    match publication {
        Ok(Ok(sync_error)) => RecoveryRestoreResult {
            status: if sync_error.is_some() {
                RecoveryRestoreStatus::PublishedDurabilityUncertain
            } else {
                RecoveryRestoreStatus::Restored
            },
            restart_required: true,
            before_restore_backup_path: Some(before),
            error: sync_error,
            warning,
        },
        Ok(Err(error)) => {
            let mut result = unchanged(format!("{error:#}"), Some(backup), true);
            result.warning = warning;
            result
        }
        Err(error) => {
            let mut result = unknown(
                format!("Recovery publication task failed; publication state is unknown: {error}"),
                Some(backup),
            );
            result.warning = warning;
            result
        }
    }
}

fn unknown(error: impl Into<String>, backup: Option<PathBuf>) -> RecoveryRestoreResult {
    let mut result = unchanged(error, backup, true);
    result.status = RecoveryRestoreStatus::Unknown;
    result
}

fn unchanged(
    error: impl Into<String>,
    backup: Option<PathBuf>,
    restart_required: bool,
) -> RecoveryRestoreResult {
    RecoveryRestoreResult {
        status: RecoveryRestoreStatus::Unchanged,
        restart_required,
        before_restore_backup_path: backup.map(|path| path.to_string_lossy().into_owned()),
        error: Some(error.into()),
        warning: None,
    }
}

/// All preparation and the safety backup precede this function. Keep a fully
/// fsynced same-filesystem replacement before touching the current WAL state.
fn replace_database_with_sync(
    closed: &ClosedWorkspace,
    incoming: &DatabaseBackup,
    sync_parent: fn(&Path) -> std::result::Result<(), notes_blob::BlobStoreError>,
) -> Result<Option<String>> {
    let current = closed.database_path();
    let parent = current
        .parent()
        .context("workspace database has no parent")?;
    require_directory(parent)?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".tangleaf-replacement-")
        .tempfile_in(parent)?;
    let mut source = open_regular(&incoming.path)?;
    ensure!(
        source.metadata()?.len() == incoming.size,
        "ready recovery file size changed"
    );
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut copied = 0_u64;
    loop {
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        copied += count as u64;
        ensure!(
            copied <= incoming.size,
            "ready recovery file grew while copying"
        );
        hash.update(&buffer[..count]);
        temporary.write_all(&buffer[..count])?;
    }
    ensure!(
        copied == incoming.size && BlobHash::from_bytes(hash.finalize().into()) == incoming.hash,
        "ready recovery file hash changed"
    );
    temporary.as_file().sync_all()?;
    seal_current_database(current)?;
    // persist() replaces atomically on each supported desktop/mobile host.
    let published = temporary
        .persist(current)
        .map_err(|error| error.error)
        .context("publishing recovery database")?;
    // The file was already synced before rename. A directory sync failure is
    // post-publication and cannot truthfully be reported as an unchanged DB.
    drop(published);
    Ok(sync_parent(parent).err().map(|error| {
        format!("Recovery database was published, but syncing its directory failed: {error}")
    }))
}

fn seal_current_database(path: &Path) -> Result<()> {
    open_regular(path)?;
    let database = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let seal = (|| -> Result<()> {
        notes_core::sqlite::policy::configure_existing_primary_writer(&database)?;
        let checkpoint: (i64, i64, i64) =
            database.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;
        ensure!(
            checkpoint.0 == 0 && checkpoint.1 == checkpoint.2,
            "current workspace checkpoint is busy or incomplete"
        );
        let mode: String =
            database.pragma_update_and_check(None, "journal_mode", "DELETE", |row| row.get(0))?;
        ensure!(
            mode == "delete",
            "current workspace could not become standalone"
        );
        Ok(())
    })();
    let close = database.close().map_err(|(connection, error)| {
        drop(connection);
        error
    });
    seal?;
    close.context("closing checkpointed current database")?;
    open_regular(path)?.sync_all()?;
    retire_old_sidecars(path)?;
    notes_blob::sync_directory(path.parent().unwrap_or(Path::new(".")))?;
    Ok(())
}

/// Only after the current main file is checkpointed, standalone and closed.
fn retire_old_sidecars(path: &Path) -> Result<()> {
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push(suffix);
        let sidecar = PathBuf::from(sidecar);
        match fs::symlink_metadata(&sidecar) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
            Ok(_) => {}
        }
        let file = open_regular(&sidecar)?;
        ensure!(
            suffix != "-wal" || file.metadata()?.len() == 0,
            "new WAL frames appeared after the closed-workspace checkpoint"
        );
        drop(file);
        fs::remove_file(&sidecar)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
