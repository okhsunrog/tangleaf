use super::*;
use crate::operation::{BlockCreate, BlockDelete, BlockMove, BlockSetMarkdown, BlockSetStyle};
use rusqlite::OptionalExtension;

#[derive(Debug, Clone, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BlockContent {
    pub markdown: String,
}

pub(crate) fn next_append_order_key(last: Option<&OrderKey>) -> crate::CoreResult<OrderKey> {
    let Some(last) = last else {
        return Ok(OrderKey::first());
    };
    let next = last.after();
    if next == *last {
        return Err(crate::CoreError::conflict(
            "sibling order-key space is exhausted",
        ));
    }
    Ok(next)
}

pub async fn get_block(conn: &Connection, uuid: uuid::Uuid) -> Result<Option<Block>> {
    conn.call(move |database| {
        let sql = format!("SELECT {BLOCK_COLUMNS} FROM blocks WHERE uuid = ?1");
        database.query_row(&sql, [uuid], row_to_block).optional()
    })
    .await
}

pub async fn get_blocks(conn: &Connection, uuids: Vec<uuid::Uuid>) -> Result<Vec<Block>> {
    if uuids.is_empty() {
        return Ok(Vec::new());
    }
    conn.call(move |database| {
        let transaction = database.transaction()?;
        let by_uuid = read_blocks_by_uuid(&transaction, &uuids)?;
        let blocks = uuids
            .into_iter()
            .filter_map(|uuid| by_uuid.get(&uuid).cloned())
            .collect();
        transaction.commit()?;
        Ok(blocks)
    })
    .await
}

