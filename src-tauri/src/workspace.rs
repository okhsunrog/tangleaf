//! One confirmed close boundary shared by exit and physical recovery restore.

use anyhow::{Context, Result};
use notes_blob::BlobStore;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::Manager;
use tokio::sync::watch;

type Outcome = std::result::Result<Option<Arc<ClosedWorkspace>>, String>;

/// Only this module can mint the proof that current raw ink/sync/SQLite users
/// have drained. It retains the application lock through backup/replacement.
pub(crate) struct ClosedWorkspace {
    database: PathBuf,
    blobs: BlobStore,
    _application_lock: Arc<crate::app_lock::ApplicationLock>,
    pub(crate) warning: Option<String>,
}

impl ClosedWorkspace {
    pub(crate) fn database_path(&self) -> &Path {
        &self.database
    }

    pub(crate) fn blobs(&self) -> &BlobStore {
        &self.blobs
    }

    #[cfg(test)]
    pub(crate) async fn close_test_workspace(
        store: &notes_core::store::NotesStore,
        blobs: BlobStore,
        application_lock: crate::app_lock::ApplicationLock,
    ) -> Result<Arc<Self>> {
        store.close().await?;
        Ok(Arc::new(Self {
            database: blobs.root().join("notes.db"),
            blobs,
            _application_lock: Arc::new(application_lock),
            warning: None,
        }))
    }
}

struct Inner {
    started: Mutex<bool>,
    finished: watch::Sender<Option<Outcome>>,
}

#[derive(Clone)]
pub(crate) struct WorkspaceShutdown(Arc<Inner>);

impl Default for WorkspaceShutdown {
    fn default() -> Self {
        let (finished, _) = watch::channel(None);
        Self(Arc::new(Inner {
            started: Mutex::new(false),
            finished,
        }))
    }
}

impl WorkspaceShutdown {
    pub(crate) fn begin(&self, app: tauri::AppHandle) {
        if let Some(startup) = app.try_state::<crate::startup::StartupLifecycle>() {
            startup.stop();
        }
        self.begin_close(drain_application(app));
    }

    fn begin_close(
        &self,
        work: impl Future<Output = Result<Option<Arc<ClosedWorkspace>>>> + Send + 'static,
    ) -> bool {
        let mut started = self
            .0
            .started
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if *started {
            return false;
        }
        *started = true;
        let worker = tauri::async_runtime::spawn(work);
        let owner = self.clone();
        tauri::async_runtime::spawn(async move {
            let outcome = match worker.await {
                Ok(outcome) => outcome.map_err(|error| format!("{error:#}")),
                Err(error) => Err(format!("Workspace close task failed: {error}")),
            };
            owner.0.finished.send_replace(Some(outcome));
        });
        true
    }

    pub(crate) async fn wait(&self) -> Outcome {
        let mut finished = self.0.finished.subscribe();
        loop {
            if let Some(outcome) = finished.borrow_and_update().clone() {
                return outcome;
            }
            finished
                .changed()
                .await
                .map_err(|error| error.to_string())?;
        }
    }
}

