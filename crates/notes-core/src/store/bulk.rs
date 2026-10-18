//! Closed set of prepared bulk mutations. Ordinary commands retain their one
//! ordered Apply batch; only these plans own replacement-specific SQL stages.

use super::{CommitNotice, Committed, NotesStore, begin_write, publish_commit};
use crate::CoreError;
use crate::changes::ChangeSet;
use crate::external_import::{ExternalImportOutcome, PreparedExternalImport};
use crate::operation::{self, PreparedSyncSnapshot};

pub(crate) struct BulkMutation<T> {
    pub(crate) value: T,
    pub(crate) wrote: bool,
}

mod sealed {
    pub trait Sealed {}
}

// Implementations live here, not beside an arbitrary SQL callback. Adding a
// bulk input requires an explicit prepared plan and review of its contract.
pub(crate) trait BulkCommand: sealed::Sealed + Send + 'static {
    type Value: Send + 'static;
    fn execute(self, sql: &rusqlite::Transaction<'_>) -> anyhow::Result<BulkMutation<Self::Value>>;
}

impl sealed::Sealed for PreparedExternalImport {}
impl BulkCommand for PreparedExternalImport {
    type Value = ExternalImportOutcome;
    fn execute(self, sql: &rusqlite::Transaction<'_>) -> anyhow::Result<BulkMutation<Self::Value>> {
        Ok(self.apply_in_transaction(sql)?)
    }
}

impl sealed::Sealed for PreparedSyncSnapshot {}
impl BulkCommand for PreparedSyncSnapshot {
    type Value = ();
    fn execute(self, sql: &rusqlite::Transaction<'_>) -> anyhow::Result<BulkMutation<Self::Value>> {
        Ok(self.apply_in_transaction(sql)?)
    }
}

impl<F> sealed::Sealed for crate::db::PreparedArchiveRestore<F> where
    F: FnOnce() -> anyhow::Result<()> + Send + 'static
{
}

impl<F> BulkCommand for crate::db::PreparedArchiveRestore<F>
where
    F: FnOnce() -> anyhow::Result<()> + Send + 'static,
{
    type Value = crate::db::ArchiveImportStats;
    fn execute(self, sql: &rusqlite::Transaction<'_>) -> anyhow::Result<BulkMutation<Self::Value>> {
        self.apply_in_transaction(sql)
    }
}

impl NotesStore {
    pub(crate) async fn execute_bulk<P: BulkCommand>(
        &self,
        plan: P,
    ) -> anyhow::Result<Committed<P::Value>> {
        let observer = self.observer.clone();
        self.connection
            .call_domain(move |database| -> anyhow::Result<_> {
                let started = std::time::Instant::now();
                let sql = begin_write(database)?;
                let write_lock = started.elapsed();
                operation::require_initialized_structure_projection(&sql)?;
                let before = sql.total_changes();
                let BulkMutation { value, wrote } = plan.execute(&sql)?;
                if !wrote && sql.total_changes() != before {
                    return Err(CoreError::invalid(
                        "a quiet bulk command cannot perform SQL writes",
                    )
                    .into());
                }
                let (commit_version, workspace_epoch) = if wrote {
                    let (version, epoch) = sql.query_row(
                    "UPDATE store_state SET version=version+1, workspace_epoch=workspace_epoch+1
                     WHERE singleton=1 AND version<9223372036854775807
                       AND workspace_epoch<9223372036854775807
                     RETURNING version, workspace_epoch",
                    [], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                )?;
                    (Some(version), epoch)
                } else {
                    (
                        None,
                        sql.query_row(
                            "SELECT workspace_epoch FROM store_state WHERE singleton=1",
                            [],
                            |row| row.get(0),
                        )?,
                    )
                };
                let committing = std::time::Instant::now();
                sql.commit()?;
                let commit = committing.elapsed();
                let changes = ChangeSet::default();
                if let Some(commit_version) = commit_version {
                    publish_commit(
                        observer,
                        CommitNotice {
                            commit_version,
                            workspace_epoch,
                            workspace_replaced: true,
                            changes: changes.clone(),
                        },
                    );
                }
                tracing::debug!(
                    commit_version,
                    workspace_epoch,
                    workspace_replaced = wrote,
                    commit_ms = commit.as_secs_f64() * 1000.0,
                    write_lock_ms = write_lock.as_secs_f64() * 1000.0,
                    transaction_ms = started.elapsed().as_secs_f64() * 1000.0,
                    "bulk store transaction completed"
                );
                Ok(Committed {
                    value,
                    changes,
                    commit_version,
                    workspace_epoch,
                    // Legacy per-kind metadata applies only to ordinary commands;
                    // a bulk result publishes one explicit replacement notice.
                    operations: Vec::new(),
                    #[cfg(test)]
                    projection_stats: operation::DeferredApplyStats::default(),
                })
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Connection, db};
    use std::sync::{Arc, Mutex, mpsc};

    // Deliberately malformed/noisy plans exercise the executor's invariants;
    // production callers can only construct the prepared types above.
    struct TestPlan {
        wrote: bool,
        entered: Option<tokio::sync::oneshot::Sender<()>>,
        release: Option<mpsc::Receiver<()>>,
    }
    impl sealed::Sealed for TestPlan {}
    impl BulkCommand for TestPlan {
        type Value = ();
        fn execute(self, sql: &rusqlite::Transaction<'_>) -> anyhow::Result<BulkMutation<()>> {
            sql.execute(
                "INSERT INTO sync_meta(key,value) VALUES('bulk-test','committed')",
                [],
            )?;
            if let Some(entered) = self.entered {
                let _ = entered.send(());
            }
            if let Some(release) = self.release {
                release.recv_timeout(std::time::Duration::from_secs(10))?;
            }
            Ok(BulkMutation {
                value: (),
                wrote: self.wrote,
            })
        }
    }
    fn plan(wrote: bool) -> TestPlan {
        TestPlan {
            wrote,
            entered: None,
            release: None,
        }
    }
    async fn fixture() -> (
        tempfile::TempDir,
        Connection,
        NotesStore,
        Arc<Mutex<Vec<CommitNotice>>>,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let connection = db::open(directory.path().join("notes.db")).await.unwrap();
        let notices = Arc::new(Mutex::new(Vec::new()));
        let observed = notices.clone();
        let store = NotesStore::new(connection.clone())
            .with_commit_observer(move |notice| observed.lock().unwrap().push(notice.clone()));
        (directory, connection, store, notices)
    }
    async fn facts(connection: &Connection) -> (i64, i64, i64) {
        connection.call(|sql| sql.query_row("SELECT version,workspace_epoch,(SELECT count(*) FROM sync_meta WHERE key='bulk-test') FROM store_state WHERE singleton=1", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)))).await.unwrap()
    }

