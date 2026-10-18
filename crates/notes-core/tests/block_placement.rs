use notes_core::db::{self, AppliedMutation, Block};
use notes_core::store::NotesStore;
use notes_core::{
    BlockCreate, BlockMove, BlockStyle, Connection, Hlc, Op, OpKind, OrderKey, Origin,
    ReorderDirection,
};
use std::cell::RefCell;
use uuid::Uuid;

fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}
fn key(value: u64) -> OrderKey {
    format!("{value:016X}").parse().unwrap()
}

struct Fixture {
    _directory: tempfile::TempDir,
    connection: Connection,
    page: Uuid,
}

async fn database() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(directory.path().join("notes.db")).await.unwrap();
    let page = db::create_page(&connection, "Placement".into())
        .await
        .unwrap()
        .uuid;
    Fixture {
        _directory: directory,
        connection,
        page,
    }
}

fn block(
    uuid: Uuid,
    page_uuid: Uuid,
    parent_uuid: Option<Uuid>,
    order_key: OrderKey,
) -> BlockCreate {
    BlockCreate {
        uuid,
        page_uuid,
        parent_uuid,
        order_key,
        style: BlockStyle::Bullet,
        markdown: format!("Block {uuid}"),
        created_at: 1,
    }
}

async fn remote_kinds(connection: &Connection, kinds: Vec<OpKind>) {
    let workspace_uuid = db::workspace_uuid(connection).await.unwrap();
    let device_id = Uuid::now_v7();
    let previous = connection
        .call(|database| {
            database.query_row(
                "SELECT value FROM sync_meta WHERE key = 'last_hlc'",
                [],
                |row| row.get::<_, String>(0),
            )
        })
        .await
        .unwrap()
        .parse::<Hlc>()
        .unwrap();
    let wall_ms = previous.wall_ms() + 1_000;
    let operations = kinds
        .into_iter()
        .enumerate()
        .map(|(index, kind)| Op {
            op_id: Uuid::now_v7(),
            workspace_uuid,
            device_id,
            hlc: Hlc::new(wall_ms, index as u32, device_id),
            format_version: notes_core::operation::FORMAT_VERSION,
            kind,
        })
        .collect::<Vec<_>>();
    notes_core::apply_batch(connection, &operations, Origin::Remote)
        .await
        .unwrap();
}

async fn configure(connection: &Connection) {
    notes_core::configure_sync(connection, "https://placement.example")
        .await
        .unwrap();
    connection
        .call(|database| {
            database.execute("DELETE FROM history_undo", [])?;
            database.execute("DELETE FROM history_redo", [])?;
            Ok(())
        })
        .await
        .unwrap();
}

async fn roots(connection: &Connection, page: Uuid) -> Vec<Block> {
    db::list_block_children(connection, page, None)
        .await
        .unwrap()
}

fn uuids(blocks: &[Block]) -> Vec<Uuid> {
    blocks.iter().map(|block| block.uuid).collect()
}

#[derive(Debug, Default, PartialEq, Eq)]
struct SqlWork {
    rows: usize,
    selects: usize,
    placement_queries: Vec<String>,
    placement_expanded: Vec<String>,
}
thread_local! { static WORK: RefCell<Option<SqlWork>> = const { RefCell::new(None) }; }

fn trace_work(event: rusqlite::trace::TraceEvent<'_>) {
    match event {
        rusqlite::trace::TraceEvent::Stmt(statement, _) => {
            let sql = statement.sql();
            if !sql.starts_with("SELECT") && !sql.starts_with("WITH") {
                return;
            }
            WORK.with(|work| {
                let mut work = work.borrow_mut();
                let Some(work) = work.as_mut() else {
                    return;
                };
                work.selects += 1;
                if sql.starts_with("SELECT uuid, order_key FROM blocks") {
                    work.placement_queries.push(sql.to_string());
                    work.placement_expanded
                        .push(statement.expanded_sql().unwrap());
                }
            });
        }
        rusqlite::trace::TraceEvent::Row(statement) => {
            let sql = statement.sql();
            if sql.starts_with("SELECT") || sql.starts_with("WITH") {
                WORK.with(|work| {
                    if let Some(work) = work.borrow_mut().as_mut() {
                        work.rows += 1;
                    }
                });
            }
        }
        _ => {}
    }
}

