use notes_core::{
    BlockCreate, BlockDelete, BlockMove, BlockSetMarkdown, BlockStyle, Connection, Hlc, Op, OpKind,
    OrderKey, Origin, PageCreate, PageDelete, PageKind, PageLayout, PageSetTitle, SyncSnapshot, db,
};
use std::collections::HashMap;
use uuid::Uuid;

const WORKSPACE: Uuid = Uuid::from_u128(0xC0DE);
const DEVICE: Uuid = Uuid::from_u128(0xD3A1CE);
const A: Uuid = Uuid::from_u128(1);
const B: Uuid = Uuid::from_u128(2);
const C: Uuid = Uuid::from_u128(3);
const ROOT: Uuid = Uuid::from_u128(10);
const CHILD: Uuid = Uuid::from_u128(11);
const GRANDCHILD: Uuid = Uuid::from_u128(12);
const OTHER: Uuid = Uuid::from_u128(13);

struct TestDatabase {
    _directory: tempfile::TempDir,
    connection: Connection,
}

async fn database() -> TestDatabase {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(directory.path().join("notes.db")).await.unwrap();
    connection
        .call(|database| {
            database.execute(
                "UPDATE workspace SET uuid = ?1 WHERE singleton = 1",
                [WORKSPACE],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    TestDatabase {
        _directory: directory,
        connection,
    }
}

fn operation(index: u128, kind: OpKind) -> Op {
    Op {
        op_id: Uuid::from_u128(0x1000 + index),
        workspace_uuid: WORKSPACE,
        device_id: DEVICE,
        hlc: Hlc::new(index as u64 * 1_000, 0, DEVICE),
        format_version: notes_core::operation::FORMAT_VERSION,
        kind,
    }
}

fn page(index: u128, uuid: Uuid, title: &str) -> Op {
    operation(
        index,
        OpKind::PageCreate(PageCreate {
            uuid,
            kind: PageKind::Note,
            title: Some(title.into()),
            layout: PageLayout::Outline,
            created_at: index as i64,
        }),
    )
}

fn block(index: u128, uuid: Uuid, page_uuid: Uuid, parent_uuid: Option<Uuid>) -> Op {
    operation(
        index,
        OpKind::BlockCreate(BlockCreate {
            uuid,
            page_uuid,
            parent_uuid,
            order_key: OrderKey::from_ordinal(index as usize),
            style: BlockStyle::Paragraph,
            markdown: format!("Block {uuid} [[B]]"),
            created_at: index as i64,
        }),
    )
}

fn move_block(index: u128, uuid: Uuid, page_uuid: Uuid, parent_uuid: Option<Uuid>) -> Op {
    operation(
        index,
        OpKind::BlockMove(BlockMove {
            uuid,
            page_uuid,
            parent_uuid,
            order_key: OrderKey::from_ordinal(index as usize),
        }),
    )
}

fn delete_page(index: u128, uuid: Uuid) -> Op {
    operation(index, OpKind::PageDelete(PageDelete { uuid }))
}

fn delete_block(index: u128, uuid: Uuid, page_uuid: Uuid) -> Op {
    operation(index, OpKind::BlockDelete(BlockDelete { uuid, page_uuid }))
}

#[derive(Debug, Clone, Copy)]
enum Transport {
    Remote,
    Sequenced,
}

impl Transport {
    async fn apply(self, connection: &Connection, operations: &[Op], offset: usize) {
        let outcomes = match self {
            Self::Remote => notes_core::apply_batch(connection, operations, Origin::Remote)
                .await
                .unwrap(),
            Self::Sequenced => notes_core::apply_sequenced_batch(
                connection,
                operations
                    .iter()
                    .enumerate()
                    .map(|(index, operation)| ((offset + index + 1) as u64, operation.clone()))
                    .collect(),
            )
            .await
            .unwrap(),
        };
        assert_eq!(outcomes.len(), operations.len());
        assert!(outcomes.iter().all(|outcome| outcome.applied));
        assert_eq!(
            notes_core::sync_cursor(connection).await.unwrap(),
            match self {
                Self::Remote => 0,
                Self::Sequenced => (offset + operations.len()) as u64,
            },
        );
    }
}

type Placement = (Uuid, Uuid, Option<Uuid>);
type PageLink = (Uuid, String, Option<Uuid>);

async fn page_links(connection: &Connection) -> Vec<PageLink> {
    connection
        .call(|database| {
            database
                .prepare(
                    "SELECT source_block_uuid, target_title, target_page_uuid
                     FROM page_links ORDER BY source_block_uuid, target_title",
                )?
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect()
        })
        .await
        .unwrap()
}

// Setup establishes the same starting state. Partition every boundary of the subsequent
// ordered stream, including one transaction and one transaction per operation.
async fn assert_partitions(
    setup: &[Op],
    operations: &[Op],
    expected_pages: &[Uuid],
    expected_blocks: &[Placement],
) -> SyncSnapshot {
    assert!(!operations.is_empty());
    let mut reference: Option<SyncSnapshot> = None;
    let mut expected_pages = expected_pages.to_vec();
    expected_pages.sort();
    let mut expected_blocks = expected_blocks.to_vec();
    expected_blocks.sort();
    let expected_links = expected_blocks
        .iter()
        .map(|(uuid, _, _)| (*uuid, "b".into(), expected_pages.contains(&B).then_some(B)))
        .collect::<Vec<PageLink>>();

    // Start with single-operation transactions so the independent expectations also
    // validate the reference, instead of blessing whatever the old executor happens to do.
    for transport in [Transport::Remote, Transport::Sequenced] {
        for boundaries in (0..1usize << (operations.len() - 1)).rev() {
            let database = database().await;
            let connection = &database.connection;
            transport.apply(connection, setup, 0).await;
            let mut start = 0;
            for end in 1..=operations.len() {
                if end == operations.len() || boundaries & (1 << (end - 1)) != 0 {
                    transport
                        .apply(connection, &operations[start..end], setup.len() + start)
                        .await;
                    start = end;
                }
            }
            // Only the transport cursor is normalized; timestamps, existence generations,
            // tombstones, original structure intents and all other snapshot fields must agree.
            let snapshot = notes_core::export_sync_snapshot(connection, 0)
                .await
                .unwrap();
            let context = format!("{transport:?}, partition bitmask {boundaries:b}");
            assert_eq!(
                snapshot
                    .pages
                    .iter()
                    .map(|page| page.uuid)
                    .collect::<Vec<_>>(),
                expected_pages,
                "pages: {context}",
            );
            assert_eq!(
                snapshot
                    .blocks
                    .iter()
                    .map(|block| (block.uuid, block.page_uuid, block.parent_uuid))
                    .collect::<Vec<_>>(),
                expected_blocks,
                "surviving blocks and parents: {context}",
            );
            assert_eq!(
                page_links(connection).await,
                expected_links,
                "links: {context}"
            );
            if let Some(reference) = &reference {
                assert_eq!(&snapshot, reference, "full snapshot: {context}");
            } else {
                reference = Some(snapshot);
            }
        }
    }
    reference.unwrap()
}

fn tree_setup() -> Vec<Op> {
    vec![
        page(1, A, "A"),
        page(2, B, "B"),
        block(3, ROOT, A, None),
        block(4, CHILD, A, Some(ROOT)),
        block(5, GRANDCHILD, A, Some(CHILD)),
        block(6, OTHER, A, None),
    ]
}

#[tokio::test]
async fn moving_subtree_out_before_page_delete_preserves_every_moved_block() {
    assert_partitions(
        &tree_setup(),
        &[
            // A subtree move is an explicit operation for each descendant.
            move_block(10, ROOT, B, None),
            move_block(11, CHILD, B, Some(ROOT)),
            move_block(12, GRANDCHILD, B, Some(CHILD)),
            delete_page(13, A),
        ],
        &[B],
        &[
            (ROOT, B, None),
            (CHILD, B, Some(ROOT)),
            (GRANDCHILD, B, Some(CHILD)),
        ],
    )
    .await;
}

#[tokio::test]
async fn moving_subtree_into_page_before_deleting_it_deletes_every_moved_block() {
    assert_partitions(
        &tree_setup(),
        &[
            move_block(10, ROOT, B, None),
            move_block(11, CHILD, B, Some(ROOT)),
            move_block(12, GRANDCHILD, B, Some(CHILD)),
            delete_page(13, B),
        ],
        &[A],
        &[(OTHER, A, None)],
    )
    .await;
}

#[tokio::test]
async fn reparenting_before_block_delete_keeps_children_and_their_new_parent() {
    assert_partitions(
        &tree_setup(),
        &[
            move_block(10, CHILD, A, Some(OTHER)),
            delete_block(11, ROOT, A),
        ],
        &[A, B],
        &[
            (CHILD, A, Some(OTHER)),
            (GRANDCHILD, A, Some(CHILD)),
            (OTHER, A, None),
        ],
    )
    .await;
}

#[tokio::test]
async fn moving_and_deleting_parent_detaches_surviving_children() {
    assert_partitions(
        &tree_setup(),
        &[move_block(10, ROOT, B, None), delete_block(11, ROOT, B)],
        &[A, B],
        &[
            (CHILD, A, None),
            (GRANDCHILD, A, Some(CHILD)),
            (OTHER, A, None),
        ],
    )
    .await;
}

#[tokio::test]
async fn page_recreation_does_not_reclaim_blocks_moved_to_another_page() {
    assert_partitions(
        &[page(1, A, "A"), page(2, B, "B"), block(3, ROOT, A, None)],
        &[
            move_block(10, ROOT, B, None),
            delete_page(11, A),
            page(12, A, "A recreated"),
            block(13, OTHER, A, None),
        ],
        &[A, B],
        &[(ROOT, B, None), (OTHER, A, None)],
    )
    .await;
}

#[tokio::test]
async fn newer_page_create_preserves_blocks_moved_out_of_the_replaced_generation() {
    assert_partitions(
        &[page(1, A, "A"), page(2, B, "B"), block(3, ROOT, A, None)],
        &[move_block(10, ROOT, B, None), page(11, A, "A recreated")],
        &[A, B],
        &[(ROOT, B, None)],
    )
    .await;
}

#[tokio::test]
async fn newer_page_create_deletes_blocks_moved_into_the_replaced_generation() {
    assert_partitions(
        &[page(1, A, "A"), page(2, B, "B"), block(3, ROOT, A, None)],
        &[move_block(10, ROOT, B, None), page(11, B, "B")],
        &[A, B],
        &[],
    )
    .await;
}

#[tokio::test]
async fn move_to_missing_page_keeps_last_valid_placement_and_pending_intent() {
    let snapshot = assert_partitions(
        &[page(1, A, "A"), page(2, B, "B"), block(3, ROOT, A, None)],
        &[move_block(10, ROOT, B, None), move_block(11, ROOT, C, None)],
        &[A, B],
        &[(ROOT, B, None)],
    )
    .await;
    assert_eq!(snapshot.structures.len(), 1);
    assert_eq!(snapshot.structures[0].page_uuid, C);
}

#[tokio::test]
async fn move_to_deleted_page_keeps_last_valid_placement_and_pending_intent() {
    let snapshot = assert_partitions(
        &[
            page(1, A, "A"),
            page(2, B, "B"),
            page(3, C, "C"),
            block(4, ROOT, A, None),
        ],
        &[
            delete_page(10, C),
            move_block(11, ROOT, B, None),
            move_block(12, ROOT, C, None),
        ],
        &[A, B],
        &[(ROOT, B, None)],
    )
    .await;
    assert_eq!(snapshot.structures.len(), 1);
    assert_eq!(snapshot.structures[0].page_uuid, C);
}

#[tokio::test]
async fn missing_parent_created_then_moved_into_place_attaches_existing_child() {
    // A missing-parent intent is retained, but cannot bind to a newer parent generation.
    // Deliver the parent's older create after the child's intent to model delayed sync.
    assert_partitions(
        &[
            page(1, A, "A"),
            page(2, B, "B"),
            block(5, CHILD, B, Some(ROOT)),
        ],
        &[
            block(3, ROOT, A, None),
            move_block(10, ROOT, B, None),
            delete_page(11, A),
        ],
        &[B],
        &[(ROOT, B, None), (CHILD, B, Some(ROOT))],
    )
    .await;
}

#[tokio::test]
async fn recreated_parent_does_not_capture_children_from_its_deleted_generation() {
    assert_partitions(
        &tree_setup(),
        &[
            move_block(10, CHILD, A, Some(ROOT)),
            delete_block(11, ROOT, A),
            block(12, ROOT, A, None),
        ],
        &[A, B],
        &[
            (ROOT, A, None),
            (CHILD, A, None),
            (GRANDCHILD, A, Some(CHILD)),
            (OTHER, A, None),
        ],
    )
    .await;
}

#[tokio::test]
async fn sequenced_gap_after_move_and_delete_rolls_back_state_and_cursor() {
    let database = database().await;
    let connection = &database.connection;
    let setup = tree_setup();
    Transport::Sequenced.apply(connection, &setup, 0).await;
    let before = notes_core::export_sync_snapshot(connection, 0)
        .await
        .unwrap();
    let links_before = page_links(connection).await;
    let next = setup.len() as u64 + 1;
    let error = notes_core::apply_sequenced_batch(
        connection,
        vec![
            (next, move_block(10, ROOT, B, None)),
            (next + 1, delete_page(11, A)),
            (next + 3, block(12, OTHER, B, None)),
        ],
    )
    .await
    .expect_err("a sequence gap must abort previous operations in the same transaction");
    assert!(matches!(
        error.downcast_ref::<notes_core::CoreError>(),
        Some(notes_core::CoreError::SyncSequenceGap { expected, received })
            if *expected == next + 2 && *received == next + 3
    ));
    assert_eq!(
        notes_core::sync_cursor(connection).await.unwrap(),
        setup.len() as u64
    );
    assert_eq!(
        notes_core::export_sync_snapshot(connection, 0)
            .await
            .unwrap(),
        before
    );
    assert_eq!(page_links(connection).await, links_before);

    // Rolled-back operation IDs must not remain in applied_ops and suppress the retry.
    Transport::Sequenced
        .apply(
            connection,
            &[move_block(10, ROOT, B, None), delete_page(11, A)],
            setup.len(),
        )
        .await;
    let after = notes_core::export_sync_snapshot(connection, 0)
        .await
        .unwrap();
    assert_eq!(after.blocks.len(), 1);
    assert_eq!((after.blocks[0].uuid, after.blocks[0].page_uuid), (ROOT, B));
}

#[tokio::test]
async fn equal_clock_cycles_resolve_by_uuid_independently_of_batch_boundaries() {
    let setup = [
        page(1, A, "A"),
        page(2, B, "B"),
        block(3, ROOT, A, None),
        block(4, CHILD, A, None),
    ];
    let first = move_block(10, ROOT, B, Some(CHILD));
    let mut second = move_block(11, CHILD, B, Some(ROOT));
    second.hlc = first.hlc.clone();
    for moves in [[first.clone(), second.clone()], [second, first]] {
        assert_partitions(
            &setup,
            &[moves[0].clone(), moves[1].clone(), delete_page(12, A)],
            &[B],
            &[(ROOT, B, Some(CHILD)), (CHILD, B, None)],
        )
        .await;
    }
}

#[tokio::test]
async fn losing_move_cannot_return_a_block_to_a_page_about_to_be_deleted() {
    for stale_target in [A, C] {
        assert_partitions(
            &[page(1, A, "A"), page(2, B, "B"), block(3, ROOT, A, None)],
            &[
                move_block(10, ROOT, B, None),
                move_block(9, ROOT, stale_target, Some(OTHER)),
                delete_page(11, A),
            ],
            &[B],
            &[(ROOT, B, None)],
        )
        .await;
    }
}

#[tokio::test]
async fn delayed_block_delete_cannot_remove_a_recreated_parent_or_new_children() {
    assert_partitions(
        &tree_setup(),
        &[
            delete_block(10, ROOT, A),
            block(12, ROOT, A, None),
            delete_block(11, ROOT, A),
            move_block(13, CHILD, A, Some(ROOT)),
        ],
        &[A, B],
        &[
            (ROOT, A, None),
            (CHILD, A, Some(ROOT)),
            (GRANDCHILD, A, Some(CHILD)),
            (OTHER, A, None),
        ],
    )
    .await;
}

#[tokio::test]
async fn delayed_page_delete_cannot_remove_a_recreated_page_or_new_blocks() {
    assert_partitions(
        &tree_setup(),
        &[
            delete_page(10, A),
            page(12, A, "A recreated"),
            block(13, ROOT, A, Some(Uuid::from_u128(99))),
            delete_page(11, A),
        ],
        &[A, B],
        &[(ROOT, A, None)],
    )
    .await;
}

#[derive(Clone, Debug, PartialEq)]
struct ObservedState {
    snapshot: SyncSnapshot,
    page_links: Vec<PageLink>,
    block_refs: Vec<(Uuid, Uuid)>,
}

async fn observed_state(connection: &Connection) -> ObservedState {
    let block_refs = connection
        .call(|database| {
            database
                .prepare(
                    "SELECT source_block_uuid, target_block_uuid FROM block_refs
                     ORDER BY source_block_uuid, target_block_uuid",
                )?
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
        .await
        .unwrap();
    ObservedState {
        snapshot: notes_core::export_sync_snapshot(connection, 0)
            .await
            .unwrap(),
        page_links: page_links(connection).await,
        block_refs,
    }
}

struct Delivery {
    operation: Op,
    seq: u64,
    is_new: bool,
    cursor_after: u64,
}

fn deliveries(setup: &[Op], operations: &[Op]) -> Vec<Delivery> {
    let mut known = setup
        .iter()
        .enumerate()
        .map(|(index, operation)| (operation.op_id, ((index + 1) as u64, operation.clone())))
        .collect::<HashMap<_, _>>();
    let mut cursor = setup.len() as u64;
    operations
        .iter()
        .map(|operation| {
            let (seq, is_new) = if let Some((seq, prior)) = known.get(&operation.op_id) {
                assert_eq!(
                    prior, operation,
                    "a redelivery must preserve the entire envelope"
                );
                (*seq, false)
            } else {
                cursor += 1;
                known.insert(operation.op_id, (cursor, operation.clone()));
                (cursor, true)
            };
            Delivery {
                operation: operation.clone(),
                seq,
                is_new,
                cursor_after: cursor,
            }
        })
        .collect()
}

async fn apply_deliveries(transport: Transport, connection: &Connection, deliveries: &[Delivery]) {
    let outcomes = match transport {
        Transport::Remote => notes_core::apply_batch(
            connection,
            &deliveries
                .iter()
                .map(|item| item.operation.clone())
                .collect::<Vec<_>>(),
            Origin::Remote,
        )
        .await
        .unwrap(),
        Transport::Sequenced => notes_core::apply_sequenced_batch(
            connection,
            deliveries
                .iter()
                .map(|item| (item.seq, item.operation.clone()))
                .collect(),
        )
        .await
        .unwrap(),
    };
    assert_eq!(outcomes.len(), deliveries.len());
    for (outcome, delivery) in outcomes.iter().zip(deliveries) {
        assert_eq!(
            outcome.applied, delivery.is_new,
            "{transport:?}: {:?}",
            delivery.operation
        );
    }
    let expected_cursor = match transport {
        Transport::Remote => 0,
        Transport::Sequenced => deliveries.last().unwrap().cursor_after,
    };
    assert_eq!(
        notes_core::sync_cursor(connection).await.unwrap(),
        expected_cursor
    );
}

fn all_partitions(length: usize) -> Vec<Vec<usize>> {
    (0..1usize << (length - 1))
        .map(|mask| {
            (1..=length)
                .filter(|end| *end == length || mask & (1 << (end - 1)) != 0)
                .collect()
        })
        .collect()
}

// Long generated streams use selected partitions and compare each committed prefix,
// so a later deletion cannot conceal an earlier divergence. Short tests below also
// assert an independent final result; singleton execution alone is not a semantic oracle.
async fn assert_stream_partitions(
    setup: &[Op],
    operations: &[Op],
    partitions: &[Vec<usize>],
    label: &str,
) -> ObservedState {
    let deliveries = deliveries(setup, operations);
    let baseline = database().await;
    Transport::Remote
        .apply(&baseline.connection, setup, 0)
        .await;
    let mut prefixes = Vec::with_capacity(operations.len());
    for delivery in &deliveries {
        apply_deliveries(
            Transport::Remote,
            &baseline.connection,
            std::slice::from_ref(delivery),
        )
        .await;
        prefixes.push(observed_state(&baseline.connection).await);
    }
    for transport in [Transport::Remote, Transport::Sequenced] {
        for ends in partitions {
            assert_eq!(ends.last(), Some(&operations.len()));
            let database = database().await;
            transport.apply(&database.connection, setup, 0).await;
            let mut start = 0;
            for &end in ends {
                assert!(start < end && end <= operations.len());
                apply_deliveries(transport, &database.connection, &deliveries[start..end]).await;
                let state = observed_state(&database.connection).await;
                let expected = &prefixes[end - 1];
                let context = format!("{label}, {transport:?}, partition {ends:?}, prefix {end}");
                // Check individual fields first to keep any failure report small, then the
                // complete snapshot so new fields are automatically covered too.
                assert_eq!(
                    state.snapshot.pages, expected.snapshot.pages,
                    "pages: {context}"
                );
                assert_eq!(
                    state.snapshot.blocks, expected.snapshot.blocks,
                    "blocks: {context}"
                );
                assert_eq!(
                    state.snapshot.structures, expected.snapshot.structures,
                    "intents: {context}"
                );
                assert_eq!(state.snapshot, expected.snapshot, "snapshot: {context}");
                assert_eq!(
                    state.page_links, expected.page_links,
                    "page links: {context}"
                );
                assert_eq!(
                    state.block_refs, expected.block_refs,
                    "block references: {context}"
                );
                start = end;
            }
        }
    }
    prefixes.pop().unwrap()
}

#[tokio::test]
async fn redelivery_preserves_state_and_does_not_advance_the_original_server_sequence() {
    let moved = move_block(10, ROOT, B, None);
    let deleted = delete_page(11, A);
    let operations = [
        moved.clone(),
        moved.clone(),
        deleted.clone(),
        moved,
        deleted,
    ];
    let state = assert_stream_partitions(
        &[page(1, A, "A"), page(2, B, "B"), block(3, ROOT, A, None)],
        &operations,
        &all_partitions(operations.len()),
        "repeated move and delete",
    )
    .await;
    assert_eq!(state.snapshot.pages.len(), 1);
    assert_eq!(state.snapshot.pages[0].uuid, B);
    assert_eq!(state.snapshot.blocks.len(), 1);
    assert_eq!(
        (
            state.snapshot.blocks[0].uuid,
            state.snapshot.blocks[0].page_uuid
        ),
        (ROOT, B)
    );
    assert_eq!(state.page_links, vec![(ROOT, "b".into(), Some(B))]);
}

fn markdown(index: u128, uuid: Uuid, markdown: String) -> Op {
    operation(
        index,
        OpKind::BlockSetMarkdown(BlockSetMarkdown { uuid, markdown }),
    )
}

#[tokio::test]
async fn losing_markdown_does_not_resurrect_old_page_or_block_references() {
    let text = format!("Keep [[A]] (({CHILD}))");
    let operations = [
        markdown(10, ROOT, text.clone()),
        markdown(9, ROOT, format!("Stale [[B]] (({OTHER}))")),
        move_block(11, ROOT, B, None),
        delete_page(12, A),
    ];
    let state = assert_stream_partitions(
        &tree_setup(),
        &operations,
        &all_partitions(operations.len()),
        "stale markdown",
    )
    .await;
    assert_eq!(state.snapshot.blocks.len(), 1);
    assert_eq!(state.snapshot.blocks[0].markdown, text);
    assert_eq!(state.page_links, vec![(ROOT, "a".into(), None)]);
    assert_eq!(state.block_refs, vec![(ROOT, CHILD)]);
}

#[tokio::test]
async fn title_ambiguity_and_reassignment_resolve_links_at_each_batch_boundary() {
    let operations = [
        markdown(10, ROOT, "[[A]] [[B]]".into()),
        operation(
            11,
            OpKind::PageSetTitle(PageSetTitle {
                uuid: B,
                title: Some("A".into()),
            }),
        ),
        operation(
            12,
            OpKind::PageSetTitle(PageSetTitle {
                uuid: A,
                title: Some("C".into()),
            }),
        ),
    ];
    let state = assert_stream_partitions(
        &[page(1, A, "A"), page(2, B, "B"), block(3, ROOT, A, None)],
        &operations,
        &all_partitions(operations.len()),
        "ambiguous titles",
    )
    .await;
    assert_eq!(
        state.page_links,
        vec![(ROOT, "a".into(), Some(B)), (ROOT, "b".into(), None)]
    );
}

// Small deterministic generator, not a random seed chosen by the test runner. A failure
// names the seed and committed prefix, making the exact stream reproducible in CI.
fn next_random(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn generated_stream(seed: u64, length: usize) -> Vec<Op> {
    let mut random = seed;
    let pages = [A, B, C];
    let nodes = [ROOT, CHILD, GRANDCHILD, OTHER, Uuid::from_u128(14)];
    let mut live_pages = [true, true, false];
    let mut operations = Vec::<Op>::new();
    for step in 0..length {
        let index = 100 + step as u128;
        let node = nodes[next_random(&mut random) as usize % nodes.len()];
        let page_index = next_random(&mut random) as usize % pages.len();
        let destination = pages[page_index];
        let parent = match next_random(&mut random) % 3 {
            0 => None,
            1 => Some(Uuid::from_u128(99)),
            _ => Some(nodes[next_random(&mut random) as usize % nodes.len()]),
        }
        .filter(|parent| *parent != node);
        let operation = match next_random(&mut random) % 12 {
            0 | 1 => move_block(index, node, destination, parent),
            2 => {
                live_pages[page_index] = false;
                delete_page(index, destination)
            }
            3 => {
                live_pages[page_index] = true;
                page(index, destination, ["A", "B", "C"][page_index])
            }
            4 => delete_block(index, node, destination),
            5 => {
                if let Some(live) = live_pages.iter().position(|live| *live) {
                    block(index, node, pages[live], parent)
                } else {
                    live_pages[page_index] = true;
                    page(index, destination, ["A", "B", "C"][page_index])
                }
            }
            6 => markdown(
                index,
                node,
                format!("Revision {step} [[A]] [[B]] (({OTHER}))"),
            ),
            7 => {
                let mut stale = move_block(index, node, destination, parent);
                stale.hlc = Hlc::new(7_000, 0, DEVICE);
                stale
            }
            8 if !operations.is_empty() => {
                operations[next_random(&mut random) as usize % operations.len()].clone()
            }
            9 => {
                let mut stale = markdown(index, node, format!("Old [[C]] (({ROOT}))"));
                stale.hlc = Hlc::new(7_000, 0, DEVICE);
                stale
            }
            _ => operation(
                index,
                OpKind::PageSetTitle(PageSetTitle {
                    uuid: destination,
                    title: Some(if step % 2 == 0 { "A" } else { "B" }.into()),
                }),
            ),
        };
        operations.push(operation);
    }
    operations
}

fn sampled_partitions(length: usize, seed: u64) -> Vec<Vec<usize>> {
    let mut partitions = vec![(1..=length).collect(), vec![length]];
    for chunk_size in [2, 5, 11] {
        let mut ends = (chunk_size..length).step_by(chunk_size).collect::<Vec<_>>();
        ends.push(length);
        partitions.push(ends);
    }
    let mut random = seed;
    for _ in 0..3 {
        let mut ends = (1..length)
            .filter(|_| next_random(&mut random).is_multiple_of(4))
            .collect::<Vec<_>>();
        ends.push(length);
        partitions.push(ends);
    }
    partitions
}

#[tokio::test]
async fn fixed_seed_streams_preserve_every_committed_prefix_across_partitions() {
    for seed in [1, 0x5EED, 0xCAFE, 0xDEAD_BEEF] {
        let operations = generated_stream(seed, 64);
        assert_stream_partitions(
            &tree_setup(),
            &operations,
            &sampled_partitions(operations.len(), seed),
            &format!("seed={seed:#x}"),
        )
        .await;
    }
}
