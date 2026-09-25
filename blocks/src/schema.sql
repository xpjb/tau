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
CREATE TABLE IF NOT EXISTS block_usage (singleton INTEGER PRIMARY KEY CHECK(singleton=1), bytes INTEGER NOT NULL, clock INTEGER NOT NULL DEFAULT 0);
INSERT OR IGNORE INTO block_usage(singleton,bytes) SELECT 1,(SELECT coalesce(sum(length(data)),0) FROM block_chunks) WHERE NOT EXISTS(SELECT 1 FROM block_usage);
CREATE TRIGGER IF NOT EXISTS block_bytes_insert AFTER INSERT ON block_chunks BEGIN
    UPDATE block_usage SET bytes=bytes+length(NEW.data) WHERE singleton=1;
END;
CREATE TRIGGER IF NOT EXISTS block_bytes_delete AFTER DELETE ON block_chunks BEGIN
    UPDATE block_usage SET bytes=bytes-length(OLD.data) WHERE singleton=1;
END;
CREATE TRIGGER IF NOT EXISTS block_bytes_update AFTER UPDATE OF data ON block_chunks BEGIN
    UPDATE block_usage SET bytes=bytes+length(NEW.data)-length(OLD.data) WHERE singleton=1;
END;
CREATE TABLE IF NOT EXISTS block_cache_access (scope TEXT NOT NULL,id TEXT NOT NULL,touched INTEGER NOT NULL,PRIMARY KEY(scope,id),FOREIGN KEY(scope,id) REFERENCES blocks(scope,id) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS block_metadata_usage(singleton INTEGER PRIMARY KEY CHECK(singleton=1),headers INTEGER NOT NULL,bytes INTEGER NOT NULL);
INSERT OR IGNORE INTO block_metadata_usage SELECT 1,(SELECT count(*) FROM blocks),(SELECT coalesce(sum(length(CAST(header AS BLOB))),0) FROM blocks) WHERE NOT EXISTS(SELECT 1 FROM block_metadata_usage);
CREATE TABLE IF NOT EXISTS block_limits(singleton INTEGER PRIMARY KEY CHECK(singleton=1),headers INTEGER NOT NULL,bytes INTEGER NOT NULL);
INSERT OR IGNORE INTO block_limits VALUES(1,250000,268435456);
CREATE TRIGGER IF NOT EXISTS block_header_insert AFTER INSERT ON blocks BEGIN
 UPDATE block_metadata_usage SET headers=headers+1,bytes=bytes+length(CAST(NEW.header AS BLOB));
 SELECT CASE WHEN EXISTS(SELECT 1 FROM block_metadata_usage u JOIN block_limits l ON u.singleton=l.singleton WHERE u.headers>l.headers OR u.bytes>l.bytes) THEN RAISE(ABORT,'Block metadata quota reached; archive/delete old chats or clear replica cache') END;
END;
CREATE TRIGGER IF NOT EXISTS block_header_update AFTER UPDATE OF header ON blocks BEGIN
 UPDATE block_metadata_usage SET bytes=bytes+length(CAST(NEW.header AS BLOB))-length(CAST(OLD.header AS BLOB));
 SELECT CASE WHEN EXISTS(SELECT 1 FROM block_metadata_usage u JOIN block_limits l ON u.singleton=l.singleton WHERE u.bytes>l.bytes) THEN RAISE(ABORT,'Block metadata quota reached; archive/delete old chats or clear replica cache') END;
END;
CREATE TRIGGER IF NOT EXISTS block_header_delete AFTER DELETE ON blocks BEGIN
 UPDATE block_metadata_usage SET headers=headers-1,bytes=bytes-length(CAST(OLD.header AS BLOB));
END;
CREATE TRIGGER IF NOT EXISTS block_tombstone_limit AFTER INSERT ON block_cache_tombstones BEGIN
 SELECT CASE WHEN (SELECT count(*) FROM block_cache_tombstones)>100000 THEN RAISE(ABORT,'Replica tombstone quota reached; clear replica cache') END;
END;
