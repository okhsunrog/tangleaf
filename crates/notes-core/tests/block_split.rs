use notes_core::db::{self, BlockContent};
use notes_core::{BlockCreate, BlockStyle, Connection, OpKind, OrderKey};
use std::cell::Cell;

thread_local! {
    static TREE_SCANS: Cell<usize> = const { Cell::new(0) };
}

async fn count_tree_scans(connection: &Connection, start: bool) -> usize {
    connection
        .call(move |database| {
            if start {
                TREE_SCANS.set(0);
                database.trace_v2(
                    rusqlite::trace::TraceEventCodes::SQLITE_TRACE_STMT,
                    Some(|event| {
                        let rusqlite::trace::TraceEvent::Stmt(statement, _) = event else {
                            return;
                        };
                        // Count the workspace-wide structural reads, independently
                        // of machine speed and the size of this test's fixture.
                        if statement
                            .sql()
                            .starts_with("SELECT uuid, existence_hlc, page_uuid")
                        {
                            TREE_SCANS.set(TREE_SCANS.get() + 1);
                        }
                    }),
                );
            } else {
                database.trace_v2(rusqlite::trace::TraceEventCodes::empty(), None);
            }
            Ok(TREE_SCANS.get())
        })
        .await
        .unwrap()
}

async fn database() -> (tempfile::TempDir, Connection) {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(directory.path().join("notes.db")).await.unwrap();
    (directory, connection)
}

async fn replay_and_compare(source: &Connection, replica: &Connection, page: uuid::Uuid) {
    notes_core::apply_batch(
        replica,
        &notes_core::pending_outbox(source, 1_000).await.unwrap(),
        notes_core::Origin::Remote,
    )
    .await
    .unwrap();
    assert_eq!(
        db::get_page_document(source, page).await.unwrap(),
        db::get_page_document(replica, page).await.unwrap()
    );
    let left = notes_core::export_sync_snapshot(source, 0).await.unwrap();
    let right = notes_core::export_sync_snapshot(replica, 0).await.unwrap();
    assert_eq!(left, right);
    assert_eq!(
        serde_json::to_value(db::graph_snapshot(source, None).await.unwrap()).unwrap(),
        serde_json::to_value(db::graph_snapshot(replica, None).await.unwrap()).unwrap()
    );
}

#[tokio::test]
async fn split_preserves_neighbors_and_children_and_replays_with_history() {
    let (_directory, connection) = database().await;
    let page = db::create_page(&connection, "Split".into()).await.unwrap();
    let parent = db::create_block(
        &connection,
        page.uuid,
        None,
        None,
        BlockStyle::Bullet,
        "Parent".into(),
    )
    .await
    .unwrap();
    let before = db::create_block(
        &connection,
        page.uuid,
        Some(parent.uuid),
        None,
        BlockStyle::Bullet,
        "Before".into(),
    )
    .await
    .unwrap();
    let source = db::create_block(
        &connection,
        page.uuid,
        Some(parent.uuid),
        None,
        BlockStyle::Quote,
        "Before split".into(),
    )
    .await
    .unwrap();
    let after = db::create_block(
        &connection,
        page.uuid,
        Some(parent.uuid),
        None,
        BlockStyle::Bullet,
        "After".into(),
    )
    .await
    .unwrap();
    let child = db::create_block(
        &connection,
        page.uuid,
        Some(source.uuid),
        None,
        BlockStyle::Bullet,
        "Child".into(),
    )
    .await
    .unwrap();
    let (_replica_directory, replica) = database().await;
    notes_core::import_sync_snapshot(
        &replica,
        notes_core::export_sync_snapshot(&connection, 0)
            .await
            .unwrap(),
    )
    .await
    .unwrap();
    notes_core::configure_sync(&connection, "https://sync.example")
        .await
        .unwrap();
    let history_before = db::history_status(&connection).await.unwrap().undo_count;

    count_tree_scans(&connection, true).await;
    let split = db::split_block_with_ops(
        &connection,
        source.uuid,
        [
            "First [[Split]]".into(),
            format!("Second (({}))", child.uuid),
            "Third".into(),
        ]
        .map(|markdown| BlockContent { markdown })
        .to_vec(),
        source.markdown_revision.clone(),
    )
    .await
    .unwrap();
    assert_eq!(count_tree_scans(&connection, false).await, 1);
    assert_eq!(split.operations.len(), 3);
    let result = split.value.0;
    assert_eq!(result[0].uuid, source.uuid);
    assert_eq!(result[0].order_key, source.order_key);
    assert!(
        result
            .iter()
            .all(|block| block.parent_uuid == Some(parent.uuid) && block.style == source.style)
    );
    for unchanged in [&before, &after, &child] {
        assert_eq!(
            db::get_block(&connection, unchanged.uuid)
                .await
                .unwrap()
                .as_ref(),
            Some(unchanged)
        );
    }
    assert_eq!(
        db::list_block_children(&connection, page.uuid, Some(parent.uuid))
            .await
            .unwrap()
            .into_iter()
            .map(|block| block.uuid)
            .collect::<Vec<_>>(),
        vec![
            before.uuid,
            source.uuid,
            result[1].uuid,
            result[2].uuid,
            after.uuid
        ]
    );
    assert_eq!(
        db::history_status(&connection).await.unwrap().undo_count,
        history_before + 1
    );
    assert_eq!(
        db::find_backlinks(&connection, page.uuid)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        db::find_backlinks(&connection, child.uuid)
            .await
            .unwrap()
            .len(),
        1
    );
    replay_and_compare(&connection, &replica, page.uuid).await;
    assert_eq!(
        db::undo_history(&connection).await.unwrap(),
        db::HistoryMoveResult::Applied
    );
    assert_eq!(
        db::get_block(&connection, source.uuid)
            .await
            .unwrap()
            .unwrap()
            .markdown,
        source.markdown
    );
    assert!(
        db::get_block(&connection, result[1].uuid)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        db::get_block(&connection, child.uuid).await.unwrap(),
        Some(child)
    );
    assert!(
        db::find_backlinks(&connection, page.uuid)
            .await
            .unwrap()
            .is_empty()
    );
    replay_and_compare(&connection, &replica, page.uuid).await;
    assert_eq!(
        db::redo_history(&connection).await.unwrap(),
        db::HistoryMoveResult::Applied
    );
    replay_and_compare(&connection, &replica, page.uuid).await;
}

