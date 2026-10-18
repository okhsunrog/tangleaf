-- Pending and quarantined publications own their binary graph independently
-- of visible pages, version heads, native history and structural Undo.
-- NULL records a root whose bytes were already unavailable at activation or
-- whose metadata arrived before its validated graph.
CREATE TABLE ink_outbox_roots (
  op_id BLOB PRIMARY KEY REFERENCES sync_outbox(op_id) ON DELETE CASCADE,
  root_hash BLOB NOT NULL CHECK(length(root_hash)=32),
  root_id BLOB REFERENCES ink_records(id)
) WITHOUT ROWID;
CREATE INDEX ink_outbox_roots_hash ON ink_outbox_roots(root_hash);
CREATE INDEX ink_outbox_roots_record ON ink_outbox_roots(root_id) WHERE root_id IS NOT NULL;