async fn begin_work(connection: &Connection) {
    connection
        .call(|database| {
            WORK.with(|work| *work.borrow_mut() = Some(SqlWork::default()));
            database.trace_v2(
                rusqlite::trace::TraceEventCodes::SQLITE_TRACE_STMT
                    | rusqlite::trace::TraceEventCodes::SQLITE_TRACE_ROW,
                Some(trace_work),
            );
            Ok(())
        })
        .await
        .unwrap();
}

async fn end_work(connection: &Connection) -> SqlWork {
    connection
        .call(|database| {
            database.trace_v2(rusqlite::trace::TraceEventCodes::empty(), None);
            let work = WORK.with(|work| work.borrow_mut().take().unwrap());
            for sql in &work.placement_expanded {
                // Explain the exact SQL and bound values captured from the
                // production command, including its excluded moving block.
                let plan = database
                    .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?
                    .query_map([], |row| row.get::<_, String>(3))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                assert!(
                    plan.iter()
                        .all(|step| !step.contains("SCAN blocks") && !step.contains("TEMP B-TREE")),
                    "{sql}: {plan:?}"
                );
                let expected = if sql.contains("WHERE uuid =") {
                    "sqlite_autoindex_blocks_1"
                } else {
                    "COVERING INDEX idx_blocks_page_parent_order"
                };
                assert!(
                    plan.iter()
                        .any(|step| step.contains("SEARCH blocks") && step.contains(expected)),
                    "{sql}: {plan:?}"
                );
            }
            Ok(work)
        })
        .await
        .unwrap()
}

async fn assert_one_operation(
    connection: &Connection,
    applied: AppliedMutation<Block>,
) -> (Block, SqlWork) {
    let work = end_work(connection).await;
    assert_eq!(applied.operations.len(), 1);
    assert!(work.rows > 0);
    assert!(!work.placement_queries.is_empty());
    for query in &work.placement_queries {
        assert!(
            query.contains("LIMIT 1") || query.contains("WHERE uuid = ?1"),
            "unbounded placement query: {query}"
        );
    }
    (applied.value, work)
}

#[tokio::test]
async fn normal_commands_emit_one_operation_and_read_fixed_sql_rows_with_many_root_siblings() {
    let mut measured = Vec::new();
    for unrelated in [1_000, 10_000] {
        let f = database().await;
        let mut blocks = (1..=3)
            .map(|value| {
                OpKind::BlockCreate(block(
                    id(value),
                    f.page,
                    None,
                    OrderKey::from_ordinal(value as usize),
                ))
            })
            .collect::<Vec<_>>();
        blocks.extend((0..unrelated).map(|index| {
            OpKind::BlockCreate(block(
                id(1_000 + index as u128),
                f.page,
                None,
                OrderKey::from_ordinal(index + 10),
            ))
        }));
        remote_kinds(&f.connection, blocks).await;
        configure(&f.connection).await;
        let untouched = db::get_block(&f.connection, id(1_000))
            .await
            .unwrap()
            .unwrap();
        let mut work = Vec::new();
        begin_work(&f.connection).await;
        let applied = db::create_block_with_ops(
            &f.connection,
            f.page,
            None,
            Some(id(1)),
            BlockStyle::Bullet,
            "Middle".into(),
        )
        .await
        .unwrap();
        let (middle, stats) = assert_one_operation(&f.connection, applied).await;
        work.push(stats);
        begin_work(&f.connection).await;
        let applied = db::create_block_with_ops(
            &f.connection,
            f.page,
            None,
            None,
            BlockStyle::Bullet,
            "Append".into(),
        )
        .await
        .unwrap();
        work.push(assert_one_operation(&f.connection, applied).await.1);
        begin_work(&f.connection).await;
        let applied = db::move_block_with_ops(&f.connection, middle.uuid, None, None)
            .await
            .unwrap();
        work.push(assert_one_operation(&f.connection, applied).await.1);
        begin_work(&f.connection).await;
        let applied = db::reorder_block_with_ops(&f.connection, id(3), ReorderDirection::Up)
            .await
            .unwrap();
        work.push(assert_one_operation(&f.connection, applied).await.1);
        begin_work(&f.connection).await;
        let applied = db::reorder_block_with_ops(&f.connection, id(3), ReorderDirection::Down)
            .await
            .unwrap();
        work.push(assert_one_operation(&f.connection, applied).await.1);
        begin_work(&f.connection).await;
        let applied = db::indent_block_with_ops(&f.connection, id(3))
            .await
            .unwrap();
        let (indented, stats) = assert_one_operation(&f.connection, applied).await;
        assert_eq!(indented.parent_uuid, Some(id(2)));
        work.push(stats);
        begin_work(&f.connection).await;
        let applied = db::outdent_block_with_ops(&f.connection, id(3))
            .await
            .unwrap();
        work.push(assert_one_operation(&f.connection, applied).await.1);
        assert_eq!(
            db::get_block(&f.connection, untouched.uuid)
                .await
                .unwrap()
                .unwrap(),
            untouched
        );
        assert_eq!(
            notes_core::pending_outbox(&f.connection, 100)
                .await
                .unwrap()
                .len(),
            7
        );
        measured.push(
            work.into_iter()
                .map(|stats| (stats.rows, stats.selects, stats.placement_queries.len()))
                .collect::<Vec<_>>(),
        );
    }
    assert_eq!(
        measured[0], measured[1],
        "actual SQL work must stay bounded: {measured:?}"
    );
    eprintln!(
        "placement (returned SQL rows, SELECTs, anchor queries): {:?}",
        measured[0]
    );
}