async fn drain_application(app: tauri::AppHandle) -> Result<Option<Arc<ClosedWorkspace>>> {
    let startup = app
        .try_state::<crate::startup::StartupLifecycle>()
        .map(|state| state.inner().clone());
    if let Some(startup) = &startup
        && let Err(error) = startup.wait().await
    {
        tracing::warn!(%error, "Startup finished with an error before workspace drain");
    }
    let mut warning = None;
    if let Some(ink) = app.try_state::<crate::commands::HandwritingStore>()
        && let Err(error) = ink.shutdown().await
    {
        // The ink contract confirms all raw jobs are gone even on failure;
        // unpublished durable intent is captured by the before-restore backup.
        warning = Some(error.to_string());
        tracing::warn!(?error, "Ink drained; publication retained for recovery");
    }
    crate::sync::finish_before_exit(&app).await;
    let state = app.try_state::<crate::commands::AppState>();
    let store = startup
        .as_ref()
        .and_then(crate::startup::StartupLifecycle::opened_store)
        .or_else(|| state.as_ref().map(|state| state.store.clone()));
    if let Some(store) = store {
        store
            .close()
            .await
            .context("closing workspace SQLite workers")?;
        tracing::info!("Workspace SQLite workers closed");
    }
    let Some(state) = state else {
        return Ok(None);
    };
    let application_lock = startup
        .and_then(|startup| startup.application_lock())
        .context("closed workspace has no retained application lock")?;
    Ok(Some(Arc::new(ClosedWorkspace {
        database: state.blob_store.root().join("notes.db"),
        blobs: state.blob_store.clone(),
        _application_lock: application_lock,
        warning,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn competing_close_callers_share_one_drain_and_canceled_waiters_do_not_cancel_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notes.db");
        let store = notes_core::store::NotesStore::open(&path).await.unwrap();
        let retained = store.clone();
        let lock = Arc::new(
            crate::app_lock::ApplicationLock::acquire(directory.path())
                .await
                .unwrap(),
        );
        let blobs = BlobStore::new(directory.path());
        let shutdown = WorkspaceShutdown::default();
        let (entered, paused) = tokio::sync::oneshot::channel();
        let (release, released) = std::sync::mpsc::channel();
        let writer = store.write_connection().clone();
        let write = tokio::spawn(async move {
            writer
                .call(move |database| {
                    entered.send(()).unwrap();
                    released.recv_timeout(Duration::from_secs(10)).unwrap();
                    database.execute(
                        "INSERT INTO sync_meta(key,value) VALUES('close-proof','accepted')",
                        [],
                    )?;
                    Ok(())
                })
                .await
        });
        paused.await.unwrap();
        assert!(shutdown.begin_close(async move {
            store.close().await?;
            Ok(Some(Arc::new(ClosedWorkspace {
                database: path,
                blobs,
                _application_lock: lock,
                warning: None,
            })))
        }));
        assert!(!shutdown.begin_close(async { panic!("a second drain must never run") }));
        let waiter = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { shutdown.wait().await }
        });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());
        waiter.abort();
        release.send(()).unwrap();
        write.await.unwrap().unwrap();
        let closed = shutdown.wait().await.unwrap().unwrap();
        let second = shutdown.wait().await.unwrap().unwrap();
        assert!(Arc::ptr_eq(&closed, &second));
        assert!(retained.write_connection().call(|_| Ok(())).await.is_err());
        assert!(retained.read_connection().call(|_| Ok(())).await.is_err());
        let captured = notes_core::store::backup_closed_database(
            closed.database_path(),
            directory.path().join("closed.db"),
        )
        .await
        .unwrap();
        let reader = notes_core::Connection::open_read_only(captured.path)
            .await
            .unwrap();
        assert_eq!(
            reader
                .call(|db| db.query_row(
                    "SELECT value FROM sync_meta WHERE key='close-proof'",
                    [],
                    |row| row.get::<_, String>(0)
                ))
                .await
                .unwrap(),
            "accepted"
        );
        reader.close().await.unwrap();
    }

    #[tokio::test]
    async fn failed_or_panicked_drain_never_issues_a_closed_token_and_is_terminal() {
        for panic in [false, true] {
            let shutdown = WorkspaceShutdown::default();
            shutdown.begin_close(async move {
                if panic {
                    panic!("injected drain panic");
                }
                anyhow::bail!("worker join unconfirmed")
            });
            let outcome = tokio::time::timeout(Duration::from_secs(5), shutdown.wait())
                .await
                .unwrap();
            assert!(outcome.is_err());
            assert!(shutdown.wait().await.is_err());
            assert!(!shutdown.begin_close(async { Ok(None) }));
        }
    }
}
