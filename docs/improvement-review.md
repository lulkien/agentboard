# Improvement review

Written 2026-11-11 against `agentboard 0.4.0` at commit `27ffa1f`. This is a review
file, like `manager-loop-gaps.md`: it records what could be better, with the
evidence and the smallest fix. Nothing here is filed as a row. Findings marked
**Done** are implemented; the commit is named under each one.

## Summary

The project is in good shape. The design is small and consistent, the rules live in
one store behind both adapters, and the full gate passes. The findings below are
mostly about what happens as a board grows and about a few edges of the manager
loop. Two are real bugs that I reproduced.

Do these first:

| # | Finding | Kind | Size |
| --- | --- | --- | --- |
| [1](#1-a-comment-only-update-skips-actor-validation) | A comment-only `update` writes an invalid actor name to the roster | bug, reproduced | **Done** `8744808` |
| [2](#2-wait---assignee-can-miss-a-handoff-between-re-arms) | `wait --assignee` can miss a handoff that lands between two re-arms | loop gap | **Done** `f61ab67` |
| [5](#5-wait---epic-scans-every-task-40-times-a-second) | `wait --epic` scans and decodes every task 40 times a second | performance, measured | a few lines in `wait.rs` |
| [9](#9-a-full-map-has-no-way-out-except-reset) | A full map has no remedy except `reset`, and events grow faster than tasks | data lifecycle | decided, see D1 to D3 |
| [16](#16-protocolmd-has-drifted-from-the-code) | `PROTOCOL.md` has five stale statements the doc gates do not catch | docs | **Done** `567aed4`, see the note under it |

## How this was checked

- Read every non-test source file under `src/`, the dashboard, the gate script and
  the three core pages.
- Ran `bash scripts/gate.sh` for a baseline: `GATES OK: 16 gates passed`.
- Ran the release binary on a scratch board (`AGENTBOARD_DB` in a temp directory,
  256 MiB map) with about 1,750 tasks, 1,500 of them carrying a 4 KB description,
  and a scratch daemon on a loopback port. No live board was touched.

Timings are from one machine and one run. Read them as ratios, not as targets.

## What is already good

These are worth keeping as they are:

- **One rule, one place.** Ownership, revision and force rules are `Task::apply`;
  `release` and `claim` are built as patches on top of it and add no second path
  (`src/domain.rs:352`, `src/store/tasks.rs:209`).
- **Atomic writes.** Task, indexes, roster, event and metadata commit in one LMDB
  transaction, and the indexes are checked on every replace
  (`src/store/tasks.rs:637`).
- **Careful open and reset.** The `(dev, inode)` check across the lock downgrade,
  the symlink refusals and the foreign-directory refusal are all deliberate and
  tested (`src/store/open.rs:277`).
- **The daemon guard.** Host, Origin and loopback-peer checks run before every
  route, the CSP is strict, and the dashboard builds its DOM with `textContent` only.
- **The gate.** Sixteen offline gates, scratch boards only, with two doc-drift
  checks. Few projects of this size have that.

---

## Correctness

### 1. A comment-only update skips actor validation

**Status: done** in `8744808`; the comment-only branch also refuses discarded
`--force`, `--reason` and `--new-actor` options. `tests/cli.sh` covers both.

`Board::update` has an early branch for "a comment with no field change". That
branch calls `touch_actor` and appends the event without ever calling
`validate_actor` (`src/store/tasks.rs:112-134`). Every other write validates the
actor through `Task::apply`, `claim` or `comment`.

Reproduced on a scratch board:

```
$ agentboard --actor " bad<TAB>name" update 2 --if-revision 99 --comment hi
ok
...
exit=0
$ agentboard agents
xxxx…(100 characters)…  last_seen 1794351918180  last_task T2
 bad	name            last_seen 1794351918174  last_task T2
```

A name with a leading space and a control character, and a 100-character name
(the limit is 64), both joined the roster and both are now event authors. The same
path is reachable over MCP through `task_update` with only `comment` set, where
the actor comes from the URL segment.

**Fix.** Call `validate_actor` inside `touch_actor`. It is the one function every
write passes through, so the check then exists once and no future branch can skip
it.

**Test.** `tests/cli.sh`: a comment-only update under an invalid actor exits 1 and
`agents` does not list the name. Mutation-check by removing the call.

**Also note.** The same branch ignores `--if-revision`, `--force`, `--reason` and
`--new-actor` without saying so. Ignoring the revision is intended (a displaced
holder can still leave notes). Silently dropping `--force --reason` is the kind of
discarded argument the release path refuses by name; it is worth refusing here too.

### 2. `wait --assignee` can miss a handoff between re-arms

**Status: done** in `f61ab67`. `wait --after CURSOR` passes the watermark through;
the README, the manager skill and `workflow.md` re-arm with it.

The assignee branch of a wait is decided from `updated` events after a watermark.
The store accepts that watermark (`WaitSpec.after`, `src/store/wait.rs:29`), and
the verdict prints `cursor …` "to follow on with" (`src/render.rs:247`). But the
CLI has no flag to pass it back: `main.rs` always sends `after: None`
(`src/main.rs:425`), so every wait starts from "now".

A manager wait is capped at 300 s and re-armed. If the analyst hands the row to the
coder after one wait times out and before the next one starts, the status does not
change, the handoff event is already below the new watermark, and the new wait
never matches on it. The manager then sits for the full timeout, repeatedly, on a
row that already moved.

**Fix.** Add `wait --after CURSOR` and pass it through. The store side and its
tests already exist. Update the manager skill to re-arm with the cursor the
previous wait printed.

**Test.** `tests/cli.sh`: read a cursor, hand the row off, then
`wait ID --assignee --after CURSOR` matches at once; without `--after` it times out.

### 3. A wildcard bind cannot be reached from another host

**Status: done** in `8add61a`, as decision D6: `0.0.0.0` and `[::]` are refused at
startup, with or without `--allow-non-loopback`.

`agentboard daemon 0.0.0.0:PORT --allow-non-loopback` starts, but the Host guard
accepts only the literal bound address, `localhost` or `127.0.0.1`
(`src/daemon.rs:271`). A browser on another machine sends the server's LAN address:

```
Host: 192.168.1.50:18765  -> 421
Host: 0.0.0.0:18765       -> 200
Host: localhost:18765     -> 200
```

So a wildcard bind serves only the clients that loopback already served. Binding a
specific LAN address works.

**Fix.** Either refuse an unspecified address at startup with a message that says
"bind a specific address", or document it in `PROTOCOL.md`. Refusing is smaller and
keeps the guard strict. Do not relax the Host check for the wildcard case; that
would remove the DNS-rebinding protection.

### 4. Every MCP failure is "invalid params"

**Status: done** in T303: store errors stay typed to the adapter, which maps refusals to `-32602` and faults to `-32603`.

`server.rs` turns every store error into a string and then into
`ErrorData::invalid_params` (`src/server.rs:86`). That is right for a refusal, and
`PROTOCOL.md` says so. It is wrong for an I/O error, a full map, a corrupt record
or a panicked blocking task, which are server faults (`-32603`) and which a client
may reasonably retry or escalate differently.

The cause is that errors are stringified at the first `map_err`. The CLI has the
same shape: `run` returns `Result<_, String>`.

**Fix.** Keep `store::Error` until the adapter boundary and add one
`fn is_refusal(&self) -> bool` on it. The MCP adapter maps a refusal to `-32602`
and the rest to `-32603`; the CLI keeps exit 1 for both. This also removes most of
the `.map_err(|e| e.to_string())` calls (there are about forty).

---

## Performance and scaling

Nothing here is slow on today's boards. Each item grows linearly with the number
of tasks or events, and the board never shrinks, so they are worth fixing before
they are felt.

### 5. `wait --epic` scans every task 40 times a second

**Status: done**: wait polls the event sequence each tick and re-reads the rows only when it moved; handoff and event reads seek from the watermark.

`watched_ids` resolves an epic's children by iterating the whole `tasks` database
and JSON-decoding every record, descriptions and reports included
(`src/store/wait.rs:231`). `wait_tasks` calls it on every poll, and the poll
interval is 25 ms.

Measured over a 10-second wait on 1,500 tasks of 4 KB each:

| Command | CPU used in 10 s |
| --- | --- |
| `wait 1 --epic --timeout 10` | 2.36 s (about 24% of a core) |
| `wait 2 --timeout 10` | 0.009 s |
| `feed --wait 10` | 0.006 s |

A manager parks on `wait --epic` for 300 s at a time, for hours. The cost scales
with the whole board, including closed tasks from old epics.

**Fix, no format change.** Every change to a task appends an event, so the watched
set and its statuses cannot change unless `next_event_seq` moves. Remember the
sequence from the last poll, read only the metadata on each tick, and re-run the
full poll only when it differs. An idle wait then costs the same as `feed --wait`.
Keep the last poll's rows so a timeout can still print them.

**Fix, with a format change.** A `tasks_by_parent` index makes the scan itself
cheap. See [finding 8](#8-many-reads-decode-the-whole-task-table).

The same loop has a smaller cost: `handoff` and `wait_events` iterate a task's
events from its first event and skip up to the watermark
(`src/store/wait.rs:252`, `:291`). Seeking with a range that starts at
`event_by_task(id, since + 1)` removes that.

### 6. `/api/epics` runs one full scan per epic

**Status: done**: `/api/epics` uses one snapshot and one task-table walk, then
reads each epic by key. The daemon shape checks and store walk counter pass.

`epics_json` calls `brief_for_project` (two task scans) and then `board.list` with
a `parent` filter once per epic (`src/daemon/api.rs:135`). With no status or
assignee, `list` reads every task id and decodes every task
(`src/store/list.rs:265`). The page is therefore O(epics × tasks).

Measured on 61 epics and about 1,750 tasks:

| Endpoint | Time |
| --- | --- |
| `/api/epics` | 230 ms |
| `/api/board` | 27 ms |
| `/api/tasks/2` | 32 ms |
| `/api/tasks` | 12 ms |

With **Live update** on, the dashboard reloads this page whenever any event lands.

Each of those `list` calls also opens its own read transaction, so the page is
assembled from up to N+1 different snapshots.

**Fix.** Add one store read model, like `overview`, that walks the tasks once in
one transaction and returns each epic with its counts and its first four children.
The API handler then builds JSON from it and does no further reads.

### 7. A filtered feed reads the whole event log

**Status: done**: task-filtered pages walk `events_by_task` and apply
remaining filters only to that task's events.

`newest_matching` walks from the newest sequence down to 1 with a point read and a
JSON decode per event, and when the page fills, `any_match_at_or_below` keeps
walking to answer `older` (`src/store/feed.rs:90-137`). For a filter that matches
few events, such as `--task ID` or `--recipient NAME`, that is the entire log.

`events_by_task` already indexes events by task, and `wait.rs` uses it. The feed
does not: `feed --task 2` took 23 ms against 3 ms for the unfiltered feed on a log
of only 1,700 events. The dashboard task page calls this for every detail view
(`src/daemon/api.rs:214`).

**Fix.** When `filter.task` is set, walk `events_by_task` with a reverse prefix
iterator and apply the remaining filters to those events only. No new index is
needed. The `recipient` filter stays a scan; it is used rarely enough to leave.

### 8. Many reads decode the whole task table

These all iterate `tasks` and decode every full record:

| Caller | Where | When it runs |
| --- | --- | --- |
| `child_counts` | `src/store/list.rs:98` | inside the write transaction of every update to `done` or `canceled` |
| `epic` | `src/store/list.rs:117` | `task_epic`, each dashboard detail page |
| `brief_for_project` | `src/store/list.rs:205` | twice per `board_brief` |
| `overview` | `src/store/overview.rs:41` | each dashboard board page |
| `status_counts` | `src/store/list.rs:80` | each dashboard task list, after `list_newest` already scanned once |
| `delete_project` | `src/store/projects.rs:19` | inside a write transaction |
| `watched_ids` | `src/store/wait.rs:231` | finding 5 |

At 1,750 tasks each is 13 to 30 ms. They become noticeable at around ten times
that, and the first row holds the single writer lock while it scans.

Two fixes, in order of cost:

1. **Decode less.** For a scan that needs only `status`, `parent_id`, `project_id`
   and `assignee`, deserialize into a small borrowed struct with just those fields.
   serde skips the rest without allocating the description and report strings. No
   format change, and it applies to every row of the table above.
2. **Index parents.** A `tasks_by_parent` database keyed `(parent_id, task_id)`
   makes `child_counts`, `epic`, `watched_ids` and `list --parent` proportional to
   the number of children. This is a ninth named database, so it is a board format
   change and a migration step. Decided in D4: not yet.

I would do the first now and the second only when a board actually gets large.

**Status: done**: the seven scans use a borrowed `TaskHead` decode, retaining full
decodes only for rows returned by read models. The store proof covers long text,
escaped assignees, counts, epic rollups, overview and newest-page rows.

---

## Data lifecycle

### 9. A full map has no way out except reset

The map size is fixed when the board is created. `open` refuses a different
`AGENTBOARD_MAP_SIZE_MB` (`MapSizeMismatch`), and when the map fills the error
text offers exactly one remedy:

> LMDB map full; set AGENTBOARD_MAP_SIZE_MB for a fresh board or reset this
> disposable board with a larger size

`reset` discards every task and the whole audit trail. For a board that the
README describes as "an audit trail that survives the process that wrote it",
that is a hard edge.

Three things make the map fill sooner than the task count suggests:

- **An `updated` event stores the old and the new value of every changed field**
  (`src/store/tasks.rs:672`). Two hundred small edits to one task's 4 KB
  description grew `data.mdb` by 1,612 KiB, about 8 KiB per edit, while the task
  itself stayed at 4 KB.
- **A `created` event embeds the whole task**, description and goal included
  (`src/store/tasks.rs:95`), so every task is stored twice from birth.
- **No text field has a size limit.** Titles are capped at 200 characters;
  descriptions, goals, reports and comments are not. One mistaken
  `--coder-report-file` pointing at a build log is accepted.

At the default 1 GiB this is far away for normal use. It is still a failure with
no recovery path, and it arrives without warning.

Options, smallest first:

1. **Warn early.** `brief` and `/api/health` report map usage (LMDB exposes the
   last used page), so a manager sees 80% long before a write fails.
2. **Cap text sizes** in `domain.rs`, with a refusal that names the field and the
   limit. This is a behaviour change, so pick generous limits.
3. **Stop duplicating text into events.** Record `{"description": {"old_chars": N,
   "new_chars": M}}` for long fields, the way reports already do
   (`report_changes`, `src/store/tasks.rs:710`). Title, status, assignee and
   priority keep their full old and new values. This loses the old description
   text from history; decided in D1.
4. **Allow growth.** LMDB can reopen an environment with a larger map. The cost is
   coordinating other attached processes, which is why `AGENTS.md` records "no
   map growth in place". A narrower version fits the existing model: a `grow`
   verb that, like `reset`, requires the exclusive lifetime lock, then rewrites
   the stored `map_size_mb`.
5. **Export.** See the next finding.

Options 1 and 2 are cheap and safe. Options 3 and 4 are decided in D1 and D3.

### 10. There is no export or backup verb

The only supported way to copy a board is to stop every process and copy the
directory. There is no way to get the data out as text, to move a board to a
larger map, or to archive closed epics.

**Fix.** An `agentboard export` that writes tasks, projects, agents and events as
JSON lines from one read snapshot. It needs no lock beyond a read transaction and
no format change, and it makes option 4 above less urgent: export, reset with a
larger map, import. Import is the larger half and can wait until it is needed.

### 11. The map size must be repeated on every open

A board created with `AGENTBOARD_MAP_SIZE_MB=4096` refuses to open in any shell
or seat card that does not set the same variable. The size is already stored in
the metadata envelope, so the variable carries no information after creation.

**Fix.** On an existing board, read the size from the metadata and use the
variable only when it is set and disagrees (keep the mismatch refusal for that
case). Seat cards then never need the variable.

**Status: done in T307.** Existing boards use their stored map size when the
environment variable is unset; an explicitly mismatched value still refuses.

---

## Code structure

### 12. Argument-conflict rules live in both adapters

**Status: done** in T303: one store update entry point handles release conflicts, while shared patch helpers cover empty and set/clear rules.

`AGENTS.md` says to put behaviour in the store "so a rule exists once". These
rules are written twice, once per adapter, with slightly different wording:

- `assignee` with `clear_assignee`, `parent` with `clear_parent`, `project` with
  `clear_project` (`src/main.rs:251-259`, `src/server.rs:449-457`).
- `release` beside any field flag, as a twelve-term boolean expression
  (`src/main.rs:260-284`, `src/server.rs:458-479`).

Adding a field means editing both lists, and `patch_is_empty`
(`src/store/tasks.rs:735`) is a third hand-kept list of the same fields.

**Fix.** Give the store one update entry point that takes `release: bool` beside
the patch and refuses the combination itself. `TaskPatch` already derives
`Default` and `PartialEq`, so `patch_is_empty` becomes a comparison with
`TaskPatch::default()` once `replace_reports` is set aside. The "pick one"
pairs disappear from the adapters if the adapter-level argument is modelled as
one three-state value before it reaches the patch.

### 13. Small duplications

**Status: done in T311.** The clock helper and LMDB name list are shared, and
the MCP URL uses `routes::parse`; the optional `run` extraction was not needed.

- `now_ms` exists three times: `src/domain.rs:9`, `src/main.rs:562`,
  `src/server.rs:108`. The server copy is a separate implementation.
- `project_param` in `src/daemon.rs:180` is a second query parser beside
  `routes::parse`, which `AGENTS.md` calls "the one strict query validator". It
  ignores unknown keys and does not percent-decode, so `/mcp/x?foo=1` is accepted
  where `/api/board?foo=1` is a 400.
- The database list is spelled out four times in `open.rs` (struct, create, open,
  constructor). A small array of names would keep them together.
- `run` in `src/main.rs` is one match of about 390 lines. One function per verb
  would make the file match its description as "thin".

None of these is a bug. They are where the next drift will come from.

### 14. Refusal text mixes CLI and MCP vocabulary

**Status: done in T311.** Shared refusal advice names both adapter spellings or
uses adapter-neutral terms.

The store renders one message for both adapters, so each adapter sometimes shows
the other's syntax:

- `task is held by alice; --force with a reason takes it, or --release --force …`
  is shown to MCP callers, whose arguments are `force` and `release`.
- `unknown assignee "nobody"; use new_actor=true to register the target` is shown
  to CLI callers, whose flag is `--new-actor`.

An agent reads these literally. Either phrase them neutrally ("force with a
reason") or let the adapter supply the argument spelling.

---

## MCP surface

### 15. Tool metadata is thinner than it needs to be

**Status: done** in T304.

`tools/list` returns no annotations on any of the twelve tools, and most
arguments have no description:

```
12 tools; annotations on: []
```

- **Annotations.** `task_list`, `task_get`, `task_epic`, `board_feed`,
  `board_brief` and `board_agents` are read-only; the two deletes are destructive.
  Clients use `readOnlyHint` and `destructiveHint` to decide what to run without
  asking and what to run in parallel. Check how rmcp 3.5 spells these on `#[tool]`
  before adding them.
- **Argument descriptions.** `priority` does not say that lower runs first.
  `after` does not say the cursor format or that `"0"` replays history. `kind`
  does not list its four values. An agent that guesses `priority: 1000` for
  "urgent" files the task last. One short line each costs a few tokens once per
  session.
- **Cursor shape.** `task_add`, `task_update` and `task_comment` answer
  `cursor 41` (a bare sequence), while `task_list`, `task_epic` and `board_feed`
  answer `cursor <instance>:41`. Both parse, but the bare form loses the
  wrong-board check. Print the encoded form everywhere.
- **Project tools are lopsided.** MCP can delete a project but cannot add, list or
  update one. The two delete tools also need no actor and write no event, so they
  are the only writes with no audit trail. If seats are meant to manage projects,
  expose `list` at least; if not, the deletes may not belong on the seat surface
  either.

---

## Documentation

### 16. `PROTOCOL.md` has drifted from the code

**Status: done** in `567aed4`; `tests/doc-names.py` now checks that the task field
list equals `Task`'s serde fields.

The two doc gates check tool names, CLI names and the published tool count. They
did not catch these:

| Line | Says | Actual |
| --- | --- | --- |
| 28 | the daemon "serves the same ten tools" | twelve |
| 71 | a task has `path`, `created_at`, `updated_at` | `project_id`, `created_at_ms`, `updated_at_ms`; `reporter` is missing |
| 169 | "likewise for `parent` and `path`" | `parent` and `project` |
| 262 | `board_brief`: "No arguments" | it takes `project` (`BriefArgs`) |
| 265 | "the six global status counts" | seven statuses |

Line 28 is the kind of count `scripts/sync-tool-count.py` exists for; its pattern
does not match the phrase "the same ten tools".

**Fix.** Correct the five lines. Then widen the count pattern to any number word
directly before "tools", and add a check in `tests/doc-names.py` that the field
list on line 71 equals the serde field names of `Task`.

### 17. `manager-loop-gaps.md` describes 0.2.0

It carries a status note, but a reader still has to work out which gaps are open.
Since every gap is now either landed or closed by decision, a one-line header
saying "historical; all items resolved as of 0.4.0, see the status note" would
save that work.

---

## Tests and process

### 18. The gate does not pin the counts it promises

`AGENTS.md` lists exact results, for example `369/369 passed (cli, lmdb)`, and
`gate.sh` says its checks are "what stops the counts drifting by hand". The checks
are patterns that accept any numbers:

```
want '^[0-9]+/[0-9]+ passed \(cli, lmdb\)$'
```

If a refactor silently drops half of `tests/cli.sh`, the suite prints
`180/180 passed`, the gate passes, and the table in `AGENTS.md` is wrong. The
counts are kept by hand in one place and checked in none.

**Fix.** Have `gate.sh` read each expected line out of the `AGENTS.md` table and
compare it with the output. The table then cannot drift, and a count that goes
down is a failure someone has to acknowledge by editing the table.

### 19. Nothing runs the gate automatically

There is no CI configuration and no git hook; the gate runs when someone remembers
to. With 229 commits in three weeks, mostly from agents, a `pre-commit` hook that
runs the fast gates (`fmt`, `clippy`, `test --lib`, the two doc gates) would catch
the common failures without the full two-minute run. Whether to add hosted CI is
tied to pushing, which is the owner's decision.

### 20. No test exercises a large board

**Status: done**: the ignored 2,510-task proof counts `/api/epics` task reads,
idle wait polls and task-filtered feed visits without elapsed-time assertions.

Every suite uses a few dozen rows, so findings 5 to 8 are invisible to the gate.
The ignored `claim_latency` test shows the pattern to follow: one ignored test
that builds a few thousand tasks and asserts a gross ceiling on `wait --epic` CPU
or on scan counts. Assert on work done (records decoded) rather than wall-clock
time, in line with the existing trap about latency assertions.

### 21. A compiled Python file is tracked

**Status: done**: the compiled file has been removed from the Git index.

`tests/__pycache__/mcp_client.cpython-314.pyc` is in the index although
`.gitignore` lists `__pycache__/`. It was added before the ignore rule.
`git rm --cached` it.

---

## Small operational items

- **Done: `HOME` unset.** When neither `--db`/`AGENTBOARD_DB` nor
  `XDG_STATE_HOME` supplies the board path, `board_path` requires `HOME` and
  refuses by name instead of creating `./.local/state/agentboard/lmdb`
  (`src/main.rs:46`).
- **Daemon shutdown.** `axum::serve` runs without graceful shutdown
  (`src/daemon.rs:72`). LMDB is crash-safe, so no data is at risk, but an
  in-flight `board_feed(wait=…)` is cut mid-response on `systemctl stop`. A
  SIGTERM handler would need tokio's `signal` feature; low priority.
- **Done: migration hint.** The format-mismatch refusal says to run
  `python3 scripts/migrate-board.py`, a path relative to this repository
  (`src/store/open.rs:170`). An installed binary has no such file nearby, and the
  README's working invocation is `uv run --with lmdb …`. Use that wording.
- **Done: release profile.** `opt-level = 2` with no `strip` gives a 9.8 MB binary.
  `strip = true` is free. `lto` is optional and slows the build gate.
- **Done: two polling constants.** `POLL_INTERVAL_MS = 25` is defined separately in
  `feed.rs` and `wait.rs`.

---

## What I am not recommending

`AGENTS.md` fixes the shape: the board stores state and blocks, the manager is the
only wake, a seat does one row and stops. Nothing above adds delivery, a lease, a
scheduler, a per-seat queue or liveness. I also do not recommend:

- **A JSON output mode for the CLI.** The compact text is the product; the daemon
  already has a JSON API for programs.
- **Authentication.** Names are addresses on a trusted team and the daemon is
  loopback-first. Finding 3 is about a confusing bind, not about access control.
- **Status transition rules.** Any actor can move an unheld row to any status.
  That is loose, but the holder and epic guards cover the cases that cost work,
  and a transition table would be a second place that decides the workflow.
- **Replacing the 25 ms poll with notifications.** With finding 5 fixed, an idle
  poll is one metadata read. Cross-process wakeups would add far more than they
  save.

## Suggested order

1. Findings 1 and 21: a validation call and an index cleanup.
2. Finding 16: five doc lines, then the two gate extensions.
3. Finding 2: `wait --after`, plus the manager skill line.
4. Finding 5 (sequence check) and finding 7 (use the task index in the feed).
5. Finding 6 (one read model for epics) and finding 8, option 1.
6. Findings 4 and 12 together, since both are about what crosses the
   store-adapter boundary.
7. Finding 15: annotations and argument descriptions.
8. Finding 18, then 20.

## Decisions

The owner delegated these six on 2026-11-11. Each is decided from what the project
already does elsewhere, and each names what would reopen it.

### D1. Events record sizes, not text, for descriptions and goals

**Decision.** An `updated` event stores `{"old_chars": N, "new_chars": M}` for
`description` and `goal`. `title`, `status`, `priority`, `assignee`, `parent_id`
and `project_id` keep full old and new values. A `created` event embeds the task
with `description` and `goal` replaced the same way.

**Why.**

- The project already made this choice for reports: "the text itself never enters
  the event or the feed" (`report_changes`). Descriptions and goals are the same
  kind of long text, so one rule should cover both.
- Nothing reads the stored text. `render.rs` prints only `text`, `comment` and
  `reason` from a payload, so `feed --full` never shows it, and the dashboard
  dumps `changes` as raw JSON. The bytes are paid for and unused.
- The feed is what agents read, and the stated rule is that agents pay per byte.
- The short fields that explain a workflow (who, what status, when) keep their
  full history, which is what the audit trail is used for.

**Cost.** The previous wording of a description is no longer recoverable from the
board. Old events keep their full payloads and still read, because a payload is
free-form JSON; this is not a format change and needs no migration.

**Reopens if** someone needs to diff a description across revisions. The answer
then is a separate history record, not a larger feed.

**Status: done** in T305: created and updated events record description and goal
character counts, and the task timeline renders those size pairs.

### D2. Text gets size limits, enforced on write only

**Status: done** in T306: descriptions, goals, comments, force reasons, update
comments and reports are bounded on write; oversized stored rows remain readable.

**Decision.** Limits in characters, counted the way the title limit is:

| Field | Limit |
| --- | --- |
| `description`, `goal` | 32,768 each |
| a comment, a force reason, an update comment | 32,768 |
| each report, total after the append | 262,144 |

A refusal names the field, the size given and the limit.

**Why.** The largest real payloads are plans and review verdicts of a few
kilobytes, so these limits are about ten times what a seat writes on purpose and
still stop a build log passed to `--coder-report-file`. A report limit on the
total, not the section, is what bounds the record.

**Trap.** Put the check in `Task::new` and `Task::apply`, on the fields the patch
changes. Do not put it in `Task::validate`: that runs on every read
(`src/store/tasks.rs:314`, `:416`), so a limit there would make an existing
oversized row unreadable. An existing row over the limit stays readable and
editable in its other fields.

### D3. A full map is fixed with a `grow` verb, not export

**Decision.** Add `agentboard grow --map-size-mb N`. It takes the exclusive
lifetime lock exactly as `reset` does, refuses while any client is attached,
refuses a size that is not larger, reopens the environment with the new size and
rewrites `map_size_mb` in the metadata in one commit. With it:

**Status: done in T307.** `grow` persists the larger LMDB map and metadata size
under the exclusive lifetime lock while preserving the board instance id.

- Finding 11 lands at the same time: an existing board opens at its stored size,
  and `AGENTBOARD_MAP_SIZE_MB` matters only when a board is created. Otherwise
  every seat card would need editing after a grow.
- The map-full message names `grow` instead of `reset`.
- `brief` and `/api/health` report map usage, so the manager sees it coming.

**Why not export.** Export alone does not get anyone out of a full map; it needs
an import that reproduces ids, sequences, indexes and the roster exactly, which
is a second write path into the store and several hundred lines. `grow` reuses
the lock path `reset` already proves and is a few dozen lines. It also keeps the
instance id, so cursors held by seats stay valid; a re-created board would
refuse all of them.

**Why this does not break the existing rule.** `AGENTS.md` says there is no map
growth *in place*, meaning under attached clients, where another process would
see `MDB_MAP_RESIZED`. `grow` runs only when nothing else is attached, which is
the same condition `reset` and migration already require.

**Prove it.** A `tests/cli.sh` case: fill a small map until a write refuses, run
`grow`, repeat the write, then reopen with the variable unset and read the row
back. Also check that `grow` refuses while a daemon holds the board. The reopen
step matters: it is what shows the new size was persisted by LMDB and not only
written to the metadata.

**Export** stays unbuilt until someone needs an archive or a text backup. Copying
the directory with every process stopped remains the backup.

### D4. No parent index yet

**Decision.** Do not add `tasks_by_parent` now. Do the two fixes that need no
format change: the sequence check in `wait` (finding 5) and decoding only the
needed fields in scans (finding 8, option 1).

**Why.** A ninth database means a format bump, a migration step and a stopped
board for every user, to speed up reads that take 13 to 30 ms today. The one
measured problem, `wait --epic`, is solved by the sequence check alone.

**Reopens when** a real board passes roughly 10,000 tasks, or when closing a task
(the scan inside the write transaction) is measurably slow. Finding 20's large
board test is what will show it.

### D5. Seats do not manage projects; the two delete tools stay

**Decision.** No `project add`, `ls` or `update` over MCP. Keep
`board_project_delete` and `board_agent_delete` as they are, and mark them with
`destructiveHint` (finding 15).

**Why.**

- Registering a project binds a name to a path on the owner's machine. That is
  setup, done once by a human or the manager at a shell, and the CLI covers it.
  A seat receives its project from the MCP URL and never needs to look one up.
- The two deletes were added on purpose one commit before 0.4.0 (`7ca16d9`), and
  they are manager housekeeping: clearing a roster name after a seat is closed,
  removing a project nobody uses. Both refuse while anything still depends on
  the target, so the worst outcome is a name that registers itself again on its
  next write.
- They stay without an event. An event belongs to a task, and neither delete has
  one; inventing a task-less event kind for two housekeeping calls would change
  the feed for every reader.

The surface stays at twelve tools.

### D6. A wildcard bind is refused at startup

**Decision.** `agentboard daemon 0.0.0.0:PORT` and `[::]:PORT` exit 1 before
binding, with or without `--allow-non-loopback`:

```
agentboard: daemon cannot bind the unspecified address 0.0.0.0; name one address, for example 192.168.1.50:8765
```

**Why.** Today it starts and then answers 421 to every remote client, which looks
like a network fault. The project's habit is to refuse by name instead of
guessing, and the alternative, accepting any Host on a wildcard bind, would
remove the DNS-rebinding guard for exactly the case that is exposed to a network.

**Test.** `tests/daemon.py`: both wildcard forms exit 1 with that line and open no
listener.

### What these change in the order of work

D1 and D2 go together after step 4 above, since both touch what `apply` writes.
D3 and finding 11 are one change and come before finding 9's warning line. D6 is
small enough to ride with step 1. D4 and D5 add no work.
