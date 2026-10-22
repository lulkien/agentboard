# Reproducing a worker's "cannot be done" claim

A worker that reports a blocker has usually only proved that *its* probe failed. The
blocker is worth a manager's five minutes because the cost of accepting a false one is a
design built on a nonexistent limit, or a whole plan narrowed to the wrong library.

## When this applies

- A design or spike task comes back with decisions marked *unverified*, *blocked* or
  "the engine/crate cannot X".
- A worker's fix says a test cannot be written because the harness will not allow it.
- A reviewer rejects a finding as impossible.

## Procedure

1. **Isolate the claim.** Restate it as a single falsifiable sentence: "a record created
   in one connection is not visible after reopening", "field names of one letter break
   the deserializer", "crate X does not compile on this toolchain".
2. **Probe in `$TMPDIR`, never in the repo.** A throwaway crate with the same dependency
   line and features as the real manifest. Nothing from a probe belongs in the tree, and
   nothing in the tree should be edited to run one.
3. **Build in the background.** A first `cargo build`/`cargo run --release` of a database
   or engine crate routinely passes a minute; run it
   `terminal(background=true, notify=true)` and read the tail, instead of letting a
   foreground timeout read as a failure.
4. **Probe the failure, not the happy path.** For a persistence claim: write, close the
   handle, reopen, read back. For a concurrency claim: N actors, M attempts, assert no
   item twice. For a compile claim: build the exact version the worker named and keep
   the error.
5. **Record the smallest output that could have failed.** A `SELECT` returning the row,
   two distinct integers, `N/N claimed once`. A probe that prints `ok` proves nothing.
6. **Rule in writing**: *verified* (with the output), *disproved* (with the output that
   should have failed), or *unverified with the version* the worker used. "Probably
   fine" is not one of the three.

## Traps that make a probe — or a worker's spike — lie

- **Namespace/database not selected.** Embedded engines default to a namespace of their
  own; a write in one connection and a read in another that picked a different
  namespace looks exactly like "CREATE does not persist".
- **Statement not executed.** A query builder returns a handle; if nothing awaited it,
  the write never ran and the error surface is empty.
- **Engine/session lifetime.** Reopening the same path is the test for durability; a
  read on the same live handle proves only that memory works.
- **Version drift.** `cargo add X` resolving a version the worker did not name, or a
  transitive crate (a vector index, a codec) failing on this toolchain, gets reported as
  "X is broken". Pin and paste the version in the claim.
- **Missing capability stated as bug.** A feature absent from a major version (a sequence
  type, a CLI verb) reads as a parse error; grep the released docs before calling it a
  defect.
- **Scratch leaking into the repo.** Stub binaries and logs written next to the checkout
  (`hbin/`, a `log` file) are untracked noise a commit can swallow. `git status
  --porcelain` on every wake-up, and file the cleanup as part of the round.

## Multi-process safety probe (embedded engines)

An embedded database is not automatically what SQLite is. SQLite coordinates through
POSIX file locks and re-reads pages under them, so separate processes share one truth;
most embedded KV engines (SurrealKV, RocksDB, sled, redb) do not. A migration designed
against a single handle can pass every single-process test and still hand the same task
to two agents.

Count the openers first: the CLI, one MCP/sidecar process per agent, and the test suite
spawning N workers with a fresh binary each. That number is the requirement.

Two probes settle it, both in `$TMPDIR` against a throwaway crate:

1. **Atomicity across processes.** N OS processes, each running the same guarded write
   (`UPDATE ... WHERE status='ready' RETURN AFTER`) K times over M rows, printing the ids
   each one won. More distinct winners than M is duplication. Important: a row that
   refuses to deserialize into a plain JSON value reads as "nothing claimed" — decode
   into a typed struct before concluding the guard works.
2. **Read coherence.** One process opens the store and ticks a count every few seconds
   while another commits writes into the same directory. A count that never moves means
   each process serves a stale snapshot. No application lock repairs that: holding a
   flock does not refresh another process's cache, so a lock plus guarded writes still
   races on old state.

Also record **open cost** in ms. If the engine has no cross-process coordination, the
only safe shape is reopen-per-operation, and that latency multiplies into every call
against SQLite's few milliseconds.

Interpretation:

| Probe result | Meaning |
|---|---|
| second process refuses to open | exclusive directory lock: single-process only |
| opens, duplicate winners, stale reads | no coordination at all: unusable for a multi-process board |
| opens, no duplicates, coherent reads | genuinely multi-process: proceed |

The honest options after a failure are a local server process owning the store, or no
migration. A design that answers this with "may force a different engine or a single
owning process" has deferred the decision, not made one — send it back.

## Server mode: reproduce the fix, not just the failure

When the answer is a local daemon owning the store, prove the property against the
daemon before the design is accepted, because "a server will fix it" is also a claim —
and one clean run is not a measurement.

