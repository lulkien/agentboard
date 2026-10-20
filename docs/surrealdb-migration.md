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
surreal = ["dep:surrealdb", "dep:tokio"]
```

`rusqlite` and `surrealdb` become optional dependencies. A build with neither or both features fails to compile, in `src/main.rs`:

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
| `src/store/sqlite/` (moved) | `#[cfg(feature = "sqlite")]`. Every module that calls rusqlite today moves here as SQLite code: `tasks.rs`, `epics.rs`, `sessions.rs`, `wait.rs`, `render.rs`, `dispatch.rs`, plus the `open`, `check_schema` and `INIT` logic now in `src/store/mod.rs`. |
| `src/store/surreal/` (new) | `#[cfg(feature = "surreal")]`. `mod.rs` (connect, schema bootstrap), `serve.rs` (auto-serve), `lock.rs` (write lock, Decision 6), and one port of each of `tasks.rs`, `epics.rs`, `sessions.rs`, `wait.rs`, `render.rs`, `dispatch.rs`. |
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
4. Two processes racing: both see a refusal and both spawn. Only one server wins the port; the loser's `surreal` exits with a bind error, and the loser's agentboard re-probes `/health`, which now answers, and connects. The loser's exit text lands in the same log.
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
- **Surreal unit tests:** `cargo test --no-default-features --features surreal`.
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
| Daemon lifecycle | A new long-lived process the tool did not have | agentboard starts it; nothing stops it. It outlives the CLI call. It does not survive a reboot; the next board call starts it again. Stopping is `pkill -f 'surreal db start'`, or a future `agentboard stop`. |
| Two starters racing | Duplicate spawn | Port bind is the arbiter; the loser exits and re-probes (Decision 4). No test yet. |
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
5. **`epics.rs`, then `sessions.rs`.** Smaller, independent queries; each uses `open()` today and must take the lock for its writes.
6. **`render.rs`, then `dispatch.rs`.** They come after the modules whose row types they read: `render.rs` (10 rusqlite hits) formats rows from tasks, epics and sessions through `Connection`; `dispatch.rs` reads session and task rows through `&rusqlite::Connection` (lines 57, 78) and calls `open()` (lines 125, 191), so it needs the ported session and task reads first. Neither is a pure pass-through.
7. **`wait.rs` last of the store modules.** It polls events and is the largest (523 lines); it also reads through `Connection` (7 hits).
8. **CLI adapter (`src/main.rs`, `src/cli.rs`).** `--db` help text and `path` output per feature.
9. **rmcp adapter (`src/server.rs`) last.** It has no rusqlite reference and already calls the sync surface through `spawn_blocking`, so it should need no logic change; run `tests/mcp_client.py` on both builds. Then make `cli.sh` and `dispatch.sh` runnable against the surreal build if wanted, and add the `tests/surreal.sh` row to AGENTS.md.
10. **Install.** `cargo install --path . --locked --no-default-features --features surreal` on this host; agents run `~/.cargo/bin/agentboard` (AGENTS.md).

---

## Unverified

One list; everything else above was measured as stated.

- The manager's 4-process server run and embedded multi-process run were not rerun by the coder.
- Client crate features for `surrealdb 2.7.0` (`protocol-ws`), and that dropping `kv-surrealkv` from the client still builds.
- The lock: a holder killed with `kill -9` releasing it, a `SIGSTOP`ed holder and the bounded-wait error, per-operation lock latency, and one lock spanning a multi-statement store operation. Only a per-statement lock was measured.
- Several threads of one process contending through the lock and one connection (the rmcp server); the measured clients were separate processes.
- Run counts: the counter test ran once per variant (unguarded, CAS, lock), so the lock's 160/160 is one run; the claim test ran 10 times per variant. The mutation check in `tests/surreal.sh` assumes unguarded counter loss repeats; it was seen in 2 of 2 counter runs without the lock (unguarded 147, CAS 142 of 160).
- Auto-serve end to end: detached spawn, bounded poll, two-starter race, missing-binary line.
- Server error text when pointed at a regular file; `/version` and marker-check behaviour against a stale server.
- Schema `DEFINE` statements on the server (executed on the embedded engine in an earlier spike only, output not retained).
- Lease timestamps stored as `int` ms.
- MVCC isolation beyond the claim and counter cases above.
- Crash and WAL behaviour: no `kill -9` of the server mid-write.
- Surreal build size and cold build time (65 s was an earlier session figure).
- Latency of a CLI call that starts the server.
