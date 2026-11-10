# AGENTS.md — changing this code

agentboard is one Rust binary that is both a shared task board on an embedded LMDB
directory and a bridge-free MCP server on stdio. This file is about CHANGING this
tree, not about running a team with it.

Read first:

- `README.md` — the model, the verbs and the ten tools.
- `PROTOCOL.md` — the wire surface agents see.
- `skills/agentboard-team` — running a team with this tool; the repo copy is canonical.

## Shape

Keep the loop in three parts, and keep each part small.

- The board stores state and blocks on it. `agentboard wait` (CLI only; the MCP
  surface has no wait tool) is how the manager blocks on rows; `release` is how a
  row a dead holder left comes back. The board never schedules work, dispatches
  it, or wakes anybody.
- The manager is the only wake. It reads the board, blocks on the rows it cares
  about, and prompts the next seat by name with `herdr` or `hc`, outside this binary.
- A seat does one row, reports it, and stops. It does not poll for its next row.

Do not add delivery, a lease, a scheduler or a per-seat queue. Each one makes the
board a second place that decides who works next, and the manager already is that
place.

## Prove it

Run these from the repo root. A change is not done until each prints what is beside
it. `bash scripts/gate.sh` runs the whole table in order, checks each line, and stops
at the first failure; `--full` adds the ignored latency run. Everything is offline
and uses scratch boards; the dashboard test starts its own local listener.

| Command | Expected |
| --- | --- |
| `cargo fmt --check` | no output |
| `cargo test --offline --lib` | `test result: ok. 107 passed; 0 failed; 1 ignored` |
| `cargo clippy --offline --all-targets -- -D warnings` | no warnings or errors (`Finished` is normal) |
| `cargo build --release` | `Finished` |
| `bash tests/cli.sh` (default binary) | `337/337 passed (cli, lmdb)` |
| `AGENTBOARD_TEST_BIN=target/release/agentboard` with `bash tests/cli.sh` (dashboard-feature binary) | `339/339 passed (cli, lmdb)` |
| `python3 tests/dashboard.py` (feature binary) | `dashboard: all checks passed (read-only HTTP, filters, paging, escaping, bounds)` |
| `python3 tests/mcp_client.py` | `188/188 passed (mcp, lmdb)` |
| `AGENTBOARD_MCP_TRANSPORT=http python3 tests/mcp_client.py` (the same suite against `agentboard daemon`) | `188/188 passed (mcp-http, lmdb)` |
| `python3 tests/daemon.py` | `daemon: 301/301 passed (http, lmdb)` (scratch boards, loopback port 0) |
| `bash tests/race.sh` | `ok: 8 processes claimed 24 tasks, 24 distinct, 24 claimed events` |
| `bash tests/migrate.sh` | `migrate: 32/32 passed (scratch LMDB, stepwise)`; or `SKIP migrate: Python lmdb unavailable offline (use uv run --with lmdb)` |
| `bash tests/seats-install.sh` | `220/220 passed (seats installer)` (isolated homes; no board writes) |
| `bash tests/start-agentboard-manager.sh` | `51/51 passed (manager launcher)` (stub `herdr` and `hc`; no live pane or terminal) |
| `bash tests/skills-install.sh` | `8/8 passed (skills installer)` (isolated homes; no board writes) |
| `python3 scripts/sync-tool-count.py --check` | `MCP router: 10 tools; published counts match` |
| `python3 tests/doc-names.py` | `doc-names: 14 pages, 130 CLI names, 141 MCP names passed` |
| `python3 scripts/doc-overlap.py` | four zero-count lines, exit 0 |
| `bash scripts/gate.sh` | `GATES OK: 16 gates passed`; with `--full` `GATES OK: 17 gates passed` |
| `bash scripts/gate.sh --features dashboard` | `GATES OK: 18 gates passed` |
| `cargo test --offline --lib -- --ignored --nocapture claim_latency` | `claim latency, 8 processes over 1000 tasks: p50=... p95=...`; gross-regression ceilings 75 ms debug / 40 ms release, not targets, overridable with `AGENTBOARD_CLAIM_P95_MS` |

