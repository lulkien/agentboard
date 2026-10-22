# Working the board (v2)

The board is the only status channel. No `NOTES.md`, `STATUS.md`, or summary files — use `task_update(note=...)`. One task at a time per session is a convention, not enforced by the server: finish or block before claiming again. Long task? `task_update(heartbeat=true)` before the lease expires (30 min default). Touch only your task; `session_tail` to watch another pane.

Never guess a missing requirement. `task_ask` with ONE narrow question, then `task_wait(after_seq=<#seq from ack>)`. If the answer is partial, ask again.

`done` means verified — run the check the goal names and put its output in `report`. `review` means ready for a second pair of eyes: the reviewer in the loop below, or a human when the change is an API break or an irreversible operation. It is not a finished state; `done` is. Notes are for the next reader: decisions, surprises, commands worth rerunning.

## Workflow: epic, implement, review, fix

Work that needs review runs as one loop. An epic is not a task: it has its own table and ids (`E1`, `E2`...), is never claimed, leased or dispatched, and names the project the work belongs to (`path`). Tasks point at an epic (`epic=E1`) and optionally at another task (`parent=T7`, a subtask). `board_brief` shows one `EPIC E1 2/4 done [1 blocking] /abs/path <title>` line per open epic, then the open tasks that belong to no epic. Dropped tasks are left out of the total. Nothing closes an epic automatically.

The chain is exactly three levels, **implement → review → fix**, and the board enforces it: the review row is a child of the implement row, the fix row is a child of the review row, and a child of a level-3 row (or a re-parent that would put a row at level 4) is refused with exit 1 and a `chain too deep` line. There is one fix row per review, reused across rounds.

1. Manager opens the epic: `epic_add(title, goal, path=<project root>)` (CLI: `agentboard epic add "..." --path /abs/dir`). A path that does not exist yet is accepted with a warning.
2. Manager adds the implement row with `epic=<E id>` and dispatches it. Hand the worker only the task id; the row carries description and goal.
3. Coder claims it with `task_claim`, does the work, records decisions with `task_update(note=...)`, records the change with `diff_add(id, body=<the diff>)`, and finishes with `task_update(report=..., status="review")`. The report names files and pastes the output of the check the goal asked for. A move into `review` is refused unless a submission row was recorded on that task after its newest claim (the gate reads the events table, not `started_at`). Two forms satisfy it: a normal diff, or an **evidence-only** submission for a re-run round whose tree did not change — `diff_add(id, body=<what you ran and its output>, reviews="D<n>")` (CLI: `--evidence-only --reviews D<n> --body-file -`), naming the newest patch of this row (same row, no re-parent). The evidence row is never read as a patch; the brief names the patch under review. The refusal names both forms.
4. The board files the review row. When a **parentless** row moves into `review`, the same transaction creates one child, `Review <implement title>`: status `ready`, no assignee, epic, path, branch and priority inherited, description carrying the implement row's report verbatim, the diff id the reviewer must read, and one line saying whether this round is a diff or evidence-only. The ack names it (`T7 review #31 (review row T8)`) and the implement row gets a note `review row T8 filed`. If the implement row is submitted again after a reopen, the existing review row is reused and gets a note naming the new submission; a second review row is never filed. A fix row or a review row (both have a parent) never files one.
5. Manager reads `report` (`task_get`, or `task_wait` to block server-side — never a poll loop) and dispatches the reviewer to the review row.
6. Reviewer claims the review row and reads only the diff the brief names (`diff_list` shows which, `diff_get` reads it; an evidence-only row is labelled as such and points at the patch it re-runs). Clean: `task_update(id, report=<what was checked>, status="done")` on the review row. Findings: the reviewer files the fix row itself — `task_add(..., parent=<review row id>)`, which inherits the epic — and sets its own review row `needs_fix`.
7. Coder claims the fix row, fixes only the listed findings, records `diff_add` on the fix row, answers each finding in `report`, and sets the fix row `review`. The coder never sets a fix row `done`, never closes a review row, and there is no `fixed` status: on a fix row `review` means fixed and awaiting re-check.
8. Reviewer re-checks its own findings against the new diff on the fix row — the same reviewer, no separate checker row. Every finding closed: set the **fix** row `done` with the evidence in `report`, then re-read the file and set the review row `done`. Anything still open: write a `note` naming it on the fix row and set that fix row `needs_fix`; the review row stays `needs_fix`. Loop back to step 7 with the same fix row.
9. Manager re-runs the gates the goal names, in that tree, before believing any report — a report is a claim, not proof. Only the manager closes the implement row (`done`), and closes the epic once every row is `done` or `dropped`: `epic_update(id, status="done")`.

