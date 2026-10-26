---
name: agentboard-team
description: "Use when running a team of CLI agents on the agentboard, or working one board task."
version: 5.0.0
author: Hermes Agent
license: MIT
platforms: [linux, macos]
metadata:
  hermes:
    tags: [agentboard, manager, analyst, coder, reviewer, lmdb, feed]
    related_skills: [claude-code, opencode, herdr]
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
| manager | the name its client was started with (`hermes` here) | `agentboard-manager` | file rows, read the board, hand rows out, quota, decide with the human |
| analyst | `analyst` | `agentboard-analyst` | read and plan, hand the row to the coder |
| coder | `coder` | `agentboard-coder` | implement one row, verify it, hand it to review |
| reviewer | `reviewer` | `agentboard-reviewer` | read the work and judge it; close or reopen the row |

Nothing in the binary starts processes, and nothing in the board enforces these
roles: they are the team's contract. What the board enforces is narrower — a stale
revision, a row held by someone else, a forced write with no reason — and each
refusal is what makes a broken contract visible. `roles.md` has the detail.

## Launch a team (herdr, one agent per tab)

One agent per tab. Never two agents as split panes of one tab: a tab is one seat's
workspace, and a split puts two agents on one screen and one set of keys. The
manager keeps its own tab; every worker gets a fresh tab.

Preferred models for worker seats (pass after `--`; herdr forwards them to the
agent). Analyst gets the stronger model; coder and reviewer share the cheaper
one. Swap the id only when the human names a different model.

| Seat | Claude (`--kind claude`) | Codex (`--kind codex`) |
| --- | --- | --- |
| analyst | `--model claude-opus-5-5` | `-m gpt-6-sol` |
| coder | `--model claude-sonnet-5-5` | `-m gpt-6-luna` |
| reviewer | `--model claude-sonnet-5-5` | `-m gpt-6-luna` |

```bash
export REPO=/path/to/repo
herdr tab create --cwd "$REPO" --label analyst
herdr pane list                                    # note the new tab's pane id

# Claude seats (MCP config + preferred model + permission mode)
# Analyst/reviewer are read-only on the tree: dontAsk refuses file/shell
# writes instead of auto-approving, and --allowedTools names the read tools it
# still permits (without it the seat has no read tools). One command per Bash
# call: under dontAsk a compound line (`cd ...; for ...`) is refused even when
# each verb is allowed. Coder needs unattended shell/tests → bypass.
herdr agent start analyst --kind claude --pane "$PANE_ANALYST" -- \
  --model claude-opus-5-5 \
  --strict-mcp-config --mcp-config ~/.claude/analyst-mcp.json \
  --permission-mode dontAsk \
  --allowedTools mcp__agentboard mcp__codebase-memory-mcp mcp__git-mcp-rs \
  mcp__ferris-search Read Glob Grep LS 'Bash(rg:*)' 'Bash(find:*)' 'Bash(ls:*)' \
  'Bash(readlink:*)' 'Bash(git status:*)' 'Bash(git log:*)' 'Bash(git diff:*)'
herdr agent start coder --kind claude --pane "$PANE_CODER" -- \
  --model claude-sonnet-5-5 \
  --strict-mcp-config --mcp-config ~/.claude/coder-mcp.json \
  --dangerously-skip-permissions
herdr agent start reviewer --kind claude --pane "$PANE_REVIEWER" -- \
  --model claude-sonnet-5-5 \
  --strict-mcp-config --mcp-config ~/.claude/reviewer-mcp.json \
  --permission-mode dontAsk \
  --allowedTools mcp__agentboard mcp__codebase-memory-mcp mcp__git-mcp-rs \
  mcp__ferris-search Read Glob Grep LS 'Bash(rg:*)' 'Bash(find:*)' 'Bash(ls:*)' \
  'Bash(readlink:*)' 'Bash(git status:*)' 'Bash(git log:*)' 'Bash(git diff:*)'

# Codex seats (profile + preferred model)
herdr agent start analyst --kind codex --pane "$PANE_ANALYST" -- \
  -p analyst -m gpt-6-sol
herdr agent start coder --kind codex --pane "$PANE_CODER" -- \
  -p coder -m gpt-6-luna
herdr agent start reviewer --kind codex --pane "$PANE_REVIEWER" -- \
  -p reviewer -m gpt-6-luna

# First prompt is the seat skill and the row: /agentboard-<seat> T<id> for Claude,
# opencode, Hermes and Grok; $agentboard-<seat> T<id> for Codex.
herdr agent prompt analyst '/agentboard-analyst T<id>' --wait
herdr agent prompt coder '$agentboard-coder T<id>' --wait   # Codex seat
```

Start only the seats the work needs: a reviewer with nothing to review is a tab
that idles. Pick one kind per team; do not mix Claude and Codex rows of the table
in one launch unless the human asks.

A launch prompt uses `--wait` for proof it landed; without it a stalled prompt (an
open overlay) goes unnoticed. The manager's later wake prompts go without `--wait`
(it never blocks) and are checked with `herdr agent read <seat> --lines 40`. If a prompt does not turn the
agent busy, read the pane: `herdr agent read coder --lines 40`.

## Register a seat's MCP server

The seat's board name is not a herdr concept: the MCP server carries it. Register
once per client, with the seat's actor and the shared board:

```bash
# codex: ~/.codex/<seat>.config.toml, layered by codex -p <seat>
#   [mcp_servers.agentboard]
#   command = "agentboard"
#   [mcp_servers.agentboard.env]
#   AGENTBOARD_ACTOR = "coder"
#
# claude: one mcp-config file per seat, named on the command line
claude --strict-mcp-config --mcp-config ~/.claude/coder-mcp.json

# opencode: one global config, so the seat names itself in its pane environment
# and the config's agentboard entry runs seats/opencode/agentboard-mcp.sh.
# --standalone gives the seat its own server, which is what inherits that env.
herdr tab create --label coder --env AGENTBOARD_SEAT=coder --env AGENTBOARD_ACTOR=coder
herdr agent start coder --kind opencode --pane "$PANE" -- --standalone
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
- If a seat must be poked outside the board, that is a herdr prompt, not a board
  write.
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
- A wait wakes the manager, not the seat. It reports that the row moved; no pane
  received a prompt. The seat that must act still needs a herdr prompt.

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
- Never push. Commits are local and the owner decides when they leave.

## Handoff and quota

A handoff is one assignment: `task_update(id, if_revision, assignee="next-actor")`.
The event carries `recipient`, which names the next seat; nothing in the board
wakes it — the manager prompts it. Move the status in
the same call: `reviewing` with `assignee="reviewer"`, `implementing` back with
`assignee="coder"`.

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
