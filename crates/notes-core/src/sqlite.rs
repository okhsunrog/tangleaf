use anyhow::{Context, Result, anyhow};
use std::future::Future;
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{JoinHandle, ThreadId};
use std::time::{Duration, Instant};
use tokio::sync::{Semaphore, watch};

pub mod policy;

type Job = Box<dyn FnOnce(&mut rusqlite::Connection) + Send + 'static>;

enum WorkerMessage {
    Call(Job),
    Close,
}

// Bound accepted outstanding work, including the job currently executing.
// This is a resource limit, not a claim about the optimal throughput: at most
// 63 jobs queue behind the running job. Callers waiting for admission hold no
// connection or blocking-pool thread and may still cancel without doing SQL.
const MAX_ACCEPTED_JOBS: usize = 64;

type CloseOutcome = std::result::Result<(), Arc<str>>;

struct AdmissionGate {
    sender: Option<mpsc::Sender<WorkerMessage>>,
    thread: Option<JoinHandle<CloseOutcome>>,
}

struct Worker {
    admission: Arc<Semaphore>,
    gate: Mutex<AdmissionGate>,
    closed: watch::Sender<Option<CloseOutcome>>,
    thread_id: ThreadId,
}

impl Worker {
    fn begin_close(&self) {
        // Closing the semaphore wakes unaccepted callers; the gate orders the
        // last accepted send before Close, including callers that acquired a
        // permit just before the semaphore was closed.
        self.admission.close();
        let thread = {
            let mut gate = self.gate.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(sender) = gate.sender.take() {
                let _ = sender.send(WorkerMessage::Close);
            }
            gate.thread.take()
        };
        let Some(thread) = thread else {
            return;
        };
        // Never join from a runtime or SQLite thread. The reaper owns only the
        // JoinHandle and completion sender, so it cannot keep Worker alive in
        // an Arc cycle. Dropping any close waiter cannot interrupt this work.
        let closed = self.closed.clone();
        let spawn = std::thread::Builder::new()
            .name("tangleaf-sqlite-close".into())
            .spawn(move || {
                let outcome = thread.join().unwrap_or_else(|payload| {
                    Err(format!("SQLite worker panicked: {}", panic_message(&*payload)).into())
                });
                closed.send_replace(Some(outcome));
            });
        if let Err(error) = spawn {
            // Close was already sent. The detached worker can still drain and
            // close, but without a reaper we cannot promise that it was joined.
            self.closed.send_replace(Some(Err(format!(
                "spawning SQLite close thread failed; worker join unconfirmed: {error}"
            )
            .into())));
        }
    }

    async fn wait_closed(&self) -> Result<()> {
        let mut closed = self.closed.subscribe();
        loop {
            let outcome = closed.borrow_and_update().clone();
            if let Some(outcome) = outcome {
                return outcome.map_err(|error| anyhow!("{error}"));
            }
            closed
                .changed()
                .await
                .context("SQLite close completion channel stopped")?;
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.begin_close();
    }
}

/// Cloneable asynchronous handle backed by one dedicated SQLite thread.
///
/// Calls asynchronously wait for bounded admission and then in the connection's
/// channel, without occupying Tokio blocking-pool threads. Closures may return
/// either `rusqlite::Error` or a typed domain error; both remain available in
/// the returned `anyhow::Error` chain.
#[derive(Clone)]
pub struct Connection {
    worker: Arc<Worker>,
}

impl Connection {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_owned();
        Self::open_with(move || Ok(rusqlite::Connection::open(path)?)).await
    }

    pub async fn open_writer(
        path: impl AsRef<Path>,
        durability: policy::WriterDurability,
    ) -> Result<Self> {
        let path = path.as_ref().to_owned();
        Self::open_with(move || {
            let connection = rusqlite::Connection::open(path)?;
            policy::configure_writer(&connection, durability)?;
            Ok(connection)
        })
        .await
    }

