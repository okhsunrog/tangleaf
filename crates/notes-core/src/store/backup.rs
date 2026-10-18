use super::NotesStore;
use crate::sqlite::policy::configure_primary_writer;
use anyhow::{Context, Result, anyhow, ensure};
use notes_blob::BlobHash;
use rusqlite::backup::{Backup, StepResult};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const BACKUP_RETRY_LIMIT: Duration = Duration::from_secs(30);
const BACKUP_RETRY_PAUSE: Duration = Duration::from_millis(10);

/// Collect every external attachment body needed by the captured database,
/// including unmaterialized present intents, pending/quarantined publications,
/// and both retained history stacks.
pub async fn required_attachment_blobs(
    connection: &crate::Connection,
) -> Result<BTreeMap<BlobHash, u64>> {
    connection.call_domain(|database| -> Result<_> {
        let transaction = database.transaction()?;
        let mut required = BTreeMap::new();
        let mut add = |hash, size| -> Result<()> {
            ensure!(required.insert(hash, size).is_none_or(|old| old == size),
                "conflicting sizes for retained attachment blob {hash}");
            Ok(())
        };
        for row in transaction.prepare(
            "SELECT blob_hash,size FROM attachments
             UNION ALL SELECT blob_hash,size FROM attachment_lww WHERE present=1"
        )?
            .query_map([], |row| Ok((crate::db::row_blob_hash(row, 0)?, row.get::<_, i64>(1)?)))? {
            let (hash, size) = row?;
            add(hash, u64::try_from(size)?)?;
        }
        for envelope in transaction.prepare("SELECT envelope FROM sync_outbox ORDER BY rowid")?
            .query_map([], |row| row.get::<_, String>(0))? {
            let operation: crate::Op = crate::operation::decode_persisted_envelope(&envelope?)?;
            if let crate::OpKind::AttachmentAdd(body) = operation.kind { add(body.blob_hash, body.size)?; }
        }
        for payload in transaction.prepare(
            "SELECT forward_json FROM history_undo UNION ALL SELECT inverse_json FROM history_undo
             UNION ALL SELECT forward_json FROM history_redo UNION ALL SELECT inverse_json FROM history_redo"
        )?.query_map([], |row| row.get::<_, String>(0))? {
            for operation in crate::db::history::retained_operations(&payload?)? {
                if let crate::OpKind::AttachmentAdd(body) = operation { add(body.blob_hash, body.size)?; }
            }
        }
        transaction.commit()?;
        Ok(required)
    }).await
}

/// Inspect a standalone recovery file without migrating it or changing its
/// journal mode. Hosts validate file hashes before accepting its manifest.
pub async fn inspect_database_backup(path: impl AsRef<Path>) -> Result<DatabaseBackup> {
    let path = path.as_ref().to_owned();
    let connection = crate::Connection::open_read_only(&path).await?;
    let inspection = connection
        .call_domain(|database| -> Result<_> {
            let transaction = database.transaction()?;
            let metadata = read_metadata(&transaction)?;
            ensure!(
                metadata.schema_version > 0
                    && metadata.schema_version <= crate::db::supported_schema_version(),
                "unsupported recovery database schema version {}",
                metadata.schema_version
            );
            ensure!(
                !metadata.workspace_uuid.is_nil()
                    && metadata.commit_version >= 0
                    && metadata.workspace_epoch >= 0,
                "invalid recovery database metadata"
            );
            ensure!(
                transaction
                    .pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))?
                    == "delete",
                "recovery database must be standalone DELETE mode"
            );
            let checks = transaction
                .prepare("PRAGMA quick_check")?
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            ensure!(checks == ["ok"], "recovery database quick_check failed");
            ensure!(
                !transaction
                    .prepare("PRAGMA foreign_key_check")?
                    .exists([])?,
                "recovery database foreign_key_check failed"
            );
            transaction.commit()?;
            Ok(metadata)
        })
        .await;
    let close = connection.close().await;
    let metadata = inspection?;
    close?;
    tokio::task::spawn_blocking(move || {
        let file = File::open(&path)?;
        let size = file.metadata()?.len();
        let hash = hash_reader(file)?;
        Ok(DatabaseBackup {
            path,
            size,
            hash,
            workspace_uuid: metadata.workspace_uuid,
            commit_version: metadata.commit_version,
            workspace_epoch: metadata.workspace_epoch,
            schema_version: metadata.schema_version,
        })
    })
    .await?
}

