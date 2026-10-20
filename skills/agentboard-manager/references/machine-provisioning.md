# Provisioning a machine for the coder/reviewer team

Use when a new box must run the delegation loop, or when an agent on an existing
box cannot claim board tasks. Everything here is provisioning; the daily loop is
in SKILL.md.

## Prerequisites to check, never install silently

| Piece | Check |
|---|---|
| `claude` | `claude --version`, then `claude auth status --text` — login is the user's step |
| `agentboard` | `agentboard path` |
| `codebase-memory-mcp` | `codebase-memory-mcp --version` |
| `git-mcp-rs` | `git-mcp-rs --version` |
| `ferris-search` | resolves on PATH |
| pane host | `tmux -V` or `herdr --version` |

`~/.local/bin` and `~/.cargo/bin` must be on PATH or the wrappers and the MCP
commands resolve differently for the manager and for the agents.

## One config per actor

Write `~/.claude/coding-mcp.json` with `AGENTBOARD_ACTOR=claude1` and
`~/.claude/reviewer-mcp.json` with `AGENTBOARD_ACTOR=reviewer`. Each entry:

```json
{
  "mcpServers": {
    "agentboard": {
      "type": "stdio",
      "command": "/abs/path/agentboard",
      "args": [],
      "env": { "AGENTBOARD_DB": "/abs/board.db", "AGENTBOARD_ACTOR": "claude1" }
    }
  }
}
```

A shared config makes every board write land under one actor and the code-vs-review
split disappears. Absolute paths, because the agent's cwd is the repo under review.
Hermes itself gets the same server with `AGENTBOARD_ACTOR=hermes` via
`hermes mcp add`, so manager writes are attributable.

## Launcher wrappers

One wrapper per actor, both pinning `--strict-mcp-config`:

```bash
model_arg=(--model=${CLAUDE_MODEL:-sonnet})
for a in "$@"; do case "$a" in --model|--model=*) model_arg=(); break;; esac; done
exec claude --strict-mcp-config --mcp-config="$HOME/.claude/coding-mcp.json" \
  "${model_arg[@]}" "$@"
```

- Strict mode drops account-level claude.ai connectors and any project `.mcp.json`
  the agent wanders into.
- Always `--mcp-config=VALUE`; the space form consumes the next argument.
- Default the model, let an explicit `--model` in `$@` win.

## Installer rules learned the hard way

- **Merge MCP configs, never replace them.** Read the existing file, write the
  entries you own, keep every server you do not (`gerrit-mcp`, a hand-added local
  server). A replace step silently deletes the user's servers.
- **Never overwrite a differing wrapper without an explicit force flag.** A box's
  wrapper often appends its own system-prompt file (voice, policy); a template
  overwrite drops it and the change is invisible until the agent talks wrong. Print
  `exists, differs — FORCE=1 to replace` and move on.
- `cmp -s` against a freshly rendered temp file gives free idempotency: second run
  prints `unchanged` for everything.
- Offer `SKIP_HERMES=1` so the installer can be tested in a sandbox HOME without
  touching the live Hermes config.
- Everything path-like is an env override with a `command -v` fallback
  (`CLAUDE_BIN`, `AGENTBOARD_BIN`, `CBMEM_BIN`, `AGENTBOARD_DB`, ...). Machines differ;
  hardcoded `~/.bun/bin` or `~/.cargo/bin` assumptions break silently.
- No credentials in anything you copy to another machine. Mirror the server entry
  and let that machine's owner add the secret.

## Reviewer on opencode (preferred when installed)

The reviewer should be a different vendor from the coder. Wire opencode as an
additional actor rather than replacing the Claude pair:

- Detect `opencode2` first, then `opencode`. The two names are often symlinks to
the same binary — `readlink -f` both before writing version-specific advice.
- Merge into `~/.config/opencode/opencode.json` under `mcp.servers.<name>`, with
`"type": "local"`, `command` as an ARRAY, and env under `environment` (NOT `env`).
A server entry copied from the Claude shape loads zero tools and reports no error.
Back the file up and keep `model` plus every server you do not own.
- Actor is `opencode`: `AGENTBOARD_ACTOR=opencode` inside `environment`, plus an
explicit `AGENTBOARD_DB`.
- Dispatch one-shot with `opencode run --standalone --format json --auto`: the flag
keeps a private server so two runs cannot share a session, and the background
service caches config until `opencode reload`.
- In the verify script, a missing `claude-reviewer` wrapper is `WARN` when opencode
is wired and `FAIL` only when it is the sole reviewer.

## Prove the install

Two tiers, both re-runnable:

1. Local (always): claude auth, both configs parse and name distinct actors,
   wrappers executable, `hermes mcp list` shows the servers, board round-trip on a
   throwaway DB (`AGENTBOARD_DB=$(mktemp -d)/probe.db`, then add/claim/done).
2. Paid (~one short print-mode turn): force an MCP tool answer with the shell denied —
   `--allowedTools 'mcp__git-mcp-rs__git_status' --disallowedTools 'Bash'` in a git
   repo, and require the branch name to match reality. A green wrapper start proves
   nothing.

Exit non-zero on any failed local check. A verify script that always exits 0 trains
nobody to read it.

## Package the install as a skill bundle

When the target is another machine whose own Hermes performs the install, ship a
directory plus a zip (SKILL.md, README.md, `scripts/`, `references/`), not prose.
Full shape, installer rules, sandbox test, verify-script design and the report
format: `hermes-agent/references/skill-bundle-export.md`. Two rules specific to
this team bundle: the README must spell out the target commands verbatim
(`unzip -d ~/`, copy into `$HERMES_HOME/skills/<name>`, `chmod +x scripts/*.sh`,
`install.sh --dry-run`, `install.sh`, `verify.sh`), and a helper script that also
lives in an installed skill ships once — note the equivalence instead of forking a
second copy that drifts.

## Sandbox test before shipping an installer

Run it once against a fake home (`HOME=$PWD/.sandbox SKIP_HERMES=1`) with a
pre-seeded extra server in the config, confirm the merge kept it and the second run
is all `unchanged`, then delete the sandbox and validate scripts with `bash -n`. Zip
with `zip -qr`, and check the executable bit survived (`unzip -Z` prints the mode).
