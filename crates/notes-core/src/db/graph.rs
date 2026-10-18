use super::*;
use crate::PageListFilter;
use std::collections::{BTreeSet, HashSet};
use uuid::Uuid;

#[cfg(test)]
#[path = "graph_tests.rs"]
mod tests;

async fn read_graph<T: Send + 'static>(
    conn: &Connection,
    read: impl FnOnce(&rusqlite::Transaction<'_>) -> rusqlite::Result<T> + Send + 'static,
) -> Result<T> {
    conn.call(move |database| {
        let transaction = database.transaction()?;
        let value = read(&transaction)?;
        transaction.commit()?;
        Ok(value)
    })
    .await
}

pub async fn neighbors(conn: &Connection, uuid: Uuid, depth: u32) -> Result<Vec<Content>> {
    read_graph(conn, move |transaction| {
        let uuids = neighbor_uuids(transaction, uuid, depth)?;
        get_contents_in_transaction(transaction, &uuids)
    })
    .await
}

/// Breadth-first traversal keeps minimum-depth/UUID order without constructing
/// a workspace-wide adjacency relation. Missing block-reference destinations
/// remain vertices so they can connect live referrers at a later depth.
fn neighbor_uuids(
    transaction: &rusqlite::Transaction<'_>,
    uuid: Uuid,
    depth: u32,
) -> rusqlite::Result<Vec<Uuid>> {
    let mut visited = HashSet::from([uuid]);
    let mut frontier = vec![uuid];
    let mut result = Vec::new();
    for _ in 0..depth {
        if frontier.is_empty() {
            break;
        }
        let mut next = BTreeSet::new();
        for chunk in frontier.chunks(UUID_READ_CHUNK_SIZE) {
            let placeholders = placeholders(chunk.len());
            for query in [
                format!(
                    "SELECT target_page_uuid FROM page_links WHERE source_block_uuid IN ({placeholders}) AND target_page_uuid IS NOT NULL"
                ),
                format!(
                    "SELECT source_block_uuid FROM page_links WHERE target_page_uuid IN ({placeholders})"
                ),
                format!(
                    "SELECT target_block_uuid FROM block_refs WHERE source_block_uuid IN ({placeholders})"
                ),
                format!(
                    "SELECT source_block_uuid FROM block_refs WHERE target_block_uuid IN ({placeholders})"
                ),
                format!(
                    "SELECT COALESCE(parent_uuid, page_uuid) FROM blocks WHERE uuid IN ({placeholders})"
                ),
                format!("SELECT uuid FROM blocks WHERE parent_uuid IN ({placeholders})"),
                format!(
                    "SELECT uuid FROM blocks WHERE page_uuid IN ({placeholders}) AND parent_uuid IS NULL"
                ),
            ] {
                let mut statement = transaction.prepare_cached(&query)?;
                for target in statement
                    .query_map(rusqlite::params_from_iter(chunk.iter()), |row| {
                        row.get::<_, Uuid>(0)
                    })?
                {
                    let target = target?;
                    if !visited.contains(&target) {
                        next.insert(target);
                    }
                }
            }
        }
        frontier = next.into_iter().collect();
        visited.extend(frontier.iter().copied());
        result.extend(frontier.iter().copied());
    }
    Ok(result)
}

pub async fn find_backlinks(conn: &Connection, uuid: Uuid) -> Result<Vec<Content>> {
    read_graph(conn, move |transaction| {
        let uuids = backlink_uuids(transaction, uuid)?;
        get_contents_in_transaction(transaction, &uuids)
    })
    .await
}

/// Source blocks and their distinct owning pages from one read snapshot.
/// Blocks retain backlink UUID order; pages use UUID order. An absent owning
/// page is omitted explicitly rather than represented by a placeholder page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BacklinksSnapshot {
    pub blocks: Vec<Block>,
    pub pages: Vec<Page>,
}

pub async fn find_backlinks_snapshot(conn: &Connection, uuid: Uuid) -> Result<BacklinksSnapshot> {
    read_graph(conn, move |transaction| {
        let uuids = backlink_uuids(transaction, uuid)?;
        let mut by_uuid = blocks::read_blocks_by_uuid(transaction, &uuids)?;
        let blocks = uuids
            .into_iter()
            .filter_map(|uuid| by_uuid.remove(&uuid))
            .collect::<Vec<_>>();
        let page_uuids = blocks
            .iter()
            .map(|block| block.page_uuid)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let mut pages = Vec::with_capacity(page_uuids.len());
        for chunk in page_uuids.chunks(UUID_READ_CHUNK_SIZE) {
            let sql = format!(
                "SELECT {PAGE_COLUMNS} FROM pages WHERE uuid IN ({}) ORDER BY pages.uuid",
                placeholders(chunk.len())
            );
            pages.extend(
                transaction
                    .prepare_cached(&sql)?
                    .query_map(rusqlite::params_from_iter(chunk.iter()), row_to_page)?
                    .collect::<rusqlite::Result<Vec<_>>>()?,
            );
        }
        Ok(BacklinksSnapshot { blocks, pages })
    })
    .await
}

