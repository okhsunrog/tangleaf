use notes_core::{
    BlockCreate, BlockDelete, BlockMove, BlockStyle, Connection, Hlc, Op, OpKind, OrderKey, Origin,
    PageCreate, PageDelete, PageKind, PageLayout, SyncSnapshot, db,
};
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
