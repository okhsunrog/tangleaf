use super::*;
use std::future::{Future, poll_fn};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::Poll;

struct Fixture {
    directory: tempfile::TempDir,
    connection: crate::Connection,
    runtime: WorkspaceRuntime,
    sessions: Vec<Session>,
    published: Arc<AtomicUsize>,
}

async fn fixture(count: usize) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("notes.db");
    let connection = crate::db::open(&path).await.unwrap();
    crate::configure_sync(&connection, "https://ink.example")
        .await
        .unwrap();
    let runtime = WorkspaceRuntime::default();
    let published = Arc::new(AtomicUsize::new(0));
    let mut sessions = Vec::new();
    for _ in 0..count {
        let page = crate::db::create_handwritten_note_with_ops(&connection, None)
            .await
            .unwrap()
            .value;
        let sink = published.clone();
        sessions.push(
            Session::new_in_runtime(
                Store::new(&path, page.uuid),
                runtime.clone(),
                "Test device".into(),
                move |_| {
                    sink.fetch_add(1, Ordering::SeqCst);
                },
            )
            .unwrap(),
        );
    }
    Fixture {
        directory,
        connection,
        runtime,
        sessions,
        published,
    }
}

fn patch() -> InkDraftPatch {
    InkDraftPatch {
        order: Vec::new(),
        upserts: Vec::new(),
        background: InkBackground::Grid,
    }
}

async fn until(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !predicate() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("lifecycle state did not become ready");
}

fn jobs(runtime: &WorkspaceRuntime) -> usize {
    runtime.inner.state.lock().unwrap().jobs
}

fn idle(runtime: &WorkspaceRuntime) -> bool {
    let state = runtime.inner.state.lock().unwrap();
    state.jobs == 0 && state.tasks == 0
}

async fn block_input(
    session: &Session,
) -> (std::sync::mpsc::Sender<()>, std::thread::JoinHandle<()>) {
    let input = session.input.clone();
    let (entered, locked) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let _guard = input.lock().unwrap();
        entered.send(()).unwrap();
        released.recv_timeout(Duration::from_secs(10)).unwrap();
    });
    locked.await.unwrap();
    (release, thread)
}

