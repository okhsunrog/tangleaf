//! Full recovery backups. Portable `.notes` export remains a separate format.
use anyhow::{Context, Result, ensure};
use notes_blob::{BlobHash, BlobStore};
use notes_core::store::{
    NotesStore, backup_closed_database, inspect_database_backup, required_attachment_blobs,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const FORMAT: &str = "tangleaf-recovery";
const VERSION: u32 = 1;
mod recovery;
pub(crate) use recovery::RecoveryRuntime;
pub use recovery::{
    RecoveryBackupEntry, RecoveryRestorePreview, RecoveryRestoreResult, RecoveryRestoreStatus,
};
const MANIFEST: &str = "manifest.json";
const DATABASE: &str = "notes.db";
const MAX_MANIFEST_BYTES: u64 = 32 * 1024 * 1024;
const MAX_BLOBS: usize = 100_000;
const MAX_DATABASE_BYTES: u64 = 64 * 1024 * 1024 * 1024;
#[cfg(not(target_os = "android"))]
const MAX_BLOB_BYTES: u64 = 8 * 1024 * 1024 * 1024;
#[cfg(target_os = "android")]
const MAX_BLOB_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RecoveryManifest {
    pub format: String,
    pub version: u32,
    pub created_at: i64,
    pub database: DatabaseEntry,
    pub blobs: Vec<BlobEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DatabaseEntry {
    pub file: String,
    pub hash: BlobHash,
    pub size: u64,
    pub schema_version: i64,
    pub workspace_uuid: uuid::Uuid,
    pub commit_version: i64,
    pub workspace_epoch: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlobEntry {
    pub hash: BlobHash,
    pub size: u64,
}

/// Owns validated private staging files. It never replaces a running workspace.
pub struct VerifiedRecoveryBackup {
    directory: tempfile::TempDir,
    manifest: RecoveryManifest,
}

impl VerifiedRecoveryBackup {
    pub fn database_path(&self) -> PathBuf {
        self.directory.path().join(DATABASE)
    }
    pub fn manifest(&self) -> &RecoveryManifest {
        &self.manifest
    }

    /// Install immutable bodies additively. Existing corrupt bytes cause an
    /// error and are never overwritten. The restore coordinator must keep its
    /// destination GC lease until the replacement database is published.
    pub fn install_blobs(&self, destination: &BlobStore) -> Result<()> {
        let source = BlobStore::new(self.directory.path());
        for blob in &self.manifest.blobs {
            let verified = source.open_verified(blob.hash, blob.size)?;
            ensure!(verified.blob.size == blob.size, "staged blob size mismatch");
            destination.install_reader(verified.into_file(), blob.hash, blob.size)?;
        }
        Ok(())
    }
}

/// Capture SQLite first under a deletion lease, then copy the complete verified
/// canonical body set. Extra immutable orphans conservatively preserve bodies
/// referenced only by retained history or pending/quarantined publications.
pub async fn create_recovery_backup(
    store: &NotesStore,
    blobs: &BlobStore,
    destination: impl AsRef<Path>,
) -> Result<PathBuf> {
    create_recovery_backup_from(CaptureSource::Live(store), blobs, destination.as_ref()).await
}

/// Capture a drained workspace before physical replacement. The coordinator
/// must retain its application lock and block new database users throughout
/// capture and publication. Use the original BlobStore to share its GC lease.
pub async fn create_recovery_backup_offline(
    database: impl AsRef<Path>,
    blobs: &BlobStore,
    destination: impl AsRef<Path>,
) -> Result<PathBuf> {
    create_recovery_backup_from(
        CaptureSource::Closed(database.as_ref()),
        blobs,
        destination.as_ref(),
    )
    .await
}

enum CaptureSource<'a> {
    Live(&'a NotesStore),
    Closed(&'a Path),
}

async fn create_recovery_backup_from(
    source: CaptureSource<'_>,
    blobs: &BlobStore,
    destination: &Path,
) -> Result<PathBuf> {
    let destination = destination.to_owned();
    ensure!(
        !destination.try_exists()?,
        "backup destination already exists"
    );
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    require_directory(parent)?;
    let lease = blobs.lease_deletions();
    let staging = tempfile::Builder::new()
        .prefix(".tangleaf-recovery-")
        .tempdir_in(parent)?;
    let path = staging.path().join(DATABASE);
    let database = match source {
        CaptureSource::Live(store) => store.backup_database(path).await?,
        CaptureSource::Closed(source) => backup_closed_database(source, path).await?,
    };
    ensure!(
        database.size <= MAX_DATABASE_BYTES,
        "recovery database exceeds size limit"
    );
    let required = captured_references(&database.path).await?;
    let blobs = blobs.clone();
    tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        let _lease = lease;
        let destination_store = BlobStore::new(staging.path());
        fs::create_dir(staging.path().join("blobs"))?;
        let hashes = canonical_blobs(&blobs, true)?;
        let mut entries = Vec::with_capacity(hashes.len());
        let mut total = 0_u64;
        for hash in hashes {
            let verified = blobs.open_verified(hash, MAX_BLOB_BYTES)?;
            let size = verified.blob.size;
            total = total
                .checked_add(size)
                .context("backup blob byte count overflow")?;
            ensure!(total <= MAX_BLOB_BYTES, "recovery blobs exceed size limit");
            destination_store.install_reader(verified.into_file(), hash, size)?;
            entries.push(BlobEntry { hash, size });
        }
        let manifest = RecoveryManifest {
            format: FORMAT.into(),
            version: VERSION,
            created_at: chrono::Utc::now().timestamp(),
            database: DatabaseEntry {
                file: DATABASE.into(),
                hash: database.hash,
                size: database.size,
                schema_version: database.schema_version,
                workspace_uuid: database.workspace_uuid,
                commit_version: database.commit_version,
                workspace_epoch: database.workspace_epoch,
            },
            blobs: entries,
        };
        let available = validate_manifest(&manifest)?;
        validate_references(&required, &available)?;
        let encoded = serde_json::to_vec_pretty(&manifest)?;
        ensure!(
            encoded.len() as u64 <= MAX_MANIFEST_BYTES,
            "recovery manifest exceeds size limit"
        );
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(staging.path().join(MANIFEST))?;
        file.write_all(&encoded)?;
        file.sync_all()?;
        notes_blob::sync_directory(&staging.path().join("blobs"))?;
        notes_blob::sync_directory(staging.path())?;
        publish_directory(staging.path(), &destination)?;
        let _ = staging.keep();
        notes_blob::sync_directory(
            destination
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?;
        Ok(destination)
    })
    .await?
}

async fn captured_references(path: &Path) -> Result<BTreeMap<BlobHash, u64>> {
    let connection = notes_core::Connection::open_read_only(path).await?;
    let references = required_attachment_blobs(&connection).await;
    let close = connection.close().await;
    let references = references?;
    close?;
    Ok(references)
}

/// Validate/copy an untrusted recovery directory into private staging before
/// opening its SQLite file. No current database or blob body is replaced.
pub async fn prepare_recovery_backup(
    source: impl AsRef<Path>,
    staging_parent: impl AsRef<Path>,
) -> Result<VerifiedRecoveryBackup> {
    let source = source.as_ref().to_owned();
    let parent = staging_parent.as_ref().to_owned();
    let prepared = tokio::task::spawn_blocking(move || -> Result<VerifiedRecoveryBackup> {
        require_directory(&source)?;
        require_directory(&parent)?;
        let names = fs::read_dir(&source)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<std::io::Result<BTreeSet<_>>>()?;
        ensure!(
            names
                == [DATABASE.into(), MANIFEST.into(), "blobs".into()]
                    .into_iter()
                    .collect(),
            "unexpected recovery directory entries"
        );
        let mut input = open_regular(&source.join(MANIFEST))?;
        ensure!(
            input.metadata()?.len() <= MAX_MANIFEST_BYTES,
            "recovery manifest exceeds size limit"
        );
        let mut encoded = Vec::new();
        Read::by_ref(&mut input)
            .take(MAX_MANIFEST_BYTES + 1)
            .read_to_end(&mut encoded)?;
        ensure!(
            encoded.len() as u64 <= MAX_MANIFEST_BYTES,
            "recovery manifest exceeds size limit"
        );
        let manifest: RecoveryManifest = serde_json::from_slice(&encoded)?;
        let available = validate_manifest(&manifest)?;
        let source_blobs = BlobStore::new(&source);
        ensure!(
            canonical_blobs(&source_blobs, false)? == available.keys().copied().collect(),
            "recovery blob tree does not match manifest"
        );
        let directory = tempfile::Builder::new()
            .prefix(".tangleaf-verified-recovery-")
            .tempdir_in(parent)?;
        copy_database(
            &source.join(DATABASE),
            &directory.path().join(DATABASE),
            &manifest.database,
        )?;
        let staged = BlobStore::new(directory.path());
        fs::create_dir(directory.path().join("blobs"))?;
        for blob in &manifest.blobs {
            let verified = source_blobs.open_verified(blob.hash, blob.size)?;
            ensure!(
                verified.blob.size == blob.size,
                "recovery blob size mismatch"
            );
            staged.install_reader(verified.into_file(), blob.hash, blob.size)?;
        }
        Ok(VerifiedRecoveryBackup {
            directory,
            manifest,
        })
    })
    .await??;
    let database = inspect_database_backup(prepared.database_path()).await?;
    let expected = &prepared.manifest.database;
    ensure!(
        database.hash == expected.hash
            && database.size == expected.size
            && database.schema_version == expected.schema_version
            && database.workspace_uuid == expected.workspace_uuid
            && database.commit_version == expected.commit_version
            && database.workspace_epoch == expected.workspace_epoch,
        "recovery database metadata differs from manifest"
    );
    validate_references(
        &captured_references(&database.path).await?,
        &validate_manifest(&prepared.manifest)?,
    )?;
    Ok(prepared)
}

fn validate_manifest(manifest: &RecoveryManifest) -> Result<BTreeMap<BlobHash, u64>> {
    ensure!(
        manifest.format == FORMAT && manifest.version == VERSION,
        "unsupported recovery backup format"
    );
    let database = &manifest.database;
    ensure!(database.file == DATABASE, "invalid recovery database path");
    ensure!(
        database.size > 0 && database.size <= MAX_DATABASE_BYTES,
        "invalid recovery database size"
    );
    ensure!(
        database.schema_version > 0
            && database.schema_version <= notes_core::db::supported_schema_version(),
        "unsupported recovery database schema version"
    );
    ensure!(
        !database.workspace_uuid.is_nil()
            && database.commit_version >= 0
            && database.workspace_epoch >= 0,
        "invalid recovery database metadata"
    );
    ensure!(
        manifest.blobs.len() <= MAX_BLOBS,
        "recovery backup has too many blobs"
    );
    ensure!(
        manifest
            .blobs
            .windows(2)
            .all(|entries| entries[0].hash < entries[1].hash),
        "recovery blobs must be unique and ordered"
    );
    let mut total = 0_u64;
    let mut entries = BTreeMap::new();
    for blob in &manifest.blobs {
        total = total
            .checked_add(blob.size)
            .context("recovery blob byte count overflow")?;
        ensure!(total <= MAX_BLOB_BYTES, "recovery blobs exceed size limit");
        entries.insert(blob.hash, blob.size);
    }
    Ok(entries)
}

fn validate_references(
    required: &BTreeMap<BlobHash, u64>,
    available: &BTreeMap<BlobHash, u64>,
) -> Result<()> {
    for (hash, size) in required {
        ensure!(
            available.get(hash) == Some(size),
            "missing or mismatched retained attachment blob {hash}"
        );
    }
    Ok(())
}

fn canonical_blobs(store: &BlobStore, allow_incoming: bool) -> Result<BTreeSet<BlobHash>> {
    require_directory(store.root())?;
    let root = store.root().join("blobs");
    match fs::symlink_metadata(&root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    require_directory(&root)?;
    let mut hashes = BTreeSet::new();
    for shard in fs::read_dir(root)? {
        let shard = shard?;
        require_directory(&shard.path())?;
        let prefix = shard
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("non-UTF8 blob shard"))?;
        ensure!(
            prefix.len() == 2
                && prefix
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "noncanonical blob shard"
        );
        for entry in fs::read_dir(shard.path())? {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "unsafe blob entry"
            );
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("non-UTF8 blob name"))?;
            if allow_incoming && name.starts_with(".incoming-") {
                continue;
            }
            let hash: BlobHash = name.parse().context("noncanonical blob filename")?;
            ensure!(name.starts_with(&prefix), "blob is in the wrong shard");
            hashes.insert(hash);
            ensure!(hashes.len() <= MAX_BLOBS, "too many canonical blobs");
        }
    }
    Ok(hashes)
}

fn require_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "unsafe recovery directory"
    );
    Ok(())
}