    /// Open an existing database without creating it, migrating it, or changing
    /// its persistent journal mode. The worker and statement profiling are the
    /// same as for writers; query_only additionally forbids temporary writes.
    pub async fn open_read_only(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_owned();
        Self::open_with(move || {
            let connection = rusqlite::Connection::open_with_flags(
                path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                    | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            policy::configure_read_only(&connection)?;
            Ok(connection)
        })
        .await
    }

    pub(crate) async fn open_in_memory() -> Result<Self> {
        Self::open_with(|| Ok(rusqlite::Connection::open_in_memory()?)).await
    }

    async fn open_with(
        opener: impl FnOnce() -> Result<rusqlite::Connection> + Send + 'static,
    ) -> Result<Self> {
        Self::open_with_capacity(opener, MAX_ACCEPTED_JOBS).await
    }

    async fn open_with_capacity(
        opener: impl FnOnce() -> Result<rusqlite::Connection> + Send + 'static,
        capacity: usize,
    ) -> Result<Self> {
        let (connection, opened_rx) = Self::start_worker(opener, capacity)?;
        // Own the shutdown guard before waiting for the opener. Cancellation,
        // an opener error, or an opener panic must all reap the spawned thread.
        opened_rx
            .await
            .context("SQLite worker stopped while opening")??;
        Ok(connection)
    }

    fn start_worker(
        opener: impl FnOnce() -> Result<rusqlite::Connection> + Send + 'static,
        capacity: usize,
    ) -> Result<(Self, tokio::sync::oneshot::Receiver<Result<()>>)> {
        assert!(capacity > 0, "SQLite admission capacity must be positive");
        let (sender, receiver) = mpsc::channel::<WorkerMessage>();
        let (opened_tx, opened_rx) = tokio::sync::oneshot::channel();
        let thread = std::thread::Builder::new()
            .name("tangleaf-sqlite".into())
            .spawn(move || match opener() {
                Ok(mut connection) => {
                    install_statement_profile(&mut connection);
                    if opened_tx.send(Ok(())).is_ok() {
                        while let Ok(message) = receiver.recv() {
                            match message {
                                WorkerMessage::Call(job) => {
                                    // The inner handler returns typed function
                                    // failures to the waiter. Destructors of an
                                    // undelivered result or closure captures can
                                    // still panic outside that handler; keep
                                    // draining work already accepted after it.
                                    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                        job(&mut connection)
                                    }))
                                    .is_err()
                                    {
                                        tracing::error!(
                                            "SQLite job panicked outside its result handler; worker continues"
                                        );
                                    }
                                }
                                WorkerMessage::Close => break,
                            }
                        }
                    }
                    close_database(connection)
                }
                Err(error) => {
                    let _ = opened_tx.send(Err(error));
                    Ok(())
                }
            })
            .context("spawning SQLite worker thread")?;
        let thread_id = thread.thread().id();
        let (closed, _) = watch::channel(None);
        Ok((
            Self {
                worker: Arc::new(Worker {
                    admission: Arc::new(Semaphore::new(capacity)),
                    gate: Mutex::new(AdmissionGate {
                        sender: Some(sender),
                        thread: Some(thread),
                    }),
                    closed,
                    thread_id,
                }),
            },
            opened_rx,
        ))
    }

    /// Stop admission, drain every accepted job, close SQLite, and join the
    /// dedicated thread. All clones observe the same result. Once this future
    /// is polled, dropping it does not cancel shutdown. Retained clones cannot
    /// submit new work after shutdown starts. Last-handle Drop also starts the
    /// same cleanup, but explicit close lets the caller observe its completion.
    pub async fn close(&self) -> Result<()> {
        if std::thread::current().id() == self.worker.thread_id {
            return Err(anyhow!(
                "cannot await SQLite close from its own worker thread"
            ));
        }
        self.worker.begin_close();
        self.worker.wait_closed().await
    }

    /// Not `async fn`: `#[track_caller]` is a no-op on those, and the caller's location is what
    /// makes a slow-call warning useful.
    #[track_caller]
    pub fn call<F, R>(&self, function: F) -> impl Future<Output = Result<R>> + '_
    where
        F: FnOnce(&mut rusqlite::Connection) -> rusqlite::Result<R> + Send + 'static,
        R: Send + 'static,
    {
        self.call_at(std::panic::Location::caller(), function)
    }

    #[track_caller]
    pub fn call_domain<F, R, E>(&self, function: F) -> impl Future<Output = Result<R>> + '_
    where
        F: FnOnce(&mut rusqlite::Connection) -> std::result::Result<R, E> + Send + 'static,
        R: Send + 'static,
        E: Into<anyhow::Error> + Send + 'static,
    {
        self.call_at(std::panic::Location::caller(), function)
    }

    async fn call_at<F, R, E>(
        &self,
        caller: &'static std::panic::Location<'static>,
        function: F,
    ) -> Result<R>
    where
        F: FnOnce(&mut rusqlite::Connection) -> std::result::Result<R, E> + Send + 'static,
        R: Send + 'static,
        E: Into<anyhow::Error> + Send + 'static,
    {
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        // Capture at submission, when this future is polled, rather than when
        // the handle or future was created. The worker thread has its own
        // tracing context, including for SQLite's statement-profile callback.
        let span = tracing::Span::current();
        let dispatch = tracing::dispatcher::get_default(Clone::clone);
        let requested_at = Instant::now();
        let permit = self
            .worker
            .admission
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| anyhow!("SQLite connection is closing or closed"))?;
        {
            let gate = self
                .worker
                .gate
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let sender = gate
                .sender
                .as_ref()
                .ok_or_else(|| anyhow!("SQLite connection is closing or closed"))?;
            let queued_at = Instant::now();
            sender
                .send(WorkerMessage::Call(Box::new(move |connection| {
                    let started = Instant::now();
                    tracing::dispatcher::with_default(&dispatch, || {
                        let _entered = span.enter();
                        // One bad job must not unwind the worker thread. A panic
                        // mid-transaction still rolls back when the transaction is
                        // dropped. Dropping the receiving future does not cancel
                        // an accepted job or skip its diagnostic events.
                        let outcome =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                function(connection).map_err(Into::into)
                            }))
                            .unwrap_or_else(|payload| {
                                Err(anyhow!("SQLite job panicked: {}", panic_message(&*payload)))
                            });
                        CallTimings::new(requested_at, queued_at, started, Instant::now())
                            .report(caller);
                        let _ = result_tx.send(outcome);
                    });
                    // Acceptance transfers admission to the job, not its waiter.
                    // Preserve accepted writes even if the receiving future drops.
                    drop(permit);
                })))
                .map_err(|_| anyhow!("SQLite connection worker has stopped"))?;
        }
        result_rx
            .await
            .context("SQLite connection worker dropped a call")?
    }
}

