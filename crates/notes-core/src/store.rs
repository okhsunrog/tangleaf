//! Transaction boundary for migrated text commands. Legacy database functions
//! remain adapters while commands move here one at a time.

use crate::changes::ChangeSet;
use crate::db::{self, HistoryMoveResult};
use crate::operation::{
    self, ApplyOutcome, DeferredApplyStats, Op, OpKind, Origin, ProjectionPolicy,
};
use crate::{Connection, CoreError, CoreResult};
use std::sync::Arc;

mod metadata;

type CommitObserver = dyn Fn(&CommitNotice) + Send + Sync;

/// Facts captured inside a successful transaction, published after its commit.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CommitNotice {
    /// Local order among NotesStore commits only. Legacy and ink writers have
    /// not migrated yet, so this must not be used as a database-wide revision.
    pub commit_version: i64,
    pub changes: ChangeSet,
}

#[derive(Debug)]
pub struct Committed<T> {
    pub value: T,
    pub changes: ChangeSet,
    /// None for a command that made no write, such as Undo on an empty stack.
    pub commit_version: Option<i64>,
    pub(crate) operations: Vec<OpKind>,
    #[cfg(test)]
    pub(crate) projection_stats: DeferredApplyStats,
}

impl<T> Committed<T> {
    /// Adapt a command's public result without losing its committed facts.
    pub fn map_value<U>(self, map: impl FnOnce(T) -> U) -> Committed<U> {
        Committed {
            value: map(self.value),
            changes: self.changes,
            commit_version: self.commit_version,
            operations: self.operations,
            #[cfg(test)]
            projection_stats: self.projection_stats,
        }
    }
}

/// Shares the existing SQLite worker; it does not open another connection.
#[derive(Clone)]
pub struct NotesStore {
    connection: Connection,
    observer: Option<Arc<CommitObserver>>,
    structure_mode: operation::StructureProjectionMode,
}

impl NotesStore {
    /// Wrap a writer initialized by [`db::open`] or [`db::open_in_memory`].
    /// Initialization establishes the canonical structural projection before
    /// commands start maintaining only affected components.
    pub fn new(connection: Connection) -> Self {
        Self {
            connection,
            observer: None,
            structure_mode: operation::StructureProjectionMode::Components,
        }
    }

    /// Called synchronously in commit order on the SQLite worker. Observers
    /// must enqueue/emit notifications without blocking or querying the store.
    /// An observer failure cannot undo an already committed write.
    pub fn with_commit_observer(
        mut self,
        observer: impl Fn(&CommitNotice) + Send + Sync + 'static,
    ) -> Self {
        self.observer = Some(Arc::new(observer));
        self
    }

    pub async fn undo(&self) -> anyhow::Result<Committed<HistoryMoveResult>> {
        self.write(|transaction| db::history::move_history_in_transaction(transaction, true))
            .await
    }

    pub async fn redo(&self) -> anyhow::Result<Committed<HistoryMoveResult>> {
        self.write(|transaction| db::history::move_history_in_transaction(transaction, false))
            .await
    }

    /// Apply original envelopes without authoring replacement IDs or clocks.
    pub async fn apply_batch(
        &self,
        operations: &[Op],
        origin: Origin,
    ) -> anyhow::Result<Committed<Vec<ApplyOutcome>>> {
        let operations = operations.to_vec();
        self.write(move |write| write.apply_replicated_batch(&operations, origin))
            .await
    }

    pub async fn apply_sequenced_batch(
        &self,
        operations: Vec<(u64, Op)>,
    ) -> anyhow::Result<Committed<Vec<ApplyOutcome>>> {
        self.write(move |write| write.apply_sequenced_batch(&operations))
            .await
    }

    pub async fn acknowledge_server_ops(
        &self,
        acknowledgements: Vec<(uuid::Uuid, u64)>,
    ) -> anyhow::Result<Committed<()>> {
        self.write(move |write| write.acknowledge_server_ops(&acknowledgements))
            .await
    }

