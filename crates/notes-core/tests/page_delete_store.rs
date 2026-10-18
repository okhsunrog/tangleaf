use notes_core::db::{self, BlockContent};
use notes_core::store::{CommitNotice, NotesStore};
use notes_core::{
    AttachmentOwner, BlobHash, BlockStyle, Connection, CoreError, Hlc, Op, OpKind, Origin,
    PageAlias, PageAliasSet,
};
use rusqlite::types::Value;
use std::collections::BTreeMap;
use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::sync::{Arc, Mutex, mpsc};
use std::task::Poll;
use uuid::Uuid;

struct Fixture {
    directory: tempfile::TempDir,
    connection: Connection,
    store: NotesStore,
    notices: Arc<Mutex<Vec<CommitNotice>>>,
}

async fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(directory.path().join("notes.db")).await.unwrap();
    notes_core::configure_sync(&connection, "https://delete.example.test")
        .await
        .unwrap();
    let notices = Arc::new(Mutex::new(Vec::new()));
    let observed = notices.clone();
    let store = NotesStore::new(connection.clone()).with_commit_observer(move |notice| {
        observed.lock().unwrap().push(notice.clone());
    });
    Fixture {
        directory,
        connection,
        store,
        notices,
    }
}

fn sorted(values: impl IntoIterator<Item = Uuid>) -> Vec<Uuid> {
    let mut values = values.into_iter().collect::<Vec<_>>();
    values.sort_unstable();
    values
}

