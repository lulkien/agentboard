# agentboard

One board shared by a team of CLI agents. A manager adds work, workers claim it,
everyone reads the same event feed. The board is a single LMDB directory: no
background daemon or database server. The default CLI and MCP transport stay local;
an optional dashboard feature serves a read-only page over HTTP.

10 MCP tools, 14 default CLI verbs plus the optional dashboard, one store behind both.

## Why it exists

An agent that writes its plan to `TODO.md` knows nothing about the other agent
editing the same file. A board gives every agent one place to claim work, comment
on it, hand it over and see what else is moving, with an audit trail that survives
the process that wrote it.

## Build and install

```bash
cargo build --release          # target/release/agentboard
cargo install --path . --locked   # ~/.cargo/bin/agentboard
cargo build --release --features dashboard # also enable the optional dashboard
```

Rust 1.89 or newer, edition 2024. The only C dependency is the one `heed` builds
with `cc`.

## The board

- Directory: `--db`, else `$AGENTBOARD_DB`, else
  `$XDG_STATE_HOME/agentboard/lmdb` (`~/.local/state/agentboard/lmdb` by default).
- Files inside: `data.mdb`, `lock.mdb`, `lifetime.lock`. Anything else makes the
  directory foreign, and the store refuses it by name rather than guessing.
- Map size: `AGENTBOARD_MAP_SIZE_MB`, default 1024. There is no growth in place; a
  bigger board means a bigger map before it is created.
- `agentboard reset --yes` discards the board. It refuses while another process
  holds it, and it refuses to delete a directory with unknown files in it.

No two boards share state and no board needs a process to stay alive between
calls.

## Actor

Every mutation is attributed to a name: `--actor` or `$AGENTBOARD_ACTOR`. A
missing or empty name is a refusal that says how to fix it. Names are board
addresses on a trusted team, not authentication.

A name joins the roster on its first accepted mutation, either as the caller or as
a target assignee created with `--new-actor`. Without that flag an unknown
assignee is refused, so a typo cannot invent a person. `agentboard agents` lists
names with their last activity and last task; that is history, never liveness.

One live process per name. Two agents answering to one name will refuse each
other's writes as `held by …`.

## Verbs

```
agentboard add "title" [--description TEXT] [--goal TEXT] [--status S]
                      [--priority N] [--assignee NAME] [--parent ID]
                      [--path PATH] [--new-actor]
agentboard ls [--status S] [--assignee NAME] [--parent ID] [--path PATH]
              [--all] [--limit N] [--offset N]
agentboard get ID [--full]
agentboard epic ID
agentboard update ID --if-revision N
              [--title|--description|--goal|--status|--priority|--assignee|
               --clear-assignee|--parent|--clear-parent|--path|--clear-path]
              [--release] [--force --reason R] [--comment TEXT] [--new-actor]
agentboard claim [ID]
agentboard comment ID (--body TEXT | --body-file PATH|-)
agentboard feed [--after CURSOR] [--kind K] [--recipient NAME] [--task ID]
                [--limit N] [--full] [--wait SECONDS]
agentboard wait ID... [--status CSV] [--assignee [NAME]] [--epic] [--timeout SECONDS]
agentboard brief
agentboard agents
agentboard path
agentboard reset --yes
agentboard serve
agentboard dashboard [IP:PORT] [--db PATH]
```

Statuses: `todo`, `planning`, `implementing`, `blocked`, `reviewing`, `done`, `canceled`
(the old input names `in_progress` and `review` are still accepted).

- `ls` hides `done` and `canceled` unless `--all` is passed or a status is named.
- Ordering is `(priority, id)` with the lowest number first, so `claim` with no id
  takes the highest-priority ready task and skips work assigned to someone else.
- `get` truncates long text with `…`; `--full` prints stored text untouched, title
  on its own line, whitespace and newlines preserved.
- `update` requires `--if-revision`, the revision the caller last read. A stale
  value refuses and prints both numbers. Changing another actor's task requires
  `--force` and a non-empty `--reason`; force without a reason refuses, and a
  reason without force refuses.
- `update --release` sends a held row back to `todo` with no owner, in one write:
  one revision bump and one `updated` event carrying `--reason` and `--comment`.
  No field flag rides along: `--status`, `--assignee`, `--clear-assignee`,
  `--title`, `--description`, `--goal`, `--priority`, `--parent`, `--clear-parent`,
  `--path` or `--clear-path` beside it refuses; pick one or the other.
- `comment` appends an event and never moves the revision.
- `epic` shows a task's direct-child status counts and done/total progress. Epics
  are derived from parent links; any task with children is an epic, including a
  nested or closed task. Closing an epic as `done` or `canceled` requires every
  direct child to be `done` or `canceled`, even with force. Creating or reparenting
  a child under a terminal parent refuses. The manager explicitly reopens the
  parent first, then retries; parents never auto-reopen. A closed child likewise
  waits for its closed parent before it can reopen, and a closed task takes a new
  owner only in the update that reopens it.
- `brief` shows all epic rows with progress first, then up to ten unparented active
  tasks, then global status counts. Each section uses `(priority, id)` order.
- `feed` is the bus. It answers a page of events plus the cursor to pass as
  `--after`. With no cursor it returns the newest page of *matching* events and
  says `older true` when more matching events sit before it. `--after 0` replays
  from the first event. `--wait N` blocks up to N seconds (accepted range 1 to 300; omit for a nonblocking read) for a
  matching event and returns the cursor it stopped at. `--full` prints whole
  comment and update bodies.
