# Protocol

What an agent sees. Eleven MCP tools, the cursor format, the refusals, the exit
codes. The store that implements this is described in `AGENTS.md`; running a team
is described in `skills/agentboard-team`.

## Transport

`agentboard serve`, or `agentboard` with no command: MCP over stdio, protocol
`2025-11-25`, tools capability only, no resources and no prompts. The process
opens the board once at start; if the board cannot be opened the process exits `1`
with one line on stderr.

Identity comes from the process environment (`AGENTBOARD_ACTOR`) or from
`agentboard serve --actor NAME`; the flag wins. A tool that mutates and has no
actor fails with `actor required: start the server with --actor or set
AGENTBOARD_ACTOR`.

## Ordering

Lists and claim use the same order: `(priority, id)` ascending, priority
an `i32` (lower runs first). Id-less claim walks the `todo` rows in that order and
skips tasks assigned to another actor; it never picks up a `review` or `blocked` row.

## Tasks

A task is a record: `id`, `title`, `description`, `goal`, `status`, `priority`,
`assignee`, `parent_id`, `path`, `revision`, `created_at`, `updated_at`. `revision`
starts at 1 and grows by one per accepted field change.

Statuses, in board order: `todo`, `in_progress`, `blocked`, `review`, `done`,
`canceled`. `done` and `canceled` are terminal; lists hide them unless asked.

`todo` rows can be claimed by id or by the id-less probe. `in_progress` and `review`
rows can only be claimed by id, and only by their assignee unless the row has none.
`blocked` is never claimable, and `done`/`canceled` are terminal: lists hide them unless
asked, and an explicit claim of one refuses by name.

## Eleven tools

### task_add

```json
{"title": "...", "description": "optional", "goal": "optional",
 "status": "todo", "priority": 100, "assignee": "optional",
 "parent": 3, "path": "optional", "new_actor": false}
```

Answers `ok`, the record, and `cursor N` for the created event. Refuses an empty
title, a title over 200 characters, an unknown status, a self-parent, and an
unknown assignee without `new_actor`. A direct parent must exist and be nonterminal.
Creating or newly reparenting under a `done` or `canceled` parent refuses; the
manager explicitly reopens the parent first, then retries. Parents never auto-reopen.

### task_list

```json
{"status": "optional", "assignee": "optional", "parent": 3,
 "path": "optional", "all": false, "limit": 20, "offset": 0}
```

One line per task, or the single line `no tasks` when the filter matches nothing, then
`total N cursor <cursor>`, then `more true` when another page follows. `limit` clamps to 1..200. An explicit `status` is honoured even when
it is terminal.

### task_get

```json
{"id": 7, "full": false}
```

The record with its revision. Compact reads collapse whitespace and cut titles at
72 and fields at 160 characters with `…`. `full: true` prints the title on its own
line and every field exactly as stored.

### task_epic

```json
{"id": 7}
```

An epic is any task with at least one direct child, derived from `parent_id` in
one read snapshot. CLI `agentboard epic ID` and this tool share exactly four lines
(the CLI appends its final newline):

```text
<compact task row>
todo <n>  in_progress <n>  blocked <n>  review <n>  done <n>  canceled <n>
done <done>/<total>
total <total> cursor <instance_id>:<seq>
```

Counts include every direct child, including terminal children, without paging;
grandchildren do not count. Only `done` contributes to the numerator; `canceled`
contributes to total. The row uses the usual compact title truncation. A leaf
refuses `task T7 is not an epic; an epic is a task with children`; a missing id
refuses `task T7 does not exist`. Reads append no events.

### task_update

```json
{"id": 7, "if_revision": 3, "title": "...", "description": "...", "goal": "...",
 "status": "review", "priority": 20, "assignee": "...", "clear_assignee": false,
 "parent": 2, "clear_parent": false, "path": "...", "clear_path": false,
 "release": false, "force": false, "reason": "quota", "comment": "optional",
 "new_actor": false}
```

Answers `ok`, the record and the event cursor. Rules, each with its own refusal:

- `if_revision` must equal the stored revision, or `stale task revision <given>; current
  revision is <stored>`.
- Changing the protected fields of a task held by another actor in `in_progress`,
  `blocked` or `review` needs `force` **and** a non-empty `reason`;
  `force` without a reason refuses, a reason without `force` refuses, and the
  reason is recorded in the event.
