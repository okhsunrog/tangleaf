use crate::changes::*;
use crate::operation::{ProjectionPolicy, apply_local_kinds_tracked};
use crate::{
    AttachmentAdd, AttachmentOwner, AttachmentRemove, BlockCreate, BlockDelete, BlockMove,
    BlockSetMarkdown, BlockSetStyle, BlockStyle, OpKind, OrderKey, PageAliasSet, PageCreate,
    PageDelete, PageKind, PageLayout, PageSetLayout, PageSetTitle, db,
};
use std::collections::BTreeSet;
use uuid::Uuid;

async fn apply(
    conn: &crate::Connection,
    kinds: Vec<OpKind>,
    policy: ProjectionPolicy,
) -> ChangeSet {
    conn.call_domain(move |db| -> crate::CoreResult<_> {
        let tx = db.transaction()?;
        let applied = apply_local_kinds_tracked(
            &tx,
            kinds,
            policy,
            crate::operation::StructureProjectionMode::Components,
        )?;
        tx.commit()?;
        Ok(applied.changes)
    })
    .await
    .unwrap()
}

fn page(uuid: Uuid, title: &str) -> OpKind {
    OpKind::PageCreate(PageCreate {
        uuid,
        title: Some(title.into()),
        layout: PageLayout::Outline,
        kind: PageKind::Note,
        created_at: 10,
    })
}

fn block(uuid: Uuid, page: Uuid, parent: Option<Uuid>, markdown: &str) -> OpKind {
    OpKind::BlockCreate(BlockCreate {
        uuid,
        page_uuid: page,
        parent_uuid: parent,
        order_key: OrderKey::first(),
        style: BlockStyle::Bullet,
        markdown: markdown.into(),
        created_at: 10,
    })
}

