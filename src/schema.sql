PRAGMA journal_mode=WAL;

CREATE TABLE IF NOT EXISTS tasks(
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  board       TEXT NOT NULL DEFAULT 'main',
  title       TEXT NOT NULL,
  spec        TEXT NOT NULL DEFAULT '',
  status      TEXT NOT NULL DEFAULT 'ready',  -- triage|ready|running|blocked|review|done|dropped
  assignee    TEXT,
  priority    INTEGER NOT NULL DEFAULT 100,
  parent      INTEGER REFERENCES tasks(id),
  needs       TEXT,                            -- 'input' when waiting on a human
  result      TEXT,
  commit_ref  TEXT,
  lease_until INTEGER,                         -- unix ms; stale lease => reclaimable
  created     INTEGER NOT NULL,
  updated     INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS events(
  seq     INTEGER PRIMARY KEY AUTOINCREMENT,
  task_id INTEGER NOT NULL REFERENCES tasks(id),
  actor   TEXT NOT NULL,
  kind    TEXT NOT NULL,          -- claim|note|ask|answer|block|review|done|reclaim|edit
  body    TEXT NOT NULL DEFAULT '',
  ts      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ev_task ON events(task_id, seq);

CREATE TABLE IF NOT EXISTS sessions(
  name    TEXT PRIMARY KEY,       -- tmux session name
  agent   TEXT NOT NULL,          -- claude|hermes|human
  cwd     TEXT,
  task_id INTEGER REFERENCES tasks(id),
  seen    INTEGER NOT NULL,
  status  TEXT NOT NULL DEFAULT 'up'
);
