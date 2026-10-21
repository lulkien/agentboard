# AGENTS.md — changing this code

agentboard is one Rust binary that is both a task board (SurrealDB by default, SQLite
opt-in) and a bridge to agent panes, with two front doors: a CLI and an MCP server on stdio. This file is
about CHANGING this tree, not about running a team with it.

Read first:

- `README.md` — the model and the tools.
- `PROTOCOL.md` — the wire surface agents see.
- `src/schema.sql` — the only schema. There is no migration path.

## Prove it

Run these from the repo root. A change is not done until each prints what is beside it.
The default build is SurrealDB; sqlite is the opt-in build (`--no-default-features --features sqlite`).

Fresh-box prerequisite: a `surreal` binary, version 2.7.x, on `PATH` or at
`~/.surrealdb/surreal`. A default `cargo test` starts throwaway servers from it, so with
`AGENTBOARD_REQUIRE_SURREAL=1` a missing binary fails the run instead of printing `SKIP`.
A server of any other version is refused by name (`want surrealdb 2.7.x`).

| Command | Expected |
|---|---|
| `cargo build --release` | `Finished` |
| `AGENTBOARD_REQUIRE_SURREAL=1 cargo test` | `test result: ok. 46 passed` (unit) and `test result: ok. 22 passed` (`tests/manager.rs`, the stub-herdr suite); add `-- --nocapture` and `rg -c SKIP` prints nothing |
| `cargo test --no-default-features --features sqlite` | `test result: ok. 36 passed` and `22 passed` (needs no `surreal` binary) |
| `bash tests/cli.sh` | `189/189 passed (backend: surreal)` (against the sqlite binary: `194/194 passed (backend: sqlite)`) |
| `bash tests/dispatch.sh` | `59/59 passed (backend: surreal)` (either build) |
| `bash tests/manager.sh` | `43/43 passed (backend: surreal)`: `manager up/down/renew` and `wait --agent` against `tests/herdr_stub.py` and a throwaway board. `AGENTBOARD_HERDR_LIVE=1 bash tests/manager-live.sh` is the one round against the real herdr (scratch agents only); without the variable it prints `skipped` |
| `bash tests/surreal.sh` | `surreal.sh: all checks passed` (builds both feature sets itself, plus a debug surreal build under `target/surreal-debug`, then restores the default release build; runs `cli.sh` and `dispatch.sh` on both builds, `mcp_client.py` on both, and prints `189/189`, `194/194`, `59/59`, `151/151` for them; about a minute once that cache is warm) |
| `bash tests/race.sh` | `claims=80 unique=80 elapsed_ms=... (backend: surreal)` (`AGENTBOARD_TEST_BIN` picks the binary; sqlite prints `(backend: sqlite)`) |
| `bash tests/wait.sh` | ends with `elapsed_ms=... (backend: surreal)`, no `FAIL` |
| `rg -c '#\[tool\(' src/server.rs` | `22`, equal to the count on line 1 of `src/server.rs` (`MCP surface: 22 tools`) |
| `cargo fmt --check` | no output |
| `cargo clippy --all-targets -- -D warnings` | no output (`Finished` only); run it again with `--no-default-features --features sqlite` |

The suites drive `target/release/agentboard`: build release before running them.
`tests/cli.sh`, `dispatch.sh`, `race.sh` and `wait.sh` source `tests/board.sh` (which sources
`tests/server.sh`): it reads the backend off the binary (`AGENTBOARD_TEST_BIN` picks it),
clears `AGENTBOARD_DB` and `AGENTBOARD_SURREAL_URL`, and starts throwaway servers on random
loopback ports, so a bare run never touches the default board. Each prints
`(backend: surreal|sqlite)` in its summary line, and `tests/surreal.sh` matches it.
`python3 tests/mcp_client.py` cannot start a server itself: on the default build it refuses
(exit 1, one line) unless `AGENTBOARD_TEST_DB` (a fresh data directory) and
`AGENTBOARD_SURREAL_URL` (a throwaway server, never port 18720) are both set. Prefer
`bash tests/surreal.sh`, which supplies both and prints `151/151 checks passed (backend: surreal)`. With a
sqlite binary and no `AGENTBOARD_TEST_DB`, it uses `.probe/client.db`.
Add or change behaviour with a test in the suite that owns that kind of check
(`cli.sh` for exit codes and stderr, `mcp_client.py` for tool text, `dispatch.sh`
for the bridge). When a count changes, update the expectation here.

