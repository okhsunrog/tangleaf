use notes_core::db;
use notes_core::store::{CommitNotice, NotesStore};
use notes_core::{BlockStyle, Connection, Hlc, Op, OpKind, Origin, PageAlias, PageAliasSet};
use std::sync::{Arc, Mutex};

struct Fixture {
    _directory: tempfile::TempDir,
    connection: Connection,
    store: NotesStore,
    notices: Arc<Mutex<Vec<CommitNotice>>>,
    initial_version: i64,
}

async fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(directory.path().join("notes.db")).await.unwrap();
    notes_core::configure_sync(&connection, "https://sync.example")
        .await
        .unwrap();
    let initial_version = counters(&connection).await.5;
    let notices = Arc::new(Mutex::new(Vec::new()));
    let observed = notices.clone();
    let store = NotesStore::new(connection.clone()).with_commit_observer(move |notice| {
        observed.lock().unwrap().push(notice.clone());
    });
    Fixture {
        _directory: directory,
        connection,
        store,
        notices,
        initial_version,
    }
}

async fn counters(connection: &Connection) -> (i64, i64, i64, i64, i64, i64) {
    connection
        .call(|database| {
            database.query_row(
                "SELECT (SELECT COUNT(*) FROM pages), (SELECT COUNT(*) FROM blocks),
                (SELECT COUNT(*) FROM applied_ops), (SELECT COUNT(*) FROM sync_outbox),
                (SELECT COUNT(*) FROM history_undo),
                (SELECT version FROM store_state WHERE singleton = 1)",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
        })
        .await
        .unwrap()
}

async fn last_hlc(connection: &Connection) -> Option<String> {
    use rusqlite::OptionalExtension;
    connection
        .call(|database| {
            database
                .query_row(
                    "SELECT value FROM sync_meta WHERE key = 'last_hlc'",
                    [],
                    |row| row.get(0),
                )
                .optional()
        })
        .await
        .unwrap()
}

async fn alias(connection: &Connection, page_uuid: uuid::Uuid, title: &str) {
    let device_id = uuid::Uuid::now_v7();
    notes_core::apply(
        connection,
        &Op {
            op_id: uuid::Uuid::now_v7(),
            workspace_uuid: db::workspace_uuid(connection).await.unwrap(),
            device_id,
            hlc: Hlc::new(1, 0, device_id),
            format_version: notes_core::operation::FORMAT_VERSION,
            kind: OpKind::PageAliasSet(PageAliasSet {
                uuid: page_uuid,
                alias: PageAlias::new(title).unwrap(),
                present: true,
            }),
        },
        Origin::Remote,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn new_capture_is_one_ordered_batch_one_notice_and_one_undo_action() {
    let fixture = fixture().await;
    let markdown = "  first paragraph\n\nwith **formatting**\n";
    let captured = fixture
        .store
        .create_page_with_markdown(" Captured ".into(), markdown.into())
        .await
        .unwrap();
    let page = captured.value;
    assert_eq!(page.title.as_deref(), Some("Captured"));
    let blocks = db::list_block_children(&fixture.connection, page.uuid, None)
        .await
        .unwrap();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].markdown, markdown);
    assert_eq!(blocks[0].style, BlockStyle::Paragraph);
    assert_eq!(captured.changes.changed_pages, vec![page.uuid]);
    assert_eq!(captured.changes.blocks.len(), 1);
    assert_eq!(captured.changes.blocks[0].uuid, blocks[0].uuid);
    assert!(captured.changes.blocks[0].before.is_none());
    assert!(captured.changes.history_changed);
    assert_eq!(captured.commit_version, Some(fixture.initial_version + 1));
    assert_eq!(
        counters(&fixture.connection).await,
        (1, 1, 2, 2, 1, fixture.initial_version + 1)
    );
    assert_eq!(fixture.notices.lock().unwrap().len(), 1);
    let pending = notes_core::pending_outbox(&fixture.connection, 10)
        .await
        .unwrap();
    assert!(matches!(&pending[0].kind, OpKind::PageCreate(create) if create.uuid == page.uuid));
    assert!(
        matches!(&pending[1].kind, OpKind::BlockCreate(create) if create.uuid == blocks[0].uuid && create.markdown == markdown)
    );

    fixture.store.undo().await.unwrap();
    assert!(
        db::get_page(&fixture.connection, page.uuid)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db::get_block(&fixture.connection, blocks[0].uuid)
            .await
            .unwrap()
            .is_none()
    );
    fixture.store.redo().await.unwrap();
    assert_eq!(
        db::get_block(&fixture.connection, blocks[0].uuid)
            .await
            .unwrap()
            .unwrap()
            .markdown,
        markdown
    );
}

#[tokio::test]
async fn explicit_alias_appends_without_renaming_or_replacing_existing_content() {
    let fixture = fixture().await;
    let owner = fixture
        .store
        .create_page_with_markdown("Canonical title".into(), "original".into())
        .await
        .unwrap()
        .value;
    alias(&fixture.connection, owner.uuid, "Former title").await;
    let original = db::list_block_children(&fixture.connection, owner.uuid, None)
        .await
        .unwrap()
        .remove(0);
    let before = counters(&fixture.connection).await;
    fixture.notices.lock().unwrap().clear();
    let appended = fixture
        .store
        .create_page_with_markdown(" FORMER TITLE ".into(), "appended".into())
        .await
        .unwrap();
    assert_eq!(appended.value.uuid, owner.uuid);
    assert_eq!(appended.value.title, owner.title);
    let blocks = db::list_block_children(&fixture.connection, owner.uuid, None)
        .await
        .unwrap();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0], original);
    assert_eq!(blocks[1].markdown, "appended");
    assert!(blocks[1].order_key > blocks[0].order_key);
    assert_eq!(
        counters(&fixture.connection).await,
        (1, 2, before.2 + 1, before.3 + 1, before.4 + 1, before.5 + 1)
    );
    assert_eq!(fixture.notices.lock().unwrap().len(), 1);
    fixture.store.undo().await.unwrap();
    assert_eq!(
        db::list_block_children(&fixture.connection, owner.uuid, None)
            .await
            .unwrap(),
        vec![original]
    );
}

