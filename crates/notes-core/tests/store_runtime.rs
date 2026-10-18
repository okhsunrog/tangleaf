use notes_core::db;
use notes_core::store::NotesStore;
use std::future::Future;
use std::sync::mpsc;
use std::task::{Poll, Waker};
use std::time::Duration;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

fn poll_pending<T>(future: std::pin::Pin<&mut impl Future<Output = T>>) {
    assert!(
        future
            .poll(&mut std::task::Context::from_waker(Waker::noop()))
            .is_pending()
    );
}

#[tokio::test]
async fn file_runtime_owns_an_independent_read_only_worker_with_the_shared_policy() {
    let directory = tempfile::tempdir().unwrap();
    let store = NotesStore::open(directory.path().join("notes.db"))
        .await
        .unwrap();
    assert!(!std::ptr::eq(
        store.read_connection(),
        store.write_connection()
    ));
    let reader_thread = store
        .read_connection()
        .call(|db| {
            assert!(db.is_readonly("main")?);
            assert_eq!(
                db.pragma_query_value(None, "query_only", |row| row.get::<_, i64>(0))?,
                1
            );
            assert_eq!(
                db.pragma_query_value(None, "foreign_keys", |row| row.get::<_, i64>(0))?,
                1
            );
            assert_eq!(
                db.pragma_query_value(None, "temp_store", |row| row.get::<_, i64>(0))?,
                2
            );
            assert_eq!(
                db.pragma_query_value(None, "busy_timeout", |row| row.get::<_, i64>(0))?,
                5_000
            );
            assert_eq!(
                db.pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))?,
                "wal"
            );
            assert!(
                db.execute_batch("CREATE TABLE forbidden (value INTEGER)")
                    .is_err()
            );
            assert!(
                db.execute_batch("CREATE TEMP TABLE forbidden_temp (value INTEGER)")
                    .is_err()
            );
            // The file itself remains protected even if query_only is disabled.
            db.pragma_update(None, "query_only", "OFF")?;
            assert!(
                db.execute_batch("CREATE TABLE still_forbidden (value INTEGER)")
                    .is_err()
            );
            db.pragma_update(None, "query_only", "ON")?;
            Ok(std::thread::current().id())
        })
        .await
        .unwrap();
    let writer_thread = store
        .write_connection()
        .call(|db| {
            assert!(!db.is_readonly("main")?);
            assert_eq!(
                db.pragma_query_value(None, "synchronous", |row| row.get::<_, i64>(0))?,
                2
            );
            Ok(std::thread::current().id())
        })
        .await
        .unwrap();
    assert_ne!(reader_thread, writer_thread);
    store.close().await.unwrap();
}

#[tokio::test]
async fn the_reader_immediately_sees_successfully_committed_store_edits() {
    let directory = tempfile::tempdir().unwrap();
    let store = NotesStore::open(directory.path().join("notes.db"))
        .await
        .unwrap();
    let mut page = store.create_page("Before".into()).await.unwrap().value;
    assert_eq!(
        db::get_page(store.read_connection(), page.uuid)
            .await
            .unwrap(),
        Some(page.clone())
    );
    for title in ["First edit", "Second edit", "Third edit"] {
        page = store
            .rename_page_if_revision(page.uuid, Some(title.into()), page.title_revision)
            .await
            .unwrap()
            .value;
        assert_eq!(
            db::get_page(store.read_connection(), page.uuid)
                .await
                .unwrap(),
            Some(page.clone())
        );
    }
    store.close().await.unwrap();
}

#[tokio::test]
async fn a_short_wal_snapshot_does_not_block_writes_and_releases_its_checkpoint_pin() {
    let directory = tempfile::tempdir().unwrap();
    let store = NotesStore::open(directory.path().join("notes.db"))
        .await
        .unwrap();
    store
        .write_connection()
        .call(|db| db.pragma_update(None, "wal_autocheckpoint", 0))
        .await
        .unwrap();
    store.create_page("Existing".into()).await.unwrap();
    let checkpoint = store
        .write_connection()
        .call(|db| {
            db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
        })
        .await
        .unwrap();
    assert_eq!(checkpoint, (0, 0, 0));
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut snapshot = Box::pin(store.read_connection().call(move |db| {
        let transaction = db.transaction()?;
        let before =
            transaction.query_row("SELECT COUNT(*) FROM pages", [], |row| row.get::<_, i64>(0))?;
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(TEST_TIMEOUT).unwrap();
        let after =
            transaction.query_row("SELECT COUNT(*) FROM pages", [], |row| row.get::<_, i64>(0))?;
        transaction.commit()?;
        Ok((before, after))
    }));
    poll_pending(snapshot.as_mut());
    entered_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    tokio::time::timeout(
        TEST_TIMEOUT,
        store.create_page("Committed while reading".into()),
    )
    .await
    .expect("reader must not occupy the writer worker")
    .unwrap();
    let pinned = store
        .write_connection()
        .call(|db| {
            db.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
        })
        .await
        .unwrap();
    assert_eq!(pinned.0, 0);
    assert!(pinned.1 > 0);
    assert!(
        pinned.2 < pinned.1,
        "the old reader snapshot still pins newer WAL frames"
    );
    release_tx.send(()).unwrap();
    assert_eq!(snapshot.await.unwrap(), (1, 1));
    let fresh = store
        .read_connection()
        .call(|db| db.query_row("SELECT COUNT(*) FROM pages", [], |row| row.get::<_, i64>(0)))
        .await
        .unwrap();
    assert_eq!(fresh, 2);
    let released = store
        .write_connection()
        .call(|db| {
            db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
        })
        .await
        .unwrap();
    assert_eq!(released, (0, 0, 0));
    store.close().await.unwrap();
}