#[tokio::test]
async fn insertion_fallback_rebalances_only_changed_keys_in_the_destination_group() {
    for tight in [OrderKey::STEP, OrderKey::STEP + 1] {
        let f = database().await;
        let foreign = db::create_page(&f.connection, "Foreign".into())
            .await
            .unwrap();
        remote_kinds(
            &f.connection,
            vec![
                OpKind::BlockCreate(block(id(1), f.page, None, key(OrderKey::STEP))),
                OpKind::BlockCreate(block(id(2), f.page, None, key(tight))),
                OpKind::BlockCreate(block(id(3), f.page, None, OrderKey::from_ordinal(4))),
                OpKind::BlockCreate(block(id(4), f.page, Some(id(1)), key(7))),
                OpKind::BlockCreate(block(id(5), foreign.uuid, None, key(9))),
            ],
        )
        .await;
        configure(&f.connection).await;
        let before = db::get_blocks(&f.connection, (1..=5).map(id).collect())
            .await
            .unwrap();
        let inserted = db::create_block_with_ops(
            &f.connection,
            f.page,
            None,
            Some(id(1)),
            BlockStyle::Bullet,
            "Inserted".into(),
        )
        .await
        .unwrap();
        assert_eq!(inserted.operations.len(), 2);
        assert!(matches!(&inserted.operations[0], OpKind::BlockCreate(_)));
        assert!(matches!(&inserted.operations[1], OpKind::BlockMove(moved) if moved.uuid == id(2)));
        let after = roots(&f.connection, f.page).await;
        assert_eq!(uuids(&after), [id(1), inserted.value.uuid, id(2), id(3)]);
        assert!(
            after
                .windows(2)
                .all(|pair| pair[0].order_key < pair[1].order_key)
        );
        for original in before.iter().filter(|block| block.uuid != id(2)) {
            assert_eq!(
                db::get_block(&f.connection, original.uuid)
                    .await
                    .unwrap()
                    .as_ref(),
                Some(original)
            );
        }
        assert_eq!(
            db::undo_history(&f.connection).await.unwrap(),
            db::HistoryMoveResult::Applied
        );
        let restored = roots(&f.connection, f.page).await;
        assert_eq!(
            restored
                .iter()
                .map(|block| (&block.uuid, &block.order_key))
                .collect::<Vec<_>>(),
            before[..3]
                .iter()
                .map(|block| (&block.uuid, &block.order_key))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            db::redo_history(&f.connection).await.unwrap(),
            db::HistoryMoveResult::Applied
        );
        assert_eq!(
            roots(&f.connection, f.page)
                .await
                .iter()
                .map(|block| (&block.uuid, &block.order_key))
                .collect::<Vec<_>>(),
            after
                .iter()
                .map(|block| (&block.uuid, &block.order_key))
                .collect::<Vec<_>>()
        );
    }
}

