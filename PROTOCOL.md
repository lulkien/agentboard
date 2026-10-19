# Working the board (v2)

The board is the only status channel. No `NOTES.md`, `STATUS.md`, or summary files — use `task_update(note=...)`. One task at a time per session is a convention, not enforced by the server: finish or block before claiming again. Long task? `task_update(heartbeat=true)` before the lease expires (30 min default). Touch only your task; `session_tail` to watch another pane.

Never guess a missing requirement. `task_ask` with ONE narrow question, then `task_wait(after_seq=<#seq from ack>)`. If the answer is partial, ask again.

`done` means verified — run the check the goal names and put its output in `report`. `review` when a human should look (API changes, irreversible ops). Notes are for the next reader: decisions, surprises, commands worth rerunning.

## Manager flow

Create the task with `task_add(title, description, goal, priority, assignee)`. `description` holds everything the worker needs: repo path, files, situation, constraints. `goal` holds how to do it and the definition of done. Hand the worker only the task id. Do not block — poll `task_get` or `board_brief` for status changes. When status is `review` or `done`, read `report` for the outcome.

## Worker flow

`task_claim(id)` → read `description` and `goal` → do the work → `task_update(note=...)` for progress → `task_update(report=..., status=review|done)` with the outcome.

## Fields

| Field | Meaning |
|-------|---------|
| id | Unique task id (T7) |
| title | One-line summary |
| description | Everything the worker needs (repo, files, situation) |
| goal | How to do it + definition of done |
| status | ready \| working \| blocking \| review \| done \| dropped |
| assignee | Profile or tmux session name |
| priority | Lower runs first (default 100) |
| parent | Parent task id (e.g., T3) |
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
- `review` — human should review
- `done` — verified complete
- `dropped` — abandoned

## Cost note

The full tool surface is ~1.8k tokens per request; `task_claim` returns the task so a follow-up `task_get` is never needed. Writes ack in one line on purpose.