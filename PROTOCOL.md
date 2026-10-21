# Working the board (v2)

The board is the only status channel. No `NOTES.md`, `STATUS.md`, or summary files — use `task_update(note=...)`. One task at a time per session is a convention, not enforced by the server: finish or block before claiming again. Long task? `task_update(heartbeat=true)` before the lease expires (30 min default). Touch only your task; `session_tail` to watch another pane.

Never guess a missing requirement. `task_ask` with ONE narrow question, then `task_wait(after_seq=<#seq from ack>)`. If the answer is partial, ask again.

`done` means verified — run the check the goal names and put its output in `report`. `review` means ready for a second pair of eyes: the reviewer in the loop below, or a human when the change is an API break or an irreversible operation. It is not a finished state; `done` is. Notes are for the next reader: decisions, surprises, commands worth rerunning.

## Workflow: epic, implement, review, fix

Work that needs review runs as one loop. An epic is not a task: it has its own table and ids (`E1`, `E2`...), is never claimed, leased or dispatched, and names the project the work belongs to (`path`). Tasks point at an epic (`epic=E1`) and optionally at another task (`parent=T7`, a subtask). `board_brief` shows one `EPIC E1 2/4 done [1 blocking] /abs/path <title>` line per open epic, then the open tasks that belong to no epic. Dropped tasks are left out of the total. Nothing closes an epic automatically.

1. Manager opens the epic: `epic_add(title, goal, path=<project root>)` (CLI: `agentboard epic add "..." --path /abs/dir`). A path that does not exist yet is accepted with a warning.
2. Manager adds the implement task with `epic=<E id>` and dispatches it. Hand the worker only the task id; the row carries description and goal.
3. Worker claims it with `task_claim`, does the work, records decisions with `task_update(note=...)`, and finishes with `task_update(report=..., status="review")`. The report names files and pastes the output of the check the goal asked for.
4. Manager reads `report` (`task_get`, or `task_wait` to block server-side — never a poll loop), files a review task with `epic=<E id>`, and dispatches the reviewer.
5. Reviewer claims the review task, reads the diff and the report. Clean: `task_update(id, report=<what was checked>, status="done")` on the review task. Findings: the reviewer files the fix task itself — `task_add(..., parent=<review task id>)`, which inherits the epic — and sets its own review task `needs_fix`.
6. Coder claims the fix task, fixes only the listed findings, answers each finding in `report`, and sets the fix task `review`. The coder never sets a fix task `done`, never closes a review task, and there is no `fixed` status: on a fix task `review` means fixed and awaiting re-check.
7. Reviewer re-checks its own findings — the same reviewer, no separate checker task. Every finding closed: set the **fix** task `done` with the evidence in `report`, then re-read the file and set the review task `done`. Anything still open: write a `note` naming it on the fix task and set that fix task `needs_fix`; the review task stays `needs_fix`. Loop back to step 6.
8. Manager re-runs the gates the goal names, in that tree, before believing any report — a report is a claim, not proof — then closes the epic once every task is `done` or `dropped`: `epic_update(id, status="done")`.

Status ownership in this loop is fixed:

- Coder: `ready`/`needs_fix` → `working` → `review`. Nothing else.
- Reviewer: `review` → `done` or `needs_fix`, on a fix task and on a review task alike.
- Manager: `ready` (re-arm a crashed task), `dropped`, `done` on an implement task it owns, and epic status.

A fix task reaching `done` is the reviewer's signature and its `report` must carry the evidence. The same loop as diagrams: [docs/workflow.md](docs/workflow.md).

Inspect with `epic_get(E1)` (task counts, path, timeline) and `task_children(id)`: an epic id lists every task in it, a task id lists its subtasks (CLI: `agentboard children E1` / `T27`). `task_get` shows `epic:` and `parent:` lines and, for a task with subtasks, `children: N (2 done, ...)`.

The two id spaces never mix: `E` ids go to epic verbs and `epic=`/`--epic`, `T` ids to task verbs and `parent=`; the wrong one is refused with a hint. A subtask must live in its parent's epic; a self-parent, a cycle and a cross-epic parent are errors (`agentboard parent T5 T3|none` re-parents). `dispatch` warns (never refuses) when a task's epic has a path and the target pane's cwd, as herdr reports it, is outside it.

