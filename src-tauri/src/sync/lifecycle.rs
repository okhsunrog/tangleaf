//! Admission and completion for the application's single background sync task.

use std::future::Future;
use std::sync::{Arc, Mutex};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[derive(Clone)]
pub(super) struct SyncWorker {
    accepting: Arc<Mutex<bool>>,
    stop: CancellationToken,
    tasks: TaskTracker,
}

impl Default for SyncWorker {
    fn default() -> Self {
        Self {
            accepting: Arc::new(Mutex::new(true)),
            stop: CancellationToken::new(),
            tasks: TaskTracker::new(),
        }
    }
}

impl SyncWorker {
    pub(super) fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) -> bool {
        let accepting = self
            .accepting
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !*accepting {
            return false;
        }
        let stop = self.stop.clone();
        // Registration and the stop decision share one gate. TaskTracker alone
        // allows spawning after close, which would race workspace shutdown.
        self.tasks.spawn(async move {
            tokio::select! {
                biased;
                _ = stop.cancelled() => {},
                () = task => {},
            }
        });
        true
    }

    pub(super) async fn shutdown(&self) {
        {
            let mut accepting = self
                .accepting
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            *accepting = false;
            self.stop.cancel();
            self.tasks.close();
        }
        // Cancellation interrupts network waits and drops their futures. Any
        // SQLite job already accepted remains owned by its connection and is
        // drained later, before Store close completes.
        self.tasks.wait().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Released(Arc<AtomicUsize>);
    impl Drop for Released {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn shutdown_drops_a_parked_network_future_and_refuses_late_work() {
        let worker = SyncWorker::default();
        let released = Arc::new(AtomicUsize::new(0));
        let owned = Released(released.clone());
        let (entered, running) = tokio::sync::oneshot::channel();
        assert!(worker.spawn(async move {
            let _owned = owned;
            entered.send(()).unwrap();
            std::future::pending::<()>().await;
        }));
        running.await.unwrap();
        worker.shutdown().await;
        assert_eq!(released.load(Ordering::SeqCst), 1);
        assert!(!worker.spawn(async { panic!("work submitted after shutdown") }));
        worker.clone().shutdown().await;
    }

    #[tokio::test]
    async fn canceled_shutdown_waiter_cannot_reopen_admission() {
        let worker = SyncWorker::default();
        let released = Arc::new(AtomicUsize::new(0));
        let owned = Released(released.clone());
        let (entered, running) = tokio::sync::oneshot::channel();
        assert!(worker.spawn(async move {
            let _owned = owned;
            entered.send(()).unwrap();
            std::future::pending::<()>().await;
        }));
        running.await.unwrap();
        let mut shutdown = Box::pin(worker.shutdown());
        // Polling is enough to initiate closure, even if this waiter is then
        // dropped. No future owns the only copy of the shutdown signal.
        std::future::poll_fn(|context| {
            assert!(shutdown.as_mut().poll(context).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        drop(shutdown);
        assert!(!worker.spawn(async {}));
        worker.shutdown().await;
        assert_eq!(released.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn stopping_sync_does_not_discard_an_already_accepted_sqlite_write() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notes.db");
        let store = notes_core::store::NotesStore::open(&path).await.unwrap();
        let connection = store.write_connection().clone();
        let worker = SyncWorker::default();
        let (entered, running) = tokio::sync::oneshot::channel();
        let (release, paused) = std::sync::mpsc::channel();
        assert!(worker.spawn(async move {
            connection
                .call(move |sql| {
                    entered.send(()).unwrap();
                    paused
                        .recv_timeout(std::time::Duration::from_secs(10))
                        .unwrap();
                    sql.execute(
                        "INSERT INTO sync_meta(key,value) VALUES('sync-close-test','retained')",
                        [],
                    )?;
                    Ok(())
                })
                .await
                .unwrap();
        }));
        running.await.unwrap();
        // The waiting sync future is canceled; the connection still owns SQL.
        worker.shutdown().await;
        release.send(()).unwrap();
        store.close().await.unwrap();
        let reopened = notes_core::store::NotesStore::open(path).await.unwrap();
        let value: String = reopened
            .read_connection()
            .call(|sql| {
                sql.query_row(
                    "SELECT value FROM sync_meta WHERE key='sync-close-test'",
                    [],
                    |row| row.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(value, "retained");
        reopened.close().await.unwrap();
    }
}