Status ownership in this loop is fixed:

- Coder: `ready`/`needs_fix` → `working` → `review`. Nothing else.
- Reviewer: `review` → `done` or `needs_fix`, on a fix row and on a review row alike. Never on the implement row.
- Manager: `ready` (re-arm a crashed row), `dropped`, `done` on the implement row it owns, epic status, and clearing a `not_done` row (below).
- Board: `not_done`, and only through the reopen cap. An agent does not set it.

A fix row reaching `done` is the reviewer's signature and its `report` must carry the evidence. The same loop as diagrams: [docs/workflow.md](docs/workflow.md), which owns every diagram.

### Reviewer rules

1. The cap is final. When a row lands `not_done` the reviewer does not re-open it or work around it; re-scoping belongs to the manager.
2. Review the new diff only: the diff the review brief names — the newest patch of the row under review, or the patch an evidence-only submission re-runs. An older diff, or code the new diff did not touch, is not in scope.
3. Classify every finding as **blocker**, **follow-up** or **nit**. An observation that is not classified is not a finding and does not go in the fix row.
4. Never re-report a closed finding without new evidence quoted from the current diff.
5. Never edit. The reviewer reads the newest patch plus the row's `description` and `goal`, judges logic and edge cases against that spec, and files classified findings; it writes nothing to the tree (no edit, commit, build or install), because an edit makes the verdict grade its own change and leaves an unowned write behind.

The board does not check a diff body, so rule 2 has a companion: a plain diff whose body is prose is not admissible. A round with no tree change records an evidence-only submission instead (step 3), which is labelled as such and names the patch to read. `diff_get` shows what the reviewer is judging.

### The reopen cap

Each move of a row into `needs_fix` counts (`reopen_count`; `get` shows `reopens: N/3`, and `reopens: 4/3 (cap hit, stopped)` once capped). The count is never reset. The **fourth** attempt is refused: the CLI exits 1 and MCP returns a tool error, both with one line naming the cap and `not_done` (`T9 reopen cap 3 hit: the row is now not_done, not needs_fix`). The refused row is not left where it was; it lands `not_done`, and the body the caller passed is preserved in an event on the row, so the reviewer's findings survive.

If the capped row has a parent that is open (`ready`, `working`, `blocking`, `needs_fix` or `review`), that parent lands `not_done` in the same transaction with its own event naming the child and the cap. One level only, and the grandparent is not touched. A parent that is `done`, `dropped` or already `not_done` is left as it is, and the capped row gets a note naming the parent and its status. So a capped **fix** row also stops its review row, and the implement row stays at `review` for the manager.

A `not_done` row is not claimable and nothing re-arms it: re-opening it with `status` would not reset the count, so its next move into `needs_fix` is refused again. The manager unsticks it by dropping the rows (`status T<n> dropped`) or by filing a replacement implement row, which starts a new chain with a fresh count.

### Closing an epic is policy

An epic is closed by the manager once every row in it is `done` or `dropped`; a `not_done` row blocks closure until the manager drops it or refiles. This is **policy, not a guard**: `epic_update` does not check the rows, and nothing on the board stops an epic being closed with `not_done` rows in it. Do not read it as a property of the code.

