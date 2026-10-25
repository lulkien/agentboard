---
name: grok-manager
description: Use when running Grok Build as the agentboard manager seat. Registers the board MCP server under the grok actor, launches the seat in herdr with grok's trust and always-approve flags, and keeps the rule that the manager never blocks.
when-to-use: starting grok as the manager, grooming the board with grok, launching seats from a grok session
allowed-tools: Bash(agentboard:*), Bash(herdr:*), Bash(grok:*), Read
---

# Grok as the manager seat

Grok Build (`grok`, herdr kind `grok`) can hold the manager seat. It reads and writes
the board over MCP, files rows, launches the other seats and wakes them by prompt.
It writes no code and runs no build, same as every manager.

Read `~/.agents/skills/agentboard-team/{SKILL.md,workflow.md,roles.md}` first: they
are the shared manual and this page only adds what is specific to Grok.

## One-time setup: give grok a board identity

    seats/grok/register-mcp.sh                 # actor "grok"
    seats/grok/register-mcp.sh --actor manager # or present as "manager"

That runs `grok mcp add --scope user agentboard -e AGENTBOARD_ACTOR=<actor> --
<abs path to agentboard> serve`, so the server is the board with that actor and the
registration lives in `~/.grok/config.toml`. Check it:

    grok mcp list                    # agentboard, stdio, enabled
    grok mcp doctor agentboard       # handshake + catalog keys, no model turn

`grok mcp doctor` is the honest proof: it starts the server, completes the MCP
handshake and lists the tools. A server that fails to start shows there, not in a
seat that silently has no board tools.

## Launch

    herdr tab create --cwd "$REPO" --label grok
    herdr pane list                                   # note the pane id
    herdr agent start grok --kind grok --pane "$PANE_GROK" -- --trust --always-approve
    herdr agent prompt grok 'Load the grok-manager skill and the agentboard-team manual. You are the manager on the agentboard, actor=grok. Start with board_brief, then launch only the seats this work needs.' --wait

- `--trust` is not optional if the seat must read the repo's `AGENTS.md` or its
  `.grok`, `.agents` and `.claude` project surfaces: untrusted, Grok loads no project
  instructions and no project skills, and nothing says so in the pane. The grant is
  recorded once in `~/.grok/trusted_folders.toml` and covers subdirectories.
- `--always-approve` is what stops the seat stalling on an approval prompt nobody is
  watching. Without it a tool call waits for a keypress in a pane the manager owns.
  Add `--deny` rules for paths or commands that must never run.
- herdr passes everything after `--` to grok, so any grok flag goes there.
- `herdr agent prompt ... --wait` is the raw herdr verb and does have `--wait`;
  nothing on the board does. Waking a seat is a herdr action, not a board write.

## Prompting the other seats

    herdr agent prompt coder 'row 27 is yours; claim it, report, stop' --wait

Wake by name, one row per prompt, no polling loop: a seat does its row, reports it
and stops. A doubt about a claim becomes a reviewer row, never a fix written here.

## Waiting: park a wait, never block the turn

    agentboard wait 27 --status review --timeout 300
    agentboard wait 12 --epic --timeout 300

Run it in the background, and re-arm it when it exits `3` (nothing matched). One wait
is at most 300 seconds — the CLI refuses `--timeout` above 300 or below 1 — so a long
round is a chain of parked waits, not one 30-minute call. Exit `0` prints `matched:`
with the rows; either way the manager, not the wait, is what prompts a seat.

## What grok actually reads

- Skills: `~/.agents/skills/` itself (this skill, the four seats, the manual), plus
  `.grok/skills`, `.agents/skills` and `.claude/skills` at repo and user level.
  `grok inspect --json` lists them under `skills` — 40-odd entries on a working box.
- Project rules: `AGENTS.md`/`CLAUDE.md` from repo root down to the cwd, but only
  when the folder is trusted (`grok inspect --json` shows `projectTrusted`).
- Board tools: MCP tools are reached through the meta-tools, catalog key
  `agentboard__task_add` (two underscores), not a bare `task_add` and not
  `mcp__agentboard__task_add`. Tell a seat to `use_tool("agentboard__task_wait", …)`
  when it must block on a row.

## Prove the seat is wired

    grok inspect --json | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d["projectTrusted"], [s["name"] for s in d["skills"] if "grok-manager" in s["name"]], [m["name"] for m in d["mcpServers"]])'
    grok mcp doctor agentboard
    grok -p 'list the board brief with the agentboard board_brief tool' --always-approve --max-turns 3

The last line is a headless read-only round: it proves the model can call the board,
not just that the server starts.

## Traps

- Never install a grok command file named `manager.md`: Grok deduplicates skills and
  commands by name, so a `manager` command beside the `manager` skill is an ambiguous
  name and the slash key stops resolving. The grok entry point is this skill,
  `grok-manager`.
- A grok seat that looks idle may be sitting on an approval prompt; check
  `herdr pane output` before re-prompting, and launch with `--always-approve`.
- Do not use headless `-p` as a seat: it runs one prompt and exits, so there is no
  pane to re-prompt and no turn to continue. `-p` is for probes.
- `grok mcp add` writes user scope by default; `--scope project` writes
  `.grok/config.toml` into the repo, which pins one actor for everyone who opens it.
