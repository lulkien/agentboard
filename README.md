# agentboard

A task board shared by a handful of CLI coding agents (Claude Code in a herdr
pane, Hermes, subagents) plus you. 19 MCP tools over stdio and one board, stored
either in a single SQLite file (default) or in SurrealDB. The point is to replace
"write status to NOTES.md and hope somebody reads it" with a board that answers in
one line and costs few tokens.

## Model

Four kinds of record — four tables in SQLite (`src/schema.sql`), four record types in
SurrealDB (`src/store/surreal/`):

    epics     id, title, description, goal, status, path, created_at, updated_at,
              completed_at                               -- E1, E2...; never leased
    tasks     id, board, title, description, goal, status, assignee, priority, epic_id,
              parent_id, needs, report, commit_ref, lease_until, created_at, updated_at,
              started_at, completed_at
    events    seq, task_id | epic_id, actor, kind, body, ts  -- append-only, one timeline
    sessions  name, agent, cwd, task_id, seen, status    -- live panes: herdr or tmux

An **epic** is not a task: it has its own ids (`E1`), is never claimed, leased or
dispatched, and carries the project `path` its work belongs to. A task points at an
epic (`epic_id`) and optionally at another task (`parent_id`, a subtask that
inherits the parent's epic; cycles and cross-epic parents are refused). A board
file written before epics is refused with the command to move it aside: there is
no migration.

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
| `task_add` | 1 line | create; returns `T7` (takes `title, description, goal, status, assignee, priority, epic, parent`) |
| `task_list` | 1 line/task | open work, ordered by urgency |
| `task_get` | ~10 lines | description, goal, timestamps, lease, report, note, last 8 events; long fields end in a `[truncated <field>: N of M chars ...]` marker, `full=true` returns them (and event bodies) whole |
| `task_children` | 1 line/task | an epic id (`E3`) lists every task in it, a task id (`T7`) its subtasks: id, status, assignee, age, title |
| `epic_add` | 1-2 lines | create an epic (`title, description, goal, path`); returns `E3`, plus a warning line if `path` does not exist yet |
| `epic_list` | 1 line/epic | epics (default open): id, status, done/total, path, title |
| `epic_get` | ~10 lines | status, path, task counts, description, goal, last 8 events; `full=true` unclipped |
| `epic_update` | 1 line ack | set an epic `open`/`done`/`dropped`; closing is never automatic |
| `epic_edit` | 1 line ack | amend title, description, goal, path (only the fields passed); logs old/new lengths |
| `task_edit` | 1 line ack | amend title, description, goal, priority, assignee; refused while `working` with a live lease, because the worker is reading that text |
| `task_claim` | work order | atomic claim, **returns description + goal** so no follow-up `get` |
| `task_update` | 1 line ack | note and/or status; `report=` when finishing |
| `task_ask` | 1 line ack | block on a human decision |
| `task_wait` | delta only | server-side long-poll over `id`/`ids`/`epic`/`open` with `status`/`kind` filters; first line `matched:` or `timeout:` |
| `board_brief` | ~1 screen | counts + one `EPIC` line per open epic (done/non-dropped tasks, blocking/needs_fix counts, path) + open tasks that belong to no epic + recent completions |
| `session_list` | 1 line/session | registered sessions and their task; `up` means a live tmux session of that name, or a herdr agent with that name and the registered agent type |
| `session_register` | ack | map session → task |
| `session_tail` | N lines | last N lines of the agent's pane, herdr first and tmux as fallback; bounded by `$AGENTBOARD_DELIVER_TIMEOUT_MS` |
| `session_send` | ack | type a line into a pane (steer another agent) |

Nineteen tools is deliberate: every tool schema is resent on every request, so a
wide surface is a permanent token tax. `answer` is not a tool — it is
`task_update(status="running", note=...)`.

## Why it is shaped this way

- **stdio, not HTTP.** Each client spawns its own process; there is no session id.
  Table stakes differ per backend: with SQLite the file is the shared state (WAL,
  `busy_timeout=5s`); with SurrealDB every process talks to one local daemon over
  `127.0.0.1:18720`, which the first caller starts and the rest reuse.
- **Claim = one guarded write.** SQLite wraps a `BEGIN IMMEDIATE` and a guarded
  `UPDATE`; SurrealDB runs the same guarded statement against the server. The guard
  is on `lease_until`, so the loser of a race retries instead of double-assigning.
  Measured: 8 workers, 80 tasks, 0 duplicates, 116 ms total, ~1.4 ms per
  cold-process claim. An *embedded* SurrealKV directory would break this — four
  processes each claimed all eight tasks — which is why the daemon is required.
- **Leases, not heartbeats-by-default.** A claim expires (30 min default) and
  becomes reclaimable; a dead session cannot wedge the board. `heartbeat=true`
  extends it.
- **Short acks.** Writes answer `ok T7 done #14`, not the serialized task.
  Reads are one line per task. `task_get` is the only verbose call.
- **No agent-side polling.** `task_wait` blocks in the server (200 ms internal
  poll) and returns only the delta, so an agent waiting 10 minutes spends one
  tool call, not forty.
- **One implementation, two front doors, two backends.** `src/store/board.rs` is the
  trait; `src/store/sqlite/` and `src/store/surreal/` implement it; the MCP handlers
  and the CLI both go through it, so a rule exists once. Shell hooks (Claude Code `Stop`/`PreToolUse`) can
  call `agentboard note` with zero model tokens.

## Storage backends

Two exclusive build features. A binary carries exactly one backend, chosen at compile
time; building with neither or with both fails on one `compile_error!` line.

| | `sqlite` (default) | `surreal` |
|---|---|---|
| board | one file | a directory behind a SurrealDB daemon |
| server | none | one local daemon, auto-started |
| right for | one box, one board file, zero setup | many processes claiming at once, a real query engine |

    cargo build --release                                    # sqlite
    cargo build --release --no-default-features --features surreal

Which build am I running? `agentboard path` prints a `.db` file for SQLite and a
directory for SurrealDB. Behaviour, not a timestamp, is the proof.

Where the board lives:

| | path |
|---|---|
| sqlite | `--db` / `$AGENTBOARD_DB`, else `$XDG_STATE_HOME/agentboard/board.db` |
| surreal | `--db` / `$AGENTBOARD_DB`, else `$XDG_STATE_HOME/agentboard/surreal` (a directory) |

Surreal settings, all optional: `$AGENTBOARD_SURREAL_URL` (`127.0.0.1:18720`, loopback
only), `$AGENTBOARD_SURREAL_USER` and `$AGENTBOARD_SURREAL_PASS` (both default `root`).
The first call starts the daemon itself — detached, log beside the data dir — polls
`/health`, refines the `/version` answer to 2.7.x, and answers in one line if anything
is wrong (no binary, port taken by something else, server that never answers). The
`surreal` binary must be on `PATH` or at `~/.surrealdb/surreal`. A server answering with
any version other than `surrealdb-2.7.x` is refused by name, and the Rust 3.x crate is not
usable at all on this tree: its `diskann` dependency fails to compile. Stop the daemon with
`pkill -f 'bind 127.0.0.1:18720'`.

There is no migration path between backends, and no migration path between board
versions: switching either one means a new board. Stop every agent first — each MCP
server is a process of the binary that started it, and an old build will happily
recreate the old schema in the new location.

## Build and install

    cd ~/Projects/agentboard
    cargo build --release                                  # ~4 s incremental, SQLite bundled
    cargo install --path . --locked                        # what every agent execs

For a SurrealDB board, install the other flavour — same lockfile, different feature:

    cargo install --path . --locked --no-default-features --features surreal

Agents run `~/.cargo/bin/agentboard`, so install after changing the CLI or MCP surface;
`./target/release/agentboard` is only the checkout, and `tests/mcp_client.py` drives it.

Hermes (`~/.hermes/config.yaml`):

    mcp_servers:
      agentboard:
        command: /home/kien1.luu/.cargo/bin/agentboard
        env:
          AGENTBOARD_ACTOR: hermes
        enabled: true

Claude Code, one session per herdr pane:

    claude mcp add agentboard --env AGENTBOARD_ACTOR=claude1 \
      -- /home/kien1.luu/.cargo/bin/agentboard

Then register the pane once so the board can map task → session:

    agentboard register claude1 --agent claude --task T7

Session rows do not expire: a pane that died hours ago still has its row, and the
row alone proves nothing. `dispatch` therefore only targets a **live** pane (a
tmux session of that name, or a herdr agent with that name and the registered
kind): the task's assignee if live, else `--to` if live, else an error that
names both and says which row is stale. A dead assignee row cannot swallow a
dispatch meant for a live `--to`. Clean up with:

    agentboard sessions prune --dry-run   # list the dead rows, delete nothing
    agentboard sessions prune             # delete them, printing each and why
    agentboard unregister claude1         # remove one row by name

`prune` refuses when neither tmux nor herdr answers (every row would look dead).
If only herdr is down it judges by tmux alone and says so, so herdr-hosted rows
are removed too: run it with herdr up.

## CLI

    agentboard --help                 # every command, flag and default
    agentboard wait --help            # selectors, filters, markers

`agentboard` with no command serves MCP on stdio. Misuse (unknown flag,
unknown task, bad value) is exit 1 with an `agentboard: ` prefix on stderr;
`wait` exits 0 on a match and 3 on a timeout, and its output starts with
`matched:` or `timeout:` (the same markers lead `task_wait`, which has no exit
code). Free-text values that begin with `-` are taken literally, so `note T7 "- done"`
works and `note T7 --bogus` stores `--bogus`; prefer `--flag=value` when unsure.

Global: `--actor NAME`, `--db PATH` (or `$AGENTBOARD_DB`: a file on SQLite, a data dir
on SurrealDB).
`$AGENTBOARD_DELIVER_TIMEOUT_MS` (default 2000) bounds each herdr/tmux call made by
`dispatch`, `send`, `tail` and `sessions`: a wedged pane client is killed (with its
process group) at the deadline instead of hanging the caller, and `dispatch` falls
back from herdr to tmux. Lower it only to test the kill path.
Default board: `$XDG_STATE_HOME/agentboard/board.db` (SQLite) or
`$XDG_STATE_HOME/agentboard/surreal` (SurrealDB).

## Tests

    python3 tests/mcp_client.py   # real stdio checks: claim, ask→wait, acks
    bash tests/cli.sh             # CLI contract: exit codes, stderr, flag spellings, no ANSI
    bash tests/dispatch.sh        # dispatch with stub herdr/tmux: refusal, --force, fallback, hang killed at the deadline
    ./tests/race.sh               # 8 concurrent claimers: 80 claims, 80 unique, ~124 ms
    bash tests/wait.sh            # long-poll returns on the event, not the timeout
    ./target/release/agentboard < tests/mcp-smoke.jsonl   # handshake + tool list

The SurrealDB backend has its own suite, which builds both feature sets and a debug
`surreal` binary, then restores the default release build (about a minute once warm):

    bash tests/surreal.sh         # parity, claims under 8 processes, auto-serve, outage, starvation

Unit tests that need a live server print `SKIP` and pass when no `surreal` binary is
found; on a box that carries one, `AGENTBOARD_REQUIRE_SURREAL=1` turns that skip into a
failure so a surreal change cannot pass unnoticed:

    AGENTBOARD_REQUIRE_SURREAL=1 cargo test --no-default-features --features surreal

Static gates, run before every commit (no CI): `cargo fmt --check` and
`cargo clippy --all-targets -- -D warnings`; both must print nothing.

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
one user, a few agents. If it needs more than 19 tools, it is not this tool.

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

The orchestrator's playbook lives in `skills/`; see [skills/README.md](skills/README.md).

Agent setup (build, MCP registration, skills): [docs/agent-install.md](docs/agent-install.md).
