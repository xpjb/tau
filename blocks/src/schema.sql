CREATE TABLE IF NOT EXISTS block_state (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    lineage TEXT NOT NULL,
    sequence INTEGER NOT NULL DEFAULT 0,
    retained_from INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS blocks (
    scope TEXT NOT NULL,
    id TEXT NOT NULL,
    parent TEXT NOT NULL,
    position INTEGER NOT NULL,
    header TEXT NOT NULL,
    PRIMARY KEY(scope,id)
);
CREATE INDEX IF NOT EXISTS block_parent ON blocks(scope,parent,position,id);
CREATE TABLE IF NOT EXISTS block_chunks (hash TEXT PRIMARY KEY, data BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS block_parts (
    scope TEXT NOT NULL,
    id TEXT NOT NULL,
    version INTEGER NOT NULL,
    offset INTEGER NOT NULL,
    hash TEXT NOT NULL REFERENCES block_chunks(hash),
    PRIMARY KEY(scope,id,version,offset),
    FOREIGN KEY(scope,id) REFERENCES blocks(scope,id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS block_part_hash ON block_parts(hash);
CREATE TABLE IF NOT EXISTS block_changes (
    sequence INTEGER PRIMARY KEY,
    scope TEXT NOT NULL,
    parent TEXT NOT NULL,
    id TEXT NOT NULL,
    position INTEGER NOT NULL,
    record TEXT NOT NULL,
    UNIQUE(scope,parent,id)
);
CREATE INDEX IF NOT EXISTS block_change_feed ON block_changes(scope,parent,sequence);
CREATE TABLE IF NOT EXISTS block_cache_feeds (
    scope TEXT NOT NULL,
    parent TEXT NOT NULL,
    page TEXT NOT NULL,
    PRIMARY KEY(scope,parent)
);
CREATE TRIGGER IF NOT EXISTS block_part_delete_gc AFTER DELETE ON block_parts BEGIN
    DELETE FROM block_chunks WHERE hash=OLD.hash AND NOT EXISTS(SELECT 1 FROM block_parts WHERE hash=OLD.hash);
END;
CREATE TRIGGER IF NOT EXISTS block_part_update_gc AFTER UPDATE OF hash ON block_parts WHEN OLD.hash != NEW.hash BEGIN
    DELETE FROM block_chunks WHERE hash=OLD.hash AND NOT EXISTS(SELECT 1 FROM block_parts WHERE hash=OLD.hash);
END;
CREATE TABLE IF NOT EXISTS block_cache_tombstones (
    scope TEXT NOT NULL,
    id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    PRIMARY KEY(scope,id)
);
