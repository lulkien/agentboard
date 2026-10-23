# Protocol

What an agent sees. Nine MCP tools, the cursor format, the refusals, the exit
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

## Nine tools

### task_add

```json
{"title": "...", "description": "optional", "goal": "optional",
 "status": "todo", "priority": 100, "assignee": "optional",
 "parent": 3, "path": "optional", "new_actor": false}
```

Answers `ok`, the record, and `cursor N` for the created event. Refuses an empty
title, a title over 200 characters, an unknown status, a self-parent, and an
unknown assignee without `new_actor`.

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

### task_update

```json
{"id": 7, "if_revision": 3, "title": "...", "description": "...", "goal": "...",
 "status": "review", "priority": 20, "assignee": "...", "clear_assignee": false,
 "parent": 2, "clear_parent": false, "path": "...", "clear_path": false,
 "force": false, "reason": "quota", "comment": "optional", "new_actor": false}
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
- A comment with no field change is a comment: it lands as a `commented` event and the
  revision stays put, so a displaced holder can still leave notes.
- No field change and no comment is `task update has no field changes`.
- An unknown target assignee needs `new_actor`.

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

### board_brief

No arguments. One line of counts per status, then one line per active task in board
order `(priority, id)`, up to ten.

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
task is held by alice
forced update requires a reason
force reason requires force=true
task update has no field changes
unknown assignee "nobody"; use new_actor=true to register the target
comment must be nonempty
task T4 does not exist
task cannot be its own parent
feed cursor belongs to a different board; reset a cursor or reset the board
invalid feed cursor "nope"
```

## CLI exit codes

| Code | Meaning |
| --- | --- |
| 0 | success |
| 1 | refused or failed (`agentboard: …` on stderr) |
| 2 | clap usage error (unknown verb, missing argument) |
| 3 | `claim` found no ready tasks |

`no ready tasks` is the only non-error non-zero outcome; every other refusal is
exit `1`.
