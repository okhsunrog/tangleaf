use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::Waker;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

fn poll_pending<T>(future: std::pin::Pin<&mut impl Future<Output = T>>) {
    assert!(
        future
            .poll(&mut std::task::Context::from_waker(Waker::noop()))
            .is_pending()
    );
}

async fn small_connection(capacity: usize) -> Connection {
    Connection::open_with_capacity(|| Ok(rusqlite::Connection::open_in_memory()?), capacity)
        .await
        .expect("open small-capacity worker")
}

async fn completion(mut closed: watch::Receiver<Option<CloseOutcome>>) -> CloseOutcome {
    tokio::time::timeout(TEST_TIMEOUT, async move {
        loop {
            if let Some(outcome) = closed.borrow_and_update().clone() {
                return outcome;
            }
            closed
                .changed()
                .await
                .expect("close completion remains available");
        }
    })
    .await
    .expect("worker was reaped")
}

#[tokio::test]
async fn cancel_before_admission_skips_sql_and_releases_capacity() {
    let connection = small_connection(2).await;
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut running = Box::pin(connection.call(move |_| {
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(TEST_TIMEOUT).unwrap();
        Ok(())
    }));
    poll_pending(running.as_mut());
    entered_rx.recv_timeout(TEST_TIMEOUT).unwrap();

    let accepted = Arc::new(AtomicUsize::new(0));
    let ran = accepted.clone();
    let mut queued = Box::pin(connection.call(move |_| {
        ran.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }));
    poll_pending(queued.as_mut());
    assert_eq!(connection.worker.admission.available_permits(), 0);

    let canceled = Arc::new(AtomicUsize::new(0));
    let ran = canceled.clone();
    let mut unaccepted = Box::pin(connection.call(move |_| {
        ran.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }));
    poll_pending(unaccepted.as_mut());
    drop(unaccepted);
    // The accepted queued job belongs to the worker even if its waiter drops.
    drop(queued);
    release_tx.send(()).unwrap();
    running.await.unwrap();
    connection.call(|_| Ok(())).await.unwrap();
    assert_eq!(accepted.load(Ordering::SeqCst), 1);
    assert_eq!(canceled.load(Ordering::SeqCst), 0);
    connection.close().await.unwrap();
    assert_eq!(connection.worker.admission.available_permits(), 2);
}

#[tokio::test]
async fn close_drains_accepted_work_and_wakes_unaccepted_callers_across_clones() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let path = file.path().to_owned();
    let connection =
        Connection::open_with_capacity(move || Ok(rusqlite::Connection::open(path)?), 2)
            .await
            .unwrap();
    let clone = connection.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut running = Box::pin(connection.call(move |db| {
        db.execute_batch("CREATE TABLE values_table (value INTEGER)")?;
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(TEST_TIMEOUT).unwrap();
        db.execute("INSERT INTO values_table VALUES (1)", [])?;
        Ok(())
    }));
    poll_pending(running.as_mut());
    entered_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    let mut queued = Box::pin(clone.call(|db| {
        db.execute("INSERT INTO values_table VALUES (2)", [])?;
        Ok(())
    }));
    poll_pending(queued.as_mut());
    let mut unaccepted = Box::pin(clone.call(|db| {
        db.execute("INSERT INTO values_table VALUES (3)", [])?;
        Ok(())
    }));
    poll_pending(unaccepted.as_mut());

    let mut closing = Box::pin(connection.close());
    poll_pending(closing.as_mut());
    assert!(connection.worker.closed.borrow().is_none());
    assert!(
        unaccepted
            .await
            .unwrap_err()
            .to_string()
            .contains("closing or closed")
    );
    assert!(
        clone
            .call(|_| Ok(()))
            .await
            .unwrap_err()
            .to_string()
            .contains("closing or closed")
    );
    release_tx.send(()).unwrap();
    running.await.unwrap();
    queued.await.unwrap();
    closing.await.unwrap();
    clone.close().await.unwrap();
    let db = rusqlite::Connection::open(file.path()).unwrap();
    let sum: i64 = db
        .query_row("SELECT SUM(value) FROM values_table", [], |row| row.get(0))
        .unwrap();
    assert_eq!(sum, 3);
    assert!(connection.worker.gate.lock().unwrap().thread.is_none());
}

