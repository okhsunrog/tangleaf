//! Indexed dependency loading for the unchanged structural resolver.

use super::{StructureIntent, StructureRow};
use rusqlite::{OptionalExtension, Transaction, params_from_iter};
use std::collections::{BTreeSet, HashMap, HashSet};
use uuid::Uuid;

const UUID_QUERY_CHUNK: usize = 256;

/// Vertices that can affect a write, including endpoints which the write may
/// disconnect. Page UUIDs are lookup keys, never structural graph vertices.
#[derive(Default)]
pub(super) struct StructureDirty {
    seeds: BTreeSet<Uuid>,
    seed_rows_read: usize,
}

impl StructureDirty {
    pub(super) fn is_empty(&self) -> bool {
        self.seeds.is_empty()
    }

    /// Actual returned rows from pre-write seed queries, including repeats.
    pub(super) fn seed_rows_read(&self) -> usize {
        self.seed_rows_read
    }

    /// Capture both previous parent endpoints before replacing an intention.
    /// A previously materialized edge may be the fallback for an invalid raw
    /// intention, so neither parent relation can stand in for the other.
    pub(super) fn touch_block(
        &mut self,
        db: &Transaction<'_>,
        uuid: Uuid,
        new_parent: Option<Uuid>,
    ) -> rusqlite::Result<()> {
        self.seeds.insert(uuid);
        self.seeds.extend(new_parent);
        for query in [
            "SELECT parent_uuid FROM blocks WHERE uuid = ?1",
            "SELECT parent_uuid FROM block_structure_lww WHERE block_uuid = ?1",
        ] {
            if let Some(parent) = db
                .prepare_cached(query)?
                .query_row([uuid], |row| row.get::<_, Option<Uuid>>(0))
                .optional()?
            {
                self.seed_rows_read += 1;
                self.seeds.extend(parent);
            }
        }
        Ok(())
    }

    pub(super) fn deleting_block(
        &mut self,
        db: &Transaction<'_>,
        uuid: Uuid,
    ) -> rusqlite::Result<()> {
        self.touch_block(db, uuid, None)?;
        // The foreign-key action erases these materialized edges before the
        // component loader runs. Raw reverse edges remain independently live.
        let mut statement = db.prepare_cached("SELECT uuid FROM blocks WHERE parent_uuid = ?1")?;
        for child in statement.query_map([uuid], |row| row.get::<_, Uuid>(0))? {
            self.seed_rows_read += 1;
            self.seeds.insert(child?);
        }
        Ok(())
    }

    /// Preserve every member and both previous parent endpoints before page
    /// creation/deletion can make a pending destination live or erase rows.
    pub(super) fn touch_page(&mut self, db: &Transaction<'_>, uuid: Uuid) -> rusqlite::Result<()> {
        // Each side uses its page-membership index. Joining the other side by
        // UUID captures divergent materialized/raw parents without N queries.
        for query in [
            "SELECT block.uuid, block.parent_uuid, intent.parent_uuid
               FROM blocks block
               LEFT JOIN block_structure_lww intent ON intent.block_uuid = block.uuid
              WHERE block.page_uuid = ?1",
            "SELECT intent.block_uuid, block.parent_uuid, intent.parent_uuid
               FROM block_structure_lww intent
               LEFT JOIN blocks block ON block.uuid = intent.block_uuid
              WHERE intent.page_uuid = ?1",
        ] {
            let mut statement = db.prepare_cached(query)?;
            for endpoints in statement.query_map([uuid], |row| {
                Ok((
                    row.get::<_, Uuid>(0)?,
                    row.get::<_, Option<Uuid>>(1)?,
                    row.get::<_, Option<Uuid>>(2)?,
                ))
            })? {
                self.seed_rows_read += 1;
                let (block, materialized_parent, intended_parent) = endpoints?;
                self.seeds.insert(block);
                self.seeds.extend(materialized_parent);
                self.seeds.extend(intended_parent);
            }
        }
        Ok(())
    }
}