    pub(crate) async fn write<T, E>(
        &self,
        command: impl FnOnce(&mut NotesTransaction<'_, '_>) -> Result<T, E> + Send + 'static,
    ) -> anyhow::Result<Committed<T>>
    where
        T: Send + 'static,
        E: Into<anyhow::Error> + Send + 'static,
    {
        let observer = self.observer.clone();
        let structure_mode = self.structure_mode;
        self.connection
            .call_domain(move |database| -> anyhow::Result<Committed<T>> {
                let started = std::time::Instant::now();
                // Preserve the existing deferred lock policy during migration.
                // Changing writer acquisition is a separate concurrency change.
                let sql = database.transaction()?;
                if structure_mode == operation::StructureProjectionMode::Components {
                    operation::require_initialized_structure_projection(&sql)?;
                }
                let mut transaction = NotesTransaction {
                    sql: &sql,
                    changes: ChangeSet::default(),
                    operations: Vec::new(),
                    projection_stats: DeferredApplyStats::default(),
                    applied: false,
                    failed: false,
                    durable_changed: false,
                    structure_mode,
                };
                let value = command(&mut transaction).map_err(Into::into)?;
                if transaction.failed {
                    return Err(CoreError::invalid("a failed store command cannot commit").into());
                }
                let NotesTransaction {
                    changes,
                    operations,
                    projection_stats,
                    durable_changed,
                    ..
                } = transaction;
                let commit_version = if durable_changed || !changes.is_empty() {
                    Some(sql.query_row(
                        "UPDATE store_state SET version = version + 1
                         WHERE singleton = 1 RETURNING version",
                        [],
                        |row| row.get(0),
                    )?)
                } else {
                    None
                };
                let committing = std::time::Instant::now();
                sql.commit()?;
                let commit = committing.elapsed();
                // Publish here, before the worker accepts its next command,
                // rather than after awaiting a response on a Tokio thread.
                if let (Some(observer), Some(commit_version)) = (observer, commit_version) {
                    let notice = CommitNotice {
                        commit_version,
                        changes: changes.clone(),
                    };
                    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| observer(&notice)))
                        .is_err()
                    {
                        tracing::error!(commit_version, "commit observer panicked after commit");
                    }
                }
                tracing::debug!(
                    operations = operations.len(),
                    durable_changed,
                    commit_version,
                    structure_reconciliations = projection_stats.structure_reconciliations,
                    full_structure_reconciliations =
                        projection_stats.full_structure_reconciliations,
                    structure_blocks_read = projection_stats.structure_blocks_read,
                    structure_intents_read = projection_stats.structure_intents_read,
                    structure_pages_read = projection_stats.structure_pages_read,
                    structure_vertices_visited = projection_stats.structure_vertices_visited,
                    structure_seed_rows_read = projection_stats.structure_seed_rows_read,
                    commit_ms = commit.as_secs_f64() * 1000.0,
                    transaction_ms = started.elapsed().as_secs_f64() * 1000.0,
                    "store transaction completed"
                );
                Ok(Committed {
                    value,
                    changes,
                    commit_version,
                    operations,
                    #[cfg(test)]
                    projection_stats,
                })
            })
            .await
    }
}

/// One ordered apply batch per command keeps before/after facts exact even
/// across the structural barriers inside Apply. It also prevents a caller from
/// concatenating separately captured inverses as if they shared one baseline.
pub(crate) struct NotesTransaction<'tx, 'connection> {
    sql: &'tx rusqlite::Transaction<'connection>,
    changes: ChangeSet,
    operations: Vec<OpKind>,
    projection_stats: DeferredApplyStats,
    applied: bool,
    failed: bool,
    // Replication can advance applied_ops/cursor/outbox while every public
    // content projection remains unchanged. Such writes still own a commit.
    durable_changed: bool,
    structure_mode: operation::StructureProjectionMode,
}

