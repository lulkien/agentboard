---
name: agentboard-team
description: "Use when running a team of CLI agents on the agentboard, or working one board task."
version: 5.0.0
license: MIT
platforms: [linux, macos]
---
# agentboard manual

The board holds the work: tasks, statuses, owners, comments and an event feed.
It is one LMDB directory. There is no server process: every CLI call and every MCP
server opens the directory, and the file lock keeps concurrent writers honest.

Pages here:

- this manual: the board, the seats, the launcher, the wake loop, the ten tools
- `workflow.md`: how a task moves and who moves it
- `roles.md`: manager, analyst, coder and reviewer: what each may run and never does

Each seat has a one-page entry skill of its own: `agentboard-manager`, `agentboard-analyst`,
`agentboard-coder`, `agentboard-reviewer`. Wire details live in `PROTOCOL.md`; changing the tree in `AGENTS.md`.

## The board

- One directory per board: `$AGENTBOARD_DB`, else
  `~/.local/state/agentboard/lmdb`. `agentboard path` prints the resolved one.
- A seat writes as its name: export `AGENTBOARD_ACTOR` or pass `--actor`, and give
  the MCP server the same name, for example `agentboard serve --actor coder`.
- Names identify a seat; they are not a credential. A name enters the roster when
  it first writes, and pointing a row at an unseen name needs `new_actor`, so a
  typo cannot invent a person.
- One live process per name. The board cannot tell two processes with one name
  apart, so they share one identity and the revision guard is the only brake.
- Every seat points at the same `AGENTBOARD_DB`. Different directories are
  different boards that cannot see each other.
- The map size is fixed at creation (`AGENTBOARD_MAP_SIZE_MB`, default 1024);
  there is no growth in place.

## The seats

| Seat | Writes as | Entry skill | Job |
| --- | --- | --- | --- |
| manager | the name its client was started with (`manager` here) | `agentboard-manager` | file rows, read the board, hand rows out, quota, decide with the human |
| analyst | `analyst` | `agentboard-analyst` | read and plan, hand the row to the coder |
| coder | `coder` | `agentboard-coder` | implement one row, verify it, hand it to review |
| reviewer | `reviewer` | `agentboard-reviewer` | read the work and judge it; close or reopen the row |

Nothing in the binary starts processes, and nothing in the board enforces these
roles: they are the team's contract. What the board enforces is narrower — a stale
revision, a row held by someone else, a forced write with no reason — and each
refusal is what makes a broken contract visible. `roles.md` has the detail.

## Which seat system

The platform answers this, and the manager reads it once at startup, in this order:

1. `HERDR_ENV=1`: the manager is inside herdr. Use herdr tabs and panes, one agent
   per tab.
2. Else any `HC_*` variable is set: the manager is inside hc. Each seat becomes its own
   named terminal.
3. Else use host builtin subagents as one-shot seats, following the internal
   subagent section below.

In herdr or hc, use the agent kind the user specified. If the kind is not
specified, ask; use the launch table's seat-specific model and effort defaults for
Claude or Codex. No variable of our own chooses the seat system, and `herdr-app
status` only says whether the person's window is open — a question for watching,
not for picking a workflow.

## Launch a team (herdr, one agent per tab)

One agent per tab. Never two agents as split panes of one tab: a tab is one seat's
workspace, and a split puts two agents on one screen and one set of keys. The
manager keeps its own tab; every worker gets a fresh tab.

To start the manager in the current herdr pane, run the installed
`start-agentboard-manager claude|codex|grok` helper from its shell prompt. It
renames that tab to `manager`, waits until the prompt is ready, and starts the
selected manager; `seats/README.md` documents its setup and prerequisites.

Analyst and reviewer judge, so they use the stronger model at high effort; the
coder executes their guidance, so that seat uses the cheaper model at medium
effort. Swap the id only when the human names a different model.