/// Shared hydration for callers that already own a read transaction.
pub(super) fn read_blocks_by_uuid(
    transaction: &rusqlite::Transaction<'_>,
    uuids: &[uuid::Uuid],
) -> rusqlite::Result<std::collections::HashMap<uuid::Uuid, Block>> {
    let mut unique = uuids.to_vec();
    unique.sort_unstable();
    unique.dedup();
    let mut by_uuid = std::collections::HashMap::with_capacity(unique.len());
    for chunk in unique.chunks(UUID_READ_CHUNK_SIZE) {
        let placeholders = std::iter::repeat_n("?", chunk.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!("SELECT {BLOCK_COLUMNS} FROM blocks WHERE uuid IN ({placeholders})");
        let mut statement = transaction.prepare_cached(&sql)?;
        for block in statement.query_map(rusqlite::params_from_iter(chunk.iter()), row_to_block)? {
            let block = block?;
            by_uuid.insert(block.uuid, block);
        }
    }
    Ok(by_uuid)
}

pub async fn list_block_children(
    conn: &Connection,
    page_uuid: uuid::Uuid,
    parent_uuid: Option<uuid::Uuid>,
) -> Result<Vec<Block>> {
    conn.call(move |database| read_block_children(database, page_uuid, parent_uuid))
        .await
}

fn read_block_children(
    database: &rusqlite::Connection,
    page_uuid: uuid::Uuid,
    parent_uuid: Option<uuid::Uuid>,
) -> rusqlite::Result<Vec<Block>> {
    let sql = format!(
        "SELECT {BLOCK_COLUMNS} FROM blocks
              WHERE page_uuid = ?1 AND parent_uuid IS ?2
              ORDER BY order_key, uuid"
    );
    database
        .prepare(&sql)?
        .query_map(rusqlite::params![page_uuid, parent_uuid], row_to_block)?
        .collect()
}

#[derive(Clone)]
struct SiblingPosition {
    uuid: uuid::Uuid,
    order_key: OrderKey,
}

#[derive(Clone, Copy)]
enum SiblingInsertion {
    End,
    Before(uuid::Uuid),
    After(uuid::Uuid),
}

fn row_to_sibling_position(row: &rusqlite::Row<'_>) -> rusqlite::Result<SiblingPosition> {
    Ok(SiblingPosition {
        uuid: row.get(0)?,
        order_key: row.get(1)?,
    })
}

/// An indexed seek returns one neighboring anchor, never sibling Markdown.
fn sibling_neighbor(
    transaction: &rusqlite::Transaction<'_>,
    page_uuid: uuid::Uuid,
    parent_uuid: Option<uuid::Uuid>,
    anchor: Option<&SiblingPosition>,
    backwards: bool,
    excluded_uuid: Option<uuid::Uuid>,
) -> rusqlite::Result<Option<SiblingPosition>> {
    let comparison = if backwards { "<" } else { ">" };
    let order = if backwards { "DESC" } else { "ASC" };
    let bound = if anchor.is_some() {
        format!(" AND (order_key, uuid) {comparison} (?4, ?5)")
    } else {
        String::new()
    };
    let sql = format!(
        "SELECT uuid, order_key FROM blocks
          WHERE page_uuid = ?1 AND parent_uuid IS ?2 AND uuid IS NOT ?3{bound}
          ORDER BY order_key {order}, uuid {order} LIMIT 1"
    );
    if let Some(anchor) = anchor {
        transaction.query_row(
            &sql,
            rusqlite::params![
                page_uuid,
                parent_uuid,
                excluded_uuid,
                anchor.order_key,
                anchor.uuid
            ],
            row_to_sibling_position,
        )
    } else {
        transaction.query_row(
            &sql,
            rusqlite::params![page_uuid, parent_uuid, excluded_uuid],
            row_to_sibling_position,
        )
    }
    .optional()
}

fn allocate_sibling_keys(
    lower: Option<&OrderKey>,
    upper: Option<&OrderKey>,
    count: usize,
) -> Option<Vec<OrderKey>> {
    if upper.is_some() {
        return OrderKey::between(lower, upper, count);
    }
    let Some(lower) = lower else {
        return Some((1..=count).map(OrderKey::from_ordinal).collect());
    };
    let mut previous = lower.clone();
    (0..count)
        .map(|_| {
            let next = previous.after();
            if next == previous {
                return None;
            }
            previous = next.clone();
            Some(next)
        })
        .collect()
}

/// Reserve a gap for created or moved siblings. Existing siblings are loaded
/// only when that precise gap is tied/exhausted and must be rebalanced.
fn sibling_placement(
    transaction: &rusqlite::Transaction<'_>,
    page_uuid: uuid::Uuid,
    parent_uuid: Option<uuid::Uuid>,
    insertion: SiblingInsertion,
    excluded_uuid: Option<uuid::Uuid>,
    count: usize,
    invalid_anchor: crate::CoreError,
) -> crate::CoreResult<(Vec<OrderKey>, Vec<OpKind>)> {
    if count == 0 {
        return Ok((Vec::new(), Vec::new()));
    }
    let (lower, upper) = match insertion {
        SiblingInsertion::End => (
            sibling_neighbor(
                transaction,
                page_uuid,
                parent_uuid,
                None,
                true,
                excluded_uuid,
            )?,
            None,
        ),
        SiblingInsertion::Before(uuid) | SiblingInsertion::After(uuid) => {
            let anchor = transaction
                .query_row(
                    "SELECT uuid, order_key FROM blocks
                  WHERE uuid = ?1 AND page_uuid = ?2 AND parent_uuid IS ?3 AND uuid IS NOT ?4",
                    rusqlite::params![uuid, page_uuid, parent_uuid, excluded_uuid],
                    row_to_sibling_position,
                )
                .optional()?
                .ok_or(invalid_anchor)?;
            if matches!(insertion, SiblingInsertion::After(_)) {
                let upper = sibling_neighbor(
                    transaction,
                    page_uuid,
                    parent_uuid,
                    Some(&anchor),
                    false,
                    excluded_uuid,
                )?;
                (Some(anchor), upper)
            } else {
                let lower = sibling_neighbor(
                    transaction,
                    page_uuid,
                    parent_uuid,
                    Some(&anchor),
                    true,
                    excluded_uuid,
                )?;
                (lower, Some(anchor))
            }
        }
    };
    if let Some(keys) = allocate_sibling_keys(
        lower.as_ref().map(|anchor| &anchor.order_key),
        upper.as_ref().map(|anchor| &anchor.order_key),
        count,
    ) {
        return Ok((keys, Vec::new()));
    }

    let siblings = transaction
        .prepare(
            "SELECT uuid, order_key FROM blocks
          WHERE page_uuid = ?1 AND parent_uuid IS ?2 AND uuid IS NOT ?3
          ORDER BY order_key, uuid",
        )?
        .query_map(
            rusqlite::params![page_uuid, parent_uuid, excluded_uuid],
            row_to_sibling_position,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let insertion_index = match insertion {
        SiblingInsertion::End => siblings.len(),
        SiblingInsertion::Before(uuid) | SiblingInsertion::After(uuid) => {
            siblings
                .iter()
                .position(|sibling| sibling.uuid == uuid)
                .ok_or_else(|| crate::CoreError::conflict("placement anchor disappeared"))?
                + usize::from(matches!(insertion, SiblingInsertion::After(_)))
        }
    };
    let moves = siblings
        .into_iter()
        .enumerate()
        .filter_map(|(index, sibling)| {
            let ordinal = index + 1 + if index >= insertion_index { count } else { 0 };
            let order_key = OrderKey::from_ordinal(ordinal);
            (order_key != sibling.order_key).then_some(OpKind::BlockMove(BlockMove {
                uuid: sibling.uuid,
                page_uuid,
                parent_uuid,
                order_key,
            }))
        })
        .collect();
    let keys = (1..=count)
        .map(|offset| OrderKey::from_ordinal(insertion_index + offset))
        .collect();
    Ok((keys, moves))
}

pub async fn create_block(
    conn: &Connection,
    page_uuid: uuid::Uuid,
    parent_uuid: Option<uuid::Uuid>,
    after_uuid: Option<uuid::Uuid>,
    style: BlockStyle,
    markdown: String,
) -> Result<Block> {
    Ok(
        create_block_with_ops(conn, page_uuid, parent_uuid, after_uuid, style, markdown)
            .await?
            .value,
    )
}

#[doc(hidden)]
pub async fn create_block_with_ops(
    conn: &Connection,
    page_uuid: uuid::Uuid,
    parent_uuid: Option<uuid::Uuid>,
    after_uuid: Option<uuid::Uuid>,
    style: BlockStyle,
    markdown: String,
) -> Result<AppliedMutation<Block>> {
    let committed = crate::store::NotesStore::new(conn.clone())
        .create_block(page_uuid, parent_uuid, after_uuid, style, markdown)
        .await?;
    Ok(AppliedMutation {
        value: committed.value,
        operations: committed.operations,
    })
}

pub async fn set_block_content(
    conn: &Connection,
    uuid: uuid::Uuid,
    content: BlockContent,
) -> Result<(Block, bool)> {
    Ok(crate::store::NotesStore::new(conn.clone())
        .set_block_content(uuid, content)
        .await?
        .value)
}

/// Update block Markdown only when the caller still represents its current
/// persisted content. Equal stale writes are accepted as idempotent success.
pub async fn set_block_content_if_revision(
    conn: &Connection,
    uuid: uuid::Uuid,
    content: BlockContent,
    expected_revision: ContentRevision,
) -> Result<(Block, bool, bool)> {
    let applied =
        set_block_content_if_revision_with_ops(conn, uuid, content, expected_revision).await?;
    let changed = !applied.operations.is_empty();
    Ok((applied.value.0, changed, applied.value.1))
}

#[doc(hidden)]
#[tracing::instrument(level = "debug", skip_all, fields(markdown_bytes = content.markdown.len()))]
pub async fn set_block_content_if_revision_with_ops(
    conn: &Connection,
    uuid: uuid::Uuid,
    content: BlockContent,
    expected_revision: ContentRevision,
) -> Result<AppliedMutation<(Block, bool)>> {
    let committed = crate::store::NotesStore::new(conn.clone())
        .set_block_content_if_revision(uuid, content, expected_revision)
        .await?;
    Ok(AppliedMutation {
        value: committed.value,
        operations: committed.operations,
    })
}

pub async fn set_block_style(
    conn: &Connection,
    uuid: uuid::Uuid,
    style: BlockStyle,
) -> Result<Block> {
    Ok(set_block_style_with_ops(conn, uuid, style).await?.value)
}

#[doc(hidden)]
pub async fn set_block_style_with_ops(
    conn: &Connection,
    uuid: uuid::Uuid,
    style: BlockStyle,
) -> Result<AppliedMutation<Block>> {
    let committed = crate::store::NotesStore::new(conn.clone())
        .set_block_style(uuid, style)
        .await?;
    Ok(AppliedMutation {
        value: committed.value,
        operations: committed.operations,
    })
}

pub async fn set_task_state(
    conn: &Connection,
    uuid: uuid::Uuid,
    state: TaskState,
) -> Result<Block> {
    Ok(set_task_state_with_ops(conn, uuid, state).await?.value)
}

#[doc(hidden)]
pub async fn set_task_state_with_ops(
    conn: &Connection,
    uuid: uuid::Uuid,
    state: TaskState,
) -> Result<AppliedMutation<Block>> {
    let committed = crate::store::NotesStore::new(conn.clone())
        .set_task_state(uuid, state)
        .await?;
    Ok(AppliedMutation {
        value: committed.value,
        operations: committed.operations,
    })
}

fn require_block(
    transaction: &rusqlite::Transaction<'_>,
    uuid: uuid::Uuid,
) -> crate::CoreResult<Block> {
    read_block(transaction, uuid)?.ok_or_else(|| crate::CoreError::not_found("block was not found"))
}

fn read_block(
    transaction: &rusqlite::Transaction<'_>,
    uuid: uuid::Uuid,
) -> rusqlite::Result<Option<Block>> {
    let sql = format!("SELECT {BLOCK_COLUMNS} FROM blocks WHERE uuid = ?1");
    transaction.query_row(&sql, [uuid], row_to_block).optional()
}

fn apply_block_style(
    write: &mut crate::store::NotesTransaction<'_, '_>,
    block: Block,
    style: BlockStyle,
) -> crate::CoreResult<Block> {
    if block.style == style {
        return Ok(block);
    }
    write.apply_action(
        "set block style",
        vec![OpKind::BlockSetStyle(BlockSetStyle {
            uuid: block.uuid,
            style,
        })],
        operation::ProjectionPolicy::Immediate,
    )?;
    require_block(write.sql(), block.uuid)
}

fn move_block_in_transaction(
    write: &mut crate::store::NotesTransaction<'_, '_>,
    block: Block,
    new_parent_uuid: Option<uuid::Uuid>,
    after_uuid: Option<uuid::Uuid>,
) -> Result<Block> {
    let transaction = write.sql();
    if new_parent_uuid == Some(block.uuid) {
        return Err(crate::CoreError::conflict("a block cannot be its own parent").into());
    }
    if let Some(parent) = new_parent_uuid {
        let parent_block = read_block(transaction, parent)?
            .ok_or_else(|| crate::CoreError::not_found("parent block was not found"))?;
        if parent_block.page_uuid != block.page_uuid {
            return Err(
                crate::CoreError::conflict("parent block belongs to a different page").into(),
            );
        }
        let creates_cycle = transaction.query_row(
            "WITH RECURSIVE descendants(uuid) AS (
               SELECT uuid FROM blocks WHERE parent_uuid = ?1
               UNION ALL
               SELECT block.uuid FROM blocks block
                 JOIN descendants child ON block.parent_uuid = child.uuid
             )
             SELECT EXISTS(SELECT 1 FROM descendants WHERE uuid = ?2)",
            rusqlite::params![block.uuid, parent],
            |row| row.get::<_, bool>(0),
        )?;
        if creates_cycle {
            return Err(crate::CoreError::conflict(
                "a block cannot be moved under one of its descendants",
            )
            .into());
        }
    }
    let insertion = after_uuid.map_or(SiblingInsertion::End, SiblingInsertion::After);
    let (keys, mut kinds) = sibling_placement(
        transaction,
        block.page_uuid,
        new_parent_uuid,
        insertion,
        Some(block.uuid),
        1,
        crate::CoreError::invalid("after block is not a sibling of the moved block"),
    )?;
    kinds.insert(
        0,
        OpKind::BlockMove(BlockMove {
            uuid: block.uuid,
            page_uuid: block.page_uuid,
            parent_uuid: new_parent_uuid,
            order_key: keys[0].clone(),
        }),
    );
    write.apply_action("move block", kinds, operation::ProjectionPolicy::Immediate)?;
    Ok(require_block(write.sql(), block.uuid)?)
}

impl crate::store::NotesStore {
    pub async fn create_block(
        &self,
        page_uuid: uuid::Uuid,
        parent_uuid: Option<uuid::Uuid>,
        after_uuid: Option<uuid::Uuid>,
        style: BlockStyle,
        markdown: String,
    ) -> Result<crate::store::Committed<Block>> {
        self.write(move |write| -> crate::CoreResult<Block> {
            let transaction = write.sql();
            let page_exists = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM pages WHERE uuid = ?1)",
                [page_uuid],
                |row| row.get::<_, bool>(0),
            )?;
            if !page_exists {
                return Err(crate::CoreError::not_found("page was not found"));
            }
            if let Some(parent_uuid) = parent_uuid {
                let parent = read_block(transaction, parent_uuid)?
                    .ok_or_else(|| crate::CoreError::not_found("parent block was not found"))?;
                if parent.page_uuid != page_uuid {
                    return Err(crate::CoreError::conflict(
                        "parent block belongs to a different page",
                    ));
                }
            }
            let insertion = after_uuid.map_or(SiblingInsertion::End, SiblingInsertion::After);
            let (keys, moves) = sibling_placement(
                transaction,
                page_uuid,
                parent_uuid,
                insertion,
                None,
                1,
                crate::CoreError::invalid("after block is not a sibling of the new block"),
            )?;
            let uuid = uuid::Uuid::now_v7();
            let now = chrono::Utc::now().timestamp();
            let mut kinds = vec![OpKind::BlockCreate(BlockCreate {
                uuid,
                page_uuid,
                parent_uuid,
                order_key: keys[0].clone(),
                style,
                markdown,
                created_at: now,
            })];
            kinds.extend(moves);
            write.apply_action(
                "create block",
                kinds,
                operation::ProjectionPolicy::Immediate,
            )?;
            require_block(write.sql(), uuid)
        })
        .await
    }

    /// Legacy unchecked save, serialized with all other store commands.
    pub async fn set_block_content(
        &self,
        uuid: uuid::Uuid,
        content: BlockContent,
    ) -> Result<crate::store::Committed<(Block, bool)>> {
        self.save_block_content(uuid, content, None).await
    }

    /// Equal stale writes remain idempotent; different stale text conflicts.
    pub async fn set_block_content_if_revision(
        &self,
        uuid: uuid::Uuid,
        content: BlockContent,
        expected_revision: ContentRevision,
    ) -> Result<crate::store::Committed<(Block, bool)>> {
        self.save_block_content(uuid, content, Some(expected_revision))
            .await
    }

    #[tracing::instrument(level = "debug", skip_all, fields(markdown_bytes = content.markdown.len()))]
    async fn save_block_content(
        &self,
        uuid: uuid::Uuid,
        content: BlockContent,
        expected_revision: Option<ContentRevision>,
    ) -> Result<crate::store::Committed<(Block, bool)>> {
        self.write(move |write| -> crate::CoreResult<(Block, bool)> {
            let started = std::time::Instant::now();
            let block = require_block(write.sql(), uuid)?;
            if block.markdown == content.markdown {
                return Ok((block, false));
            }
            if expected_revision.is_some_and(|expected| block.markdown_revision != expected) {
                return Err(crate::CoreError::conflict(
                    "block content changed since editing began",
                ));
            }
            let graph_changed =
                operation::content_references_changed(&block.markdown, &content.markdown);
            let planning = started.elapsed();
            let applying = std::time::Instant::now();
            // Ordinary Markdown edits do not add structural undo entries.
            write.apply_local(
                vec![OpKind::BlockSetMarkdown(BlockSetMarkdown {
                    uuid,
                    markdown: content.markdown,
                })],
                operation::ProjectionPolicy::Immediate,
            )?;
            let apply = applying.elapsed();
            let reading = std::time::Instant::now();
            let block = require_block(write.sql(), uuid)?;
            tracing::debug!(
                planning_ms = planning.as_secs_f64() * 1000.0,
                apply_ms = apply.as_secs_f64() * 1000.0,
                result_ms = reading.elapsed().as_secs_f64() * 1000.0,
                "content command prepared"
            );
            Ok((block, graph_changed))
        })
        .await
    }

    pub async fn set_block_style(
        &self,
        uuid: uuid::Uuid,
        style: BlockStyle,
    ) -> Result<crate::store::Committed<Block>> {
        self.write(move |write| -> crate::CoreResult<Block> {
            let block = require_block(write.sql(), uuid)?;
            apply_block_style(write, block, style)
        })
        .await
    }

    pub async fn set_task_state(
        &self,
        uuid: uuid::Uuid,
        state: TaskState,
    ) -> Result<crate::store::Committed<Block>> {
        self.write(move |write| -> crate::CoreResult<Block> {
            let block = require_block(write.sql(), uuid)?;
            if block.style.task_state().is_none() {
                return Err(crate::CoreError::invalid(
                    "task state requires a task block",
                ));
            }
            apply_block_style(write, block, BlockStyle::task(state))
        })
        .await
    }

    pub async fn move_block(
        &self,
        uuid: uuid::Uuid,
        new_parent_uuid: Option<uuid::Uuid>,
        after_uuid: Option<uuid::Uuid>,
    ) -> Result<crate::store::Committed<Block>> {
        self.write(move |write| -> Result<Block> {
            let block = require_block(write.sql(), uuid)?;
            move_block_in_transaction(write, block, new_parent_uuid, after_uuid)
        })
        .await
    }

    pub async fn reorder_block(
        &self,
        uuid: uuid::Uuid,
        direction: ReorderDirection,
    ) -> Result<crate::store::Committed<Block>> {
        self.write(move |write| -> Result<Block> {
            let block = require_block(write.sql(), uuid)?;
            let source = SiblingPosition {
                uuid,
                order_key: block.order_key.clone(),
            };
            let backwards = direction == ReorderDirection::Up;
            let Some(neighbor) = sibling_neighbor(
                write.sql(),
                block.page_uuid,
                block.parent_uuid,
                Some(&source),
                backwards,
                None,
            )?
            else {
                return Ok(block);
            };
            let insertion = if backwards {
                SiblingInsertion::Before(neighbor.uuid)
            } else {
                SiblingInsertion::After(neighbor.uuid)
            };
            let (keys, mut kinds) = sibling_placement(
                write.sql(),
                block.page_uuid,
                block.parent_uuid,
                insertion,
                Some(uuid),
                1,
                crate::CoreError::conflict("block is absent from its parent"),
            )?;
            kinds.insert(
                0,
                OpKind::BlockMove(BlockMove {
                    uuid,
                    page_uuid: block.page_uuid,
                    parent_uuid: block.parent_uuid,
                    order_key: keys[0].clone(),
                }),
            );
            write.apply_action(
                "reorder block",
                kinds,
                operation::ProjectionPolicy::Immediate,
            )?;
            Ok(require_block(write.sql(), uuid)?)
        })
        .await
    }

    pub async fn move_block_in_direction(
        &self,
        uuid: uuid::Uuid,
        direction: ReorderDirection,
    ) -> Result<crate::store::Committed<Block>> {
        self.reorder_block(uuid, direction).await
    }

    pub async fn indent_block(&self, uuid: uuid::Uuid) -> Result<crate::store::Committed<Block>> {
        self.write(move |write| -> Result<Block> {
            let block = require_block(write.sql(), uuid)?;
            let source = SiblingPosition {
                uuid,
                order_key: block.order_key.clone(),
            };
            let Some(previous) = sibling_neighbor(
                write.sql(),
                block.page_uuid,
                block.parent_uuid,
                Some(&source),
                true,
                None,
            )?
            else {
                return Ok(block);
            };
            move_block_in_transaction(write, block, Some(previous.uuid), None)
        })
        .await
    }

    pub async fn outdent_block(&self, uuid: uuid::Uuid) -> Result<crate::store::Committed<Block>> {
        self.write(move |write| -> Result<Block> {
            let block = require_block(write.sql(), uuid)?;
            let parent_uuid = block.parent_uuid.ok_or_else(|| {
                crate::CoreError::conflict("top-level blocks cannot be outdented")
            })?;
            let parent =
                read_block(write.sql(), parent_uuid)?.context("parent block disappeared")?;
            move_block_in_transaction(write, block, parent.parent_uuid, Some(parent.uuid))
        })
        .await
    }

    /// Only a currently existing leaf can be removed by the outliner command.
    pub async fn delete_block(&self, uuid: uuid::Uuid) -> Result<crate::store::Committed<bool>> {
        self.write(move |write| -> crate::CoreResult<bool> {
            let Some(block) = read_block(write.sql(), uuid)? else {
                return Ok(false);
            };
            let child_count = write.sql().query_row(
                "SELECT COUNT(*) FROM blocks WHERE parent_uuid = ?1",
                [uuid],
                |row| row.get::<_, i64>(0),
            )?;
            if child_count != 0 {
                return Ok(false);
            }
            write.apply_action(
                "delete block",
                vec![OpKind::BlockDelete(BlockDelete {
                    uuid,
                    page_uuid: block.page_uuid,
                })],
                operation::ProjectionPolicy::Immediate,
            )?;
            Ok(true)
        })
        .await
    }
}

