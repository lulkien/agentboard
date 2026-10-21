# herdr's own socket API (measured, not documented)

The `herdr` CLI is a client of a server socket. Anything that wants to drive
herdr from inside another tool (a manager command, a team launcher, an event
bridge) can talk that socket directly or shell the CLI. The facts below were
measured with read-only probes; re-measure before relying on them after a herdr
upgrade.

## Transport

- Socket: `$XDG_CONFIG_HOME/herdr/herdr.sock` (default
  `~/.config/herdr/herdr.sock`), unix **stream**, mode `0600` (same user only).
  Discovery order worth implementing: `$AGENTBOARD_HERDR_SOCK` → `$HERDR_SOCKET`
  → `$XDG_CONFIG_HOME/herdr/herdr.sock` → `~/.config/herdr/herdr.sock`; absent
  socket = one line naming the paths tried, never a silent fallback to the CLI.
- Leave `herdr-client.sock` alone: that is the TUI client's own socket, not the
  control surface.
- Framing: **NDJSON** both directions. Request `{"id":"…","method":"…","params":{…}}`,
  one line. Success `{"id":"…","result":{…}}`, failure
  `{"id":"…","error":{"code":"…","message":"…"}}`, also one line each.
  Read exactly one response line per request, then close; do not assume the
  server keeps a chatty session open for you.
- Every call needs a deadline (connect + read). A wedged server must not hang
  the caller that is supposed to be supervising agents.

## Handshake and version pinning

`ping` with empty params answers with the version and capability set:

```json
{"type":"pong","version":"0.9.0","protocol":22,
 "capabilities":{"live_handoff":true,"detached_server_daemon":true,
                 "endpoint_protocol_generation":1,"surface_interest":true,
                 "health_check":true}}
```