fn close_database(connection: rusqlite::Connection) -> CloseOutcome {
    connection.close().map_err(|(connection, error)| {
        let message = format!("closing SQLite connection failed: {error}");
        // Even an explicit close failure must release the owned connection
        // before reporting thread completion.
        drop(connection);
        message.into()
    })
}

/// Report calls that either wait in the worker queue or occupy the connection
/// for this long. These are separate causes, even if both affect the same call.
const SLOW_CALL: Duration = Duration::from_millis(500);

struct CallTimings {
    admission: Duration,
    queue: Duration,
    execution: Duration,
    worker_total: Duration,
}

impl CallTimings {
    fn new(requested_at: Instant, queued_at: Instant, started: Instant, finished: Instant) -> Self {
        Self {
            admission: queued_at.duration_since(requested_at),
            queue: started.duration_since(queued_at),
            execution: finished.duration_since(started),
            worker_total: finished.duration_since(queued_at),
        }
    }

    fn report(&self, caller: &'static std::panic::Location<'static>) {
        // Admission is the cancellable semaphore/gate wait before acceptance.
        // Queue time includes channel submission and worker scheduling, not a
        // SQLite lock wait. Execution includes the closure, panic handling and
        // tracing-context entry; any SQLite busy/lock wait happens here. Total
        // ends before logging/result delivery and excludes the awaiting Tokio
        // task's scheduling delay, IPC and UI work.
        let admission_ms = self.admission.as_secs_f64() * 1_000.0;
        let queue_ms = self.queue.as_secs_f64() * 1_000.0;
        let execution_ms = self.execution.as_secs_f64() * 1_000.0;
        let worker_total_ms = self.worker_total.as_secs_f64() * 1_000.0;
        tracing::debug!(
            caller = %caller,
            admission_ms,
            queue_ms,
            execution_ms,
            worker_total_ms,
            "database call completed"
        );
        if self.admission >= SLOW_CALL {
            tracing::warn!(
                caller = %caller,
                admission_ms,
                "database call waited for admission"
            );
        }
        if self.queue >= SLOW_CALL {
            tracing::warn!(
                caller = %caller,
                queue_ms,
                execution_ms,
                worker_total_ms,
                "database call waited in worker queue"
            );
        }
        if self.execution >= SLOW_CALL {
            tracing::warn!(
                caller = %caller,
                queue_ms,
                execution_ms,
                worker_total_ms,
                // Preserve the existing slow-call field's execution meaning.
                elapsed_ms = self.execution.as_millis() as u64,
                "slow database call blocked the connection"
            );
        }
    }
}