Measured shape for SurrealDB 2.7 on a multi-process tool: the daemon fixes *coherence*
(every ws client sees one store, unlike an embedded engine's per-process cache) but it
does **not** make a guarded write atomic against concurrent connections. One 4-process
run happened to be clean (8 claims for 8 rows, one process `[1..8]`, three empty); a
10-run repeat of 8 processes x 10 guarded attempts over 5 rows duplicated in 2 of 10
runs, and 8 processes x 20 counter increments lost 13 of 160 (final 147). Retrying on
the engine's write-conflict error did not fix it, and a version/compare-and-set retry
still duplicated in 2 of 10 runs.

What did fix it: a **client-side flock held around each store operation**, on top of the
daemon — 10/10 claim runs clean and 160/160 counter values, zero conflicts. Same
measurement against SQLite: 10/10 clean, so the flock is buying back what SQLite's file
locking already does. Treat "the server serialises writers" as false until a repeat run
says otherwise, and put the lock in the design plus a mutation check (disable the lock,
the contention test must fail).

The same rule applies to a *single* handle: concurrent tasks inside one process on one
engine handle lost updates and handed one row to two callers, so an in-process mutex is
part of any sane design, not a substitute for the cross-process lock.

Start it explicitly and keep the exact command in the reference:

```bash
surreal db start --no-banner --log warn --bind 127.0.0.1:18720 \
  --username root --password root surrealkv:///abs/data/dir
curl -sS -o /dev/null -w '%{http_code}' http://127.0.0.1:18720/health   # 200 when ready
```

Traps priced in this session:

- **The client URL takes host:port, not a scheme.** `Surreal::new::<Ws>("ws://127.0.0.1:PORT")`
  fails with `failed to lookup address information`; `"127.0.0.1:PORT"` connects.
- **The CLI moved between releases.** `--kv-surrealkv` as a flag is gone; the engine comes
  from the path scheme (`surrealkv://<dir>`), and `surreal start` is now `surreal db
  start` with `--username/--password`. Read `<bin> db start --help` before writing the
  spawn line into a design.
- **A version that is pinned cannot be bumped.** A transitive vector-index crate pinned
  by the engine's own index crate failed to compile (`E0277 ... Batch`); `cargo update -p
  <index> --precise <newer>` is refused because the parent pins `^old`. Either the engine
  version that ships a fixed pin is used, or the newer engine is skipped — say which in
  the doc instead of retrying.

- **Two starters on one store directory corrupt it.** A store engine opens the data dir
  before it fails to bind the port, so "the loser exits with a bind error" is false: two
  spawns on a fresh dir produced `Invalid revision N for type DefineTableStatement` and a
  lost row after restart. Arbitrate starts with a lock file held across probe → spawn →
  poll (`<state>/<engine>.start.lock`), have the waiter re-probe and connect to the
  winner, and prove it by deleting the start lock: two spawns and corruption must come
  back. One clean race run is not proof; run it three times.
- **`/health` answering is not "ready".** A server can answer `/health` and a version
  probe and then never complete the client's connect/upgrade, which blocked a verb past
  25 s with every earlier timeout respected. Put a deadline on the connect itself and
  state the whole budget (probe + start-lock wait + poll + connect) in the design; test a
  server that spawns and never answers, and assert the test finishes under a few seconds.

On the implementation side, make the backend a **compile-time** switch when the user asks
for one build per host: mutually exclusive cargo features (`default = ["sqlite"]`,
`surreal = ["dep:surrealdb"]`) with a `compile_error!` for neither-or-both, so a wrong
build fails loudly and the surreal binary carries no sqlite dependency. Recipe that held
up:

- Optional deps only (`rusqlite`, `surrealdb`), each behind `dep:<crate>` in its feature.
  `dep:tokio` is invalid when tokio is already a non-optional dependency — drop it rather
  than making tokio optional for no reason.
- A std API newer than the manifest's `rust-version` (file locks are 1.89) fails clippy as
  `incompatible_msrv`. Bump `rust-version` and drop the `#[allow]` unless the crate must
  still build on the older toolchain — an allow hides the real minimum from every future
  reader.
- Two `compile_error!` arms in `main.rs`: `all(feature="a", feature="b")` and
  `not(any(...))`, each naming the two valid build commands. The guard line prints first,
  followed by a wall of `E0425` noise from the missing backend alias; that is cosmetic
  and only worth a `cfg` alias if someone complains.
- One trait in `store/board.rs`, the old modules moved wholesale to `store/sqlite/`, and a
  thin free-function forwarder layer so `main.rs`/`server.rs` compile unchanged in the
  split commit. Prove behaviour with an old-vs-new binary diff over a batch of verbs
  (normalise relative ages) instead of asserting "no behaviour change".
- Post-split checks that catch leaks: `rg rusqlite src/ | rg -v 'src/store/sqlite/'` empty,
  `cargo tree` per feature showing the other engine absent, and clippy `-D warnings` run
  once per feature set after `cargo clean -p <crate>`.
- The other engine's stub returns one clear error string and exits non-zero; a stub that
  panics makes every later probe look like a crash rather than an unimplemented path.

## What to do with the result

- Claim disproved: file the correction as a review round on the doc or spec that stated
  it, with the probe output in the review row's description, and require the worker to
  re-run the probe and paste real output. Do not silently rewrite the design yourself.
- Claim verified: accept it, and put the output in the next row so nobody re-litigates
  it.
- Probe itself inconclusive: say so to the user in one line and stop widening; a manager
  guessing at engine behaviour is how a migration gets filed against the wrong crate.
