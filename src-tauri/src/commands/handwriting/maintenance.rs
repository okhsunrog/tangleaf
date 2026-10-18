//! Application lifecycle binding for the core ink session scheduler.
use super::*;
use notes_core::ink::runtime::{Session, WorkspaceRuntime};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
type ShutdownResult = Result<(), String>;
#[derive(Clone)]
pub struct HandwritingStore {
    sessions: Arc<Mutex<HashMap<uuid::Uuid, Session>>>,
    runtime: WorkspaceRuntime,
    background: Arc<AtomicBool>,
    shutdown_started: Arc<AtomicBool>,
    shutdown: Arc<tokio::sync::watch::Sender<Option<ShutdownResult>>>,
}
impl Default for HandwritingStore {
    fn default() -> Self {
        let (shutdown, _) = tokio::sync::watch::channel(None);
        Self {
            sessions: Arc::default(),
            runtime: WorkspaceRuntime::default(),
            background: Arc::default(),
            shutdown_started: Arc::default(),
            shutdown: Arc::new(shutdown),
        }
    }
}
impl HandwritingStore {
    pub(super) fn session(&self, app: &AppHandle, page: uuid::Uuid) -> CommandResult<Session> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|e| err(anyhow::anyhow!(e.to_string())))?;
        self.runtime.ensure_accepting().map_err(err)?;
        if let Some(session) = sessions.get(&page) {
            return Ok(session.clone());
        }
        let path = app.path().app_data_dir().map_err(err)?.join("notes.db");
        let handle = app.clone();
        let session = Session::new_in_runtime(
            ink::Store::new(path, page),
            self.runtime.clone(),
            device_name(app),
            move |p| {
                emit_domain(
                    &handle,
                    DomainEvent::PagesChanged {
                        page_uuids: vec![p.page_uuid],
                    },
                );
            },
        )
        .map_err(err)?;
        sessions.insert(page, session.clone());
        Ok(session)
    }
    /// A note that no longer exists cannot be completed or reopened, so its
    /// cached session would otherwise be retried for the life of the process.
    fn forget(&self, page: uuid::Uuid) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(&page);
        }
    }
    pub(super) fn is_background(&self) -> bool {
        self.background.load(Ordering::Relaxed)
    }
    pub(super) fn finish_in_background(&self, session: Session) {
        // Admit the owned completion before spawning a logging-only waiter.
        // A delayed host task must not start raw work after shutdown.
        // Android suspend arrives on the window event thread, which need not
        // already be entered into Tokio. Admission still happens right here.
        let executor = tauri::async_runtime::handle();
        let started = {
            let _entered = executor.inner().enter();
            session.start_completion()
        };
        let completion = match started {
            Ok(completion) => completion,
            Err(error) => {
                tracing::debug!(?error, "Ink background completion refused during shutdown");
                return;
            }
        };
        let store = self.clone();
        tauri::async_runtime::spawn(async move {
            match completion.wait().await {
                Ok(()) => {}
                Err(notes_core::CoreError::NotFound(_)) => store.forget(session.page()),
                Err(error) => tracing::warn!(?error, "Ink publication deferred"),
            }
        });
    }
    pub(crate) fn set_background(&self, background: bool) {
        self.background.store(background, Ordering::Relaxed);
        if background && let Ok(sessions) = self.sessions.lock() {
            for session in sessions
                .values()
                .filter(|session| session.needs_completion())
            {
                self.finish_in_background(session.clone());
            }
        }
    }
    pub(crate) async fn complete_all(&self) -> CommandResult<()> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|e| err(anyhow::anyhow!(e.to_string())))?
            .values()
            .filter(|session| session.needs_completion())
            .cloned()
            .collect::<Vec<_>>();
        let mut failure = None;
        for session in sessions {
            match session.complete().await {
                Ok(()) => {}
                Err(notes_core::CoreError::NotFound(_)) => self.forget(session.page()),
                Err(error) => failure = Some(err(error)),
            }
        }
        failure.map_or(Ok(()), Err)
    }
    pub(crate) async fn recover(&self, app: &AppHandle, conn: &Connection) -> CommandResult<()> {
        for page in ink::recoverable_notes(conn).await.map_err(err)? {
            let session = self.session(app, page)?;
            match session.complete().await {
                Ok(()) => {}
                Err(notes_core::CoreError::NotFound(_)) => self.forget(page),
                Err(error) => {
                    tracing::warn!(?error,%page,"Interrupted ink session retained for retry");
                }
            }
        }
        Ok(())
    }
    /// Refuse new input, drain every admitted native job, and finish durable
    /// publication intent. An error still means raw file users are gone.
    pub(crate) async fn shutdown(&self) -> CommandResult<()> {
        let mut finished = self.shutdown.subscribe();
        {
            // Creation and the final session snapshot share the same host lock;
            // the core gate independently protects every retained Session clone.
            let sessions = self
                .sessions
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            self.runtime.stop();
            if !self.shutdown_started.swap(true, Ordering::AcqRel) {
                let sessions = sessions.values().cloned().collect::<Vec<_>>();
                let manager = self.clone();
                tauri::async_runtime::spawn(async move {
                    let result = manager
                        .runtime
                        .shutdown(sessions)
                        .await
                        .map_err(|error| error.to_string());
                    // Cached callbacks capture AppHandle, whose state owns this
                    // cache. Clear that cycle even if the caller stopped waiting.
                    manager
                        .sessions
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .clear();
                    manager.shutdown.send_replace(Some(result));
                });
            }
        }
        loop {
            let result = finished.borrow_and_update().clone();
            if let Some(result) = result {
                return result.map_err(err);
            }
            finished.changed().await.map_err(err)?;
        }
    }
}
pub(super) fn device_name(app: &AppHandle) -> String {
    #[cfg(target_os = "android")]
    if let Ok(name) = app.mobile_system().device_name() {
        return name;
    }
    let _ = app;
    std::env::var("COMPUTERNAME")
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| std::env::consts::OS.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::{Future, poll_fn};
    use std::sync::atomic::AtomicUsize;
    use std::task::Poll;

    struct CallbackOwner(Arc<AtomicUsize>);
    impl Drop for CallbackOwner {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn canceled_shutdown_waiter_still_drains_jobs_and_releases_cached_callbacks() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notes.db");
        let connection = db::open(&path).await.unwrap();
        let page = db::create_handwritten_note_with_ops(&connection, None)
            .await
            .unwrap()
            .value;
        let manager = HandwritingStore::default();
        let released_callbacks = Arc::new(AtomicUsize::new(0));
        let owner = CallbackOwner(released_callbacks.clone());
        let session = Session::new_in_runtime(
            ink::Store::new(&path, page.uuid),
            manager.runtime.clone(),
            "Test".into(),
            move |_| {
                let _ = &owner;
            },
        )
        .unwrap();
        session.open(true).await.unwrap();
        session
            .patch(
                ink::InkDraftPatch {
                    order: Vec::new(),
                    upserts: Vec::new(),
                    background: ink::InkBackground::Grid,
                },
                None,
            )
            .await
            .unwrap();
        manager
            .sessions
            .lock()
            .unwrap()
            .insert(page.uuid, session.clone());
        let (entered, running) = tokio::sync::oneshot::channel();
        let (release, released) = std::sync::mpsc::channel();
        let job = tokio::spawn(async move {
            session
                .access(move |store| {
                    entered.send(()).unwrap();
                    released
                        .recv_timeout(std::time::Duration::from_secs(10))
                        .unwrap();
                    store.read().map(|_| ())
                })
                .await
        });
        running.await.unwrap();
        let mut shutdown = Box::pin(manager.shutdown());
        poll_fn(|context| {
            assert!(shutdown.as_mut().poll(context).is_pending());
            Poll::Ready(())
        })
        .await;
        drop(shutdown);
        assert!(manager.runtime.ensure_accepting().is_err());
        assert_eq!(manager.sessions.lock().unwrap().len(), 1);
        assert_eq!(released_callbacks.load(Ordering::SeqCst), 0);
        release.send(()).unwrap();
        job.await.unwrap().unwrap();
        manager.shutdown().await.unwrap();
        assert!(manager.sessions.lock().unwrap().is_empty());
        assert_eq!(released_callbacks.load(Ordering::SeqCst), 1);
        manager.set_background(true);
        manager.shutdown().await.unwrap();
        assert!(
            ink::recoverable_notes(&connection)
                .await
                .unwrap()
                .is_empty()
        );
        connection.close().await.unwrap();
    }

    #[tokio::test]
    async fn failed_publication_still_clears_the_cache_and_reports_the_same_terminal_result() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notes.db");
        let connection = db::open(&path).await.unwrap();
        let page = db::create_handwritten_note_with_ops(&connection, None)
            .await
            .unwrap()
            .value;
        let manager = HandwritingStore::default();
        let session = Session::new_in_runtime(
            ink::Store::new(&path, page.uuid),
            manager.runtime.clone(),
            "Test".into(),
            |_| {},
        )
        .unwrap();
        session
            .patch(
                ink::InkDraftPatch {
                    order: Vec::new(),
                    upserts: Vec::new(),
                    background: ink::InkBackground::Grid,
                },
                None,
            )
            .await
            .unwrap();
        manager
            .sessions
            .lock()
            .unwrap()
            .insert(page.uuid, session.clone());
        connection.call(|database| database.execute_batch("CREATE TRIGGER reject_exit_publication BEFORE INSERT ON ink_versions BEGIN SELECT RAISE(ABORT,'retain for next launch'); END;")).await.unwrap();
        let first = manager.shutdown().await.unwrap_err();
        assert!(first.message.contains("retain for next launch"));
        assert!(manager.sessions.lock().unwrap().is_empty());
        assert!(session.start_completion().is_err());
        let repeated = manager.shutdown().await.unwrap_err();
        assert_eq!(first.message, repeated.message);
        assert_eq!(
            ink::recoverable_notes(&connection).await.unwrap(),
            [page.uuid]
        );
        connection.close().await.unwrap();
    }

    #[tokio::test]
    async fn background_window_callback_can_admit_completion_outside_a_tokio_thread() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notes.db");
        let connection = db::open(&path).await.unwrap();
        let page = db::create_handwritten_note_with_ops(&connection, None)
            .await
            .unwrap()
            .value;
        let manager = HandwritingStore::default();
        let session = Session::new_in_runtime(
            ink::Store::new(&path, page.uuid),
            manager.runtime.clone(),
            "Test".into(),
            |_| {},
        )
        .unwrap();
        session
            .patch(
                ink::InkDraftPatch {
                    order: Vec::new(),
                    upserts: Vec::new(),
                    background: ink::InkBackground::Grid,
                },
                None,
            )
            .await
            .unwrap();
        manager
            .sessions
            .lock()
            .unwrap()
            .insert(page.uuid, session.clone());
        let callback = manager.clone();
        std::thread::spawn(move || callback.finish_in_background(session))
            .join()
            .unwrap();
        manager.shutdown().await.unwrap();
        assert!(
            ink::recoverable_notes(&connection)
                .await
                .unwrap()
                .is_empty()
        );
        connection.close().await.unwrap();
    }
}
