-- Local replacement generation, committed atomically with bulk content changes.
-- This never travels in replication envelopes and is not an ink revision.
ALTER TABLE store_state ADD COLUMN workspace_epoch INTEGER NOT NULL DEFAULT 0
    CHECK (workspace_epoch >= 0);
