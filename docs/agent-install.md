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

## 3. Prove the MCP surface before wiring anything

Pipe a handshake into the binary. A scratch `AGENTBOARD_DB` keeps it off your board.

```sh
export AGENTBOARD_DB=$(mktemp -d)/probe.db
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"probe","version":"0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
| timeout 5 agentboard 2>/dev/null \
| python3 -c '
import sys, json
for line in sys.stdin:
    m = json.loads(line)
    if m.get("id") == 2:
        names = [t["name"] for t in m["result"]["tools"]]
        print(len(names)); print(" ".join(names))'
unset AGENTBOARD_DB
```

Expected: `19`, then

```
board_brief epic_add epic_edit epic_get epic_list epic_update session_list session_register session_send session_tail task_add task_ask task_children task_claim task_edit task_get task_list task_update task_wait
```

A smaller number means the installed binary is older than the checkout: go back to step 2.

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
./skills/install.sh [target-dir]       # default ~/.hermes/skills
./skills/install.sh --force            # refresh an existing install
```

Expected: `installed: agentboard-manager -> <target>/agentboard-manager`. An existing
copy is refused (`exists: <path> (use --force)`, exit 1) and nothing is touched.

- The repo copy is canonical. An installed copy is a snapshot: re-run with `--force`
  after pulling.
- `agentboard-manager` names two companion skills, `herdr-kit` and
  `subagent-team-setup`. They live outside this repo.
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
