# AGENTS.md — changing this code

agentboard is one Rust binary that is both a task board (SQLite) and a bridge to
agent panes, with two front doors: a CLI and an MCP server on stdio. This file is
about CHANGING this tree, not about running a team with it.

Read first:

- `README.md` — the model and the tools.
- `PROTOCOL.md` — the wire surface agents see.
- `src/schema.sql` — the only schema. There is no migration path.

## Prove it

Run these from the repo root. A change is not done until each prints what is beside it.

| Command | Expected |
|---|---|
| `cargo build --release` | `Finished` |
| `cargo test` | `test result: ok. 12 passed` |
| `AGENTBOARD_REQUIRE_SURREAL=1 cargo test --no-default-features --features surreal` | `test result: ok. 14 passed`; add `-- --nocapture` and `rg -c SKIP` prints nothing |
| `python3 tests/mcp_client.py` | `121/121 checks passed` |
| `bash tests/cli.sh` | `111/111 passed` |
| `bash tests/dispatch.sh` | `59/59 passed` |
| `bash tests/race.sh` | `claims=80 unique=80` |
| `bash tests/wait.sh` | ends with `elapsed_ms=...`, no `FAIL` |
| `cargo fmt --check` | no output |
| `cargo clippy --all-targets -- -D warnings` | no output (`Finished` only) |

The suites drive `target/release/agentboard`: build release before running them.
Add or change behaviour with a test in the suite that owns that kind of check
(`cli.sh` for exit codes and stderr, `mcp_client.py` for tool text, `dispatch.sh`
for the bridge). When a count changes, update the expectation here.

A change under `src/store/surreal/` is not done on a box where any of its tests
printed `SKIP` (no `surreal` binary): run the `AGENTBOARD_REQUIRE_SURREAL=1` row, which
turns a skip into a failure. SKIP is fine on a device that builds the sqlite binary.

## Layout

- `src/main.rs` is thin: parse, call the store, print, set the exit code.
- `src/cli.rs` holds argv as types (clap derive); validation stays in the store.
- `src/store/` has one module per concern: `tasks`, `epics`, `sessions`, `wait`,
  `dispatch`, `proc`, `render`. Event logging and id parsing live in `mod.rs`.
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

- `skills/agentboard-manager` — running a team with this tool.
- `PROTOCOL.md` — the wire surface.
- `docs/` — diagrams.
