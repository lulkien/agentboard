# skills

`agentboard-team/` is the shared manual (`SKILL.md`, `workflow.md`, `roles.md`,
plus `references/herdr-mechanics.md`). The four seat directories are one-page entry
points: `agentboard-manager/`, `agentboard-analyst/`, `agentboard-coder/`,
`agentboard-reviewer/`. Each confirms the seat's
board name, names what it may run on the board and what it never does, and ends by
reporting and stopping. The `SKILL.md` is the one source: Claude, Codex, Hermes and
Grok load it directly. opencode has no skill slash command, so each seat also keeps a
four-line `opencode.md` that only says to load the skill.

## Install

    ./skills/install.py --force [canonical-dir]

Default canonical home: `~/.agents/skills`. Each canonical name is a symlink into
this repo, and every agent alias points at that canonical name:

    ~/.agents/skills/<name> -> <repo>/skills/<name>
    ~/.claude/skills/<name> -> ~/.agents/skills/<name>
    ~/.codex/skills/<name> -> ~/.agents/skills/<name>
    ~/.config/opencode/skills/<name> -> ~/.agents/skills/<name>
    ~/.hermes/skills/<name> -> ~/.agents/skills/<name>

Five names are installed — `agentboard-team` and the four `agentboard-<seat>` entry
points — so each run creates `5 x 5 = 25` directory links and four command links, each printing
`linked:`. Only the canonical links contain the repo location; all five paths for a
name resolve to one physical repo directory, so an edit is visible without
reinstalling. Unrelated skills are left alone. Codex honours `CODEX_HOME`.

| CLI | Entry point in repo | Installed entry point | Invocation |
|---|---|---|---|
| Claude Code | `agentboard-<seat>/SKILL.md` | `~/.claude/skills/agentboard-<seat>` → canonical `agentboard-<seat>` | `/agentboard-<seat> [focus]` |
| opencode | `agentboard-<seat>/opencode.md` | `~/.config/opencode/commands/agentboard-<seat>.md` → canonical `agentboard-<seat>/opencode.md` | `/agentboard-<seat> [focus]` |
| Codex | `agentboard-<seat>/SKILL.md` | `$CODEX_HOME/skills/agentboard-<seat>` → canonical `agentboard-<seat>` | `$agentboard-<seat> [focus]` via `/skills` |
| Hermes | `agentboard-<seat>/SKILL.md` | `~/.hermes/skills/agentboard-<seat>` → canonical `agentboard-<seat>` | `/agentboard-<seat> [focus]` |
| Grok Build | `agentboard-manager/SKILL.md` (+ `grok.md`) | `~/.agents/skills/agentboard-manager` → canonical (no alias needed) | `/agentboard-manager [focus]` |

Grok scans `~/.agents/skills` itself, at every tier, so it needs no alias directory
of its own; the seat entry points and the manual it shares all resolve through the
canonical links.

The installer preflights all twenty-nine destinations before changing any parent.
Without `--force`, an existing file, directory or dangling symlink prints
`exists: <path> (use --force)` and exits 1 with no changes. `--force` replaces only
the named destinations. Missing parents are created. Retired names (manager, analyst, coder, reviewer,
grok-manager, analyzer) are pruned when the link is exactly one this installer wrote;
anything else is left alone. A custom canonical directory
redirects every alias, but the command bodies' default `~/.agents/skills` manual
paths then need adjusting.

Each seat's MCP entry must carry its board name: `agentboard serve --actor coder`
(or the same in the client's MCP config), pointed at the shared `AGENTBOARD_DB`.
That name is what the board attributes writes to and what the feed addresses.
`seats/` holds the canonical client cards for those entries.

`tests/skills-install.sh` proves the layout in isolated homes: five canonical
links, four aliases per skill, four command links, retired links pruned, one edit visible through all
four aliases, a second run that refuses without `--force` while naming every
conflict, and the four seat entry points present with an opencode command and no Claude command.

## Discovery notes (2026-10-22)

- Claude documents personal skills at `~/.claude/skills` and follows symlinked
  skill directories; its interactive discovery was not exercised here.
- Codex 0.163.0 lists installed skill names in `/skills` and inserts `$<name>` when
  selected
  ([custom prompts are deprecated](https://learn.chatgpt.com/docs/custom-prompts);
  Codex uses the [skill picker](https://learn.chatgpt.com/docs/build-skills)), so
  no `~/.codex/prompts/` file is installed.
- Hermes' installed loader finds one catalog row per name, resolves each slash key,
  and loads the body through both symlink hops.
- OpenCode documents scanning `~/.config/opencode/skills`; its CLI was not on this
  shell's `PATH`, so live discovery was not exercised. The opencode command files are kept.
- Grok Build 1.0.70 was exercised here: `grok inspect --json` lists the canonical
  `agentboard-team` and the four `agentboard-<seat>` skills from `~/.agents/skills`
  without any alias, reports `projectTrusted`, and lists the MCP servers it will
  start. Project instructions and project skills load only in a trusted folder, so a
  grok seat is launched with `--trust`; `grok mcp doctor <server>` proves the board
  handshake without a model turn. MCP tools reach the model through `search_tool` /
  `use_tool` with catalog keys like `agentboard__task_add`.