| Seat | Claude (`--kind claude`) | Codex (`--kind codex`) |
| --- | --- | --- |
| analyst | `--model claude-opus-5-5 --effort high` | `-m gpt-6-sol` (profile: high) |
| coder | `--model claude-sonnet-5-5 --effort medium` | `-m gpt-6-luna` (profile: medium) |
| reviewer | `--model claude-opus-5-5 --effort high` | `-m gpt-6-luna` (profile: high) |

```bash
export REPO=/path/to/repo
herdr tab create --cwd "$REPO" --label analyst
herdr pane list                                    # note the new tab's pane id

# Claude seats (MCP config + preferred model + permission mode)
# Analyst and reviewer skip permission prompts and deny the four file-mutation
# tools, so they read and run the shell but cannot edit. The coder denies nothing:
# it edits and runs the gate.
herdr agent start analyst --kind claude --pane "$PANE_ANALYST" -- \
  --model claude-opus-5-5 \
  --effort high \
  --mcp-config ~/.claude/analyst-mcp.json \
  --dangerously-skip-permissions \
  --disallowedTools Edit Write MultiEdit NotebookEdit
herdr agent start coder --kind claude --pane "$PANE_CODER" -- \
  --model claude-sonnet-5-5 \
  --effort medium \
  --mcp-config ~/.claude/coder-mcp.json \
  --dangerously-skip-permissions
herdr agent start reviewer --kind claude --pane "$PANE_REVIEWER" -- \
  --model claude-opus-5-5 \
  --effort high \
  --mcp-config ~/.claude/reviewer-mcp.json \
  --dangerously-skip-permissions \
  --disallowedTools Edit Write MultiEdit NotebookEdit

# Codex seats (profile + preferred model)
# Analyst's permission profile allows writes only in this scratch directory;
# keep the pane rooted in the repository so it can read source files there.
install -d -m 700 /tmp/agentboard/scratch
herdr agent start analyst --kind codex --pane "$PANE_ANALYST" -- \
  -p analyst -m gpt-6-sol
herdr agent start coder --kind codex --pane "$PANE_CODER" -- \
  -p coder -m gpt-6-luna
herdr agent start reviewer --kind codex --pane "$PANE_REVIEWER" -- \
  -p reviewer -m gpt-6-luna

# First prompt is the seat skill and the row: /agentboard-<seat> T<id> for Claude
# and Grok; $agentboard-<seat> T<id> for Codex.
herdr agent prompt analyst '/agentboard-analyst T<id>' --wait
herdr agent prompt coder '$agentboard-coder T<id>' --wait   # Codex seat
```

Start only the seats the work needs: a reviewer with nothing to review is a tab
that idles. Pick one kind per team; do not mix Claude and Codex rows of the table
in one launch unless the human asks.

The deny list removes the four file-mutation tools only; the shell stays open, and
the seat's own rules (analyst and reviewer never commit, build or install; the
coder commits only on its task branch) are what cover it.

A launch prompt uses `--wait` for proof it landed; without it a stalled prompt (an
open overlay) goes unnoticed. The manager's later wake prompts go without `--wait`
(it never blocks) and are checked with `herdr agent read <seat> --lines 40`. If a
prompt does not turn the agent busy, read the pane: `herdr agent read coder --lines 40`.

## Launch a team (hc, one terminal per seat)

One agent per terminal. Never start two agents in one terminal: it is one seat's
workspace, and the manager keeps its own. Every worker gets a terminal of its own,
named after the seat.

To start the manager in the current hc terminal, run the installed
`start-agentboard-manager claude|codex|grok` helper from its shell prompt. It starts
the selected manager as a new terminal named `manager` and prints its address;
`seats/README.md` documents its prerequisites.

Analyst and reviewer judge, so they use the stronger model at high effort; the
coder executes their guidance, so that seat uses the cheaper model at medium
effort. Swap the id only when the human names a different model.

