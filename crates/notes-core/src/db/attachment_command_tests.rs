use super::*;
use crate::store::{CommitNotice, NotesStore};
use std::sync::{Arc, Mutex};

async fn fixture() -> (
    Connection,
    Page,
    Block,
    NotesStore,
    Arc<Mutex<Vec<CommitNotice>>>,
) {
    let connection = open_in_memory().await.unwrap();
    operation::configure_sync(&connection, "https://sync.example")
        .await
        .unwrap();
    let note = create_note(&connection, None)
        .await
        .unwrap()
        .into_created()
        .unwrap();
    let notices = Arc::new(Mutex::new(Vec::new()));
    let capture = notices.clone();
    let store = NotesStore::new(connection.clone()).with_commit_observer(move |notice| {
        capture.lock().unwrap().push(notice.clone());
    });
    (connection, note.page, note.initial_block, store, notices)
}

async fn state(connection: &Connection) -> serde_json::Value {
    let snapshot = operation::export_sync_snapshot(connection, 0)
        .await
        .unwrap();
    let local = connection
        .call(|db| {
            let counts: (i64, i64, i64) = db.query_row(
                "SELECT (SELECT COUNT(*) FROM applied_ops), (SELECT COUNT(*) FROM sync_outbox),
                    (SELECT version FROM store_state)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
            let meta = db
                .prepare("SELECT key, value FROM sync_meta ORDER BY key")?
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok((counts, meta))
        })
        .await
        .unwrap();
    let history = history_status(connection).await.unwrap();
    serde_json::json!({ "snapshot": snapshot, "local": local, "history": history })
}

#[tokio::test]
async fn attachment_commands_publish_precise_owner_facts_and_no_history() {
    let (connection, page, block, store, notices) = fixture().await;
    let history = history_status(&connection).await.unwrap();
    for (index, owner) in [
        AttachmentOwner::Page(page.uuid),
        AttachmentOwner::Block(block.uuid),
    ]
    .into_iter()
    .enumerate()
    {
        let added = store
            .create_attachment(
                owner,
                BlobHash::digest(&[index as u8]),
                "image.png".into(),
                "image/png".into(),
                12,
            )
            .await
            .unwrap();
        assert_eq!(added.changes.attachment_owners, vec![owner.uuid()]);
        assert_eq!(added.changes.affected_pages, vec![page.uuid]);
        assert!(added.changes.blocks.is_empty());
        assert!(added.changes.graph_changed_content.is_empty());
        assert!(!added.changes.history_changed);
        assert_eq!(
            get_attachment(&connection, added.value.uuid).await.unwrap(),
            Some(added.value.clone())
        );
        let removed = store.delete_attachment(added.value.uuid).await.unwrap();
        assert_eq!(removed.value, Some(added.value.clone()));
        assert_eq!(removed.changes.attachment_owners, vec![owner.uuid()]);
        assert!(
            get_attachment(&connection, added.value.uuid)
                .await
                .unwrap()
                .is_none()
        );
        let empty = store.delete_attachment(added.value.uuid).await.unwrap();
        assert!(empty.value.is_none());
        assert!(empty.commit_version.is_none());
        assert!(empty.changes.is_empty());
    }
    {
        let captured = notices.lock().unwrap();
        assert_eq!(captured.len(), 4);
        assert!(
            captured
                .windows(2)
                .all(|pair| pair[1].commit_version == pair[0].commit_version + 1)
        );
    }
    assert_eq!(
        serde_json::to_value(history_status(&connection).await.unwrap()).unwrap(),
        serde_json::to_value(history).unwrap()
    );
}

#[tokio::test]
async fn invalid_attachment_owner_or_metadata_changes_no_persistent_state() {
    let (connection, page, _, store, notices) = fixture().await;
    let before = state(&connection).await;
    let missing = store
        .create_attachment(
            AttachmentOwner::Page(uuid::Uuid::now_v7()),
            BlobHash::digest(b"missing"),
            "image.png".into(),
            "image/png".into(),
            12,
        )
        .await
        .unwrap_err();
    assert!(missing.to_string().contains("owner was not found"));
    assert_eq!(state(&connection).await, before);
    let invalid = store
        .create_attachment(
            AttachmentOwner::Page(page.uuid),
            BlobHash::digest(b"invalid"),
            "../image.png".into(),
            "image/png".into(),
            12,
        )
        .await
        .unwrap_err();
    assert!(invalid.to_string().contains("portable basename"));
    assert_eq!(state(&connection).await, before);
    assert!(notices.lock().unwrap().is_empty());
}

#[tokio::test]
async fn failed_attachment_materialization_rolls_back_intent_clocks_and_outbox() {
    let (connection, page, _, store, notices) = fixture().await;
    connection
        .call(|db| {
            db.execute_batch(
                "CREATE TRIGGER reject_attachment BEFORE INSERT ON attachments
         BEGIN SELECT RAISE(ABORT, 'reject attachment'); END;",
            )
        })
        .await
        .unwrap();
    let before = state(&connection).await;
    let error = store
        .create_attachment(
            AttachmentOwner::Page(page.uuid),
            BlobHash::digest(b"rollback"),
            "image.png".into(),
            "image/png".into(),
            12,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("reject attachment"));
    assert_eq!(state(&connection).await, before);
    assert!(notices.lock().unwrap().is_empty());
}