#[tokio::test]
async fn split_rebalances_only_when_order_key_space_is_exhausted() {
    for keys in [
        vec![5, 6],
        vec![9, 9],
        vec![u64::MAX - 1, u64::MAX],
        vec![u64::MAX],
    ] {
        let (_directory, connection) = database().await;
        let page = db::create_page(&connection, "Tight keys".into())
            .await
            .unwrap();
        let kinds = keys.into_iter().map(|key| {
            OpKind::BlockCreate(BlockCreate {
                uuid: uuid::Uuid::now_v7(),
                page_uuid: page.uuid,
                parent_uuid: None,
                order_key: format!("{key:016X}").parse::<OrderKey>().unwrap(),
                style: BlockStyle::Bullet,
                markdown: "Original".into(),
                created_at: 1,
            })
        });
        let workspace_uuid = db::workspace_uuid(&connection).await.unwrap();
        let device_id = uuid::Uuid::now_v7();
        let wall_ms = chrono::Utc::now().timestamp_millis() as u64 + 1_000;
        let operations = kinds
            .enumerate()
            .map(|(index, kind)| notes_core::Op {
                op_id: uuid::Uuid::now_v7(),
                workspace_uuid,
                device_id,
                hlc: notes_core::Hlc::new(wall_ms, index as u32, device_id),
                format_version: notes_core::operation::FORMAT_VERSION,
                kind,
            })
            .collect::<Vec<_>>();
        notes_core::apply_batch(&connection, &operations, notes_core::Origin::Remote)
            .await
            .unwrap();
        let before = db::list_block_children(&connection, page.uuid, None)
            .await
            .unwrap();
        let source = &before[0];
        count_tree_scans(&connection, true).await;
        let split = db::split_block(
            &connection,
            source.uuid,
            ["One", "Two", "Three"]
                .map(|markdown| BlockContent {
                    markdown: markdown.into(),
                })
                .to_vec(),
            source.markdown_revision.clone(),
        )
        .await
        .unwrap();
        assert_eq!(count_tree_scans(&connection, false).await, 1);
        let after = db::list_block_children(&connection, page.uuid, None)
            .await
            .unwrap();
        let expected = split
            .iter()
            .chain(before.iter().skip(1))
            .map(|block| block.uuid)
            .collect::<Vec<_>>();
        assert_eq!(
            after.iter().map(|block| block.uuid).collect::<Vec<_>>(),
            expected
        );
        assert!(
            after
                .windows(2)
                .all(|pair| pair[0].order_key < pair[1].order_key)
        );
        assert_eq!(
            db::undo_history(&connection).await.unwrap(),
            db::HistoryMoveResult::Applied
        );
        let restored = db::list_block_children(&connection, page.uuid, None)
            .await
            .unwrap();
        assert_eq!(
            restored
                .iter()
                .map(|block| (&block.uuid, &block.order_key))
                .collect::<Vec<_>>(),
            before
                .iter()
                .map(|block| (&block.uuid, &block.order_key))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            db::redo_history(&connection).await.unwrap(),
            db::HistoryMoveResult::Applied
        );
        assert_eq!(
            db::list_block_children(&connection, page.uuid, None)
                .await
                .unwrap()
                .iter()
                .map(|block| block.uuid)
                .collect::<Vec<_>>(),
            expected
        );
    }
}

