//! Facts needed to invalidate projections after a committed text command.

use crate::{AttachmentOwner, OrderKey};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BlockPlacement {
    pub page_uuid: Uuid,
    pub parent_uuid: Option<Uuid>,
    pub order_key: OrderKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BlockChange {
    pub uuid: Uuid,
    pub before: Option<BlockPlacement>,
    pub after: Option<BlockPlacement>,
}

/// Sorted, unique invalidation facts from the final transaction state.
/// A block may occur even when its placement is unchanged: content, revision,
/// style or timestamps can have changed instead.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ChangeSet {
    pub changed_pages: Vec<Uuid>,
    pub deleted_pages: Vec<Uuid>,
    pub affected_pages: Vec<Uuid>,
    pub blocks: Vec<BlockChange>,
    pub graph_changed_content: Vec<Uuid>,
    pub attachment_owners: Vec<Uuid>,
    #[serde(default)]
    pub handwriting_pages: Vec<Uuid>,
    pub history_changed: bool,
}

impl ChangeSet {
    pub fn is_empty(&self) -> bool {
        self.changed_pages.is_empty()
            && self.deleted_pages.is_empty()
            && self.affected_pages.is_empty()
            && self.blocks.is_empty()
            && self.graph_changed_content.is_empty()
            && self.attachment_owners.is_empty()
            && self.handwriting_pages.is_empty()
            && !self.history_changed
    }
}

// Use exactly the public read projections, including their revision fallback.
// Internal LWW clocks alone must not invalidate an unchanged observable row.
type BlockState = crate::db::Block;
type PageState = crate::db::Page;

#[derive(Default, PartialEq, Eq)]
struct GraphState {
    pages: Vec<(String, Option<Uuid>)>,
    blocks: Vec<Uuid>,
}

#[derive(PartialEq, Eq)]
struct AttachmentState {
    uuid: Uuid,
    hash: Vec<u8>,
    filename: String,
    mime: String,
    size: i64,
    created_at: i64,
}

/// Private before-images are captured once, immediately before the first write
/// to a candidate. Final comparisons query only those candidates.
#[derive(Default)]
pub(crate) struct ChangeTracker {
    pages: BTreeMap<Uuid, Option<PageState>>,
    blocks: BTreeMap<Uuid, Option<BlockState>>,
    graph: BTreeMap<Uuid, GraphState>,
    attachments: BTreeMap<Uuid, (AttachmentOwner, Vec<AttachmentState>, Option<Uuid>)>,
    handwriting: BTreeMap<Uuid, Option<crate::ink::InkNoteStatus>>,
}

impl ChangeTracker {
    pub(crate) fn handwriting(&mut self, db: &Connection, uuid: Uuid) -> rusqlite::Result<()> {
        if let std::collections::btree_map::Entry::Vacant(entry) = self.handwriting.entry(uuid) {
            entry.insert(read_handwriting(db, uuid)?);
        }
        Ok(())
    }

    pub(crate) fn page(&mut self, db: &Connection, uuid: Uuid) -> rusqlite::Result<()> {
        if let std::collections::btree_map::Entry::Vacant(entry) = self.pages.entry(uuid) {
            entry.insert(read_page(db, uuid)?);
        }
        Ok(())
    }

    pub(crate) fn block(&mut self, db: &Connection, uuid: Uuid) -> rusqlite::Result<()> {
        if let std::collections::btree_map::Entry::Vacant(entry) = self.blocks.entry(uuid) {
            entry.insert(read_block(db, uuid)?);
        }
        Ok(())
    }

    pub(crate) fn graph(&mut self, db: &Connection, uuid: Uuid) -> rusqlite::Result<()> {
        if let std::collections::btree_map::Entry::Vacant(entry) = self.graph.entry(uuid) {
            entry.insert(read_graph(db, uuid)?);
        }
        Ok(())
    }

    pub(crate) fn graph_alias(&mut self, db: &Connection, alias: &str) -> rusqlite::Result<()> {
        let mut statement =
            db.prepare("SELECT source_block_uuid FROM page_links WHERE target_title = ?1")?;
        for uuid in statement.query_map([alias], |row| row.get::<_, Uuid>(0))? {
            self.graph(db, uuid?)?;
        }
        Ok(())
    }

    pub(crate) fn attachment(
        &mut self,
        db: &Connection,
        owner: AttachmentOwner,
    ) -> rusqlite::Result<()> {
        if let std::collections::btree_map::Entry::Vacant(entry) =
            self.attachments.entry(owner.uuid())
        {
            entry.insert((owner, read_attachments(db, owner)?, owner_page(db, owner)?));
        }
        Ok(())
    }

    pub(crate) fn deleting_block(&mut self, db: &Connection, uuid: Uuid) -> rusqlite::Result<()> {
        self.block(db, uuid)?;
        self.graph(db, uuid)?;
        self.attachment(db, AttachmentOwner::Block(uuid))?;
        let mut statement = db.prepare("SELECT uuid FROM blocks WHERE parent_uuid = ?1")?;
        for child in statement.query_map([uuid], |row| row.get::<_, Uuid>(0))? {
            self.block(db, child?)?;
        }
        Ok(())
    }

    pub(crate) fn deleting_page(&mut self, db: &Connection, uuid: Uuid) -> rusqlite::Result<()> {
        self.page(db, uuid)?;
        self.handwriting(db, uuid)?;
        self.attachment(db, AttachmentOwner::Page(uuid))?;
        // ON DELETE SET NULL changes incoming edges before alias reconciliation.
        let mut statement = db.prepare(
            "SELECT DISTINCT source_block_uuid FROM page_links WHERE target_page_uuid = ?1",
        )?;
        for source in statement.query_map([uuid], |row| row.get::<_, Uuid>(0))? {
            self.graph(db, source?)?;
        }
        Ok(())
    }

