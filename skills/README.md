# skills

`agentboard-manager/` is the shared manual: `SKILL.md` covers verbs and exit codes,
`workflow.md` covers the review chain, and `roles.md` defines each seat. The four
entry points below only initialize the manager seat: register this session, print
the brief, and arm the 300 s watch. Read the pages for everything else.

All file paths in the table are relative to `agentboard-manager/`.

| CLI | File | Install target | Invocation |
|---|---|---|---|
| Claude Code | `commands/claude/manager.md` | `~/.claude/commands/manager.md` | `/manager [focus]` |
| opencode | `commands/opencode/manager.md` | `~/.config/opencode/commands/manager.md` | `/manager [focus]` |
| Codex | `commands/codex/manager/SKILL.md` | `$CODEX_HOME/skills/manager/SKILL.md` (default `~/.codex/skills/manager/SKILL.md`; linked directory) | `$manager [focus]` (select through `/skills`) |
| Hermes | `commands/hermes/manager/SKILL.md` | `~/.hermes/skills/manager/SKILL.md` (linked directory) | `/manager [focus]` |

Claude Code uses `description`, `allowed-tools`, and `argument-hint` frontmatter
and bang-prefixed command injection. opencode uses bang-prefixed backtick commands.
Codex and Hermes use small skill directories with `name: manager`; their bodies
instruct the agent to initialize the seat. Hermes exposes its installed skill as
`/manager`, without a commands directory.

[OpenAI's custom-prompt documentation](https://learn.chatgpt.com/docs/custom-prompts)
marks the old Markdown prompts mechanism deprecated in favor of skills. Installed
Codex 0.163.0 rejects both `/manager` and `/prompts:manager` for a prompts file, so
we ship no ignored prompts artifact. Its entry point is a
[skill](https://developers.openai.com/codex/skills), installed under `CODEX_HOME`.
On Codex 0.163.0, `/skills` → **List skills** → type `manager` lists the installed
skill; Enter inserts `$manager` in the composer. Add a focus and submit to invoke
it. Bare `/manager` is unavailable. The coder seat can also receive instructions
through the dispatch prompt and repository `AGENTS.md`.

## Install

    ./skills/install.sh --force [target-dir]

The default manual target is `~/.hermes/skills`. The installer creates missing
parent directories, including the Codex skills directory, and prints a
`linked:` line for each link. It links the manual and all four entry points into
this repo, never copies them. Hermes's `manager` directory always goes under
`~/.hermes/skills`; Codex honors `CODEX_HOME`. Claude and opencode targets can be
overridden with `AGENTBOARD_CLAUDE_COMMANDS` and `AGENTBOARD_OPENCODE_COMMANDS`.

Without `--force`, the installer checks every destination before changing anything.
An existing file, directory or symlink produces `exists: <path> (use --force)` on
stderr and exit 1; no destination or parent directory is changed. `--force`
replaces the named destinations.

The entry points refer to `~/.hermes/skills/agentboard-manager/`. A different
manual target needs those paths edited. The default Hermes skill and the command
links remain repo-backed, so edit their repo sources rather than copying over links.