fn sorted(values: impl IntoIterator<Item = Uuid>) -> Vec<Uuid> {
    values
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[tokio::test]
async fn content_and_revision_changes_retain_placement_without_page_row_invalidation() {
    let conn = db::open_in_memory().await.unwrap();
    let page_id = Uuid::now_v7();
    let block_id = Uuid::now_v7();
    apply(
        &conn,
        vec![
            page(page_id, "Page"),
            block(block_id, page_id, None, "same"),
        ],
        ProjectionPolicy::Deferred,
    )
    .await;
    let before = db::get_block(&conn, block_id).await.unwrap().unwrap();
    let changes = apply(
        &conn,
        vec![OpKind::BlockSetMarkdown(BlockSetMarkdown {
            uuid: block_id,
            markdown: "same".into(),
        })],
        ProjectionPolicy::Deferred,
    )
    .await;
    let after = db::get_block(&conn, block_id).await.unwrap().unwrap();
    assert_ne!(before.markdown_revision, after.markdown_revision);
    assert!(changes.changed_pages.is_empty());
    assert_eq!(changes.affected_pages, [page_id]);
    assert_eq!(changes.blocks.len(), 1);
    assert_eq!(changes.blocks[0].uuid, block_id);
    assert_eq!(changes.blocks[0].before, changes.blocks[0].after);
    assert!(changes.graph_changed_content.is_empty());
    assert!(!changes.is_empty());
    let no_op = apply(
        &conn,
        vec![OpKind::BlockSetStyle(BlockSetStyle {
            uuid: Uuid::now_v7(),
            style: BlockStyle::Quote,
        })],
        ProjectionPolicy::Deferred,
    )
    .await;
    assert!(no_op.is_empty());
}

#[tokio::test]
async fn hidden_lww_clock_updates_do_not_invalidate_identical_observable_rows() {
    let conn = db::open_in_memory().await.unwrap();
    let page_id = Uuid::now_v7();
    let block_id = Uuid::now_v7();
    apply(
        &conn,
        vec![
            page(page_id, "Page"),
            block(block_id, page_id, None, "Block"),
        ],
        ProjectionPolicy::Deferred,
    )
    .await;
    conn.call(move |db| {
        // Keep MAX(updated_at, timestamp) fixed independently of wall-clock speed.
        db.execute(
            "UPDATE pages SET updated_at = ?2 WHERE uuid = ?1",
            rusqlite::params![page_id, i64::MAX],
        )?;
        db.execute(
            "UPDATE blocks SET updated_at = ?2 WHERE uuid = ?1",
            rusqlite::params![block_id, i64::MAX],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let before_page = db::get_page(&conn, page_id).await.unwrap();
    let before_block = db::get_block(&conn, block_id).await.unwrap();
    let changes = apply(
        &conn,
        vec![
            OpKind::PageSetLayout(PageSetLayout {
                uuid: page_id,
                layout: PageLayout::Outline,
            }),
            OpKind::BlockSetStyle(BlockSetStyle {
                uuid: block_id,
                style: BlockStyle::Bullet,
            }),
            OpKind::BlockMove(BlockMove {
                uuid: block_id,
                page_uuid: page_id,
                parent_uuid: None,
                order_key: OrderKey::first(),
            }),
        ],
        ProjectionPolicy::Deferred,
    )
    .await;
    assert_eq!(before_page, db::get_page(&conn, page_id).await.unwrap());
    assert_eq!(before_block, db::get_block(&conn, block_id).await.unwrap());
    assert!(changes.is_empty(), "{changes:?}");
}

#[tokio::test]
async fn new_references_and_title_conflict_victims_are_reported() {
    let conn = db::open_in_memory().await.unwrap();
    let source = Uuid::now_v7();
    let victim = Uuid::now_v7();
    let winner = Uuid::now_v7();
    let block_id = Uuid::now_v7();
    apply(
        &conn,
        vec![
            page(source, "Source"),
            page(victim, "Target"),
            page(winner, "Other"),
        ],
        ProjectionPolicy::Deferred,
    )
    .await;
    let created = apply(
        &conn,
        vec![block(block_id, source, None, "[[Target]]")],
        ProjectionPolicy::Deferred,
    )
    .await;
    assert_eq!(created.graph_changed_content, [block_id]);
    assert_eq!(created.blocks[0].before, None);
    assert!(created.blocks[0].after.is_some());
    assert!(created.changed_pages.is_empty());
    let changed = apply(
        &conn,
        vec![OpKind::PageSetTitle(PageSetTitle {
            uuid: winner,
            title: Some("Target".into()),
        })],
        ProjectionPolicy::Deferred,
    )
    .await;
    assert_eq!(changed.changed_pages, sorted([victim, winner]));
    assert_eq!(
        changed.graph_changed_content,
        sorted([victim, winner, block_id])
    );
    assert_eq!(changed.affected_pages, sorted([victim, winner, source]));
    assert!(changed.blocks.is_empty());
    assert_eq!(
        db::get_page(&conn, victim).await.unwrap().unwrap().title,
        None
    );
}

#[tokio::test]
async fn deleting_parent_captures_child_before_foreign_key_detachment() {
    let conn = db::open_in_memory().await.unwrap();
    let page_id = Uuid::now_v7();
    let parent = Uuid::now_v7();
    let child = Uuid::now_v7();
    let sibling = Uuid::now_v7();
    apply(
        &conn,
        vec![
            page(page_id, "Page"),
            block(parent, page_id, None, "parent"),
            block(child, page_id, Some(parent), "child"),
            block(sibling, page_id, None, "sibling"),
        ],
        ProjectionPolicy::Deferred,
    )
    .await;
    let changes = apply(
        &conn,
        vec![OpKind::BlockDelete(BlockDelete {
            uuid: parent,
            page_uuid: page_id,
        })],
        ProjectionPolicy::Deferred,
    )
    .await;
    assert_eq!(
        changes
            .blocks
            .iter()
            .map(|change| change.uuid)
            .collect::<Vec<_>>(),
        sorted([parent, child])
    );
    let parent_change = changes
        .blocks
        .iter()
        .find(|change| change.uuid == parent)
        .unwrap();
    assert!(parent_change.before.is_some());
    assert!(parent_change.after.is_none());
    let child_change = changes
        .blocks
        .iter()
        .find(|change| change.uuid == child)
        .unwrap();
    assert_eq!(
        child_change.before.as_ref().unwrap().parent_uuid,
        Some(parent)
    );
    assert_eq!(child_change.after.as_ref().unwrap().parent_uuid, None);
    assert_eq!(changes.affected_pages, [page_id]);
    assert!(changes.changed_pages.is_empty());
}

#[tokio::test]
async fn indirect_structure_changes_include_both_page_containers() {
    let conn = db::open_in_memory().await.unwrap();
    let source = Uuid::now_v7();
    let destination = Uuid::now_v7();
    let parent = Uuid::now_v7();
    let child = Uuid::now_v7();
    apply(
        &conn,
        vec![
            page(source, "Source"),
            page(destination, "Destination"),
            block(parent, source, None, "parent"),
            block(child, source, Some(parent), "child"),
        ],
        ProjectionPolicy::Deferred,
    )
    .await;
    let changes = apply(
        &conn,
        vec![OpKind::BlockMove(BlockMove {
            uuid: parent,
            page_uuid: destination,
            parent_uuid: None,
            order_key: OrderKey::first(),
        })],
        ProjectionPolicy::Deferred,
    )
    .await;
    assert_eq!(
        changes
            .blocks
            .iter()
            .map(|change| change.uuid)
            .collect::<Vec<_>>(),
        sorted([parent, child])
    );
    let child_change = changes
        .blocks
        .iter()
        .find(|change| change.uuid == child)
        .unwrap();
    assert_eq!(
        child_change.before.as_ref().unwrap().parent_uuid,
        Some(parent)
    );
    assert_eq!(child_change.after.as_ref().unwrap().parent_uuid, None);
    assert_eq!(changes.affected_pages, sorted([source, destination]));
    assert_eq!(changes.graph_changed_content, sorted([parent, child]));
    assert!(changes.changed_pages.is_empty());
}

#[tokio::test]
async fn graph_labels_and_same_page_containment_are_actual_graph_changes() {
    let conn = db::open_in_memory().await.unwrap();
    let page_id = Uuid::now_v7();
    let parent = Uuid::now_v7();
    let child = Uuid::now_v7();
    apply(
        &conn,
        vec![
            page(page_id, "Page"),
            block(parent, page_id, None, "Parent"),
            block(child, page_id, None, &"a".repeat(80)),
        ],
        ProjectionPolicy::Deferred,
    )
    .await;
    let moved = apply(
        &conn,
        vec![OpKind::BlockMove(BlockMove {
            uuid: child,
            page_uuid: page_id,
            parent_uuid: Some(parent),
            order_key: OrderKey::first(),
        })],
        ProjectionPolicy::Deferred,
    )
    .await;
    assert_eq!(moved.graph_changed_content, [child]);
    let suffix_only = apply(
        &conn,
        vec![OpKind::BlockSetMarkdown(BlockSetMarkdown {
            uuid: child,
            markdown: format!("{} suffix", "a".repeat(80)),
        })],
        ProjectionPolicy::Deferred,
    )
    .await;
    assert!(suffix_only.graph_changed_content.is_empty());
    let label = apply(
        &conn,
        vec![OpKind::BlockSetMarkdown(BlockSetMarkdown {
            uuid: child,
            markdown: format!("b{} suffix", "a".repeat(79)),
        })],
        ProjectionPolicy::Deferred,
    )
    .await;
    assert_eq!(label.graph_changed_content, [child]);
}

#[tokio::test]
async fn page_cascade_tracks_incoming_edges_and_attachment_owners() {
    let conn = db::open_in_memory().await.unwrap();
    let page_id = Uuid::now_v7();
    let source_page = Uuid::now_v7();
    let deleted_block = Uuid::now_v7();
    let source_block = Uuid::now_v7();
    let hash = "12".repeat(32).parse().unwrap();
    let attachment = |owner| {
        OpKind::AttachmentAdd(AttachmentAdd {
            owner,
            blob_hash: hash,
            filename: "image.png".into(),
            mime: "image/png".into(),
            size: 42,
        })
    };
    apply(
        &conn,
        vec![
            page(page_id, "Target"),
            page(source_page, "Source"),
            block(deleted_block, page_id, None, "[[Source]]"),
            block(source_block, source_page, None, "[[Target]]"),
            attachment(AttachmentOwner::Page(page_id)),
            attachment(AttachmentOwner::Block(deleted_block)),
        ],
        ProjectionPolicy::Deferred,
    )
    .await;
    let changes = apply(
        &conn,
        vec![OpKind::PageDelete(PageDelete { uuid: page_id })],
        ProjectionPolicy::Deferred,
    )
    .await;
    assert_eq!(changes.deleted_pages, [page_id]);
    assert!(changes.changed_pages.is_empty());
    assert_eq!(
        changes
            .blocks
            .iter()
            .map(|change| change.uuid)
            .collect::<Vec<_>>(),
        [deleted_block]
    );
    assert_eq!(
        changes.graph_changed_content,
        sorted([page_id, deleted_block, source_block])
    );
    assert_eq!(changes.attachment_owners, sorted([page_id, deleted_block]));
    assert_eq!(changes.affected_pages, sorted([page_id, source_page]));
}

#[tokio::test]
async fn alias_retargeting_changes_graph_without_mutating_source_markdown() {
    let conn = db::open_in_memory().await.unwrap();
    let source = Uuid::now_v7();
    let target = Uuid::now_v7();
    let source_block = Uuid::now_v7();
    apply(
        &conn,
        vec![
            page(source, "Source"),
            page(target, "Target"),
            block(source_block, source, None, "[[alias]]"),
        ],
        ProjectionPolicy::Deferred,
    )
    .await;
    let before = db::get_block(&conn, source_block).await.unwrap();
    let changes = apply(
        &conn,
        vec![OpKind::PageAliasSet(PageAliasSet {
            uuid: target,
            alias: "alias".parse().unwrap(),
            present: true,
        })],
        ProjectionPolicy::Deferred,
    )
    .await;
    assert_eq!(changes.graph_changed_content, [source_block]);
    assert_eq!(changes.affected_pages, [source]);
    assert!(changes.changed_pages.is_empty());
    assert!(changes.blocks.is_empty());
    assert_eq!(before, db::get_block(&conn, source_block).await.unwrap());
}

#[tokio::test]
async fn creating_owner_materializes_delayed_attachment_without_fake_early_change() {
    let conn = db::open_in_memory().await.unwrap();
    let page_id = Uuid::now_v7();
    let block_id = Uuid::now_v7();
    apply(
        &conn,
        vec![page(page_id, "Page")],
        ProjectionPolicy::Deferred,
    )
    .await;
    let pending = apply(
        &conn,
        vec![OpKind::AttachmentAdd(AttachmentAdd {
            owner: AttachmentOwner::Block(block_id),
            blob_hash: "56".repeat(32).parse().unwrap(),
            filename: "delayed.png".into(),
            mime: "image/png".into(),
            size: 42,
        })],
        ProjectionPolicy::Deferred,
    )
    .await;
    assert!(pending.is_empty());
    let materialized = apply(
        &conn,
        vec![block(block_id, page_id, None, "Block")],
        ProjectionPolicy::Deferred,
    )
    .await;
    assert_eq!(materialized.attachment_owners, [block_id]);
    assert_eq!(materialized.affected_pages, [page_id]);
    assert!(materialized.changed_pages.is_empty());
}

#[tokio::test]
async fn projection_policies_produce_identical_exact_change_sets() {
    let source = Uuid::from_u128(1);
    let destination = Uuid::from_u128(2);
    let parent = Uuid::from_u128(3);
    let child = Uuid::from_u128(4);
    let referrer = Uuid::from_u128(5);
    let mut results = Vec::new();
    for policy in [ProjectionPolicy::Immediate, ProjectionPolicy::Deferred] {
        let conn = db::open_in_memory().await.unwrap();
        apply(
            &conn,
            vec![
                page(source, "Source"),
                page(destination, "Destination"),
                block(parent, source, None, "Parent"),
                block(child, source, Some(parent), "Child"),
                block(referrer, destination, None, "[[alias]]"),
            ],
            policy,
        )
        .await;
        results.push(
            apply(
                &conn,
                vec![
                    OpKind::BlockMove(BlockMove {
                        uuid: parent,
                        page_uuid: destination,
                        parent_uuid: None,
                        order_key: OrderKey::first(),
                    }),
                    OpKind::PageAliasSet(PageAliasSet {
                        uuid: source,
                        alias: "alias".parse().unwrap(),
                        present: true,
                    }),
                    OpKind::BlockSetMarkdown(BlockSetMarkdown {
                        uuid: referrer,
                        markdown: "[[alias]] and [[Source]]".into(),
                    }),
                ],
                policy,
            )
            .await,
        );
    }
    assert_eq!(results[0], results[1]);
}

#[tokio::test]
async fn page_generation_replacement_reports_deleted_blocks_and_lost_attachments() {
    let conn = db::open_in_memory().await.unwrap();
    let page_id = Uuid::now_v7();
    let parent = Uuid::now_v7();
    let child = Uuid::now_v7();
    apply(
        &conn,
        vec![
            page(page_id, "Page"),
            block(parent, page_id, None, "parent"),
            block(child, page_id, Some(parent), "child"),
        ],
        ProjectionPolicy::Deferred,
    )
    .await;
    let changes = apply(
        &conn,
        vec![page(page_id, "Recreated")],
        ProjectionPolicy::Deferred,
    )
    .await;
    assert_eq!(changes.changed_pages, [page_id]);
    assert!(changes.deleted_pages.is_empty());
    assert_eq!(
        changes
            .blocks
            .iter()
            .map(|change| change.uuid)
            .collect::<Vec<_>>(),
        sorted([parent, child])
    );
    assert!(
        changes
            .blocks
            .iter()
            .all(|change| change.before.is_some() && change.after.is_none())
    );
}

#[tokio::test]
async fn net_create_delete_and_absent_owners_do_not_emit_fake_entities() {
    let conn = db::open_in_memory().await.unwrap();
    let page_id = Uuid::now_v7();
    let temporary = Uuid::now_v7();
    apply(
        &conn,
        vec![page(page_id, "Page")],
        ProjectionPolicy::Deferred,
    )
    .await;
    let changes = apply(
        &conn,
        vec![
            block(temporary, page_id, None, "[[Target]]"),
            OpKind::BlockDelete(BlockDelete {
                uuid: temporary,
                page_uuid: page_id,
            }),
            OpKind::AttachmentRemove(AttachmentRemove {
                owner: AttachmentOwner::Block(Uuid::now_v7()),
                blob_hash: "34".repeat(32).parse().unwrap(),
            }),
        ],
        ProjectionPolicy::Deferred,
    )
    .await;
    assert!(changes.is_empty(), "{changes:?}");
}

#[tokio::test]
async fn read_barriers_preserve_initial_placement_until_final_delete() {
    let conn = db::open_in_memory().await.unwrap();
    let source = Uuid::now_v7();
    let destination = Uuid::now_v7();
    let block_id = Uuid::now_v7();
    apply(
        &conn,
        vec![
            page(source, "Source"),
            page(destination, "Destination"),
            block(block_id, source, None, "Block"),
        ],
        ProjectionPolicy::Deferred,
    )
    .await;
    let changes = apply(
        &conn,
        vec![
            OpKind::BlockMove(BlockMove {
                uuid: block_id,
                page_uuid: destination,
                parent_uuid: None,
                order_key: OrderKey::first(),
            }),
            OpKind::PageDelete(PageDelete { uuid: destination }),
        ],
        ProjectionPolicy::Deferred,
    )
    .await;
    assert_eq!(changes.blocks.len(), 1);
    assert_eq!(changes.blocks[0].before.as_ref().unwrap().page_uuid, source);
    assert!(changes.blocks[0].after.is_none());
    assert_eq!(changes.deleted_pages, [destination]);
    assert_eq!(changes.affected_pages, sorted([source, destination]));
}

#[test]
fn change_set_serialization_and_empty_contract_include_all_categories() {
    let page_id = Uuid::from_u128(1);
    let uuid = Uuid::from_u128(2);
    let placement = |order_key| {
        Some(BlockPlacement {
            page_uuid: page_id,
            parent_uuid: None,
            order_key,
        })
    };
    let first = placement(OrderKey::first());
    let last = placement(OrderKey::first().after().after());
    let changes = ChangeSet {
        blocks: vec![BlockChange {
            uuid,
            before: first.clone(),
            after: last.clone(),
        }],
        affected_pages: vec![page_id],
        history_changed: true,
        ..ChangeSet::default()
    };
    assert_eq!(
        changes.blocks,
        [BlockChange {
            uuid,
            before: first,
            after: last
        }]
    );
    assert_eq!(changes.affected_pages, [page_id]);
    assert!(changes.history_changed);
    let wire = serde_json::to_value(&changes).unwrap();
    assert!(wire.get("affectedPages").is_some());
    assert!(wire["blocks"][0]["before"].get("pageUuid").is_some());
    assert_eq!(serde_json::from_value::<ChangeSet>(wire).unwrap(), changes);
    assert!(ChangeSet::default().is_empty());
    assert!(
        !ChangeSet {
            history_changed: true,
            ..ChangeSet::default()
        }
        .is_empty()
    );
}