#[tokio::test]
async fn blank_capture_only_ensures_a_page_and_existing_alias_is_quiet() {
    let fixture = fixture().await;
    let created = fixture
        .store
        .create_page_with_markdown("Blank capture".into(), " \n\t ".into())
        .await
        .unwrap();
    assert!(created.changes.blocks.is_empty());
    assert_eq!(
        counters(&fixture.connection).await,
        (1, 0, 1, 1, 1, fixture.initial_version + 1)
    );
    alias(&fixture.connection, created.value.uuid, "Other name").await;
    let before = counters(&fixture.connection).await;
    fixture.notices.lock().unwrap().clear();
    let existing = fixture
        .store
        .create_page_with_markdown(" OTHER NAME ".into(), String::new())
        .await
        .unwrap();
    assert_eq!(existing.value, created.value);
    assert!(existing.changes.is_empty());
    assert_eq!(existing.commit_version, None);
    assert_eq!(counters(&fixture.connection).await, before);
    assert!(fixture.notices.lock().unwrap().is_empty());
}

#[tokio::test]
async fn paragraph_failure_rolls_back_new_and_existing_capture_including_clocks_and_history() {
    for existing in [false, true] {
        let fixture = fixture().await;
        if existing {
            fixture
                .store
                .create_page_with_markdown("Rollback".into(), "original".into())
                .await
                .unwrap();
        }
        let before = serde_json::to_value(
            notes_core::export_sync_snapshot(&fixture.connection, 0)
                .await
                .unwrap(),
        )
        .unwrap();
        let before_counters = counters(&fixture.connection).await;
        let before_hlc = last_hlc(&fixture.connection).await;
        fixture.notices.lock().unwrap().clear();
        fixture
            .connection
            .call(|database| {
                database.execute_batch(
                    "CREATE TEMP TRIGGER reject_capture BEFORE INSERT ON blocks BEGIN
             SELECT RAISE(ABORT, 'injected paragraph failure'); END;",
                )
            })
            .await
            .unwrap();
        let error = fixture
            .store
            .create_page_with_markdown("Rollback".into(), "rejected".into())
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("injected paragraph failure"),
            "{error:#}"
        );
        assert_eq!(
            serde_json::to_value(
                notes_core::export_sync_snapshot(&fixture.connection, 0)
                    .await
                    .unwrap()
            )
            .unwrap(),
            before
        );
        assert_eq!(counters(&fixture.connection).await, before_counters);
        assert_eq!(last_hlc(&fixture.connection).await, before_hlc);
        assert!(fixture.notices.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn concurrent_named_captures_append_every_exact_paragraph_to_one_page() {
    let fixture = fixture().await;
    let (first, second, third) = tokio::join!(
        fixture
            .store
            .create_page_with_markdown(" Shared ".into(), "first\n".into()),
        fixture
            .store
            .create_page_with_markdown("SHARED".into(), " second".into()),
        fixture
            .store
            .create_page_with_markdown("shared".into(), "third **paragraph**".into()),
    );
    let results = [first.unwrap(), second.unwrap(), third.unwrap()];
    let page_uuid = results[0].value.uuid;
    assert!(results.iter().all(|result| result.value.uuid == page_uuid));
    let mut versions = results
        .iter()
        .map(|result| result.commit_version.unwrap())
        .collect::<Vec<_>>();
    versions.sort_unstable();
    let expected_versions = (1..=3)
        .map(|offset| fixture.initial_version + offset)
        .collect::<Vec<_>>();
    assert_eq!(versions, expected_versions);
    let blocks = db::list_block_children(&fixture.connection, page_uuid, None)
        .await
        .unwrap();
    assert!(
        blocks
            .windows(2)
            .all(|pair| pair[0].order_key < pair[1].order_key)
    );
    let mut markdown = blocks
        .into_iter()
        .map(|block| block.markdown)
        .collect::<Vec<_>>();
    markdown.sort();
    assert_eq!(markdown, vec![" second", "first\n", "third **paragraph**"]);
    assert_eq!(
        counters(&fixture.connection).await,
        (1, 3, 4, 4, 3, fixture.initial_version + 3)
    );
    assert_eq!(
        fixture
            .notices
            .lock()
            .unwrap()
            .iter()
            .map(|notice| notice.commit_version)
            .collect::<Vec<_>>(),
        expected_versions
    );
}

#[tokio::test]
async fn capture_rebalances_exhausted_root_keys_in_the_same_action() {
    let fixture = fixture().await;
    let page = fixture
        .store
        .create_page_with_markdown("Exhausted".into(), "original".into())
        .await
        .unwrap()
        .value;
    let block = db::list_block_children(&fixture.connection, page.uuid, None)
        .await
        .unwrap()
        .remove(0);
    let device_id = uuid::Uuid::now_v7();
    let key = serde_json::from_value::<notes_core::OrderKey>(serde_json::json!("FFFFFFFFFFFFFFFF"))
        .unwrap();
    notes_core::apply(
        &fixture.connection,
        &Op {
            op_id: uuid::Uuid::now_v7(),
            workspace_uuid: db::workspace_uuid(&fixture.connection).await.unwrap(),
            device_id,
            hlc: Hlc::new(
                chrono::Utc::now().timestamp_millis() as u64 + 1_000,
                0,
                device_id,
            ),
            format_version: notes_core::operation::FORMAT_VERSION,
            kind: OpKind::BlockMove(notes_core::BlockMove {
                uuid: block.uuid,
                page_uuid: page.uuid,
                parent_uuid: None,
                order_key: key.clone(),
            }),
        },
        Origin::Remote,
    )
    .await
    .unwrap();
    let appended = fixture
        .store
        .create_page_with_markdown("Exhausted".into(), "appended".into())
        .await
        .unwrap();
    let blocks = db::list_block_children(&fixture.connection, page.uuid, None)
        .await
        .unwrap();
    assert_eq!(
        blocks
            .iter()
            .map(|block| block.markdown.as_str())
            .collect::<Vec<_>>(),
        vec!["original", "appended"]
    );
    assert!(blocks[0].order_key < blocks[1].order_key);
    assert_eq!(appended.changes.blocks.len(), 2);
    fixture.store.undo().await.unwrap();
    let remaining = db::list_block_children(&fixture.connection, page.uuid, None)
        .await
        .unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].order_key, key);
}

#[tokio::test]
async fn invalid_capture_title_is_rejected_without_progress() {
    let fixture = fixture().await;
    assert!(
        fixture
            .store
            .create_page_with_markdown(" \n\t".into(), "text".into())
            .await
            .is_err()
    );
    assert_eq!(
        counters(&fixture.connection).await,
        (0, 0, 0, 0, 0, fixture.initial_version)
    );
    assert_eq!(last_hlc(&fixture.connection).await, None);
    assert!(fixture.notices.lock().unwrap().is_empty());
}
