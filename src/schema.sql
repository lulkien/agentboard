-- Publish the complete schema and stamp together, including on first use.
BEGIN IMMEDIATE;

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
  path         TEXT NOT NULL DEFAULT '',      -- absolute project root; '' = not repo-bound
  branch       TEXT NOT NULL DEFAULT 'HEAD',  -- git ref the worker starts on; HEAD = leave the checkout alone
  status       TEXT NOT NULL DEFAULT 'ready', -- ready|working|blocking|not_done|needs_fix|review|done|dropped
  assignee     TEXT,
  priority     INTEGER NOT NULL DEFAULT 100,
  epic_id      INTEGER REFERENCES epics(id),   -- the epic the work belongs to
  parent_id    INTEGER REFERENCES tasks(id),   -- subtask link: a fix task under the review task that found it
  reopen_count INTEGER NOT NULL DEFAULT 0,    -- moves into needs_fix; over 3 the row lands not_done
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
  kind    TEXT NOT NULL,          -- add|claim|note|report|answer|block|review|done|needs_fix|edit|dispatch|diff
  body    TEXT NOT NULL DEFAULT '',
  ts      INTEGER NOT NULL,
  CHECK ((task_id IS NULL) != (epic_id IS NULL))
);
CREATE INDEX IF NOT EXISTS ev_task ON events(task_id, seq);

-- What a coder submitted for review, verbatim and append-only: no edit, no delete, and
-- a row outlives its task being dropped. `round` is the task's reopen_count + 1 at the
-- time. Printed D1, D2... `reviews` makes an evidence-only submission machine-checkable:
-- non-NULL names the diff this round re-runs, and the body is evidence, never a patch.
CREATE TABLE IF NOT EXISTS diffs(
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  task_id    INTEGER NOT NULL REFERENCES tasks(id),
  round      INTEGER NOT NULL,
  body       TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL,
  reviews    INTEGER
);
CREATE INDEX IF NOT EXISTS diffs_task ON diffs(task_id, id);
CREATE INDEX IF NOT EXISTS ev_epic ON events(epic_id, seq);

CREATE TABLE IF NOT EXISTS sessions(
  name    TEXT PRIMARY KEY,       -- tmux session name
  agent   TEXT NOT NULL,          -- claude|hermes|human
  cwd     TEXT,
  task_id INTEGER REFERENCES tasks(id),
  seen    INTEGER NOT NULL,
  status  TEXT NOT NULL DEFAULT 'up'
);

-- Stamp protocol generation. Existing unstamped files are refused before DDL.
CREATE TABLE IF NOT EXISTS board_stamp(
  id INTEGER PRIMARY KEY CHECK(id=1),
  generation INTEGER NOT NULL,
  surface_tools INTEGER,
  opened_by TEXT NOT NULL DEFAULT '',
  last_writer TEXT NOT NULL DEFAULT ''
);
INSERT OR IGNORE INTO board_stamp(id,generation) VALUES(1,2);

COMMIT;
