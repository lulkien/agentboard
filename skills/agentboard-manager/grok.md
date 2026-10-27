# Grok as the manager seat

Grok Build (`grok`, herdr kind `grok`) can hold the manager seat. It reads and writes
the board over MCP, files rows, launches the other seats and wakes them by prompt.
It writes no code and runs no build, same as every manager. The steps are
`SKILL.md`; this page only adds what is specific to Grok.

Read `~/.agents/skills/agentboard-team/{SKILL.md,workflow.md,roles.md}` first: they
are the shared manual.

## One-time setup: give grok a board identity

    seats/install.py                         # actor "grok", installs all seat cards
    seats/install.py --actor manager         # use the manager actor for Grok
    seats/grok/register-mcp.sh --actor grok  # compatibility wrapper, Grok only

The installer manages the marked block in `~/.grok/config.toml` from
`seats/grok/agentboard-mcp.toml`; it uses `command = "agentboard"`, so the binary
must resolve on Grok's PATH. It preserves host config and the existing actor when
`--actor` is omitted. An unmarked `[mcp_servers.agentboard]` entry is refused, even
with `--force`; review and remove or convert it manually before installing. Check it:

    grok mcp list                    # agentboard, stdio, enabled
    grok mcp doctor agentboard       # handshake + catalog keys, no model turn

`grok mcp doctor` is the honest proof: it starts the server, completes the MCP
handshake and lists the tools. A server that fails to start shows there, not in a
seat that silently has no board tools.

## Launch

    herdr tab create --cwd "$REPO" --label grok
    herdr pane list                                   # note the pane id
    herdr agent start grok --kind grok --pane "$PANE_GROK" -- --trust --always-approve
    herdr agent prompt grok '/agentboard-manager focus: <what this session is for>' --wait

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

## What grok actually reads

- Skills: `~/.agents/skills/` itself (the manual, the manager and the three worker seats), plus
  `.grok/skills`, `.agents/skills` and `.claude/skills` at repo and user level.
  `grok inspect --json` lists them under `skills` — 40-odd entries on a working box.
- Project rules: `AGENTS.md`/`CLAUDE.md` from repo root down to the cwd, but only
  when the folder is trusted (`grok inspect --json` shows `projectTrusted`).
- Board tools: MCP tools are reached through the meta-tools, catalog key
  `agentboard__task_add` (two underscores), not a bare `task_add` and not
  `mcp__agentboard__task_add`. Waiting on a row is the
  binary's `agentboard wait`, not a board tool.

## Prove the seat is wired

    grok inspect --json | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d["projectTrusted"], [s["name"] for s in d["skills"] if s["name"].startswith("agentboard-")], [m["name"] for m in d["mcpServers"]])'
    grok mcp doctor agentboard
    grok -p 'list the board brief with the agentboard board_brief tool' --always-approve --max-turns 3

The skills list must be exactly `agentboard-team`, `agentboard-manager`,
`agentboard-analyst`, `agentboard-coder` and `agentboard-reviewer`; an empty list
looks like success and means the skills are not installed.

The last line is a headless read-only round: it proves the model can call the board,
not just that the server starts.

## Traps

- Never install a grok command file whose name matches a skill: Grok deduplicates
  skills and commands by name, so the slash key stops resolving. The grok entry
  point is the `agentboard-manager` skill.
- A grok seat that looks idle may be sitting on an approval prompt; check
  `herdr pane output` before re-prompting, and launch with `--always-approve`.
- Do not use headless `-p` as a seat: it runs one prompt and exits, so there is no
  pane to re-prompt and no turn to continue. `-p` is for probes.
- `grok mcp add` writes user scope by default; `--scope project` writes
  `.grok/config.toml` into the repo, which pins one actor for everyone who opens it.