Create the task with `task_add(title, description, goal, priority, assignee, epic, parent, path, branch)`. `path` is the project root the work lives in (`~` expands, relative resolves against the server's cwd) and `branch` the git ref the worker starts on; the default `HEAD` (or an empty string) means leave the pane's checkout alone. Branch names are case-sensitive: only the literal `HEAD` means that, `head` is an ordinary branch name. `task_get` shows both as `path:` (`-` when unset) and `branch:`. `description` holds everything the worker needs: repo path, files, situation, constraints. `goal` holds how to do it and the definition of done. Do not block — poll `task_get` or `board_brief` for status changes. When status is `review` or `done`, read `report` for the outcome.

## Worker flow

`task_claim(id)` → read `description` and `goal` → do the work → `task_update(note=...)` for progress → `task_update(report=..., status=review|done)` with the outcome. Any agent may create a child task (`parent=<id>`); a reviewer uses this to file the fix task.

## Fields

| Field | Meaning |
|-------|---------|
| id | Unique task id (T7) |
| title | One-line summary |
| description | Everything the worker needs (repo, files, situation) |
| goal | How to do it + definition of done |
| status | ready \| working \| blocking \| not_done \| needs_fix \| review \| done \| dropped |
| assignee | Profile or tmux session name |
| priority | Lower runs first (default 100) |
| epic | Epic id (e.g., E3) the work belongs to; inherited from `parent` when omitted |
| parent | Parent task id (e.g., T3): a subtask link, such as a fix task under the review that found it |
| needs | Input type the task waits for (e.g., input) |
| report | Worker's findings/outcome |
| note | Free-form progress note |
| commit_ref | Commit or PR reference |
| lease_until | Lease expiry timestamp |
| created_at | Creation timestamp |
| updated_at | Last update timestamp |
| started_at | When work began |
| completed_at | When status reached done/dropped |

## Editing a spec

`agentboard edit T7|E3 ...` / `task_edit` / `epic_edit` amend a spec and are refused while the task is `working` with a live lease (the worker is reading it); free it with `agentboard status T7 ready` first. Each edit logs one `edit` event: short fields record the values, `title "old"->"new"` (title, assignee, path, branch; each clipped to 80 chars, so a same-length change shows and the old text survives), `priority 100->7`, and long fields only the lengths, `description 812->830` (description, goal). A worker re-reading its task sees the spec moved in the timeline.

## Long text and truncation

`task_get` and `task_claim` fold newlines to ` ⏎ ` (3 chars) and cap long fields so a read stays cheap: description 4000, goal 4000, report 6000, note 800. Caps, counts and markers are all in chars of the folded view, so a text with many short lines hits the cap sooner than its raw length suggests. A field over its cap ends in a marker, never silently:

    [truncated description: 4000 of 5000 chars shown; full text: `agentboard get T7 --full` or task_get id=T7 full=true]
    [truncated description: 4000 of 6800 chars shown (newline = 3 chars; 3400 raw); full text: ...]

The marker names the field, the chars shown, the folded length (plus the raw length when newlines make them differ) and the way to the rest. A worker that sees it must fetch the full value before acting: `task_get(id, full=true)` over MCP, `agentboard get T7 --full` on the CLI. `full` returns description, goal, report, note and the event bodies of the last-8 events unclipped, byte-for-byte, with real newlines and no marker.

Other clips, all marked or absent:

- `get` events (last 8): cut at 220 chars with `[truncated: 220 of N chars; ...]`; `full` returns them whole.
- `task_wait` / `agentboard wait`: event bodies are returned whole (answers arrive here). On timeout the reply starts with `timeout:`, then one status line per watched task, then a `last (T7):` tail of the freshest task, cut at 400 chars with a `[truncated: 400 of N chars; task_get id=T7 for the record]` marker.
- Titles are cut at 60-64 chars with a bare `…` in `task_list`, `task_children` and `board_brief`, and `board_brief` result lines at 80; those are one compact line per task by design. Read `task_get` for the whole title.

## Status meanings

- `ready` — waiting to be claimed (alias: `triage`)
- `working` — actively being done (alias: `running`)
- `blocking` — waiting on external input (alias: `blocked`)
- `needs_fix` — review found work, waiting for a fix (aliases: `need fix`, `needs-fix`, `fix`)
- `not_done` — the board stopped the row: its fourth move into `needs_fix` (the reopen cap is 3) is refused with exit 1, the row lands here with the caller's body kept in the event, and its parent row, if open (`ready`, `working`, `blocking`, `needs_fix`, `review`), lands here too (one level only; a `done`, `dropped` or `not_done` parent is left alone and the capped row gets a note naming it); aliases `not-done`, `nd`. `get` shows `reopens: N/3`, and `(cap hit, stopped)` above 3. A parent chain is at most three levels (implement → review → fix); a child of a level-3 row is refused.
- `review` — ready for a second pair of eyes: the reviewer, or a human for an API break or an irreversible operation
- `done` — verified complete
- `dropped` — abandoned

## Cost note

The full tool surface is ~1.8k tokens per request; `task_claim` returns the task so a follow-up `task_get` is never needed. Writes ack in one line on purpose.