`AGENTBOARD_TEST_BIN` points any suite at another binary; the suites never touch a
live board, because each one sets `AGENTBOARD_DB` to a fresh `mktemp` directory.
UI-affecting dashboard rows must include screenshot paths and overflow measurements
in the coder report.
Add or change behaviour with a test in the suite that owns that kind of check:
`tests/cli.sh` for verbs, exit codes and text, `tests/mcp_client.py` for tool text
and JSON-RPC shape, `tests/race.sh` for cross-process claims. When a count changes,
update the expectation here.

## Layout

- `src/domain.rs` — task, status, patch, claim and event rules; no I/O, no printing.
- `src/store/` — the LMDB store: `open` (Board, options, reset,
  lifetime lock), `keys` (ordered byte codecs), `tasks` (add/update/release/claim/
  comment, index upkeep, events), `feed` (cursors, filters, bounded older-page read,
  wait), `wait` (per-row
  blocking wait that re-queries its watched set), `list` (list, brief,
  status counts), `overview` (dashboard lanes, attention and held work),
  `tests`/`m3_tests`/`overview_tests` (store proofs).
- The board format is version 2 and uses eight named databases, including
  `projects`; format 1 boards refuse to open until migrated.
- `src/render.rs` — the only place compact text is built, including the `…`
  truncation and the full-text path.
- Epics are derived direct-child rollups in `store::epic`, shared by both
  adapters. The terminal-update guard runs in the same write transaction before
  roster, index and event writes; no epic flag or database is stored.
- `src/cli.rs` — argv as types. Validation stays in the store.
- `src/main.rs` — thin: resolve board and actor, call the store, print, exit.
- `src/dashboard.rs` and `src/dashboard/` — optional, feature-gated, read-only HTTP
  dashboard adapter, route/query parsing, page rendering, shared HTML helpers,
  and embedded CSS/theme assets. `template.rs` fills the embedded files in
  `templates/`; text slots are escaped once at fill time.
- `src/daemon.rs` — `agentboard daemon`: the same MCP adapter over streamable HTTP
  (axum + rmcp, stateless) at `/mcp/<actor>`, one shared `Arc<Board>`, and the guard
  (Host, Origin, loopback peer for `/mcp`) that runs before every route.
  `src/daemon/api.rs` is the read-only JSON API beside it (GET only, no write route);
  it builds JSON from the store read models and takes every query through
  `src/routes.rs::parse`, the one strict query validator the HTML dashboard shares.
- `dashboard/` — the daemon's dashboard: `index.html`, `app.js`, `style.css`,
  embedded with rust-embed (`debug-embed`, so debug and release serve the same
  bytes) and served by `src/daemon.rs`: every page path answers `index.html`,
  files live under `/assets/`, and `app.js` builds the DOM from the JSON API with
  `createElement`/`textContent` only. The feature-gated SSR dashboard still
  includes `dashboard/style.css` until it is removed.
- `src/server.rs` — the MCP adapter, ten tools, store work on `spawn_blocking`.
- `scripts/gate.sh` — the proof table as one command; `scripts/sync-tool-count.py`
  and `tests/doc-names.py` are the two doc-drift gates it calls.

Dependencies point one way: `main.rs` and `server.rs` call `store`; `store` never
calls them. Store modules never print and never exit; the adapters render text and
map errors to exit codes or MCP codes. Put new behaviour in the store first, then
have both adapters call it, so a rule exists once.

## Traps

- A `flock` exclusive→shared downgrade is a release, so a `reset` can unlink
  `data.mdb` inside it. Open compares `(dev, inode)` across the downgrade and
  retries; do not remove that check.
- `serve()` returns a running service. Awaiting only that returns immediately and a
  client sees its initialize reply followed by a closed stream; call
  `.waiting().await`.
- rmcp 3.5 accepts `initialize` for protocol `2025-11-25` with no `_meta`. The
  2026-07-28 draft drops `initialize` and demands inline
  `io.modelcontextprotocol/protocolVersion` and `clientCapabilities` in
  `params._meta`; a plain `initialize` for that version is refused.
- Give every free-text argv slot `allow_hyphen_values` and numeric slots the same,
  or `add "- dash title"` and `--priority -3` are rejected as flags. `tests/cli.sh`
  exists to catch exactly that.
- There is no map growth in place: a full map fails the write atomically and names
  `AGENTBOARD_MAP_SIZE_MB`. Set it before the board is created.
- `reset` must take the exclusive lock before it scans for unknown files, or a
  stray file turns disposal into a half-deleted board.
