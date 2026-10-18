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
    let siblings = read_block_children(transaction, block.page_uuid, new_parent_uuid)?;
    let mut ordered = siblings
        .into_iter()
        .filter(|sibling| sibling.uuid != block.uuid)
        .map(|sibling| sibling.uuid)
        .collect::<Vec<_>>();
    let insertion = match after_uuid {
        Some(after) => ordered
            .iter()
            .position(|candidate| *candidate == after)
            .map(|index| index + 1)
            .ok_or_else(|| {
                crate::CoreError::invalid("after block is not a sibling of the moved block")
            })?,
        None => ordered.len(),
    };
    ordered.insert(insertion, block.uuid);
    let kinds = ordered
        .into_iter()
        .enumerate()
        .map(|(index, block_uuid)| {
            OpKind::BlockMove(BlockMove {
                uuid: block_uuid,
                page_uuid: block.page_uuid,
                parent_uuid: new_parent_uuid,
                order_key: OrderKey::from_ordinal(index + 1),
            })
        })
        .collect();
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
            let siblings = read_block_children(transaction, page_uuid, parent_uuid)?;
            let insertion = match after_uuid {
                Some(after) => siblings
                    .iter()
                    .position(|block| block.uuid == after)
                    .map(|index| index + 1)
                    .ok_or_else(|| {
                        crate::CoreError::invalid("after block is not a sibling of the new block")
                    })?,
                None => siblings.len(),
            };
            let uuid = uuid::Uuid::now_v7();
            let now = chrono::Utc::now().timestamp();
            let appending = insertion == siblings.len();
            let order_key = if appending {
                next_append_order_key(siblings.last().map(|block| &block.order_key))?
            } else {
                OrderKey::from_ordinal(insertion + 1)
            };
            let mut kinds = vec![OpKind::BlockCreate(BlockCreate {
                uuid,
                page_uuid,
                parent_uuid,
                order_key,
                style,
                markdown,
                created_at: now,
            })];
            if !appending {
                let mut ordered = siblings.iter().map(|block| block.uuid).collect::<Vec<_>>();
                ordered.insert(insertion, uuid);
                kinds.extend(ordered.into_iter().enumerate().map(|(index, block_uuid)| {
                    OpKind::BlockMove(BlockMove {
                        uuid: block_uuid,
                        page_uuid,
                        parent_uuid,
                        order_key: OrderKey::from_ordinal(index + 1),
                    })
                }));
            }
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
            let mut siblings =
                read_block_children(write.sql(), block.page_uuid, block.parent_uuid)?;
            let index = siblings
                .iter()
                .position(|sibling| sibling.uuid == uuid)
                .context("block is absent from its parent")?;
            let target = match direction {
                ReorderDirection::Up if index > 0 => Some(index - 1),
                ReorderDirection::Down if index + 1 < siblings.len() => Some(index + 1),
                ReorderDirection::Up | ReorderDirection::Down => None,
            };
            let Some(target) = target else {
                return Ok(block);
            };
            siblings.swap(index, target);
            let kinds = siblings
                .into_iter()
                .enumerate()
                .map(|(index, sibling)| {
                    OpKind::BlockMove(BlockMove {
                        uuid: sibling.uuid,
                        page_uuid: block.page_uuid,
                        parent_uuid: block.parent_uuid,
                        order_key: OrderKey::from_ordinal(index + 1),
                    })
                })
                .collect();
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
            let siblings = read_block_children(write.sql(), block.page_uuid, block.parent_uuid)?;
            let index = siblings
                .iter()
                .position(|sibling| sibling.uuid == uuid)
                .context("block is absent from its parent")?;
            let Some(previous) = index.checked_sub(1).and_then(|index| siblings.get(index)) else {
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
    if count == 0 {
        return Ok((Vec::new(), Vec::new()));
    }
    // The sibling index covers this lookup; the ordinary Enter/paste path
    // does not need to load every sibling's Markdown or renumber the page.
    let next = transaction
        .query_row(
            "SELECT order_key FROM blocks
              WHERE page_uuid = ?1 AND parent_uuid IS ?2
                AND (order_key, uuid) > (?3, ?4)
              ORDER BY order_key, uuid LIMIT 1",
            rusqlite::params![
                source.page_uuid,
                source.parent_uuid,
                source.order_key,
                source.uuid,
            ],
            |row| row.get::<_, OrderKey>(0),
        )
        .optional()?;
    let keys = if let Some(next) = next {
        OrderKey::between(Some(&source.order_key), Some(&next), count)
    } else {
        let mut previous = source.order_key.clone();
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
    };
    if let Some(keys) = keys {
        return Ok((keys, Vec::new()));
    }

    // Tied or exhausted keys can occur after concurrent edits or repeated
    // insertions. Rebalance only this sibling group and only changed keys.
    let siblings = transaction
        .prepare(
            "SELECT uuid, order_key FROM blocks
              WHERE page_uuid = ?1 AND parent_uuid IS ?2 ORDER BY order_key, uuid",
        )?
        .query_map(
            rusqlite::params![source.page_uuid, source.parent_uuid],
            |row| Ok((row.get::<_, uuid::Uuid>(0)?, row.get::<_, OrderKey>(1)?)),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let source_index = siblings
        .iter()
        .position(|(uuid, _)| *uuid == source.uuid)
        .ok_or_else(|| crate::CoreError::conflict("split source is absent from its siblings"))?;
    let moves = siblings
        .into_iter()
        .enumerate()
        .filter_map(|(index, (uuid, current))| {
            let ordinal = index + 1 + if index > source_index { count } else { 0 };
            let order_key = OrderKey::from_ordinal(ordinal);
            (order_key != current).then_some(OpKind::BlockMove(BlockMove {
                uuid,
                page_uuid: source.page_uuid,
                parent_uuid: source.parent_uuid,
                order_key,
            }))
        })
        .collect();
    let keys = (1..=count)
        .map(|offset| OrderKey::from_ordinal(source_index + offset + 1))
        .collect();
    Ok((keys, moves))
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
