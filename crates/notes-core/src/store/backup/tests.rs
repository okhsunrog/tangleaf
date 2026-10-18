use super::*;
use crate::{BlockStyle, Connection, db};
use std::future::Future;
use std::sync::mpsc;
use std::task::{Context, Waker};

async fn state(connection: &Connection) -> serde_json::Value {
    let snapshot =
        crate::export_sync_snapshot(connection, crate::sync_cursor(connection).await.unwrap())
            .await
            .unwrap();
    let outbox = crate::pending_outbox(connection, u32::MAX).await.unwrap();
    let rows = connection.call(|db| {
        let history = db.prepare("SELECT id, forward_json, inverse_json FROM history_undo ORDER BY id")?
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let metadata = db.prepare("SELECT key,value FROM sync_meta ORDER BY key")?
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let ink = db.query_row("SELECT (SELECT COUNT(*) FROM ink_documents), (SELECT COUNT(*) FROM ink_records), (SELECT COUNT(*) FROM ink_chunks), (SELECT COUNT(*) FROM ink_history)", [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?)))?;
        Ok((history, metadata, ink))
    }).await.unwrap();
    serde_json::json!([snapshot, outbox, rows])
}

#[tokio::test]
async fn physical_backup_keeps_text_history_outbox_cursor_and_unpublished_ink() {
    use crate::ink::{InkBackground, InkDraftPatch, InkPoint, InkStroke, Store};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("notes.db");
    let store = NotesStore::open(&path).await.unwrap();
    crate::configure_sync(store.write_connection(), "https://sync.example")
        .await
        .unwrap();
    let page = store.create_page("Original".into()).await.unwrap().value;
    store
        .create_block(
            page.uuid,
            None,
            None,
            BlockStyle::Bullet,
            "Text with history".into(),
        )
        .await
        .unwrap();
    store
        .rename_page_if_revision(page.uuid, Some("Saved title".into()), page.title_revision)
        .await
        .unwrap();
    let ink_page = store
        .create_handwritten_note(Some("Unpublished drawing".into()))
        .await
        .unwrap()
        .value;
    let ink = Store::new(&path, ink_page.uuid);
    let stroke = InkStroke {
        id: uuid::Uuid::now_v7(),
        width: 2.0,
        points: (0..4)
            .map(|n| InkPoint {
                x: f64::from(n),
                y: f64::from(n * 2),
                pressure: 0.5,
                tilt_x: 0.0,
                tilt_y: 0.0,
                time: f64::from(n),
            })
            .collect(),
    };
    ink.patch(
        InkDraftPatch {
            order: vec![stroke.id],
            upserts: vec![stroke],
            background: InkBackground::Grid,
        },
        None,
    )
    .unwrap();
    store.write_connection().call(|db| { db.execute("INSERT INTO sync_meta(key,value) VALUES('last_server_seq','17') ON CONFLICT(key) DO UPDATE SET value=excluded.value", [])?; Ok(()) }).await.unwrap();
    let before = state(store.read_connection()).await;
    let expected_ink = serde_json::to_value(ink.read().unwrap().draft).unwrap();
    let backup = store
        .backup_database(directory.path().join("snapshot.db"))
        .await
        .unwrap();
    assert_eq!(
        backup.hash,
        BlobHash::digest(&std::fs::read(&backup.path).unwrap())
    );
    assert_eq!(backup.size, std::fs::metadata(&backup.path).unwrap().len());
    let reader = Connection::open_read_only(&backup.path).await.unwrap();
    assert_eq!(state(&reader).await, before);
    assert!(
        crate::pending_outbox(&reader, u32::MAX)
            .await
            .unwrap()
            .len()
            >= 4
    );
    let copied_metadata = reader.call_domain(|db| read_metadata(db)).await.unwrap();
    assert_eq!(backup.workspace_uuid, copied_metadata.workspace_uuid);
    assert_eq!(backup.commit_version, copied_metadata.commit_version);
    assert_eq!(backup.workspace_epoch, copied_metadata.workspace_epoch);
    assert_eq!(backup.schema_version, copied_metadata.schema_version);
    reader.close().await.unwrap();
    // Keep the published backup immutable; exercise a prospective restore copy.
    let restored = directory.path().join("restored.db");
    std::fs::copy(&backup.path, &restored).unwrap();
    let restored_store = NotesStore::open(&restored).await.unwrap();
    assert_eq!(
        serde_json::to_value(Store::new(&restored, ink_page.uuid).read().unwrap().draft).unwrap(),
        expected_ink
    );
    restored_store.close().await.unwrap();
    assert_eq!(state(store.read_connection()).await, before);
    store.close().await.unwrap();
}

