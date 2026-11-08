CREATE TABLE sessions (
    id TEXT PRIMARY KEY NOT NULL,
    starter INTEGER NOT NULL CHECK (starter IN (0,1)),
    activity INTEGER NOT NULL,
    data TEXT NOT NULL CHECK (json_valid(data)),
    queue TEXT NOT NULL CHECK (json_valid(queue))
);
CREATE UNIQUE INDEX one_starter ON sessions(starter) WHERE starter=1;
CREATE INDEX session_activity ON sessions(activity DESC,id);
CREATE INDEX session_parent ON sessions(json_extract(data,'$.parent_id'));
CREATE TABLE entries (
    position INTEGER PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    id TEXT NOT NULL,
    kind TEXT NOT NULL,
    data TEXT NOT NULL CHECK (json_valid(data)),
    UNIQUE (session_id,id)
);
CREATE INDEX history_order ON entries(session_id,position);
CREATE INDEX history_kind ON entries(session_id,kind,position DESC);
-- The display projection excludes provider-private payloads and image bytes.
-- It is written in the same transaction as its source entry, never independently.
CREATE TABLE events (
    session_id TEXT NOT NULL,
    position INTEGER NOT NULL,
    id TEXT NOT NULL,
    entry_id TEXT NOT NULL,
    data TEXT NOT NULL CHECK (json_valid(data)),
    PRIMARY KEY (session_id,position),
    UNIQUE (session_id,id),
    FOREIGN KEY (session_id,entry_id) REFERENCES entries(session_id,id) ON DELETE CASCADE
);
CREATE TABLE receipts (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    request_id TEXT NOT NULL,
    data TEXT NOT NULL CHECK (json_valid(data)),
    PRIMARY KEY (session_id,request_id)
);
CREATE TABLE queue (
    session_id TEXT NOT NULL,
    request_id TEXT NOT NULL,
    position INTEGER NOT NULL,
    data TEXT NOT NULL CHECK (json_valid(data)),
    PRIMARY KEY (session_id,request_id),
    FOREIGN KEY (session_id,request_id) REFERENCES receipts(session_id,request_id) ON DELETE CASCADE
);
CREATE INDEX queue_order ON queue(session_id,position);
PRAGMA user_version=1;
