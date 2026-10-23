---
name: agentboard-manager
description: "Use when working a task on the agentboard, or running a team of CLI agents on it."
version: 4.0.0
author: Hermes Agent
license: MIT
platforms: [linux, macos]
metadata:
  hermes:
    tags: [agentboard, manager, coder, reviewer, lmdb, feed]
    related_skills: [claude-code, opencode, herdr]
---
# agentboard manual

The board holds the work: tasks, statuses, owners, comments and an event feed.
It is one LMDB directory. There is no server process: every CLI call and every MCP
server opens the directory, and the file lock keeps concurrent writers honest.

Three pages here:

- this manual: the board, the launcher, the wake loop, the nine tools
- `workflow.md`: how a task moves and who moves it
- `roles.md`: manager, coder, reviewer: what each may run and never does

Wire details: `PROTOCOL.md`. Changing the tree: `AGENTS.md`. Both in the repo.

## The board

- One directory per board: `$AGENTBOARD_DB`, else
  `~/.local/state/agentboard/lmdb`.
- A seat writes as its name: export `AGENTBOARD_ACTOR` or pass `--actor`, and
  register the MCP server with that name, for example
  `agentboard serve --actor coder`.
- Names are addresses on a trusted team, not authentication. One live process per
  name: two agents sharing one name refuse each other's writes as `held by …`.
- Every seat points at the same `AGENTBOARD_DB`. Different directories are
  different boards that cannot see each other.

## Launch a team (herdr-direct)

The manager owns panes; the board owns the work. Nothing in the binary starts
processes, so these commands are yours to run. `references/herdr-mechanics.md` has
the traps.

```bash
export REPO=/path/to/repo
herdr tab create --cwd "$REPO" --label team        # worker tab, never the manager's
herdr pane list                                    # note the new tab's pane id
herdr pane split --pane "$P" --direction right --cwd "$REPO"
herdr pane split --pane "$P" --direction down  --cwd "$REPO"
herdr agent start coder    --kind claude   --pane "$PANE_CODER"
herdr agent start reviewer --kind opencode --pane "$PANE_REVIEWER"
herdr agent list                                   # name, kind, status, pane
```

Then each seat is told its board name in one line, with no apostrophes:

```bash
herdr agent prompt coder 'You are coder on the agentboard. Use MCP tools only, actor=coder. Start with task_claim.'
herdr agent prompt reviewer 'You are reviewer on the agentboard. MCP tools only, actor=reviewer. Start with board_feed wait=300.'
```

`herdr agent prompt` needs `--wait` when you want proof the prompt landed; without
it a stalled prompt (an open overlay) goes unnoticed. If a prompt does not turn the
agent busy, read the pane: `herdr agent read coder --lines 40`.

## Wake loop

There is no push channel. A seat waits on the feed, and the manager wakes it by
writing to the board:

```bash
agentboard feed --after "$CURSOR" --recipient coder --wait 300
```

- The cursor is `<instance>:<seq>`. Keep the one the feed printed and pass it back;
  `--after 0` replays from the first event.
- `--wait 300` is the maximum. A timeout is a normal empty page with a valid
  cursor: print it, re-read the board, arm again.
- The manager wakes a seat by assigning it work (`task_update --assignee coder`),
  by commenting on its task, or by moving a status. All three land in the feed; the
  assignment carries the recipient.
- If a seat must be poked outside the board, that is a herdr prompt, not a board
  write.

## Nine tools, same store as the CLI

| Tool | Use |
| --- | --- |
| `task_add` | file work; assigns on creation when you name an assignee |
| `task_list` | status, assignee, parent, path; `all=true` includes closed rows |
| `task_get` | one task and its revision; `full=true` for stored text |
| `task_update` | fields, status, owner; needs `if_revision`; `force`+`reason` to override |
| `task_claim` | next ready task, or one by id |
| `task_comment` | append context; never moves the revision |
| `board_feed` | events after a cursor, filters, `wait`; `full=true` for whole bodies |
| `board_brief` | counts per status plus the oldest active work |
| `board_agents` | roster with last activity and last task |

CLI parity: `add`, `ls`, `get`, `update`, `claim`, `comment`, `feed`, `brief`,
`agents`, `path`, `reset --yes`, `serve`. Exit codes: `0` ok, `1` refused, `3` no
ready tasks.

## Rules for every agent

- The board is the only status channel. No `NOTES.md`, no `STATUS.md`.
- The task row is the prompt: `task_get` returns `description` and `goal` verbatim.
  Do not stage prompt text in files.
- Write through the MCP tools, not by shelling out to `agentboard`.
- Never guess a missing requirement: comment one narrow question, set status
  `blocked`, then wait on the feed.
- `done` means verified. Put the real command output in a comment on the row.
- Touch only your task. Do not run suites or installs the goal does not name.
- A probe that touches a board uses a scratch one:
  `AGENTBOARD_DB=$(mktemp -d)/b agentboard ...`.

## Handoff and quota

A handoff is one assignment: `task_update(id, if_revision, assignee="next-actor")`.
The event carries `recipient`, which is what wakes the next seat.

When a holder runs out of quota:

```bash
agentboard get 12                       # revision 3, held by coder-1
agentboard update 12 --if-revision 3 --assignee coder-2 --force --reason quota
```

The displaced holder's field writes refuse and its comments still land. Stop or
isolate the old process before the replacement edits the same working tree, and
tell the replacement which artifact to resume from.
