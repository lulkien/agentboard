# Herdr mechanics

Herdr is the terminal workspace manager that holds the panes; agentboard holds the
work. This page is only about the pane side. Verbs live in `SKILL.md`.

Run `herdr --skill` for the authoritative text shipped with the installed binary.
If this page and the binary disagree about a flag, the binary wins.

## Check the context first

```bash
test "${HERDR_ENV:-}" = 1 || echo NOT_INSIDE_HERDR
herdr status                 # local client and running server
herdr pane current           # this pane's id
herdr pane list              # every pane, with tab and cwd
herdr agent list             # name, kind, agent_status, pane id
```

`HERDR_ENV` not `1` means this shell is not inside herdr: say so and stop rather
than starting processes that nothing can see.

## One tab per seat

The manager stays in its own tab. Every worker gets a fresh tab of its own: one
agent per tab, never two agents as split panes of one tab — a split puts two
agents on one screen and one set of keys, and a pane read then shows the wrong
seat.

```bash
herdr tab create --cwd "$REPO" --label coder
herdr pane list                       # find the new tab's pane id
```

`SKILL.md` has the `herdr agent start` command for each seat, with its preferred
model and permission mode. Panes inherit cwd; pass `--cwd` anyway so a seat never
opens in the wrong tree.

## Start and prompt a seat

```bash
herdr agent prompt coder '/agentboard-coder T<id>' --wait     # Codex: '$agentboard-coder T<id>'
```

Use the supported kind names from `herdr agent start --help`. Everything after
`--` belongs to the agent process; the shared launcher table documents its flags.
Each seat receives its board identity through `AGENTBOARD_ACTOR`, and the global
`agentboard` MCP entry carries it to the daemon.

## Troubleshooting

Herdr overlay recovery and prompt quoting use the terminal guidance in
[the team manual](../SKILL.md#prompt-delivery-and-quoting). Before restarting a pane,
inspect `task_get` and `board_agents` to confirm its board state.

## Teardown

`tab close` takes the id, never the label:

```bash
tab=$(herdr tab list | python3 -c \
  'import json,sys; tabs=json.load(sys.stdin)["result"]["tabs"]; print(next(t["tab_id"] for t in tabs if t.get("label")=="team"))')
herdr tab close "$tab"
herdr agent list            # confirm nothing named coder/reviewer remains
```

Pane closure only ends the terminal process; task state remains on the board.
