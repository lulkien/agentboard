# agentboard

A task board shared by a handful of CLI coding agents (Claude Code in a herdr
pane, Hermes, subagents) plus you. 17 MCP tools over stdio and one board, stored
either in SurrealDB (the default build) or in a single SQLite file (opt-in). The point is to replace
"write status to NOTES.md and hope somebody reads it" with a board that answers in
one line and costs few tokens.

## Model

Five kinds of record — five tables in SQLite (`src/schema.sql`), five record types in
SurrealDB (`src/store/surreal/`):

    epics     id, title, description, goal, status, path, created_at, updated_at,
              completed_at                               -- E1, E2...; never leased
    tasks     id, board, reporter, title, description, goal, path, branch, status, assignee, priority, epic_id,
              parent_id, needs, report, commit_ref, lease_until, created_at, updated_at,
              started_at, completed_at, reopen_count     -- moves into needs_fix; 4 = capped
    events    seq, task_id | epic_id, actor, kind, body, ts  -- append-only, one timeline
    diffs     id, task_id, round, body, created_at, reviews -- D1, D2...; append-only, verbatim,
                                                         -- outlive a dropped task; non-NULL reviews
                                                         -- marks an evidence-only re-run, not a patch
    sessions  name, agent, cwd, task_id, seen, status    -- live panes: herdr or tmux

A **diff** is what a coder submitted for review, stored verbatim and never checked by
the board. It is append-only (no edit, no delete, no renumber) and outlives its task
being dropped. `get` shows the newest patch for the task (`diff: D12 round 2 3m 184
lines`); the CLI verbs are `agentboard diff add T7 --body-file PATH|- | --body TEXT`,
`diff ls T7|E3` and `diff get D12 [--full]`. A re-run round whose tree did not change
records **evidence-only** instead: `diff add T7 --evidence-only --reviews D12 --body-file
PATH|-` (MCP: `reviews="D12"`). The row is marked in `diff ls`, labelled `not a diff` by
`diff get`, and never becomes the newest patch; the brief names D12 as the diff to read.
A board file from before diffs gains the table; nothing is refused.