fn backlink_uuids(
    transaction: &rusqlite::Transaction<'_>,
    uuid: Uuid,
) -> rusqlite::Result<Vec<Uuid>> {
    transaction
        .prepare_cached(
            "SELECT source_block_uuid FROM page_links WHERE target_page_uuid = ?1
             UNION
             SELECT source_block_uuid FROM block_refs WHERE target_block_uuid = ?1
             ORDER BY 1",
        )?
        .query_map([uuid], |row| row.get(0))?
        .collect()
}

pub async fn read_ancestors(conn: &Connection, uuid: Uuid) -> Result<Vec<Content>> {
    read_graph(conn, move |transaction| {
        let Some(block) = blocks::read_blocks_by_uuid(transaction, &[uuid])?.remove(&uuid) else {
            return get_contents_in_transaction(transaction, &[uuid]);
        };
        let mut statement = transaction.prepare_cached(
            "WITH RECURSIVE ancestors(uuid, parent_uuid, depth) AS (
               SELECT uuid, parent_uuid, 0 FROM blocks WHERE uuid = ?1
               UNION ALL
               SELECT parent.uuid, parent.parent_uuid, ancestors.depth + 1
                 FROM ancestors JOIN blocks parent ON parent.uuid = ancestors.parent_uuid
             )
             SELECT uuid FROM ancestors ORDER BY depth DESC",
        )?;
        let mut uuids = vec![block.page_uuid];
        uuids.extend(
            statement
                .query_map([uuid], |row| row.get::<_, Uuid>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?,
        );
        get_contents_in_transaction(transaction, &uuids)
    })
    .await
}

pub async fn read_subtree(conn: &Connection, uuid: Uuid, depth: u32) -> Result<Vec<Content>> {
    read_graph(conn, move |transaction| {
        let Some(content) = get_contents_in_transaction(transaction, &[uuid])?.pop() else {
            return Ok(Vec::new());
        };
        let (page_uuid, parent_uuid) = match content {
            Content::Page(page) => (page.uuid, None),
            Content::Block(block) => (block.page_uuid, Some(block.uuid)),
        };
        // Preserve existing depth semantics: direct children are the anchor,
        // including for depth zero; depth limits only recursive expansion.
        let sql = format!(
            "WITH RECURSIVE subtree(uuid, path, depth) AS (
               SELECT block.uuid, block.order_key, 1
                 FROM blocks block
                WHERE block.page_uuid = ?1 AND block.parent_uuid IS ?2
               UNION ALL
               SELECT child.uuid, subtree.path || '/' || child.order_key, subtree.depth + 1
                 FROM subtree JOIN blocks child ON child.parent_uuid = subtree.uuid
                WHERE subtree.depth < ?3
             )
             SELECT {QUALIFIED_BLOCK_COLUMNS} FROM subtree
               JOIN blocks ON blocks.uuid = subtree.uuid
              ORDER BY subtree.path"
        );
        transaction
            .prepare_cached(&sql)?
            .query_map(
                rusqlite::params![page_uuid, parent_uuid, depth],
                row_to_block,
            )?
            .map(|block| block.map(Content::Block))
            .collect()
    })
    .await
}

pub async fn graph_snapshot(conn: &Connection, focus_uuid: Option<Uuid>) -> Result<GraphSnapshot> {
    read_graph(conn, move |transaction| {
        let content = match focus_uuid {
            Some(uuid) => {
                let mut uuids = vec![uuid];
                uuids.extend(neighbor_uuids(transaction, uuid, 1)?);
                get_contents_in_transaction(transaction, &uuids)?
            }
            None => pages::list_pages_in_transaction(transaction, PageListFilter::Notes, u32::MAX)?
                .into_iter()
                .map(Content::Page)
                .collect(),
        };
        let selected = content.iter().map(Content::uuid).collect::<HashSet<_>>();
        let items = content
            .into_iter()
            .map(|content| match content {
                Content::Page(page) => GraphItem {
                    uuid: page.uuid,
                    kind: ObjectKind::Page,
                    label: match page.kind {
                        PageKind::Note | PageKind::Handwriting => {
                            page.title.unwrap_or_else(|| "Untitled".into())
                        }
                        PageKind::Journal { date } => date.to_string(),
                    },
                },
                Content::Block(block) => GraphItem {
                    uuid: block.uuid,
                    kind: ObjectKind::Block,
                    label: block.markdown.chars().take(80).collect(),
                },
            })
            .collect();
        let edges = if focus_uuid.is_none() {
            page_level_edges(transaction, &selected)?
        } else {
            focused_edges(transaction, &selected)?
        };
        Ok(GraphSnapshot { items, edges })
    })
    .await
}

