PRAGMA journal_mode=WAL;

-- An epic is not a task: it is never claimed, leased or dispatched. It names the
-- project the work belongs to (`path`), so a manager can tell when a pane rooted
-- elsewhere is about to produce a diff in the wrong repo. Printed E1, E2...
CREATE TABLE IF NOT EXISTS epics(
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  title        TEXT NOT NULL,
  description  TEXT NOT NULL DEFAULT '',
  goal         TEXT NOT NULL DEFAULT '',
  status       TEXT NOT NULL DEFAULT 'open',  -- open|done|dropped
  path         TEXT NOT NULL DEFAULT '',      -- absolute project root; '' = not repo-bound
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL,
  completed_at INTEGER
);

-- One row per unit of work. Fields a worker needs are in the row, not in a
-- prose blob: description says where the code is and what the situation is,
-- goal says how the work is judged finished.
CREATE TABLE IF NOT EXISTS tasks(
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  board        TEXT NOT NULL DEFAULT 'main',
  title        TEXT NOT NULL,
  description  TEXT NOT NULL DEFAULT '',      -- context: repo, files, situation
  goal         TEXT NOT NULL DEFAULT '',      -- how to do it + definition of done
  status       TEXT NOT NULL DEFAULT 'ready', -- ready|working|blocking|needs_fix|review|done|dropped
  assignee     TEXT,
  priority     INTEGER NOT NULL DEFAULT 100,
  epic_id      INTEGER REFERENCES epics(id),   -- the epic the work belongs to
  parent_id    INTEGER REFERENCES tasks(id),   -- subtask link: a fix task under the review task that found it
  needs        TEXT,                          -- 'input' while blocking on a human
  report       TEXT,                          -- worker's findings/outcome, written any time
  note         TEXT,                          -- last progress note (history lives in events)
  commit_ref   TEXT,
  lease_until  INTEGER,                       -- unix ms; stale lease => reclaimable
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL,
  started_at   INTEGER,                       -- set on first claim
  completed_at INTEGER                        -- set on done/dropped
);
CREATE INDEX IF NOT EXISTS tasks_status ON tasks(status, priority, id);
CREATE INDEX IF NOT EXISTS tasks_lease ON tasks(lease_until);
CREATE INDEX IF NOT EXISTS tasks_assignee ON tasks(assignee, status);
CREATE INDEX IF NOT EXISTS tasks_epic ON tasks(epic_id);
CREATE INDEX IF NOT EXISTS tasks_parent ON tasks(parent_id);

CREATE TABLE IF NOT EXISTS events(
  seq     INTEGER PRIMARY KEY AUTOINCREMENT,
  task_id INTEGER REFERENCES tasks(id),   -- exactly one of task_id / epic_id is set
  epic_id INTEGER REFERENCES epics(id),
  actor   TEXT NOT NULL,
  kind    TEXT NOT NULL,          -- add|claim|note|report|answer|block|review|done|needs_fix|edit|dispatch
  body    TEXT NOT NULL DEFAULT '',
  ts      INTEGER NOT NULL,
  CHECK ((task_id IS NULL) != (epic_id IS NULL))
);
CREATE INDEX IF NOT EXISTS ev_task ON events(task_id, seq);
CREATE INDEX IF NOT EXISTS ev_epic ON events(epic_id, seq);

CREATE TABLE IF NOT EXISTS sessions(
  name    TEXT PRIMARY KEY,       -- tmux session name
  agent   TEXT NOT NULL,          -- claude|hermes|human
  cwd     TEXT,
  task_id INTEGER REFERENCES tasks(id),
  seen    INTEGER NOT NULL,
  status  TEXT NOT NULL DEFAULT 'up'
);
