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
  or `$AGENTBOARD_DB`). The state directory is created on the first call.
- The installed binary is what agents run. `target/release/agentboard` from a
  checkout build is not. Reinstall after every change to the CLI or MCP surface.

### Schema compatibility

Every CLI invocation and MCP tool call checks the board's schema version.
Version 1 migrates additively to version 2 in the same call. Definitions,
missing-default backfills and the version bump commit together under the board
write lock. The caller prints one migration line on stderr. Concurrent callers
wait and recheck, so only one migrates. Newer boards refuse an older binary by
name. Type conflicts and unsupported transitions name a manual step.
See [the schema protocol](../PROTOCOL.md#board-schema-version).

The owner-only offline `board upgrade --owner` maintenance command remains
available; ordinary additive transitions do not require it. Never run two raw
SurrealDB servers against the same storage directory. Managed daemons hold
`board.lock` for their lifetime; store operations use `board.write.lock`.
The offline command retains its stopped-server ownership and exclusion checks.

Rollback is not an automatic downgrade. Use a binary that understands the
resulting schema, or restore the stopped backup with its old binary.

## 3. Restart MCP processes when needed

The manager reloads each MCP process to use an installed binary. There is no
seat-start check, commit comparison or catalog refusal. `board-build` remains
informational and prints version, derived tool count, commit and backend without
stale or unverified annotations. Its removal is separate from schema migration.

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

- **`no schema version`**: an unstamped legacy board requires the named manual
  step: inspect and stamp the legacy board offline before using it.
- **`command not found: agentboard`** after `cargo install`: add `~/.cargo/bin` to `PATH`.
- **A verb works in the checkout but not for agents**: they run the installed
  binary. Run `cargo install --path . --locked` again and restart the agents (an MCP
  server is a process of the binary it started from).
- **A task shows an assignee nobody answers to**: the assignee is the MCP actor.
  Check `AGENTBOARD_ACTOR` in that client's config; each agent needs its own.
- **Stale session rows**: `agentboard sessions prune --dry-run` lists dead rows,
  `agentboard sessions prune` deletes them.