#[tokio::test]
async fn exhausted_append_and_tied_front_reorder_use_a_changed_key_only_fallback() {
    let f = database().await;
    remote_kinds(
        &f.connection,
        vec![
            OpKind::BlockCreate(block(id(1), f.page, None, OrderKey::first())),
            OpKind::BlockCreate(block(id(2), f.page, None, key(u64::MAX))),
        ],
    )
    .await;
    configure(&f.connection).await;
    let first = db::get_block(&f.connection, id(1)).await.unwrap().unwrap();
    let appended = db::create_block_with_ops(
        &f.connection,
        f.page,
        None,
        None,
        BlockStyle::Bullet,
        "Append".into(),
    )
    .await
    .unwrap();
    assert_eq!(appended.operations.len(), 2);
    assert!(matches!(&appended.operations[1], OpKind::BlockMove(moved) if moved.uuid == id(2)));
    assert_eq!(
        db::get_block(&f.connection, first.uuid).await.unwrap(),
        Some(first)
    );
    assert_eq!(
        uuids(&roots(&f.connection, f.page).await),
        [id(1), id(2), appended.value.uuid]
    );

    let f = database().await;
    remote_kinds(
        &f.connection,
        vec![
            OpKind::BlockCreate(block(id(1), f.page, None, key(0))),
            OpKind::BlockCreate(block(id(2), f.page, None, key(0))),
            OpKind::BlockCreate(block(id(3), f.page, None, OrderKey::from_ordinal(3))),
        ],
    )
    .await;
    configure(&f.connection).await;
    let third = db::get_block(&f.connection, id(3)).await.unwrap().unwrap();
    let reordered = db::reorder_block_with_ops(&f.connection, id(2), ReorderDirection::Up)
        .await
        .unwrap();
    assert_eq!(reordered.operations.len(), 2);
    assert_eq!(
        uuids(&roots(&f.connection, f.page).await),
        [id(2), id(1), id(3)]
    );
    assert_eq!(
        db::get_block(&f.connection, third.uuid).await.unwrap(),
        Some(third)
    );
    assert_eq!(
        db::undo_history(&f.connection).await.unwrap(),
        db::HistoryMoveResult::Applied
    );
    let restored = roots(&f.connection, f.page).await;
    assert_eq!(uuids(&restored), [id(1), id(2), id(3)]);
    assert_eq!(restored[0].order_key, key(0));
    assert_eq!(restored[1].order_key, key(0));
    assert_eq!(
        db::redo_history(&f.connection).await.unwrap(),
        db::HistoryMoveResult::Applied
    );
    assert_eq!(
        uuids(&roots(&f.connection, f.page).await),
        [id(2), id(1), id(3)]
    );
}

#[tokio::test]
async fn move_indent_and_outdent_rebalance_only_the_exhausted_destination_group_and_replay() {
    for command in ["move", "indent", "outdent"] {
        let f = database().await;
        let outdent = command == "outdent";
        let mut kinds = vec![
            OpKind::BlockCreate(block(
                id(1),
                f.page,
                None,
                if outdent {
                    key(u64::MAX)
                } else {
                    OrderKey::first()
                },
            )),
            OpKind::BlockCreate(block(
                id(2),
                f.page,
                outdent.then_some(id(1)),
                if outdent {
                    OrderKey::first()
                } else {
                    OrderKey::from_ordinal(2)
                },
            )),
            OpKind::BlockCreate(block(
                id(3),
                f.page,
                None,
                if outdent {
                    OrderKey::first()
                } else {
                    OrderKey::from_ordinal(3)
                },
            )),
            OpKind::BlockCreate(block(id(20), f.page, Some(id(2)), OrderKey::first())),
        ];
        if !outdent {
            kinds.extend([
                OpKind::BlockCreate(block(id(10), f.page, Some(id(1)), OrderKey::first())),
                OpKind::BlockCreate(block(id(11), f.page, Some(id(1)), key(u64::MAX))),
            ]);
        }
        remote_kinds(&f.connection, kinds).await;
        configure(&f.connection).await;
        let (_replica_directory, replica) = replica(&f.connection).await;
        let before = db::get_page_document(&f.connection, f.page)
            .await
            .unwrap()
            .unwrap();
        let unchanged = if outdent {
            vec![id(3), id(20)]
        } else {
            vec![id(1), id(3), id(10), id(20)]
        };
        let unchanged = db::get_blocks(&f.connection, unchanged).await.unwrap();
        let applied = match command {
            "move" => db::move_block_with_ops(&f.connection, id(2), Some(id(1)), None)
                .await
                .unwrap(),
            "indent" => db::indent_block_with_ops(&f.connection, id(2))
                .await
                .unwrap(),
            "outdent" => db::outdent_block_with_ops(&f.connection, id(2))
                .await
                .unwrap(),
            _ => unreachable!(),
        };
        assert_eq!(applied.operations.len(), 2, "{command}");
        let mut moved = applied
            .operations
            .iter()
            .map(|operation| match operation {
                OpKind::BlockMove(moved) => moved.uuid,
                _ => panic!("unexpected {command} operation: {operation:?}"),
            })
            .collect::<Vec<_>>();
        moved.sort_unstable();
        assert_eq!(
            moved,
            if outdent {
                vec![id(1), id(2)]
            } else {
                vec![id(2), id(11)]
            }
        );
        assert_eq!(applied.value.parent_uuid, (!outdent).then_some(id(1)));
        for original in unchanged {
            assert_eq!(
                db::get_block(&f.connection, original.uuid).await.unwrap(),
                Some(original),
                "{command}"
            );
        }
        let after = db::get_page_document(&f.connection, f.page)
            .await
            .unwrap()
            .unwrap();
        replay(&f.connection, &replica, &[f.page]).await;
        assert_eq!(
            db::undo_history(&f.connection).await.unwrap(),
            db::HistoryMoveResult::Applied,
            "{command}"
        );
        assert_eq!(
            semantic(
                &db::get_page_document(&f.connection, f.page)
                    .await
                    .unwrap()
                    .unwrap()
                    .blocks
            ),
            semantic(&before.blocks),
            "{command}"
        );
        replay(&f.connection, &replica, &[f.page]).await;
        assert_eq!(
            db::redo_history(&f.connection).await.unwrap(),
            db::HistoryMoveResult::Applied,
            "{command}"
        );
        assert_eq!(
            semantic(
                &db::get_page_document(&f.connection, f.page)
                    .await
                    .unwrap()
                    .unwrap()
                    .blocks
            ),
            semantic(&after.blocks),
            "{command}"
        );
        replay(&f.connection, &replica, &[f.page]).await;
    }
}