| Seat | Claude | Codex |
| --- | --- | --- |
| analyst | `--model claude-opus-5-5 --effort high` | `-m gpt-6-sol` (profile: high) |
| coder | `--model claude-sonnet-5-5 --effort medium` | `-m gpt-6-luna` (profile: medium) |
| reviewer | `--model claude-opus-5-5 --effort high` | `-m gpt-6-luna` (profile: high) |

```bash
export REPO=/path/to/repo

# Claude seats (MCP config + preferred model + permission mode)
# Analyst and reviewer skip permission prompts and deny the four file-mutation
# tools, so they read and run the shell but cannot edit. The coder denies nothing:
# it edits and runs the gate.
hc run --bg --name analyst --cwd "$REPO" -- \
  claude --model claude-opus-5-5 \
  --effort high \
  --mcp-config ~/.claude/analyst-mcp.json \
  --dangerously-skip-permissions \
  --disallowedTools Edit Write MultiEdit NotebookEdit
hc run --bg --name coder --cwd "$REPO" -- \
  claude --model claude-sonnet-5-5 \
  --effort medium \
  --mcp-config ~/.claude/coder-mcp.json \
  --dangerously-skip-permissions
hc run --bg --name reviewer --cwd "$REPO" -- \
  claude --model claude-opus-5-5 \
  --effort high \
  --mcp-config ~/.claude/reviewer-mcp.json \
  --dangerously-skip-permissions \
  --disallowedTools Edit Write MultiEdit NotebookEdit

# Codex seats (profile + preferred model). --no-daemon is mandatory: without it the
# seat's own commands run outside hc and nothing it starts is addressable.
# The analyst's permission profile writes only in this scratch directory.
install -d -m 700 /tmp/agentboard/scratch
hc run --bg --name analyst --cwd "$REPO" -- codex --no-daemon -p analyst -m gpt-6-sol
hc run --bg --name coder --cwd "$REPO" -- codex --no-daemon -p coder -m gpt-6-luna
hc run --bg --name reviewer --cwd "$REPO" -- codex --no-daemon -p reviewer -m gpt-6-luna

# The first prompt is the seat skill and the row: /agentboard-<seat> T<id> for
# Claude and Grok; $agentboard-<seat> T<id> for Codex. A terminal that has not
# reported yet is working, so wait for each seat before its first prompt.
hc wait analyst
hc wait coder
hc wait reviewer
hc send --wait analyst '/agentboard-analyst T<id>'
hc send --wait coder '$agentboard-coder T<id>'   # Codex seat
```

Start only the seats the work needs: a reviewer with nothing to review is a
terminal that idles. Pick one kind per team; do not mix Claude and Codex rows of the
table in one launch unless the human asks.

The deny list removes the four file-mutation tools only; the shell stays open, and
the seat's own rules (analyst and reviewer never commit, build or install; the
coder commits only on its task branch) are what cover it.

Wake every seat with a plain `hc send`, never `--if idle`: it accepts only `idle` or
`working`, and a seat that finished a turn reports `done`, so it is refused with exit
10 and nothing is typed. Plain `hc send` waits for the program to pause, which is what
keeps a live turn from being interrupted. `hc wait <seat>` first on a booting seat,
then send with `--wait`, as proof it landed. A
`started:false` reply is the other case: the text went in and the program did not
start working. The manager's later wake prompts go without `--wait` (it never
blocks) and are checked with `hc read --lines 40 <seat>`. If a prompt does not turn
the agent busy, read the terminal before sending again.

Seats start hidden: a terminal joins the person's window only when it carries
`--label herdr.space=<space-id>` at launch, or gets
`hc label <address> herdr.space=<space-id>` afterwards. The launch adds no label;
`herdr-app do agent.focus --terminal <address>` then shows a seat that has one.

## Harness map

An index to what is documented elsewhere, not a second copy of it. Flags are in the
launch table above; install, cards and permissions are in `seats/README.md`.