Inspect with `epic_get(E1)` (task counts, path, timeline) and `task_children(id)`: an epic id lists every task in it, a task id lists its subtasks (CLI: `agentboard children E1` / `T27`). `task_get` shows `epic:` and `parent:` lines and, for a task with subtasks, `children: N (2 done, ...)`.

The two id spaces never mix: `E` ids go to epic verbs and `epic=`/`--epic`, `T` ids to task verbs and `parent=`; the wrong one is refused with a hint. A subtask must live in its parent's epic; a self-parent, a cycle and a cross-epic parent are errors (`agentboard parent T5 T3|none` re-parents). `dispatch` warns (never refuses) when a task's epic has a path and the target pane's cwd, as herdr reports it, is outside it.

Create the task with `task_add(title, description, goal, priority, assignee, epic, parent, path, branch)`. `path` is the project root the work lives in (`~` expands, relative resolves against the server's cwd) and `branch` the git ref the worker starts on; the default `HEAD` (or an empty string) means leave the pane's checkout alone. Branch names are case-sensitive: only the literal `HEAD` means that, `head` is an ordinary branch name. `task_get` shows both as `path:` (`-` when unset) and `branch:`. `description` holds everything the worker needs: repo path, files, situation, constraints. `goal` holds how to do it and the definition of done. Do not block — poll `task_get` or `board_brief` for status changes. When status is `review` or `done`, read `report` for the outcome.

## Worker flow

`task_claim(id)` → read `description` and `goal` → do the work → `task_update(note=...)` for progress → `diff_add(id, body=...)` → `task_update(report=..., status=review|done)` with the outcome. A move to `review` needs the diff (see the workflow above); `done` does not. Any agent may create a child task (`parent=<id>`); a reviewer uses this to file the fix task.

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
| reopen_count | Moves into `needs_fix` so far (0 on a new row, never reset); a refused fourth is recorded as 4, shown by `get` as `reopens: 4/3 (cap hit, stopped)` |
| report | Worker's findings/outcome |
| note | Free-form progress note |
| commit_ref | Commit or PR reference |
| lease_until | Lease expiry timestamp |
| created_at | Creation timestamp |
| updated_at | Last update timestamp |
| started_at | When work began |
| completed_at | When status reached done/dropped |

## Diffs

A diff is what a coder submitted for review: one row per submission, stored verbatim and append-only (no edit, no delete; it survives the task being dropped). The board never verifies the body. A plain diff (no `reviews`) is a patch. Only the newest patch of a task is reviewed, so record the complete change each time.

- `diff_add(id=T7, body=...)` — `agentboard diff add T7 --body-file PATH|- | --body TEXT`. Returns `D12 (T7 round 2)`; `round` is the task's reopen count + 1. A whitespace-only body, an unknown task and an epic id are refused. Logs one `diff` event on the task (`D12 round 2 184 lines`, never the diff itself).
- `diff_add(id=T7, body=..., reviews=D3)` — an evidence-only submission for a round whose tree did not change: `agentboard diff add T7 --evidence-only --reviews D3 --body-file PATH` (MCP: pass `reviews="D3"`; the argument itself means evidence-only). `D3` must be a patch recorded on T7 and must be the newest patch of T7; evidence naming a superseded patch, a diff on another row, or another evidence-only row is refused. The row is never a patch: `diff_list` marks it `D5 evidence-only -> D3`, and `diff_get` labels it as evidence. The body is what was run and its output; a body saying only that nothing changed is not admissible.
- `diff_get(id=D12, full=)` — `agentboard diff get D12 [--full]`: task, round, age and the body, cut with a `[truncated: N of M chars; ...]` marker unless `full`.
- `diff_list(id=T7|E3)` — `agentboard diff ls T7|E3`: newest first, one line per diff, `D12 round 2 3m 184 lines` (an epic's listing names the task: `D12 T7 round 2 ...`), or `(no diffs)`. Evidence-only rows appear too, marked.

Exit codes: every diff verb exits 0 on success and 1 on any refusal — a bad id, an unknown task or diff, a whitespace-only body, an unreadable `--body-file`, no body source, a task id where a diff id belongs, or an evidence target that is not the newest patch of the submitting row. Over MCP the same text arrives as a tool error.

`task_get` shows the newest patch of the task as one `diff:` line. Recording a diff moves no status; the only status rule that reads diffs is the gate on a move into `review` (workflow step 3). `task_wait`/`agentboard wait` accept `diff` as an event kind.

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
- `not_done` — set by the board, not by an agent: the row's fourth move into `needs_fix` was refused (see [The reopen cap](#the-reopen-cap)) and the row landed here instead, with the caller's body kept in an event; an open parent lands here too, one level only. Not claimable; the manager drops it or refiles (aliases `not-done`, `nd`). `get` shows `reopens: N/3`, and `(cap hit, stopped)` above 3.
- `review` — ready for a second pair of eyes: the reviewer, or a human for an API break or an irreversible operation. A move into it needs a diff recorded since the last claim. On a parentless row the board files its review row.
- `done` — verified complete
- `dropped` — abandoned

## Cost note

The full tool surface is ~1.8k tokens per request; `task_claim` returns the task so a follow-up `task_get` is never needed. Writes ack in one line on purpose.
## Board surface stamp

Schema generation 2 also versions the tool-surface rules: bump it when those rules
change incompatibly. SurrealDB checks `meta:schema.version` on every session before
DDL; SQLite refuses existing unstamped or mismatched files before DDL, naming the
file, and stamp-aware processes check on every open, even after initialization.
There is no automatic upgrade. An owner can explicitly upgrade a stopped
generation-1 SurrealDB board using `agentboard board upgrade --owner`; see the
[cutover order](docs/agent-install.md#owner-cutover-for-an-existing-surrealdb-board).
Seats report the refusal to the owner and never perform live maintenance. Refusals are CLI errors and MCP -32603 errors,
naming board and client stamps. A legacy binary can only print its legacy error.

The board stores the creating/opening build and last stamp-aware writer (version,
derived tool count, commit, backend): SurrealDB `meta:schema`, `meta:opened`,
`meta:writer`; SQLite `board_stamp`. Write transactions update the writer stamp;
reads preserve it. These records are inspectable through the database's read API.
A stamp-aware SQLite build rechecks on every operation: after a generation change,
its next operation is refused without a restart. A stamp-aware SurrealDB session
checks `meta:schema.version` when opened: an already-open session keeps its view
until restarted, while a new session is refused after a generation change.
A pre-stamp SQLite binary never consults the stamp and can keep writing; its writes
leave the last-writer record unchanged and cannot be attributed on the board.
Replace/retire pre-stamp binaries and stop their processes at cutover; restarting
a pre-stamp binary is not enough. Older SurrealDB binaries that already check the
schema version refuse incompatible generations through that existing gate.

### Per-write tool count

Generation-2 boards also record `surface_tools` on SurrealDB `meta:schema` and
SQLite `board_stamp`. Initialization, additive v2 backfill and the offline SurrealDB
upgrade derive it from the binary's router count. Normal writes never replace a
published count. Every guarded write transaction supplies its own compiled count
from `ident`; no CLI argument or MCP tool parameter accepts a seat-provided count.
A missing or mismatched count refuses before mutation, naming both counts and
requiring the seat binary to be reinstalled and its MCP server reloaded. The
comparison runs again for already-open clients. The friendly seat-start catalog
and commit check remains in `roles.md`.

This additive field does not change generation 2 or remove existing data. Readers
may ignore the field. Binaries predating this write guard still need owner
retirement, as with pre-stamp clients above: a new client-side guard cannot fence
an old process that connects directly to the database and never executes it.
Rolling back the binary leaves the additive field/data intact but also removes
that binary's enforcement; restart only matching guarded seats to restore it.