pub async fn split_block(
    conn: &Connection,
    uuid: uuid::Uuid,
    parts: Vec<BlockContent>,
    expected_revision: ContentRevision,
) -> Result<Vec<Block>> {
    Ok(split_block_with_ops(conn, uuid, parts, expected_revision)
        .await?
        .value
        .0)
}

fn split_placement(
    transaction: &rusqlite::Transaction<'_>,
    source: &Block,
    count: usize,
) -> crate::CoreResult<(Vec<OrderKey>, Vec<OpKind>)> {
    sibling_placement(
        transaction,
        source.page_uuid,
        source.parent_uuid,
        SiblingInsertion::After(source.uuid),
        None,
        count,
        crate::CoreError::conflict("split source is absent from its siblings"),
    )
}

#[doc(hidden)]
pub async fn split_block_with_ops(
    conn: &Connection,
    uuid: uuid::Uuid,
    parts: Vec<BlockContent>,
    expected_revision: ContentRevision,
) -> Result<AppliedMutation<(Vec<Block>, bool)>> {
    let committed = crate::store::NotesStore::new(conn.clone())
        .split_block(uuid, parts, expected_revision)
        .await?;
    let graph_changed = committed.changes.graph_changed_content.contains(&uuid);
    Ok(AppliedMutation {
        value: (committed.value, graph_changed),
        operations: committed.operations,
    })
}

