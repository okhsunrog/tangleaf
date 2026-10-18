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
    count_tree_scans(&connection, true).await;
    assert_eq!(
        db::undo_history(&connection).await.unwrap(),
        db::HistoryMoveResult::Applied
    );
    assert_eq!(count_tree_scans(&connection, false).await, 1);
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
    count_tree_scans(&connection, true).await;
    assert_eq!(
        db::redo_history(&connection).await.unwrap(),
        db::HistoryMoveResult::Applied
    );
    assert_eq!(count_tree_scans(&connection, false).await, 1);
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
        count_tree_scans(&connection, true).await;
        assert_eq!(
            db::undo_history(&connection).await.unwrap(),
            db::HistoryMoveResult::Applied
        );
        assert_eq!(count_tree_scans(&connection, false).await, 1);
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
        count_tree_scans(&connection, true).await;
        assert_eq!(
            db::redo_history(&connection).await.unwrap(),
            db::HistoryMoveResult::Applied
        );
        assert_eq!(count_tree_scans(&connection, false).await, 1);
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

#[tokio::test]
async fn large_paragraph_split_and_history_each_reconcile_once() {
    let (_directory, connection) = database().await;
    let page = db::create_page(&connection, "Large split".into())
        .await
        .unwrap();
    let paragraphs = (0..243)
        .map(|index| format!("Paragraph {index}: a longer passage with **Markdown**, Unicode текст, and a link to [[Large split]]."))
        .collect::<Vec<_>>();
    let original = paragraphs.join("\n\n");
    let source = db::create_block(
        &connection,
        page.uuid,
        None,
        None,
        BlockStyle::Paragraph,
        original.clone(),
    )
    .await
    .unwrap();
    count_tree_scans(&connection, true).await;
    let blocks = db::split_block(
        &connection,
        source.uuid,
        paragraphs
            .iter()
            .cloned()
            .map(|markdown| BlockContent { markdown })
            .collect(),
        source.markdown_revision,
    )
    .await
    .unwrap();
    assert_eq!(count_tree_scans(&connection, false).await, 1);
    assert_eq!(
        blocks
            .iter()
            .map(|block| &block.markdown)
            .collect::<Vec<_>>(),
        paragraphs.iter().collect::<Vec<_>>()
    );
    assert_eq!(
        db::find_backlinks(&connection, page.uuid)
            .await
            .unwrap()
            .len(),
        paragraphs.len()
    );
    count_tree_scans(&connection, true).await;
    assert_eq!(
        db::undo_history(&connection).await.unwrap(),
        db::HistoryMoveResult::Applied
    );
    assert_eq!(count_tree_scans(&connection, false).await, 1);
    let restored = db::list_block_children(&connection, page.uuid, None)
        .await
        .unwrap();
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].markdown, original);
    assert_eq!(
        db::find_backlinks(&connection, page.uuid)
            .await
            .unwrap()
            .len(),
        1
    );
    count_tree_scans(&connection, true).await;
    assert_eq!(
        db::redo_history(&connection).await.unwrap(),
        db::HistoryMoveResult::Applied
    );
    assert_eq!(count_tree_scans(&connection, false).await, 1);
    let restored = db::list_block_children(&connection, page.uuid, None)
        .await
        .unwrap();
    assert_eq!(
        restored
            .iter()
            .map(|block| (&block.uuid, &block.markdown, &block.order_key))
            .collect::<Vec<_>>(),
        blocks
            .iter()
            .map(|block| (&block.uuid, &block.markdown, &block.order_key))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        db::find_backlinks(&connection, page.uuid)
            .await
            .unwrap()
            .len(),
        paragraphs.len()
    );
}

