//! Admission and draining for every raw connection used by one workspace.

use super::*;
use tokio::sync::{Notify, oneshot, watch};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Open,
    Stopping,
    Closed,
}

struct State {
    phase: Phase,
    jobs: usize,
    tasks: usize,
    shutdown_started: bool,
}

type ShutdownResult = Result<(), Arc<str>>;

struct Inner {
    state: Mutex<State>,
    changed: Notify,
    stopped: Notify,
    shutdown: watch::Sender<Option<ShutdownResult>>,
}

/// One gate and packing lane shared by all raw ink sessions in a workspace.
#[derive(Clone)]
pub struct WorkspaceRuntime {
    inner: Arc<Inner>,
    pub(super) lane: Arc<tokio::sync::Mutex<()>>,
}

impl Default for WorkspaceRuntime {
    fn default() -> Self {
        Self::with_lane(Arc::default())
    }
}

#[derive(Clone, Copy)]
enum PermitKind {
    Job,
    Task,
}

/// The closure/task owns this registration; an awaiting caller never does.
pub(super) struct Permit {
    runtime: WorkspaceRuntime,
    kind: PermitKind,
}

impl Drop for Permit {
    fn drop(&mut self) {
        let mut state = self
            .runtime
            .inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match self.kind {
            PermitKind::Job => state.jobs -= 1,
            PermitKind::Task => state.tasks -= 1,
        }
        drop(state);
        self.runtime.inner.changed.notify_waiters();
    }
}

/// Waiting is cancelable; the admitted completion and its notification are not.
pub struct Completion(oneshot::Receiver<CommandResult<()>>);

impl Completion {
    pub async fn wait(self) -> CommandResult<()> {
        self.0.await.map_err(err)?
    }
}

impl WorkspaceRuntime {
    pub(super) fn with_lane(lane: Arc<tokio::sync::Mutex<()>>) -> Self {
        let (shutdown, _) = watch::channel(None);
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    phase: Phase::Open,
                    jobs: 0,
                    tasks: 0,
                    shutdown_started: false,
                }),
                changed: Notify::new(),
                stopped: Notify::new(),
                shutdown,
            }),
            lane,
        }
    }

    pub fn ensure_accepting(&self) -> CommandResult<()> {
        let state = self.inner.state.lock().map_err(err)?;
        if state.phase == Phase::Open {
            Ok(())
        } else {
            Err(CommandError::conflict(
                "Handwriting workspace is shutting down",
            ))
        }
    }

    pub(super) fn new_session(
        &self,
        store: Store,
        device_name: String,
        on_publish: impl Fn(Publish) + Send + Sync + 'static,
    ) -> CommandResult<Session> {
        let state = self.inner.state.lock().map_err(err)?;
        if state.phase != Phase::Open {
            return Err(CommandError::conflict(
                "Handwriting workspace is shutting down",
            ));
        }
        Ok(Session {
            store,
            input: Arc::default(),
            modified: Arc::default(),
            editing: Arc::default(),
            pending: Arc::default(),
            lane: self.lane.clone(),
            runtime: self.clone(),
            wake: Arc::default(),
            device_name,
            on_publish: Arc::new(on_publish),
        })
    }

    fn admit(&self, kind: PermitKind, finishing: bool) -> CommandResult<Permit> {
        let mut state = self.inner.state.lock().map_err(err)?;
        if state.phase != Phase::Open && !(finishing && state.phase == Phase::Stopping) {
            return Err(CommandError::conflict(
                "Handwriting workspace is shutting down",
            ));
        }
        match kind {
            PermitKind::Job => state.jobs += 1,
            PermitKind::Task => state.tasks += 1,
        }
        Ok(Permit {
            runtime: self.clone(),
            kind,
        })
    }

    pub(super) fn job(&self, finishing: bool) -> CommandResult<Permit> {
        self.admit(PermitKind::Job, finishing)
    }

    pub(super) fn task(&self) -> CommandResult<Permit> {
        self.admit(PermitKind::Task, false)
    }

    pub(super) fn completion(&self, session: Session) -> CommandResult<Completion> {
        let permit = self.task()?;
        // Enqueue publication intent at the command boundary, before spawning
        // its owned continuation, just like ordinary per-note access.
        let requested = session.request_publication(false)?;
        let (finished, completion) = oneshot::channel();
        tokio::spawn(async move {
            let _permit = permit;
            let result = match requested.await.map_err(err).and_then(|result| result) {
                Ok(()) => session.finish_publication(false).await,
                Err(error) => Err(error),
            };
            let _ = finished.send(result);
        });
        Ok(Completion(completion))
    }

    /// Stop is synchronous so the host can hold its session-cache lock while
    /// closing admission and taking the final set of sessions to drain.
    pub fn stop(&self) {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.phase == Phase::Open {
            state.phase = Phase::Stopping;
        }
        drop(state);
        self.inner.stopped.notify_waiters();
    }

    pub(super) async fn stopped(&self) {
        let stopped = self.inner.stopped.notified();
        tokio::pin!(stopped);
        stopped.as_mut().enable();
        if self.ensure_accepting().is_err() {
            return;
        }
        stopped.await;
    }

    async fn drain(&self) {
        loop {
            let changed = self.inner.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let state = self
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if state.jobs == 0 && state.tasks == 0 {
                    return;
                }
            }
            changed.await;
        }
    }

    /// Errors report retained publication intent only after all admitted raw
    /// jobs have finished. Dropping a waiter never cancels this shutdown.
    pub async fn shutdown(&self, sessions: Vec<Session>) -> CommandResult<()> {
        self.stop();
        let mut completed = self.inner.shutdown.subscribe();
        let start = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let start = !state.shutdown_started;
            state.shutdown_started = true;
            start
        };
        if start {
            let runtime = self.clone();
            let finishing = tokio::spawn(async move {
                runtime.drain().await;
                let mut failure = None;
                for session in sessions {
                    if !Arc::ptr_eq(&runtime.inner, &session.runtime.inner) {
                        failure = Some("Ink session belongs to another workspace runtime".into());
                        continue;
                    }
                    if session.needs_completion() {
                        match session.complete_inner(true).await {
                            Ok(()) | Err(CommandError::NotFound(_)) => {}
                            Err(error) => failure = Some(error.to_string()),
                        }
                    }
                }
                failure.map_or(Ok(()), |error| Err(Arc::<str>::from(error)))
            });
            let runtime = self.clone();
            tokio::spawn(async move {
                let result = finishing
                    .await
                    .unwrap_or_else(|error| Err(error.to_string().into()));
                // Even a panicking continuation can leave a blocking closure
                // alive. Its registration and owned lane must finish first.
                runtime.drain().await;
                runtime
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .phase = Phase::Closed;
                runtime.inner.shutdown.send_replace(Some(result));
            });
        }
        loop {
            let result = completed.borrow_and_update().clone();
            if let Some(result) = result {
                return result.map_err(err);
            }
            completed.changed().await.map_err(err)?;
        }
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
