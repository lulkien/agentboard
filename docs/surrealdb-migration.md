# SurrealDB Migration Design

agentboard gets a second storage backend, SurrealDB 2.7, selected **at compile time** by a Cargo feature. A build is SQLite or SurrealDB, never both. This host builds the SurrealDB variant and talks to a local `surreal` server that the binary starts itself. Every board write also takes a cross-process file lock, because the server alone is not atomic under contention (Evidence, 10 runs). Every other device keeps the default SQLite build and behaves exactly as today. Old SQLite data is not migrated.

The document runs in this order: decisions, evidence, rejected alternatives, risks, implementation order, unverified list.

---

## Decisions

### 1. Two Cargo features, exclusive

```toml
[features]
default = ["sqlite"]
sqlite  = ["dep:rusqlite"]
surreal = ["dep:surrealdb"]
```

`rusqlite` and `surrealdb` become optional dependencies. `tokio` stays a normal dependency (`rmcp` needs it in both builds), so the `surreal` feature does not list `dep:tokio`; cargo rejects `dep:` on a non-optional dependency. A build with neither or both features fails to compile, in `src/main.rs`:

```rust
#[cfg(all(feature = "sqlite", feature = "surreal"))]
compile_error!("features `sqlite` and `surreal` are exclusive. Build with `cargo build --release` (sqlite) or `cargo build --release --no-default-features --features surreal`.");
#[cfg(not(any(feature = "sqlite", feature = "surreal")))]
compile_error!("enable exactly one backend. Build with `cargo build --release` (sqlite) or `cargo build --release --no-default-features --features surreal`.");
```

Build commands:

| Device | Command |
|---|---|
| Every other device (default) | `cargo build --release` and `cargo install --path . --locked` |
| This host | `cargo build --release --no-default-features --features surreal` and `cargo install --path . --locked --no-default-features --features surreal` |

Client crate line: `surrealdb = { version = "2.0", default-features = false, features = ["protocol-ws"], optional = true }`. The server speaks WebSocket; the embedded `kv-surrealkv` engine is not compiled into the client (see Rejected alternatives). The exact client feature list is confirmed when the crate is added (see Unverified).

SurrealDB **3.x is not an option.** `surrealdb 3.3.4` does not compile on this box, nightly included: `diskann 0.56.0` fails with `E0277: Arc<B>: Batch` not satisfied. `surrealdb-idx 3.3.4` pins that `diskann`, so bumping `diskann` is refused by the resolver. 2.7.0 compiles and its server runs. Do not retry 3.x without a new `diskann` release.

### 2. One board trait, two implementations, one compiled

A trait for board operations lives in `src/store/board.rs` (new). The existing rusqlite code becomes its SQLite implementation; a new module is the SurrealDB one. Gating is by `#[cfg(feature)]`, so a surreal binary contains no `rusqlite` and a sqlite binary contains no `surrealdb`.

| File | Role |
|---|---|
| `src/store/board.rs` (new) | The trait and the shared row/return types. No backend code, no cfg. |
| `src/store/text.rs` (new, both builds) | Backend-neutral rows (`TaskRow`, `EpicRow`, `EventRow`, `BriefData`) and the pure text built from them: `ls`, `get`, `children`, `epic ls/get`, `brief`, the status summary and event blocks, `held_text`, the change-diff helpers, `normalize_path` and `path_note`. Each store loads plain values (normalising ids and timestamps at that boundary) and calls these, so the output cannot drift between backends. Added at step 6. |
| `src/store/bridge.rs` (new, both builds) | The pane bridges and the pure dispatch logic: `Liveness`, `herdr_agents`, `tmux_alive`, `tmux_up`, the herdr parsing, `tail`, `send`, `deliver` (herdr first, tmux fallback), target resolution, the not-ready refusal and the epic-path warning. No database access. Added at step 6.|
| `src/store/sqlite/` (moved) | `#[cfg(feature = "sqlite")]`. Every module that calls rusqlite today moves here as SQLite code: `tasks.rs`, `epics.rs`, `sessions.rs`, `wait.rs`, `render.rs`, `dispatch.rs`, plus the `open`, `check_schema` and `INIT` logic now in `src/store/mod.rs`. |
| `src/store/surreal/` (new) | `#[cfg(feature = "surreal")]`. `mod.rs` (connect, schema bootstrap), `serve.rs` (auto-serve), `lock.rs` (write lock, Decision 6), and one port of each of `tasks.rs`, `epics.rs`, `sessions.rs`, `wait.rs`, `render.rs`, `dispatch.rs`. |
| `src/store/stub.rs` | `StubBoard`, a `Board` whose every operation returns one "not implemented" error. It backs the surreal build until its operations are ported, and stands in for `BACKEND` when both or neither feature is on, so the `compile_error!` is the only error rustc prints. |
| `src/store/mod.rs` | Keeps id parsing, event logging and `db_path`; its `open()`, `Connection` and `check_schema` are rusqlite and move behind the trait. |
| `src/store/proc.rs` | Unchanged: no rusqlite, no `Connection`, no `open(`. |
| `src/main.rs`, `src/cli.rs`, `src/server.rs`, `src/clip.rs` | No rusqlite references; they call `store::` only. |

