# hc mechanics

`hc` runs programs and coding agents in terminals that stay alive; agentboard holds
the work. This page is only about the terminal side. Board verbs live in `SKILL.md`.

Run `hc --skill` for the authoritative text shipped with the installed binary, and
`herdr-app --skill` for the window. Where this page and those disagree, the binary
wins.

## Check the context first

```bash
[[ -n ${HC_TERMINAL_ID:-} ]] || echo NOT_INSIDE_HC
hc get "$HC_TERMINAL_ID"     # this terminal: state live|finished, status, exit_code
hc ls                        # every live terminal, not only yours
```

`HC_TERMINAL_ID` unset means this shell is not inside hc: say so and stop rather than
starting processes that nothing can address. It is an address, not a pane id and not a
name.

## One terminal per seat

The manager stays in its own terminal. Every worker gets one of its own, named after
the seat: one agent per terminal, never two agents in one terminal, because a read
then shows the wrong seat and the two share one set of keys.

```bash
hc run --bg --name coder --cwd "$REPO" -- \
  claude --mcp-config ~/.claude/coder-mcp.json --dangerously-skip-permissions
```

`SKILL.md` has the per-seat command with its preferred model and permission mode.
`hc run` starts the agent as the new terminal's first process — nothing is typed into
an existing shell prompt — so the address in its `--json` reply is the handle from
then on. Codex needs `--no-daemon`, or its own commands run outside hc and nothing it
starts is addressable.

## Start, prompt and read a seat

```bash
hc wait coder                                               # until it reports; before then it is working
hc send --if idle --wait coder '/agentboard-coder T<id>'   # Codex: '$agentboard-coder T<id>'
hc read --lines 40 coder                                   # screen plus history
hc get coder                                               # state, status, exit_code
hc ls                                                      # the roster
```

A terminal that has not reported yet is working, so `hc send --if idle` to a booting
seat is refused with exit 10 `status_mismatch`: nothing is typed and the seat never
wakes. Wait first. A `started:false` reply is a different outcome: the text went in
and the program did not start working.

`hc send --if idle` refuses any terminal that is not exactly idle, a seat that is
mid-turn included: queue its row and prompt it later. `--force` overrides a `blocked`
terminal (exit 9 otherwise) and is not a manager verb: a blocked seat waits on a
person, so read the screen and ask.

Status comes from the program's own reports, and a plain program has none. Liveness
is `state: live|finished` plus `board_agents` last activity on the board, never a
guess from screen text.

## Traps

- A prompt that does not turn the agent busy is usually an overlay: a usage panel, a
  recap, a permission dialog. `hc keys coder esc`, then send again.
- Apostrophes end the shell quote in `hc send`. Keep the line free of them; never
  stage the prompt in a file and pass `"$(cat file)"` — the task row is the prompt,
  and a second copy is exactly what the board exists to prevent.
- A send without `--wait` gives no evidence the text landed; verify on the board.
- `hc run` without `--bg` streams the agent's whole screen into your output and holds
  your shell. Use `--bg` for seats, then `hc wait` and `hc read` to collect.
- A seat that went idle is not a seat that failed: check the board (`task_get`,
  `board_agents`) before restarting anything.
- Restarting an agent mid-write loses its claim context. Comment what it was doing
  first, then reassign with `force` and a reason.

## Watching

Seats start hidden. A terminal joins the person's window only when it carries
`--label herdr.space=<space-id>` at launch, or gets
`hc label <address> herdr.space=<space-id>` afterwards, and only the person decides
that: the launcher adds no label, so it also runs on a headless machine. To show a
seat that is in the window, focus it with
`herdr-app do agent.focus --terminal <address>`; `herdr-app ls` and `herdr-app
status` describe what the window holds. Do not start, signal or restart the person's
app or core.

## Teardown

Clean up only the terminals you started: `hc ls` includes the person's own work.
Check ownership first, then end and remove. A live program needs `hc kill` before
`hc rm`; a terminal that has already finished refuses `hc kill` with exit 11
`finished`, so `hc rm` alone removes it.

```bash
hc get coder --json | jq -e --arg me "$HC_TERMINAL_ID" '.parent == $me'
hc kill coder    # live only: a finished terminal answers exit 11 finished
hc rm coder
```

Closing a terminal does not touch the board: rows stay, claims stay attributed, and a
replacement seat resumes from the row. That is why the board, not the terminal, is
the record.
