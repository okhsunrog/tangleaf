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
        let placeholders = std::iter::repeat_n("?", uuids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!("SELECT {BLOCK_COLUMNS} FROM blocks WHERE uuid IN ({placeholders})");
        let blocks = database
            .prepare(&sql)?
            .query_map(rusqlite::params_from_iter(uuids.iter()), row_to_block)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let by_uuid = blocks
            .into_iter()
            .map(|block| (block.uuid, block))
            .collect::<std::collections::HashMap<_, _>>();
        Ok(uuids
            .into_iter()
            .filter_map(|uuid| by_uuid.get(&uuid).cloned())
            .collect())
    })
    .await
}

pub async fn list_block_children(
    conn: &Connection,
    page_uuid: uuid::Uuid,
    parent_uuid: Option<uuid::Uuid>,
) -> Result<Vec<Block>> {
    conn.call(move |database| {
        let sql = format!(
            "SELECT {BLOCK_COLUMNS} FROM blocks
              WHERE page_uuid = ?1 AND parent_uuid IS ?2
              ORDER BY order_key, uuid"
        );
        database
            .prepare(&sql)?
            .query_map(rusqlite::params![page_uuid, parent_uuid], row_to_block)?
            .collect()
    })
    .await
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
    if get_page(conn, page_uuid).await?.is_none() {
        return Err(crate::CoreError::not_found("page was not found").into());
    }
    if let Some(parent_uuid) = parent_uuid {
        let parent = get_block(conn, parent_uuid)
            .await?
            .ok_or_else(|| crate::CoreError::not_found("parent block was not found"))?;
        if parent.page_uuid != page_uuid {
            return Err(
                crate::CoreError::conflict("parent block belongs to a different page").into(),
            );
        }
    }
    let siblings = list_block_children(conn, page_uuid, parent_uuid).await?;
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
    let operations = apply_local_action(conn, "create block", kinds).await?;
    let value = get_block(conn, uuid)
        .await?
        .context("created block disappeared")?;
    Ok(AppliedMutation { value, operations })
}

pub async fn set_block_content(
    conn: &Connection,
    uuid: uuid::Uuid,
    content: BlockContent,
) -> Result<(Block, bool)> {
    let block = get_block(conn, uuid)
        .await?
        .ok_or_else(|| crate::CoreError::not_found("block was not found"))?;
    let graph_changed = operation::content_references_changed(&block.markdown, &content.markdown);
    if block.markdown != content.markdown {
        apply_local(
            conn,
            vec![OpKind::BlockSetMarkdown(BlockSetMarkdown {
                uuid,
                markdown: content.markdown,
            })],
        )
        .await?;
    }
    Ok((
        get_block(conn, uuid)
            .await?
            .context("updated block disappeared")?,
        graph_changed,
    ))
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
    conn.call_domain(
        move |database| -> crate::CoreResult<AppliedMutation<(Block, bool)>> {
            let started = std::time::Instant::now();
            let transaction = database.transaction()?;
            let sql = format!("SELECT {BLOCK_COLUMNS} FROM blocks WHERE uuid = ?1");
            let block = transaction
                .query_row(&sql, [uuid], row_to_block)
                .optional()?
                .ok_or_else(|| crate::CoreError::not_found("block was not found"))?;
            if block.markdown == content.markdown {
                let planning_ms = started.elapsed().as_secs_f64() * 1_000.0;
                let commit_started = std::time::Instant::now();
                transaction.commit()?;
                tracing::debug!(
                    changed = false,
                    planning_ms,
                    apply_ms = 0.0,
                    result_ms = 0.0,
                    commit_ms = commit_started.elapsed().as_secs_f64() * 1_000.0,
                    transaction_ms = started.elapsed().as_secs_f64() * 1_000.0,
                    "content transaction completed"
                );
                return Ok(AppliedMutation {
                    value: (block, false),
                    operations: Vec::new(),
                });
            }
            if block.markdown_revision != expected_revision {
                return Err(crate::CoreError::conflict(
                    "block content changed since editing began",
                ));
            }
            let graph_changed =
                operation::content_references_changed(&block.markdown, &content.markdown);
            let operations = vec![OpKind::BlockSetMarkdown(BlockSetMarkdown {
                uuid,
                markdown: content.markdown,
            })];
            let planning_ms = started.elapsed().as_secs_f64() * 1_000.0;
            let apply_started = std::time::Instant::now();
            operation::apply_local_kinds_in_transaction(&transaction, operations.clone())?;
            let apply_ms = apply_started.elapsed().as_secs_f64() * 1_000.0;
            let result_started = std::time::Instant::now();
            let block = transaction.query_row(&sql, [uuid], row_to_block)?;
            let result_ms = result_started.elapsed().as_secs_f64() * 1_000.0;
            let commit_started = std::time::Instant::now();
            transaction.commit()?;
            tracing::debug!(
                changed = true,
                planning_ms,
                apply_ms,
                result_ms,
                commit_ms = commit_started.elapsed().as_secs_f64() * 1_000.0,
                transaction_ms = started.elapsed().as_secs_f64() * 1_000.0,
                "content transaction completed"
            );
            Ok(AppliedMutation {
                value: (block, graph_changed),
                operations,
            })
        },
    )
    .await
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
    let block = get_block(conn, uuid)
        .await?
        .ok_or_else(|| crate::CoreError::not_found("block was not found"))?;
    let operations = if block.style != style {
        apply_local_action(
            conn,
            "set block style",
            vec![OpKind::BlockSetStyle(BlockSetStyle { uuid, style })],
        )
        .await?
    } else {
        Vec::new()
    };
    let value = get_block(conn, uuid)
        .await?
        .context("updated block disappeared")?;
    Ok(AppliedMutation { value, operations })
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
    let block = get_block(conn, uuid)
        .await?
        .ok_or_else(|| crate::CoreError::not_found("block was not found"))?;
    if block.style.task_state().is_none() {
        return Err(crate::CoreError::invalid("task state requires a task block").into());
    }
    set_block_style_with_ops(conn, uuid, BlockStyle::task(state)).await
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
    let block = get_block(conn, uuid)
        .await?
        .ok_or_else(|| crate::CoreError::not_found("block was not found"))?;
    if new_parent_uuid == Some(uuid) {
        return Err(crate::CoreError::conflict("a block cannot be its own parent").into());
    }
    if let Some(parent) = new_parent_uuid {
        let parent_block = get_block(conn, parent)
            .await?
            .ok_or_else(|| crate::CoreError::not_found("parent block was not found"))?;
        if parent_block.page_uuid != block.page_uuid {
            return Err(
                crate::CoreError::conflict("parent block belongs to a different page").into(),
            );
        }
        let creates_cycle = conn
            .call(move |database| {
                database.query_row(
                    "WITH RECURSIVE descendants(uuid) AS (
                       SELECT uuid FROM blocks WHERE parent_uuid = ?1
                       UNION ALL
                       SELECT block.uuid FROM blocks block
                         JOIN descendants child ON block.parent_uuid = child.uuid
                     )
                     SELECT EXISTS(SELECT 1 FROM descendants WHERE uuid = ?2)",
                    rusqlite::params![uuid, parent],
                    |row| row.get::<_, bool>(0),
                )
            })
            .await?;
        if creates_cycle {
            return Err(crate::CoreError::conflict(
                "a block cannot be moved under one of its descendants",
            )
            .into());
        }
    }
    let siblings = list_block_children(conn, block.page_uuid, new_parent_uuid).await?;
    let mut ordered = siblings
        .into_iter()
        .filter(|sibling| sibling.uuid != uuid)
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
    ordered.insert(insertion, uuid);
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
    let operations = apply_local_action(conn, "move block", kinds).await?;
    let value = get_block(conn, uuid)
        .await?
        .context("moved block disappeared")?;
    Ok(AppliedMutation { value, operations })
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
    let block = get_block(conn, uuid)
        .await?
        .ok_or_else(|| crate::CoreError::not_found("block was not found"))?;
    let mut siblings = list_block_children(conn, block.page_uuid, block.parent_uuid).await?;
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
        return Ok(AppliedMutation {
            value: block,
            operations: Vec::new(),
        });
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
    let operations = apply_local_action(conn, "reorder block", kinds).await?;
    let value = get_block(conn, uuid)
        .await?
        .context("reordered block disappeared")?;
    Ok(AppliedMutation { value, operations })
}

