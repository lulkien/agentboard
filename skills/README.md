# skills

`agentboard-team/` is the shared manual (`SKILL.md`, `workflow.md`, `roles.md`,
plus `references/herdr-mechanics.md` and `references/hc-mechanics.md`). The five seat directories are one-page entry
points: `agentboard-manager/`, `agentboard-analyst/`, `agentboard-counter-analyst/`, `agentboard-coder/`,
`agentboard-reviewer/`. Each confirms the seat's
board name, preserves its startup check and seat limits, and links to the shared
workflow. Claude, Codex, Grok, Codewhale and agy load these skills directly.

## Install

The repository-root installer links them; there is no separate skill installer:

    ./install.py [--agent AGENT[,AGENT...]] [--force] [--skip-binary]

Every run, whatever `--agent` selects, makes one canonical link per skill:

    ~/.agents/skills/<name> -> <repo>/skills/<name>

Six names are installed: `agentboard-team` and the five `agentboard-<seat>` entry
points. Both the link and the repo directory are one physical copy, so an edit is
visible without reinstalling. Unrelated skills are left alone.

Where each client finds them under the global-config launch (checked 2026-11-22
against an isolated `HOME`; seats start as `env AGENTBOARD_ACTOR=<seat> <client>`):

| CLI | Reads skills from | What the installer adds | Invocation |
|---|---|---|---|
| Codex 0.171.0 | `$HOME/.agents/skills` (`codex debug prompt-input` lists it) | nothing beyond the canonical link | `$agentboard-<seat>` via `/skills` |
| Grok Build 1.0.89 | `$HOME/.agents/skills` (`grok inspect --json` reports it as a `user` skill) | nothing beyond the canonical link | `/agentboard-<seat>` |
| Codewhale 0.10.2 | `$HOME/.agents/skills` (`codewhale doctor`: "selected skills dir: ~/.agents/skills") | nothing beyond the canonical link | `/agentboard-<seat>` |
| Claude Code 2.1.330 | `~/.claude/skills` | canonical links via `~/.agents/skills` | `/agentboard-<seat>` |
| Antigravity (`agy`) | `~/.gemini/config/skills` | canonical links via `~/.agents/skills` | `/agentboard-<seat>` |

The installer writes `~/.claude/skills` only with `claude` selected (the default), never `$CODEX_HOME/skills` or any role home; aliases an
older skill installer left there still resolve and are left alone.

An existing canonical entry that is exactly this repo's link is left unchanged.
Any other file or link at one of the six names refuses the whole install, naming
each conflict as `exists: <path> (use --force)`; `--force` replaces those links or
files. A real directory at one of the names is refused even under `--force`, and a
non-directory `~/.agents` or `~/.agents/skills` refuses in preflight.

Each seat's board name is the `AGENTBOARD_ACTOR` it is launched with; the global
`agentboard` MCP entry sends it as a bearer token to the shared board. That name is
what the board attributes writes to and what the feed addresses.

`tests/install.sh` proves the layout in isolated homes: six canonical links
from every `--agent` choice, `~/.claude/skills` links only for `claude`, and the refusal and `--force` cases above.

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
  `agentboard-team` and the five `agentboard-<seat>` skills from `~/.agents/skills`
  without any alias, reports `projectTrusted`, and lists the MCP servers it will
  start. Project instructions and project skills load only in a trusted folder, so a
  grok seat is launched with `--trust`; `grok mcp doctor <server>` proves the board
  handshake without a model turn. MCP tools reach the model through `search_tool` /
  `use_tool` with catalog keys like `agentboard__task_add`.
