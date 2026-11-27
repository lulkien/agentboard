# skills

`agentboard-team/` is the shared manual (`SKILL.md`, `workflow.md`, `roles.md`,
plus `references/herdr-mechanics.md` and `references/hc-mechanics.md`). The seven seat directories are one-page entry
points: `agentboard-manager/`, `agentboard-analyst/`, `agentboard-counter-analyst/`, `agentboard-coder/`,
`agentboard-reviewer/`, `agentboard-designer/`, `agentboard-librarian/`. Each confirms the seat's
board name, preserves its startup check and seat limits, and links to the shared
workflow. Claude, Codex, Grok, Codewhale, agy and OpenCode load these skills
directly.

## Install

The repository-root installer delegates installation to the `skills` CLI:

    ./install.py [--agent AGENT[,AGENT...]] [--force] [--skip-binary]

It runs `bunx skills@1.9.0 add <repo> -g -a <targets...> -s '*' -y`; if `bunx`
is unavailable it uses `npx -y skills@1.9.0`. Install Bun or Node.js so one of
those runners is on `PATH`. The CLI installs the eight repo skills globally, with
targets selected by `--agent`.

Eight names are installed: `agentboard-team` and the seven `agentboard-<seat>` entry
points. `skills` manages the client-specific destinations. Re-run `./install.py
--skip-binary` after editing repo skills to refresh installed copies.

Where each client finds them under the global-config launch (checked 2026-11-22
against an isolated `HOME`; seats start as `env AGENTBOARD_ACTOR=<seat> <client>`):

| CLI | Reads skills from | What the installer adds | Invocation |
|---|---|---|---|
| Codex | `$HOME/.agents/skills` | `codex` | `$agentboard-<seat>` via `/skills` |
| Grok | `$HOME/.agents/skills` | `universal` | `/agentboard-<seat>` |
| Codewhale | `$HOME/.agents/skills` | `universal` | `/agentboard-<seat>` |
| Claude Code | `~/.claude/skills` | `claude-code` | `/agentboard-<seat>` |
| Antigravity (`agy`) | `~/.gemini/config/skills` | `antigravity` | `/agentboard-<seat>` |
| OpenCode | `~/.agents/skills` | `opencode` | See the team launch recipe |

Target mapping is: `codex` → `codex`, `claude` → `claude-code`, `grok` and
`codewhale` → `universal`, `agy` → `antigravity`, and `opencode` → `opencode`.
`--agent` selects these targets; the default selects all six clients. The installer
sets `DO_NOT_TRACK=1` for the CLI. `--force` applies to launcher and unit links;
the skills CLI handles skill destination conflicts.

Each seat's board name is the `AGENTBOARD_ACTOR` it is launched with; the global
`agentboard` MCP entry sends it as a bearer token to the shared board. That name is
what the board attributes writes to and what the feed addresses.

`tests/install.sh` proves the CLI arguments and runner selection with local stubs;
it does not make network calls.

## Discovery notes (2026-10-22)

- Claude documents personal skills at `~/.claude/skills` and follows symlinked
  skill directories; its interactive discovery was not exercised here.
- Codex 0.163.0 lists installed skill names in `/skills` and inserts `$<name>` when
  selected
  ([custom prompts are deprecated](https://learn.chatgpt.com/docs/custom-prompts);
  Codex uses the [skill picker](https://learn.chatgpt.com/docs/build-skills)), so
  no `~/.codex/prompts/` file is installed.
- Codex 0.167.2's `codex debug prompt-input` lists skills from `~/.agents/skills`;
  the old `$CODEX_HOME/skills` alias was redundant and duplicate paths list once.
- Grok Build 1.0.70 was exercised here: `grok inspect --json` lists the canonical
  `agentboard-team` and the seven `agentboard-<seat>` skills from `~/.agents/skills`
  without any alias, reports `projectTrusted`, and lists the MCP servers it will
  start. Project instructions and project skills load only in a trusted folder, so a
  grok seat is launched with `--trust`; `grok mcp doctor <server>` proves the board
  handshake without a model turn. MCP tools reach the model through `search_tool` /
  `use_tool` with catalog keys like `agentboard__task_add`.