async fn counters(connection: &Connection) -> (i64, i64, i64, i64) {
    connection.call(|database| database.query_row(
        "SELECT (SELECT COUNT(*) FROM applied_ops),(SELECT COUNT(*) FROM sync_outbox),(SELECT COUNT(*) FROM history_undo),(SELECT version FROM store_state)",
        [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    )).await.unwrap()
}

async fn rows(connection: &Connection) -> BTreeMap<String, Vec<Vec<Value>>> {
    connection
        .call(|database| {
            let names = database
                .prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")?
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut result = BTreeMap::new();
            for name in names {
                let mut statement = database
                    .prepare(&format!("SELECT * FROM \"{}\"", name.replace('"', "\"\"")))?;
                let columns = statement.column_count();
                let mut entries = statement
                    .query_map([], |row| {
                        (0..columns)
                            .map(|index| row.get(index))
                            .collect::<rusqlite::Result<Vec<Value>>>()
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                entries.sort_by_cached_key(|row| format!("{row:?}"));
                result.insert(name, entries);
            }
            Ok(result)
        })
        .await
        .unwrap()
}

async fn alias_target(connection: &Connection, block: Uuid) -> Option<Uuid> {
    connection.call(move |database| database.query_row("SELECT target_page_uuid FROM page_links WHERE source_block_uuid=?1 AND target_title='target-alias'", [block], |row| row.get(0))).await.unwrap()
}

#[tokio::test]
async fn subtree_attachments_aliases_and_incoming_links_publish_one_exact_commit() {
    let f = fixture().await;
    let note = db::create_note(&f.connection, Some("Delete target".into()))
        .await
        .unwrap()
        .into_created()
        .unwrap();
    let child = db::create_block(
        &f.connection,
        note.page.uuid,
        Some(note.initial_block.uuid),
        None,
        BlockStyle::Bullet,
        "Child".into(),
    )
    .await
    .unwrap();
    let grandchild = db::create_block(
        &f.connection,
        note.page.uuid,
        Some(child.uuid),
        None,
        BlockStyle::Bullet,
        "Grandchild".into(),
    )
    .await
    .unwrap();
    let page_attachment = db::create_attachment(
        &f.connection,
        AttachmentOwner::Page(note.page.uuid),
        BlobHash::from_bytes([1; 32]),
        "page.bin".into(),
        "application/octet-stream".into(),
        10,
    )
    .await
    .unwrap();
    let block_attachment = db::create_attachment(
        &f.connection,
        AttachmentOwner::Block(grandchild.uuid),
        BlobHash::from_bytes([2; 32]),
        "block.bin".into(),
        "application/octet-stream".into(),
        20,
    )
    .await
    .unwrap();
    let device = Uuid::now_v7();
    notes_core::apply(
        &f.connection,
        &Op {
            op_id: Uuid::now_v7(),
            workspace_uuid: db::workspace_uuid(&f.connection).await.unwrap(),
            device_id: device,
            hlc: Hlc::new(
                chrono::Utc::now().timestamp_millis().max(0) as u64 + 10,
                0,
                device,
            ),
            format_version: notes_core::operation::FORMAT_VERSION,
            kind: OpKind::PageAliasSet(PageAliasSet {
                uuid: note.page.uuid,
                alias: PageAlias::new("target-alias").unwrap(),
                present: true,
            }),
        },
        Origin::Remote,
    )
    .await
    .unwrap();
    let source = db::create_note(&f.connection, Some("Source".into()))
        .await
        .unwrap()
        .into_created()
        .unwrap();
    db::set_block_content(
        &f.connection,
        source.initial_block.uuid,
        BlockContent {
            markdown: "[[target-alias]]".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        alias_target(&f.connection, source.initial_block.uuid).await,
        Some(note.page.uuid)
    );
    let before = counters(&f.connection).await;
    let deleted = f.store.delete_page(note.page.uuid).await.unwrap();
    let value = deleted.value.as_ref().unwrap();
    assert_eq!(value.page_uuid, note.page.uuid);
    assert_eq!(
        value.block_uuids,
        [grandchild.uuid, child.uuid, note.initial_block.uuid]
    );
    assert_eq!(
        sorted(value.attachments.iter().map(|attachment| attachment.uuid)),
        sorted([page_attachment.uuid, block_attachment.uuid])
    );
    assert_eq!(value.operations.len(), 6);
    assert!(
        value.operations[..2]
            .iter()
            .all(|kind| matches!(kind, OpKind::AttachmentRemove(_)))
    );
    assert!(
        matches!(value.operations.last(), Some(OpKind::PageDelete(payload)) if payload.uuid == note.page.uuid)
    );
    assert_eq!(deleted.changes.deleted_pages, [note.page.uuid]);
    assert!(deleted.changes.changed_pages.is_empty());
    assert_eq!(
        deleted.changes.affected_pages,
        sorted([note.page.uuid, source.page.uuid])
    );
    assert_eq!(
        deleted.changes.attachment_owners,
        sorted([note.page.uuid, grandchild.uuid])
    );
    assert_eq!(
        deleted.changes.graph_changed_content,
        sorted([
            note.page.uuid,
            note.initial_block.uuid,
            child.uuid,
            grandchild.uuid,
            source.initial_block.uuid
        ])
    );
    assert_eq!(
        deleted
            .changes
            .blocks
            .iter()
            .map(|block| block.uuid)
            .collect::<Vec<_>>(),
        sorted([note.initial_block.uuid, child.uuid, grandchild.uuid])
    );
    for change in &deleted.changes.blocks {
        assert!(change.after.is_none());
        let before = change.before.as_ref().unwrap();
        assert_eq!(before.page_uuid, note.page.uuid);
        let expected_parent = if change.uuid == grandchild.uuid {
            Some(child.uuid)
        } else if change.uuid == child.uuid {
            Some(note.initial_block.uuid)
        } else {
            None
        };
        assert_eq!(before.parent_uuid, expected_parent);
    }
    assert!(deleted.changes.history_changed);
    assert_eq!(deleted.commit_version, Some(before.3 + 1));
    assert_eq!(
        counters(&f.connection).await,
        (before.0 + 6, before.1 + 6, before.2 + 1, before.3 + 1)
    );
    assert_eq!(
        f.notices.lock().unwrap().as_slice(),
        [CommitNotice {
            commit_version: before.3 + 1,
            workspace_epoch: deleted.workspace_epoch,
            workspace_replaced: false,
            changes: deleted.changes.clone()
        }]
    );
    assert_eq!(
        alias_target(&f.connection, source.initial_block.uuid).await,
        None
    );
    assert!(
        db::get_page_by_title(&f.connection, "target-alias".into())
            .await
            .unwrap()
            .is_none()
    );
    f.store.undo().await.unwrap();
    assert_eq!(
        alias_target(&f.connection, source.initial_block.uuid).await,
        Some(note.page.uuid)
    );
    assert_eq!(
        db::get_page_by_title(&f.connection, "target-alias".into())
            .await
            .unwrap()
            .unwrap()
            .uuid,
        note.page.uuid
    );
    assert_eq!(
        db::get_block(&f.connection, grandchild.uuid)
            .await
            .unwrap()
            .unwrap()
            .parent_uuid,
        Some(child.uuid)
    );
    for expected in [page_attachment, block_attachment] {
        let mut restored = db::list_attachments(&f.connection, expected.owner)
            .await
            .unwrap();
        assert_eq!(restored.len(), 1);
        // Undo authors a new attachment operation; its timestamp is fresh.
        restored[0].created_at = expected.created_at;
        assert_eq!(restored, [expected]);
    }
}

#[tokio::test]
async fn absent_page_is_quiet_and_legacy_adapter_keeps_its_result() {
    let f = fixture().await;
    let before = rows(&f.connection).await;
    let missing = f.store.delete_page(Uuid::now_v7()).await.unwrap();
    assert!(missing.value.is_none());
    assert!(missing.changes.is_empty());
    assert_eq!(missing.commit_version, None);
    assert_eq!(rows(&f.connection).await, before);
    assert!(f.notices.lock().unwrap().is_empty());
    let note = db::create_note(&f.connection, None)
        .await
        .unwrap()
        .into_created()
        .unwrap();
    let legacy = db::delete_page(&f.connection, note.page.uuid)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(legacy.page_uuid, note.page.uuid);
    assert_eq!(legacy.block_uuids, [note.initial_block.uuid]);
    assert_eq!(legacy.operations.len(), 2);
    assert!(
        f.notices.lock().unwrap().is_empty(),
        "observer-free adapter does not publish through a different handle"
    );
}

async fn block_worker(connection: &Connection) -> (mpsc::Sender<()>, tokio::task::JoinHandle<()>) {
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let connection = connection.clone();
    let worker = tokio::spawn(async move {
        connection
            .call(move |_| {
                let _ = entered_tx.send(());
                let _ = release_rx.recv();
                Ok(())
            })
            .await
            .unwrap();
    });
    entered_rx.await.unwrap();
    (release_tx, worker)
}

async fn enqueue<F: Future>(mut future: Pin<&mut F>) {
    poll_fn(|context| {
        assert!(future.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
}

#[tokio::test]
async fn queued_creation_and_deletion_use_one_current_state_in_either_order() {
    for delete_first in [false, true] {
        let f = fixture().await;
        let note = db::create_note(&f.connection, None)
            .await
            .unwrap()
            .into_created()
            .unwrap();
        let before = counters(&f.connection).await;
        let (release, blocker) = block_worker(&f.connection).await;
        let mut create = Box::pin(f.store.create_block(
            note.page.uuid,
            Some(note.initial_block.uuid),
            None,
            BlockStyle::Bullet,
            "Queued child".into(),
        ));
        let mut delete = Box::pin(f.store.delete_page(note.page.uuid));
        if delete_first {
            enqueue(delete.as_mut()).await;
            enqueue(create.as_mut()).await;
        } else {
            enqueue(create.as_mut()).await;
            enqueue(delete.as_mut()).await;
        }
        release.send(()).unwrap();
        blocker.await.unwrap();
        let deleted = delete.await.unwrap();
        if delete_first {
            let error = create.await.unwrap_err();
            assert!(matches!(
                error.downcast_ref::<CoreError>(),
                Some(CoreError::NotFound(_))
            ));
            assert_eq!(
                deleted.value.unwrap().block_uuids,
                [note.initial_block.uuid]
            );
            assert_eq!(
                counters(&f.connection).await,
                (before.0 + 2, before.1 + 2, before.2 + 1, before.3 + 1)
            );
            assert_eq!(f.notices.lock().unwrap().len(), 1);
        } else {
            let child = create.await.unwrap().value;
            assert_eq!(
                deleted.value.unwrap().block_uuids,
                [child.uuid, note.initial_block.uuid]
            );
            assert_eq!(
                counters(&f.connection).await,
                (before.0 + 4, before.1 + 4, before.2 + 2, before.3 + 2)
            );
            assert_eq!(f.notices.lock().unwrap().len(), 2);
            f.store.undo().await.unwrap();
            assert_eq!(
                db::get_block(&f.connection, child.uuid)
                    .await
                    .unwrap()
                    .unwrap()
                    .parent_uuid,
                Some(note.initial_block.uuid)
            );
        }
    }
}

#[tokio::test]
async fn history_failure_rolls_back_ink_retention_and_every_persistent_write() {
    let f = fixture().await;
    let page = db::create_handwritten_note_with_ops(&f.connection, Some("Ink".into()))
        .await
        .unwrap()
        .value;
    let ink = notes_core::ink::Store::new(f.directory.path().join("notes.db"), page.uuid);
    ink.patch(
        notes_core::ink::InkDraftPatch {
            order: vec![],
            upserts: vec![],
            background: notes_core::ink::InkBackground::Grid,
        },
        None,
    )
    .unwrap();
    f.connection.call(|database| database.execute_batch("CREATE TRIGGER fail_page_delete_history BEFORE INSERT ON history_undo WHEN NEW.action='delete page' BEGIN SELECT RAISE(ABORT,'delete history failed'); END")).await.unwrap();
    let before = rows(&f.connection).await;
    assert!(f.store.delete_page(page.uuid).await.is_err());
    assert_eq!(rows(&f.connection).await, before);
    assert!(matches!(
        ink.read().unwrap().draft.background,
        notes_core::ink::InkBackground::Grid
    ));
    assert!(f.notices.lock().unwrap().is_empty());
}

#[tokio::test]
async fn observed_ink_delete_undo_redo_uses_the_retained_native_drawing() {
    let f = fixture().await;
    let page = f
        .store
        .create_handwritten_note(Some("Ink".into()))
        .await
        .unwrap()
        .value;
    f.notices.lock().unwrap().clear();
    let ink = notes_core::ink::Store::new(f.directory.path().join("notes.db"), page.uuid);
    ink.patch(
        notes_core::ink::InkDraftPatch {
            order: vec![],
            upserts: vec![],
            background: notes_core::ink::InkBackground::Grid,
        },
        None,
    )
    .unwrap();
    let before = counters(&f.connection).await;
    let deleted = f.store.delete_page(page.uuid).await.unwrap();
    assert_eq!(deleted.changes.deleted_pages, [page.uuid]);
    assert_eq!(deleted.changes.affected_pages, [page.uuid]);
    assert!(deleted.changes.blocks.is_empty());
    assert!(deleted.changes.history_changed);
    assert_eq!(deleted.commit_version, Some(before.3 + 1));
    assert_eq!(f.notices.lock().unwrap().len(), 1);
    f.store.undo().await.unwrap();
    assert!(matches!(
        ink.read().unwrap().draft.background,
        notes_core::ink::InkBackground::Grid
    ));
    assert!(ink.open_editor(false).unwrap().can_undo);
    f.store.redo().await.unwrap();
    assert!(ink.read().is_err());
    assert_eq!(f.notices.lock().unwrap().len(), 3);
    assert_eq!(counters(&f.connection).await.3, before.3 + 3);
}
