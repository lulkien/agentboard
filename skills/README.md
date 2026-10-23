# skills

`agentboard-team/` is the shared manual (`SKILL.md`, `workflow.md`, `roles.md`,
plus `references/herdr-mechanics.md`). The four seat directories are one-page entry
points: `manager/`, `analyzer/`, `coder/`, `reviewer/`. Each confirms the seat's
board name, names what it may run on the board and what it never does, and ends by
arming the feed. Their `claude.md` and `opencode.md` resources keep the proven
command-file formats; the `SKILL.md` is what Codex and Hermes load.

## Install

    ./skills/install.sh --force [canonical-dir]

Default canonical home: `~/.agents/skills`. Each canonical name is a symlink into
this repo, and every agent alias points at that canonical name:

    ~/.agents/skills/<name> -> <repo>/skills/<name>
    ~/.claude/skills/<name> -> ~/.agents/skills/<name>
    ~/.codex/skills/<name> -> ~/.agents/skills/<name>
    ~/.config/opencode/skills/<name> -> ~/.agents/skills/<name>
    ~/.hermes/skills/<name> -> ~/.agents/skills/<name>

Five names are installed — `agentboard-team` plus the four seats — so each run
creates `5 x 5 = 25` directory links and eight command links, each printing
`linked:`. Only the canonical links contain the repo location; all five paths for a
name resolve to one physical repo directory, so an edit is visible without
reinstalling. Unrelated skills are left alone. Codex honours `CODEX_HOME`.

| CLI | Entry point in repo | Installed entry point | Invocation |
|---|---|---|---|
| Claude Code | `<seat>/claude.md` | `~/.claude/commands/<seat>.md` → canonical `<seat>/claude.md` | `/<seat> [focus]` |
| opencode | `<seat>/opencode.md` | `~/.config/opencode/commands/<seat>.md` → canonical `<seat>/opencode.md` | `/<seat> [focus]` |
| Codex | `<seat>/SKILL.md` | `$CODEX_HOME/skills/<seat>` → canonical `<seat>` | `$<seat> [focus]` via `/skills` |
| Hermes | `<seat>/SKILL.md` | `~/.hermes/skills/<seat>` → canonical `<seat>` | `/<seat> [focus]` |

The installer preflights all thirty-three destinations before changing any parent.
Without `--force`, an existing file, directory or dangling symlink prints
`exists: <path> (use --force)` and exits 1 with no changes. `--force` replaces only
the named destinations. Missing parents are created. A custom canonical directory
redirects every alias, but the command bodies' default `~/.agents/skills` manual
paths then need adjusting.

Each seat's MCP entry must carry its board name: `agentboard serve --actor coder`
(or the same in the client's MCP config), pointed at the shared `AGENTBOARD_DB`.
That name is what the board attributes writes to and what the feed addresses.
`seats/` holds the canonical client cards for those entries.

`tests/skills-install.sh` proves the layout in isolated homes: five canonical
links, four aliases per skill, eight command links, one edit visible through all
four aliases, a second run that refuses without `--force` while naming every
conflict, and the four seat entry points present with both command files.

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
  shell's `PATH`, so live discovery was not exercised. The command files are kept.
