---
name: agentboard-manager
description: "Use when working a task on the agentboard, or managing an analyzer, coder and reviewer team on it."
version: 3.0.0
author: Hermes Agent
license: MIT
platforms: [linux, macos]
metadata:
  hermes:
    tags: [agentboard, herdr, manager, analyzer, coder, reviewer, delegation]
    related_skills: [claude-code, opencode]
---
# agentboard manual

The board holds work (rows, status, reports, events). herdr holds processes (tabs, panes, agents).
Full wire reference: `PROTOCOL.md`. Tool reference: `README.md`. Both live in the agentboard repo.
Three pages here, nothing else binding:

- this manual: the verbs and exit codes
- `workflow.md`: how a row moves, who moves which status
- `roles.md`: manager, analyzer, coder, reviewer: what each may run and never does

Read this page and `workflow.md`. Read `roles.md` for your seat.
Start the manager seat with `/manager` (Claude Code, opencode, Hermes) or `$manager` (Codex); entry points live in the repo at `skills/manager/`.

## The three-row chain

The manager files ONE analyze root and waits. The analyzer publishes the plan as
`needs_impl` and files the implement child; the coder implements that plan,
records the diff, files the review child, and publishes `needs_review`; the
reviewer reopens the implement row or closes all three rows. The board enforces
the shape: one child per row, a review row takes no child, and rework reopens the
implement row instead of adding a fourth row. The same review row serves every
round.

Two things the manager must never do on a chain:

- file the implement row or the review row; those belong to the analyzer and the
  coder, and the board refuses a review row the coder did not file. The manager
  files only the analyze root;
- chase a seat by hand. Read the board, dispatch the next stage, and let the
  lease and the chain move the row. There is no pane typing by the manager.

A stage that is waiting on the next seat can sit quietly without being stalled:
add `needs_impl` and `needs_review` to `needs_fix`, `blocking` and `done` in
every watch set. A status filter matches a status the board already holds, so
arm the stage row with statuses it has not reached and never re-arm a satisfied
list; an event-kind filter is the broad alternative.

## Rules for every agent

- The board is the only status channel. No `NOTES.md`, `STATUS.md`, summary files.
- The row is the prompt: `claim` returns `description` and `goal` verbatim. Do not stage prompt text in files.
- Write to the board through the MCP tools. Do not shell out to the `agentboard` binary for board writes. One exception: a diff body too big for an inline MCP string goes in with `diff add --body-file` (`references/diff-submission.md`).
- Never guess a missing requirement: `task_update(status="blocking", note=<one narrow question>)`, then `task_wait`.
- `done` means verified. Paste the real output of the check the goal names into `report`.
- Touch only your row. Do not run suites or installs the goal does not name.
- A probe that touches a board uses a throwaway one: set `AGENTBOARD_DB` (and `AGENTBOARD_SURREAL_URL`) in the same command. A bare `agentboard` call hits the live board.

## Board verbs (MCP tool, CLI verb)

| Job | MCP tool | CLI |
|---|---|---|
| take a row, get description and goal | `task_claim`, then `task_get full=true` | `claim [T]`, `get T --full` |
| progress note | `task_update note=` | `note T "..."` |
| keep a long lease | `task_update heartbeat=true` | `heartbeat T` |
| record the change | `diff_add id body=` | `diff add T (--body TEXT\|--body-file PATH\|-)` |
| outcome and status | `task_update report= status=` | `report T "..."`, `status T <s> [note]` |
| hand the next row to its seat | `task_dispatch id= to=` | `dispatch T<n> --to <name>` |
| block on a human | `task_update status=blocking note=<question>` | `ask T "..."` |
| wait for the board | `task_wait` | `wait [T..] [--epic E] [--open] [--status csv] [--kind csv] [--timeout S]` |
| read | `task_get`, `task_list`, `task_children`, `diff_list`, `diff_get`, `board_brief` | `get`, `ls`, `children`, `diff ls`, `diff get`, `brief` |

Lease is 30 min by default. A long diff has no MCP shape (inline string only): use `diff add --body-file`; see `references/diff-submission.md`.

## Exit codes

- Any refusal exits 1 with an `agentboard: ` prefix on stderr (unknown task, bad value, refused move, cap hit, `chain too deep`, refused `claim` of an explicit id). MCP returns the same text as a tool error.
- `wait` exits 0 and prints `matched:`; exits 3 and prints `timeout:`. `manager events wait` uses the same two codes.
- The id-less `claim` with nothing ready prints `no ready tasks` and exits 0.
- `dispatch` refuses a row that is not `ready` (exit 1; `--force` resets it first).

## Manager verbs (manager only)

`agentboard manager ...` drives herdr. There is no shell helper.

| Move | Verb |
|---|---|
| open a team: coder and reviewer, optionally analyzer, registered in one tab | `manager up --repo P [--coder claude] [--reviewer opencode] [--analyzer K] [--role-name-prefix X] [--env K=V]` |
| board sessions joined with live agents (registered / gone / unregistered) | `manager status [--json]` |
| herdr's session snapshot | `manager snapshot [--json]` |
| map a pane's agent name to the board | `register <name> --agent claude\|opencode [--task T]` (`manager up` already registers) |
| close a team; refuses a working or blocked agent | `manager down [--force] [--names A,B]` |
| clear context at an epic boundary; refuses a working seat | `manager renew [--names A,B]` |
| one agent: list, get, read, start, prompt, keys, rename, focus, wait | `manager agent <verb>` |
| panes and tabs | `manager pane <verb>`, `manager tab <verb>` |
| wake on an agent status | `manager events wait --until S [--agent N] [--timeout 300]`, or `wait T7 --agent N --status ...` |
| hand a row to a pane | `dispatch T<n> --to <name> [--force] [--print]` |
| file and amend | `epic add`, `add --epic E --parent T`, `edit`, `parent`, `epic status` |

`manager up --analyzer K` adds the analyzer to the usual coder/reviewer pair.
All requested binaries and required profiles must exist before any pane is made.
The names are the role names with any prefix; each pane gets its role actor.
Repeated `--env` keys are refused by name before any pane is created.
For a three-seat team, pass all names to `manager down` or `manager renew`
(`--names analyzer,coder,reviewer`, with the prefix if used); their defaults
continue to target coder and reviewer only.

Manual moves that still need plain `herdr` (attach, `/btw`, killing a TUI by process group, seat swap): `references/herdr-mechanics.md`.

## References (read only when the case arises)

- `references/diff-submission.md`: recording a large or verbatim diff, proving the bytes.
- `references/worker-claim-probes.md`: reproducing a worker's "the engine cannot do this" claim.
- `references/herdr-mechanics.md`: what the manager verbs do not cover.
- `references/herdr-socket-api.md`: herdr socket protocol notes.

## Schema compatibility

Each CLI and MCP call reads the board schema version first. A newer board
refuses by name; an older one migrates additively under the write lock within
that call. Seat start runs no build comparison or refusal. The manager reloads
MCP processes when a new binary is needed. See `PROTOCOL.md`, Board schema version.