| Client | Seats | Start | Actor wiring | Permission posture | Seat skill |
| --- | --- | --- | --- | --- | --- |
| Claude | all four | herdr `herdr agent start <seat> --kind claude`; hc `hc run --bg --name <seat> -- claude`; manager via `start-agentboard-manager claude` in either | `~/.claude/<seat>-mcp.json` via `--mcp-config` | bypass; analyst/reviewer add a four-tool edit deny list | `/agentboard-<seat>` |
| Codex | all four | herdr `herdr agent start <seat> --kind codex -- -p <seat>`; hc `hc run --bg --name <seat> -- codex --no-daemon -p <seat>`; manager via `start-agentboard-manager codex` in either | `codex -p <seat>` loads `<seat>.config.toml` | per-profile sandbox: analyst scratch-write, coder full access, reviewer read-only | `$agentboard-<seat>` |
| Grok | manager only | `start-agentboard-manager grok` (herdr or hc) | `${AGENTBOARD_SEAT:-manager}` in `~/.grok/config.toml` | `--trust --always-approve` | `/agentboard-manager` |
| Internal subagent | any seat, one-shot | the host's Agent call, when neither `HERDR_ENV=1` nor any `HC_*` variable is set | CLI with `--actor <seat>` (see the subagent section) | the host's | the seat's entry skill, named in the prompt |

Model and effort per seat come from the launch table for every client. Skills are
one physical copy under `~/.agents/skills`, linked into each client.

## Register a seat's MCP server

The seat's board name is not a herdr or hc concept: the MCP server carries it. Register
once per client, with the seat's actor and the shared board:

```bash
# codex: ~/.codex/<seat>.config.toml, layered by codex -p <seat>
#   [mcp_servers.agentboard]
#   command = "agentboard"
#   [mcp_servers.agentboard.env]
#   AGENTBOARD_ACTOR = "coder"
#
# claude: one mcp-config file per seat, named on the command line
claude --mcp-config ~/.claude/coder-mcp.json


```

The repo's `seats/` directory is the canonical copy of those cards, and
`seats/README.md` says which are linked into a home directory and which are copied.

## Wake loop

There is no push channel, and there will not be one: the MANAGER is the wake. A
seat does one row, reports it and stops; it never arms a feed wait or polls for
its next row, and the manager prompts it with the next row.

What follows is how the manager watches a board, not how a seat waits for work:

```bash
agentboard feed --after "$CURSOR" --recipient coder --wait 300
```

- The cursor is `<instance>:<seq>`. Keep the one the feed printed and pass it back;
  `--after 0` replays from the first event.
- `--wait 300` is the maximum. A timeout is a normal empty page with a valid
  cursor: print it, re-read the board, arm again.
- Only an assignment carries `recipient`: `task_update --assignee coder`, or a
  `task_add` that names one. A comment or a status move lands in the feed with no
  recipient, so a waiter filtered by `--recipient` never sees it. Wait unfiltered, or
  wake the seat by assigning it.
- If a seat must be poked outside the board, that is a prompt (`herdr agent
  prompt` or `hc send`) to that seat, not a board write.
- To block on named rows, use the wait instead of re-arming the feed:
  `agentboard wait 27 --status reviewing --timeout 300` returns the moment T27 reaches
  review. There is no MCP wait tool; the binary is the only way to wait. An epic
  works too: `agentboard wait 12 --epic` watches its direct children and returns
  once every one of them is `done` or `canceled`. The verdict's first line is
  `matched:` (exit 0) or `timeout:` (exit 3), and it writes nothing.
- An analyst-to-coder handoff changes the assignee and leaves the status alone, so
  a status wait never sees it. Add `--assignee` to the same wait:
  `agentboard wait 27 --status reviewing,done,blocked --assignee --timeout 300`. It returns on a
  handoff after the wait starts and says `; handoff T27 to coder` on the first line.