    pub(crate) fn finish(self, db: &Connection) -> rusqlite::Result<ChangeSet> {
        let mut changes = ChangeSet::default();
        let mut affected_pages = BTreeSet::new();
        let mut graph_changed = BTreeSet::new();
        for (uuid, before) in self.pages {
            let after = read_page(db, uuid)?;
            if before == after {
                continue;
            }
            affected_pages.insert(uuid);
            if before.as_ref().map(|page| &page.title) != after.as_ref().map(|page| &page.title) {
                graph_changed.insert(uuid);
            }
            if after.is_some() {
                changes.changed_pages.push(uuid);
            } else {
                changes.deleted_pages.push(uuid);
            }
        }
        for (uuid, before) in self.blocks {
            let after = read_block(db, uuid)?;
            if before == after {
                continue;
            }
            for state in before.iter().chain(after.iter()) {
                affected_pages.insert(state.page_uuid);
            }
            // Local graph snapshots include labels and containment edges;
            // workspace graphs additionally project links onto owning pages.
            let graph_node_changed = match (&before, &after) {
                (Some(before), Some(after)) => {
                    before.page_uuid != after.page_uuid
                        || before.parent_uuid != after.parent_uuid
                        || !before
                            .markdown
                            .chars()
                            .take(80)
                            .eq(after.markdown.chars().take(80))
                }
                _ => true,
            };
            if graph_node_changed {
                graph_changed.insert(uuid);
            }
            changes.blocks.push(BlockChange {
                uuid,
                before: before.as_ref().map(placement),
                after: after.as_ref().map(placement),
            });
        }
        for (uuid, before) in self.graph {
            if before != read_graph(db, uuid)? {
                graph_changed.insert(uuid);
                if let Some(page) = owner_page(db, AttachmentOwner::Block(uuid))? {
                    affected_pages.insert(page);
                }
            }
        }
        for (uuid, (owner, before, previous_page)) in self.attachments {
            if before != read_attachments(db, owner)? {
                changes.attachment_owners.push(uuid);
                affected_pages.extend(previous_page);
                affected_pages.extend(owner_page(db, owner)?);
            }
        }
        for (uuid, before) in self.handwriting {
            if before != read_handwriting(db, uuid)? {
                changes.handwriting_pages.push(uuid);
                affected_pages.insert(uuid);
            }
        }
        changes.affected_pages = affected_pages.into_iter().collect();
        changes.graph_changed_content = graph_changed.into_iter().collect();
        Ok(changes)
    }
}

fn read_handwriting(
    db: &Connection,
    uuid: Uuid,
) -> rusqlite::Result<Option<crate::ink::InkNoteStatus>> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM pages p JOIN page_identities i ON i.page_uuid=p.uuid
         WHERE p.uuid=?1 AND i.content_type='ink')",
        [uuid],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(None);
    }
    crate::ink::read_status(db, uuid)
        .map(Some)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
}

fn read_page(db: &Connection, uuid: Uuid) -> rusqlite::Result<Option<PageState>> {
    db.query_row(
        &format!(
            "SELECT {} FROM pages WHERE uuid = ?1",
            crate::db::PAGE_COLUMNS
        ),
        [uuid],
        crate::db::row_to_page,
    )
    .optional()
}

fn read_block(db: &Connection, uuid: Uuid) -> rusqlite::Result<Option<BlockState>> {
    db.query_row(
        &format!(
            "SELECT {} FROM blocks WHERE uuid = ?1",
            crate::db::BLOCK_COLUMNS
        ),
        [uuid],
        crate::db::row_to_block,
    )
    .optional()
}

fn placement(block: &BlockState) -> BlockPlacement {
    BlockPlacement {
        page_uuid: block.page_uuid,
        parent_uuid: block.parent_uuid,
        order_key: block.order_key.clone(),
    }
}

fn read_graph(db: &Connection, uuid: Uuid) -> rusqlite::Result<GraphState> {
    let pages = db.prepare(
        "SELECT target_title, target_page_uuid FROM page_links WHERE source_block_uuid = ?1 ORDER BY target_title",
    )?.query_map([uuid], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let blocks = db.prepare(
        "SELECT target_block_uuid FROM block_refs WHERE source_block_uuid = ?1 ORDER BY target_block_uuid",
    )?.query_map([uuid], |row| row.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(GraphState { pages, blocks })
}

fn owner_page(db: &Connection, owner: AttachmentOwner) -> rusqlite::Result<Option<Uuid>> {
    match owner {
        AttachmentOwner::Page(uuid) => db
            .query_row("SELECT uuid FROM pages WHERE uuid = ?1", [uuid], |row| {
                row.get(0)
            })
            .optional(),
        AttachmentOwner::Block(uuid) => db
            .query_row(
                "SELECT page_uuid FROM blocks WHERE uuid = ?1",
                [uuid],
                |row| row.get(0),
            )
            .optional(),
    }
}

fn read_attachments(
    db: &Connection,
    owner: AttachmentOwner,
) -> rusqlite::Result<Vec<AttachmentState>> {
    let column = match owner {
        AttachmentOwner::Page(_) => "page_uuid",
        AttachmentOwner::Block(_) => "block_uuid",
    };
    db.prepare(&format!(
        "SELECT uuid, blob_hash, filename, mime, size, created_at FROM attachments WHERE {column} = ?1 ORDER BY uuid",
    ))?.query_map([owner.uuid()], |row| Ok(AttachmentState {
        uuid: row.get(0)?, hash: row.get(1)?, filename: row.get(2)?,
        mime: row.get(3)?, size: row.get(4)?, created_at: row.get(5)?,
    }))?.collect()
}
