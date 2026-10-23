# agentboard

One board shared by a team of CLI agents. A manager adds work, workers claim it,
everyone reads the same event feed. The board is a single LMDB directory: no
daemon, no socket, no network, no database server. Every CLI call and every MCP
server opens that directory and the file lock keeps them honest.

9 MCP tools, 12 CLI verbs, one store behind both.

## Why it exists

An agent that writes its plan to `TODO.md` knows nothing about the other agent
editing the same file. A board gives every agent one place to claim work, comment
on it, hand it over and see what else is moving, with an audit trail that survives
the process that wrote it.

## Build and install

```bash
cargo build --release          # target/release/agentboard
cargo install --path . --locked   # ~/.cargo/bin/agentboard
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
agentboard update ID --if-revision N
              [--title|--description|--goal|--status|--priority|--assignee|
               --clear-assignee|--parent|--clear-parent|--path|--clear-path]
              [--force --reason R] [--comment TEXT] [--new-actor]
agentboard claim [ID]
agentboard comment ID (--body TEXT | --body-file PATH|-)
agentboard feed [--after CURSOR] [--kind K] [--recipient NAME] [--task ID]
                [--limit N] [--full] [--wait SECONDS]
agentboard brief
agentboard agents
agentboard path
agentboard reset --yes
agentboard serve
```

Statuses: `todo`, `in_progress`, `blocked`, `review`, `done`, `canceled`.

- `ls` hides `done` and `canceled` unless `--all` is passed or a status is named.
- Ordering is `(priority, id)` with the lowest number first, so `claim` with no id
  takes the highest-priority ready task and skips work assigned to someone else.
- `get` truncates long text with `…`; `--full` prints stored text untouched, title
  on its own line, whitespace and newlines preserved.
- `update` requires `--if-revision`, the revision the caller last read. A stale
  value refuses and prints both numbers. Changing another actor's task requires
  `--force` and a non-empty `--reason`; force without a reason refuses, and a
  reason without force refuses.
- `comment` appends an event and never moves the revision.
- `feed` is the bus. It answers a page of events plus the cursor to pass as
  `--after`. With no cursor it returns the newest page of *matching* events and
  says `older true` when more matching events sit before it. `--after 0` replays
  from the first event. `--wait N` blocks up to N seconds (300 maximum) for a
  matching event and returns the cursor it stopped at. `--full` prints whole
  comment and update bodies.

Exit codes: `0` ok, `1` refused or failed, `3` no ready tasks. A refusal prints
one line naming the field, the value and what to do:

```
agentboard: stale task revision 3; current revision is 4
agentboard: task is held by alice
agentboard: forced update requires a reason
agentboard: unknown assignee "nobody"; use new_actor=true to register the target
```

## Quota take-over

When a holder runs out of quota, the manager does not need a special verb: it
reassigns the task with the current revision, `--force` and a reason.

```bash
agentboard get 12           # revision 3, held by coder-1
agentboard update 12 --if-revision 3 --assignee coder-2 --force --reason quota
```

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
| `task_update` | `ok`, the record, its cursor; refuses stale, held, missing reason |
| `task_claim` | the work order, or `no ready tasks` |
| `task_comment` | `ok`, cursor |
| `board_feed` | events, cursor, `more`/`older`; `wait` seconds; `full=true` |
| `board_brief` | counts per status plus the oldest active tasks |
| `board_agents` | roster lines, newest activity first |

Both adapters call the same store functions, so a refusal reads the same in both
and no rule lives in only one of them. `PROTOCOL.md` has the exact shapes.

## Where the rest lives

- `PROTOCOL.md` — the wire surface.
- `AGENTS.md` — changing this tree: gates, layout, traps.
- `skills/agentboard-manager` — running a team: launcher, wake, roles, workflow.
- `skills/manager` — seat entry points.