Pin the protocol integer and refuse anything else by name ("herdr protocol 23,
want 22") the way the store pins its database version. A protocol bump that is
silently mis-parsed is worse than a refusal.

## Schema: the method inventory

`herdr api schema --output <path>` writes a JSON Schema bundle:
`schema_version`, `protocol`, and five named schemas (`request`,
`success_response`, `error_response`, `event`, `subscription_event`). The
`request` schema is a `oneOf` over methods; each variant's
`properties.method.const` is the method name and `properties.params.$ref`
points at its params type in `$defs`.

Measured at protocol 22 (herdr 0.9.0): **102 methods in 19 groups**, a group
being the part of the method name before the first dot — `pane` (36), `agent`
(12), `plugin` (11), `workspace` (9), `tab` (7), `server` (5), `worktree` (4),
`integration` (3), `layout` (3), `events` (2), `client` (2), plus eight
one-method groups: `command`, `client_shell`, `notification`, `popup`,
`session`, `ping`, `product_announcement`, `release_notes`.

The handful a manager actually uses. Required params are listed bare; optional ones
are in brackets. Both come from the `required` array of each method's params type in
the schema, not from memory.

| Need | Method | Params |
|---|---|---|
| liveness + version | `ping` | none |
| whole session in one call | `session.snapshot` | none |
| who is live and in what state | `agent.list` | none |
| one agent's record | `agent.get` | `target` |
| read pane output | `agent.read` | `target`, `source` [`lines`, `format`, `strip_ansi`] |
| block on state | `agent.wait` | `target` [`until[]`, `timeout_ms`] |
| submit a turn | `agent.prompt` | `target`, `text` [`wait`] |
| keystrokes | `agent.send_keys` | `target`, `keys` |
| start an agent in an existing shell pane | `agent.start` | `name`, `kind`, `pane_id` [`args[]`, `timeout_ms`: greater than 3000 and at most 300000] |
| rename an agent | `agent.rename` | `target` [`name`] |
| list tabs | `tab.list` | none [`workspace_id`] |
| create a tab | `tab.create` | none [`cwd`, `label`, `focus`, `env`, `workspace_id`] |
| one tab's record | `tab.get` | `tab_id` |
| split | `pane.split` | `direction` [`target_pane_id`, `cwd`, `env`, `ratio`, `focus`, `workspace_id`, `right_click`] |
| close | `pane.close` / `tab.close` | `pane_id` / `tab_id` |
| capture a tab | `layout.export` | none [`tab_id`, `pane_id`] |
| restore a tab | `layout.apply` | `root` [`tab_id`, `tab_label`, `workspace_id`, `focus`] |
| subscribe to transitions | `events.subscribe` | `subscriptions` |
| block on one event | `events.wait` | `match_event` [`timeout_ms`] |
| notifications | `notification.show` | `title` [`body`, `position`, `sound`] |

`agent.start` never creates layout: it needs an existing shell pane, so a team
launcher is `tab.create` → `pane.split` → `agent.start` ×N, in that order.
Agent targets accept a live **name** or its pane id, never a terminal id.

## Shape of the answers (for typing your own structs)

`agent.list` → `{"type":"agent_list","agents":[{terminal_id, agent,
agent_status, workspace_id, tab_id, pane_id, focused, state_change_seq, cwd,
foreground_cwd, …}]}`. `agent_status` is one of `idle`, `working`, `blocked`,
`done`, `unknown`; `idle` and `done` both mean "ready for input" and only differ
in the server's seen-state, so never treat `done` as "finished".
`tab.list` / `pane.list` carry the same ids plus `label`, `pane_count`,
`agent_status`, `scroll` metadata. `session.snapshot` wraps the lot with the
focused workspace/tab/pane ids.

**Names are per live pane.** A name follows the current occupant and is cleared
when that agent exits, is released, or is replaced, so a registry row keyed by
name goes stale on restart and must be reconciled (`prune`, re-register), never
assumed.

## Probing it read-only

This is the whole probe; it touches nothing:

```python
import socket, json, os
p = os.path.expanduser('~/.config/herdr/herdr.sock')
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.settimeout(5)
s.connect(p)
s.sendall((json.dumps({"id":"probe","method":"agent.list","params":{}})+"\n").encode())
buf = b''
while b"\n" not in buf: buf += s.recv(65536)
print(json.loads(buf.split(b"\n")[0])["result"])
```

Swap `method` for `ping`, `session.snapshot`, `tab.list`, `pane.list` — all four
are safe on a live desktop session. Treat anything that writes (`agent.start`,
`pane.split`, `pane.close`, `tab.create`, `agent.prompt`) as a side effect on
someone's workspace: test it against a scratch tab you opened, and never from
the manager's own pane.

## What agentboard builds on this (decided shape, mid-implementation)

The owner picked the interface; a row that invents another layer is out of scope.

- **Grammar: `agentboard manager [--herdr|--tmux] <verb>`.** `--herdr` is the
  default and may be omitted; `--tmux` is parsed and **refused by name** (`no
  tmux backend yet; only --herdr is implemented`) so it can become a real
  backend later instead of a flag that quietly did nothing.
- **No MCP.** herdr management is CLI-only — the manager runs the binary.
  Workers keep their board MCP and must not manage panes, so
  `rg -c '#\[tool\(' src/server.rs` stays at its pre-epic count.
- **Four rules belong in the client library, not in each verb**, so a new verb
  cannot forget them: socket discovery order (env, env, XDG, home; refuse with
  the paths tried), protocol pin 22 on a first `ping`, deadline
  `$AGENTBOARD_HERDR_TIMEOUT_MS` (default 2000) with the caller's timeout for
  long-poll methods, and read-`agent_status`-then-refuse for anything that
  stops work (`refusing to stop coder: status=working`, `--force` overrides).
- **Verbs.** Read-only: `status` (board sessions joined with live truth — a dead
  pane prints `gone`, a live agent with no registry row prints `unregistered`),
  `snapshot`, `agent list|get|read`, `tab list`, `pane list|get|read`. Mutating:
  `tab new`, `pane split`, `agent start <name> --kind K --pane P [-- ARGV…]`
  (argv passthrough is the point — opencode needs `--standalone`, claude needs
  its `--mcp-config` line), `agent prompt|keys|rename|focus|wait`, `pane close`.
  Composites: `up --repo P [--coder claude] [--reviewer opencode]` (tab →
  split → start ×2 → register both by NAME), `down [--force]`,
  `renew [--names coder,reviewer]`.
- **Tests.** A stub herdr server on a temp unix socket speaking scripted NDJSON
  proves framing, refusals, deadlines, the request log of each mutating verb and
  the working-guard without touching the live session; one guarded live round
  (`AGENTBOARD_HERDR_LIVE=1`) opens a scratch tab, proves up/status/renew/down
  and closes it, and may only touch pane ids it created.

## Pitfalls

- **`herdr api schema` takes `--json` (stdout) or `--output <path>`, never both**
  (together they exit 2). At herdr 0.9.0 / protocol 22 the stdout form parses:
  `herdr api schema --json | python3 -c 'import json,sys; json.load(sys.stdin)'`
  exits 0. An earlier version of this note called it unparseable; that was a
  failure of the capture path, not of herdr, so when a parse fails check how the
  output was captured before blaming the tool.
- **One response line per request.** A reader that loops until EOF hangs on a
  server that keeps the connection open after answering; stop at the first
  newline.
- **Errors are in-band.** An `error` response is a normal line, not a closed
  socket, so check for `error` before touching `result` and surface
  `error.code` + `error.message` as one line.
- **Long-poll methods hold the connection** (`agent.wait`, `events.wait`,
  `pane.wait_for_output`). They are the one place a caller's own timeout, not a
  fixed 2 s deadline, must govern.
- **A stale registry and a live pane can disagree silently.** Read state from
  `agent.list`, not from whatever the tool recorded last: the daemon's view is
the only one that knows an agent died.