fn hash_reader(mut reader: impl Read) -> Result<BlobHash> {
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(BlobHash::from_bytes(hash.finalize().into()))
}

/// A complete, verified SQLite file. External attachment files must still be
/// captured under a BlobStore deletion lease before calling it a full backup.
#[derive(Debug, Clone)]
pub struct DatabaseBackup {
    pub path: PathBuf,
    pub hash: BlobHash,
    pub size: u64,
    pub workspace_uuid: uuid::Uuid,
    pub commit_version: i64,
    pub workspace_epoch: i64,
    /// Application migration version (`PRAGMA user_version`).
    pub schema_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SnapshotMetadata {
    workspace_uuid: uuid::Uuid,
    commit_version: i64,
    workspace_epoch: i64,
    schema_version: i64,
}

struct BusyRetries {
    blocked_since: Option<Instant>,
    limit: Duration,
}

impl BusyRetries {
    fn observe(&mut self, step: StepResult, now: Instant) -> Result<()> {
        match step {
            StepResult::More | StepResult::Done => self.blocked_since = None,
            StepResult::Busy | StepResult::Locked => {
                let blocked_since = *self.blocked_since.get_or_insert(now);
                ensure!(
                    now.duration_since(blocked_since) < self.limit,
                    "SQLite backup exceeded its busy/locked retry deadline"
                );
            }
            _ => return Err(anyhow!("unexpected SQLite backup step result")),
        }
        Ok(())
    }
}

impl NotesStore {
    /// Capture all SQLite state, including history, cursor/outbox and native
    /// ink, into one standalone file using SQLite's online backup mechanism.
    /// The reader owns one stable read transaction. With `NotesStore::open`,
    /// the independent writer remains free; legacy `new` shares its worker.
    /// Publication never overwrites an existing path. The accepted worker job
    /// owns its temporary file and finishes cleanup even if its waiter drops.
    pub async fn backup_database(&self, destination: impl AsRef<Path>) -> Result<DatabaseBackup> {
        capture_database(self.read_connection(), destination.as_ref().to_owned()).await
    }
}

/// Capture an existing, closed workspace with a fresh read-only worker. This
/// includes committed WAL frames even when the last connection was a reader;
/// copying only the main database file would lose those accepted writes.
///
/// The host owns exclusive workspace admission for the entire operation. No
/// migration or source journal-mode assignment runs here. Once polled, an
/// owned task captures and explicitly closes/joins its worker even if its
/// caller drops the future; runtime destruction is not a completed drain.
pub async fn backup_closed_database(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
) -> Result<DatabaseBackup> {
    let source = source.as_ref().to_owned();
    let destination = destination.as_ref().to_owned();
    tokio::spawn(async move {
        let connection = crate::Connection::open_read_only(source).await?;
        capture_and_close(connection, destination).await
    })
    .await?
}

async fn capture_and_close(
    connection: crate::Connection,
    destination: PathBuf,
) -> Result<DatabaseBackup> {
    let capture = capture_database(&connection, destination).await;
    let close = connection.close().await;
    match (capture, close) {
        (Ok(backup), Ok(())) => Ok(backup),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error).context("closing offline backup reader"),
        (Err(capture), Err(close)) => Err(capture).context(format!(
            "offline backup reader also failed to close: {close:#}"
        )),
    }
}

async fn capture_database(
    connection: &crate::Connection,
    destination: PathBuf,
) -> Result<DatabaseBackup> {
    connection
        .call_domain(move |source| -> Result<_> {
            let transaction = source.transaction()?;
            let metadata = read_metadata(&transaction)?;
            write_snapshot(transaction, destination, metadata, BACKUP_RETRY_LIMIT)
        })
        .await
}

