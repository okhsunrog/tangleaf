-- Local notification order for commands migrated to NotesStore. This is not
-- the replication cursor or a database-wide revision while legacy writers exist.
CREATE TABLE store_commit_clock (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    version INTEGER NOT NULL CHECK (version >= 0)
);
INSERT INTO store_commit_clock(singleton, version) VALUES (1, 0);
