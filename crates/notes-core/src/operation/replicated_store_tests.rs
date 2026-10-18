//! Executor contract regressions for externally supplied operation envelopes.

use super::*;
use crate::store::{CommitNotice, NotesStore};
use std::sync::{Arc, Mutex};

type Notices = Arc<Mutex<Vec<CommitNotice>>>;

async fn fixture() -> (Connection, NotesStore, Notices, uuid::Uuid) {
    let connection = crate::db::open_in_memory().await.unwrap();
    configure_sync(&connection, "https://replicated.example")
        .await
        .unwrap();
    let workspace = crate::db::workspace_uuid(&connection).await.unwrap();
    let notices = Arc::new(Mutex::new(Vec::new()));
    let observed = notices.clone();
    let store = NotesStore::new(connection.clone()).with_commit_observer(move |notice| {
        observed.lock().unwrap().push(notice.clone());
    });
    (connection, store, notices, workspace)
}

fn operation(workspace: uuid::Uuid, index: u128) -> Op {
    let device_id = uuid::Uuid::from_u128(99);
    Op {
        op_id: uuid::Uuid::from_u128(100 + index),
        workspace_uuid: workspace,
        device_id,
        hlc: Hlc::new(index as u64, 0, device_id),
        format_version: FORMAT_VERSION,
        kind: OpKind::PageCreate(PageCreate {
            uuid: uuid::Uuid::from_u128(index),
            kind: PageKind::Note,
            title: Some(format!("Page {index}")),
            layout: PageLayout::Outline,
            created_at: 1,
        }),
    }
}

async fn state(connection: &Connection) -> serde_json::Value {
    let content = export_sync_snapshot(connection, 0).await.unwrap();
    let metadata = connection
        .call(|database| {
            let values = database
                .prepare("SELECT key, value FROM sync_meta ORDER BY key")?
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let applied = database
                .prepare("SELECT op_id, seq FROM applied_ops ORDER BY op_id")?
                .query_map([], |row| {
                    Ok((row.get::<_, uuid::Uuid>(0)?, row.get::<_, Option<i64>>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let outbox = database
                .prepare("SELECT op_id, envelope FROM sync_outbox ORDER BY op_id")?
                .query_map([], |row| {
                    Ok((row.get::<_, uuid::Uuid>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let version: i64 =
                database.query_row("SELECT version FROM store_state", [], |row| row.get(0))?;
            Ok((values, applied, outbox, version))
        })
        .await
        .unwrap();
    serde_json::json!([content, metadata])
}

#[tokio::test]
async fn catching_a_late_sequenced_apply_error_cannot_commit_its_prefix() {
    let (connection, store, notices, workspace) = fixture().await;
    let before = state(&connection).await;
    let operations = vec![(1, operation(workspace, 1)), (3, operation(workspace, 2))];
    let error = store
        .write(move |write| -> CoreResult<()> {
            let error = write.apply_sequenced_batch(&operations).unwrap_err();
            assert!(matches!(
                error,
                CoreError::SyncSequenceGap {
                    expected: 2,
                    received: 3
                }
            ));
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
async fn catching_a_second_batch_error_rolls_back_the_successful_first_batch() {
    for second_is_ack in [false, true] {
        let (connection, store, notices, workspace) = fixture().await;
        let before = state(&connection).await;
        let first = operation(workspace, 1);
        let second = operation(workspace, 2);
        let error = store
            .write(move |write| -> CoreResult<()> {
                write.apply_replicated_batch(std::slice::from_ref(&first), Origin::Remote)?;
                let failed = if second_is_ack {
                    write.acknowledge_server_ops(&[(first.op_id, 1)]).is_err()
                } else {
                    write
                        .apply_replicated_batch(&[second], Origin::Remote)
                        .is_err()
                };
                assert!(failed);
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

#[tokio::test]
async fn catching_a_late_ack_error_cannot_persist_partial_sequence_assignments() {
    let (connection, store, notices, workspace) = fixture().await;
    let first = operation(workspace, 1);
    let second = operation(workspace, 2);
    store
        .apply_batch(&[first.clone(), second.clone()], Origin::Local)
        .await
        .unwrap();
    notices.lock().unwrap().clear();
    let before = state(&connection).await;
    let error = store
        .write(move |write| -> CoreResult<()> {
            assert!(
                write
                    .acknowledge_server_ops(&[(first.op_id, 1), (second.op_id, 1)])
                    .is_err()
            );
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
async fn explicit_full_bulk_helper_skips_tracking_and_preserves_its_loader_mode() {
    let (connection, _store, notices, workspace) = fixture().await;
    let page = operation(workspace, 1);
    let block = Op {
        kind: OpKind::BlockCreate(BlockCreate {
            uuid: uuid::Uuid::from_u128(10),
            page_uuid: uuid::Uuid::from_u128(1),
            parent_uuid: None,
            order_key: OrderKey::first(),
            style: BlockStyle::Bullet,
            markdown: "Block".into(),
            created_at: 1,
        }),
        ..operation(workspace, 2)
    };
    connection
        .call_domain(move |database| -> CoreResult<()> {
            let tx = database.transaction()?;
            let applied = apply_batch_in_transaction(
                &tx,
                &[page, block],
                Origin::Remote,
                StructureProjectionMode::Full,
                false,
            )?;
            assert!(applied.outcomes.iter().all(|outcome| outcome.applied));
            assert!(applied.changes.is_empty());
            assert!(applied.operations.is_empty());
            assert!(applied.durable_changed);
            assert_eq!(applied.stats.full_structure_reconciliations, 1);
            tx.commit()?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(
        crate::db::get_block(&connection, uuid::Uuid::from_u128(10))
            .await
            .unwrap()
            .is_some()
    );
    assert!(notices.lock().unwrap().is_empty());
}
