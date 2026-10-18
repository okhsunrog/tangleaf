use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

async fn exit(lifecycle: StartupLifecycle) -> Outcome {
    lifecycle.stop();
    let outcome = lifecycle.wait().await;
    if let Some(store) = lifecycle.opened_store() {
        store.close().await.map_err(|error| error.to_string())?;
    }
    outcome
}

async fn assert_closed(store: &NotesStore) {
    assert!(store.write_connection().call(|_| Ok(())).await.is_err());
    assert!(store.read_connection().call(|_| Ok(())).await.is_err());
}

#[tokio::test]
async fn exit_waits_for_a_late_file_opener_and_closes_both_workers_without_ready_or_sync() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("notes.db");
    let (lifecycle, registration) = StartupLifecycle::register();
    let (entered, opening) = tokio::sync::oneshot::channel();
    let (release, released) = tokio::sync::oneshot::channel();
    let ready_and_sync = Arc::new(AtomicUsize::new(0));
    let published = ready_and_sync.clone();
    let startup = lifecycle.clone();
    registration.spawn(
        async move {
            entered.send(()).unwrap();
            released.await?;
            let store = NotesStore::open(path).await?;
            startup.retain_store(&store);
            startup.while_starting(|| {
                published.fetch_add(1, Ordering::SeqCst);
            });
            Ok(())
        },
        |_| panic!("late successful opener must not report an error"),
    );
    opening.await.unwrap();
    lifecycle.stop();
    let shutting_down = tokio::spawn(exit(lifecycle.clone()));
    tokio::task::yield_now().await;
    assert!(!shutting_down.is_finished());
    assert!(lifecycle.opened_store().is_none());
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), shutting_down)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let store = lifecycle
        .opened_store()
        .expect("late Store must remain owned until exit");
    assert_closed(&store).await;
    assert_eq!(ready_and_sync.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_dropped_exit_waiter_does_not_cancel_the_opening_or_the_owned_cleanup() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("notes.db");
    let (lifecycle, registration) = StartupLifecycle::register();
    let (release, released) = tokio::sync::oneshot::channel();
    let startup = lifecycle.clone();
    registration.spawn(
        async move {
            released.await?;
            let store = NotesStore::open(path).await?;
            startup.retain_store(&store);
            Ok(())
        },
        |_| panic!("successful opener must not report an error"),
    );
    lifecycle.stop();
    let (finished, completion) = tokio::sync::oneshot::channel();
    let owner = lifecycle.clone();
    let cleanup = tokio::spawn(async move {
        let outcome = exit(owner).await;
        let _ = finished.send(outcome);
    });
    tokio::task::yield_now().await;
    drop(cleanup); // The root exit coordinator is owned; its waiter is not.
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), completion)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_closed(&lifecycle.opened_store().unwrap()).await;
}

#[tokio::test]
async fn startup_panic_after_open_keeps_the_uninstalled_store_available_for_exit_drain() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("notes.db");
    let (lifecycle, registration) = StartupLifecycle::register();
    let reported = Arc::new(AtomicUsize::new(0));
    let errors = reported.clone();
    let startup = lifecycle.clone();
    registration.spawn(
        async move {
            let store = NotesStore::open(path).await?;
            startup.retain_store(&store);
            panic!("failed before AppState installation");
        },
        move |message| {
            assert!(message.contains("failed before AppState installation"));
            errors.fetch_add(1, Ordering::SeqCst);
        },
    );
    let outcome = tokio::time::timeout(Duration::from_secs(5), lifecycle.wait())
        .await
        .unwrap();
    assert!(
        outcome
            .unwrap_err()
            .contains("failed before AppState installation")
    );
    assert_eq!(reported.load(Ordering::SeqCst), 1);
    assert!(exit(lifecycle.clone()).await.is_err());
    assert_closed(&lifecycle.opened_store().unwrap()).await;
}

#[tokio::test]
async fn canceled_startup_after_retaining_store_still_drains_its_already_accepted_sql_write() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("notes.db");
    let (lifecycle, registration) = StartupLifecycle::register();
    let (entered, running) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let startup = lifecycle.clone();
    let worker = tauri::async_runtime::spawn(async move {
        let store = NotesStore::open(path).await?;
        startup.retain_store(&store);
        store
            .write_connection()
            .call(move |database| {
                entered.send(()).unwrap();
                released.recv_timeout(Duration::from_secs(10)).unwrap();
                database.execute(
                    "INSERT INTO sync_meta(key,value) VALUES('startup-cancel-test','retained')",
                    [],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    });
    // This private test hook aborts only after the real Store is retained. The
    // production registration intentionally exposes no cancellation handle.
    let cancel = worker.inner().abort_handle();
    registration.supervise(worker, |_| {
        panic!("exit must suppress late startup error events")
    });
    running.await.unwrap();
    lifecycle.stop();
    cancel.abort();
    let outcome = tokio::time::timeout(Duration::from_secs(5), lifecycle.wait())
        .await
        .unwrap();
    assert!(outcome.unwrap_err().contains("cancel"));
    let closing = tokio::spawn(exit(lifecycle.clone()));
    tokio::task::yield_now().await;
    assert!(!closing.is_finished());
    release.send(()).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(5), closing)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    let store = lifecycle.opened_store().unwrap();
    assert_closed(&store).await;
    let reopened = NotesStore::open(directory.path().join("notes.db"))
        .await
        .unwrap();
    let value: String = reopened
        .read_connection()
        .call(|database| {
            database.query_row(
                "SELECT value FROM sync_meta WHERE key='startup-cancel-test'",
                [],
                |row| row.get(0),
            )
        })
        .await
        .unwrap();
    assert_eq!(value, "retained");
    reopened.close().await.unwrap();
}

#[tokio::test]
async fn setup_errors_and_early_task_failures_always_finish_the_startup_barrier() {
    for mode in ["setup", "open error", "panic"] {
        let (lifecycle, registration) = StartupLifecycle::register();
        match mode {
            "setup" => registration.finish(Err("device settings unavailable".into())),
            "open error" => {
                registration.spawn(async { anyhow::bail!("injected opener error") }, |_| {})
            }
            _ => registration.spawn(
                async { panic!("opener panic before creating a Store") },
                |_| {},
            ),
        }
        let outcome = tokio::time::timeout(Duration::from_secs(5), lifecycle.wait())
            .await
            .unwrap();
        assert!(outcome.is_err(), "{mode}");
        assert!(lifecycle.opened_store().is_none(), "{mode}");
        assert!(exit(lifecycle).await.is_err(), "{mode}");
    }
}

#[tokio::test]
async fn unwinding_synchronous_setup_drops_its_registration_without_blocking_exit() {
    let (lifecycle, registration) = StartupLifecycle::register();
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _registration = registration;
        panic!("synchronous setup failure");
    }));
    assert!(failure.is_err());
    assert!(
        tokio::time::timeout(Duration::from_secs(5), exit(lifecycle))
            .await
            .unwrap()
            .is_err()
    );
}