pub async fn indent_block(conn: &Connection, uuid: uuid::Uuid) -> Result<Block> {
    Ok(indent_block_with_ops(conn, uuid).await?.value)
}

#[doc(hidden)]
pub async fn indent_block_with_ops(
    conn: &Connection,
    uuid: uuid::Uuid,
) -> Result<AppliedMutation<Block>> {
    let block = get_block(conn, uuid)
        .await?
        .ok_or_else(|| crate::CoreError::not_found("block was not found"))?;
    let siblings = list_block_children(conn, block.page_uuid, block.parent_uuid).await?;
    let index = siblings
        .iter()
        .position(|sibling| sibling.uuid == uuid)
        .context("block is absent from its parent")?;
    let Some(previous) = index.checked_sub(1).and_then(|index| siblings.get(index)) else {
        return Ok(AppliedMutation {
            value: block,
            operations: Vec::new(),
        });
    };
    move_block_with_ops(conn, uuid, Some(previous.uuid), None).await
}

pub async fn outdent_block(conn: &Connection, uuid: uuid::Uuid) -> Result<Block> {
    Ok(outdent_block_with_ops(conn, uuid).await?.value)
}

#[doc(hidden)]
pub async fn outdent_block_with_ops(
    conn: &Connection,
    uuid: uuid::Uuid,
) -> Result<AppliedMutation<Block>> {
    let block = get_block(conn, uuid)
        .await?
        .ok_or_else(|| crate::CoreError::not_found("block was not found"))?;
    let parent_uuid = block
        .parent_uuid
        .ok_or_else(|| crate::CoreError::conflict("top-level blocks cannot be outdented"))?;
    let parent = get_block(conn, parent_uuid)
        .await?
        .context("parent block disappeared")?;
    move_block_with_ops(conn, uuid, parent.parent_uuid, Some(parent.uuid)).await
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
    let Some(block) = get_block(conn, uuid).await? else {
        return Ok(AppliedMutation {
            value: false,
            operations: Vec::new(),
        });
    };
    let child_count = conn
        .call(move |database| {
            database.query_row(
                "SELECT COUNT(*) FROM blocks WHERE parent_uuid = ?1",
                [uuid],
                |row| row.get::<_, i64>(0),
            )
        })
        .await?;
    if child_count != 0 {
        return Ok(AppliedMutation {
            value: false,
            operations: Vec::new(),
        });
    }
    let operations = apply_local_action(
        conn,
        "delete block",
        vec![OpKind::BlockDelete(BlockDelete {
            uuid,
            page_uuid: block.page_uuid,
        })],
    )
    .await?;
    Ok(AppliedMutation {
        value: true,
        operations,
    })
}
