-- Local structural history pins immutable drawing bodies independently of the
-- live page. A page delete can purge its live ink rows without erasing Undo.
CREATE TABLE history_ink_snapshots (
  action_uuid BLOB NOT NULL CHECK(length(action_uuid)=16),
  page_uuid BLOB NOT NULL CHECK(length(page_uuid)=16),
  metadata_json TEXT NOT NULL,
  metadata_hash BLOB NOT NULL CHECK(length(metadata_hash)=32),
  PRIMARY KEY(action_uuid,page_uuid)
) WITHOUT ROWID;

CREATE TABLE history_ink_roots (
  action_uuid BLOB NOT NULL,
  page_uuid BLOB NOT NULL,
  root_id BLOB NOT NULL REFERENCES ink_records(id),
  PRIMARY KEY(action_uuid,page_uuid,root_id),
  FOREIGN KEY(action_uuid,page_uuid)
    REFERENCES history_ink_snapshots(action_uuid,page_uuid) ON DELETE CASCADE
) WITHOUT ROWID;
