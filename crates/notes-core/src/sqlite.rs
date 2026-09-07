use anyhow::{Context, Result, anyhow};
use std::future::Future;
use std::path::Path;
use std::sync::{Arc, mpsc};

type Job = Box<dyn FnOnce(&mut rusqlite::Connection) + Send + 'static>;

enum WorkerMessage {
    Call(Job),
    Close,
}

struct Worker {
    sender: mpsc::Sender<WorkerMessage>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.sender.send(WorkerMessage::Close);
    }
}

/// Cloneable asynchronous handle backed by one dedicated SQLite thread.
///
/// Calls wait in the connection's channel instead of occupying Tokio blocking
/// pool threads while another query owns the connection. Closures may return
/// either `rusqlite::Error` or a typed domain error; both remain available in
/// the returned `anyhow::Error` chain.
#[derive(Clone)]
pub struct Connection {
    worker: Arc<Worker>,
}

impl Connection {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_owned();
        Self::open_with(move || rusqlite::Connection::open(path)).await
    }

    pub(crate) async fn open_in_memory() -> Result<Self> {
        Self::open_with(rusqlite::Connection::open_in_memory).await
    }

    async fn open_with(
        opener: impl FnOnce() -> rusqlite::Result<rusqlite::Connection> + Send + 'static,
    ) -> Result<Self> {
        let (sender, receiver) = mpsc::channel::<WorkerMessage>();
        let (opened_tx, opened_rx) = tokio::sync::oneshot::channel();
        std::thread::Builder::new()
            .name("tangleaf-sqlite".into())
            .spawn(move || match opener() {
                Ok(mut connection) => {
                    install_statement_profile(&mut connection);
                    if opened_tx.send(Ok(())).is_err() {
                        return;
                    }
                    while let Ok(message) = receiver.recv() {
                        match message {
                            WorkerMessage::Call(job) => job(&mut connection),
                            WorkerMessage::Close => return,
                        }
                    }
                }
                Err(error) => {
                    let _ = opened_tx.send(Err(error));
                }
            })
            .context("spawning SQLite worker thread")?;
        opened_rx
            .await
            .context("SQLite worker stopped while opening")??;
        Ok(Self {
            worker: Arc::new(Worker { sender }),
        })
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
        self.worker
            .sender
            .send(WorkerMessage::Call(Box::new(move |connection| {
                let started = std::time::Instant::now();
                // One bad job must not unwind the worker thread: the connection
                // is shared by the whole process, and losing it would fail every
                // later call for the rest of the run. A panic mid-transaction
                // still rolls back as the transaction is dropped.
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    function(connection).map_err(Into::into)
                }))
                .unwrap_or_else(|payload| {
                    Err(anyhow!("SQLite job panicked: {}", panic_message(&*payload)))
                });
                // There is one connection for the whole process, so a slow job is not slow for
                // its own caller — it is slow for everything, and from the outside that looks
                // like the app hanging rather than like a query taking its time. Name the caller
                // so the log points at the command rather than at this file.
                let elapsed = started.elapsed();
                if elapsed >= SLOW_CALL {
                    tracing::warn!(
                        caller = %caller,
                        elapsed_ms = elapsed.as_millis() as u64,
                        "slow database call blocked the connection"
                    );
                }
                let _ = result_tx.send(outcome);
            })))
            .map_err(|_| anyhow!("SQLite connection worker has stopped"))?;
        result_rx
            .await
            .context("SQLite connection worker dropped a call")?
    }
}

/// A call that holds the single connection for this long is reported. Chosen to be quiet in
/// normal use — the slowest ordinary operations here are tens of milliseconds — while still
/// catching anything a person would perceive as a stall.
const SLOW_CALL: std::time::Duration = std::time::Duration::from_millis(500);

/// Individual statements are far smaller than whole calls; this catches the one statement that
/// explains a slow call, without logging a line per row in a loop of fast ones.
const SLOW_STATEMENT: std::time::Duration = std::time::Duration::from_millis(200);

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
            if elapsed < SLOW_STATEMENT {
                return;
            }
            let sql = statement.sql();
            tracing::warn!(
                elapsed_ms = elapsed.as_millis() as u64,
                sql = %sql.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(200).collect::<String>(),
                "slow SQL statement"
            );
        }),
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
    use std::fmt::Debug;

    #[derive(Debug, thiserror::Error)]
    #[error("typed domain failure")]
    struct DomainFailure;

    /// Captures the fields of every `warn` event, so a test can assert what the log would say.
    #[derive(Clone, Default)]
    struct CapturedWarnings(Arc<std::sync::Mutex<Vec<String>>>);

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CapturedWarnings {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _: tracing_subscriber::layer::Context<'_, S>,
        ) {
            if *event.metadata().level() != tracing::Level::WARN {
                return;
            }
            struct Collect(String);
            impl tracing::field::Visit for Collect {
                fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn Debug) {
                    self.0.push_str(&format!("{}={value:?} ", field.name()));
                }
            }
            let mut collected = Collect(String::new());
            event.record(&mut collected);
            self.0.lock().expect("warnings").push(collected.0);
        }
    }

    #[tokio::test]
    async fn a_slow_call_names_the_caller_and_the_statement() {
        use tracing_subscriber::layer::SubscriberExt;

        // The warnings are emitted on the worker thread, so a thread-local subscriber would see
        // nothing; this is the only test in the crate that installs a global one.
        let warnings = CapturedWarnings::default();
        let subscriber = tracing_subscriber::registry().with(warnings.clone());
        tracing::subscriber::set_global_default(subscriber).expect("install subscriber");

        let connection = Connection::open_in_memory().await.expect("open");
        connection
            .call(|database| {
                // Long enough to trip both thresholds, and expressed in SQL so the statement
                // profile has something to report.
                database.execute_batch(
                    "CREATE TABLE slow AS
                       WITH RECURSIVE counter(n) AS (
                         SELECT 1 UNION ALL SELECT n + 1 FROM counter WHERE n < 1500000
                       )
                       SELECT n FROM counter",
                )
            })
            .await
            .expect("slow call");

        let warnings = warnings.0.lock().expect("warnings").clone();
        assert!(
            warnings
                .iter()
                .any(|line| line.contains("slow database call") && line.contains("sqlite.rs")),
            "expected a slow-call warning naming this file, got {warnings:?}"
        );
        assert!(
            warnings
                .iter()
                .any(|line| line.contains("slow SQL statement") && line.contains("RECURSIVE")),
            "expected a slow-statement warning carrying the SQL, got {warnings:?}"
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
}