- In hc, park a long wait in a terminal of its own, so the manager stays reachable:
  `hc run --bg --name watch-27 --cwd "$REPO" agentboard wait 27 --status
  reviewing,done,blocked --assignee --timeout 300` returns at once, and the watch's
  exit arrives as a note in the manager's terminal. Read the verdict with
  `hc read --lines 5 watch-27`. One watcher per row: a `--name` is unique among live
  terminals, so clear the old arm before arming the new one — `hc rm watch-27` once
  it has finished, because `hc kill` on a finished terminal answers exit 11
  `finished`, or `hc kill watch-27; hc rm watch-27` while it is still live.
- A wait wakes the manager, not the seat. It reports that the row moved; no pane or
  terminal received a prompt. The seat that must act still needs a `herdr agent
  prompt` or an `hc send`.

## Internal subagent seats (outside herdr and hc)

This is the one-shot path for internal subagents; the herdr and hc paths above stay
available for dedicated CLI seats. Give each subagent its seat name, task ID, the
shared board directory, and that seat's entry skill. Spawning the subagent is the
wake. It works one row and returns; its completion notice tells the manager to
read the row and its feed, then spawn the next assigned seat or finish if the row
is terminal. In this mode, do not start a tab or terminal for the seat, wait on the board,
poll, or keep a per-seat queue.

Supervise the subagents in prose; the binary adds no lease, heartbeat, stuck
timeout or auto-retry.

- Spawn each seat with the launch table's model and effort (the Agent call's
  `model`/`effort` for Claude; the documented profile and model for Codex).
- Spawn a fresh subagent on every wake. The row is the whole scope: no pasted
  chat history.
- Judge stalls by side effects: `board_agents` last activity, the row's feed, and
  `git status`. Pane or terminal output alone is not evidence (see `references/herdr-mechanics.md`, `references/hc-mechanics.md`). Account for every
  seat you spawned.
- If a subagent returns and the row did not move and carries no handoff comment,
  comment a failure note on the row (what you saw, the likely cause) and retry by
  cause: the same brief after a drop, a narrower row after a context limit,
  another model after a tool error ("another model" is the other kind's row of
  the launch table). After 2 retries set the row `blocked` with the question for
  the human, using `force` and the retry history as the `reason`: a plain status
  write refuses while the seat still holds the row.

The host session's agentboard MCP server keeps its configured actor, often
`manager`. A subagent sharing that session must use the CLI for mutations with its
own seat actor and the manager's board directory. This is the subagent exception
to the general rule below to write through MCP:

```bash
agentboard --db DIR --actor coder comment ID --body "text"
AGENTBOARD_DB=DIR AGENTBOARD_ACTOR=coder agentboard comment ID --body-file PATH
AGENTBOARD_DB=DIR AGENTBOARD_ACTOR=coder agentboard comment ID --body-file -
```

`--body-file -` reads the comment from stdin. A positional comment body is not
accepted. Reads may use the shared MCP server, but a full-text task lookup shows
stored task fields only; it does not show comments. Before reviewing or resuming
work, read the task's complete event history and coder evidence with the CLI feed,
following the printed cursor while `more true`:

```bash
agentboard --db DIR --actor reviewer feed --task ID --after 0 --full
```

## Ten tools, same store as the CLI

| Tool | Use |
| --- | --- |
| `task_add` | file work; assigns on creation when you name an assignee |
| `task_list` | status, assignee, parent, path; `all=true` includes closed rows |
| `task_get` | one task and its revision; `full=true` for stored text |
| `task_epic` | umbrella row, direct-child counts and done/total progress |
| `task_update` | fields, status, owner; needs `if_revision`; `force`+`reason` to override; `release` to return a held row to `todo` |
| `task_claim` | next ready task, or one by id |
| `task_comment` | append context; never moves the revision |
| `board_feed` | events after a cursor, filters, `wait`; `full=true` for whole bodies |
| `board_brief` | epic progress, unparented active work, then global status counts |
| `board_agents` | roster with last activity and last task |