#[tokio::test]
async fn dropping_the_first_close_waiter_does_not_cancel_shared_shutdown() {
    let connection = small_connection(1).await;
    let clone = connection.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut running = Box::pin(connection.call(move |_| {
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(TEST_TIMEOUT).unwrap();
        Ok(())
    }));
    poll_pending(running.as_mut());
    entered_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    let mut first_close = Box::pin(connection.close());
    poll_pending(first_close.as_mut());
    drop(first_close);
    let mut another_close = Box::pin(clone.close());
    poll_pending(another_close.as_mut());
    release_tx.send(()).unwrap();
    running.await.unwrap();
    another_close.await.unwrap();
    let outcomes: Vec<_> = (0..8)
        .map(|_| {
            let clone = connection.clone();
            tokio::spawn(async move { clone.close().await })
        })
        .collect();
    for outcome in outcomes {
        outcome.await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn last_handle_drop_drains_and_joins_without_an_explicit_close_waiter() {
    let connection = small_connection(1).await;
    let closed = connection.worker.closed.subscribe();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let ran = Arc::new(AtomicUsize::new(0));
    let ran_in_job = ran.clone();
    let mut accepted = Box::pin(connection.call(move |_| {
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(TEST_TIMEOUT).unwrap();
        ran_in_job.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }));
    poll_pending(accepted.as_mut());
    entered_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    drop(accepted);
    drop(connection);
    assert!(closed.borrow().is_none());
    release_tx.send(()).unwrap();
    completion(closed).await.unwrap();
    assert_eq!(ran.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn last_handle_can_drop_on_the_sqlite_thread_without_joining_itself() {
    let connection = small_connection(1).await;
    let last_handle = connection.clone();
    let closed = connection.worker.closed.subscribe();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut accepted = Box::pin(connection.call(move |_| {
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(TEST_TIMEOUT).unwrap();
        // The worker closure owns the final handle after its waiter drops.
        // Its destructor must send Close and delegate join to the reaper.
        drop(last_handle);
        Ok(())
    }));
    poll_pending(accepted.as_mut());
    entered_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    drop(accepted);
    drop(connection);
    assert!(closed.borrow().is_none());
    release_tx.send(()).unwrap();
    completion(closed).await.unwrap();
}

#[tokio::test]
async fn cancel_open_while_opener_is_running_releases_the_database_lock() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let db = rusqlite::Connection::open(file.path()).unwrap();
    db.execute_batch("CREATE TABLE values_table (value INTEGER)")
        .unwrap();
    let path = file.path().to_owned();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut opening = Box::pin(Connection::open_with(move || {
        let db = rusqlite::Connection::open(path)?;
        db.execute_batch("BEGIN EXCLUSIVE")?;
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(TEST_TIMEOUT).unwrap();
        Ok(db)
    }));
    poll_pending(opening.as_mut());
    entered_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    drop(opening);
    release_tx.send(()).unwrap();
    // This must succeed only after the canceled opener's owned connection has
    // closed. SQLite's busy timeout is a failure bound, not a timing assertion.
    db.busy_timeout(TEST_TIMEOUT).unwrap();
    db.execute("INSERT INTO values_table VALUES (1)", [])
        .unwrap();
}

#[tokio::test]
async fn failed_and_panicking_openers_can_be_joined_and_report_their_failures() {
    let (connection, opened) =
        Connection::start_worker(|| Err(anyhow!("opener failed")), 1).unwrap();
    let error = opened.await.unwrap().unwrap_err();
    assert_eq!(error.to_string(), "opener failed");
    connection.close().await.unwrap();

    let (connection, opened) = Connection::start_worker(|| panic!("opener panic"), 1).unwrap();
    assert!(opened.await.is_err());
    let first = connection.close().await.unwrap_err().to_string();
    assert!(first.contains("opener panic"), "{first}");
    assert_eq!(
        connection.clone().close().await.unwrap_err().to_string(),
        first
    );

    let failed = Connection::open_with(|| Err(anyhow!("public open failure"))).await;
    assert!(
        failed
            .err()
            .unwrap()
            .to_string()
            .contains("public open failure")
    );
    let panicked = Connection::open_with(|| panic!("public opener panic")).await;
    assert!(
        panicked
            .err()
            .unwrap()
            .to_string()
            .contains("stopped while opening")
    );
}

#[tokio::test]
async fn close_from_its_sqlite_thread_is_rejected_without_starting_shutdown() {
    let connection = small_connection(1).await;
    let clone = connection.clone();
    let runtime = tokio::runtime::Handle::current();
    let error = connection
        .call(move |_| Ok(runtime.block_on(clone.close()).unwrap_err().to_string()))
        .await
        .unwrap();
    assert!(error.contains("own worker thread"), "{error}");
    connection.call(|_| Ok(())).await.unwrap();
    connection.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn racing_close_and_submissions_run_exactly_the_accepted_jobs() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let connection = Connection::open(file.path()).await.unwrap();
    connection
        .call(|db| {
            db.execute_batch("CREATE TABLE values_table (value INTEGER)")?;
            Ok(())
        })
        .await
        .unwrap();
    let barrier = Arc::new(tokio::sync::Barrier::new(34));
    let mut jobs = Vec::new();
    for value in 0..32 {
        let connection = connection.clone();
        let barrier = barrier.clone();
        jobs.push(tokio::spawn(async move {
            barrier.wait().await;
            connection
                .call(move |db| {
                    db.execute("INSERT INTO values_table VALUES (?1)", [value])?;
                    Ok(())
                })
                .await
        }));
    }
    let closer = connection.clone();
    let close_barrier = barrier.clone();
    let close = tokio::spawn(async move {
        close_barrier.wait().await;
        closer.close().await
    });
    barrier.wait().await;
    let mut accepted = 0;
    for job in jobs {
        match job.await.unwrap() {
            Ok(()) => accepted += 1,
            Err(error) => assert!(error.to_string().contains("closing or closed"), "{error}"),
        }
    }
    close.await.unwrap().unwrap();
    let db = rusqlite::Connection::open(file.path()).unwrap();
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM values_table", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, i64::from(accepted));
}

#[tokio::test]
async fn sqlite_close_failure_is_shared_and_never_reported_as_joined_success() {
    let connection = small_connection(1).await;
    let handle = connection
        .call(|db| {
            // Deliberately leave a live statement to exercise SQLITE_BUSY on close.
            // Keep the raw handle only to finalize it after the worker has joined;
            // it is never accessed concurrently with rusqlite.
            let handle = unsafe { db.handle() } as usize;
            std::mem::forget(db.prepare("SELECT 1")?);
            Ok(handle)
        })
        .await
        .unwrap();
    let error = connection.close().await.unwrap_err().to_string();
    assert!(
        error.contains("closing SQLite connection failed"),
        "{error}"
    );
    assert_eq!(connection.close().await.unwrap_err().to_string(), error);
    // SAFETY: close returned SQLITE_BUSY, leaving this connection open; the
    // joined worker has dropped every Rust owner. Finalize only its deliberately
    // leaked statement, then release the raw connection to avoid a test leak.
    unsafe {
        let db = handle as *mut rusqlite::ffi::sqlite3;
        let statement = rusqlite::ffi::sqlite3_next_stmt(db, std::ptr::null_mut());
        assert!(!statement.is_null());
        assert_eq!(
            rusqlite::ffi::sqlite3_finalize(statement),
            rusqlite::ffi::SQLITE_OK
        );
        assert_eq!(rusqlite::ffi::sqlite3_close(db), rusqlite::ffi::SQLITE_OK);
    }
}

#[tokio::test]
async fn an_undelivered_result_with_a_panicking_destructor_cannot_skip_accepted_writes() {
    struct PanickingResult;
    impl Drop for PanickingResult {
        fn drop(&mut self) {
            panic!("undelivered result destructor");
        }
    }

    let connection = small_connection(3).await;
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut blocker = Box::pin(connection.call(move |db| {
        db.execute_batch("CREATE TABLE values_table (value INTEGER)")?;
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(TEST_TIMEOUT).unwrap();
        Ok(())
    }));
    poll_pending(blocker.as_mut());
    entered_rx.recv_timeout(TEST_TIMEOUT).unwrap();

    let mut undelivered = Box::pin(connection.call(|_| Ok(PanickingResult)));
    poll_pending(undelivered.as_mut());
    drop(undelivered);
    let mut accepted_write = Box::pin(connection.call(|db| {
        db.execute("INSERT INTO values_table VALUES (42)", [])?;
        Ok(())
    }));
    poll_pending(accepted_write.as_mut());
    assert_eq!(connection.worker.admission.available_permits(), 0);
    release_tx.send(()).unwrap();
    blocker.await.unwrap();
    accepted_write
        .await
        .expect("write accepted after the panicking result still runs");
    let value: i64 = connection
        .call(|db| db.query_row("SELECT value FROM values_table", [], |row| row.get(0)))
        .await
        .unwrap();
    assert_eq!(value, 42);
    connection.close().await.unwrap();
}