fn focused_edges(
    transaction: &rusqlite::Transaction<'_>,
    selected: &HashSet<Uuid>,
) -> rusqlite::Result<Vec<GraphEdge>> {
    let mut selected_ids = selected.iter().copied().collect::<Vec<_>>();
    selected_ids.sort_unstable();
    let mut page_links = Vec::new();
    let mut block_refs = Vec::new();
    let mut contains = Vec::new();
    for chunk in selected_ids.chunks(UUID_READ_CHUNK_SIZE) {
        let placeholders = placeholders(chunk.len());
        for (query, edges, reverse) in [
            (
                format!(
                    "SELECT source_block_uuid, target_page_uuid FROM page_links WHERE source_block_uuid IN ({placeholders}) AND target_page_uuid IS NOT NULL"
                ),
                &mut page_links,
                false,
            ),
            (
                format!(
                    "SELECT source_block_uuid, target_block_uuid FROM block_refs WHERE source_block_uuid IN ({placeholders})"
                ),
                &mut block_refs,
                false,
            ),
            (
                format!(
                    "SELECT uuid, COALESCE(parent_uuid, page_uuid) FROM blocks WHERE uuid IN ({placeholders})"
                ),
                &mut contains,
                true,
            ),
        ] {
            let mut statement = transaction.prepare_cached(&query)?;
            for row in statement.query_map(rusqlite::params_from_iter(chunk.iter()), |row| {
                Ok((row.get::<_, Uuid>(0)?, row.get::<_, Uuid>(1)?))
            })? {
                let (source, target) = row?;
                if selected.contains(&target) {
                    edges.push(if reverse {
                        (target, source)
                    } else {
                        (source, target)
                    });
                }
            }
        }
    }
    for edges in [&mut page_links, &mut block_refs, &mut contains] {
        edges.sort_unstable();
    }
    Ok([
        (page_links, GraphRelation::PageLink),
        (block_refs, GraphRelation::BlockReference),
        (contains, GraphRelation::Contains),
    ]
    .into_iter()
    .flat_map(|(edges, relation)| {
        edges
            .into_iter()
            .map(move |(source_uuid, target_uuid)| GraphEdge {
                source_uuid,
                target_uuid,
                relation,
            })
    })
    .collect())
}

fn placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(",")
}

/// Project content-level references onto their owning pages for the workspace
/// graph. This intentionally examines the complete workspace; focused graphs
/// use indexed bounded queries instead.
fn page_level_edges(
    database: &rusqlite::Connection,
    selected: &HashSet<Uuid>,
) -> rusqlite::Result<Vec<GraphEdge>> {
    let mut edges = Vec::new();
    for (query, relation) in [
        (
            "SELECT DISTINCT source.page_uuid, link.target_page_uuid
            FROM page_links link
            JOIN blocks source ON source.uuid = link.source_block_uuid
           WHERE link.target_page_uuid IS NOT NULL
           ORDER BY source.page_uuid, link.target_page_uuid",
            GraphRelation::PageLink,
        ),
        (
            "SELECT DISTINCT source.page_uuid, target.page_uuid
            FROM block_refs reference
            JOIN blocks source ON source.uuid = reference.source_block_uuid
            JOIN blocks target ON target.uuid = reference.target_block_uuid
           ORDER BY source.page_uuid, target.page_uuid",
            GraphRelation::BlockReference,
        ),
    ] {
        let mut statement = database.prepare_cached(query)?;
        for row in statement.query_map([], |row| {
            Ok((row.get::<_, Uuid>(0)?, row.get::<_, Uuid>(1)?))
        })? {
            let (source_uuid, target_uuid) = row?;
            if selected.contains(&source_uuid) && selected.contains(&target_uuid) {
                edges.push(GraphEdge {
                    source_uuid,
                    target_uuid,
                    relation,
                });
            }
        }
    }
    Ok(edges)
}