CLI parity: `add`, `ls`, `get`, `epic`, `update`, `claim`, `comment`, `feed`, `wait`,
`brief`, `agents`, `path`, `reset --yes`, `serve`. Exit codes: `0` ok, `1` refused,
`3` no ready tasks or a wait timeout.

Use `task_epic` (CLI `epic ID`) to read an umbrella's direct children as status
counts and done/total progress. Any task with children is an epic; nested epics
also appear in `board_brief`. Brief puts epic progress first, unparented active
work next, and global counts last. Close or drop all direct children before
closing an umbrella; force does not bypass that store rule. Creating or reparenting
a child under a done or canceled parent refuses, even with force. To attach more
work, the manager must change that parent back to an open status and retry the
attachment. This never happens automatically.

## Rules for every agent

- The board is the only status channel. No `NOTES.md`, no `STATUS.md`.
- The task row is the prompt: `task_get` returns `description` and `goal` verbatim.
  Do not stage prompt text in files.
- Write through the MCP tools, not by shelling out to `agentboard`.
- Prefer MCP over shell for reading and searching: code through
  `codebase-memory-mcp`, git through `git-mcp-rs`, the web through `ferris-search`,
  the board through the agentboard tools. Shell is not banned: builds, tests,
  installs and the commands a goal names run there, and a command with no MCP
  equivalent is fine. Keep `grep`, `cat`, `sed`, `find` and `ls` for the cases MCP
  cannot answer, not as the default reading path.
- Never guess a missing requirement: comment one narrow question, set status
  `blocked`, then stop and report; the manager brings the answer back.
- `done` means verified. Put the real command output in a comment on the row.
- Touch only your task. Do not run suites or installs the goal does not name.
- A probe that touches a board uses a scratch one:
  `AGENTBOARD_DB=$(mktemp -d)/b agentboard ...`.
- Implementation lives on `task-<id>-analyst-<n>-implement-<m>`, one commit per
  branch. A reopened row gets a new branch from the previous implement branch, and
  there is no `implement-4`: the cap returns the row to the analyst.
- Never push. The manager squash-merges a done row's chain into the working
  branch as one commit and deletes the task branches; the owner pushes.

## Handoff and quota

A handoff is one assignment: `task_update(id, if_revision, assignee="next-actor")`.
The event carries `recipient`, which names the next seat; nothing in the board
wakes it — the manager prompts it. Move the status in
the same call: `reviewing` with `assignee="reviewer"`, `implementing` back with
`assignee="coder"` below the cap, or `planning` with `assignee="analyst"` when the
third implement turn is found short. The coder's branch is
`task-<id>-analyst-<n>-implement-<m>`; the handoff names it and its one commit.

When a holder is gone for good — out of quota, its seat replaced by another name —
send the row back instead of handing it over:

```bash
agentboard get 12                                     # revision 3, held by coder
agentboard update 12 --if-revision 3 --release --force --reason "the coder seat is gone"
```

One call, one event: the row is `todo` with nobody on it, ready for `claim` again.
The holder itself releases without `--force`; every other name needs the force and
the reason, so nobody strips a live holder's row by accident.

When the row must change hands rather than go back to the queue, reassign it:

```bash
agentboard get 12                       # revision 3, held by coder
agentboard update 12 --if-revision 3 --assignee coder-2 --force --reason quota
```

The displaced holder's field writes refuse and its comments still land. Stop or
isolate the old process before the replacement edits the same working tree, and
tell the replacement which artifact to resume from.

A seat that must stop before its handoff comments a resume note first: state, the
task branch and its commit, what is verified, next step, any uncommitted paths, and
what a replacement must not redo. A replacement reads
`board_feed(task=ID, after="0", full=true)` (CLI `feed --task ID --after 0
--full`) before acting, follows the printed cursor while `more` is true, and does
not redo steps already marked verified.
