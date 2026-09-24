CREATE TABLE projects (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    prompt TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 0
);
INSERT INTO projects(id,name,prompt) VALUES('general','General','');
UPDATE sessions SET data=json_set(data,'$.project_id','general');
DROP INDEX one_starter;
CREATE UNIQUE INDEX one_starter ON sessions(json_extract(data,'$.project_id')) WHERE starter=1;
CREATE INDEX session_project ON sessions(json_extract(data,'$.project_id'));
PRAGMA user_version=2;