enum ReplicationBatch<'a> {
    Unsequenced(&'a [Op], Origin),
    Sequenced(&'a [(u64, Op)]),
}

impl<'connection> NotesTransaction<'_, 'connection> {
    pub(crate) fn sql(&self) -> &rusqlite::Transaction<'connection> {
        self.sql
    }

    pub(crate) fn mark_history_changed(&mut self) {
        self.changes.history_changed = true;
    }

    pub(crate) fn apply_local(
        &mut self,
        kinds: Vec<OpKind>,
        policy: ProjectionPolicy,
    ) -> CoreResult<DeferredApplyStats> {
        self.apply_kinds(kinds, policy, false)
    }

    /// The history module alone constructs a validated restoration plan. Normal
    /// commands cannot use this boundary to publish arbitrary ink versions.
    pub(crate) fn apply_history_restore(
        &mut self,
        plan: &db::history::HistoryRestorePlan,
        policy: ProjectionPolicy,
    ) -> CoreResult<DeferredApplyStats> {
        let result = (|| {
            let stats = self.apply_kinds(plan.operations().to_vec(), policy, true)?;
            plan.restore_native(self.sql)?;
            Ok(stats)
        })();
        // Native metadata belongs to the same restoration as its replicated
        // operations. Catching a late failure cannot authorize a partial commit.
        self.failed |= result.is_err();
        result
    }

    fn apply_kinds(
        &mut self,
        kinds: Vec<OpKind>,
        policy: ProjectionPolicy,
        history_restore: bool,
    ) -> CoreResult<DeferredApplyStats> {
        let result = (|| {
            if self.failed || self.applied {
                return Err(CoreError::invalid(
                    "a store command must apply one ordered batch",
                ));
            }
            if !history_restore
                && kinds
                    .iter()
                    .any(|kind| matches!(kind, OpKind::InkPublish(_)))
            {
                return Err(CoreError::invalid(
                    "ink writes have not migrated to the text store",
                ));
            }
            let applied =
                operation::apply_local_kinds_tracked(self.sql, kinds, policy, self.structure_mode)?;
            let history_changed = self.changes.history_changed;
            self.changes = applied.changes;
            self.changes.history_changed |= history_changed;
            self.operations = applied.operations.into_iter().map(|op| op.kind).collect();
            self.durable_changed |= !self.operations.is_empty();
            self.projection_stats = applied.stats;
            self.applied = true;
            Ok(applied.stats)
        })();
        self.failed |= result.is_err();
        result
    }

    pub(crate) fn apply_replicated_batch(
        &mut self,
        operations: &[Op],
        origin: Origin,
    ) -> CoreResult<Vec<ApplyOutcome>> {
        self.apply_replication(ReplicationBatch::Unsequenced(operations, origin))
    }

    pub(crate) fn apply_sequenced_batch(
        &mut self,
        operations: &[(u64, Op)],
    ) -> CoreResult<Vec<ApplyOutcome>> {
        self.apply_replication(ReplicationBatch::Sequenced(operations))
    }

    fn apply_replication(&mut self, batch: ReplicationBatch<'_>) -> CoreResult<Vec<ApplyOutcome>> {
        let result = (|| {
            if self.failed || self.applied {
                return Err(CoreError::invalid(
                    "a store command must apply one ordered batch",
                ));
            }
            let applied = match batch {
                ReplicationBatch::Unsequenced(operations, origin) => {
                    operation::apply_batch_in_transaction(
                        self.sql,
                        operations,
                        origin,
                        self.structure_mode,
                        true,
                    )?
                }
                ReplicationBatch::Sequenced(operations) => {
                    operation::apply_sequenced_batch_in_transaction(
                        self.sql,
                        operations,
                        self.structure_mode,
                        true,
                    )?
                }
            };
            let history_changed = self.changes.history_changed;
            self.changes = applied.changes;
            self.changes.history_changed |= history_changed;
            self.operations = applied.operations;
            self.projection_stats = applied.stats;
            self.durable_changed |= applied.durable_changed;
            self.applied = true;
            Ok(applied.outcomes)
        })();
        self.failed |= result.is_err();
        result
    }

    pub(crate) fn acknowledge_server_ops(
        &mut self,
        acknowledgements: &[(uuid::Uuid, u64)],
    ) -> CoreResult<()> {
        let result = (|| {
            if self.failed || self.applied {
                return Err(CoreError::invalid(
                    "a store command must apply one ordered batch",
                ));
            }
            self.durable_changed |=
                operation::acknowledge_server_ops_in_transaction(self.sql, acknowledgements)?;
            self.applied = true;
            Ok(())
        })();
        self.failed |= result.is_err();
        result
    }

    pub(crate) fn apply_action(
        &mut self,
        action: &str,
        kinds: Vec<OpKind>,
        policy: ProjectionPolicy,
    ) -> CoreResult<DeferredApplyStats> {
        let result = (|| {
            let prepared = db::history::prepare_action(self.sql, &kinds)?;
            let has_history = prepared.has_history();
            let stats = self.apply_local(kinds.clone(), policy)?;
            db::history::record_prepared_action(self.sql, action, kinds, prepared, policy)?;
            if has_history {
                self.mark_history_changed();
            }
            Ok(stats)
        })();
        self.failed |= result.is_err();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Block, BlockContent};
    use crate::{BlockSetMarkdown, BlockStyle};
    use std::sync::Mutex;

    async fn fixture() -> (tempfile::TempDir, Connection, Block) {
        let directory = tempfile::tempdir().unwrap();
        let connection = db::open(directory.path().join("notes.db")).await.unwrap();
        operation::configure_sync(&connection, "https://sync.example")
            .await
            .unwrap();
        let page = db::create_page(&connection, "Store".into()).await.unwrap();
        let block = db::create_block(
            &connection,
            page.uuid,
            None,
            None,
            BlockStyle::Bullet,
            "Original".into(),
        )
        .await
        .unwrap();
        connection
            .call(|db| {
                db.execute("DELETE FROM history_undo", [])?;
                db.execute("DELETE FROM history_redo", [])?;
                Ok(())
            })
            .await
            .unwrap();
        (directory, connection, block)
    }

    fn parts() -> Vec<BlockContent> {
        ["First [[Target]]", "Second", "Third"]
            .into_iter()
            .map(|markdown| BlockContent {
                markdown: markdown.into(),
            })
            .collect()
    }

    async fn clock(connection: &Connection) -> i64 {
        connection
            .call(|db| db.query_row("SELECT version FROM store_state", [], |row| row.get(0)))
            .await
            .unwrap()
    }

    async fn state(connection: &Connection) -> serde_json::Value {
        let snapshot = operation::export_sync_snapshot(connection, 0)
            .await
            .unwrap();
        let outbox = operation::pending_outbox(connection, 10_000).await.unwrap();
        let history = connection.call(|db| {
            let mut rows = db.prepare(
                "SELECT 'undo', id, action_uuid, action, forward_json, inverse_json FROM history_undo
                 UNION ALL SELECT 'redo', id, action_uuid, action, forward_json, inverse_json FROM history_redo
                 ORDER BY 1, 2",
            )?;
            rows.query_map([], |row| Ok((
                row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, uuid::Uuid>(2)?,
                row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?,
            )))?.collect::<rusqlite::Result<Vec<_>>>()
        }).await.unwrap();
        let hlc = connection
            .call(|db| {
                db.query_row(
                    "SELECT value FROM sync_meta WHERE key = 'last_hlc'",
                    [],
                    |row| row.get::<_, String>(0),
                )
            })
            .await
            .unwrap();
        serde_json::json!([snapshot, outbox, history, hlc, clock(connection).await])
    }

    #[tokio::test]
    async fn commits_publish_final_facts_in_durable_order_and_empty_undo_does_not_publish() {
        let (directory, connection, block) = fixture().await;
        let baseline = clock(&connection).await;
        let path = directory.path().join("notes.db");
        let observed = Arc::new(Mutex::new(Vec::new()));
        let notices = observed.clone();
        let store = NotesStore::new(connection.clone()).with_commit_observer(move |notice| {
            // A separate connection sees committed facts, never uncommitted state.
            let reader = rusqlite::Connection::open(&path).unwrap();
            let version: i64 = reader
                .query_row("SELECT version FROM store_state", [], |row| row.get(0))
                .unwrap();
            assert_eq!(version, notice.commit_version);
            for change in &notice.changes.blocks {
                let exists: bool = reader
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM blocks WHERE uuid = ?1)",
                        [change.uuid],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(exists, change.after.is_some());
            }
            notices.lock().unwrap().push(notice.clone());
        });
        let empty = store.undo().await.unwrap();
        assert_eq!(empty.value, HistoryMoveResult::Empty);
        assert_eq!(empty.commit_version, None);
        assert!(empty.changes.is_empty());
        assert!(observed.lock().unwrap().is_empty());

        let split = store
            .split_block(block.uuid, parts(), block.markdown_revision.clone())
            .await
            .unwrap();
        assert_eq!(split.commit_version, Some(baseline + 1));
        assert_eq!(split.projection_stats.structure_reconciliations, 1);
        assert_eq!(split.changes.affected_pages, vec![block.page_uuid]);
        assert!(split.changes.changed_pages.is_empty());
        assert_eq!(split.changes.blocks.len(), 3);
        assert!(split.changes.history_changed);
        let source = split
            .changes
            .blocks
            .iter()
            .find(|c| c.uuid == block.uuid)
            .unwrap();
        assert_eq!(source.before, source.after);
        assert!(source.before.is_some());
        let undo = store.undo().await.unwrap();
        assert_eq!(undo.value, HistoryMoveResult::Applied);
        assert_eq!(undo.commit_version, Some(baseline + 2));
        assert_eq!(undo.projection_stats.structure_reconciliations, 1);
        assert_eq!(
            undo.changes
                .blocks
                .iter()
                .filter(|c| c.after.is_none())
                .count(),
            2
        );
        let redo = store.redo().await.unwrap();
        assert_eq!(redo.value, HistoryMoveResult::Applied);
        assert_eq!(redo.commit_version, Some(baseline + 3));
        assert_eq!(redo.projection_stats.structure_reconciliations, 1);
        assert_eq!(
            observed
                .lock()
                .unwrap()
                .iter()
                .map(|n| n.commit_version)
                .collect::<Vec<_>>(),
            vec![baseline + 1, baseline + 2, baseline + 3]
        );
        assert_eq!(
            db::get_block(&connection, block.uuid)
                .await
                .unwrap()
                .unwrap()
                .markdown,
            "First [[Target]]"
        );
        drop(store);
        drop(connection);
        let reopened = db::open(directory.path().join("notes.db")).await.unwrap();
        assert_eq!(clock(&reopened).await, baseline + 3);
        assert_eq!(
            NotesStore::new(reopened)
                .undo()
                .await
                .unwrap()
                .commit_version,
            Some(baseline + 4)
        );
    }

    #[tokio::test]
    async fn store_rejects_uninitialized_structure_projection_before_planning_and_publishing() {
        let (_directory, connection, block) = fixture().await;
        let other = db::create_note(&connection, Some("Unrelated source".into()))
            .await
            .unwrap()
            .into_created()
            .unwrap();
        let wrong_page = db::create_page(&connection, "Stale placement".into())
            .await
            .unwrap();
        let unrelated_uuid = other.initial_block.uuid;
        connection
            .call(move |database| {
                database.execute(
                    "UPDATE blocks SET page_uuid = ?2 WHERE uuid = ?1",
                    rusqlite::params![unrelated_uuid, wrong_page.uuid],
                )?;
                database.execute(
                    "UPDATE store_state SET structure_projection_version = 0",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let observed = Arc::new(Mutex::new(Vec::new()));
        let notices = observed.clone();
        let store = NotesStore::new(connection.clone()).with_commit_observer(move |notice| {
            notices.lock().unwrap().push(notice.clone());
        });
        let before = state(&connection).await;

        let error = store
            .split_block(block.uuid, parts(), block.markdown_revision.clone())
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("structural projection version 0")
        );
        assert_eq!(state(&connection).await, before);
        assert!(observed.lock().unwrap().is_empty());

        assert!(
            operation::initialize_structure_projection(&connection)
                .await
                .unwrap()
        );
        assert_eq!(
            db::get_block(&connection, unrelated_uuid)
                .await
                .unwrap()
                .unwrap()
                .page_uuid,
            other.page.uuid
        );
        assert!(
            observed.lock().unwrap().is_empty(),
            "startup repair does not publish a user command"
        );
        store
            .split_block(block.uuid, parts(), block.markdown_revision)
            .await
            .unwrap();
        assert_eq!(observed.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn store_rejects_future_structure_projection_before_mutation() {
        let (_directory, connection, block) = fixture().await;
        connection
            .call(|database| {
                database.execute(
                    "UPDATE store_state SET structure_projection_version = 2",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let observed = Arc::new(Mutex::new(Vec::new()));
        let notices = observed.clone();
        let store = NotesStore::new(connection.clone()).with_commit_observer(move |notice| {
            notices.lock().unwrap().push(notice.clone());
        });
        let before = state(&connection).await;

        let error = store
            .split_block(block.uuid, parts(), block.markdown_revision)
            .await
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("structural projection version 2")
        );
        assert_eq!(state(&connection).await, before);
        assert!(observed.lock().unwrap().is_empty());
        assert_eq!(
            connection
                .call(|database| database.query_row(
                    "SELECT structure_projection_version FROM store_state",
                    [],
                    |row| row.get::<_, i64>(0)
                ))
                .await
                .unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn error_after_apply_rolls_back_content_history_outbox_clock_and_notifications() {
        let (_directory, connection, block) = fixture().await;
        let observed = Arc::new(Mutex::new(Vec::new()));
        let notices = observed.clone();
        let store = NotesStore::new(connection.clone()).with_commit_observer(move |notice| {
            notices.lock().unwrap().push(notice.clone());
        });
        let before = state(&connection).await;
        let error = store
            .write(move |write| -> CoreResult<()> {
                write.apply_action(
                    "edit",
                    vec![OpKind::BlockSetMarkdown(BlockSetMarkdown {
                        uuid: block.uuid,
                        markdown: "Changed [[Target]]".into(),
                    })],
                    ProjectionPolicy::Immediate,
                )?;
                Err(CoreError::conflict("injected result validation failure"))
            })
            .await
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<CoreError>(),
            Some(CoreError::Conflict(_))
        ));
        assert_eq!(state(&connection).await, before);
        assert!(observed.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn caught_apply_errors_still_prevent_a_partial_commit() {
        let (_directory, connection, block) = fixture().await;
        let store = NotesStore::new(connection.clone());
        let before = state(&connection).await;
        let result = store
            .write(move |write| -> CoreResult<()> {
                // The first operation can write before the missing-page operation fails.
                let result = write.apply_local(
                    vec![
                        OpKind::BlockSetMarkdown(BlockSetMarkdown {
                            uuid: block.uuid,
                            markdown: "Partial".into(),
                        }),
                        OpKind::BlockCreate(crate::BlockCreate {
                            uuid: uuid::Uuid::now_v7(),
                            page_uuid: uuid::Uuid::now_v7(),
                            parent_uuid: None,
                            order_key: crate::OrderKey::from_ordinal(1),
                            style: BlockStyle::Bullet,
                            markdown: "Invalid destination".into(),
                            created_at: 1,
                        }),
                    ],
                    ProjectionPolicy::Immediate,
                );
                assert!(result.is_err());
                Ok(()) // An accidental swallowed error must not authorize commit.
            })
            .await;
        assert!(result.is_err());
        assert_eq!(state(&connection).await, before);
    }

    #[tokio::test]
    async fn clock_write_failure_rolls_back_the_entire_split() {
        let (_directory, connection, block) = fixture().await;
        let baseline = clock(&connection).await;
        let store = NotesStore::new(connection.clone());
        let before = state(&connection).await;
        connection
            .call(|db| {
                db.execute_batch(
                    "CREATE TEMP TRIGGER reject_clock BEFORE UPDATE ON store_state
             BEGIN SELECT RAISE(ABORT, 'injected clock failure'); END;",
                )
            })
            .await
            .unwrap();
        assert!(
            store
                .split_block(block.uuid, parts(), block.markdown_revision.clone())
                .await
                .is_err()
        );
        assert_eq!(state(&connection).await, before);
        connection
            .call(|db| db.execute_batch("DROP TRIGGER reject_clock"))
            .await
            .unwrap();
        assert_eq!(
            store
                .split_block(block.uuid, parts(), block.markdown_revision)
                .await
                .unwrap()
                .commit_version,
            Some(baseline + 1)
        );
    }

    #[tokio::test]
    async fn stale_split_and_conflicting_history_publish_no_false_content_changes() {
        let (_directory, connection, block) = fixture().await;
        let baseline = clock(&connection).await;
        let observed = Arc::new(Mutex::new(Vec::new()));
        let notices = observed.clone();
        let store = NotesStore::new(connection.clone()).with_commit_observer(move |notice| {
            notices.lock().unwrap().push(notice.clone());
        });
        store
            .split_block(block.uuid, parts(), block.markdown_revision.clone())
            .await
            .unwrap();
        let before = state(&connection).await;
        let error = store
            .split_block(block.uuid, parts(), block.markdown_revision)
            .await
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<CoreError>(),
            Some(CoreError::Conflict(_))
        ));
        assert_eq!(state(&connection).await, before);
        assert_eq!(observed.lock().unwrap().len(), 1);

        // A subsequent non-history edit invalidates the inverse's HLC guard.
        operation::apply_local_kinds(
            &connection,
            vec![OpKind::BlockSetMarkdown(BlockSetMarkdown {
                uuid: block.uuid,
                markdown: "External edit".into(),
            })],
        )
        .await
        .unwrap();
        let skipped = store.undo().await.unwrap();
        assert_eq!(skipped.value, HistoryMoveResult::Skipped);
        assert_eq!(skipped.commit_version, Some(baseline + 2));
        assert_eq!(
            skipped.changes,
            ChangeSet {
                history_changed: true,
                ..Default::default()
            }
        );
        assert_eq!(
            db::get_block(&connection, block.uuid)
                .await
                .unwrap()
                .unwrap()
                .markdown,
            "External edit"
        );
    }

    #[tokio::test]
    async fn aborting_the_awaiter_does_not_cancel_an_accepted_commit_or_its_notice() {
        let (_directory, connection, block) = fixture().await;
        let baseline = clock(&connection).await;
        let observed = Arc::new(Mutex::new(Vec::new()));
        let notices = observed.clone();
        let store = NotesStore::new(connection.clone()).with_commit_observer(move |notice| {
            notices.lock().unwrap().push(notice.clone());
        });
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let task = tokio::spawn(async move {
            store
                .write(move |write| -> CoreResult<()> {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    write.apply_action(
                        "accepted edit",
                        vec![OpKind::BlockSetMarkdown(BlockSetMarkdown {
                            uuid: block.uuid,
                            markdown: "Accepted".into(),
                        })],
                        ProjectionPolicy::Immediate,
                    )?;
                    Ok(())
                })
                .await
        });
        started_rx.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        release_tx.send(()).unwrap();
        // Queued behind the accepted job, this read also waits for publication.
        assert_eq!(clock(&connection).await, baseline + 1);
        assert_eq!(observed.lock().unwrap().len(), 1);
        assert_eq!(
            db::get_block(&connection, block.uuid)
                .await
                .unwrap()
                .unwrap()
                .markdown,
            "Accepted"
        );
    }

    #[tokio::test]
    async fn page_delete_counts_seed_reads_without_requesting_a_new_projection() {
        let (_directory, connection, block) = fixture().await;
        let store = NotesStore::new(connection.clone());
        let deleted = store
            .write(move |write| -> CoreResult<()> {
                write.apply_action(
                    "delete page",
                    vec![OpKind::PageDelete(crate::PageDelete {
                        uuid: block.page_uuid,
                    })],
                    ProjectionPolicy::Deferred,
                )?;
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(deleted.projection_stats.structure_reconciliations, 0);
        assert!(deleted.projection_stats.structure_seed_rows_read > 0);
        assert_eq!(deleted.changes.deleted_pages, vec![block.page_uuid]);
        assert!(
            db::get_page(&connection, block.page_uuid)
                .await
                .unwrap()
                .is_none()
        );
    }

    async fn assert_split_component_scope(unrelated_blocks: usize) {
        let (_directory, connection, block) = fixture().await;
        let unrelated = db::create_page(&connection, "Unrelated".into())
            .await
            .unwrap();
        let kinds = (0..unrelated_blocks)
            .map(|index| {
                OpKind::BlockCreate(crate::BlockCreate {
                    uuid: uuid::Uuid::now_v7(),
                    page_uuid: unrelated.uuid,
                    parent_uuid: None,
                    order_key: crate::OrderKey::from_ordinal(index + 1),
                    style: BlockStyle::Bullet,
                    markdown: format!("Unrelated {index}"),
                    created_at: 1,
                })
            })
            .collect();
        connection
            .call_domain(move |database| -> CoreResult<()> {
                let tx = database.transaction()?;
                operation::apply_local_kinds_deferred_in_transaction(
                    &tx,
                    kinds,
                    operation::StructureProjectionMode::Full,
                )?;
                tx.commit()?;
                Ok(())
            })
            .await
            .unwrap();
        let mut store = NotesStore::new(connection.clone());
        store.structure_mode = operation::StructureProjectionMode::Components;
        let parts = (0..243)
            .map(|index| BlockContent {
                markdown: format!("Part {index}"),
            })
            .collect();
        let split = store
            .split_block(block.uuid, parts, block.markdown_revision)
            .await
            .unwrap();
        assert_eq!(split.value.len(), 243);
        assert_eq!(split.projection_stats.structure_reconciliations, 1);
        assert_eq!(split.projection_stats.full_structure_reconciliations, 0);
        // Only the 242 new roots need resolution. Updating the existing first
        // block's Markdown does not introduce a structural dependency.
        assert_eq!(split.projection_stats.structure_blocks_read, 242);
        assert_eq!(split.projection_stats.structure_intents_read, 242);
        assert_eq!(split.projection_stats.structure_pages_read, 1);
        assert_eq!(split.projection_stats.structure_vertices_visited, 242);
        assert_eq!(split.projection_stats.structure_seed_rows_read, 0);
        assert_eq!(split.changes.blocks.len(), 243);
        let undo = store.undo().await.unwrap();
        assert_eq!(undo.value, HistoryMoveResult::Applied);
        assert_eq!(undo.projection_stats.structure_reconciliations, 1);
        assert_eq!(undo.projection_stats.full_structure_reconciliations, 0);
        assert_eq!(undo.projection_stats.structure_blocks_read, 0);
        assert_eq!(undo.projection_stats.structure_intents_read, 0);
        assert_eq!(undo.projection_stats.structure_vertices_visited, 242);
        assert_eq!(undo.projection_stats.structure_seed_rows_read, 484);
        let redo = store.redo().await.unwrap();
        assert_eq!(redo.value, HistoryMoveResult::Applied);
        assert_eq!(redo.projection_stats.structure_reconciliations, 1);
        assert_eq!(redo.projection_stats.full_structure_reconciliations, 0);
        assert_eq!(redo.projection_stats.structure_blocks_read, 242);
        assert_eq!(redo.projection_stats.structure_intents_read, 242);
        assert_eq!(redo.projection_stats.structure_pages_read, 1);
        let unchanged = db::list_block_children(&connection, unrelated.uuid, None)
            .await
            .unwrap();
        assert_eq!(unchanged.len(), unrelated_blocks);
        assert!(
            unchanged
                .iter()
                .enumerate()
                .all(|(index, b)| b.markdown == format!("Unrelated {index}"))
        );
    }

    #[tokio::test]
    async fn split_component_reads_are_independent_of_unrelated_database_size() {
        for count in [1_000, 10_000] {
            assert_split_component_scope(count).await;
        }
    }

    #[tokio::test]
    #[ignore = "explicit scale validation with 100000 unrelated blocks"]
    async fn split_component_reads_with_one_hundred_thousand_unrelated_blocks() {
        assert_split_component_scope(100_000).await;
    }

    #[tokio::test]
    async fn observer_failure_cannot_turn_a_successful_commit_into_a_write_error() {
        let (_directory, connection, block) = fixture().await;
        let baseline = clock(&connection).await;
        let store = NotesStore::new(connection.clone())
            .with_commit_observer(|_| panic!("observer failure"));
        let split = store
            .split_block(block.uuid, parts(), block.markdown_revision)
            .await
            .unwrap();
        assert_eq!(split.commit_version, Some(baseline + 1));
        assert_eq!(clock(&connection).await, baseline + 1);
        assert_eq!(db::history_status(&connection).await.unwrap().undo_count, 1);
    }
}
