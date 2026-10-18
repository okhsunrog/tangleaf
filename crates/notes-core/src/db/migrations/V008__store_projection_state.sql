-- Local runtime state is separate from replicated content and sync cursors.
-- Zero means startup must establish the structural projection before enabling
-- component updates; only the successful rebuild transaction advances it.
ALTER TABLE store_commit_clock RENAME TO store_state;
ALTER TABLE store_state ADD COLUMN structure_projection_version INTEGER NOT NULL
  DEFAULT 0 CHECK (structure_projection_version >= 0);