impl crate::store::NotesStore {
    #[tracing::instrument(level = "debug", skip_all, fields(parts = parts.len()))]
    pub async fn split_block(
        &self,
        uuid: uuid::Uuid,
        parts: Vec<BlockContent>,
        expected_revision: ContentRevision,
    ) -> Result<crate::store::Committed<Vec<Block>>> {
        if parts.is_empty() {
            return Err(crate::CoreError::invalid("split requires at least one part").into());
        }
        self.write(move |write| -> crate::CoreResult<Vec<Block>> {
            let started = std::time::Instant::now();
            let transaction = write.sql();
            let block_sql = format!("SELECT {BLOCK_COLUMNS} FROM blocks WHERE uuid = ?1");
            let source = transaction
                .query_row(&block_sql, [uuid], row_to_block)
                .optional()?
                .ok_or_else(|| crate::CoreError::not_found("block was not found"))?;

            // A split is a non-idempotent structural intent: even when its first
            // part equals the current Markdown, retrying against a stale revision
            // would create another set of sibling blocks.
            if source.markdown_revision != expected_revision {
                return Err(crate::CoreError::conflict(
                    "block content changed since split began",
                ));
            }
            let (keys, moves) = split_placement(transaction, &source, parts.len() - 1)?;
            let now = chrono::Utc::now().timestamp();
            let mut result_uuids = vec![uuid];
            let mut kinds = vec![OpKind::BlockSetMarkdown(BlockSetMarkdown {
                uuid,
                markdown: parts[0].markdown.clone(),
            })];
            for (part, order_key) in parts.into_iter().skip(1).zip(keys) {
                let block_uuid = uuid::Uuid::now_v7();
                result_uuids.push(block_uuid);
                kinds.push(OpKind::BlockCreate(BlockCreate {
                    uuid: block_uuid,
                    page_uuid: source.page_uuid,
                    parent_uuid: source.parent_uuid,
                    order_key,
                    style: source.style,
                    markdown: part.markdown,
                    created_at: now,
                }));
            }
            kinds.extend(moves);
            let planning = started.elapsed();
            let applying = std::time::Instant::now();
            // Every inserted block has its final parent/key. The shared apply
            // finishes projections before the history layer captures guards.
            write.apply_action(
                "split block",
                kinds.clone(),
                operation::ProjectionPolicy::Deferred,
            )?;
            let apply_and_history = applying.elapsed();
            let reading = std::time::Instant::now();
            let blocks = result_uuids
                .into_iter()
                .map(|uuid| write.sql().query_row(&block_sql, [uuid], row_to_block))
                .collect::<rusqlite::Result<Vec<_>>>()?;
            tracing::debug!(
                operations = kinds.len(),
                planning_ms = planning.as_secs_f64() * 1000.0,
                apply_and_history_ms = apply_and_history.as_secs_f64() * 1000.0,
                result_ms = reading.elapsed().as_secs_f64() * 1000.0,
                "split command prepared"
            );
            Ok(blocks)
        })
        .await
    }
}