- A bare sequence cursor (for example `--after 0`) means "this board", so history
  replay works without knowing the instance id; a foreign `instance:seq` refuses
  with `CursorInstanceMismatch`.
- A wait re-queries the watched rows on every poll and it is an ALL predicate: a
  frozen id list misses a child filed mid-epic, and an any-row match turns "every
  child terminal" into "the first child finished". Both mutations are caught by the
  wait proofs in `tests.rs`/`m3_tests.rs` and `tests/cli.sh`. One row contributes at
  most `MAX_WAIT_EVENTS` event lines; the rest are counted, never printed. The
  optional assignee branch is the opposite shape: ANY watched row, decided from
  `updated` events with a recipient after the watermark, scanning past the event
  cap. Do not make it ALL-rows or read it from current state.
- Full text must never go through the compact renderer: no title clip, no
  whitespace collapse. A reader that asks for full text gets stored bytes.
- The template whitespace rule applies only to template source lines; it never
  applies to slot values.
- Tool counts in `src/server.rs` (first line), `README.md`, `PROTOCOL.md`, this file
  and `seats/README.md` come from `python3 scripts/sync-tool-count.py`; run it with
  `--check` in a gate and never count by hand. A count written in words counts too.
- A published tool *list* is a second way to drift: `tests/doc-names.py` checks that
  a "the board tool set is the N tools" bullet states the router's count and names
  exactly the router's set, so adding a tool means fixing the list, not just the
  number.
- An explicit-id `claim` refusal is an error (CLI exit 1, MCP `-32602`); only the
  id-less probe answers `no ready tasks` with exit 3. A refusal returned as `Ok` is
  read by scripts as a successful claim.
- Never assert on a rendered timestamp or relative age; assert on events, counts or
  a range, and never on a wall-clock latency: the claim-latency ceilings exist to
  catch a pathological claim path, not to hold a machine to a number.
- Mutation-check a guard you add: remove it and confirm a test fails.

- The daemon's actor and project are per request (`RequestScope` in the request
  extensions): never read `AGENTBOARD_ACTOR`, `AGENTBOARD_PROJECT` or the cwd for an
  HTTP call. `tests/daemon.py` mutation-checks both the guard and the actor read.
- A resident daemon holds the board's lifetime lock: `reset` and migration refuse
  until it is stopped.

## Git

- Branch `master`; one objective per commit.
- `git add` explicit paths, never `-A`: it sweeps scratch files into the commit.
- Commits land locally and are never pushed by an agent; pushing is the human's call.
- `rework-plan/` is untracked on purpose. `tmp-review-*.md` are throwaway and stay out.

## If you are the reviewer

Reviewers read and judge the commit or row the brief names, its `description` and
`goal`. They report findings classified blocker, follow-up or nit.
Reviewers may re-run suites against the already-built binary on scratch boards,
and directly run that binary on a scratch board; they never build. For a dashboard
UI change, the Claude reviewer may start the already-built dashboard on a scratch
board and use `agent-browser` only against its local loopback URL, following
`skills/agentboard-reviewer/SKILL.md`; screenshots and notes stay outside the
repository, and the server and browser must be stopped. The Codex reviewer profile
remains read-only/git-only because no verified setting safely bounds browser
network access to loopback while permitting scratch writes; Codex reviewers judge
from the coder report and screenshot paths.
`git status --short` stays clean apart from the one report file the brief allows.

## Decide with the human, not alone

- Pushing.
- Wiping or migrating a live board.
- Changing the board format or the eight database names.
- Adding a dependency the task at hand does not need.

## Where the rest lives

- `skills/agentboard-team` — running a team: launcher, wake commands, roles,
  workflow. `skills/install.py --force` creates canonical repo links under
  `~/.agents/skills`, then links Claude and Codex skills to those
  canonical names. There is one physical copy: never copy over the installed links.
- `PROTOCOL.md` — the wire surface: ten tools, cursors, refusals, exit codes.
- `skills/agentboard-{manager,analyst,coder,reviewer}/` — one seat entry point each
  (`/agentboard-<seat>`; `$agentboard-<seat>` under Codex). Grok uses
  `agentboard-manager/SKILL.md`.
- `seats/` — the canonical per-seat client cards (codex profiles, claude MCP
  configs) and `seats/install.py` to link them into a home directory.