#[tokio::test]
async fn single_part_split_does_not_reorder_or_scan_the_tree() {
    let (_directory, connection) = database().await;
    let note = db::create_note(&connection, None)
        .await
        .unwrap()
        .into_created()
        .unwrap();
    count_tree_scans(&connection, true).await;
    let split = db::split_block_with_ops(
        &connection,
        note.initial_block.uuid,
        vec![BlockContent {
            markdown: "Changed".into(),
        }],
        note.initial_block.markdown_revision,
    )
    .await
    .unwrap();
    assert_eq!(count_tree_scans(&connection, false).await, 0);
    assert_eq!(split.operations.len(), 1);
    assert_eq!(split.value.0[0].order_key, note.initial_block.order_key);
}

#[tokio::test]
async fn split_at_end_allocates_new_keys_without_moving_existing_blocks() {
    let (_directory, connection) = database().await;
    let note = db::create_note(&connection, None)
        .await
        .unwrap()
        .into_created()
        .unwrap();
    let first = note.initial_block;
    let source = db::create_block(
        &connection,
        note.page.uuid,
        None,
        None,
        BlockStyle::Bullet,
        "Last".into(),
    )
    .await
    .unwrap();
    let split = db::split_block_with_ops(
        &connection,
        source.uuid,
        ["One", "Two", "Three"]
            .map(|markdown| BlockContent {
                markdown: markdown.into(),
            })
            .to_vec(),
        source.markdown_revision,
    )
    .await
    .unwrap();
    assert_eq!(split.operations.len(), 3);
    assert_eq!(
        db::get_block(&connection, first.uuid).await.unwrap(),
        Some(first)
    );
    let result = split.value.0;
    assert_eq!(result[0].order_key, source.order_key);
    assert!(
        result
            .windows(2)
            .all(|pair| pair[0].order_key < pair[1].order_key)
    );
}

/// Manual timing on a real-sized workspace, without ever writing to the input.
/// Set TANGLEAF_BENCH_DATABASE and TANGLEAF_BENCH_BLOCK, then run this ignored test.
#[tokio::test]
#[ignore = "requires a database and source block; edits disposable copies only"]
async fn benchmark_split_on_database_copy() {
    let source_path = std::env::var("TANGLEAF_BENCH_DATABASE").expect("database path");
    let uuid = std::env::var("TANGLEAF_BENCH_BLOCK")
        .expect("block UUID")
        .parse()
        .unwrap();
    let source = rusqlite::Connection::open_with_flags(
        source_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    for round in 1..=3 {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("benchmark.db");
        source
            .execute("VACUUM INTO ?1", [path.to_str().unwrap()])
            .unwrap();
        let connection = db::open(path).await.unwrap();
        let block = db::get_block(&connection, uuid).await.unwrap().unwrap();
        count_tree_scans(&connection, true).await;
        let start = std::time::Instant::now();
        let split = db::split_block_with_ops(
            &connection,
            uuid,
            vec![
                BlockContent {
                    markdown: block.markdown,
                },
                BlockContent {
                    markdown: String::new(),
                },
                BlockContent {
                    markdown: String::new(),
                },
            ],
            block.markdown_revision,
        )
        .await
        .unwrap();
        let elapsed = start.elapsed();
        let scans = count_tree_scans(&connection, false).await;
        println!(
            "round={round} split_ms={:.2} operations={} tree_scans={scans}",
            elapsed.as_secs_f64() * 1_000.0,
            split.operations.len()
        );
    }
}