fn read_metadata(database: &rusqlite::Connection) -> Result<SnapshotMetadata> {
    Ok(SnapshotMetadata {
        workspace_uuid: database.query_row(
            "SELECT uuid FROM workspace WHERE singleton=1",
            [],
            |row| row.get(0),
        )?,
        commit_version: database.query_row(
            "SELECT version FROM store_state WHERE singleton=1",
            [],
            |row| row.get(0),
        )?,
        workspace_epoch: database.query_row(
            "SELECT workspace_epoch FROM store_state WHERE singleton=1",
            [],
            |row| row.get(0),
        )?,
        schema_version: database.pragma_query_value(None, "user_version", |row| row.get(0))?,
    })
}

fn write_snapshot(
    source: rusqlite::Transaction<'_>,
    destination: PathBuf,
    metadata: SnapshotMetadata,
    retry_limit: Duration,
) -> Result<DatabaseBackup> {
    ensure!(
        !destination.try_exists()?,
        "backup destination already exists"
    );
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::Builder::new()
        .prefix(".tangleaf-db-backup-")
        .tempfile_in(parent)?;
    {
        let mut target = rusqlite::Connection::open(temporary.path())?;
        configure_primary_writer(&target)?;
        {
            let backup = Backup::new(&source, &mut target)?;
            let mut retries = BusyRetries {
                blocked_since: None,
                limit: retry_limit,
            };
            loop {
                let step = backup.step(128)?;
                retries.observe(step, Instant::now())?;
                match step {
                    StepResult::Done => break,
                    StepResult::More => {}
                    StepResult::Busy | StepResult::Locked => std::thread::sleep(BACKUP_RETRY_PAUSE),
                    _ => return Err(anyhow!("unexpected SQLite backup step result")),
                }
            }
        }
        // The physical copy is complete. Do not retain its WAL read mark while
        // validating, hashing or syncing the standalone destination artifact.
        source.commit()?;
        let checks = target
            .prepare("PRAGMA quick_check")?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ensure!(checks == ["ok"], "SQLite backup quick_check failed");
        ensure!(
            !target.prepare("PRAGMA foreign_key_check")?.exists([])?,
            "SQLite backup foreign_key_check failed"
        );
        ensure!(
            read_metadata(&target)? == metadata,
            "SQLite backup metadata does not match its source snapshot"
        );
        let checkpoint: (i64, i64, i64) =
            target.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;
        ensure!(
            checkpoint.0 == 0 && checkpoint.1 == checkpoint.2,
            "SQLite backup destination checkpoint did not complete"
        );
        // A sealed artifact is a standalone database, not a live WAL writer.
        // Its read-only consumer must not need to create -wal/-shm siblings.
        let journal: String = target.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
        if journal != "delete" {
            target.pragma_update(None, "journal_mode", "DELETE")?;
        }
        ensure!(
            target.pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))?
                == "delete",
            "SQLite backup destination did not become standalone"
        );
        target
            .close()
            .map_err(|(target, error)| {
                drop(target);
                error
            })
            .context("closing SQLite backup destination")?;
    }
    temporary.as_file().sync_all()?;
    let size = temporary.as_file().metadata()?.len();
    temporary.as_file_mut().seek(SeekFrom::Start(0))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = temporary.as_file_mut().read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let hash = BlobHash::from_bytes(hasher.finalize().into());
    let published = publish_snapshot(temporary, &destination)?;
    published.sync_all()?;
    notes_blob::sync_directory(parent)?;
    Ok(DatabaseBackup {
        path: destination,
        hash,
        size,
        workspace_uuid: metadata.workspace_uuid,
        commit_version: metadata.commit_version,
        workspace_epoch: metadata.workspace_epoch,
        schema_version: metadata.schema_version,
    })
}

fn publish_snapshot(temporary: tempfile::NamedTempFile, destination: &Path) -> Result<File> {
    match temporary.persist_noclobber(destination) {
        Ok(file) => Ok(file),
        Err(error) => {
            let tempfile::PersistError { error, file } = error;
            drop(file);
            Err(error).context("publishing SQLite backup without overwrite")
        }
    }
}

#[cfg(test)]
mod tests;