fn open_regular(path: &Path) -> Result<File> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "unsafe recovery file"
    );
    #[cfg(unix)]
    let file = File::from(rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NOFOLLOW,
        rustix::fs::Mode::empty(),
    )?);
    #[cfg(not(unix))]
    let file = File::open(path)?;
    ensure!(file.metadata()?.is_file(), "unsafe open recovery file");
    Ok(file)
}

fn copy_database(source: &Path, destination: &Path, expected: &DatabaseEntry) -> Result<()> {
    let mut input = open_regular(source)?;
    ensure!(
        input.metadata()?.len() == expected.size,
        "recovery database size mismatch"
    );
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let mut hash = Sha256::new();
    let mut copied = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        copied = copied
            .checked_add(count as u64)
            .context("recovery database size overflow")?;
        ensure!(
            copied <= expected.size,
            "recovery database grew while copying"
        );
        hash.update(&buffer[..count]);
        output.write_all(&buffer[..count])?;
    }
    ensure!(
        copied == expected.size && BlobHash::from_bytes(hash.finalize().into()) == expected.hash,
        "recovery database hash mismatch"
    );
    output.sync_all()?;
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
fn publish_directory(source: &Path, destination: &Path) -> Result<()> {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        source,
        rustix::fs::CWD,
        destination,
        rustix::fs::RenameFlags::NOREPLACE,
    )?;
    Ok(())
}

#[cfg(windows)]
fn publish_directory(source: &Path, destination: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    let source = source.as_os_str().encode_wide().collect::<Vec<_>>();
    let destination = destination.as_os_str().encode_wide().collect::<Vec<_>>();
    ensure!(
        !source.contains(&0) && !destination.contains(&0),
        "backup path contains NUL"
    );
    let source = source.into_iter().chain(Some(0)).collect::<Vec<_>>();
    let destination = destination.into_iter().chain(Some(0)).collect::<Vec<_>>();
    // MoveFileW fails if the destination exists; paths are terminated UTF-16.
    if unsafe {
        windows_sys::Win32::Storage::FileSystem::MoveFileW(source.as_ptr(), destination.as_ptr())
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    windows
)))]
fn publish_directory(_source: &Path, _destination: &Path) -> Result<()> {
    anyhow::bail!("atomic directory backup publication is unavailable on this platform")
}

#[cfg(test)]
mod tests;