- `assignee` together with `clear_assignee` refuses, and likewise for `parent` and
  `path`: pick one.
- `release: true` puts a held row back in `todo` with no assignee, in one write: the
  revision moves, and one `updated` event carries `reason` and any `comment`. The
  holder may release its own row; another actor needs `force` with a reason, as for
  any other protected write. It cannot be combined with any field change —
  `status`, `assignee`, `clear_assignee`, `title`, `description`, `goal`,
  `priority`, `parent`, `clear_parent`, `path` or `clear_path` — which refuses with
  `choose release or a field change, not both`; only `force`, `reason` and `comment`
  ride along. A terminal row and a row without an owner refuse by name. The
  event kind is unchanged and no field is added, so an older binary still reads the
  row.
- A comment with no field change is a comment: it lands as a `commented` event and the
  revision stays put, so a displaced holder can still leave notes.
- No field change and no comment is `task update has no field changes`.
- An unknown target assignee needs `new_actor`.
- An explicit `done` or `canceled` status update checks direct children in the
  same write transaction. If any are neither `done` nor `canceled`, it refuses
  `epic T7 has 2 open children; close or drop them first`, substituting the id and
  open count. Force cannot bypass this rule. The refusal changes no task, roster,
  index or event. Leaves and epics with all children terminal may close. Other
  edits, comments and reopening remain allowed; statuses never cascade.
- A new parent attachment must name an existing nonterminal direct parent, even
  when the child is terminal. Force cannot bypass this rule. Refusal changes no
  task, roster, index or event. Unchanged parent links and clearing a parent stay
  allowed. The manager explicitly reopens a closed parent before retrying; no
  automatic reopening occurs.
- An update whose result is `done` or `canceled` cannot set, switch or clear
  `assignee`: `done task cannot change assignee; reopen it with an open status in
  the same update`. Force does not lift it, there is no holder to take over from.
  Handing a closed row to a seat is one write carrying an open `status` plus the
  new `assignee`. Titles, descriptions, goals and comments on a closed row stay
  editable, and closing a row while keeping its holder is fine.
- Moving a closed task to an open status also looks at its direct parent in that
  write. While the parent is closed it refuses `parent T5 is done; reopen it
  before reopening T7` and writes nothing, force or not. Reopen ancestors from the
  top down, then the task itself. `done` to `canceled` under a closed parent is
  allowed.

### task_claim

```json
{"id": 7}
```

With no id, claims the first eligible task in `(priority, id)` order and answers the
work order, or `no ready tasks` when nothing is eligible. With an id, claims that
task and refuses `held by NAME` when another actor holds it, `not claimable` when
its status cannot be claimed. Claiming a `todo` row assigned to you starts the work:
it becomes `in_progress`, the revision moves and a `claimed` event lands. Claiming a row
you already hold in `in_progress` or `review` changes nothing and appends no event.

### task_comment

```json
{"id": 7, "body": "..."}
```

Answers `ok` and the event cursor. The revision does not move. An empty comment
refuses.

### board_feed

```json
{"after": "instance:41", "kind": "commented", "recipient": "coder-2",
 "task": 7, "limit": 50, "full": false, "wait": 30}
```

Answers one line per event (`seq timestamp actor kind TASK [-> recipient] | body`),
then `cursor <cursor>`, then `more true` and/or `older true` when applicable.

- Cursor format is `<instance_id>:<seq>`, where seq is the last sequence the caller
  has seen. A bare sequence (`"0"`, `"41"`) means the board currently open, so
  `{"after": "0"}` replays history. A cursor from another board refuses
  `feed cursor belongs to a different board`.
- With no cursor, the page is the newest page *of matching events*, and `older` says
  whether more matching events sit before it.
- `wait` (seconds, clamped to 300) blocks until a matching event is written after the
  read starts, or the timeout expires. With no cursor it starts *now*: existing history
  is not replayed, so an empty page is the normal first answer. The answer is always a
  normal page with a valid cursor, and a waiter holds no read transaction while it
  sleeps.
- `full: true` prints event bodies untruncated.

### task_wait

```json
{"ids": [7], "status": "done,canceled", "epic": false, "timeout_s": 300, "assignee": true}
```

Blocks until every watched row is in one of `status` (a CSV, default `done,canceled`)
and answers the verdict as its first line, so a caller needs no error channel to tell
the two outcomes apart:

