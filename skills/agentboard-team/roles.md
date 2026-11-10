# Seats

Four seats, one board: manager, analyst, coder, reviewer. A seat is a CLI agent
process with its own `AGENTBOARD_ACTOR`, pointed at the same `AGENTBOARD_DB`.
Nothing in the board enforces these rules; they are the team's contract, and the
board's refusals (stale revision, held task, missing reason) are what make breaking
them visible. Each seat also has a one-page entry skill of its own.

## Manager

Runs no code beyond the board and `hc`. Files work, reads the board, hands rows to
seats, handles quota, and decides with the human.

May:
- `task_add` to file work, including the assignee, and to file the children of an
  analyst split plan under the returned row
- `task_list`, `task_get`, `task_epic`, `board_brief`, `board_agents`, `board_feed`
- `task_comment` to record a decision
- `task_update` with `force`+`reason` to reassign a stalled or quota-exhausted row
- `hc`: start one named terminal per seat, send it its first prompt, read it, and
  kill and remove only the terminals it started
- squash-merge a `done` row's branch chain into the working branch as one commit,
  and delete that row's task branches

Never:
- claim a task or write product code
- close a reviewer's row
- push, or resolve a conflicted squash by editing product code
- restart an agent that is mid-write without stopping it first

## Coder

Owns implementation rows.

May:
- `task_claim` its assigned or ready work
- `task_comment` the short handoff line; long progress and evidence go in its report
- `task_update` its own row: status, fields, the coder report, and the handoff to review
- create and switch its task branch, and commit once on that branch
- run builds, tests and board probes inside the repo the task names
- deploy or install through the project's development tools and MCP when the row
  asks for it

Never:
- write to another actor's row without `force`+`reason`
- close its own row when the task names a reviewer
- push, merge into the working branch, or switch to another task's branch
- open `implement-4`; the cap returns the row to the analyst
- reset a live board
- use the shell to install or deploy binaries (`scp`, `rsync`, `ssh`, and similar
  remote copy/login tools)
- stage prompt text in files

## Reviewer

Reads and judges. The review row is the work product.

May:
- `task_get` with `full=true`, `board_feed`, `task_list`, `board_brief`
- read the row's branch, its last commit and its diff through git reads
- `task_comment` findings as a short verdict line
- `task_update` the row it reviews, with the reviewer report: `done`, back to
  `implementing` with the coder below the cap, or `planning` with the analyst at
  the cap

Never:
- edit files, commit, build, run tests, or install
- use a browser for anything but the local dashboard; for a dashboard review,
  the Claude reviewer may create a scratch board and run the already-built
  dashboard as described in `skills/agentboard-reviewer/SKILL.md`. It must never
  touch the live board or repository, and must stop the server and close its
  browser.
- close a row it did not review
- accept "it should work" as evidence
- let an unnecessary or verbose comment pass unnamed (rule: `agentboard-reviewer`)

## Analyst

The seat that turns a question into a plan. The manager files the row with the
question; the analyst reads it, plans, and hands the same row on.

May:
- read the row the manager set to `planning` with the analyst as assignee; it does
  not `task_claim` it, because a claim starts `implementing`
- `task_comment` one short line naming the analyst turn; the plan goes in the
  analyst report
- `task_update` the row to write its report and hand it to the coder
  (`status="implementing"`, `assignee="coder"`), or to return an oversized row to
  the manager (`status="todo"`, `assignee="manager"`) with a split plan

Never:
- implement the plan, edit product files, or run the change
- file a second row for the same work
- close a row; the reviewer or the coder closes it

## Talking between seats

There is no direct message channel. Everything goes on the row:

- a question is a comment plus `blocked`
- a handoff is an assignment, which carries `recipient` and wakes the next seat
- a decision is a comment on the row it affects

An agent that needs to reach a human writes the question on the row and waits; it
does not open a side file or a chat thread that the board cannot see.