/// Individual statements are far smaller than whole calls; this catches the one statement that
/// explains a slow call, without logging a line per row in a loop of fast ones.
const SLOW_STATEMENT: Duration = Duration::from_millis(200);

/// Log the SQL of any single statement that runs long. `profile` reports each statement once it
/// finishes, with the text SQLite actually executed, so a slow call can be attributed to the
/// statement inside it rather than guessed at from the call site alone.
fn install_statement_profile(connection: &mut rusqlite::Connection) {
    connection.trace_v2(
        rusqlite::trace::TraceEventCodes::SQLITE_TRACE_PROFILE,
        Some(|event: rusqlite::trace::TraceEvent<'_>| {
            let rusqlite::trace::TraceEvent::Profile(statement, elapsed) = event else {
                return;
            };
            if elapsed >= SLOW_STATEMENT {
                report_statement_profile(&statement.sql(), elapsed);
            }
        }),
    );
}

fn report_statement_profile(sql: &str, elapsed: Duration) {
    if elapsed < SLOW_STATEMENT {
        return;
    }
    tracing::warn!(
        elapsed_ms = elapsed.as_millis() as u64,
        sql = %sql.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(200).collect::<String>(),
        "slow SQL statement"
    );
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown payload".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    const TEST_TIMEOUT: Duration = Duration::from_secs(10);
    use std::collections::BTreeMap;
    use std::fmt::Debug;
    use std::task::{Poll, Waker};
    use tracing::Instrument;
    use tracing::instrument::WithSubscriber;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::registry::LookupSpan;

    #[derive(Debug, thiserror::Error)]
    #[error("typed domain failure")]
    struct DomainFailure;

    #[derive(Clone, Debug)]
    struct CapturedEvent {
        level: tracing::Level,
        fields: BTreeMap<String, String>,
        spans: Vec<String>,
    }

    impl CapturedEvent {
        fn message_contains(&self, message: &str) -> bool {
            self.fields
                .get("message")
                .is_some_and(|value| value.contains(message))
        }

        fn milliseconds(&self, name: &str) -> f64 {
            self.fields[name].parse().expect("numeric timing field")
        }
    }

    #[derive(Clone, Default)]
    struct CapturedEvents(Arc<std::sync::Mutex<Vec<CapturedEvent>>>);

    impl CapturedEvents {
        fn dispatch(&self) -> tracing::Dispatch {
            // tracing-core's single-dispatch fast path registers a new
            // callsite against the current thread's dispatcher. Other tests
            // can first hit that callsite on an untraced worker while this
            // scoped subscriber exists elsewhere, caching Interest::never.
            // Keep a second registered dispatcher alive so interest is
            // aggregated across dispatchers. It is never installed as a
            // thread-local or global default and receives no events.
            static INTEREST_ANCHOR: std::sync::OnceLock<tracing::Dispatch> =
                std::sync::OnceLock::new();
            INTEREST_ANCHOR.get_or_init(|| {
                tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default())
            });
            tracing::Dispatch::new(tracing_subscriber::registry().with(self.clone()))
        }

        fn snapshot(&self) -> Vec<CapturedEvent> {
            self.0.lock().expect("events").clone()
        }
    }

    impl<S> tracing_subscriber::Layer<S> for CapturedEvents
    where
        S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
    {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            context: tracing_subscriber::layer::Context<'_, S>,
        ) {
            struct Collect(BTreeMap<String, String>);
            impl tracing::field::Visit for Collect {
                fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn Debug) {
                    self.0.insert(field.name().to_owned(), format!("{value:?}"));
                }
            }
            let mut collected = Collect(BTreeMap::new());
            event.record(&mut collected);
            let spans = context
                .event_scope(event)
                .map(|scope| {
                    scope
                        .from_root()
                        .map(|span| span.name().to_owned())
                        .collect()
                })
                .unwrap_or_default();
            self.0.lock().expect("events").push(CapturedEvent {
                level: *event.metadata().level(),
                fields: collected.0,
                spans,
            });
        }
    }

    #[tokio::test]
    async fn scoped_dispatch_and_command_span_follow_each_job_without_leaking() {
        let connection = Connection::open_in_memory().await.expect("open");
        let first = CapturedEvents::default();
        let first_dispatch = first.dispatch();
        let span = tracing::dispatcher::with_default(&first_dispatch, || {
            tracing::info_span!("editing_command")
        });
        connection
            .call(|database| {
                database.execute_batch("CREATE TABLE values_table (value INTEGER NOT NULL)")?;
                // Exercise the same reporter used by SQLite's profile callback
                // without depending on a particular machine's query speed.
                report_statement_profile("SELECT   1", SLOW_STATEMENT);
                Ok(())
            })
            .instrument(span)
            .with_subscriber(first_dispatch)
            .await
            .expect("first scoped call");

        let first_events = first.snapshot();
        assert!(
            first_events
                .iter()
                .all(|event| event.spans == ["editing_command"])
        );
        let profile = first_events
            .iter()
            .find(|event| {
                event.message_contains("slow SQL statement")
                    && event.fields.get("sql").is_some_and(|sql| sql == "SELECT 1")
            })
            .expect("profile event on the submitting subscriber");
        assert_eq!(profile.fields["sql"], "SELECT 1");
        let completed = first_events
            .iter()
            .find(|event| event.message_contains("database call completed"))
            .expect("completed event");
        assert!(completed.fields["caller"].contains("sqlite.rs"));

        let second = CapturedEvents::default();
        connection
            .call(|_| {
                tracing::info!("second subscriber's call");
                Ok(())
            })
            .with_subscriber(second.dispatch())
            .await
            .expect("second scoped call");
        assert_eq!(first.snapshot().len(), first_events.len());
        let second_events = second.snapshot();
        assert!(
            second_events
                .iter()
                .any(|event| event.message_contains("second subscriber's call"))
        );
        assert!(
            second_events
                .iter()
                .any(|event| event.message_contains("database call completed"))
        );
        assert!(second_events.iter().all(|event| event.spans.is_empty()));

        connection
            .call(|_| {
                tracing::info!("unscoped call");
                Ok(())
            })
            .await
            .expect("unscoped call");
        assert_eq!(first.snapshot().len(), first_events.len());
        assert_eq!(second.snapshot().len(), second_events.len());
    }

    #[test]
    fn timings_separate_admission_queue_and_execution_and_warn_independently() {
        let events = CapturedEvents::default();
        let dispatch = events.dispatch();
        let base = Instant::now();
        let cases = [
            (
                Duration::ZERO,
                Duration::from_millis(70),
                Duration::from_millis(20),
            ),
            (SLOW_CALL, Duration::ZERO, Duration::from_millis(10)),
            (Duration::ZERO, SLOW_CALL, Duration::from_millis(10)),
            (Duration::ZERO, Duration::from_millis(10), SLOW_CALL),
            (SLOW_CALL, SLOW_CALL, SLOW_CALL),
        ];
        for (admission, queue, execution) in cases {
            let timings = CallTimings::new(
                base,
                base + admission,
                base + admission + queue,
                base + admission + queue + execution,
            );
            assert_eq!(timings.admission, admission);
            assert_eq!(timings.queue, queue);
            assert_eq!(timings.execution, execution);
            assert_eq!(timings.worker_total, queue + execution);
            tracing::dispatcher::with_default(&dispatch, || {
                timings.report(std::panic::Location::caller());
            });
        }
        let events = events.snapshot();
        let completions: Vec<_> = events
            .iter()
            .filter(|event| event.level == tracing::Level::DEBUG)
            .collect();
        assert_eq!(completions.len(), cases.len());
        for (event, (admission, queue, execution)) in completions.iter().zip(cases) {
            assert_eq!(
                event.milliseconds("admission_ms"),
                admission.as_secs_f64() * 1_000.0
            );
            assert_eq!(
                event.milliseconds("queue_ms"),
                queue.as_secs_f64() * 1_000.0
            );
            assert_eq!(
                event.milliseconds("execution_ms"),
                execution.as_secs_f64() * 1_000.0
            );
            assert_eq!(
                event.milliseconds("worker_total_ms"),
                (queue + execution).as_secs_f64() * 1_000.0
            );
        }
        assert_eq!(
            events
                .iter()
                .filter(|event| event.message_contains("waited for admission"))
                .count(),
            2
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event.message_contains("waited in worker queue"))
                .count(),
            2
        );
        let slow_calls: Vec<_> = events
            .iter()
            .filter(|event| event.message_contains("slow database call"))
            .collect();
        assert_eq!(slow_calls.len(), 2);
        assert!(
            slow_calls
                .iter()
                .all(|event| event.fields["elapsed_ms"] == "500")
        );
    }

    #[test]
    fn statement_profile_reports_only_statements_at_the_threshold() {
        let events = CapturedEvents::default();
        tracing::dispatcher::with_default(&events.dispatch(), || {
            report_statement_profile("SELECT 0", SLOW_STATEMENT - Duration::from_nanos(1));
            report_statement_profile("SELECT\n  1", SLOW_STATEMENT);
        });
        let events = events.snapshot();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].fields["sql"], "SELECT 1");
        assert_eq!(events[0].fields["elapsed_ms"], "200");
    }

    #[tokio::test]
    async fn waiting_in_the_worker_queue_is_measured_separately() {
        let connection = Connection::open_in_memory().await.expect("open");
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let mut blocker = Box::pin(connection.call(move |_| {
            entered_tx.send(()).expect("signal worker entry");
            release_rx.recv().expect("release worker");
            Ok(())
        }));
        let mut context = std::task::Context::from_waker(Waker::noop());
        assert!(matches!(blocker.as_mut().poll(&mut context), Poll::Pending));
        entered_rx.recv().expect("blocker owns connection");

        let events = CapturedEvents::default();
        let (ran_tx, ran_rx) = mpsc::channel();
        let mut queued = Box::pin(
            connection
                .call(move |_| {
                    ran_tx.send(()).expect("signal queued job");
                    Ok(())
                })
                .with_subscriber(events.dispatch()),
        );
        assert!(matches!(queued.as_mut().poll(&mut context), Poll::Pending));
        let queued_after = Instant::now();
        assert_eq!(ran_rx.try_recv(), Err(mpsc::TryRecvError::Empty));
        let released_before = Instant::now();
        release_tx.send(()).expect("release worker");
        blocker.await.expect("blocker finished");
        queued.await.expect("queued job finished");
        ran_rx.recv().expect("queued job ran");

        let events = events.snapshot();
        let completed = events
            .iter()
            .find(|event| event.message_contains("database call completed"))
            .expect("queued call timing");
        // This lower bound comes from the latches, with no minimum workload
        // duration or upper bound tied to the speed of the test machine.
        let known_wait_ms = released_before.duration_since(queued_after).as_secs_f64() * 1_000.0;
        assert!(completed.milliseconds("queue_ms") >= known_wait_ms);
        let total_parts =
            completed.milliseconds("queue_ms") + completed.milliseconds("execution_ms");
        assert!((completed.milliseconds("worker_total_ms") - total_parts).abs() < 0.000_001);
    }

    #[tokio::test]
    async fn dropping_a_queued_future_does_not_cancel_the_accepted_job() {
        let connection = Connection::open_in_memory().await.expect("open");
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let mut blocker = Box::pin(connection.call(move |database| {
            database.execute_batch("CREATE TABLE values_table (value INTEGER NOT NULL)")?;
            entered_tx.send(()).expect("signal worker entry");
            release_rx.recv().expect("release worker");
            Ok(())
        }));
        let mut context = std::task::Context::from_waker(Waker::noop());
        assert!(matches!(blocker.as_mut().poll(&mut context), Poll::Pending));
        entered_rx.recv().expect("blocker owns connection");

        let events = CapturedEvents::default();
        let mut queued = Box::pin(
            connection
                .call(|database| {
                    database.execute("INSERT INTO values_table (value) VALUES (42)", [])?;
                    Ok(())
                })
                .with_subscriber(events.dispatch()),
        );
        assert!(matches!(queued.as_mut().poll(&mut context), Poll::Pending));
        drop(queued);
        release_tx.send(()).expect("release worker");
        blocker.await.expect("blocker finished");
        let value: i64 = connection
            .call(|database| {
                database.query_row("SELECT value FROM values_table", [], |row| row.get(0))
            })
            .await
            .expect("accepted write finished before the next call");
        assert_eq!(value, 42);
        assert!(
            events
                .snapshot()
                .iter()
                .any(|event| event.message_contains("database call completed"))
        );
    }

    #[tokio::test]
    async fn cloned_handles_serialize_database_calls() {
        let file = tempfile::NamedTempFile::new().expect("temporary database");
        let connection = Connection::open(file.path()).await.expect("open database");
        connection
            .call(|db| {
                db.execute("CREATE TABLE values_table (value INTEGER NOT NULL)", [])?;
                Ok::<_, rusqlite::Error>(())
            })
            .await
            .expect("create table");

        let writes = (0..8).map(|value| {
            let connection = connection.clone();
            tokio::spawn(async move {
                connection
                    .call(move |db| {
                        db.execute("INSERT INTO values_table (value) VALUES (?1)", [value])?;
                        Ok::<_, rusqlite::Error>(())
                    })
                    .await
            })
        });
        for write in writes {
            write.await.expect("join write task").expect("insert value");
        }

        let count: i64 = connection
            .call(|db| db.query_row("SELECT COUNT(*) FROM values_table", [], |row| row.get(0)))
            .await
            .expect("count rows");
        assert_eq!(count, 8);
    }

    #[tokio::test]
    async fn a_panicking_job_fails_only_itself() {
        let file = tempfile::NamedTempFile::new().expect("temporary database");
        let connection = Connection::open(file.path()).await.expect("open database");
        connection
            .call(|db| {
                db.execute("CREATE TABLE values_table (value INTEGER NOT NULL)", [])?;
                Ok::<_, rusqlite::Error>(())
            })
            .await
            .expect("create table");

        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let error = connection
            .call::<_, ()>(|_| panic!("bad job"))
            .await
            .expect_err("a panicking job reports an error");
        std::panic::set_hook(previous);
        assert!(error.to_string().contains("bad job"), "{error}");

        // The connection is shared by the whole process; one bad job must not
        // take it down with it.
        let count: i64 = connection
            .call(|db| db.query_row("SELECT COUNT(*) FROM values_table", [], |row| row.get(0)))
            .await
            .expect("the worker still answers");
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn preserves_typed_errors_from_database_calls() {
        let file = tempfile::NamedTempFile::new().expect("temporary database");
        let connection = Connection::open(file.path()).await.expect("open database");
        let error = connection
            .call_domain(|_| Err::<(), _>(DomainFailure))
            .await
            .expect_err("domain failure");
        assert!(error.downcast_ref::<DomainFailure>().is_some());
    }
    #[tokio::test]
    async fn admission_wait_is_separate_from_time_in_the_worker() {
        let connection =
            Connection::open_with_capacity(|| Ok(rusqlite::Connection::open_in_memory()?), 1)
                .await
                .unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let mut first = Box::pin(connection.call(move |_| {
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(TEST_TIMEOUT).unwrap();
            Ok(())
        }));
        assert!(
            first
                .as_mut()
                .poll(&mut std::task::Context::from_waker(Waker::noop()))
                .is_pending()
        );
        entered_rx.recv_timeout(TEST_TIMEOUT).unwrap();
        let events = CapturedEvents::default();
        let mut waiting = Box::pin(tracing::instrument::WithSubscriber::with_subscriber(
            connection.call(|_| Ok(())),
            events.dispatch(),
        ));
        assert!(
            waiting
                .as_mut()
                .poll(&mut std::task::Context::from_waker(Waker::noop()))
                .is_pending()
        );
        let admission_after = Instant::now();
        let released_before = Instant::now();
        release_tx.send(()).unwrap();
        first.await.unwrap();
        // Polling once more proves this caller was reawakened after admission.
        waiting.await.unwrap();
        let events = events.snapshot();
        let completed = events
            .iter()
            .find(|event| event.message_contains("database call completed"))
            .unwrap();
        let known_wait = released_before
            .duration_since(admission_after)
            .as_secs_f64()
            * 1_000.0;
        assert!(completed.milliseconds("admission_ms") >= known_wait);
        let total_parts =
            completed.milliseconds("queue_ms") + completed.milliseconds("execution_ms");
        assert!((completed.milliseconds("worker_total_ms") - total_parts).abs() < 0.000_001);
        connection.close().await.unwrap();
    }
}

#[cfg(test)]
mod lifecycle_tests;