A change under `src/store/surreal/` is not done on a box where any of its tests
printed `SKIP` (no `surreal` binary): the `AGENTBOARD_REQUIRE_SURREAL=1` row turns a
skip into a failure. SKIP is fine on a device that builds only the sqlite binary.

## Layout

- `src/main.rs` is thin: parse, call the store, print, set the exit code.
- `src/cli.rs` holds argv as types (clap derive); validation stays in the store.
- `src/store/` has one module per concern: `tasks`, `epics`, `diffs`, `sessions`, `wait`,
  `dispatch`, `proc`, `render`. Event logging and id parsing live in `mod.rs`. `diffs` is
  the append-only diff rows (`diff_add`/`diff_get`/`diff_list`); the review gate and the
  auto-filed review row live in `tasks::apply`, and the reopen cap and chain-depth rules in
  `apply`, `add` and `set_parent`, once per backend. The strings every adapter prints for
  those rules are in `text.rs`.
- `src/server.rs` is the MCP adapter.

Dependencies point one way: `main.rs` and `server.rs` call `store`; `store` never
calls them. Store modules never print and never exit; the adapters render text and
map errors to exit codes or MCP codes. Put new behaviour in the store first, then
have both adapters call it, so a rule exists once.

## Traps

- An explicit-id `claim` refusal is an `Err` (CLI exit 1, MCP -32603); only the
  id-less probe stays `Ok("no ready tasks")`. A refusal returned as `Ok` is read by
  scripts as a successful claim.
- Give every free-text argv slot `allow_hyphen_values` (and `allow_negative_numbers`
  on numeric ones). Without it clap rejects `--description "- bullet"`; `tests/cli.sh`
  exists to catch that.
- Never assert on a rendered relative age (`created: 0s`, `lease: 59m left`): the
  result depends on where the second hand falls. Compare a render with the
  age/lease lines stripped, assert a range, or assert on events.
- The tool count on line 1 of `src/server.rs` must equal
  `rg -c "#\[tool\(" src/server.rs`. Derive it; do not count by hand.
- Agents run `~/.cargo/bin/agentboard`; `./target/release/agentboard` is only the
  checkout, and `tests/mcp_client.py` drives the latter. After changing the CLI or
  MCP surface run `cargo install --path . --locked`, or the verbs you built are
  missing from the binary every agent executes.
- A schema change has no migration: an old board is refused before any DDL.
  Switching boards means stopping every agent first, because their MCP servers are
  processes of the old binary and will recreate the old schema in the new file.
- Route every herdr/tmux bridge call through the one bounded runner in
  `src/store/proc.rs`, which honours `$AGENTBOARD_DELIVER_TIMEOUT_MS`. Before
  claiming "every call is bounded", run `rg "Command::output" src/` and check.
- A kill must kill the process group (`process_group(0)` plus a group kill), and
  its test must assert the process is dead: a test that only times the caller
  passes with the kill deleted.
- There is no `sqlite3` CLI on the dev box; inspect a board with python3's stdlib
  `sqlite3` module.
- Mutation-check a guard you add: remove it and confirm a test fails.

## Git

- Branch `master`; one objective per commit.
- `git add` explicit paths, never `-A`: it sweeps scratch files into the commit.
- Commits land locally and are never pushed by an agent; pushing is the human's call.

## Decide with the human, not alone

- Pushing.
- Wiping or migrating a live board.
- Changing the schema.
- Adding a dependency the task at hand does not need.

## Where the rest lives

- `skills/agentboard-manager` — running a team with this tool. The repo copy is canonical:
  edit it, run `skills/install.sh --force`, then `diff -r` the repo copy against the
  installed one in both directions. Compare line counts first: an installed copy that is
  ahead of the repo copy is destroyed by `--force`.
- `PROTOCOL.md` — the wire surface: the chain, the cap, the diff verbs, who moves which status.
- `docs/workflow.md` — the only page with diagrams; the other pages link to it.