An **epic** is not a task: it has its own ids (`E1`), is never claimed, leased or
dispatched, and carries the project `path` its work belongs to. A task points at an
epic (`epic_id`) and optionally at another task (`parent_id`, a subtask that
inherits the parent's epic at creation). Cycles are refused; creation and
re-parenting also refuse cross-epic parents. A move may split a family and records
a note event naming immediate relatives outside the destination, including
ungrouped relatives. A board file without a schema version is refused with the
manual step to inspect and stamp the legacy board offline.

Every task records its creation actor as immutable `reporter`. `get` shows it;
edits and worker reassignment preserve it. Old rows use their earliest creation
event, or show `unknown (filed before this field existed)` when no actor is known.

Statuses: `ready → working → (blocking → working)* → review → done`,
plus `needs_fix` (review found work, waiting for a fix; shown as `fix`),
`not_done` (set by the board when a row's fourth move into `needs_fix` is refused;
shown as `nd`) and `dropped`. `needs='input'` marks "blocked on a human", which is the only
thing that should ever interrupt you. Input aliases `running`→`working`,
`blocked`→`blocking`, `triage`→`ready`, `need fix`/`needs-fix`/`fix`→`needs_fix` accepted on input.

Reviewable work is a chain of three levels, implement → review → fix, and no fourth.
Moving a task to `review` needs a submission recorded since its last claim: a diff
(`diff_add`), or an evidence-only row for a re-run round that names the newest patch of the same row.
On a parentless task the board files the review row as its child, once. A row may move
into `needs_fix` three times; the fourth is refused (CLI exit 1) and the row lands
`not_done`, taking an open parent with it. The rules, who moves which status and the
reviewer's four rules are in [PROTOCOL.md](PROTOCOL.md); the diagrams are in
[docs/workflow.md](docs/workflow.md). Closing an epic only when every row is `done` or
`dropped` is the manager's policy, not something the board enforces.

`events` is the file you used to write. Every note, question, answer, claim and
completion is one row; nothing is mutable history.

## Tools

| tool | cost | what it does |
|---|---|---|
| `task_add` | 1 line | create; returns `T7` (takes `title, description, goal, status, assignee, priority, epic, parent, path, branch`; `branch` defaults to `HEAD`, which leaves the pane's checkout alone; names are case-sensitive and only the literal `HEAD` means that, `head` is an ordinary branch) |
| `task_list` | 1 line/task | open work, ordered by urgency |
| `task_get` | ~10 lines | description, goal, timestamps, lease, report, note, last 8 events; long fields end in a `[truncated <field>: N of M chars ...]` marker, `full=true` returns them (and event bodies) whole |
| `task_children` | 1 line/task | an epic id (`E3`) lists every task in it, a task id (`T7`) its subtasks: id, status, assignee, age, title |
| `epic_add` | 1-2 lines | create an epic (`title, description, goal, path`); returns `E3`, plus a warning line if `path` does not exist yet |
| `epic_list` | 1 line/epic | epics (default open): id, status, done/total, path, title |
| `epic_get` | ~10 lines | status, path, task counts, description, goal, last 8 events; `full=true` unclipped |
| `epic_update` | 1 line ack | status (`open`/`done`/`dropped`), note, title, description, goal, path; combined changes are atomic; closing is never automatic; unknown fields refuse by name |
| `diff_add` | 1 line ack | record what you changed for a task, verbatim and append-only: `D12 (T7 round 2)`, round = the task's reopen count + 1; whitespace-only is refused. A re-run round with no tree change passes `reviews="D<n>"` naming the newest patch of this row and records evidence, marked not a patch. Only the newest patch is reviewed |
| `diff_get` | header + body | one diff: task, round, age; evidence-only rows are labelled and point at the patch; the body is cut with a `[truncated ...]` marker, `full=true` returns it whole |
| `diff_list` | 1 line/diff | a task's diffs (or an epic's, naming each task), newest first: `D12 round 2 3m 184 lines` or `D13 evidence-only -> D12`, or `(no diffs)` |
| `task_dispatch` | short ack | dispatch a ready row to a live seat; root reporter manages the chain, analyze claimant hands its implement child to `coder`, implement claimant hands its review child to `reviewer`; legacy review goes to `reviewer`, legacy fix to `coder`; same store and refusal text as CLI `dispatch` |
| `task_claim` | work order | atomic claim, **returns description + goal** so no follow-up `get` |
| `task_update` | 1 line ack | status, report, note, commit, heartbeat and spec fields title, description, goal, priority, assignee, path, branch, epic; combined spec/state changes are atomic; spec edits refuse a live worker lease; unknown fields refuse by name |
| `task_wait` | delta only | server-side long-poll over `id`/`ids`/`epic`/`open` with `status`/`kind` filters; first line `matched:` or `timeout:` |
| `board_brief` | ~1 screen | counts + one `EPIC` line per open epic (done/non-dropped tasks, blocking/needs_fix counts, path) + open tasks that belong to no epic + recent completions |
| `session_register` | ack | map session → task |

The surface has 17 MCP tools: every tool schema is resent on every request, so a
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

| | `surreal` (default) | `sqlite` (opt-in) |
|---|---|---|
| board | a directory behind a SurrealDB daemon | one file |
| server | one local daemon, auto-started | none |
| right for | many processes claiming at once, a real query engine | one box, one board file, zero setup |

    cargo build --release                                    # surreal (default)
    cargo build --release --no-default-features --features sqlite

Which build am I running? `agentboard path` prints a `.db` file for SQLite and a
directory for SurrealDB. Behaviour, not a timestamp, is the proof.

Where the board lives:

| | path |
|---|---|
| surreal (default) | `--db` / `$AGENTBOARD_DB`, else `$XDG_STATE_HOME/agentboard/surreal` (a directory) |
| sqlite (opt-in) | `--db` / `$AGENTBOARD_DB`, else `$XDG_STATE_HOME/agentboard/board.db` |

Surreal settings, all optional: `$AGENTBOARD_SURREAL_URL` (`127.0.0.1:18720`, loopback
only), `$AGENTBOARD_SURREAL_USER` and `$AGENTBOARD_SURREAL_PASS` (both default `root`).
The first call starts the daemon itself — detached, log beside the data dir — polls
`/health`, refines the `/version` answer to 2.7.x, and answers in one line if anything
is wrong (no binary, port taken by something else, server that never answers). The
`surreal` binary must be on `PATH` or at `~/.surrealdb/surreal`. A server answering with
any version other than `surrealdb-2.7.x` is refused by name, and the Rust 3.x crate is not
usable at all on this tree: its `diskann` dependency fails to compile. Stop the daemon with
`pkill -f 'bind 127.0.0.1:18720'`.

There is no migration between backends. Schema versions 1 and 2 migrate additively to
version 3 on the first CLI or MCP call, under the write lock in one transaction.
Newer boards refuse an older binary by name. See the
[schema version protocol](PROTOCOL.md#board-schema-version).
The owner-only offline `board upgrade --owner` command remains available for
maintenance; it is no longer required for an additive version transition.

## Build and install

    cd ~/Projects/agentboard
    cargo build --release                                  # surreal, the default build
    cargo install --path . --locked                        # what every agent execs

The default build needs a `surreal` 2.7.x binary on `PATH` or at `~/.surrealdb/surreal`
(it starts the local board server from it). For a single-file SQLite board, install the
opt-in flavour — same lockfile, different feature:

    cargo install --path . --locked --no-default-features --features sqlite

Agents run `~/.cargo/bin/agentboard`, so install after changing the CLI or MCP surface;
`./target/release/agentboard` is only the checkout, and `tests/mcp_client.py` drives it.

Hermes (`~/.hermes/config.yaml`):

    mcp_servers:
      agentboard:
        command: agentboard
        env:
          AGENTBOARD_ACTOR: hermes
        enabled: true

Claude Code, one session per herdr pane:

    claude mcp add agentboard --env AGENTBOARD_ACTOR=claude1 -- agentboard

The command is exec'd directly, not through a shell, so `$HOME` and `~` are passed
literally and fail with `ENOENT` — verified: `["$HOME/.cargo/bin/agentboard","--help"]`
raises `No such file or directory`, while bare `agentboard` resolves on `PATH`. Use the
bare name: it needs no home directory, so the same config works for any user on any
machine. Two cases want a literal absolute path instead — a client that does not inherit
your `PATH` (some GUI launchers), or one specific build — and then paste what
`command -v agentboard` prints for that machine.

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

Waking on an agent instead of polling: `agentboard wait T221 --agent coder --status review,done`
watches the rows and herdr's `coder` at once and returns on whichever moves first
(`matched: agent coder -> idle after 4.2s` for the agent side; the board's own text for a
row). `--until` picks the agent statuses (default `idle,blocked,done`); `--agent` alone
needs no task ids, and `--status`/`--kind`/`--after` without ids are refused. Only
transitions after the subscription is established count (names are resolved first); `manager agent wait` is the "has it reached"
check. `agentboard manager events wait --until working [--agent coder] [--timeout 60] [--json]`
is the agent side alone, exit 0 or 3. Both ride `events.subscribe` (herdr's `events.wait`
takes one pane and one status) and close the stream on a match, a timeout and a cancel.

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
Default board: `$XDG_STATE_HOME/agentboard/surreal` (SurrealDB, the default build) or
`$XDG_STATE_HOME/agentboard/board.db` (SQLite, the opt-in build).

## Tests

    AGENTBOARD_REQUIRE_SURREAL=1 cargo test          # default (surreal) build; a missing surreal binary FAILS, not SKIPs
    cargo test --no-default-features --features sqlite
    bash tests/cli.sh             # CLI contract: exit codes, stderr, flag spellings, no ANSI
    bash tests/dispatch.sh        # dispatch with stub herdr/tmux: refusal, --force, fallback, hang killed at the deadline
    bash tests/surreal.sh         # cli.sh, dispatch.sh and mcp_client.py on BOTH builds, plus parity, claims under 8 processes, auto-serve, outage, starvation
    ./target/release/agentboard < tests/mcp-smoke.jsonl   # handshake + tool list

`AGENTBOARD_REQUIRE_SURREAL=1` matters because unit tests that need a live server print
`SKIP` and pass when no `surreal` binary is found; with it set, the skip is a failure, so
a surreal change cannot pass unnoticed. A default `cargo test` expects a `surreal` 2.7.x
binary on `PATH` or at `~/.surrealdb/surreal`; the sqlite test run needs none.

`cli.sh` and `dispatch.sh` pick the backend from the binary under test
(`AGENTBOARD_TEST_BIN=<binary>`) and start their own throwaway servers.
`tests/mcp_client.py` needs `AGENTBOARD_TEST_DB` and `AGENTBOARD_SURREAL_URL` on the
surreal build; `tests/surreal.sh` supplies both and builds both feature sets, a debug
`surreal` binary, then restores the default release build (about a minute once warm).
`tests/race.sh` and `tests/wait.sh` (8 concurrent claimers: 80 claims, 80 unique; the long-poll
returns on the event) also own a throwaway board through `tests/board.sh`. No suite targets
the default board: `mcp_client.py` refuses to run without a throwaway one.

Static gates, run before every commit (no CI): `cargo fmt --check` and
`cargo clippy --all-targets -- -D warnings`; both must print nothing (clippy: `Finished` only).

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
one user, a few agents. Every MCP operation must earn its place in the loop.

## Do agents need a skill for this?

No for the mechanics: the tool descriptions were enough in a field test — a bare
"you have work waiting on this machine" prompt produced `task_list` →
`task_claim` → edit → verify → `task_update(review)` with no guidance (a move to `review` now also needs a `diff_add` first, and the refusal names it), and no
status file was written. Only the retained board operations enter the MCP surface.

Yes for the conventions, which no schema can express: board is the only status
channel, claim the next task, never guess a missing requirement, what counts as
done. Those nine rules live in `PROTOCOL.md` — paste them into `CLAUDE.md` or
`AGENTS.md` once per worker. A skill is for the orchestrator (dispatch, answer
asks, read `board_brief`), not for the worker.

The orchestrator's playbook lives in `skills/`; see [skills/README.md](skills/README.md).

Agent setup (build, MCP registration, skills): [docs/agent-install.md](docs/agent-install.md).

Boards carry a schema version, checked on every CLI invocation and MCP tool call.
Older boards migrate in the same call; newer boards refuse. Build identity is
informational. See the [schema version protocol](PROTOCOL.md#board-schema-version).