#[tokio::test]
async fn canceled_access_waiter_keeps_its_queued_patch_registered_until_shutdown_finishes_it() {
    let f = fixture(1).await;
    let session = f.sessions[0].clone();
    let (release_input, input) = block_input(&session).await;
    let target = session.clone();
    let waiter = tokio::spawn(async move { target.patch(patch(), None).await });
    until(|| jobs(&f.runtime) == 1).await;
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    assert_eq!(jobs(&f.runtime), 1);
    let mut shutdown = Box::pin(f.runtime.shutdown(f.sessions.clone()));
    poll_fn(|context| {
        assert!(shutdown.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
    assert!(session.access(|_| Ok(())).await.is_err());
    release_input.send(()).unwrap();
    input.join().unwrap();
    shutdown.await.unwrap();
    assert!(idle(&f.runtime));
    assert_eq!(f.published.load(Ordering::SeqCst), 1);
    let reopened = Store::new(f.directory.path().join("notes.db"), session.page());
    assert!(matches!(
        reopened.read().unwrap().draft.background,
        InkBackground::Grid
    ));
    assert!(!reopened.status().unwrap().unpublished_changes);
    f.connection.close().await.unwrap();
}

#[tokio::test]
async fn stop_during_a_running_raw_connection_and_canceled_shutdown_waiter_still_drains() {
    let f = fixture(1).await;
    let session = f.sessions[0].clone();
    let (entered, running) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let target = session.clone();
    let waiter = tokio::spawn(async move {
        target
            .access(move |store| {
                let connection = crate::ink::storage::open(store)?;
                entered.send(()).unwrap();
                released.recv_timeout(Duration::from_secs(10)).unwrap();
                connection.execute(
                    "UPDATE ink_documents SET editing = 0 WHERE page_uuid = ?1",
                    [store.document()],
                )?;
                Ok(())
            })
            .await
    });
    running.await.unwrap();
    waiter.abort();
    let mut shutdown = Box::pin(f.runtime.shutdown(f.sessions.clone()));
    poll_fn(|context| {
        assert!(shutdown.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(shutdown);
    assert_eq!(jobs(&f.runtime), 1);
    assert!(
        Session::new_in_runtime(
            session.store.clone(),
            f.runtime.clone(),
            "Late".into(),
            |_| {}
        )
        .is_err()
    );
    assert!(session.open(true).await.is_err());
    assert!(session.patch(patch(), None).await.is_err());
    assert!(session.history(false, None).await.is_err());
    assert!(session.start_completion().is_err());
    release.send(()).unwrap();
    f.runtime.shutdown(f.sessions.clone()).await.unwrap();
    assert!(idle(&f.runtime));
    assert!(f.runtime.inner.state.lock().unwrap().phase == Phase::Closed);
    f.connection.close().await.unwrap();
}

#[tokio::test]
async fn sleeping_scheduler_is_woken_and_multiple_sessions_complete_without_a_minute_wait() {
    let f = fixture(3).await;
    for session in &f.sessions {
        session.open(true).await.unwrap();
    }
    let mut revisions = Vec::new();
    for session in &f.sessions[..2] {
        revisions.push(session.patch(patch(), None).await.unwrap());
        session.schedule();
    }
    until(|| f.runtime.inner.state.lock().unwrap().tasks == 2).await;
    tokio::time::timeout(
        Duration::from_secs(5),
        f.runtime.shutdown(f.sessions.clone()),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(idle(&f.runtime));
    assert_eq!(f.published.load(Ordering::SeqCst), 2);
    for (index, session) in f.sessions.iter().enumerate() {
        assert!(!session.pending.lock().unwrap().running);
        assert!(!session.needs_completion());
        let snapshot = session.store.read().unwrap();
        if index < 2 {
            assert_eq!(snapshot.revision.as_ref(), Some(&revisions[index]));
        }
    }
    assert!(recoverable_notes(&f.connection).await.unwrap().is_empty());
    f.connection.close().await.unwrap();
}

#[tokio::test]
async fn running_scheduler_holds_its_raw_registration_until_the_database_unblocks() {
    let f = fixture(1).await;
    let session = f.sessions[0].clone();
    session.open(true).await.unwrap();
    let revision = session.patch(patch(), None).await.unwrap();
    let mut blocker = rusqlite::Connection::open(f.directory.path().join("notes.db")).unwrap();
    let locked = blocker
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    session.schedule();
    tokio::task::yield_now().await;
    session.wake.notify_waiters();
    until(|| jobs(&f.runtime) == 1).await;
    let mut shutdown = Box::pin(f.runtime.shutdown(f.sessions.clone()));
    poll_fn(|context| {
        assert!(shutdown.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
    assert!(session.lane.try_lock().is_err());
    locked.commit().unwrap();
    shutdown.await.unwrap();
    assert!(idle(&f.runtime));
    assert_eq!(
        session.store.read().unwrap().revision.as_ref(),
        Some(&revision)
    );
    assert_eq!(f.published.load(Ordering::SeqCst), 1);
    f.connection.close().await.unwrap();
}

#[tokio::test]
async fn canceled_completion_waiter_keeps_the_owned_lane_and_notifies_one_committed_publication() {
    let f = fixture(1).await;
    let session = f.sessions[0].clone();
    session.open(true).await.unwrap();
    session.patch(patch(), None).await.unwrap();
    let lane = session.lane.lock().await;
    let target = session.clone();
    let waiter = tokio::spawn(async move { target.complete().await });
    // Completion has durably recorded intent before waiting on the shared lane.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let page = session.page();
            let requested: bool = f
                .connection
                .call(move |database| {
                    database.query_row(
                        "SELECT publication_requested FROM ink_documents WHERE page_uuid=?1",
                        [page],
                        |row| row.get(0),
                    )
                })
                .await
                .unwrap();
            if requested {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let (release_input, input) = block_input(&session).await;
    drop(lane);
    until(|| jobs(&f.runtime) == 1).await;
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    assert!(session.lane.try_lock().is_err());
    release_input.send(()).unwrap();
    input.join().unwrap();
    until(|| f.published.load(Ordering::SeqCst) == 1 && idle(&f.runtime)).await;
    assert!(!session.needs_completion());
    f.runtime.shutdown(f.sessions.clone()).await.unwrap();
    assert_eq!(f.published.load(Ordering::SeqCst), 1);
    f.connection.close().await.unwrap();
}

#[tokio::test]
async fn completion_registers_its_intent_job_before_the_owned_continuation_can_run() {
    let f = fixture(1).await;
    let session = f.sessions[0].clone();
    session.patch(patch(), None).await.unwrap();
    let (release_input, input) = block_input(&session).await;
    let completion = session.start_completion().unwrap();
    {
        let state = f.runtime.inner.state.lock().unwrap();
        assert_eq!(state.jobs, 1);
        assert_eq!(state.tasks, 1);
    }
    drop(completion);
    let mut shutdown = Box::pin(f.runtime.shutdown(f.sessions.clone()));
    poll_fn(|context| {
        assert!(shutdown.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
    release_input.send(()).unwrap();
    input.join().unwrap();
    shutdown.await.unwrap();
    assert!(idle(&f.runtime));
    assert_eq!(f.published.load(Ordering::SeqCst), 1);
    f.connection.close().await.unwrap();
}

#[tokio::test]
async fn failed_shutdown_is_fully_drained_and_retains_publication_intent_for_reopen() {
    let f = fixture(1).await;
    let session = f.sessions[0].clone();
    let revision = session.patch(patch(), None).await.unwrap();
    f.connection.call(|database| database.execute_batch("CREATE TRIGGER reject_shutdown_publication BEFORE INSERT ON ink_versions BEGIN SELECT RAISE(ABORT,'retained publication'); END;")).await.unwrap();
    let error = f.runtime.shutdown(f.sessions.clone()).await.unwrap_err();
    assert!(error.to_string().contains("retained publication"));
    assert!(idle(&f.runtime));
    assert!(session.access(|_| Ok(())).await.is_err());
    assert_eq!(f.published.load(Ordering::SeqCst), 0);
    f.connection.close().await.unwrap();
    let reopened = crate::db::open(f.directory.path().join("notes.db"))
        .await
        .unwrap();
    assert_eq!(
        recoverable_notes(&reopened).await.unwrap(),
        [session.page()]
    );
    reopened
        .call(|database| database.execute_batch("DROP TRIGGER reject_shutdown_publication"))
        .await
        .unwrap();
    let runtime = WorkspaceRuntime::default();
    let published = f.published.clone();
    let recovered = Session::new_in_runtime(
        session.store.clone(),
        runtime.clone(),
        "Reopened".into(),
        move |_| {
            published.fetch_add(1, Ordering::SeqCst);
        },
    )
    .unwrap();
    recovered.complete().await.unwrap();
    runtime.shutdown(vec![recovered]).await.unwrap();
    assert_eq!(f.published.load(Ordering::SeqCst), 1);
    assert_eq!(
        session.store.read().unwrap().revision.as_ref(),
        Some(&revision)
    );
    assert!(recoverable_notes(&reopened).await.unwrap().is_empty());
    reopened.close().await.unwrap();
}

#[tokio::test]
async fn panicking_publication_observer_does_not_republish_or_fail_a_committed_completion() {
    let f = fixture(1).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let sink = calls.clone();
    let session = Session::new_in_runtime(
        f.sessions[0].store.clone(),
        f.runtime.clone(),
        "Test".into(),
        move |_| {
            sink.fetch_add(1, Ordering::SeqCst);
            panic!("publication observer failure");
        },
    )
    .unwrap();
    session.patch(patch(), None).await.unwrap();
    session.complete().await.unwrap();
    session.complete().await.unwrap();
    f.runtime.shutdown(vec![session]).await.unwrap();
    assert!(idle(&f.runtime));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    f.connection.close().await.unwrap();
}
