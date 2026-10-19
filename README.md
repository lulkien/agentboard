# agentboard

A task board shared by a handful of CLI coding agents (Claude Code in tmux,
Hermes, subagents) plus you. 12 MCP tools over stdio, one SQLite file, no
server to run. The point is to replace "write status to NOTES.md and hope
somebody reads it" with a board that answers in one line and costs few tokens.

## Model

Three tables (`src/schema.sql`):

    tasks     id, board, title, description, goal, status, assignee, priority, parent,
              needs, report, commit_ref, lease_until, created_at, updated_at,
              started_at, completed_at
    events    seq, task_id, actor, kind, body, ts        -- append-only report log
    sessions  name, agent, cwd, task_id, seen, status    -- tmux registry

Statuses: `ready → working → (blocking → working)* → review → done`,
plus `needs_fix` (review found work, waiting for a fix; shown as `fix`) and
`dropped`. `needs='input'` marks "blocked on a human", which is the only
thing that should ever interrupt you. Input aliases `running`→`working`,
`blocked`→`blocking`, `triage`→`ready`, `need fix`/`needs-fix`/`fix`→`needs_fix` accepted on input.

`events` is the file you used to write. Every note, question, answer, claim and
completion is one row; nothing is mutable history.

## Tools

| tool | cost | what it does |
|---|---|---|
| `task_add` | 1 line | create; returns `T7` (takes `title, description, goal, status, assignee, priority, parent`) |
| `task_list` | 1 line/task | open work, ordered by urgency |
| `task_get` | ~10 lines | description, goal, timestamps, lease, report, note, last 8 events |
| `task_children` | 1 line/child | direct children of an epic: id, status, assignee, age, title |
| `task_claim` | work order | atomic claim, **returns description + goal** so no follow-up `get` |
| `task_update` | 1 line ack | note and/or status; `report=` when finishing |
| `task_ask` | 1 line ack | block on a human decision |
| `task_wait` | delta only | server-side long-poll for the answer |
| `board_brief` | ~1 screen | counts + open tasks + `EPIC` rollup lines (done/non-dropped children, open epics only) + recent completions |
| `session_list` | 1 line/session | registered tmux sessions and their task |
| `session_register` | ack | map session → task |
| `session_tail` | N lines | last N lines of a tmux pane |
| `session_send` | ack | type a line into a pane (steer another agent) |

Twelve tools is deliberate: every tool schema is resent on every request, so a
wide surface is a permanent token tax. `answer` is not a tool — it is
`task_update(status="running", note=...)`.

## Why it is shaped this way

- **stdio, not HTTP.** Each client spawns its own process; there is no daemon,
  no port, no session id. Concurrent writers are fine because SQLite is the
  shared state (WAL, `busy_timeout=5s`).
- **Claim = `BEGIN IMMEDIATE` + guarded `UPDATE`.** The guard is on
  `lease_until`, so the loser of a race retries instead of double-assigning.
  Measured: 8 workers, 80 tasks, 0 duplicates, 116 ms total, ~1.4 ms per
  cold-process claim.
- **Leases, not heartbeats-by-default.** A claim expires (30 min default) and
  becomes reclaimable; a dead session cannot wedge the board. `heartbeat=true`
  extends it.
- **Short acks.** Writes answer `ok T7 done #14`, not the serialized task.
  Reads are one line per task. `task_get` is the only verbose call.
- **No agent-side polling.** `task_wait` blocks in the server (200 ms internal
  poll) and returns only the delta, so an agent waiting 10 minutes spends one
  tool call, not forty.
- **One implementation, two front doors.** `src/store.rs` is used by both the
  MCP handlers and the CLI. Shell hooks (Claude Code `Stop`/`PreToolUse`) can
  call `agentboard note` with zero model tokens.

## Build and install

    cd ~/Projects/agentboard
    cargo build --release            # ~4 s incremental, SQLite bundled

Hermes (`~/.hermes/config.yaml`):

    mcp_servers:
      agentboard:
        command: /home/kien1.luu/Projects/agentboard/target/release/agentboard
        env:
          AGENTBOARD_ACTOR: hermes
        enabled: true

Claude Code, one session per tmux pane:

    claude mcp add agentboard --env AGENTBOARD_ACTOR=claude1 \
      -- /home/kien1.luu/Projects/agentboard/target/release/agentboard

Then register the pane once so the board can map task → session:

    agentboard register claude1 --agent claude --task T7

## CLI

    agentboard add|ls|get|children|claim|note|ask|answer|done|review|block|brief
    agentboard wait T7 [--after 42] [--timeout 60]
    agentboard sessions|register|tail|send
    agentboard path                    # where the db lives

Global: `--actor NAME`, `--db PATH` (or `$AGENTBOARD_DB`).
Default db: `$XDG_STATE_HOME/agentboard/board.db`.

## Tests

    python3 tests/mcp_client.py   # 21 checks over real stdio: claim, ask→wait, acks
    ./tests/race.sh               # 8 concurrent claimers: 80 claims, 80 unique, ~124 ms
    bash tests/wait.sh            # long-poll returns on the event, not the timeout
    ./target/release/agentboard < tests/mcp-smoke.jsonl   # handshake + tool list

Cold start: ~5 MB RSS, `brief` in <10 ms. One claim, process included, is ~1.4 ms.

## Field test (real Claude Code, sonnet)

Two tasks driven end to end through MCP only, no status files written:

    t1  claim -> read -> fix -> python3 test_calc.py -> done   10 turns, 35 s, $0.088
    t2  claim -> ask -> wait 13.4 s -> answer -> implement -> review   9 turns, 35 s, $0.083

`task_wait` returned 176 ms after the answer landed, and the agent passed the
ack's `#seq` back as `after_seq` — the seq round-trip is the whole protocol.
The client tolerated a 13 s tool call, so the 300 s cap is not a client limit.
Claude Code's deferred tool loading adds one `ToolSearch` call before the first
MCP call; that is client-side, not a schema-size problem.


Note: MCP requests are handled concurrently. A client that fires `task_add` and
`task_claim` in the same parallel batch can lose that race — the board is
correct, the ordering is the client's.

## Not in scope

No web UI, sprints, story points, attachments, or multi-user auth. One board,
one user, a few agents. If it needs more than 12 tools, it is not this tool.

## Do agents need a skill for this?

No for the mechanics: the tool descriptions were enough in a field test — a bare
"you have work waiting on this machine" prompt produced `task_list` →
`task_claim` → edit → verify → `task_update(review)` with no guidance, and no
status file was written. The surface is ~1.8k tokens.

Yes for the conventions, which no schema can express: board is the only status
channel, claim the next task, never guess a missing requirement, what counts as
done. Those nine rules live in `PROTOCOL.md` — paste them into `CLAUDE.md` or
`AGENTS.md` once per worker. A skill is for the orchestrator (dispatch, answer
asks, read `board_brief`), not for the worker.