#[tokio::test]
async fn an_old_reader_snapshot_stays_consistent_while_wal_writes_commit() {
    let directory = tempfile::tempdir().unwrap();
    let store = NotesStore::open(directory.path().join("notes.db"))
        .await
        .unwrap();
    store.create_page("Before snapshot".into()).await.unwrap();
    let destination = directory.path().join("old.db");
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut capture = Box::pin(
        store
            .read_connection()
            .call_domain(move |source| -> Result<_> {
                let transaction = source.transaction()?;
                let metadata = read_metadata(&transaction)?;
                entered_tx.send(metadata.commit_version).unwrap();
                release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                write_snapshot(transaction, destination, metadata, BACKUP_RETRY_LIMIT)
            }),
    );
    assert!(
        capture
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    let old_version = entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let edited = tokio::time::timeout(
        Duration::from_secs(10),
        store.create_page("After snapshot".into()),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(edited.commit_version.unwrap() > old_version);
    release_tx.send(()).unwrap();
    let backup = capture.await.unwrap();
    assert_eq!(backup.commit_version, old_version);
    let old = Connection::open_read_only(backup.path).await.unwrap();
    let old_titles = db::list_pages(&old, 100)
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.title.unwrap())
        .collect::<Vec<_>>();
    assert_eq!(old_titles, ["Before snapshot"]);
    assert_eq!(
        db::list_pages(store.read_connection(), 100)
            .await
            .unwrap()
            .len(),
        2
    );
    old.close().await.unwrap();
    store.close().await.unwrap();
}

fn temporary_files(directory: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".tangleaf-db-backup-")
        })
        .collect()
}

#[tokio::test]
async fn an_existing_destination_is_preserved_and_unblocked_copy_needs_no_retry_budget() {
    let directory = tempfile::tempdir().unwrap();
    let store = NotesStore::open(directory.path().join("notes.db"))
        .await
        .unwrap();
    let destination = directory.path().join("existing.db");
    std::fs::write(&destination, b"existing destination").unwrap();
    assert!(store.backup_database(&destination).await.is_err());
    assert_eq!(
        std::fs::read(&destination).unwrap(),
        b"existing destination"
    );
    assert!(temporary_files(directory.path()).is_empty());
    let unblocked = directory.path().join("unblocked.db");
    let path = unblocked.clone();
    store
        .read_connection()
        .call_domain(move |db| -> Result<_> {
            let transaction = db.transaction()?;
            let metadata = read_metadata(&transaction)?;
            write_snapshot(transaction, path, metadata, Duration::ZERO)
        })
        .await
        .unwrap();
    assert!(unblocked.exists());
    assert!(temporary_files(directory.path()).is_empty());
    store.close().await.unwrap();
}

#[test]
fn only_consecutive_busy_or_locked_steps_exhaust_the_retry_budget() {
    let now = Instant::now();
    let mut retries = BusyRetries {
        blocked_since: None,
        limit: Duration::from_secs(30),
    };
    retries.observe(StepResult::Busy, now).unwrap();
    retries
        .observe(StepResult::Locked, now + Duration::from_secs(29))
        .unwrap();
    assert!(
        retries
            .observe(StepResult::Busy, now + Duration::from_secs(30))
            .is_err()
    );
    // Genuine page-copy progress resets the wait budget, even after a long
    // interval. A stable copy is not capped by its total elapsed duration.
    retries
        .observe(StepResult::More, now + Duration::from_secs(100))
        .unwrap();
    retries
        .observe(StepResult::Locked, now + Duration::from_secs(100))
        .unwrap();
    retries
        .observe(StepResult::Busy, now + Duration::from_secs(129))
        .unwrap();
    assert!(
        retries
            .observe(StepResult::Locked, now + Duration::from_secs(130))
            .is_err()
    );
    let mut no_retries = BusyRetries {
        blocked_since: None,
        limit: Duration::ZERO,
    };
    no_retries.observe(StepResult::More, now).unwrap();
    no_retries
        .observe(StepResult::Done, now + Duration::from_secs(1_000))
        .unwrap();
    assert!(no_retries.observe(StepResult::Busy, now).is_err());
}

