# Installing agentboard for an agent

Instructions for an agent (or the human setting one up). Each step has a command
and the output that proves it worked. Run them in order from a clone of this repo.

## 1. Prerequisites

- A Rust toolchain with `cargo`. Nothing else: no server, and no `sqlite3` CLI
  (the binary bundles SQLite).

```sh
cargo --version        # prints a version
```

## 2. Build and install

```sh
cargo install --path . --locked
agentboard path
```

- The binary lands in `~/.cargo/bin/agentboard`. Make sure `~/.cargo/bin` is on `PATH`.
- `agentboard path` prints the board file (default `~/.local/state/agentboard/board.db`,
  or `$AGENTBOARD_DB`). The state directory is created on the first write.
- The installed binary is what agents run. `target/release/agentboard` from a
  checkout build is not. Reinstall after every change to the CLI or MCP surface.

### Owner cutover for an existing SurrealDB board

Reinstalling a build whose client generation is newer than the live board fails
closed on board access until this cutover is followed. The refusal names
`agentboard board upgrade --owner`; opening a board never upgrades it. Build the
new checkout first (`cargo build --release`). Use the same board path and server
environment throughout the cutover.

1. Stop every seat, its MCP process, and the SurrealDB server. Keep them stopped
   throughout maintenance. Preserve a backup of the stopped data directory.
2. As the board's owner, never as root or with sudo, run
   `./target/release/agentboard --db /absolute/board/data board upgrade --owner`
   with the board's `AGENTBOARD_SURREAL_URL`, user and password. The owner flag
   confirms ownership and that all seats and servers are stopped. Seats report a
   mismatch to the owner; they never run this command against a live board.
3. Reinstall with `cargo install --path . --locked`.
4. Restart the seats and their MCP servers using the installed build.
5. Run `python3 scripts/check-seat-surface.py` before dispatching work.

Upgrade supports SurrealDB generation 1 to 2 (and retrying an already-current
board). It refuses an occupied endpoint or another process holding board files,
including a server on another port. It holds the existing startup and write locks,
uses an unadvertised temporary loopback server, kills its process group on exit,
and gives it a parent-death signal so killing the upgrader also kills the server.
Run maintenance as the owner to avoid creating root-owned storage files that
later owner processes cannot update. Keep the storage private to the owner.

Agentboard-managed v2 SurrealDB daemons hold an exclusive `flock` on
`board.lock` for their entire process lifetime, including after the starting CLI
exits. A second managed daemon on that directory and an upgrade both refuse a
held lock and name its holder pid. Here “managed v2” means a daemon started by a
build containing this lifetime-lock change; earlier generation-2 builds are
legacy too and must be stopped/restarted. The upgrade retains the same flock
while its private server runs. Per-operation writers use a separate `board.write.lock`.
This lock belongs to agentboard; SurrealKV 2.7.0 itself permits two servers to
open the same directory and provides no such exclusion.

**First-upgrade limit:** v1 daemons do not hold the lifetime lock. Manually starting
raw `surreal db start` also bypasses agentboard's lock, regardless of board
schema generation. Stop every such daemon before cutover. For this transition,
the configured-port check, positive fd evidence and the owner's `--owner`
confirmation cover old/manual servers; do not claim the new flock proves their
absence. Once all starts go through agentboard v2, `board.lock` is the exact
cooperative server/upgrade exclusion gate.

The Linux `/proc` scan checks the owner's effective UID and reports what it can
observe. A listener on the configured port or an fd link resolving into the board
directory is positive evidence and refuses the upgrade. Unreadable processes,
including a same-UID non-dumpable `systemd`, print named warnings with pid, step,
error and “treating as not a holder”; they never refuse merely for being
unreadable. Other UIDs are outside the fd scan, and root may bypass storage
permissions. The explicit stopped-all-servers confirmation includes root/other-UID
and manual daemons. The scan reports; the managed-server lifetime flock decides.
Do not start manually managed servers during maintenance; they do not honor
agentboard's startup or lifetime locks.

Host-runnable scratch proofs (no root, PID namespace, or stopped user service):

```sh
python3 tests/board-upgrade.py /tmp/agentboard-t281-old target/release/agentboard
python3 tests/server-lock.py target/release/agentboard
```

The first binary is the pre-S2 build at `ea016707`; both proofs own random-port
scratch storage. The first verifies v1 preservation, interruption, warnings and
positive holder checks; the second verifies second-server refusal, locked-upgrade
refusal and progress after release using actual daemon processes and kernel flocks.

Only additive `IF NOT EXISTS` definitions and missing-default backfills run.
Definitions, backfills and `meta:schema`, `meta:opened`, `meta:writer` stamps commit
in one transaction. An interruption before commit rolls everything back; retry
finishes the upgrade, including if additive definitions already existed. After
commit, retry is idempotent. No original task, epic, event, diff or session is
removed or renumbered. Output names every checked definition and backfill, prints
before/after counts for every data table, and identifies the upgrading build.
The SurrealDB board stamp is the existing `meta` records, equivalent to SQLite's
`board_stamp` table; no second stamp format is introduced.

An upgrade is forward-only: rollback means restoring the stopped backup and the
old binary together, with every seat stopped. There is no in-place downgrade.
SQLite and boards older than generation 1 are outside this command's scope.
Pre-stamp SQLite binaries ignore stamps: retire them rather than assuming that
upgrading a stamp can fence them.

## 3. Assert the installed seat surface before wiring anything

From this checkout, after installing and before starting each seat:

```sh
~/.cargo/bin/agentboard board-build
python3 scripts/check-seat-surface.py
```