async fn replay(source: &Connection, replica: &Connection, pages: &[Uuid]) {
    notes_core::apply_batch(
        replica,
        &notes_core::pending_outbox(source, 1_000).await.unwrap(),
        Origin::Remote,
    )
    .await
    .unwrap();
    for page in pages {
        assert_eq!(
            db::get_page_document(source, *page).await.unwrap(),
            db::get_page_document(replica, *page).await.unwrap()
        );
    }
    assert_eq!(
        notes_core::export_sync_snapshot(source, 0).await.unwrap(),
        notes_core::export_sync_snapshot(replica, 0).await.unwrap()
    );
}

async fn replica(source: &Connection) -> (tempfile::TempDir, Connection) {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(directory.path().join("replica.db")).await.unwrap();
    notes_core::import_sync_snapshot(
        &connection,
        notes_core::export_sync_snapshot(source, 0).await.unwrap(),
    )
    .await
    .unwrap();
    (directory, connection)
}

fn semantic(blocks: &[Block]) -> Vec<(Uuid, Option<Uuid>, OrderKey, BlockStyle, String)> {
    blocks
        .iter()
        .map(|block| {
            (
                block.uuid,
                block.parent_uuid,
                block.order_key.clone(),
                block.style,
                block.markdown.clone(),
            )
        })
        .collect()
}

#[tokio::test]
async fn sparse_structure_and_history_replay_exactly_with_nested_subtrees() {
    let f = database().await;
    remote_kinds(
        &f.connection,
        vec![
            OpKind::BlockCreate(block(id(1), f.page, None, OrderKey::from_ordinal(1))),
            OpKind::BlockCreate(block(id(2), f.page, None, OrderKey::from_ordinal(2))),
            OpKind::BlockCreate(block(id(3), f.page, None, OrderKey::from_ordinal(3))),
            OpKind::BlockCreate(block(id(4), f.page, Some(id(3)), OrderKey::first())),
            OpKind::BlockCreate(block(id(5), f.page, Some(id(4)), OrderKey::first())),
        ],
    )
    .await;
    configure(&f.connection).await;
    let before = db::get_page_document(&f.connection, f.page)
        .await
        .unwrap()
        .unwrap();
    let (_replica_directory, replica) = replica(&f.connection).await;
    let store = NotesStore::new(f.connection.clone());
    let inserted = store
        .create_block(
            f.page,
            None,
            Some(id(1)),
            BlockStyle::Bullet,
            "Middle".into(),
        )
        .await
        .unwrap()
        .value;
    replay(&f.connection, &replica, &[f.page]).await;
    store.move_block(id(3), Some(id(1)), None).await.unwrap();
    replay(&f.connection, &replica, &[f.page]).await;
    store
        .reorder_block(id(2), ReorderDirection::Up)
        .await
        .unwrap();
    replay(&f.connection, &replica, &[f.page]).await;
    store.indent_block(inserted.uuid).await.unwrap();
    replay(&f.connection, &replica, &[f.page]).await;
    store.outdent_block(inserted.uuid).await.unwrap();
    replay(&f.connection, &replica, &[f.page]).await;
    let final_document = db::get_page_document(&f.connection, f.page)
        .await
        .unwrap()
        .unwrap();
    for _ in 0..5 {
        assert_eq!(
            store.undo().await.unwrap().value,
            db::HistoryMoveResult::Applied
        );
        replay(&f.connection, &replica, &[f.page]).await;
    }
    assert_eq!(
        semantic(
            &db::get_page_document(&f.connection, f.page)
                .await
                .unwrap()
                .unwrap()
                .blocks
        ),
        semantic(&before.blocks)
    );
    for _ in 0..5 {
        assert_eq!(
            store.redo().await.unwrap().value,
            db::HistoryMoveResult::Applied
        );
        replay(&f.connection, &replica, &[f.page]).await;
    }
    assert_eq!(
        semantic(
            &db::get_page_document(&f.connection, f.page)
                .await
                .unwrap()
                .unwrap()
                .blocks
        ),
        semantic(&final_document.blocks)
    );
}