    #[tokio::test]
    async fn cancelled_bulk_waiter_keeps_the_accepted_write_and_ordered_publication() {
        let (_directory, connection, store, notices) = fixture().await;
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, paused) = mpsc::channel();
        let worker_store = store.clone();
        let pending = tokio::spawn(async move {
            worker_store
                .execute_bulk(TestPlan {
                    wrote: true,
                    entered: Some(entered),
                    release: Some(paused),
                })
                .await
        });
        started.await.unwrap();
        pending.abort();
        assert!(pending.await.unwrap_err().is_cancelled());
        release.send(()).unwrap();
        assert_eq!(facts(&connection).await, (1, 1, 1));
        let notices = notices.lock().unwrap();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].commit_version, 1);
        assert_eq!(notices[0].workspace_epoch, 1);
        assert!(notices[0].workspace_replaced && notices[0].changes.is_empty());
    }

    #[tokio::test]
    async fn a_bulk_plan_cannot_hide_sql_writes_as_a_quiet_result() {
        let (_directory, connection, store, notices) = fixture().await;
        let error = store.execute_bulk(plan(false)).await.unwrap_err();
        assert!(error.to_string().contains("quiet bulk command"));
        assert_eq!(facts(&connection).await, (0, 0, 0));
        assert!(notices.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn replacement_generation_belongs_to_the_read_snapshot() {
        let (directory, connection, store, _notices) = fixture().await;
        let reader = Connection::open_read_only(directory.path().join("notes.db"))
            .await
            .unwrap();
        let epoch = reader
            .call(|sql| {
                sql.execute_batch("BEGIN DEFERRED")?;
                sql.query_row("SELECT workspace_epoch FROM store_state", [], |row| {
                    row.get::<_, i64>(0)
                })
            })
            .await
            .unwrap();
        assert_eq!(epoch, 0);
        let committed = store.execute_bulk(plan(true)).await.unwrap();
        assert_eq!(committed.workspace_epoch, 1);
        assert_eq!(facts(&connection).await, (1, 1, 1));
        assert_eq!(
            reader
                .call(
                    |sql| sql
                        .query_row("SELECT workspace_epoch FROM store_state", [], |row| row
                            .get::<_, i64>(0))
                )
                .await
                .unwrap(),
            0
        );
        reader
            .call(|sql| sql.execute_batch("COMMIT"))
            .await
            .unwrap();
        assert_eq!(
            reader
                .call(
                    |sql| sql
                        .query_row("SELECT workspace_epoch FROM store_state", [], |row| row
                            .get::<_, i64>(0))
                )
                .await
                .unwrap(),
            1
        );
        reader.close().await.unwrap();
    }

    #[tokio::test]
    async fn exhausted_counters_roll_back_bulk_content_and_publish_nothing() {
        for column in ["version", "workspace_epoch"] {
            let (_directory, connection, store, notices) = fixture().await;
            connection
                .call(move |sql| {
                    sql.execute(
                        &format!("UPDATE store_state SET {column}=9223372036854775807"),
                        [],
                    )?;
                    Ok(())
                })
                .await
                .unwrap();
            let before = facts(&connection).await;
            assert!(store.execute_bulk(plan(true)).await.is_err());
            assert_eq!(facts(&connection).await, before);
            assert!(notices.lock().unwrap().is_empty());
        }
    }
}