/// Manual timing on a real-sized workspace, without ever writing to the input.
/// Set TANGLEAF_BENCH_DATABASE and TANGLEAF_BENCH_BLOCK, then run this ignored test.
/// TANGLEAF_BENCH_PARTS may point to a JSON string array produced by the UI's
/// paragraph splitter to exercise the long-block Split action instead of Enter.
/// TANGLEAF_BENCH_ROUNDS defaults to 30 independent copies per synchronous policy.
/// TANGLEAF_BENCH_SYNCHRONOUS accepts NORMAL, FULL, or BOTH (default); BOTH alternates
/// policy order each round. The source should be a stable database snapshot.
/// Only disposable copies receive the PRAGMA override. Reports median and p95;
/// 30 samples are not sufficient to estimate a stable p99.
/// RUST_LOG enables opt-in phase diagnostics for a separate profiling run;
/// leave it unset for timing comparisons to avoid log-output overhead.
#[tokio::test]
#[ignore = "requires a database and source block; edits disposable copies only"]
async fn benchmark_split_on_database_copy() {
    if let Ok(filter) = std::env::var("RUST_LOG") {
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_new(filter).expect("valid RUST_LOG filter"),
            )
            .with_test_writer()
            .try_init()
            .expect("install benchmark tracing subscriber");
    }
    let source_path = std::env::var("TANGLEAF_BENCH_DATABASE").expect("database path");
    let uuid = std::env::var("TANGLEAF_BENCH_BLOCK")
        .expect("block UUID")
        .parse()
        .unwrap();
    let paragraphs: Option<Vec<String>> = std::env::var("TANGLEAF_BENCH_PARTS")
        .ok()
        .map(|path| serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap());
    let rounds = std::env::var("TANGLEAF_BENCH_ROUNDS")
        .map(|value| {
            value
                .parse::<usize>()
                .expect("positive number of benchmark rounds")
        })
        .unwrap_or(30);
    assert!(rounds > 0, "TANGLEAF_BENCH_ROUNDS must be positive");
    let policies = match std::env::var("TANGLEAF_BENCH_SYNCHRONOUS")
        .unwrap_or_else(|_| "BOTH".into())
        .trim()
        .to_ascii_uppercase()
        .as_str()
    {
        "NORMAL" => vec![("NORMAL", 1i64)],
        "FULL" => vec![("FULL", 2)],
        "BOTH" => vec![("NORMAL", 1), ("FULL", 2)],
        _ => panic!("TANGLEAF_BENCH_SYNCHRONOUS must be NORMAL, FULL, or BOTH"),
    };
    let source = rusqlite::Connection::open_with_flags(
        source_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let mut timings = Vec::with_capacity(rounds * policies.len());
    for round in 1..=rounds {
        let mut run_policies = policies.clone();
        if round % 2 == 0 {
            run_policies.reverse();
        }
        for (mode, expected_synchronous) in run_policies {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("benchmark.db");
            source
                .execute("VACUUM INTO ?1", [path.to_str().unwrap()])
                .unwrap();
            let connection = db::open(path).await.unwrap();
            let (synchronous, journal_mode) = connection
                .call(move |database| {
                    database.pragma_update(None, "synchronous", expected_synchronous)?;
                    let synchronous = database
                        .pragma_query_value(None, "synchronous", |row| row.get::<_, i64>(0))?;
                    let journal_mode =
                        database.pragma_query_value(None, "journal_mode", |row| {
                            row.get::<_, String>(0)
                        })?;
                    Ok((synchronous, journal_mode))
                })
                .await
                .unwrap();
            assert_eq!(synchronous, expected_synchronous);
            assert!(journal_mode.eq_ignore_ascii_case("wal"));
            let block = db::get_block(&connection, uuid).await.unwrap().unwrap();
            let original = block.markdown.clone();
            let parts = paragraphs
                .clone()
                .unwrap_or_else(|| vec![original.clone(), String::new(), String::new()]);
            let part_count = parts.len();
            let contents = parts
                .iter()
                .cloned()
                .map(|markdown| BlockContent { markdown })
                .collect();
            // Keep the worker's normal PROFILE tracing intact during measurements.
            // Deterministic tests above independently assert structural scan counts.
            let start = std::time::Instant::now();
            let split = db::split_block_with_ops(
                &connection,
                uuid,
                contents,
                block.markdown_revision.clone(),
            )
            .await
            .unwrap();
            let split_ms = start.elapsed().as_secs_f64() * 1_000.0;
            assert_eq!(
                split
                    .value
                    .0
                    .iter()
                    .map(|block| &block.markdown)
                    .collect::<Vec<_>>(),
                parts.iter().collect::<Vec<_>>()
            );
            assert!(
                split
                    .value
                    .0
                    .windows(2)
                    .all(|pair| pair[0].order_key < pair[1].order_key)
            );
            let start = std::time::Instant::now();
            let undo = db::undo_history(&connection).await.unwrap();
            let undo_ms = start.elapsed().as_secs_f64() * 1_000.0;
            assert_eq!(undo, db::HistoryMoveResult::Applied);
            let restored = db::get_block(&connection, uuid).await.unwrap().unwrap();
            assert_eq!(
                (
                    &restored.markdown,
                    &restored.order_key,
                    restored.page_uuid,
                    restored.parent_uuid
                ),
                (
                    &original,
                    &block.order_key,
                    block.page_uuid,
                    block.parent_uuid
                ),
            );
            for added in split.value.0.iter().skip(1) {
                assert!(
                    db::get_block(&connection, added.uuid)
                        .await
                        .unwrap()
                        .is_none()
                );
            }
            let start = std::time::Instant::now();
            let redo = db::redo_history(&connection).await.unwrap();
            let redo_ms = start.elapsed().as_secs_f64() * 1_000.0;
            assert_eq!(redo, db::HistoryMoveResult::Applied);
            for expected in &split.value.0 {
                let restored = db::get_block(&connection, expected.uuid)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    (
                        &restored.markdown,
                        &restored.order_key,
                        restored.page_uuid,
                        restored.parent_uuid
                    ),
                    (
                        &expected.markdown,
                        &expected.order_key,
                        expected.page_uuid,
                        expected.parent_uuid
                    ),
                );
            }
            println!(
                "mode={mode} synchronous={synchronous} journal_mode={journal_mode} round={round} parts={part_count} source_bytes={} operations={} split_ms={split_ms:.2} undo_ms={undo_ms:.2} redo_ms={redo_ms:.2}",
                original.len(),
                split.operations.len(),
            );
            timings.push((mode, [split_ms, undo_ms, redo_ms]));
        }
    }
    for (mode, _) in policies {
        for (index, action) in ["split", "undo", "redo"].into_iter().enumerate() {
            let mut samples = timings
                .iter()
                .filter(|(sample_mode, _)| *sample_mode == mode)
                .map(|(_, values)| values[index])
                .collect::<Vec<_>>();
            samples.sort_by(f64::total_cmp);
            let count = samples.len();
            let median = (samples[(count - 1) / 2] + samples[count / 2]) / 2.0;
            // Nearest-rank percentile; retain individual samples for later analysis.
            let p95 = samples[(count * 95).div_ceil(100) - 1];
            println!(
                "summary mode={mode} action={action} samples={count} median_ms={median:.2} p95_ms={p95:.2}",
            );
        }
    }
}
