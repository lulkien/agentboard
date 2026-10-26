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

`--kind` is the canonical kind name from `herdr agent start --help` (`claude`,
`codex`, `opencode`, `hermes`, …). Everything after `--` is the agent's own argv
(Claude `--model` / `--permission-mode`, Codex `-m` / `-p`, MCP config flags).
The seat's board name is not a herdr concept: the MCP server is registered with
`agentboard serve --actor coder`, so the pane's tool calls arrive attributed.

## Traps

- A prompt that stalls (`no observed working state … current status is idle`) is
  usually an open overlay: usage panel, recap, permission dialog. Read the pane
  (`herdr agent read coder --lines 40`), send `escape` via
  `herdr agent send-keys coder escape`, then prompt again.
- Apostrophes end the shell quote in `herdr agent prompt`. Keep the line free of
  them; never stage the prompt in a file and pass `"$(cat file)"` — the task row is
  the prompt, and a second copy is exactly what the board exists to prevent.
- `herdr agent prompt` without `--wait` gives no evidence the text landed. Use it
  when you will verify on the board instead.
- A seat that went idle is not a seat that failed: check the board
  (`task_get`, `board_agents`) before restarting anything.
- Restarting an agent mid-write loses its claim context. Comment what it was doing
  first, then reassign with `force` and a reason.

## Teardown

`tab close` takes the id, never the label:

```bash
tab=$(herdr tab list | python3 -c \
  'import json,sys; tabs=json.load(sys.stdin)["result"]["tabs"]; print(next(t["tab_id"] for t in tabs if t.get("label")=="team"))')
herdr tab close "$tab"
herdr agent list            # confirm nothing named coder/reviewer remains
```

Closing a pane does not touch the board: rows stay, claims stay attributed, and a
replacement seat resumes from the row. That is why the board, not the pane, is the
record.
