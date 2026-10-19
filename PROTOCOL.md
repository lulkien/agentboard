# Working the board (v2)

The board is the only status channel. No `NOTES.md`, `STATUS.md`, or summary files — use `task_update(note=...)`. One task at a time per session is a convention, not enforced by the server: finish or block before claiming again. Long task? `task_update(heartbeat=true)` before the lease expires (30 min default). Touch only your task; `session_tail` to watch another pane.

Never guess a missing requirement. `task_ask` with ONE narrow question, then `task_wait(after_seq=<#seq from ack>)`. If the answer is partial, ask again.

`done` means verified — run the check the goal names and put its output in `report`. `review` when a human should look (API changes, irreversible ops). Notes are for the next reader: decisions, surprises, commands worth rerunning.

## Workflow: epic, implement, review, fix

Work that needs review runs as one loop. An epic is not a task: it has its own table and ids (`E1`, `E2`...), is never claimed, leased or dispatched, and names the project the work belongs to (`path`). Tasks point at an epic (`epic=E1`) and optionally at another task (`parent=T7`, a subtask). `board_brief` shows one `EPIC E1 2/4 done [1 blocking] /abs/path <title>` line per open epic, then the open tasks that belong to no epic. Dropped tasks are left out of the total. Nothing closes an epic automatically.

1. Manager opens the epic: `epic_add(title, goal, path=<project root>)` (CLI: `agentboard epic add "..." --path /abs/dir`). A path that does not exist yet is accepted with a warning.
2. Manager adds the implement task with `epic=<E id>`. Hand the worker only the task id.
3. Agent claims it, implements, then `task_update(report=..., status=review|done)`.
4. Manager polls `task_get` / `board_brief` (no tight loops; `task_wait` blocks server-side) and reads `report`.
5. Manager adds a review task with `epic=<E id>`; the reviewer claims it.
6. Reviewer either sets the review task `done`, or files a fix task with `parent=<review task>` (`task_add`; it inherits the epic) and sets the review task `needs_fix`.
7. Manager delegates the fix. The fixer claims the fix task, fixes, sets it `done` with a report.
8. Manager delegates verification to the same reviewer agent that filed the fix (no separate checker task). It re-checks its own findings and sets the review task `done`.
9. Manager closes the epic once every task is terminal (`done` or `dropped`): `epic_update(id, status=done)`.

Inspect with `epic_get(E1)` (task counts, path, timeline) and `task_children(id)`: an epic id lists every task in it, a task id lists its subtasks (CLI: `agentboard children E1` / `T27`). `task_get` shows `epic:` and `parent:` lines and, for a task with subtasks, `children: N (2 done, ...)`.

The two id spaces never mix: `E` ids go to epic verbs and `epic=`/`--epic`, `T` ids to task verbs and `parent=`; the wrong one is refused with a hint. A subtask must live in its parent's epic; a self-parent, a cycle and a cross-epic parent are errors (`agentboard parent T5 T3|none` re-parents). `dispatch` warns (never refuses) when a task's epic has a path and the target pane's cwd, as herdr reports it, is outside it.

Create the task with `task_add(title, description, goal, priority, assignee, epic, parent)`. `description` holds everything the worker needs: repo path, files, situation, constraints. `goal` holds how to do it and the definition of done. Do not block — poll `task_get` or `board_brief` for status changes. When status is `review` or `done`, read `report` for the outcome.

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

`agentboard edit T7|E3 ...` / `task_edit` / `epic_edit` amend a spec and are refused while the task is `working` with a live lease (the worker is reading it); free it with `agentboard status T7 ready` first. Each edit logs one `edit` event: short fields record the values, `title "old"->"new"` (title, assignee, path; each clipped to 80 chars, so a same-length change shows and the old text survives), `priority 100->7`, and long fields only the lengths, `description 812->830` (description, goal). A worker re-reading its task sees the spec moved in the timeline.

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
- `review` — human should review
- `done` — verified complete
- `dropped` — abandoned

## Cost note

The full tool surface is ~1.8k tokens per request; `task_claim` returns the task so a follow-up `task_get` is never needed. Writes ack in one line on purpose.