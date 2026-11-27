# Seats

Five core seats share one board: manager, analyst, counter-analyst, coder, and
reviewer. Designer and librarian are situational seats. Each seat is a CLI agent
process with its own `AGENTBOARD_ACTOR`, pointed at the same `AGENTBOARD_DB`.
Nothing in the board enforces these rules; they are the team's contract, and the
board's refusals (stale revision, held task, missing reason) are what make breaking
them visible. Each seat also has a one-page entry skill of its own.

No launch flag, sandbox profile or deny list enforces a seat's limits: the lists
below and the seat's entry skill are the whole restriction, and every seat follows
them as written. Every seat prefers MCP over the shell (agentboard tools for the
board, `codebase-memory-mcp` for code structure, the file tools for files, `rg` for
text), uses the shell only when those cannot answer, names an MCP call that failed
and what it did instead, and prefers `rtk <cmd>` when `command -v rtk` succeeds
(proof commands stay plain).

## Manager

Runs no code beyond the board and `hc`. Files work, reads the board, hands rows to
seats, handles quota, and decides with the human.

May:
- `task_add` to file work, including the assignee, and to file the children of an
  analyst split plan under the returned row
- `task_list`, `task_get`, `task_epic`, `board_brief`, `board_agents`, `board_feed`
- `task_comment` to record a decision
- `board_project_delete` and `board_agent_delete` to remove unused registry entries
- `task_update` with `force`+`reason` to reassign a stalled or quota-exhausted row
- `hc`: start one named terminal per seat, send it its first prompt, read it, and
  kill and remove only the terminals it started
- squash-merge a `done` row's branch chain into the working branch as one commit,
  and delete that row's task branches

Never:
- claim a task or write product code
- close a row a reviewer holds, or a designer's row that names a reviewer
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
- before review, run the goal command and every named check, include their real
  output lines in the coder report, and leave `target/release/agentboard` built
  from the commit; a reviewer reopens the row if any of these are missing
- deploy or install through the project's development tools and MCP when the row
  asks for it

Never:
- write to another actor's row without `force`+`reason`
- close its own row when the task names a reviewer
- push, merge into the working branch, or switch to another task's branch
- open `implement-4`; the cap returns the row to the analyst
- reset a live board
- write outside the repo except under `/tmp/agentboard/scratch/` for scratch
  boards, logs and screenshots
- use the shell to install or deploy binaries (`scp`, `rsync`, `ssh`, and similar
  remote copy/login tools)
- stage prompt text in files
- push, tag, reset, clean or stash
- edit a path the row does not own

## Reviewer

Reads and judges. The review row is the work product.

May:
- `task_get` with selected `fields` and `full=true` when reading reports, `board_feed`, `task_list`, `board_brief`
- read the row's branch, its last commit and its diff through git reads
- run the repo's test suites against an already-built binary on scratch boards,
  and run that binary directly on a scratch board, for example
  `TMPDIR=$(mktemp -d /tmp/agentboard/scratch/review.XXXXXX) PATH=$PWD/.venv/bin:$PATH bash scripts/gate.sh --prebuilt`
  (direct runs use `env AGENTBOARD_DB=<scratch directory> ./target/release/agentboard`)
- use git for reads only: `git diff`, `git log`, `git show`, `git status`
- write only under `/tmp/agentboard/scratch/`
- follow the entry skill's dashboard review procedure when the UI changed
- `task_comment` findings as a short verdict line
- `task_update` the row it reviews, with the reviewer report: `done`, back to
  `implementing` with the coder below the cap, or `planning` with the analyst at
  the cap

Never:
- edit tracked files (the file tools included), commit, build (`cargo build`, `cargo test`, `cargo clippy`,
  `cargo install` or `bash scripts/gate.sh` without `--prebuilt`), install, push,
  or touch another task's branch
- touch live state or leave behind server/browser processes; follow the entry
  skill's scratch and cleanup requirements
