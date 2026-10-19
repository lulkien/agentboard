# Working the board (v2)

The board is the only status channel. No `NOTES.md`, `STATUS.md`, or summary files — use `task_update(note=...)`. One task at a time per session is a convention, not enforced by the server: finish or block before claiming again. Long task? `task_update(heartbeat=true)` before the lease expires (30 min default). Touch only your task; `session_tail` to watch another pane.

Never guess a missing requirement. `task_ask` with ONE narrow question, then `task_wait(after_seq=<#seq from ack>)`. If the answer is partial, ask again.

`done` means verified — run the check the goal names and put its output in `report`. `review` when a human should look (API changes, irreversible ops). Notes are for the next reader: decisions, surprises, commands worth rerunning.

## Workflow: epic, implement, review, fix

Work that needs review runs as one loop. A task with children is an epic; `board_brief` shows one `EPIC T27 2/4 done <title>` line per open epic that still has open children. Dropped children are left out of the total. Nothing closes an epic automatically.

1. Manager opens the epic: `task_add(title, ...)`.
2. Manager adds the implement task with `parent=<epic>`. Hand the worker only the task id.
3. Agent claims it, implements, then `task_update(report=..., status=review|done)`.
4. Manager polls `task_get` / `board_brief` (no tight loops; `task_wait` blocks server-side) and reads `report`.
5. Manager adds a review task with `parent=<epic>`; the reviewer claims it.
6. Reviewer either sets the review task `done`, or files a fix task with `parent=<review task>` (`task_add`) and sets the review task `needs_fix`.
7. Manager delegates the fix. The fixer claims the fix task, fixes, sets it `done` with a report.
8. Manager delegates verification to the same reviewer agent that filed the fix (no separate checker task). It re-checks its own findings and sets the review task `done`.
9. Manager closes the epic (`done`) once every child is terminal (`done` or `dropped`).

Inspect an epic with `task_children(id)` (CLI: `agentboard children T27`); `task_get` appends `children: N (2 done, 1 working, ...)`.

`needs_fix` means review found work and is waiting for a fix. It holds no lease. From `needs_fix` a task may go to `working` or `review`, or back to `ready` to restart.

## Manager flow

Create the task with `task_add(title, description, goal, priority, assignee, parent)`. `description` holds everything the worker needs: repo path, files, situation, constraints. `goal` holds how to do it and the definition of done. Do not block — poll `task_get` or `board_brief` for status changes. When status is `review` or `done`, read `report` for the outcome.

## Worker flow

`task_claim(id)` → read `description` and `goal` → do the work → `task_update(note=...)` for progress → `task_update(report=..., status=review|done)` with the outcome. Any agent may create a child task (`parent=<id>`); a reviewer uses this to file the fix task.

## Fields

| Field | Meaning |
|-------|---------|
| id | Unique task id (T7) |
| title | One-line summary |
| description | Everything the worker needs (repo, files, situation) |
| goal | How to do it + definition of done |
| status | ready \| working \| blocking \| needs_fix \| review \| done \| dropped |
| assignee | Profile or tmux session name |
| priority | Lower runs first (default 100) |
| parent | Parent task id (e.g., T3); a task with children is an epic |
| needs | Input type the task waits for (e.g., input) |
| report | Worker's findings/outcome |
| note | Free-form progress note |
| commit_ref | Commit or PR reference |
| lease_until | Lease expiry timestamp |
| created_at | Creation timestamp |
| updated_at | Last update timestamp |
| started_at | When work began |
| completed_at | When status reached done/dropped |

## Status meanings

- `ready` — waiting to be claimed (alias: `triage`)
- `working` — actively being done (alias: `running`)
- `blocking` — waiting on external input (alias: `blocked`)
- `needs_fix` — review found work, waiting for a fix (aliases: `need fix`, `needs-fix`, `fix`)
- `review` — human should review
- `done` — verified complete
- `dropped` — abandoned

## Cost note

The full tool surface is ~1.8k tokens per request; `task_claim` returns the task so a follow-up `task_get` is never needed. Writes ack in one line on purpose.