- `wait` blocks until the rows you name reach a status, and answers a verdict
  instead of a page. `--status` is a CSV of wanted statuses, default
  `done,canceled`; every watched row must be in one of them, so it is an all-rows
  predicate and `wait 27 --status reviewing` is the everyday case. `--epic` watches
  the direct children of each id and re-reads them on every poll, so a child filed
  mid-wait joins the set. `--assignee` also returns when any one watched row is
  handed to a new owner after the wait starts, even with its status unchanged, which
  is how an analyst-to-coder reassignment reaches the manager; `--assignee NAME`
  accepts only a handoff to that actor, and a release is never a handoff.
  `--timeout` accepts 1 to 300 seconds and defaults to 300.
  Values outside that range are rejected before waiting to bound manager waits. A match
  exits `0` and prints `matched: …`, a timeout exits `3` and still prints each row
  under `timeout: …`. A row already wanted when the wait began shows its newest
  earlier event as `last:`; a row that moved shows the events that moved it, at
  most 20 lines then `… N more events`. The last line is
  `[ATTENTION] wait task is done, please check result` on a match and
  `[ATTENTION] wait timed out, nothing matched; re-arm or check the board` on a
  timeout; a refusal prints neither. It appends no event and needs no actor.

Exit codes: `0` ok, `1` refused or failed, `3` no ready tasks or a `wait`
timeout. A refusal prints one line naming the field, the value and what to do:

```
agentboard: stale task revision 3; current revision is 4
agentboard: task is held by alice; --force with a reason takes it, or --release --force with a reason returns it to todo
agentboard: forced update requires a reason
agentboard: unknown assignee "nobody"; use new_actor=true to register the target
agentboard: task T7 has no holder to release
```

## Optional dashboard

The `dashboard` subcommand is available when the binary is built with the
`dashboard` Cargo feature:

```bash
cargo build --release --features dashboard
agentboard dashboard [ip:port] [--db PATH]
```

It defaults to `127.0.0.1:8080`; the global `--db` option works before or after
the subcommand. The page is read-only and has no authentication. It binds only to
loopback by default; bind a non-loopback address only deliberately. The
`agentboard serve` command remains MCP over stdio and its 10-tool surface is
unchanged. The dashboard routes are `/` and `/epics` (epic overview), `/tasks`
(filtered and paged task list), `/tasks/ID` (task details and recent activity),
and `/feed` (board event history). `/assets/dashboard.css` and
`/assets/theme.js` are embedded dashboard assets. The theme button cycles through
light, dark and system appearance, saving a manual choice in local storage.
Every route is GET-only; other methods receive `405 Method Not Allowed`. The feed
shows the newest 20 matching events and uses its opaque `before` cursor to page
toward older events while retaining task and event-kind filters.

## Quota take-over

When a holder runs out of quota, the manager does not need a special verb: it
reassigns the task with the current revision, `--force` and a reason.

```bash
agentboard get 12           # revision 3, held by coder-1
agentboard update 12 --if-revision 3 --assignee coder-2 --force --reason quota
```

When nobody should take the row over, the same verb sends it back to the queue:

```bash
agentboard update 12 --if-revision 3 --release --force --reason "coder-1 is gone"
```

That one call makes the row `todo`, clears its owner and records the reason, so the
next `claim` can pick it up. The holder may release its own row without `--force`;
any other name needs the force and the reason, exactly as above, so a live holder is
never stripped silently. A `done` or `canceled` row answers `nothing to release`, and
a row with no owner answers `no holder to release`.

The displaced holder's field writes now refuse; its comments still land. The
replacement resumes from the artifact the row names, and the event trail shows the
handoff with its recipient. Stop or isolate the displaced process before the
replacement edits the same working tree.

## MCP

`agentboard serve` speaks MCP on stdio, and `agentboard` with no command does the
same. The server needs `$AGENTBOARD_ACTOR` or `--actor`; the client cannot pass
one.

| Tool | Answers |
| --- | --- |
| `task_add` | `ok`, the record, its event cursor |
| `task_list` | one line per task, `total N`, cursor, `more true` |
| `task_get` | the record; `full=true` for stored text |
| `task_epic` | compact epic row, direct-child counts, done/total, total and cursor |
| `task_update` | `ok`, the record, its cursor; refuses stale, held, missing reason |
| `task_claim` | the work order, or `no ready tasks` |
| `task_comment` | `ok`, cursor |
| `board_feed` | events, cursor, `more`/`older`; `wait` seconds; `full=true` |
| `board_brief` | epic progress, unparented active tasks, then counts per status |
| `board_agents` | roster lines, newest activity first |

Both adapters call the same store functions, so a refusal reads the same in both
and no rule lives in only one of them. `PROTOCOL.md` has the exact shapes.

## Where the rest lives

- `PROTOCOL.md` — the wire surface.
- `AGENTS.md` — changing this tree: gates, layout, traps.
- `skills/agentboard-team` — running a team: the board, launcher, wake, roles,
  workflow.
- `skills/agentboard-manager`, `skills/agentboard-analyst`, `skills/agentboard-coder`,
  `skills/agentboard-reviewer` — one entry skill per seat; Grok's manager recipe is
  `skills/agentboard-manager/grok.md`.