The file list comes from `rg -c 'rusqlite' src/**/*.rs`, not from memory. Hits per file (`rusqlite` / `Connection` / `open(`):

| File | rusqlite | Connection | open( |
|---|---|---|---|
| `src/store/render.rs` | 10 | 3 | 4 |
| `src/store/tasks.rs` | 8 | 1 | 7 |
| `src/store/epics.rs` | 4 | 3 | 5 |
| `src/store/dispatch.rs` | 3 | 2 | 2 |
| `src/store/mod.rs` | 3 | 9 | 2 |
| `src/store/sessions.rs` | 3 | 1 | 4 |
| `src/store/wait.rs` | 2 | 7 | 1 |
| `src/store/proc.rs`, `src/main.rs`, `src/cli.rs`, `src/server.rs`, `src/clip.rs` | 0 | 0 | 0 |

`dispatch.rs` imports `rusqlite::{OptionalExtension, params}`, takes `&rusqlite::Connection` (lines 57 and 78) and calls `open()` (lines 125 and 191); `render.rs` does its row queries on `Connection` directly. A surreal build would not compile with either left as is, so both are ported behind the trait.

Sync/async boundary. The store surface stays **sync**, because `src/main.rs` (CLI) is sync and `src/server.rs` already wraps store calls in `tokio::task::spawn_blocking` (the `blocking` helper). The SurrealDB implementation owns one shared `tokio` runtime (`static RUNTIME: OnceLock<Runtime>`, surreal build only) and `block_on`s each call. The sqlite build gains no tokio dependency from this change. `rmcp` keeps calling the sync surface through `spawn_blocking`; a natively async trait is a later option, not part of this migration.

### 3. Settings (surreal build only)

| Variable | Default | Notes |
|---|---|---|
| `AGENTBOARD_SURREAL_URL` | `127.0.0.1:18720` | Bind address and client endpoint. Loopback only. |
| `AGENTBOARD_SURREAL_USER` | `root` | |
| `AGENTBOARD_SURREAL_PASS` | `root` | Acceptable only because the bind is loopback. |
| data dir | `~/.local/state/agentboard/surreal` (`$XDG_STATE_HOME` honoured, as `db_path` does today) | `surrealkv://<dir>` for the server. `--db` and `AGENTBOARD_DB` name this directory in the surreal build. |

The SQLite build keeps `AGENTBOARD_DB` and `--db` exactly as today. There is no `AGENTBOARD_BACKEND` variable: selection is compile time.

`agentboard path` prints `board.db` (a file) in the sqlite build and the data directory in the surreal build. Scripts that `rm -f "$AGENTBOARD_DB"` need `rm -rf` in the surreal build; `tests/surreal.sh` does that itself.

### 4. Auto-serve (surreal build)

On the first board call in a process:

1. `GET http://<url>/health` with a short timeout (about 300 ms). Status 200: connect over WebSocket and go on.
2. Connection refused: spawn, detached (own session, so it outlives the CLI call that started it):
   ```
   surreal db start --no-banner --log warn --bind <url> --username <u> --password <p> surrealkv://<dir>
   ```
   stdout and stderr go to `<state dir>/surreal.log`.
3. Poll `/health` every 100 ms for a bounded window (about 10 s). On 200, connect.
4. Two processes racing: starters are **serialised by a lock**, the port does not decide it. After a failed probe a starter takes `<state dir>/surreal.start.lock` (`START_LOCK` in `src/store/surreal/serve.rs`, taken with `acquire_named`, waiting up to twice the start-up deadline), re-probes `/health`, and spawns only if nothing answers. A waiter that gets the lock finds the winner's server up and connects without spawning. Losing the port is no protection: the server that loses it exits with code 1, but only after it has opened and touched the data directory, and two servers on one SurrealKV directory corrupt it. Reproduced with the real 2.7.0 binary: a control (one server, kill, restart) keeps table `t` and row `t:1`; with a second server on the same directory, the next `DEFINE` fails with `Versioned error: A deserialization error occured: Invalid revision `24` for type `DefineTableStatement``, and after a restart the row is gone (`SELECT * FROM t` returns `[]`). The test is `two_starters_race_and_both_end_up_connected` (two threads, fresh port, asserts exactly one spawn); with the start lock removed it failed 5 of 5 runs at `exactly one starter spawns ... left: 2 right: 1`. **Residual gap:** a server started by hand or by another tool on the same directory is not covered by the lock.
5. Binary missing, or `/health` never answers: one line, no hang, non-zero exit:
   ```
   board server unavailable: ran `surreal db start --no-banner --bind 127.0.0.1:18720 ... surrealkv://<dir>`; see <state dir>/surreal.log. Install: curl -sSf https://install.surrealdb.com | sh -s -- --version 2.7.0
   ```
6. After connecting, check the server version (`/version`, expected `2.7.x`) and a schema marker record; refuse with a clear line on a mismatch (see Risks).

### 5. Schema

`SCHEMAFULL` tables with `DEFINE FIELD` and `DEFINE INDEX`, applied idempotently on connect (`DEFINE ... IF NOT EXISTS`). This replaces `check_schema` and `SCHEMA` from `src/store/mod.rs`. The old-format refusal becomes a schema-marker check: a `meta:schema` record holding a version integer; a missing or different value on a non-empty database is refused with a "move the data dir aside" line. A fresh directory has no marker and bootstraps.

### 6. Integer IDs and the claim

Task ids stay integers (`T3`, `E1`; events carry a monotonic `seq`). Allocation is a counter record, one statement:

```sql
UPDATE counters:task SET n += 1 RETURN AFTER;
```

Single-letter field names work (`n` returned 1, 2, 3 on 2.7.0). An earlier version of this document claimed the deserializer rejects them; that was a spike artefact: reading rows into `serde_json::Value` fails on record ids, reading into a struct with only the needed fields works. `DEFINE SEQUENCE` does not parse on 2.x.

The claim is one guarded statement, the same shape as the SQLite one; an empty result means not claimed:

```sql
UPDATE tasks SET status='working', assignee=$actor, lease_until=$lease
WHERE id=$id AND status='ready' RETURN AFTER
```

**This statement is not atomic by itself, on the server either.** In 10 runs of 8 client processes over 5 ready tasks, 2 runs handed one task to two processes, and a counter lost updates (Evidence). The server reports a retryable read/write conflict on some commits and silently lets others through. Retrying on the conflict error does not fix it, and neither does a version compare-and-set that re-reads after a failed CAS (Evidence).

**Guard (chosen): an exclusive cross-process file lock, held by the client around every board write.** Implemented in `src/store/surreal/lock.rs` with `std::fs::File::lock` (flock) on `<state dir>/board.lock`, a file opened fresh per operation. It is taken before the first statement of a store operation and released after the last, so a read-modify-write such as claim-then-event-insert is covered as a unit. Separate open file descriptions exclude each other, so it also serializes threads inside one process (the rmcp server's `spawn_blocking` workers), which the earlier in-process mutex did. With the lock: 10 of 10 claim runs clean and 160/160 distinct counter values (Evidence).

Why a file lock and not the alternatives:
- **Version CAS with re-read:** measured, still 2 of 10 runs with duplicates and 142 distinct of 160 counter values. The engine commits two writers that each saw the same version.
- **A single agentboard-owned writer process** (all writes proxied through one daemon): would work, but it adds a second daemon with its own protocol on top of the SurrealDB server. The file lock gives the same exclusion with one flock call and no new process.
- **Why the earlier rejection of `flock` does not apply:** that was a lock around the embedded store, where each process keeps its own cached view. Here every client reads and writes through the server, which has the one authoritative view; the lock only orders writers.

A holder that dies releases the lock (the kernel drops it on process exit). A holder that is alive but stuck would block waiters, so waiters take the lock with a bounded retry (`try_lock` in a loop, about 5 s) and then fail with one line naming the lock path. Both behaviours are the design; neither was exercised yet (see Unverified).

### 7. Tests

- **Default suite, sqlite, unchanged.** `cargo test`, `tests/cli.sh`, `tests/dispatch.sh`, `tests/race.sh`, `tests/wait.sh` and `tests/mcp_client.py` keep their counts (AGENTS.md table). They drive the built binary through `AGENTBOARD_DB` and argv/MCP only, so they do not know which feature produced it; the default build is what they run against.
- **Surreal unit tests:** `cargo test --no-default-features --features surreal` (22 tests today). Tests that need a real server start a throwaway one on a random port; when no `surreal` binary is found they print `SKIP <name>: ...` and pass, which is right on a device that builds the sqlite binary. On the surreal host that would prove nothing, so `AGENTBOARD_REQUIRE_SURREAL=1` turns every SKIP into a panic naming the missing binary. A surreal-feature change is not done on a box where any of those tests printed SKIP; `AGENTS.md` carries the command (`AGENTBOARD_REQUIRE_SURREAL=1 cargo test --no-default-features --features surreal` ends `ok. 22 passed`, and its `--nocapture` output has no SKIP).
- **`tests/surreal.sh` (new).** Expects `target/release/agentboard` built with `--features surreal` and a `surreal` binary. It picks a random free port, creates a throwaway data dir under `$TMPDIR`, starts `surreal db start` there, points `AGENTBOARD_SURREAL_URL` at the port, then:
  1. CRUD through the CLI over WebSocket (add, get, update, ls, done).
  2. The `tests/race.sh` property: 8 worker processes, 80 claims, expecting `claims=80 unique=80`.
  3. The contention probes that actually failed without the lock: 8 processes x 10 claim attempts over 5 ready tasks, repeated 10 times, expecting `duplicates=0` and `distinct=5` every run; and 8 processes x 20 id allocations expecting 160 distinct values and a final counter of 160.
  4. A mutation check: the same counter run with the lock disabled (an env switch read only by a test build) must show fewer than 160 distinct values, so a test that passes with the guard deleted is caught. The counter case is the likelier to show it (147 distinct of 160 in the one unguarded run, 142 with CAS); the claim case failed in 2 of 10 unguarded runs.
  5. Kills the server (process group) and removes the dir on exit, also on failure (`trap`).
  It is not part of the default suite. AGENTS.md gets a row for it when it lands.
- **Backend-agnostic scripts.** `cli.sh` and `dispatch.sh` are written against `AGENTBOARD_DB` file semantics (`rm -f` of a file, paths under `.probe/`). Running them against the surreal build is neither required nor claimed; the surreal build is covered by `tests/surreal.sh`. Making `cli.sh` runnable on both builds is implementation step 7, not a precondition.

---

## Evidence

The first server run below (4 processes, one run) and the embedded multi-process figures were the manager's, taken before T16; the coder did not rerun them. The 10-run server measurements, the guard runs and the SQLite control are the coder's from T17, on this box, SurrealDB server and client crate both 2.7.0 (`surreal 2.7.0 for linux on x86_64`), a throwaway `surrealkv://` directory and a random loopback port per run, server killed afterwards (the script checks it is dead). The single-handle embedded figures are the coder's, from T15.

**Server (the chosen design).** `~/.surrealdb/surreal db start --no-banner --log warn --bind 127.0.0.1:18720 --username root --password root surrealkv://<dir>`; `/health` returned 200. WebSocket clients, guarded claim as in Decision 6:

```
4 OS processes x 20 guarded claim attempts over 8 ready tasks:
  process 1 CLAIMED [1..8]
  process 2 CLAIMED []
  process 3 CLAIMED []
  process 4 CLAIMED []
8 claims for 8 tasks, zero duplicates.
80 guarded claim attempts on one connection: 0.02 s total.
```

**The 4-process server run above was one run, and it was clean by luck.** Repeated, the server is not safe (T17):

**Server, 10 runs, no guard.** 8 concurrent OS client processes (WebSocket, one connection each), 10 guarded claim attempts per process over 5 ready tasks (80 attempts per run), fresh tasks per run. Clients retry on `read or write conflict ... can be retried`; `conflict_retries` counts those. A run with `duplicates>0` or `distinct<5` is a failure.
```
run=1 procs_finished=8/8 conflict_retries=4 claimed=5 distinct=5 duplicates=0 ok
run=2 procs_finished=8/8 conflict_retries=0 claimed=5 distinct=5 duplicates=0 ok
run=3 procs_finished=8/8 conflict_retries=7 claimed=5 distinct=5 duplicates=0 ok
run=4 procs_finished=8/8 conflict_retries=5 claimed=5 distinct=5 duplicates=0 ok
run=5 procs_finished=8/8 conflict_retries=8 claimed=5 distinct=5 duplicates=0 ok
run=6 procs_finished=8/8 conflict_retries=0 claimed=5 distinct=5 duplicates=0 ok
run=7 procs_finished=8/8 conflict_retries=12 claimed=6 distinct=5 duplicates=1 FAIL
run=8 procs_finished=8/8 conflict_retries=5 claimed=5 distinct=5 duplicates=0 ok
run=9 procs_finished=8/8 conflict_retries=2 claimed=5 distinct=5 duplicates=0 ok
run=10 procs_finished=8/8 conflict_retries=4 claimed=6 distinct=5 duplicates=1 FAIL
failed runs: 2/10
```
**Server, counter allocation, no guard.** 8 processes x 20 `UPDATE counters:x SET n += 1 RETURN AFTER`, one run, expected 160 final and 160 distinct:
```
procs_finished=8/8 conflict_retries=370
observed=160 distinct=147 min=1 max=147
FINAL 147
```
**Server, version compare-and-set with re-read after a failed CAS** (`UPDATE ... WHERE status='ready' AND ver=$old`; counter `WHERE n=$old`), same two tests:
```
run=1 ... claimed=6 distinct=5 duplicates=1 FAIL
run=6 ... claimed=7 distinct=5 duplicates=2 FAIL      (other 8 runs ok; failed runs: 2/10)
observed=160 distinct=142 min=1 max=142
FINAL 142
```
**Server, exclusive file lock (`File::lock`) held around each statement, same two tests:**
```
run=1..10  procs_finished=8/8 conflict_retries=0 claimed=5 distinct=5 duplicates=0 ok   (failed runs: 0/10)
procs_finished=8/8 conflict_retries=0
observed=160 distinct=160 min=1 max=160
FINAL 160
```
**SQLite control (today's `agentboard claim T<id>` CLI, same shape: 10 runs, 8 processes, 10 attempts over 5 ready tasks):**
```
run=1..10 claimed=5 distinct=5 duplicates=0   (10 of 10 runs identical)
```

**Embedded SurrealKV, several processes (contrast).** Same test, store opened in each process:

```
4 processes each claimed all 8 tasks: 32 claims for 8 tasks.
An already-open process never saw another process's writes (ready stayed 8 across four ticks).
```

**Embedded SurrealKV, one process, one handle (T15, `surrealdb 2.7.0`).** 100 concurrent `tokio::spawn` counter increments sharing one `Surreal<Db>`: 70-76 distinct values and a final counter of 73-79 instead of 103, with or without `BEGIN/COMMIT` and with conflict retries. Behind one `tokio::sync::Mutex`: 100/100 distinct, final 103. Claim, 80 attempts over 8 tasks on one handle: duplicate winners in 3 of 5 ungated runs; 0 duplicates in 6 of 6 runs behind the mutex. Persistence: `CREATE` in one process, reopened in a second, returned the row.

**SurrealDB 3.x.** `surrealdb = { version = "3", default-features = false, features = ["kv-surrealkv"] }` resolves to `surrealdb 3.3.4` and `diskann 0.56.0`, which fails on `rustc 1.101.0-nightly (ea137335b 2026-10-05)`: `error[E0277]: the trait bound Arc<B>: Batch is not satisfied` at `diskann-0.56.0/src/graph/index.rs:514:56`. The manager's run counted 6 E0277 errors; mine ended with 8 errors from the same cause. `surrealdb-idx 3.3.4` pins that `diskann`, so a version bump is refused.

---

## Rejected alternatives

**Cross-process file lock (`flock`) around the embedded SurrealKV store, with no server.** Rejected on measurement. (A lock around writes to the server is the chosen guard, Decision 6; this rejection is only about the embedded engine.) A lock serializes writers but does not refresh another process's cached view of the embedded store: an already-open process never saw later writes (ready stayed 8 across four ticks), and four processes each claimed all 8 tasks. Reopening the store per operation inside the lock would avoid the stale view; that was not measured and costs a store open per call, which the server makes unnecessary.

**Server without a guard.** Measured unsafe: 2 of 10 claim runs with duplicates, counter 147 of 160. **Server with version CAS and re-read:** measured unsafe, 2 of 10 and 142 of 160. **A single agentboard-owned writer daemon:** not measured; a second daemon where one flock call does the job.

**Embedded SurrealKV plus an in-process mutex only.** Fixes one handle (Evidence, T15) and nothing else. Agents run one MCP process each, plus CLI calls.

**Stay on SQLite everywhere.** The costed fallback: no new cost, no daemon, and `tests/race.sh` already proves the claim. It is the default build, so it stays on every other device.

**SurrealDB 3.x.** Does not compile; see Evidence.

---

## Risks

| Risk | Impact | Handling |
|---|---|---|
| Server is not atomic under contention | Duplicate claims, lost counter updates | Cross-process file lock around every store operation (Decision 6). Removing it must fail `tests/surreal.sh`. |
| Wedged or dead lock holder | Waiters hang, or a dead holder blocks | Dead holder: the kernel releases flock on exit. Stuck holder: bounded `try_lock` wait (about 5 s), then one error line naming the lock path. Not yet exercised. |
| Daemon lifecycle | A new long-lived process the tool did not have | agentboard starts it; nothing stops it. It outlives the CLI call. It does not survive a reboot; the next board call starts it again. Stopping is `pkill -f 'bind 127.0.0.1:18720'`, or a future `agentboard stop`. |
| Two starters racing | A second server on the same directory corrupts the store (`Invalid revision ... DefineTableStatement`, rows lost) | Start-up is serialised by `<state dir>/surreal.start.lock`; the waiter re-probes and finds the winner's server (Decision 4). Test `two_starters_race_and_both_end_up_connected`, mutation-checked. A server started by hand or by another tool on the same directory is not covered. |
| `connect` hangs on a wedged server | A server that answers `/health` and `/version` but never completes the WebSocket upgrade blocked a verb past 25 s | The whole handshake, sign-in and bootstrap run under a 10 s timeout (`Timing::connect`) and fail with `board server unavailable: did not complete the WebSocket handshake within N ms for ...`. Test `a_server_that_wedges_after_health_cannot_hang_connect`; with the timeout removed it fails (`connect must return within 3 s`). |
| MSRV 1.89 | `File::try_lock` and `unlock` (the write lock) are stable from Rust 1.89 | `rust-version` is raised from 1.88 to 1.89 in `Cargo.toml`, and the `incompatible_msrv` allows are gone. The dependency tree already peaks at 1.88 (`time`, `rmcp-macros`, `serde_with`), so 1.89 is the first version the surreal build can honestly claim; an allow would hide a real floor. Cost: sqlite-only devices, which never touch the lock, get the same floor. Accepted: one number everywhere beats a per-feature MSRV, which cargo cannot express. |
| Schema bootstrap on a fresh dir | First call must create tables | Idempotent `DEFINE ... IF NOT EXISTS` plus the `meta:schema` marker; covered by `tests/surreal.sh`. |
| `agentboard path` | Prints a file today | Prints the data dir in the surreal build (Decision 3). |
| Old sqlite file in a surreal data dir | The server cannot open a file as a directory | Before spawning, if `<dir>` is a regular file, refuse: `path '<dir>' is a SQLite file, not a SurrealDB directory. Move it aside (mv '<dir>' '<dir>.old') and retry.` Embedded SurrealKV rejected a file with `Not a directory (os error 20)`; the server's own text is not measured. |
| Stale server from a different binary or version | Writing to a server on another schema or version | Check `/version` is `2.7.x` and the `meta:schema` marker matches before any write; otherwise refuse, naming the URL and the mismatch. |
| Weak default credentials | `root/root` | Loopback only; a non-loopback host in `AGENTBOARD_SURREAL_URL` is refused. |
| Cold build time | About +65 s (earlier session figure) | CI caching. Only the surreal build pays it. |

---

## Implementation order

Each step ends with the default-build suites green (AGENTS.md table); from step 3 on, also `cargo test --no-default-features --features surreal`. Every module that references rusqlite is in the port list, because a surreal build fails to compile if any one is left (counts in Decision 2).

1. **Feature split, no behaviour change.** `Cargo.toml` features and optional deps; `compile_error!` guards (Decision 1). The default build must still print `121/121`, `111/111`, `59/59`, `claims=80 unique=80`.
2. **Trait and SQLite move.** `src/store/board.rs`; move the rusqlite code behind it under `src/store/sqlite/` with `#[cfg(feature = "sqlite")]`: `tasks.rs`, `epics.rs`, `sessions.rs`, `wait.rs`, `render.rs`, `dispatch.rs`, and the `open`/`Connection`/`check_schema` part of `src/store/mod.rs`. Pure refactor, suites unchanged. `src/store/mod.rs` keeps ids and events; `proc.rs` stays.
3. **Surreal connect, serve and lock.** `src/store/surreal/mod.rs`, `serve.rs`, `lock.rs`: settings, `/health` probe, spawn, bounded poll, version and marker check, schema bootstrap, runtime, the write lock.
4. **`src/store/surreal/tasks.rs` first of the ports.** It carries add, list, get, claim and id allocation, so the contention runs in `tests/surreal.sh` (claim, counter, mutation check) prove the design before more is ported.
   **Status: done.** Ported: `add`, `list`, `get` (incl. `--full`), `claim` (explicit and id-less), `apply` and the verbs built on it, `edit`, `set_parent`, `heartbeat`, task `children`, the id allocator (`counters:task`) and event logging (`counters:event`). Every write holds the write lock across the whole read-modify-write and is one server-side transaction; reads take no lock (a write commits whole, so a read sees it or not at all, and a lock would only add contention). `tests/surreal.sh` proves it in the real binary: sqlite-vs-surreal transcript parity (119 lines identical), 10 runs x 8 processes x 10 id-less claims over 5 ready tasks with 0 duplicates, 8 x 20 parallel adds giving 160 distinct ids T1..T160 with no gaps, a mutation check (lock disabled by `AGENTBOARD_UNSAFE_NO_WRITE_LOCK=1`; in a recorded run: duplicate claims in 2 of 10 runs and 119 distinct ids of 120 handed out, plus loud conflict errors; restored it passes), and a server killed mid-run giving the one-line serve error with no hang. Concurrent first use of a fresh board is covered by phase (g): 8 simultaneous first `add`s (ids 1..8, one `meta:schema` record) and 8 simultaneous first reads. The reads are what exercise the bootstrap lock in `session_with` (a write session already holds the write lock before it connects): with that lock skipped the reads phase failed 5 of 5 runs (`read or write conflict`, `found no schema marker`, zero schema records), while the adds phase cannot show it.
5. **`epics.rs`, then `sessions.rs`.** Smaller, independent queries; each uses `open()` today and must take the lock for its writes.
   **Status: epics and sessions done.** Ported: `epic add/ls/get/status`, epic edit (through `edit E<n>`), `sessions`, `sessions prune` (including the refusal when neither bridge answers), `unregister`, `register`; tasks can now name an epic (`add --epic`, a subtask inherits its parent's, the cross-epic refusals), `get` shows the epic line, and `children E<n>` lists an epic's tasks. Locking is unchanged from step 4: every write is a write session (lock held across the whole read-modify-write, one server-side transaction); reads, including `sessions` and `epic get`/`ls` and a `prune --dry-run`, take no lock, because a committed write is visible whole to the next read from any process. `tests/surreal.sh` proves it against sqlite: phase (h) a 118-line epic transcript (create, tasks under it, status moves, a dropped child, children counts, edit, close, every error path) identical, an explicit mixed-set check (`epic ls` `1/3 done`, `epic get` `tasks: 4 (1 blocking, 1 ready, 1 done, 1 dropped)`, `children E1` 4 lines: the dropped child is out of both the done count and the denominator, as in sqlite), and 8 processes x 5 `epic add` giving 40 distinct ids E1..E40; phase (i) a 26-line sessions transcript against stub `tmux`/`herdr` executables on `PATH` (one session alive via tmux, one via herdr, one gone, then `prune --dry-run`, `prune`, `unregister` twice, a duplicate `register`, an unknown task, and the prune refusal when both stubs fail) identical. Still stubbed after this step: `brief`, `tail`, `send`, `dispatch`, `wait`. `sessions` lists newest first and, for rows seen in the same millisecond, by name in byte order on both backends (`ORDER BY seen DESC, name`; SQLite's BINARY collation and SurrealDB's string order agree on `Zed alpha mid émile`). A sqlite unit test and a forced-tie check in `tests/surreal.sh` phase (i) hold that. The surreal check catches a wrong tiebreak (`name DESC` fails it) but not a missing one: SurrealDB scans the table in record-key order, which is already name order, so the explicit clause is redundant today and kept for determinism.
6. **`render.rs`, then `dispatch.rs`.** They come after the modules whose row types they read: `render.rs` (10 rusqlite hits) formats rows from tasks, epics and sessions through `Connection`; `dispatch.rs` reads session and task rows through `&rusqlite::Connection` (lines 57, 78) and calls `open()` (lines 125, 191), so it needs the ported session and task reads first. Neither is a pure pass-through. This is also the step that hoists the pure formatting helpers copied into `src/store/surreal/tasks.rs` (`code`, `ago`, `stamp`, `when`, the change-diff helpers, the list/get/children/events formatting) into one module shared by both backends, so the copies in the debt table go away. The same hoist covers the copies made in step 5: `normalize_path`, `path_note` and the epic row formatting in `src/store/surreal/epics.rs` (`brief` needs `path_note` and the epic counts), and `Liveness`, `herdr_agents`, `tmux_alive`, `tmux_up` and the herdr parsing in `src/store/surreal/sessions.rs` (`dispatch` needs `Liveness`), so the step and the debt row agree.
   **Status: done.** Hoisted first, as `src/store/text.rs` and `src/store/bridge.rs` (see Decision 2); the sqlite modules now import them too (`sqlite/render.rs` keeps only its queries, `sqlite/sessions.rs` and `sqlite/dispatch.rs` only registry SQL), and the copies in `src/store/surreal/` are gone. Ported: `brief` (`src/store/surreal/render.rs`) and `dispatch` (`src/store/surreal/dispatch.rs`); `tail` and `send` are the shared bridge functions for both backends. Dispatch reads (the task, the registry rows, the epic) in a read session; the `--force` reset (through `apply`) and the dispatch event are each a write session, the write lock held across the write. `tests/surreal.sh` proves it against sqlite: phase (j) a 131-line render transcript (brief with and without a window, `ls` by status and assignee and limit, `get` plain and `--full`, an over-long clipped title and description, an epic line, a blocked row, a done list) identical; phase (k) a 61-line dispatch transcript against stub `herdr`/`tmux` executables (`--print` leaves no event, the refusal on a non-ready task, `--force` reset, a herdr delivery, the tmux fallback line `via tmux (herdr: no such agent)`, the epic-path warning, no assignee, unknown and stale `--to`, a stale assignee row) identical. Only `wait` is still stubbed. The `done:` list of `brief` orders by `updated_at DESC` and then by id on both backends, so rows finished in the same millisecond list in a fixed order and the LIMIT of 8 keeps the lowest ids (a sqlite unit test and a forced-tie check in `tests/surreal.sh` phase (j): 10 tasks, `updated_at` forced equal, expected `T1..T8`). Removing the id tiebreak is not detectable on either backend: SQLite scans in rowid order and SurrealDB in record-key order, both already id order, so the explicit clause is redundant today and kept for determinism. A wrong tiebreak (`id DESC`) is caught on both: the sqlite unit test fails with `[11, 10, 9, 8, 7, 6, 5, 4]`, the surreal check lists `T10 T9 ... T3`.
7. **`wait.rs` last of the store modules.** It polls events and is the largest (523 lines); it also reads through `Connection` (7 hits).
8. **CLI adapter (`src/main.rs`, `src/cli.rs`).** `--db` help text and `path` output per feature.
9. **rmcp adapter (`src/server.rs`) last.** It has no rusqlite reference and already calls the sync surface through `spawn_blocking`, so it should need no logic change; run `tests/mcp_client.py` on both builds. Then make `cli.sh` and `dispatch.sh` runnable against the surreal build if wanted, and add the `tests/surreal.sh` row to AGENTS.md.
10. **Install.** `cargo install --path . --locked --no-default-features --features surreal` on this host; agents run `~/.cargo/bin/agentboard` (AGENTS.md).

### Temporary debt from the feature-split step

Things that exist only because the surreal backend is still a stub. Each goes away at the step named; none is permanent.

| Item | Where | Removed when |
|---|---|---|
| ~~`use surrealdb as _;`~~ | `src/store/surreal/mod.rs` | **Cleared at step 3:** the connect module calls the client crate, and the line is gone. |
| `#![cfg_attr(feature = "surreal", allow(dead_code))]` (the stub calls no shared helper: `clip.rs` caps, `now_ms`, `short`, `proc.rs`, and the unused fields of the shared types in `board.rs`) | `src/main.rs` | Step 7: the last port. It cannot go at step 4, because `proc.rs` is used only by the sessions and dispatch ports (steps 5 and 6) and `CAP_LAST` only by `wait.rs` (step 7). Remove it, then fix whatever warns. |
| The stub error text ("surreal backend is not implemented yet ...") and `src/store/stub.rs` | `SurrealBoard` in `src/store/surreal/mod.rs` delegates the unported operations to `StubBoard` | **Per operation.** Cleared at step 4: the task operations. Step 5: every `epic_*`, `sessions`, `prune`, `unregister`, `register_session`, and `children` of an epic. Step 6: `brief`, `dispatch`, `tail`, `send`. Still stubbed: `wait`. `stub.rs` goes when `wait` is ported, and `BACKEND` needs another stand-in for the two unsupported feature sets (or the guard noise returns). |
| ~~`cargo test --no-default-features --features surreal` runs 0 tests~~ | `src/store/surreal/` | **Cleared at step 3:** 22 tests today (see the Tests section). `tests/surreal.sh` adds the binary-level checks (step 4). |
| ~~Rendering helpers copied into `src/store/surreal/tasks.rs` (`code`, `ago`, `stamp`, `when`, the change-diff helpers, the list/get/children/events formatting)~~ | `src/store/text.rs` | **Cleared at step 6:** hoisted into `text.rs`; both backends call it. |
| ~~`normalize_path` / `path_note`, the epic row formatting, `Liveness`, `herdr_agents`, `tmux_alive`, `tmux_up` copied into `src/store/surreal/{epics,sessions}.rs`~~ | `src/store/text.rs`, `src/store/bridge.rs` | **Cleared at step 6:** hoisted; the sqlite modules import the shared copies. |
| Small constants still duplicated between the backends: `STATUS_LIST` (`sqlite/mod.rs`, `surreal/tasks.rs`), the epic `STATUS_LIST` and `norm_status` (`sqlite/epics.rs`, `surreal/epics.rs`), the `SELECT`/`ORDER BY` shapes of `ls` and `brief` | `src/store/sqlite/`, `src/store/surreal/` | Step 7: move the two constants and `norm_status` into `text.rs` with the `wait` port. The queries stay per backend by design; the transcripts in phases (a), (h), (i), (j), (k) are what keep them in step. |
| `AGENTBOARD_UNSAFE_NO_WRITE_LOCK=1` turns the write lock into a no-op | `src/store/surreal/lock.rs` | Permanent, **debug builds only** (`#[cfg(debug_assertions)]`): a release binary has no such string and ignores the variable, so a stray variable in an agent's environment cannot switch off the cross-process guard. `tests/surreal.sh` builds a debug surreal binary (kept in `target/surreal-debug`) for the mutation phase and proves in phase (f) that the release binary still waits on a held lock and prints `board write lock busy` with the variable set. |
| Every operation opens its own connection (probe, `/version`, WebSocket, sign-in, marker read), like the sqlite build opens its file | `src/store/surreal/mod.rs` (`session`) | After step 7, if latency matters: a connection cached for the life of the MCP server. Measured today: about 20 ms per CLI call on a warm server. |

---

## Unverified

One list; everything else above was measured as stated.

- Delivery to a real herdr pane and a real tmux session is untested on both backends. The dispatch, `send`, `tail` and session-liveness checks run against stub `herdr` and `tmux` executables on `PATH`: they prove the argv, the fallback text and the refusal text, not a live bridge.
- The manager's 4-process server run and embedded multi-process run were not rerun by the coder.
- Client crate features for `surrealdb 2.7.0` (`protocol-ws`), and that dropping `kv-surrealkv` from the client still builds.
- The lock: a holder killed with `kill -9` releasing it, a `SIGSTOP`ed holder and the bounded-wait error, per-operation lock latency, and one lock spanning a multi-statement store operation. Only a per-statement lock was measured.
- Several threads of one process contending through the lock and one connection (the rmcp server); the measured clients were separate processes.
- Run counts: the counter test ran once per variant (unguarded, CAS, lock), so the lock's 160/160 is one run; the claim test ran 10 times per variant. The mutation check in `tests/surreal.sh` assumes unguarded counter loss repeats; it was seen in 2 of 2 counter runs without the lock (unguarded 147, CAS 142 of 160).
- Server error text when pointed at a regular file; `/version` against a real server of another version (only the pure check is tested).
- The `events` ASSERT (exactly one of `task_id` / `epic_id`): the schema applies on the server, but no row was inserted to show it fires.
- A server started by hand or by another tool on the same directory (outside the start lock).
- `connect` holds the write lock only during bootstrap; the per-operation hold across store operations arrives with the ports. Per-operation lock latency is unmeasured.
- The surreal tests pass by skipping when no `surreal` binary exists and `AGENTBOARD_REQUIRE_SURREAL` is unset (see Tests).
- Lease timestamps stored as `int` ms.
- MVCC isolation beyond the claim and counter cases above.
- Crash and WAL behaviour: no `kill -9` of the server mid-write.
- Surreal build size and cold build time (65 s was an earlier session figure).
- Latency of a CLI call that starts the server.
