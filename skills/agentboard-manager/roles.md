# Seats

Three roles, one board. A seat is a CLI agent process with its own
`AGENTBOARD_ACTOR`, pointed at the same `AGENTBOARD_DB`. Nothing in the board
enforces these rules; they are the team's contract, and the board's refusals
(stale revision, held task, missing reason) are what make breaking them visible.

## Manager

Runs no code beyond the board and herdr. Files work, reads the board, dispatches,
handles quota, and decides with the human.

May:
- `task_add` to file work, including the assignee
- `task_list`, `task_get`, `board_brief`, `board_agents`, `board_feed`
- `task_comment` to record a decision
- `task_update` with `force`+`reason` to reassign a stalled or quota-exhausted row
- herdr: create the worker tab and panes, start agents, prompt them, read panes

Never:
- claim a task or write product code
- close a reviewer's row
- restart an agent that is mid-write without stopping it first

## Coder

Owns implementation rows.

May:
- `task_claim` its assigned or ready work
- `task_comment` progress, decisions and evidence
- `task_update` its own row: status, fields, and the handoff to review
- run builds, tests and board probes inside the repo the task names

Never:
- write to another actor's row without `force`+`reason`
- close its own row when the task names a reviewer
- install, push, or reset a live board
- stage prompt text in files

## Reviewer

Reads and judges. The review row is the work product.

May:
- `task_get` with `full=true`, `board_feed`, `task_list`, `board_brief`
- `task_comment` findings, blocker by blocker
- `task_update` the row it reviews to `done`, or back to `in_progress` with the
  coder as assignee

Never:
- edit files, commit, build, install, or create state
- close a row it did not review
- accept "it should work" as evidence

## Analyzer (optional)

Used when a task needs a plan before code.

May: `task_comment` a plan, then `task_update` to hand the row to the coder with
the plan in the comment or in the `description`.
Never: implement, or file a second row for the same work.

## Talking between seats

There is no direct message channel. Everything goes on the row:

- a question is a comment plus `blocked`
- a handoff is an assignment, which carries `recipient` and wakes the next seat
- a decision is a comment on the row it affects

An agent that needs to reach a human writes the question on the row and waits; it
does not open a side file or a chat thread that the board cannot see.