#[tokio::test]
async fn canceled_store_close_stops_both_workers_before_waiting_and_clones_share_completion() {
    let directory = tempfile::tempdir().unwrap();
    let store = NotesStore::open(directory.path().join("notes.db"))
        .await
        .unwrap();
    let clone = store.clone();
    let reader = store.read_connection().clone();
    let writer = store.write_connection().clone();
    let (read_entered_tx, read_entered_rx) = mpsc::channel();
    let (read_release_tx, read_release_rx) = mpsc::channel();
    let mut reading = Box::pin(reader.call(move |_| {
        read_entered_tx.send(()).unwrap();
        read_release_rx.recv_timeout(TEST_TIMEOUT).unwrap();
        Ok(())
    }));
    poll_pending(reading.as_mut());
    read_entered_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    let (write_entered_tx, write_entered_rx) = mpsc::channel();
    let (write_release_tx, write_release_rx) = mpsc::channel();
    let mut writing = Box::pin(writer.call(move |db| {
        write_entered_tx.send(()).unwrap();
        write_release_rx.recv_timeout(TEST_TIMEOUT).unwrap();
        db.execute_batch("CREATE TABLE accepted_write (value INTEGER)")?;
        Ok(())
    }));
    poll_pending(writing.as_mut());
    write_entered_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    let mut closing = Box::pin(store.close());
    poll_pending(closing.as_mut());
    drop(closing);
    // Both submissions must reject immediately despite both workers still
    // being blocked. Cancellation must not leave either admission gate open.
    for connection in [&reader, &writer] {
        let mut rejected = Box::pin(connection.call(|_| Ok(())));
        assert!(matches!(
            rejected
                .as_mut()
                .poll(&mut std::task::Context::from_waker(Waker::noop())),
            Poll::Ready(Err(_))
        ));
    }
    let mut closing_again = Box::pin(clone.close());
    poll_pending(closing_again.as_mut());
    read_release_tx.send(()).unwrap();
    write_release_tx.send(()).unwrap();
    reading.await.unwrap();
    writing.await.unwrap();
    closing_again.await.unwrap();
    let (first, second, third) = tokio::join!(store.close(), clone.close(), reader.close());
    first.unwrap();
    second.unwrap();
    third.unwrap();
    assert!(writer.call(|_| Ok(())).await.is_err());
    let reopened = rusqlite::Connection::open(directory.path().join("notes.db")).unwrap();
    assert!(
        reopened
            .query_row("SELECT COUNT(*) FROM accepted_write", [], |row| row
                .get::<_, i64>(0))
            .is_ok()
    );
}

#[tokio::test]
async fn legacy_new_keeps_one_shared_worker_for_memory_databases_and_closes_it_once() {
    let connection = db::open(":memory:").await.unwrap();
    let store = NotesStore::new(connection.clone());
    assert!(std::ptr::eq(
        store.read_connection(),
        store.write_connection()
    ));
    let page = store.create_page("In memory".into()).await.unwrap().value;
    assert_eq!(
        db::get_page(store.read_connection(), page.uuid)
            .await
            .unwrap(),
        Some(page)
    );
    store.close().await.unwrap();
    store.clone().close().await.unwrap();
    assert!(connection.call(|_| Ok(())).await.is_err());
}

#[tokio::test]
async fn a_reader_close_error_still_waits_for_the_accepted_writer_and_joins_it() {
    let directory = tempfile::tempdir().unwrap();
    let store = NotesStore::open(directory.path().join("notes.db"))
        .await
        .unwrap();
    let raw_reader = store
        .read_connection()
        .call(|db| {
            // Force a real SQLITE_BUSY close failure. The raw handle is only used
            // to release this deliberately retained statement after worker join.
            let handle = unsafe { db.handle() } as usize;
            std::mem::forget(db.prepare("SELECT 1")?);
            Ok(handle)
        })
        .await
        .unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut accepted_write = Box::pin(store.write_connection().call(move |db| {
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(TEST_TIMEOUT).unwrap();
        db.execute_batch("CREATE TABLE persisted_after_error (value INTEGER)")?;
        Ok(())
    }));
    poll_pending(accepted_write.as_mut());
    entered_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    let mut closing = Box::pin(store.close());
    poll_pending(closing.as_mut());
    assert!(store.write_connection().call(|_| Ok(())).await.is_err());
    release_tx.send(()).unwrap();
    accepted_write.await.unwrap();
    let error = closing.await.unwrap_err();
    assert!(format!("{error:#}").contains("closing SQLite read worker"));
    assert!(format!("{error:#}").contains("closing SQLite connection failed"));
    assert!(store.read_connection().call(|_| Ok(())).await.is_err());
    store.write_connection().close().await.unwrap();
    let reopened = rusqlite::Connection::open(directory.path().join("notes.db")).unwrap();
    assert!(
        reopened
            .query_row("SELECT COUNT(*) FROM persisted_after_error", [], |row| row
                .get::<_, i64>(
                0
            ))
            .is_ok()
    );
    // SAFETY: SQLITE_BUSY left this reader open because of its live statement;
    // Store::close waited for thread join, so no Rust owner can access it now.
    unsafe {
        let db = raw_reader as *mut rusqlite::ffi::sqlite3;
        let statement = rusqlite::ffi::sqlite3_next_stmt(db, std::ptr::null_mut());
        assert!(!statement.is_null());
        assert_eq!(
            rusqlite::ffi::sqlite3_finalize(statement),
            rusqlite::ffi::SQLITE_OK
        );
        assert_eq!(rusqlite::ffi::sqlite3_close(db), rusqlite::ffi::SQLITE_OK);
    }
}
