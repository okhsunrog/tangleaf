//! Ownership of the application's one startup task and its opened Store.

use anyhow::Result;
use notes_core::store::NotesStore;
use std::future::Future;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

type Outcome = std::result::Result<(), String>;

struct State {
    stopping: bool,
    store: Option<NotesStore>,
}

struct Inner {
    state: Mutex<State>,
    completed: watch::Sender<Option<Outcome>>,
}

/// Registered before setup starts asynchronous database initialization. Exit
/// waits for its terminal result before draining ink/sync and closing SQLite.
#[derive(Clone)]
pub(crate) struct StartupLifecycle(Arc<Inner>);

/// Setup owns this guard until it either fails synchronously or transfers the
/// guard to the detached startup supervisor. No abort handle is exposed.
pub(crate) struct Registration {
    lifecycle: StartupLifecycle,
    finished: bool,
}

impl StartupLifecycle {
    pub(crate) fn register() -> (Self, Registration) {
        let (completed, _) = watch::channel(None);
        let lifecycle = Self(Arc::new(Inner {
            state: Mutex::new(State {
                stopping: false,
                store: None,
            }),
            completed,
        }));
        let registration = Registration {
            lifecycle: lifecycle.clone(),
            finished: false,
        };
        (lifecycle, registration)
    }

    /// Called immediately after open returns, without an intervening await.
    /// The lifecycle retains ownership even if state installation then panics.
    pub(crate) fn retain_store(&self, store: &NotesStore) {
        self.0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .store = Some(store.clone());
    }

    pub(crate) fn is_stopping(&self) -> bool {
        self.0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .stopping
    }

    /// Serialize final Ready/sync publication with the synchronous exit stop.
    /// The callback is synchronous and never awaits while holding this gate.
    pub(crate) fn while_starting<T>(&self, action: impl FnOnce() -> T) -> Option<T> {
        let state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.stopping {
            return None;
        }
        Some(action())
    }

    pub(crate) fn stop(&self) {
        self.0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .stopping = true;
    }

    pub(crate) async fn wait(&self) -> Outcome {
        let mut completed = self.0.completed.subscribe();
        loop {
            let outcome = completed.borrow_and_update().clone();
            if let Some(outcome) = outcome {
                return outcome;
            }
            completed
                .changed()
                .await
                .map_err(|error| error.to_string())?;
        }
    }

    /// Available after the completion barrier, including a failed startup that
    /// opened SQLite but never installed AppState. The exit coordinator alone
    /// closes it after all admitted raw ink and sync users have drained.
    pub(crate) fn opened_store(&self) -> Option<NotesStore> {
        self.0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .store
            .clone()
    }
}

impl Registration {
    pub(crate) fn finish(mut self, outcome: Outcome) {
        self.finished = true;
        self.lifecycle.0.completed.send_replace(Some(outcome));
    }

    pub(crate) fn spawn(
        self,
        work: impl Future<Output = Result<()>> + Send + 'static,
        report_failure: impl FnOnce(String) + Send + 'static,
    ) {
        self.supervise(tauri::async_runtime::spawn(work), report_failure);
    }

    fn supervise(
        self,
        work: tauri::async_runtime::JoinHandle<Result<()>>,
        report_failure: impl FnOnce(String) + Send + 'static,
    ) {
        tauri::async_runtime::spawn(async move {
            let outcome = match work.await {
                Ok(result) => result.map_err(|error| format!("{error:#}")),
                Err(error) => Err(format!("Startup task failed: {error}")),
            };
            if let Err(message) = &outcome {
                self.lifecycle
                    .while_starting(|| report_failure(message.clone()));
            }
            self.finish(outcome);
        });
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        if !self.finished {
            self.lifecycle.0.completed.send_replace(Some(Err(
                "Startup registration ended before initialization completed".into(),
            )));
        }
    }
}

#[cfg(test)]
#[path = "startup_tests.rs"]
mod tests;
