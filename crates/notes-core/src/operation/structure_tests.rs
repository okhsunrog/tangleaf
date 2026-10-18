use super::*;
use crate::db;
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

async fn database() -> Connection {
    let connection = db::open_in_memory().await.unwrap();
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
    connection
}

fn operation(index: u128, kind: OpKind) -> Op {
    Op {
        op_id: Uuid::from_u128(0x1000 + index),
        workspace_uuid: WORKSPACE,
        device_id: DEVICE,
        hlc: Hlc::new(index as u64 * 1_000, 0, DEVICE),
        format_version: FORMAT_VERSION,
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
            style: BlockStyle::Bullet,
            markdown: format!("Block {uuid} [[B]] (({ROOT}))"),
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

fn markdown(index: u128, uuid: Uuid, markdown: String) -> Op {
    operation(
        index,
        OpKind::BlockSetMarkdown(BlockSetMarkdown { uuid, markdown }),
    )
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

#[derive(Debug, Clone, Copy)]
enum Transport {
    Remote,
    Sequenced,
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
            let (seq, is_new) = if let Some((seq, previous)) = known.get(&operation.op_id) {
                assert_eq!(previous, operation, "redelivery must preserve the envelope");
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

async fn apply_deliveries(
    connection: &Connection,
    transport: Transport,
    mode: StructureProjectionMode,
    deliveries: &[Delivery],
) {
    let outcomes = match transport {
        Transport::Remote => {
            apply_batch_with_projection_mode(
                connection,
                &deliveries
                    .iter()
                    .map(|item| item.operation.clone())
                    .collect::<Vec<_>>(),
                Origin::Remote,
                mode,
            )
            .await
        }
        Transport::Sequenced => {
            apply_sequenced_batch_with_projection_mode(
                connection,
                deliveries
                    .iter()
                    .map(|item| (item.seq, item.operation.clone()))
                    .collect(),
                mode,
            )
            .await
        }
    }
    .unwrap_or_else(|error| panic!("{transport:?}, {mode:?}: {error:#}"));
    assert_eq!(outcomes.len(), deliveries.len());
    for (outcome, delivery) in outcomes.iter().zip(deliveries) {
        assert_eq!(outcome.applied, delivery.is_new, "{transport:?}, {mode:?}");
    }
    assert_eq!(
        sync_cursor(connection).await.unwrap(),
        match transport {
            Transport::Remote => 0,
            Transport::Sequenced => deliveries.last().unwrap().cursor_after,
        }
    );
}

type PageLink = (Uuid, String, Option<Uuid>);
type Placement = (Uuid, Uuid, Option<Uuid>);

#[derive(Debug, Clone, PartialEq)]
struct ObservedState {
    snapshot: SyncSnapshot,
    page_links: Vec<PageLink>,
    block_refs: Vec<(Uuid, Uuid)>,
    applied_ops: Vec<(Uuid, Option<i64>)>,
    cursor: u64,
}

async fn observed_state(connection: &Connection) -> ObservedState {
    let (page_links, block_refs, applied_ops) = connection
        .call(|database| {
            let page_links = database
                .prepare(
                    "SELECT source_block_uuid, target_title, target_page_uuid
                   FROM page_links ORDER BY source_block_uuid, target_title",
                )?
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let block_refs = database
                .prepare(
                    "SELECT source_block_uuid, target_block_uuid FROM block_refs
                   ORDER BY source_block_uuid, target_block_uuid",
                )?
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let applied_ops = database
                .prepare("SELECT op_id, seq FROM applied_ops ORDER BY op_id")?
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok((page_links, block_refs, applied_ops))
        })
        .await
        .unwrap();
    ObservedState {
        snapshot: export_sync_snapshot(connection, 0).await.unwrap(),
        page_links,
        block_refs,
        applied_ops,
        cursor: sync_cursor(connection).await.unwrap(),
    }
}

fn assert_projection(left: &ObservedState, right: &ObservedState, context: &str) {
    assert_eq!(
        left.snapshot.blocks, right.snapshot.blocks,
        "blocks: {context}"
    );
    assert_eq!(
        left.snapshot.structures, right.snapshot.structures,
        "raw intents: {context}"
    );
    assert_eq!(
        left.snapshot.tombstones, right.snapshot.tombstones,
        "tombstones: {context}"
    );
    assert_eq!(left.snapshot, right.snapshot, "full snapshot: {context}");
    assert_eq!(left.page_links, right.page_links, "page links: {context}");
    assert_eq!(
        left.block_refs, right.block_refs,
        "block references: {context}"
    );
    // Sequence metadata belongs to the transport. Its exact values are checked
    // between Full/Components; the reference here is the remote singleton path.
    assert_eq!(
        left.applied_ops
            .iter()
            .map(|(uuid, _)| uuid)
            .collect::<Vec<_>>(),
        right
            .applied_ops
            .iter()
            .map(|(uuid, _)| uuid)
            .collect::<Vec<_>>(),
        "applied IDs: {context}",
    );
}

fn all_partitions(length: usize) -> Vec<Vec<usize>> {
    assert!((1..=6).contains(&length));
    (0..1usize << (length - 1))
        .map(|mask| {
            (1..=length)
                .filter(|end| *end == length || mask & (1 << (end - 1)) != 0)
                .collect()
        })
        .collect()
}

fn next_random(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn sampled_partitions(length: usize, seed: u64) -> Vec<Vec<usize>> {
    let mut partitions = vec![(1..=length).collect(), vec![length]];
    for chunk in [2, 5, 11] {
        let mut ends = (chunk..length).step_by(chunk).collect::<Vec<_>>();
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

async fn assert_modes(
    setup: &[Op],
    operations: &[Op],
    partitions: &[Vec<usize>],
    label: &str,
) -> Vec<ObservedState> {
    let setup_deliveries = deliveries(&[], setup);
    let stream = deliveries(setup, operations);
    let baseline = database().await;
    apply_deliveries(
        &baseline,
        Transport::Remote,
        StructureProjectionMode::Full,
        &setup_deliveries,
    )
    .await;
    let mut prefixes = Vec::new();
    for delivery in &stream {
        apply_deliveries(
            &baseline,
            Transport::Remote,
            StructureProjectionMode::Full,
            std::slice::from_ref(delivery),
        )
        .await;
        prefixes.push(observed_state(&baseline).await);
    }
    for transport in [Transport::Remote, Transport::Sequenced] {
        for ends in partitions {
            assert_eq!(ends.last(), Some(&operations.len()));
            let full = database().await;
            let components = database().await;
            // Canonical setup is independent of the dependency collector under test.
            for connection in [&full, &components] {
                apply_deliveries(
                    connection,
                    transport,
                    StructureProjectionMode::Full,
                    &setup_deliveries,
                )
                .await;
            }
            let mut start = 0;
            for &end in ends {
                assert!(start < end && end <= operations.len());
                apply_deliveries(
                    &full,
                    transport,
                    StructureProjectionMode::Full,
                    &stream[start..end],
                )
                .await;
                apply_deliveries(
                    &components,
                    transport,
                    StructureProjectionMode::Components,
                    &stream[start..end],
                )
                .await;
                let full_state = observed_state(&full).await;
                let component_state = observed_state(&components).await;
                let context = format!("{label}, {transport:?}, partition {ends:?}, prefix {end}");
                assert_projection(&component_state, &full_state, &context);
                assert_eq!(component_state, full_state, "transport metadata: {context}");
                assert_projection(&full_state, &prefixes[end - 1], &context);
                start = end;
            }
        }
    }
    prefixes
}

fn assert_placements(state: &ObservedState, expected: &[Placement]) {
    let mut expected = expected.to_vec();
    expected.sort();
    assert_eq!(
        state
            .snapshot
            .blocks
            .iter()
            .map(|block| (block.uuid, block.page_uuid, block.parent_uuid))
            .collect::<Vec<_>>(),
        expected,
    );
}

#[tokio::test]
async fn structure_components_follow_missing_parent_bridges_in_both_directions() {
    let operations = [
        block(3, ROOT, A, None),
        move_block(10, ROOT, B, None),
        move_block(11, ROOT, A, None),
    ];
    let prefixes = assert_modes(
        &[
            page(1, A, "A"),
            page(2, B, "B"),
            block(5, CHILD, A, Some(ROOT)),
            block(6, OTHER, A, Some(ROOT)),
            block(7, GRANDCHILD, A, Some(CHILD)),
        ],
        &operations,
        &all_partitions(operations.len()),
        "missing-parent bridge",
    )
    .await;
    for index in [0, 2] {
        assert_placements(
            &prefixes[index],
            &[
                (ROOT, A, None),
                (CHILD, A, Some(ROOT)),
                (OTHER, A, Some(ROOT)),
                (GRANDCHILD, A, Some(CHILD)),
            ],
        );
    }
    assert_placements(
        &prefixes[1],
        &[
            (ROOT, B, None),
            (CHILD, A, None),
            (OTHER, A, None),
            (GRANDCHILD, A, Some(CHILD)),
        ],
    );
}

#[tokio::test]
async fn structure_components_use_intended_parent_page_when_fallback_is_still_materialized() {
    let operations = [
        move_block(10, ROOT, C, None),
        page(11, C, "C"),
        move_block(12, ROOT, A, None),
    ];
    let prefixes = assert_modes(
        &tree_setup(),
        &operations,
        &all_partitions(operations.len()),
        "missing intended destination",
    )
    .await;
    assert_placements(
        &prefixes[0],
        &[
            (ROOT, A, None),
            (CHILD, A, None),
            (GRANDCHILD, A, Some(CHILD)),
            (OTHER, A, None),
        ],
    );
    assert_eq!(
        prefixes[0]
            .snapshot
            .structures
            .iter()
            .find(|intent| intent.block_uuid == ROOT)
            .unwrap()
            .page_uuid,
        C
    );
    assert_placements(
        &prefixes[1],
        &[
            (ROOT, C, None),
            (CHILD, A, None),
            (GRANDCHILD, A, Some(CHILD)),
            (OTHER, A, None),
        ],
    );
    assert_placements(
        &prefixes[2],
        &[
            (ROOT, A, None),
            (CHILD, A, Some(ROOT)),
            (GRANDCHILD, A, Some(CHILD)),
            (OTHER, A, None),
        ],
    );
}

#[tokio::test]
async fn structure_components_restore_other_cycle_members_after_an_edge_changes() {
    let operations = [
        move_block(10, ROOT, A, Some(CHILD)),
        move_block(11, CHILD, A, Some(ROOT)),
        move_block(12, ROOT, A, Some(OTHER)),
        move_block(13, OTHER, A, Some(CHILD)),
        move_block(14, CHILD, A, None),
    ];
    let prefixes = assert_modes(
        &[
            page(1, A, "A"),
            page(2, B, "B"),
            block(3, ROOT, A, None),
            block(4, CHILD, A, None),
            block(5, OTHER, A, None),
        ],
        &operations,
        &all_partitions(operations.len()),
        "cycle restoration",
    )
    .await;
    assert_placements(
        &prefixes[1],
        &[(ROOT, A, Some(CHILD)), (CHILD, A, None), (OTHER, A, None)],
    );
    assert_placements(
        &prefixes[2],
        &[
            (ROOT, A, Some(OTHER)),
            (CHILD, A, Some(ROOT)),
            (OTHER, A, None),
        ],
    );
    assert_placements(
        &prefixes[3],
        &[
            (ROOT, A, Some(OTHER)),
            (CHILD, A, Some(ROOT)),
            (OTHER, A, None),
        ],
    );
    assert_placements(
        &prefixes[4],
        &[
            (ROOT, A, Some(OTHER)),
            (CHILD, A, None),
            (OTHER, A, Some(CHILD)),
        ],
    );
}

#[tokio::test]
async fn structure_components_equal_clock_cycle_cut_uses_uuid() {
    let first = move_block(10, ROOT, B, Some(CHILD));
    let mut second = move_block(11, CHILD, B, Some(ROOT));
    second.hlc = first.hlc.clone();
    for operations in [[first.clone(), second.clone()], [second, first]] {
        let prefixes = assert_modes(
            &[
                page(1, A, "A"),
                page(2, B, "B"),
                block(3, ROOT, A, None),
                block(4, CHILD, A, None),
            ],
            &operations,
            &all_partitions(operations.len()),
            "equal-clock cycle",
        )
        .await;
        assert_placements(
            prefixes.last().unwrap(),
            &[(ROOT, B, Some(CHILD)), (CHILD, B, None)],
        );
    }
}

#[tokio::test]
async fn structure_components_recreated_page_activates_intents_from_another_materialized_page() {
    let operations = [
        delete_page(10, C),
        move_block(11, ROOT, C, None),
        page(12, C, "C recreated"),
        move_block(13, ROOT, B, None),
        page(14, C, "C next generation"),
    ];
    let prefixes = assert_modes(
        &[
            page(1, A, "A"),
            page(2, B, "B"),
            page(3, C, "C"),
            block(4, ROOT, A, None),
            block(5, CHILD, A, Some(ROOT)),
        ],
        &operations,
        &all_partitions(operations.len()),
        "reactivate destination",
    )
    .await;
    assert_placements(&prefixes[1], &[(ROOT, A, None), (CHILD, A, None)]);
    assert_placements(&prefixes[2], &[(ROOT, C, None), (CHILD, A, None)]);
    assert_placements(&prefixes[4], &[(ROOT, B, None), (CHILD, A, None)]);
}

#[tokio::test]
async fn structure_components_old_edge_removal_splits_materialized_and_intended_components() {
    let operations = [
        move_block(10, ROOT, A, Some(OTHER)),
        delete_block(11, CHILD, A),
        move_block(12, ROOT, B, None),
        move_block(13, GRANDCHILD, B, Some(ROOT)),
    ];
    let prefixes = assert_modes(
        &[
            page(1, A, "A"),
            page(2, B, "B"),
            block(3, CHILD, A, None),
            block(4, ROOT, A, Some(CHILD)),
            block(5, GRANDCHILD, A, Some(ROOT)),
            block(6, OTHER, A, None),
        ],
        &operations,
        &all_partitions(operations.len()),
        "old edge removal",
    )
    .await;
    assert_placements(
        &prefixes[0],
        &[
            (ROOT, A, Some(OTHER)),
            (CHILD, A, None),
            (GRANDCHILD, A, Some(ROOT)),
            (OTHER, A, None),
        ],
    );
    assert_placements(
        prefixes.last().unwrap(),
        &[
            (ROOT, B, None),
            (GRANDCHILD, B, Some(ROOT)),
            (OTHER, A, None),
        ],
    );
}

#[tokio::test]
async fn structure_components_respect_generation_floors_and_page_cascades() {
    let mut stale_delete = delete_block(13, ROOT, A);
    stale_delete.hlc = Hlc::new(11_000, 0, DEVICE);
    let operations = [
        move_block(10, CHILD, A, Some(ROOT)),
        delete_block(11, ROOT, A),
        block(12, ROOT, A, None),
        stale_delete,
        move_block(14, CHILD, A, Some(ROOT)),
        page(15, A, "A next generation"),
    ];
    let prefixes = assert_modes(
        &tree_setup(),
        &operations,
        &sampled_partitions(operations.len(), 0xFACE),
        "generation floor and cascade",
    )
    .await;
    assert_placements(
        &prefixes[2],
        &[
            (ROOT, A, None),
            (CHILD, A, None),
            (GRANDCHILD, A, Some(CHILD)),
            (OTHER, A, None),
        ],
    );
    assert_placements(
        &prefixes[4],
        &[
            (ROOT, A, None),
            (CHILD, A, Some(ROOT)),
            (GRANDCHILD, A, Some(CHILD)),
            (OTHER, A, None),
        ],
    );
    assert!(prefixes.last().unwrap().snapshot.blocks.is_empty());
    assert_eq!(prefixes.last().unwrap().snapshot.tombstones.len(), 4);
}

#[tokio::test]
async fn structure_components_stale_lww_and_redelivery_preserve_raw_state_and_references() {
    let moved = move_block(10, ROOT, B, None);
    let deleted = delete_page(13, A);
    let operations = [
        moved.clone(),
        move_block(9, ROOT, A, Some(OTHER)),
        moved,
        markdown(12, ROOT, format!("Latest [[A]] (({CHILD}))")),
        markdown(11, ROOT, format!("Stale [[B]] (({OTHER}))")),
        deleted.clone(),
        deleted,
    ];
    let prefixes = assert_modes(
        &tree_setup(),
        &operations,
        &sampled_partitions(operations.len(), 0xDE1E7E),
        "stale and repeated operations",
    )
    .await;
    let last = prefixes.last().unwrap();
    assert_placements(last, &[(ROOT, B, None)]);
    assert_eq!(last.page_links, [(ROOT, "a".into(), None)]);
    assert_eq!(last.block_refs, [(ROOT, CHILD)]);
    assert_eq!(
        last.snapshot.blocks[0].markdown,
        format!("Latest [[A]] (({CHILD}))")
    );
}

#[tokio::test]
async fn structure_components_sequence_gap_rolls_back_mutations_ids_links_and_cursor() {
    let mut results = Vec::new();
    for mode in [
        StructureProjectionMode::Full,
        StructureProjectionMode::Components,
    ] {
        let connection = database().await;
        let setup = tree_setup();
        apply_deliveries(
            &connection,
            Transport::Sequenced,
            StructureProjectionMode::Full,
            &deliveries(&[], &setup),
        )
        .await;
        let before = observed_state(&connection).await;
        let next = setup.len() as u64 + 1;
        let error = apply_sequenced_batch_with_projection_mode(
            &connection,
            vec![
                (next, move_block(10, ROOT, B, None)),
                (next + 1, delete_page(11, A)),
                (next + 3, block(12, OTHER, B, None)),
            ],
            mode,
        )
        .await
        .expect_err("a gap must roll back the complete transaction");
        assert!(matches!(error.downcast_ref::<CoreError>(),
            Some(CoreError::SyncSequenceGap { expected, received })
                if *expected == next + 2 && *received == next + 3));
        assert_eq!(observed_state(&connection).await, before);
        apply_deliveries(
            &connection,
            Transport::Sequenced,
            mode,
            &deliveries(&setup, &[move_block(10, ROOT, B, None), delete_page(11, A)]),
        )
        .await;
        let after = observed_state(&connection).await;
        assert_placements(&after, &[(ROOT, B, None)]);
        results.push(after);
    }
    assert_eq!(results[0], results[1]);
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
                    title: Some(if step.is_multiple_of(2) { "A" } else { "B" }.into()),
                }),
            ),
        };
        operations.push(operation);
    }
    operations
}

#[tokio::test]
async fn structure_components_fixed_seed_streams_match_full_at_every_committed_prefix() {
    for seed in [1, 0x5EED, 0xCAFE, 0xDEAD_BEEF] {
        let operations = generated_stream(seed, 64);
        assert_modes(
            &tree_setup(),
            &operations,
            &sampled_partitions(operations.len(), seed),
            &format!("seed={seed:#x}"),
        )
        .await;
    }
}
