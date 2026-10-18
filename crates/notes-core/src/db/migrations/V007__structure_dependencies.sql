-- Find parent components and intents waiting for a page without scanning the
-- workspace. Materialized-parent/page lookups already have indexes on blocks.
CREATE INDEX idx_structure_parent
  ON block_structure_lww(parent_uuid, block_uuid);
CREATE INDEX idx_structure_page
  ON block_structure_lww(page_uuid, block_uuid);
