# skills

`agentboard-manager/` is the shared manual (`SKILL.md`, `workflow.md`, `roles.md`).
`manager/` is the small shared initialization skill: register this session with
its actual CLI kind through MCP, print the brief, and arm the 300 s watch.
Its `claude.md` and `opencode.md` resources retain the proven command-file formats.
There is one `SKILL.md` named `manager`, with no nested competing skills.

## Install

    ./skills/install.sh --force [canonical-dir]

Default canonical home: `~/.agents/skills`. We use option (a): each canonical name
is a symlink into this repo, and every agent alias points at that canonical name:

    ~/.agents/skills/<name> -> <repo>/skills/<name>
    ~/.claude/skills/<name> -> ~/.agents/skills/<name>
    ~/.codex/skills/<name> -> ~/.agents/skills/<name>
    ~/.config/opencode/skills/<name> -> ~/.agents/skills/<name>
    ~/.hermes/skills/<name> -> ~/.agents/skills/<name>

The two installed names are `agentboard-manager` and `manager`: ten directory
links, plus the two command links below, each printing `linked:`. Only the two
canonical links contain the repo location. All five paths for a name resolve to
one physical repo directory; edits are immediately visible without reinstalling.
Existing unrelated skills are left alone. Codex honors `CODEX_HOME`.

| CLI | Entry point in repo | Installed entry point | Invocation |
|---|---|---|---|
| Claude Code | `manager/claude.md` | `~/.claude/commands/manager.md` → canonical `manager/claude.md` | `/manager [focus]` |
| opencode | `manager/opencode.md` | `~/.config/opencode/commands/manager.md` → canonical `manager/opencode.md` | `/manager [focus]` |
| Codex | `manager/SKILL.md` | `$CODEX_HOME/skills/manager` → canonical `manager` | `$manager [focus]` via `/skills` |
| Hermes | `manager/SKILL.md` | `~/.hermes/skills/manager` → canonical `manager` | `/manager [focus]` |

The manual is also available in each agent's skills directory. Claude keeps its
`description`, `allowed-tools`, `argument-hint`, `$ARGUMENTS`, and bang-prefixed
command injection; opencode keeps its bang-prefixed backtick commands. Their
command links now go through the canonical manager directory, never directly to
the repo. `AGENTBOARD_CLAUDE_COMMANDS` and `AGENTBOARD_OPENCODE_COMMANDS` still
override those two command destinations.

Codex and Hermes both proved they load invocable skills, so their formerly
separate initialization bodies are consolidated. The shared body selects the
current CLI kind rather than hardcoding `codex` or `hermes`. Claude/opencode
command files remain available alongside the shared skill.
[OpenAI deprecated custom prompts](https://learn.chatgpt.com/docs/custom-prompts);
Codex 0.163.0 uses the [skill picker](https://learn.chatgpt.com/docs/build-skills).
No `~/.codex/prompts/manager.md` is installed; bare `/manager` is unavailable in
Codex. The dispatch prompt and repo `AGENTS.md` remain the coder fallback.

The installer preflights all twelve destinations before changing any parent.
Without `--force`, an existing file, directory, or dangling symlink produces
`exists: <path> (use --force)` and exit 1, with no changes. `--force` replaces only
the named destinations; it also rewires the previous direct-to-repo installation.
Missing parents are created. A custom canonical directory redirects every alias;
the command bodies' default `~/.agents/skills` manual paths must then be adjusted.

Discovery evidence for this layout (2026-10-22):

- Claude documents personal skills at `~/.claude/skills` and following symlinked
  skill directories in its [skills guide](https://code.claude.com/docs/en/skills).
  Its interactive discovery was not exercised for this change.
- Codex 0.163.0, in a fresh session after installation, lists both `manager` and
  `agentboard-manager` in `/skills` → List skills; selecting `manager` inserts
  `$manager`. This verifies discovery through the canonical links, without
  submitting the initialization.
- Hermes' installed loader finds exactly one `manager` catalog row with status
  `unique`, resolves its slash key to `/manager`, and loads the shared body through
  both symlink hops. The loader still warns that repo-backed content is outside
  its trusted directory, as it did with the previous direct link.
- OpenCode documents scanning `~/.config/opencode/skills` in its
  [skills guide](https://opencode.ai/docs/skills/). Its CLI was not on this shell's
  `PATH`, so live discovery was not exercised. The command file is retained.

To roll back, restore the preceding repo layout and installer, rerun that installer
with `--force` to restore its five destinations, and remove only the seven aliases
introduced here after checking their link targets. No skill content needs copying
or restoring from an installed snapshot: the repo remains the sole source.