/// Actual returned SQL rows, including rows returned by both direct and reverse
/// lookups. Missing UUIDs count as visited vertices, not invented SQL rows.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct StructureLoadStats {
    pub(super) blocks_read: usize,
    pub(super) intents_read: usize,
    pub(super) pages_read: usize,
    pub(super) vertices_visited: usize,
    pub(super) seed_rows_read: usize,
}

pub(super) struct OwnedStructureInput {
    pub(super) existing: HashMap<Uuid, StructureRow>,
    pub(super) pages: HashSet<Uuid>,
    pub(super) intents: HashMap<Uuid, StructureIntent>,
    pub(super) stats: StructureLoadStats,
}

/// Load weak components over both materialized and intended parent edges.
/// Missing endpoints stay in the frontier: their reverse edges can connect
/// children whose parent has not arrived yet. Root blocks sharing one page do
/// not become a component merely because their page UUID is the same.
pub(super) fn load_components(
    db: &Transaction<'_>,
    dirty: &StructureDirty,
) -> rusqlite::Result<OwnedStructureInput> {
    let mut result = OwnedStructureInput {
        existing: HashMap::new(),
        pages: HashSet::new(),
        intents: HashMap::new(),
        stats: StructureLoadStats {
            seed_rows_read: dirty.seed_rows_read,
            ..StructureLoadStats::default()
        },
    };
    if dirty.is_empty() {
        return Ok(result);
    }
    let mut visited = HashSet::new();
    let mut frontier = dirty.seeds.clone();
    while !frontier.is_empty() {
        let batch = frontier
            .iter()
            .take(UUID_QUERY_CHUNK)
            .copied()
            .collect::<Vec<_>>();
        for uuid in &batch {
            frontier.remove(uuid);
            visited.insert(*uuid);
        }
        let placeholders = placeholders(batch.len());
        for predicate in ["uuid", "parent_uuid"] {
            let sql = format!(
                "SELECT uuid, existence_hlc, page_uuid, parent_uuid, order_key, structure_hlc, updated_at
                   FROM blocks WHERE {predicate} IN ({placeholders})"
            );
            let mut statement = db.prepare_cached(&sql)?;
            for row in statement.query_map(params_from_iter(batch.iter()), |row| {
                Ok((
                    row.get::<_, Uuid>(0)?,
                    StructureRow {
                        existence_hlc: row.get(1)?,
                        page_uuid: row.get(2)?,
                        parent_uuid: row.get(3)?,
                        order_key: row.get(4)?,
                        structure_hlc: row.get(5)?,
                        updated_at: row.get(6)?,
                    },
                ))
            })? {
                result.stats.blocks_read += 1;
                let (uuid, block) = row?;
                add_frontier(&mut frontier, &visited, Some(uuid));
                add_frontier(&mut frontier, &visited, block.parent_uuid);
                result.existing.insert(uuid, block);
            }
        }
        for predicate in ["block_uuid", "parent_uuid"] {
            let sql = format!(
                "SELECT block_uuid, page_uuid, parent_uuid, order_key, hlc
                   FROM block_structure_lww WHERE {predicate} IN ({placeholders})"
            );
            let mut statement = db.prepare_cached(&sql)?;
            for row in statement.query_map(params_from_iter(batch.iter()), |row| {
                Ok((
                    row.get::<_, Uuid>(0)?,
                    StructureIntent {
                        page_uuid: row.get(1)?,
                        parent_uuid: row.get(2)?,
                        order_key: row.get(3)?,
                        hlc: row.get(4)?,
                    },
                ))
            })? {
                result.stats.intents_read += 1;
                let (uuid, intent) = row?;
                add_frontier(&mut frontier, &visited, Some(uuid));
                add_frontier(&mut frontier, &visited, intent.parent_uuid);
                result.intents.insert(uuid, intent);
            }
        }
    }
    result.stats.vertices_visited = visited.len();
    let page_ids = result
        .intents
        .values()
        .map(|intent| intent.page_uuid)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    for batch in page_ids.chunks(UUID_QUERY_CHUNK) {
        let sql = format!(
            "SELECT uuid FROM pages WHERE uuid IN ({})",
            placeholders(batch.len())
        );
        let mut statement = db.prepare_cached(&sql)?;
        for page in
            statement.query_map(params_from_iter(batch.iter()), |row| row.get::<_, Uuid>(0))?
        {
            result.stats.pages_read += 1;
            result.pages.insert(page?);
        }
    }
    Ok(result)
}