Expected identity: `agentboard <version> tools=<n> commit=<checkout revision> backend=surreal`
(or `backend=sqlite` for that build). Expected assertion:
`seat surface ok: tools=<n> commit=<checkout revision>`.
`<n>` is the tool count the command derives from `src/server.rs`: read it from the
output, never type it.
The revision is exactly `git describe --always --dirty --abbrev=8`, including `-dirty`
when tracked files are modified. The assertion compares both revision and the
source-derived tool count; a rebuilt but unreinstalled binary is refused.
It accepts an explicit binary path for checking a checkout build:
`python3 scripts/check-seat-surface.py ./target/release/agentboard`.
Neither command opens or writes a board.

A suspiciously fast install can mean Cargo reused an older artifact; elapsed time
alone is not proof. An embedded revision that differs from its build checkout is
shown as `commit=<embedded> (stale, HEAD is <checkout revision>)` in both
`board-build` and the MCP initialize title. An unavailable build checkout is marked
`(unverified, build checkout unavailable)`; the assertion refuses both cases.
The embedded revision is preserved, never replaced with a claim of a newer build.
If refused, force a rebuild (`touch build.rs`), reinstall with
`cargo install --path . --locked`, rerun the assertion, then restart the seats.
Cargo's `.git/HEAD` rerun hint cannot update an already installed/running artifact,
and symbolic HEAD can stay unchanged while the referenced branch moves.

## 4. Register the server with a client

Give every agent its own actor name. The assignee on the board is the MCP actor,
so two agents sharing one actor cannot be told apart in the event log. Use an
absolute path for `command`; replace `<you>` with your home directory and
`<actor>` with a unique name.

### Claude Code

One config file per actor, `~/.claude/<actor>-mcp.json`:

```json
{
  "mcpServers": {
    "agentboard": {
      "type": "stdio",
      "command": "/home/<you>/.cargo/bin/agentboard",
      "args": [],
      "env": {
        "AGENTBOARD_DB": "/home/<you>/.local/state/agentboard/board.db",
        "AGENTBOARD_ACTOR": "<actor>"
      }
    }
  }
}
```

Launch with `claude --strict-mcp-config --mcp-config=$HOME/.claude/<actor>-mcp.json`.

### opencode

In `~/.config/opencode/opencode.json`, under `mcp.servers`:

```json
"agentboard": {
  "type": "local",
  "command": ["/home/<you>/.cargo/bin/agentboard"],
  "environment": {
    "AGENTBOARD_ACTOR": "<actor>",
    "AGENTBOARD_DB": "/home/<you>/.local/state/agentboard/board.db"
  }
}
```

### Hermes

In `~/.hermes/config.yaml`, under `mcp_servers`:

```yaml
mcp_servers:
  agentboard:
    command: /home/<you>/.cargo/bin/agentboard
    env:
      AGENTBOARD_ACTOR: <actor>
    enabled: true
```

`AGENTBOARD_DB` is optional everywhere: without it the default board from step 2 is
used. Set it identically for every agent that should share a board.

## 5. Install the skills

```sh
./skills/install.sh [canonical-dir]    # default ~/.agents/skills
./skills/install.sh --force            # replace an existing install
```

Expected: twelve `linked:` lines: two canonical links in `~/.agents/skills`,
eight aliases in the four agents' skills directories, and the existing Claude and
opencode command links. Every agent alias points through the canonical name;
only the canonical names point to the repo. Existing destinations are all refused
without `--force` (exit 1, no changes). See [the skill layout](../skills/README.md).

- There is one physical repo directory per skill, so edits are visible immediately
  through all four agents without reinstalling. No installed copies can drift.
- `agentboard-manager` lists `claude-code` and `opencode` as related skills. They live
  outside this repo.
- `skills/agentboard-manager` is the playbook for running a team with this tool.
  [`AGENTS.md`](../AGENTS.md) holds the rules for changing the code.

## 6. End-to-end check

Use a scratch board so the check does not pollute a real one.

```sh
export AGENTBOARD_DB=$(mktemp -d)/smoke.db AGENTBOARD_ACTOR=smoke
agentboard brief
agentboard add smoke --description hello --goal g
agentboard claim T1
agentboard wait T1 --status done --timeout 2; echo "exit=$?"
agentboard done T1 "smoke ok"
agentboard wait T1 --status done --timeout 5; echo "exit=$?"
unset AGENTBOARD_DB AGENTBOARD_ACTOR
```

Expected, in order (`...` is the `last (T1):` tail, which varies):

```
0 done/120m
T1 ready
T1 working (lease 30m) smoke
description:
hello
goal:
g

timeout: nothing matched after 2s (1 watched)
...
exit=3
T1 done #3
matched: 1 of 1 watched
T1 done smoke
last: #3 smoke done: smoke ok
exit=0
```

`claim` echoes title, description and goal verbatim. `wait` exits 3 with a first
line of `timeout:` when nothing moves, and 0 with `matched:` when it does.

## 7. Troubleshooting

- **`this board file predates epics`**: the file was written by an older version.
  There is no migration. Move it aside as the message says and start fresh.
- **`command not found: agentboard`** after `cargo install`: add `~/.cargo/bin` to `PATH`.
- **A verb works in the checkout but not for agents**: they run the installed
  binary. Run `cargo install --path . --locked` again and restart the agents (an MCP
  server is a process of the binary it started from).
- **A task shows an assignee nobody answers to**: the assignee is the MCP actor.
  Check `AGENTBOARD_ACTOR` in that client's config; each agent needs its own.
- **Stale session rows**: `agentboard sessions prune --dry-run` lists dead rows,
  `agentboard sessions prune` deletes them.
