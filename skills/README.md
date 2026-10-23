# skills

`agentboard-manager/` is the shared manual (`SKILL.md`, `workflow.md`, `roles.md`,
plus `references/herdr-mechanics.md`). `manager/` is the small initialization
skill: confirm the board, launch the worker panes with herdr, file one row, arm the
feed. Its `claude.md` and `opencode.md` resources keep the proven command-file
formats.

## Install

    ./skills/install.sh --force [canonical-dir]

Default canonical home: `~/.agents/skills`. Each canonical name is a symlink into
this repo, and every agent alias points at that canonical name:

    ~/.agents/skills/<name> -> <repo>/skills/<name>
    ~/.claude/skills/<name> -> ~/.agents/skills/<name>
    ~/.codex/skills/<name> -> ~/.agents/skills/<name>
    ~/.config/opencode/skills/<name> -> ~/.agents/skills/<name>
    ~/.hermes/skills/<name> -> ~/.agents/skills/<name>

The two installed names are `agentboard-manager` and `manager`: ten directory
links plus two command links, each printing `linked:`. Only the canonical links
contain the repo location; all five paths for a name resolve to one physical repo
directory, so an edit is visible without reinstalling. Unrelated skills are left
alone. Codex honours `CODEX_HOME`.

| CLI | Entry point in repo | Installed entry point | Invocation |
|---|---|---|---|
| Claude Code | `manager/claude.md` | `~/.claude/commands/manager.md` → canonical `manager/claude.md` | `/manager [focus]` |
| opencode | `manager/opencode.md` | `~/.config/opencode/commands/manager.md` → canonical `manager/opencode.md` | `/manager [focus]` |
| Codex | `manager/SKILL.md` | `$CODEX_HOME/skills/manager` → canonical `manager` | `$manager [focus]` via `/skills` |
| Hermes | `manager/SKILL.md` | `~/.hermes/skills/manager` → canonical `manager` | `/manager [focus]` |

The installer preflights all twelve destinations before changing any parent.
Without `--force`, an existing file, directory or dangling symlink prints
`exists: <path> (use --force)` and exits 1 with no changes. `--force` replaces only
the named destinations. Missing parents are created. A custom canonical directory
redirects every alias, but the command bodies' default `~/.agents/skills` manual
paths then need adjusting.

Each seat's MCP entry must carry its board name: `agentboard serve --actor coder`
(or the same in the client's MCP config), pointed at the shared `AGENTBOARD_DB`.
That name is what the board attributes writes to and what the feed addresses.

`tests/skills-install.sh` proves the layout in isolated homes: two canonical links,
four aliases per skill, twelve destinations, one edit visible through all four
aliases, and a second run that refuses without `--force` while naming every
conflict.

## Discovery notes (2026-10-22)

- Claude documents personal skills at `~/.claude/skills` and follows symlinked
  skill directories; its interactive discovery was not exercised here.
- Codex 0.163.0 lists both `manager` and `agentboard-manager` in `/skills` after
  installation, and inserts `$manager` when selected
  ([custom prompts are deprecated](https://learn.chatgpt.com/docs/custom-prompts);
  Codex uses the [skill picker](https://learn.chatgpt.com/docs/build-skills)), so
  no `~/.codex/prompts/manager.md` is installed.
- Hermes' installed loader finds exactly one `manager` catalog row, resolves its
  slash key to `/manager`, and loads the body through both symlink hops.
- OpenCode documents scanning `~/.config/opencode/skills`; its CLI was not on this
  shell's `PATH`, so live discovery was not exercised. The command file is kept.