fn add_frontier(frontier: &mut BTreeSet<Uuid>, visited: &HashSet<Uuid>, uuid: Option<Uuid>) {
    if let Some(uuid) = uuid
        && !visited.contains(&uuid)
    {
        frontier.insert(uuid);
    }
}

fn placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OrderKey;

    fn uuid(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }

    fn fixture() -> rusqlite::Connection {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE pages(uuid BLOB PRIMARY KEY);
             CREATE TABLE blocks(uuid BLOB PRIMARY KEY, existence_hlc TEXT NOT NULL,
               page_uuid BLOB NOT NULL, parent_uuid BLOB, order_key TEXT NOT NULL,
               structure_hlc TEXT, updated_at INTEGER NOT NULL);
             CREATE INDEX idx_blocks_parent ON blocks(parent_uuid);
             CREATE INDEX idx_blocks_page_parent_order ON blocks(page_uuid, parent_uuid, order_key, uuid);
             CREATE TABLE block_structure_lww(block_uuid BLOB PRIMARY KEY,
               page_uuid BLOB NOT NULL, parent_uuid BLOB, order_key TEXT NOT NULL, hlc TEXT NOT NULL);
             CREATE INDEX idx_structure_parent ON block_structure_lww(parent_uuid, block_uuid);
             CREATE INDEX idx_structure_page ON block_structure_lww(page_uuid, block_uuid);",
        ).unwrap();
        db
    }

    fn page(db: &Transaction<'_>, id: Uuid) {
        db.execute("INSERT INTO pages(uuid) VALUES (?1)", [id])
            .unwrap();
    }

    fn block(db: &Transaction<'_>, id: Uuid, page: Uuid, parent: Option<Uuid>) {
        db.execute(
            "INSERT INTO blocks(uuid, existence_hlc, page_uuid, parent_uuid, order_key, structure_hlc, updated_at)
             VALUES (?1, '1', ?2, ?3, ?4, '2', 0)",
            rusqlite::params![id, page, parent, OrderKey::first()],
        ).unwrap();
    }

    fn intent(db: &Transaction<'_>, id: Uuid, page: Uuid, parent: Option<Uuid>) {
        db.execute(
            "INSERT INTO block_structure_lww(block_uuid, page_uuid, parent_uuid, order_key, hlc)
             VALUES (?1, ?2, ?3, ?4, '2')",
            rusqlite::params![id, page, parent, OrderKey::first()],
        )
        .unwrap();
    }

    fn ids<T>(values: &HashMap<Uuid, T>) -> BTreeSet<Uuid> {
        values.keys().copied().collect()
    }

    #[test]
    fn replaced_edge_preserves_old_and_new_disconnected_components() {
        let mut db = fixture();
        let tx = db.transaction().unwrap();
        page(&tx, uuid(100));
        for (id, parent) in [
            (1, None),
            (2, Some(uuid(1))),
            (3, None),
            (4, Some(uuid(3))),
            (5, Some(uuid(1))),
            (6, None),
        ] {
            block(&tx, uuid(id), uuid(100), parent);
            intent(&tx, uuid(id), uuid(100), parent);
        }
        let mut dirty = StructureDirty::default();
        dirty.touch_block(&tx, uuid(2), Some(uuid(3))).unwrap();
        tx.execute(
            "UPDATE blocks SET parent_uuid = ?2 WHERE uuid = ?1",
            rusqlite::params![uuid(2), uuid(3)],
        )
        .unwrap();
        tx.execute(
            "UPDATE block_structure_lww SET parent_uuid = ?2 WHERE block_uuid = ?1",
            rusqlite::params![uuid(2), uuid(3)],
        )
        .unwrap();
        let loaded = load_components(&tx, &dirty).unwrap();
        assert_eq!(ids(&loaded.existing), [1, 2, 3, 4, 5].map(uuid).into());
        assert_eq!(ids(&loaded.intents), ids(&loaded.existing));
        assert_eq!(loaded.stats.vertices_visited, 5);
        assert_eq!(loaded.stats.seed_rows_read, 2);
        assert!(loaded.stats.blocks_read > loaded.existing.len());
    }

    #[test]
    fn absent_parent_vertex_connects_waiting_children() {
        let mut db = fixture();
        let tx = db.transaction().unwrap();
        page(&tx, uuid(100));
        for id in [1, 2, 3] {
            block(&tx, uuid(id), uuid(100), None);
            intent(&tx, uuid(id), uuid(100), (id != 3).then_some(uuid(9)));
        }
        let mut dirty = StructureDirty::default();
        dirty.touch_block(&tx, uuid(1), None).unwrap();
        let loaded = load_components(&tx, &dirty).unwrap();
        assert_eq!(ids(&loaded.existing), [uuid(1), uuid(2)].into());
        assert_eq!(loaded.stats.vertices_visited, 3);
        assert!(!loaded.existing.contains_key(&uuid(9)));
        assert!(!loaded.intents.contains_key(&uuid(9)));
    }

    #[test]
    fn divergent_raw_and_materialized_parents_both_contribute_dependencies() {
        let mut db = fixture();
        let tx = db.transaction().unwrap();
        page(&tx, uuid(100));
        for id in [1, 2, 3, 4, 5] {
            block(
                &tx,
                uuid(id),
                uuid(100),
                if id == 1 { Some(uuid(2)) } else { None },
            );
            intent(
                &tx,
                uuid(id),
                if id == 1 { uuid(999) } else { uuid(100) },
                if id == 1 { Some(uuid(3)) } else { None },
            );
        }
        tx.execute(
            "UPDATE block_structure_lww SET parent_uuid = ?2 WHERE block_uuid = ?1",
            rusqlite::params![uuid(4), uuid(3)],
        )
        .unwrap();
        let mut dirty = StructureDirty::default();
        dirty.touch_block(&tx, uuid(1), None).unwrap();
        let loaded = load_components(&tx, &dirty).unwrap();
        assert_eq!(ids(&loaded.existing), [1, 2, 3, 4].map(uuid).into());
        assert_eq!(loaded.existing[&uuid(1)].parent_uuid, Some(uuid(2)));
        assert_eq!(loaded.intents[&uuid(1)].parent_uuid, Some(uuid(3)));
        assert_eq!(loaded.pages, [uuid(100)].into());
    }

    #[test]
    fn page_membership_never_connects_independent_roots_or_cross_page_roots() {
        let mut db = fixture();
        let tx = db.transaction().unwrap();
        page(&tx, uuid(100));
        page(&tx, uuid(200));
        for (id, container, parent) in [
            (1, 100, None),
            (2, 100, None),
            (3, 200, None),
            (4, 200, Some(uuid(1))),
        ] {
            block(&tx, uuid(id), uuid(container), None);
            intent(&tx, uuid(id), uuid(container), parent);
        }
        let mut dirty = StructureDirty::default();
        dirty.touch_block(&tx, uuid(1), None).unwrap();
        let loaded = load_components(&tx, &dirty).unwrap();
        assert_eq!(ids(&loaded.existing), [uuid(1), uuid(4)].into());
        assert_eq!(loaded.pages, [uuid(100), uuid(200)].into());
        assert_eq!(loaded.stats.vertices_visited, 2);
    }

    #[test]
    fn weak_closure_loads_complete_cycles_and_reverse_descendants() {
        let mut db = fixture();
        let tx = db.transaction().unwrap();
        page(&tx, uuid(100));
        for (id, parent) in [
            (1, Some(uuid(2))),
            (2, Some(uuid(3))),
            (3, Some(uuid(1))),
            (4, Some(uuid(2))),
            (5, None),
        ] {
            block(&tx, uuid(id), uuid(100), None);
            intent(&tx, uuid(id), uuid(100), parent);
        }
        let mut dirty = StructureDirty::default();
        dirty.touch_block(&tx, uuid(4), None).unwrap();
        let loaded = load_components(&tx, &dirty).unwrap();
        assert_eq!(ids(&loaded.existing), [1, 2, 3, 4].map(uuid).into());
        assert_eq!(ids(&loaded.intents), ids(&loaded.existing));
    }

    #[test]
    fn delete_seeds_materialized_children_before_fk_detachment() {
        let mut db = fixture();
        let tx = db.transaction().unwrap();
        page(&tx, uuid(100));
        block(&tx, uuid(1), uuid(100), None);
        block(&tx, uuid(2), uuid(100), Some(uuid(1)));
        intent(&tx, uuid(1), uuid(100), None);
        intent(&tx, uuid(2), uuid(100), None);
        let mut dirty = StructureDirty::default();
        dirty.deleting_block(&tx, uuid(1)).unwrap();
        tx.execute(
            "UPDATE blocks SET parent_uuid = NULL WHERE parent_uuid = ?1",
            [uuid(1)],
        )
        .unwrap();
        tx.execute("DELETE FROM blocks WHERE uuid = ?1", [uuid(1)])
            .unwrap();
        tx.execute(
            "DELETE FROM block_structure_lww WHERE block_uuid = ?1",
            [uuid(1)],
        )
        .unwrap();
        let loaded = load_components(&tx, &dirty).unwrap();
        assert_eq!(ids(&loaded.existing), [uuid(2)].into());
        assert_eq!(loaded.stats.vertices_visited, 2);
        assert_eq!(loaded.stats.seed_rows_read, 3);
    }

    #[test]
    fn page_touch_preserves_both_membership_sets_and_disconnected_parents() {
        let mut db = fixture();
        let tx = db.transaction().unwrap();
        page(&tx, uuid(100));
        page(&tx, uuid(200));
        for id in [1, 2, 3, 4, 5, 6] {
            block(
                &tx,
                uuid(id),
                if id == 1 { uuid(100) } else { uuid(200) },
                if id == 1 { Some(uuid(3)) } else { None },
            );
            intent(
                &tx,
                uuid(id),
                if id == 2 { uuid(100) } else { uuid(200) },
                if id == 1 {
                    Some(uuid(4))
                } else if id == 2 {
                    Some(uuid(5))
                } else {
                    None
                },
            );
        }
        let mut dirty = StructureDirty::default();
        dirty.touch_page(&tx, uuid(100)).unwrap();
        tx.execute("DELETE FROM blocks WHERE uuid = ?1", [uuid(1)])
            .unwrap();
        tx.execute(
            "DELETE FROM block_structure_lww WHERE block_uuid = ?1",
            [uuid(1)],
        )
        .unwrap();
        let loaded = load_components(&tx, &dirty).unwrap();
        assert_eq!(ids(&loaded.existing), [2, 3, 4, 5].map(uuid).into());
        assert_eq!(loaded.stats.vertices_visited, 5);
        assert_eq!(loaded.stats.seed_rows_read, 2);
        assert!(!loaded.existing.contains_key(&uuid(6)));
    }

    #[test]
    fn page_cascade_keeps_surviving_child_with_divergent_unavailable_intent() {
        let mut db = fixture();
        let tx = db.transaction().unwrap();
        page(&tx, uuid(100));
        page(&tx, uuid(200));
        block(&tx, uuid(1), uuid(100), None);
        intent(&tx, uuid(1), uuid(100), None);
        block(&tx, uuid(2), uuid(200), Some(uuid(1)));
        intent(&tx, uuid(2), uuid(999), Some(uuid(3)));
        block(&tx, uuid(3), uuid(200), None);
        intent(&tx, uuid(3), uuid(200), None);
        let mut dirty = StructureDirty::default();
        dirty.touch_page(&tx, uuid(100)).unwrap();
        dirty.deleting_block(&tx, uuid(1)).unwrap();
        tx.execute(
            "UPDATE blocks SET parent_uuid = NULL WHERE parent_uuid = ?1",
            [uuid(1)],
        )
        .unwrap();
        tx.execute("DELETE FROM blocks WHERE uuid = ?1", [uuid(1)])
            .unwrap();
        tx.execute(
            "DELETE FROM block_structure_lww WHERE block_uuid = ?1",
            [uuid(1)],
        )
        .unwrap();
        tx.execute("DELETE FROM pages WHERE uuid = ?1", [uuid(100)])
            .unwrap();
        let loaded = load_components(&tx, &dirty).unwrap();
        assert_eq!(ids(&loaded.existing), [uuid(2), uuid(3)].into());
        assert_eq!(loaded.existing[&uuid(2)].parent_uuid, None);
        assert_eq!(loaded.intents[&uuid(2)].parent_uuid, Some(uuid(3)));
        assert_eq!(loaded.pages, [uuid(200)].into());
        assert_eq!(loaded.stats.vertices_visited, 3);
        assert_eq!(loaded.stats.seed_rows_read, 5);
    }

    #[test]
    fn wide_component_chunks_uuid_queries_and_counts_all_rows() {
        let mut db = fixture();
        let tx = db.transaction().unwrap();
        page(&tx, uuid(10_000));
        for id in 1..=600 {
            block(&tx, uuid(id), uuid(10_000), None);
            intent(&tx, uuid(id), uuid(10_000), (id != 1).then_some(uuid(1)));
        }
        let mut dirty = StructureDirty::default();
        dirty.touch_block(&tx, uuid(1), None).unwrap();
        let loaded = load_components(&tx, &dirty).unwrap();
        assert_eq!(loaded.existing.len(), 600);
        assert_eq!(loaded.intents.len(), 600);
        assert_eq!(loaded.stats.vertices_visited, 600);
        assert_eq!(loaded.stats.blocks_read, 600);
        assert_eq!(loaded.stats.intents_read, 1_199);
        assert_eq!(loaded.stats.pages_read, 1);
        assert_eq!(loaded.stats.seed_rows_read, 2);
    }

    #[test]
    fn unrelated_roots_do_not_increase_reads_for_243_touched_siblings() {
        let expected = StructureLoadStats {
            blocks_read: 243,
            intents_read: 243,
            pages_read: 1,
            vertices_visited: 243,
            seed_rows_read: 486,
        };
        for unrelated in [1_000, 10_000] {
            let mut db = fixture();
            let tx = db.transaction().unwrap();
            page(&tx, uuid(100_000));
            for id in 1..=(243 + unrelated) {
                block(&tx, uuid(id), uuid(100_000), None);
                intent(&tx, uuid(id), uuid(100_000), None);
            }
            let mut dirty = StructureDirty::default();
            for id in 1..=243 {
                dirty.touch_block(&tx, uuid(id), None).unwrap();
            }
            let loaded = load_components(&tx, &dirty).unwrap();
            assert_eq!(loaded.existing.len(), 243);
            assert_eq!(loaded.intents.len(), 243);
            assert_eq!(loaded.stats, expected, "unrelated roots: {unrelated}");
        }
    }

    #[test]
    fn empty_dirty_has_no_vertices_or_rows() {
        let mut db = fixture();
        let tx = db.transaction().unwrap();
        let dirty = StructureDirty::default();
        assert!(dirty.is_empty());
        let loaded = load_components(&tx, &dirty).unwrap();
        assert_eq!(loaded.stats, StructureLoadStats::default());
        assert!(loaded.existing.is_empty());
        assert!(loaded.intents.is_empty());
        assert!(loaded.pages.is_empty());
    }
}
