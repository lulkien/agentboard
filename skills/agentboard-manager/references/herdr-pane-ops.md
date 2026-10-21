# herdr pane ops, verified shapes

The manager drives herdr by id and by agent name. Every command answers in a JSON
envelope; there is no tmux here. The binary is the authority for flags —
`herdr --skill`, `herdr agent --help`, `herdr pane --help`.

## A second team on its own tab

**Starting a seat needs a pane whose shell is ready, and that is a startup window,
not a rule about position.** `tab create` -> `pane split` -> `agent start --pane <id>`
answers `agent_pane_busy: agent target pane <id> is not an available shell` for the
first moments after the pane exists, then accepts the same target — a split tab root
accepted the start seconds later in a live round at 0.9.0. So retry on that exact
error inside one small named budget (the refusal comes back immediately, so never
retry with the caller's start timeout) and read the state with `herdr pane get --pane
<id>` / `session.snapshot` when the order matters. A stub answering scripted NDJSON
cannot model this rule, so it will happily pass a wrong order forever; only a live
round shows it. `manager up` closes the panes it created when a start never settles,
so a refused `up` leaves no half-team behind — keep that rollback.

The mechanics that ARE verified, from the manager's own shell (no `HERDR_PANE_ID` needed,
because nothing here is `--current`; the `agent start` lines assume a target whose shell
is up):

```bash
j() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }
created=$(herdr tab create --cwd "$repo" --label "$(basename "$repo")")
c1=$(printf '%s' "$created" | j "d['result']['root_pane']['pane_id']")
c2=$(herdr pane split --pane "$c1" --direction right --cwd "$repo" --no-focus \
     | j "d['result']['pane']['pane_id']")
herdr agent start coder2 --kind claude --pane "$c1" --timeout 120000 -- \
  --strict-mcp-config --mcp-config="$HOME/.claude/coding-mcp.json" \
  --append-system-prompt-file="$HOME/.claude/caveman-full.md" --model sonnet
herdr agent start reviewer2 --kind claude --pane "$c2" --timeout 120000 -- \
  --strict-mcp-config --mcp-config="$HOME/.claude/reviewer-mcp.json" \
  --append-system-prompt-file="$HOME/.claude/caveman-full.md" --model sonnet
agentboard register coder2 --agent claude --task T<n>
agentboard register reviewer2 --agent claude
```

- `tab create` -> `result.root_pane.pane_id`; `pane split` -> `result.pane.pane_id`.
- The coder needs `--strict-mcp-config` plus its config file: that is what gives it
  `task_claim`/`task_get` and its actor identity. Without the config it works and never
  claims.
- The reviewer uses the same shape with `reviewer-mcp.json`, which is what makes its board
  actor `reviewer` rather than `claude1` — start it with the coder's config and every
  review write is attributed to the coder.
- The voice prompt file must be passed only when it exists, or claude refuses to start.
- opencode needs `--standalone` in a pane; without it the TUI prints a banner and
  exits, and `agent start` reports a startup timeout. The whole start is one line —
  `herdr agent start reviewer --kind opencode --pane "$c2" --timeout 120000 -- --standalone`
  — and it takes no `--mcp-config`: opencode reads its servers from
  `~/.config/opencode/opencode.json`, so that file, not a herdr argument, decides which
  tools the seat has. `--kind opencode` resolves the binary itself, so an `opencode` that
  is not on the shell's `PATH` does not block the start; `herdr agent start --help`
  lists the kind as available.
- The opencode seat's board actor is fixed inside that config, under
  `mcp.servers.agentboard.environment.AGENTBOARD_ACTOR` (commonly `opencode`), not by the
  herdr name. Register the seat by its herdr name (`agentboard register reviewer --agent
  opencode`) and expect its claim, report and status events to be authored `opencode`:
  read the seat off the pane and the registration, never off the event actor, and name the
  actor that wrote a verdict when you quote it. Two opencode panes on one board need two
  config files with different actors, or the ledger cannot tell them apart.
- A pane that starts a fresh agent picks up the current `~/.cargo/bin/agentboard` as its
  MCP server, so a pane created after an install has the new verbs. A pane that already
  holds an agent keeps the OLD build, and `/clear` does not help: it clears the
  conversation, not the MCP server process, so the seat answers `tools/list` from the
  build it started with. Restart the agent in the pane (`/exit`, then `herdr agent start
  <name> --kind ... --pane <id>`) after any install that changes the board surface, and
  prove it per pane: `readlink /proc/<pid>/exe` must name the current file with no
  ` (deleted)` suffix. A stale seat is silent - its board calls succeed against the old
  surface, so a gate the new binary enforces (a diff required before `review`, the
  auto-filed review row) never fires and the round reads clean while proving nothing.
- **A seat started in a brand-new repo can sit on its own first-run surface.** A claude
  seat launched with `--strict-mcp-config` in a directory it has never trusted reports
  `agent_status: blocked` with `launch_pending: true` and never reaches a prompt, so
  `renew` (clear, then wait for a prompt) times out with a message that reads like a
  wedged agent. It is waiting on a trust/onboarding dialog. Make the start END at a
  prompt: pass the seat argv that trusts or auto-accepts the directory, in the same place
  the MCP config already lives, or detect `blocked` + `launch_pending`, read the pane, and
  refuse with the dialog text and the remedy. Never answer this by waiting longer - a
  longer timeout passes nowhere and hides the cause. `manager down` correctly refuses such
  a seat (`refusing to stop <name>: status=blocked`, exit 3), so prove the `--force` path
  on the same live round before claiming teardown works.
- Verify, do not assume: `agent start` can exit non-zero (or report `blocked`) while the
  agent is up at its prompt. `herdr agent list` + `herdr pane read` settles it.

## Talking to an agent

```bash
herdr agent prompt coder2 "Claim T15, then task_get id=15 full=true, and execute it."
herdr agent read coder2 --lines 30
herdr agent list
```

- `agent prompt` submits one turn atomically (bracketed paste). `pane send-text` types
  literal text and needs a second `send-keys ... enter` — interleaving the two
  concatenates lines into one mangled command.
- `agent read` is the only way to see state. A line that looks like "thinking" is often
  just a long command it started.
- `agent wait --until done --until blocked --timeout N` blocks; above the foreground
  cap it is promoted to a background job, so launch it that way deliberately.
- `agent_status` is `idle` (ready for input), `working`, `blocked`, or `done` for a
  one-shot run. **`idle` never means "task complete"** — the board says that.

## Teardown

```bash
pgid=$(herdr pane process-info --pane "$p" \
       | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["process_info"]["foreground_process_group_id"])')
kill -TERM "-$pgid"
herdr pane close "$p"
herdr tab close "$t"
```

Then verify with `tab list`, `pane list`, `agent list` rather than assuming. Closing the
pane also releases the agent name; killing the process group alone does not.

## Reading the list envelopes

`herdr agent list` -> `result.agents[]` with `name`, `agent` (kind), `agent_status`,
`cwd`, `pane_id`, `tab_id`, `terminal_title`. Panes without a registered agent name
come back as `"name": null` — filter on the name you expect instead of counting rows.

`herdr status` answers version, protocol and socket; when it cannot answer, the
scripts refuse rather than half-working, and so should the manager.