#[tokio::test]
async fn the_sealed_snapshot_opens_without_wal_sidecars_in_a_read_only_directory() {
    let directory = tempfile::tempdir().unwrap();
    let store = NotesStore::open(directory.path().join("notes.db"))
        .await
        .unwrap();
    store.create_page("Standalone".into()).await.unwrap();
    let sealed_directory = directory.path().join("sealed");
    std::fs::create_dir(&sealed_directory).unwrap();
    let backup = store
        .backup_database(sealed_directory.join("snapshot.db"))
        .await
        .unwrap();
    #[cfg(unix)]
    let _permissions = {
        use std::os::unix::fs::PermissionsExt;
        struct RestorePermissions(PathBuf);
        impl Drop for RestorePermissions {
            fn drop(&mut self) {
                let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o700));
            }
        }
        std::fs::set_permissions(&backup.path, std::fs::Permissions::from_mode(0o400)).unwrap();
        std::fs::set_permissions(&sealed_directory, std::fs::Permissions::from_mode(0o500))
            .unwrap();
        RestorePermissions(sealed_directory.clone())
    };
    let reader = Connection::open_read_only(&backup.path).await.unwrap();
    reader
        .call(|db| {
            assert_eq!(
                db.pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))?,
                "delete"
            );
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        db::list_pages(&reader, 10).await.unwrap()[0]
            .title
            .as_deref(),
        Some("Standalone")
    );
    reader.close().await.unwrap();
    assert_eq!(std::fs::read_dir(&sealed_directory).unwrap().count(), 1);
    store.close().await.unwrap();
}

#[tokio::test]
async fn foreign_key_violations_refuse_publication_and_clean_the_entire_temporary_destination() {
    let directory = tempfile::tempdir().unwrap();
    let store = NotesStore::open(directory.path().join("notes.db"))
        .await
        .unwrap();
    store
        .write_connection()
        .call(|db| {
            db.execute_batch(
                "CREATE TABLE backup_parent (id INTEGER PRIMARY KEY);
         CREATE TABLE backup_child (parent INTEGER REFERENCES backup_parent(id));
         PRAGMA foreign_keys=OFF;
         INSERT INTO backup_child VALUES(1);
         PRAGMA foreign_keys=ON;",
            )
        })
        .await
        .unwrap();
    let destination = directory.path().join("invalid.db");
    let error = store.backup_database(&destination).await.unwrap_err();
    assert!(error.to_string().contains("foreign_key_check"), "{error:#}");
    assert!(!destination.exists());
    assert!(temporary_files(directory.path()).is_empty());
    store.close().await.unwrap();
}

#[test]
fn publication_cannot_clobber_a_destination_created_after_preparation() {
    use std::io::Write;
    let directory = tempfile::tempdir().unwrap();
    let mut temporary = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
    temporary.write_all(b"new snapshot").unwrap();
    let temporary_path = temporary.path().to_owned();
    let destination = directory.path().join("concurrent.db");
    std::fs::write(&destination, b"concurrent destination").unwrap();
    assert!(publish_snapshot(temporary, &destination).is_err());
    assert_eq!(
        std::fs::read(destination).unwrap(),
        b"concurrent destination"
    );
    assert!(!temporary_path.exists());
}

#[tokio::test]
async fn dropping_an_accepted_backup_waiter_does_not_orphan_its_job_or_temporary_files() {
    let directory = tempfile::tempdir().unwrap();
    let store = NotesStore::open(directory.path().join("notes.db"))
        .await
        .unwrap();
    store.create_page("Accepted backup".into()).await.unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut blocker = Box::pin(store.read_connection().call(move |_| {
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        Ok(())
    }));
    let mut context = Context::from_waker(Waker::noop());
    assert!(blocker.as_mut().poll(&mut context).is_pending());
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let destination = directory.path().join("accepted.db");
    let mut backup = Box::pin(store.backup_database(&destination));
    assert!(backup.as_mut().poll(&mut context).is_pending());
    drop(backup);
    release_tx.send(()).unwrap();
    blocker.await.unwrap();
    store.close().await.unwrap();
    assert!(destination.exists());
    assert!(temporary_files(directory.path()).is_empty());
    let captured = Connection::open_read_only(destination).await.unwrap();
    assert_eq!(
        db::list_pages(&captured, 10).await.unwrap()[0]
            .title
            .as_deref(),
        Some("Accepted backup")
    );
    captured.close().await.unwrap();
}