```text
matched: <n> of <m> watched in <statuses>
timeout: <n> of <m> watched in <statuses> after <ms>ms
```

A match caused by `assignee` appends `; handoff T<id> to <name>` to its first line,
one entry per handed-off row in id order, comma-separated.

Then one compact task row per watched row, sorted by id, in either outcome. In a
match, a row that was already wanted shows its newest earlier event as
`  last: <event>` — context, not a claim that this event caused the return; a row
that moved shows its events after the wait's watermark, one `  <event>` line each,
truncated as the feed truncates. At most 20 event lines are printed per row: the
oldest events after the watermark are kept and the remainder appears on one line as
`  … N more events`. A timeout prints no event lines at all. The final
line is `cursor <instance_id>:<seq>`, the high sequence at the moment the wait
returned, so the caller can follow on with `board_feed`.

- `ids` is required and each id must exist. With `epic: true` each id must have
  direct children and the watched set is those children, re-resolved on every poll,
  so a child filed mid-wait joins it.
- `status` parses with the same CSV rule as the CLI: trimmed, deduplicated, ordered
  as the board orders statuses. An empty list refuses `wait needs at least one
  wanted status`, never a vacuous match.
- `assignee` is `true` or an actor name; absent or `false` keeps the status-only
  predicate. It adds a second way to match: any one watched row (an epic's children
  are re-resolved on every poll) gets an `updated` event after the wait started whose
  recipient is the new assignee. `true` takes any such handoff; a name takes only a
  handoff to that actor, and is shape-checked, not looked up on the roster. A release
  clears the assignee, so it is no handoff. A handoff that landed before the wait
  started is not seen, so read the row before arming the wait.
- `timeout_s` defaults to `300` and clamps to `1..=300`. The CLI accepts only 1 to 300 seconds and defaults `--timeout` to 300; zero and values above 300 are refused.
- A timeout is **not** a JSON-RPC error: the result is ordinary text whose first
  line is `timeout:`. Refusals are `-32602` like every other tool.
- The wait re-queries the rows it watches, so it returns when they reach the wanted
  status, not when any event lands. It writes nothing and needs no actor.

### board_brief

No arguments. All epic rows first, each compact task row followed by `  done D/N`,
then up to ten unparented non-epic tasks in `blocked`, `in_progress` or `review`.
Each section sorts by `(priority, id)`. Nested and terminal epics remain visible
while they have children. The final line is the six global status counts, counting
every task once. There are no section headings; an empty board prints just counts.

### board_agents

No arguments. Roster lines `name last_seen <ms|never> last_task T<id|->`, newest
activity first. `last_seen` is history, never liveness.

## Refusals

Every refusal is one line with a stable prefix. Over MCP a refusal is a JSON-RPC error
(`-32602`, invalid params) whose message is the same text; a tool result is never
`isError: true`. The CLI prints `agentboard: <text>` and exits `1`. Samples:

```
actor required: pass --actor or set AGENTBOARD_ACTOR (names are board addresses)
unknown task status: "queued"
stale task revision 3; current revision is 4
task is held by alice; --force with a reason takes it, or --release --force with a reason returns it to todo
forced update requires a reason
force reason requires force=true
task update has no field changes
task T7 is done; nothing to release
task T7 has no holder to release
unknown assignee "nobody"; use new_actor=true to register the target
comment must be nonempty
task T4 does not exist
task T7 is not an epic; an epic is a task with children
epic T7 has 2 open children; close or drop them first
parent T5 is done; reopen it before adding children
parent T5 is canceled; reopen it before adding children
parent T5 is done; reopen it before reopening T7
done task cannot change assignee; reopen it with an open status in the same update
task cannot be its own parent
feed cursor belongs to a different board; reset a cursor or reset the board
invalid feed cursor "nope"
wait needs at least one wanted status
wait needs at least one task id
```

## CLI exit codes

| Code | Meaning |
| --- | --- |
| 0 | success |
| 1 | refused or failed (`agentboard: …` on stderr) |
| 2 | clap usage error (unknown verb, missing argument) |
| 3 | `claim` found no ready tasks, or `wait` hit its timeout |

`no ready tasks` and a `wait` timeout are the only non-error non-zero outcomes; every
other refusal is exit `1`.