pub async fn move_block(
    conn: &Connection,
    uuid: uuid::Uuid,
    new_parent_uuid: Option<uuid::Uuid>,
    after_uuid: Option<uuid::Uuid>,
) -> Result<Block> {
    Ok(move_block_with_ops(conn, uuid, new_parent_uuid, after_uuid)
        .await?
        .value)
}

#[doc(hidden)]
pub async fn move_block_with_ops(
    conn: &Connection,
    uuid: uuid::Uuid,
    new_parent_uuid: Option<uuid::Uuid>,
    after_uuid: Option<uuid::Uuid>,
) -> Result<AppliedMutation<Block>> {
    let committed = crate::store::NotesStore::new(conn.clone())
        .move_block(uuid, new_parent_uuid, after_uuid)
        .await?;
    Ok(AppliedMutation {
        value: committed.value,
        operations: committed.operations,
    })
}

pub async fn reorder_block(
    conn: &Connection,
    uuid: uuid::Uuid,
    direction: ReorderDirection,
) -> Result<Block> {
    Ok(reorder_block_with_ops(conn, uuid, direction).await?.value)
}

#[doc(hidden)]
pub async fn reorder_block_with_ops(
    conn: &Connection,
    uuid: uuid::Uuid,
    direction: ReorderDirection,
) -> Result<AppliedMutation<Block>> {
    let committed = crate::store::NotesStore::new(conn.clone())
        .reorder_block(uuid, direction)
        .await?;
    Ok(AppliedMutation {
        value: committed.value,
        operations: committed.operations,
    })
}

