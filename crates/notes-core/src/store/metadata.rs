//! Named synchronization configuration and queue-state commands.

use super::{Committed, NotesStore, NotesTransaction};
use crate::operation::{self, ReplicaRole};
use crate::{CoreError, CoreResult};

impl NotesStore {
    pub async fn set_replica_role(&self, role: ReplicaRole) -> anyhow::Result<Committed<()>> {
        self.write(move |write| {
            write.apply_sync_metadata(|sql| {
                Ok(((), operation::set_replica_role_in_transaction(sql, role)?))
            })
        })
        .await
    }

    pub async fn configure_sync(&self, server_url: &str) -> anyhow::Result<Committed<()>> {
        let server_url = server_url.trim().to_owned();
        self.write(move |write| {
            write.apply_sync_metadata(|sql| {
                Ok((
                    (),
                    operation::configure_sync_in_transaction(sql, &server_url)?,
                ))
            })
        })
        .await
    }

    pub async fn bind_sync_workspace(
        &self,
        server_url: &str,
        workspace_uuid: uuid::Uuid,
        server_seq: u64,
    ) -> anyhow::Result<Committed<()>> {
        let server_url = server_url.trim().to_owned();
        self.write(move |write| {
            write.apply_sync_metadata(|sql| {
                Ok((
                    (),
                    operation::bind_sync_workspace_in_transaction(
                        sql,
                        &server_url,
                        workspace_uuid,
                        server_seq,
                    )?,
                ))
            })
        })
        .await
    }

    pub async fn quarantine_outbox_op(
        &self,
        op_id: uuid::Uuid,
        reason: String,
        now_ms: i64,
    ) -> anyhow::Result<Committed<bool>> {
        self.write(move |write| {
            write.apply_sync_metadata(|sql| {
                let changed =
                    operation::quarantine_outbox_op_in_transaction(sql, op_id, &reason, now_ms)?;
                Ok((changed, changed))
            })
        })
        .await
    }

    pub async fn release_quarantined_outbox(&self) -> anyhow::Result<Committed<usize>> {
        self.write(|write| {
            write.apply_sync_metadata(|sql| {
                let released = operation::release_quarantined_outbox_in_transaction(sql)?;
                Ok((released, released != 0))
            })
        })
        .await
    }
}

impl<'connection> NotesTransaction<'_, 'connection> {
    // This private callback is confined to the named commands above; it does
    // not expose arbitrary metadata keys or SQL to store callers.
    fn apply_sync_metadata<T>(
        &mut self,
        command: impl FnOnce(&rusqlite::Transaction<'connection>) -> CoreResult<(T, bool)>,
    ) -> CoreResult<T> {
        let result = (|| {
            if self.failed || self.applied {
                return Err(CoreError::invalid(
                    "a store command must apply one ordered batch",
                ));
            }
            let (value, changed) = command(self.sql)?;
            self.durable_changed |= changed;
            self.applied = true;
            Ok(value)
        })();
        self.failed |= result.is_err();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::ProjectionPolicy;
    use crate::store::CommitNotice;
    use crate::{Connection, OpKind, PageCreate, PageKind, PageLayout};
    use std::sync::{Arc, Mutex};

    async fn fixture() -> (Connection, NotesStore, Arc<Mutex<Vec<CommitNotice>>>) {
        let connection = crate::db::open_in_memory().await.unwrap();
        let notices = Arc::new(Mutex::new(Vec::new()));
        let observed = notices.clone();
        let store = NotesStore::new(connection.clone()).with_commit_observer(move |notice| {
            observed.lock().unwrap().push(notice.clone());
        });
        (connection, store, notices)
    }

    async fn state(connection: &Connection) -> serde_json::Value {
        let snapshot = crate::export_sync_snapshot(connection, 0).await.unwrap();
        let metadata = connection
            .call(|database| {
                let values = database
                    .prepare("SELECT key, value FROM sync_meta ORDER BY key")?
                    .query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let version: i64 = database.query_row(
                    "SELECT version FROM store_state WHERE singleton = 1",
                    [],
                    |row| row.get(0),
                )?;
                Ok((values, version))
            })
            .await
            .unwrap();
        serde_json::json!([snapshot, metadata])
    }

    #[tokio::test]
    async fn catching_a_late_metadata_error_cannot_commit_the_binding_delete() {
        let (connection, store, notices) = fixture().await;
        let workspace = crate::db::workspace_uuid(&connection).await.unwrap();
        store.configure_sync("https://first.example").await.unwrap();
        store
            .bind_sync_workspace("https://first.example", workspace, 7)
            .await
            .unwrap();
        connection
            .call(|database| {
                database.execute_batch(
                    "CREATE TRIGGER reject_configuration BEFORE INSERT ON sync_meta
                     WHEN NEW.key = 'server_url'
                     BEGIN SELECT RAISE(FAIL, 'injected metadata failure'); END;",
                )
            })
            .await
            .unwrap();
        notices.lock().unwrap().clear();
        let before = state(&connection).await;
        let error = store
            .write(|write| -> CoreResult<()> {
                let result = write.apply_sync_metadata(|sql| {
                    Ok((
                        (),
                        operation::configure_sync_in_transaction(sql, "https://second.example")?,
                    ))
                });
                assert!(result.is_err());
                Ok(())
            })
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("failed store command cannot commit")
        );
        assert_eq!(state(&connection).await, before);
        assert!(notices.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn metadata_and_content_share_the_one_batch_guard_in_both_directions() {
        for metadata_first in [false, true] {
            let (connection, store, notices) = fixture().await;
            let before = state(&connection).await;
            let error = store
                .write(move |write| -> CoreResult<()> {
                    let content = vec![OpKind::PageCreate(PageCreate {
                        uuid: uuid::Uuid::now_v7(),
                        kind: PageKind::Note,
                        title: Some("Must roll back".into()),
                        layout: PageLayout::Outline,
                        created_at: 1,
                    })];
                    let configure = |sql: &rusqlite::Transaction<'_>| {
                        Ok((
                            (),
                            operation::configure_sync_in_transaction(sql, "https://sync.example")?,
                        ))
                    };
                    if metadata_first {
                        write.apply_sync_metadata(configure)?;
                        assert!(
                            write
                                .apply_local(content, ProjectionPolicy::Immediate)
                                .is_err()
                        );
                    } else {
                        write.apply_local(content, ProjectionPolicy::Immediate)?;
                        assert!(write.apply_sync_metadata(configure).is_err());
                    }
                    Ok(())
                })
                .await
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("failed store command cannot commit")
            );
            assert_eq!(state(&connection).await, before);
            assert!(notices.lock().unwrap().is_empty());
        }
    }
}
