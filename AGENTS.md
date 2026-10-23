# AGENTS.md — changing this code

agentboard is one Rust binary that is both a shared task board on an embedded LMDB
directory and a bridge-free MCP server on stdio. This file is about CHANGING this
tree, not about running a team with it.

Read first:

- `README.md` — the model, the verbs and the nine tools.
- `PROTOCOL.md` — the wire surface agents see.
- `skills/agentboard-manager` — running a team with this tool; the repo copy is canonical.

## Prove it

Run these from the repo root. A change is not done until each prints what is beside it.
Everything is offline and uses scratch boards; there is no server to start and no
`surreal` binary anywhere.

| Command | Expected |
| --- | --- |
| `cargo fmt --check` | no output |
| `cargo test --offline --lib` | `test result: ok. 58 passed; 0 failed; 1 ignored` |
| `cargo clippy --offline --all-targets -- -D warnings` | no output |
| `cargo build --release` | `Finished` |
| `bash tests/cli.sh` | `82/82 passed (cli, lmdb)` |
| `python3 tests/mcp_client.py` | `41/41 passed (mcp, lmdb)` |
| `bash tests/race.sh` | `ok: 8 processes claimed 24 tasks, 24 distinct, 24 claimed events` |
| `bash tests/skills-install.sh` | `6/6 passed (skills installer)` (isolated homes; no board writes) |
| `python3 scripts/sync-tool-count.py --check` | `MCP router: 9 tools; published counts match` |
| `python3 tests/doc-names.py` | `doc-names: 11 pages, 56 CLI names, 80 MCP names passed` |
| `python3 scripts/doc-overlap.py` | four zero-count lines, exit 0 |
| `cargo test --offline --lib -- --ignored --nocapture claim_latency` | `claim latency, 8 processes over 1000 tasks: p50=... p95=...`; debug ceiling 25 ms, release 5 ms |

`AGENTBOARD_TEST_BIN` points any suite at another binary; the suites never touch a
live board, because each one sets `AGENTBOARD_DB` to a fresh `mktemp` directory.
Add or change behaviour with a test in the suite that owns that kind of check:
`tests/cli.sh` for verbs, exit codes and text, `tests/mcp_client.py` for tool text
and JSON-RPC shape, `tests/race.sh` for cross-process claims. When a count changes,
update the expectation here.

## Layout

- `src/domain.rs` — task, status, patch, claim and event rules; no I/O, no printing.
- `src/next_store/` (module `store`) — the LMDB store: `open` (Board, options, reset,
  lifetime lock), `keys` (ordered byte codecs), `tasks` (add/update/claim/comment,
  index upkeep, events), `feed` (cursors, filters, bounded wait), `list` (list,
  brief), `tests`/`m3_tests` (store proofs).
- `src/render.rs` — the only place compact text is built, including the `…`
  truncation and the full-text path.
- `src/cli.rs` — argv as types. Validation stays in the store.
- `src/main.rs` — thin: resolve board and actor, call the store, print, exit.
- `src/server.rs` — the MCP adapter, nine tools, store work on `spawn_blocking`.

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
- Full text must never go through the compact renderer: no title clip, no
  whitespace collapse. A reader that asks for full text gets stored bytes.
- Tool counts in `src/server.rs` (first line), `README.md`, `PROTOCOL.md` and this
  file come from `python3 scripts/sync-tool-count.py`; run it with `--check` in a gate
  and never count by hand.
- An explicit-id `claim` refusal is an error (CLI exit 1, MCP `-32602`); only the
  id-less probe answers `no ready tasks` with exit 3. A refusal returned as `Ok` is
  read by scripts as a successful claim.
- Never assert on a rendered timestamp or relative age; assert on events, counts or
  a range.
- Mutation-check a guard you add: remove it and confirm a test fails.

## Git

- Branch `master`; one objective per commit.
- `git add` explicit paths, never `-A`: it sweeps scratch files into the commit.
- Commits land locally and are never pushed by an agent; pushing is the human's call.
- `rework-plan/` is untracked on purpose. `tmp-review-*.md` are throwaway and stay out.

## If you are the reviewer

You read and judge; you never edit. Inputs: the commit or row the brief names, its
`description` and `goal`. Output: findings classified blocker, follow-up or nit.
No file edit, no commit, no build, no install, no probe that creates state;
`git status --short` stays clean apart from the one report file the brief allows.

## Decide with the human, not alone

- Pushing.
- Wiping or migrating a live board.
- Changing the board format or the seven database names.
- Adding a dependency the task at hand does not need.

## Where the rest lives

- `skills/agentboard-manager` — running a team: launcher, wake commands, roles,
  workflow. `skills/install.sh --force` creates canonical repo links under
  `~/.agents/skills`, then links Claude, Codex, opencode and Hermes skills to those
  canonical names. There is one physical copy: never copy over the installed links.
- `PROTOCOL.md` — the wire surface: nine tools, cursors, refusals, exit codes.
- `skills/manager/` — the seat entry points (`/manager`, `$manager`).