pub async fn indent_block(conn: &Connection, uuid: uuid::Uuid) -> Result<Block> {
    Ok(indent_block_with_ops(conn, uuid).await?.value)
}

#[doc(hidden)]
pub async fn indent_block_with_ops(
    conn: &Connection,
    uuid: uuid::Uuid,
) -> Result<AppliedMutation<Block>> {
    let committed = crate::store::NotesStore::new(conn.clone())
        .indent_block(uuid)
        .await?;
    Ok(AppliedMutation {
        value: committed.value,
        operations: committed.operations,
    })
}

pub async fn outdent_block(conn: &Connection, uuid: uuid::Uuid) -> Result<Block> {
    Ok(outdent_block_with_ops(conn, uuid).await?.value)
}

#[doc(hidden)]
pub async fn outdent_block_with_ops(
    conn: &Connection,
    uuid: uuid::Uuid,
) -> Result<AppliedMutation<Block>> {
    let committed = crate::store::NotesStore::new(conn.clone())
        .outdent_block(uuid)
        .await?;
    Ok(AppliedMutation {
        value: committed.value,
        operations: committed.operations,
    })
}

pub async fn move_block_in_direction(
    conn: &Connection,
    uuid: uuid::Uuid,
    direction: ReorderDirection,
) -> Result<Block> {
    Ok(move_block_in_direction_with_ops(conn, uuid, direction)
        .await?
        .value)
}

#[doc(hidden)]
pub async fn move_block_in_direction_with_ops(
    conn: &Connection,
    uuid: uuid::Uuid,
    direction: ReorderDirection,
) -> Result<AppliedMutation<Block>> {
    reorder_block_with_ops(conn, uuid, direction).await
}

pub async fn delete_block(conn: &Connection, uuid: uuid::Uuid) -> Result<bool> {
    Ok(delete_block_with_ops(conn, uuid).await?.value)
}

#[doc(hidden)]
pub async fn delete_block_with_ops(
    conn: &Connection,
    uuid: uuid::Uuid,
) -> Result<AppliedMutation<bool>> {
    let committed = crate::store::NotesStore::new(conn.clone())
        .delete_block(uuid)
        .await?;
    Ok(AppliedMutation {
        value: committed.value,
        operations: committed.operations,
    })
}
