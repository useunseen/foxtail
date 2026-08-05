-- Singleton persisted release-qualification realization.  The canonical
-- document bytes are retained so every read returns the exact published
-- content rather than reserializing a mutable in-memory structure.
CREATE TABLE fixture_realizations (
    singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
    definition_bytes BLOB NOT NULL,
    definition_digest TEXT NOT NULL,
    manifest_bytes BLOB NOT NULL,
    manifest_digest TEXT NOT NULL,
    generation INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