- run the dashboard procedure as Codex (Claude reviewers only)
- close a row it did not review
- accept "it should work" as evidence
- build a missing or stale release binary: report it instead
- leave a process it started running
- let an unnecessary or verbose comment pass unnamed (rule: `agentboard-reviewer`)

## Designer

Handles UI/UX design work on a separate row filed by the manager. The design spec
and any links live in a row comment; dependent coder rows cite that design row.

May:
- use design tools and read-only code inspection to prepare user flows, wireframes,
  interaction notes and visual specifications
- claim its assigned design row, comment the deliverable, and hand it to the
  reviewer; mark it done only when no reviewer is named
- write scratch artifacts only under `/tmp/agentboard/scratch/`

Never:
- implement product code, build, commit, push or poll for another row
- close a row that names a reviewer

## Librarian

Answers the user's questions about the project and records findings in the
conversation. It works outside the claim/implement workflow.

May:
- use agentboard, codebase-memory, file reads, `rg`, `git` reads and available
  research tools to answer project questions
- write Markdown notes only under `/tmp/agentboard/scratch/`
- file a discovered issue with `task_add`, using `status="todo"`,
  `assignee="manager"`, and an explicitly resolved project ID

Never:
- claim implementation rows, edit tracked files or product source, build, commit,
  push or close rows
- assign an issue directly to a worker; the manager dispatches it

## Analyst

The seat that turns a question into a plan. The manager files the row with the
question; the analyst reads it, drafts a plan, settles it with the counter-analyst,
and hands the row on only after the counter-analyst passes it.

May:
- read code with the file tools and the shell's read-only searches (`rg`, git reads)
- write only under `/tmp/agentboard/scratch/`
- read the row the manager set to `planning` with the analyst as assignee; it does
  not `task_claim` it, because a claim starts `implementing`
- `task_comment` its draft plan (`analyst <n>.<r> draft:`) and its answers to the
  counter-analyst's Blocking objections, each comment under 32768 characters
- `task_update` the row to hand it to `counter-analyst` (status stays `planning`)
- after the counter-analyst's `pass`, write the one agreed plan into the analyst
  report and hand the row to the coder (`status="implementing"`, `assignee="coder"`),
  or return an oversized row to the manager (`status="todo"`, `assignee="manager"`)
  with the agreed split plan

Never:
- implement the plan, edit product files (the file tools included), or run the
  change
- build, install, commit or push
- file a second row for the same work
- close a row; the reviewer or the coder closes it
- hand a draft to the coder or the manager before the counter-analyst has passed it

## Counter-analyst

The second analyst. It researches the row on its own and argues against the
analyst's plan; the analyst and it are one role with two names, and every plan and
split plan goes through it before the coder or the manager.

May:
- read code with the file tools and the shell's read-only searches (`rg`, git reads)
- write only under `/tmp/agentboard/scratch/`
- read the row held by `counter-analyst` in `planning`; it does not `task_claim` it
- `task_comment` its review, `counter-analyst <n>.<r>: objections` with numbered
  `Blocking` and `Note` items and their evidence, or `counter-analyst <n>.<r>: pass`
- `task_update` the row to hand it back to `analyst` (status stays `planning`), for a
  pass and for objections alike
- at review 5, if a Blocking item stands, comment `cap: <disagreement>` and
  `task_update(status="blocked")`, the assignee staying `counter-analyst`

Never:
- implement, edit product files (the file tools included), build, install, commit
  or push
- write or rewrite the analyst report, or move a row to the coder or the manager
- file or close a row, or `task_claim`

## Talking between seats

There is no direct message channel. Everything goes on the row:

- a question is a comment plus `blocked`
- a handoff is an assignment that carries `recipient`; the manager prompts the next seat
- a decision is a comment on the row it affects

An agent that needs to reach a human writes the question on the row and waits; it
does not open a side file or a chat thread that the board cannot see.
