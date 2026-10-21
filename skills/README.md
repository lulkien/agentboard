# skills

Agent playbook, not code: nothing here is built or run by agentboard. One skill, `agentboard-manager/`:

- `SKILL.md`: the manual (board verbs, exit codes, manager verbs)
- `workflow.md`: how a row moves (chain, diff gate, reopen cap, status owners, epic boundary)
- `roles.md`: manager, coder, reviewer
- `references/`: read only when the case arises
- `commands/claude/manager.md`, `commands/opencode/manager.md`: the `/manager` slash command, one file per CLI (both are markdown with `description:` frontmatter; Claude Code also reads `allowed-tools` and `argument-hint`, opencode ignores them). Each loads the three pages (Claude Code by instructing a Read, since it blocks `!cat` outside the session dirs; opencode by `!cat` injection), registers the session, prints the brief, and tells the manager to arm `agentboard wait --open ... --timeout 300`.

## Install

    ./skills/install.sh --force [target-dir]

Default target `~/.hermes/skills`. The script symlinks, never copies: `~/.hermes/skills/agentboard-manager`, `~/.claude/commands/manager.md` and `~/.config/opencode/commands/manager.md` are links into this repo, so there is one file and no drift. Hermes lists and loads a symlinked skill directory. Without `--force` an existing path is never touched (`exists: <path> (use --force)`, exit 1).

The command files hardcode `~/.hermes/skills/agentboard-manager/`: they work only with the default skills target. A different target dir needs the paths in both command files edited.
