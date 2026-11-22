# skills

`agentboard-team/` is the shared manual (`SKILL.md`, `workflow.md`, `roles.md`,
plus `references/herdr-mechanics.md` and `references/hc-mechanics.md`). The four seat directories are one-page entry
points: `agentboard-manager/`, `agentboard-analyst/`, `agentboard-coder/`,
`agentboard-reviewer/`. Each confirms the seat's
board name, names what it may run on the board and what it never does, and ends by
reporting and stopping. The `SKILL.md` is the one source: Claude, Codex, Grok and Codewhale load it directly.

## Install

The repository-root installer links them; there is no separate skill installer:

    ./install.py [--agent AGENT[,AGENT...]] [--force] [--skip-binary]

Every run, whatever `--agent` selects, makes one canonical link per skill:

    ~/.agents/skills/<name> -> <repo>/skills/<name>

Five names are installed: `agentboard-team` and the four `agentboard-<seat>` entry
points. Both the link and the repo directory are one physical copy, so an edit is
visible without reinstalling. Unrelated skills are left alone.

Where each client finds them, with its config dir pointed at a seat role home
(checked 2026-11-22 against an isolated `HOME`):

| CLI | Reads skills from | What the installer adds | Invocation |
|---|---|---|---|
| Codex 0.171.0 | `$HOME/.agents/skills` (`codex debug prompt-input` lists it with `CODEX_HOME` set to a role home) | nothing beyond the canonical link | `$agentboard-<seat>` via `/skills` |
| Grok Build 1.0.89 | `$HOME/.agents/skills` (`grok inspect --json` reports it as a `user` skill with `GROK_HOME` set) | nothing beyond the canonical link | `/agentboard-<seat>` |
| Codewhale 0.10.2 | `$HOME/.agents/skills` (`codewhale doctor`: "selected skills dir: ~/.agents/skills" with `CODEWHALE_HOME` set) | nothing beyond the canonical link | `/agentboard-<seat>` |
| Claude Code 2.1.330 | `$CLAUDE_CONFIG_DIR/skills` | `seats/<seat>/claude/skills/{agentboard-<seat>,agentboard-team}` → the canonical names | `/agentboard-<seat>` |

The installer never writes `~/.claude/skills` or `$CODEX_HOME/skills`; aliases an
older skill installer left there still resolve and are left alone.

An existing canonical entry that is exactly this repo's link is left unchanged.
Any other file or link at one of the five names refuses the whole install, naming
each conflict as `exists: <path> (use --force)`; `--force` replaces those links or
files. A real directory at one of the names is refused even under `--force`, and a
non-directory `~/.agents` or `~/.agents/skills` refuses in preflight.

Each seat's MCP card carries its board name in the HTTP URL `/mcp/<seat>`, pointed
at the shared board. That name is what the board attributes writes to and what the
feed addresses.
`seats/` holds the canonical client cards for those entries.

`tests/install.sh` proves the layout in isolated homes: five canonical links
from every `--agent` choice, no `~/.claude/skills` alias, Claude role-home links
through the canonical names, and the refusal and `--force` cases above.

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
  `agentboard-team` and the four `agentboard-<seat>` skills from `~/.agents/skills`
  without any alias, reports `projectTrusted`, and lists the MCP servers it will
  start. Project instructions and project skills load only in a trusted folder, so a
  grok seat is launched with `--trust`; `grok mcp doctor <server>` proves the board
  handshake without a model turn. MCP tools reach the model through `search_tool` /
  `use_tool` with catalog keys like `agentboard__task_add`.