#[tokio::test]
async fn sparse_placement_after_cross_page_subtree_relocation_preserves_replication_and_page_guards()
 {
    let f = database().await;
    let target = db::create_page(&f.connection, "Target".into())
        .await
        .unwrap();
    remote_kinds(
        &f.connection,
        vec![
            OpKind::BlockCreate(block(id(1), f.page, None, OrderKey::first())),
            OpKind::BlockCreate(block(id(2), f.page, Some(id(1)), OrderKey::first())),
            OpKind::BlockCreate(block(id(3), f.page, Some(id(2)), OrderKey::first())),
            OpKind::BlockCreate(block(id(4), target.uuid, None, OrderKey::first())),
            OpKind::BlockCreate(block(id(5), target.uuid, None, OrderKey::from_ordinal(2))),
        ],
    )
    .await;
    remote_kinds(
        &f.connection,
        vec![
            OpKind::BlockMove(BlockMove {
                uuid: id(1),
                page_uuid: target.uuid,
                parent_uuid: None,
                order_key: key(OrderKey::STEP + OrderKey::STEP / 2),
            }),
            OpKind::BlockMove(BlockMove {
                uuid: id(2),
                page_uuid: target.uuid,
                parent_uuid: Some(id(1)),
                order_key: OrderKey::first(),
            }),
            OpKind::BlockMove(BlockMove {
                uuid: id(3),
                page_uuid: target.uuid,
                parent_uuid: Some(id(2)),
                order_key: OrderKey::first(),
            }),
        ],
    )
    .await;
    configure(&f.connection).await;
    for uuid in [id(1), id(2), id(3)] {
        assert_eq!(
            db::get_block(&f.connection, uuid)
                .await
                .unwrap()
                .unwrap()
                .page_uuid,
            target.uuid
        );
    }
    let (_replica_directory, replica) = replica(&f.connection).await;
    let reordered = db::reorder_block_with_ops(&f.connection, id(1), ReorderDirection::Down)
        .await
        .unwrap();
    assert_eq!(reordered.operations.len(), 1);
    assert_eq!(
        uuids(&roots(&f.connection, target.uuid).await),
        [id(4), id(5), id(1)]
    );
    replay(&f.connection, &replica, &[f.page, target.uuid]).await;
    assert_eq!(
        db::undo_history(&f.connection).await.unwrap(),
        db::HistoryMoveResult::Applied
    );
    assert_eq!(
        uuids(&roots(&f.connection, target.uuid).await),
        [id(4), id(1), id(5)]
    );
    replay(&f.connection, &replica, &[f.page, target.uuid]).await;
    assert_eq!(
        db::redo_history(&f.connection).await.unwrap(),
        db::HistoryMoveResult::Applied
    );
    replay(&f.connection, &replica, &[f.page, target.uuid]).await;
    let foreign_parent = db::create_block(
        &f.connection,
        f.page,
        None,
        None,
        BlockStyle::Bullet,
        "Foreign parent".into(),
    )
    .await
    .unwrap();
    let error = db::move_block(&f.connection, id(1), Some(foreign_parent.uuid), None)
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<notes_core::CoreError>(),
        Some(notes_core::CoreError::Conflict(_))
    ));
}
