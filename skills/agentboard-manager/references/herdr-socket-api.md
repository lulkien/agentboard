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
  The order selects among candidates that exist — it does not overrule a caller:
  an env var that is set but names a missing path is a refusal naming that
  variable and path, never a fall-through to the next candidate, or a manager
  pointed at a staging socket silently drives the live one.
- Leave `herdr-client.sock` alone: that is the TUI client's own socket, not the
  control surface.
- Framing: **NDJSON** both directions. Request `{"id":"…","method":"…","params":{…}}`,
  one line. Success `{"id":"…","result":{…}}`, failure
  `{"id":"…","error":{"code":"…","message":"…"}}`, also one line each.
  Read exactly one response line per request, then close; do not assume the
  server keeps a chatty session open for you.
- **The server closes the connection after one reply.** Measured at 0.9.0: a
  second write on the same connection returns `EPIPE`, and a second request
  pipelined before the first reply is read returns `ECONNRESET`. So "send the
  handshake request and then the real method on one connection" is not a shape
  this server permits — pin on a fresh connection that ends immediately, then
  open the connection that carries the request. That also means the pin has to
  live per request, not once per process: with one request per connection, a
  process that pings only on its first connection sends every later request to
  a server it never verified (see the pin pitfall below).
- Every call needs a deadline (connect + read). A wedged server must not hang
  the caller that is supposed to be supervising agents.

## Subscriptions push on a connection you hold open

`events.subscribe` is the exception to "one request, one reply, then the server
closes": after the ack the connection **stays open** and the server pushes
`{"type":"subscription_event","event":…,"data":…}` lines unprompted. Measured at
0.9.0 / protocol 22: subscribe, ack, then 25 s with no bytes in either direction and
no client write, and the transition still arrived as a line when it happened.

So a blocking wait can be a real subscription instead of a poll: one subscribe, read
lines off that connection, never write again. Three traps:

- a reader that closes after the first line (the rule for ordinary methods) never
  sees an event — the ack is not the answer;
- the only client-side timer may be a read slice with a timeout (200 ms is plenty)
  so a cancel flag or a deadline is observed; a slice that *sends* anything turns the
  subscription back into a poll;
- pin the protocol on the subscribe connection too, before subscribing.

Judge an implementation by which side owns the interval: a fresh connection on a
timer asking "anything new" is a poll however small the timer, while one held-open
connection makes the latency herdr's. Write the claim down as what it is (`wakes on a
herdr-pushed transition; no client poll interval`) — a doc promising push while the
code polls is the defect, and it survives review because nobody measures silence.

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

`agent.start` never creates layout: it needs a pane whose shell is ready, and
the refusal is a **window, not a rule about the pane's position**. A pane — the
id `tab.create` returned or one `pane.split` just made — answers
`agent_pane_busy: agent target pane <id> is not an available shell` until its
shell is up, and then accepts the start. Live rounds on 0.9.0 confirmed both
halves: the first start right after `pane.split` was refused, the same split
tab root accepted the start seconds later. So retry on `agent_pane_busy` inside
one small named budget (never the caller's start timeout — that error comes back
immediately), and do not conclude that roots are unusable or that splitting
first is mandatory. Read `pane.get` / `session.snapshot` when the order matters,
and keep the launcher's rollback (close the panes it created) for the case where
the window never opens. Agent targets accept a live **name** or its pane id,
never a terminal id.

A seat can also start cleanly and still never reach a prompt: a `claude`
launched in a directory it has not been trusted for comes up
`agent_status: blocked` with `launch_pending: true` and stays there. Anything
that waits for "back at a prompt" must read those fields (plus
`interactive_ready`) and name the state and what the pane shows instead of
reporting a bare timeout — and a launcher should either make the seat
non-interactive for a fresh cwd or refuse and quote the dialog.

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
- **The deadline has to cover `connect`, and only a full accept queue proves it.**
  Binding `read`/`write` alone leaves the wedge: a socket file that still exists
  while the server stops accepting (backlog full) blocks `connect` forever, so
  nothing the caller times out on ever runs. Put the connect inside the same
  budget (thread + `recv_timeout`, or non-blocking connect polled to the
  deadline). The test that catches this needs `UnixListener` with a small backlog
  and one queued connection that is never accepted — a stub that calls `accept()`
  first and then sleeps only ever exercises the read path, which is why the
  obvious hang test passes on unbounded code.
- **Pin per connection, not per process.** With one request per connection, a
  process that pings only on its first connection sends every later request to a
  server it never verified, so a herdr restart or upgrade between two calls goes
  unnoticed. Open every connection with `ping`, refuse a non-matching protocol
  before any other method goes out, and write the test as *n* connections → *n*
  pings; counting one ping per process hides the gap.
