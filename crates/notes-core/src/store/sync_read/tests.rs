use super::*;
use crate::{Hlc, OpKind, Origin, PageSetTitle};
use std::future::Future;
use std::sync::mpsc;
use std::task::Waker;
use std::time::Duration;
use uuid::Uuid;

async fn fixture() -> (tempfile::TempDir, NotesStore) {
    let directory = tempfile::tempdir().unwrap();
    let store = NotesStore::open(directory.path().join("notes.db"))
        .await
        .unwrap();
    store.configure_sync("https://sync.example").await.unwrap();
    (directory, store)
}

#[tokio::test]
async fn baseline_preserves_sendable_order_and_excludes_quarantine_in_one_snapshot() {
    let (_directory, store) = fixture().await;
    for title in ["First", "Held", "Last"] {
        store.create_page(title.into()).await.unwrap();
    }
    let queued = operation::pending_outbox(store.read_connection(), u32::MAX)
        .await
        .unwrap();
    store
        .quarantine_outbox_op(queued[1].op_id, "try again later".into(), 10)
        .await
        .unwrap();
    let (snapshot, pending) = store.capture_sync_baseline(123).await.unwrap();
    assert_eq!(snapshot.seq, 123);
    assert_eq!(snapshot.pages.len(), 3);
    assert_eq!(pending, vec![queued[0].clone(), queued[2].clone()]);
    assert_eq!(
        snapshot,
        operation::export_sync_snapshot(store.read_connection(), 123)
            .await
            .unwrap()
    );
    store.close().await.unwrap();
}

#[tokio::test]
async fn public_snapshots_and_queue_status_do_not_wait_for_the_writer_worker() {
    let (_directory, store) = fixture().await;
    let page = store
        .create_page("Already committed".into())
        .await
        .unwrap()
        .value;
    let expected = store.capture_sync_baseline(5).await.unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut blocked = Box::pin(store.write_connection().call(move |_| {
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        Ok(())
    }));
    let mut context = std::task::Context::from_waker(Waker::noop());
    assert!(blocked.as_mut().poll(&mut context).is_pending());
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let baseline =
        tokio::time::timeout(Duration::from_secs(2), store.capture_sync_baseline(5)).await;
    let status = tokio::time::timeout(Duration::from_secs(2), store.sync_queue_status()).await;
    let archive = tokio::time::timeout(Duration::from_secs(2), store.export_archive()).await;
    // Release even on assertion failure; do not leave a blocked SQLite worker.
    release_tx.send(()).unwrap();
    blocked.await.unwrap();
    assert_eq!(baseline.unwrap().unwrap(), expected);
    assert_eq!(
        status.unwrap().unwrap(),
        SyncQueueStatus {
            pending_operations: 1,
            ..Default::default()
        }
    );
    let archive = archive.unwrap().unwrap();
    assert_eq!(archive.workspace_uuid, expected.0.workspace_uuid);
    assert_eq!(archive.pages.len(), 1);
    assert_eq!(archive.pages[0].uuid, page.uuid);
    assert_eq!(archive.pages[0].title, page.title);
    store.close().await.unwrap();
}

#[tokio::test]
async fn pinned_baseline_keeps_remote_content_and_outbox_at_the_same_wal_boundary() {
    let (_directory, store) = fixture().await;
    let page = store
        .create_page("Before remote".into())
        .await
        .unwrap()
        .value;
    let before = store.capture_sync_baseline(5).await.unwrap();
    let workspace = before.0.workspace_uuid;
    let wall_ms = before.1[0].hlc.wall_ms() + 1_000;
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut read = Box::pin(store.read_connection().call_domain(
        move |database| -> anyhow::Result<_> {
            let transaction = database.transaction()?;
            // Pin the same snapshot before allowing independent WAL writers.
            let _: i64 =
                transaction.query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))?;
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            let baseline = read_sync_baseline(&transaction, 5)?;
            transaction.commit()?;
            Ok(baseline)
        },
    ));
    let mut context = std::task::Context::from_waker(Waker::noop());
    assert!(read.as_mut().poll(&mut context).is_pending());
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let device_id = Uuid::now_v7();
    store
        .apply_batch(
            &[Op {
                op_id: Uuid::now_v7(),
                workspace_uuid: workspace,
                device_id,
                hlc: Hlc::new(wall_ms, 0, device_id),
                format_version: operation::FORMAT_VERSION,
                kind: OpKind::PageSetTitle(PageSetTitle {
                    uuid: page.uuid,
                    title: Some("After remote".into()),
                }),
            }],
            Origin::Remote,
        )
        .await
        .unwrap();
    assert_eq!(
        operation::pending_outbox(store.write_connection(), u32::MAX)
            .await
            .unwrap(),
        before.1,
        "remote content can change while the outbox remains identical"
    );
    store.create_page("Later local".into()).await.unwrap();
    release_tx.send(()).unwrap();
    assert_eq!(read.await.unwrap(), before);
    let (current, pending) = store.capture_sync_baseline(5).await.unwrap();
    assert_eq!(current.pages.len(), 2);
    assert_eq!(
        current
            .pages
            .iter()
            .find(|p| p.uuid == page.uuid)
            .unwrap()
            .title
            .as_deref(),
        Some("After remote")
    );
    assert_eq!(pending.len(), 2);
    store.close().await.unwrap();
}

#[tokio::test]
async fn queue_status_counts_without_decoding_envelopes_and_keeps_first_reason_order() {
    let (_directory, store) = fixture().await;
    for title in ["First held", "Pending", "Second held"] {
        store.create_page(title.into()).await.unwrap();
    }
    let operations = operation::pending_outbox(store.read_connection(), u32::MAX)
        .await
        .unwrap();
    for (index, reason) in [(2, "later reason"), (0, "first reason")] {
        store
            .quarantine_outbox_op(operations[index].op_id, reason.into(), 12)
            .await
            .unwrap();
    }
    store
        .bind_sync_workspace("https://sync.example", operations[0].workspace_uuid, 17)
        .await
        .unwrap();
    assert_eq!(
        store.sync_queue_status().await.unwrap(),
        SyncQueueStatus {
            cursor: 17,
            pending_operations: 1,
            quarantined_operations: 2,
            quarantine_reason: Some("first reason".into()),
        }
    );
    let first = operations[0].op_id;
    store
        .write_connection()
        .call(move |database| {
            database.execute("UPDATE sync_outbox SET envelope='not an envelope'", [])?;
            database.execute(
                "UPDATE sync_outbox SET quarantine_reason=NULL WHERE op_id=?1",
                [first],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        store.sync_queue_status().await.unwrap(),
        SyncQueueStatus {
            cursor: 17,
            pending_operations: 1,
            quarantined_operations: 2,
            quarantine_reason: Some(String::new()),
        }
    );
    // A baseline still decodes actual operations and rejects malformed data.
    assert!(store.capture_sync_baseline(5).await.is_err());
    store.close().await.unwrap();
}
