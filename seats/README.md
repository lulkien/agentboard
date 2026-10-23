# Seat cards

How each agentboard seat is started, kept in the repo so the checkout is the
source of truth and a review can read the whole surface without logging into a
home directory.

| Path | Seat | Loaded by | State |
|---|---|---|---|
| `codex/<role>.config.toml` | coder, analyzer, reviewer, manager | `codex -p <role>` | live, verified |
| `codex/base.config.toml` | every codex seat | merged into `$CODEX_HOME/config.toml` | live, verified |
| `claude/coding-mcp.json`, `claude/reviewer-mcp.json` | coder, reviewer | `claude --mcp-config=...` (argv built by `manager up`) | linked, unverified — claude quota is out |
| `opencode/opencode.json` | any opencode seat | `opencode --standalone` | copied, unverified — the `opencode` binary is not installed on this box |
| `hermes/manager.config.yaml` | the manager seat | the human starting Hermes | extract, applied by hand |

`coding` is not a typo: `manager up` names the coder role `coding` because that is
what the claude config was always called, and maps it to the codex profile and
board actor `coder`.

## Install

One physical copy: these files, and links in the home directory pointing at them.

    # codex role cards: link, never copy, so an edit lands in the repo
    repo=$(git rev-parse --show-toplevel)
    for role in coder analyzer reviewer manager; do
      ln -sfn "$repo/seats/codex/$role.config.toml" \
              ${CODEX_HOME:-$HOME/.codex}/$role.config.toml
    done

    # claude seat MCP configs, same rule
    for f in coding-mcp.json reviewer-mcp.json; do
      ln -sfn "$repo/seats/claude/$f" "$HOME/.claude/$f"
    done

    # shared codex servers, approvals: merge seats/codex/base.config.toml
    # into ${CODEX_HOME:-$HOME/.codex}/config.toml (host state stays there)

`agentboard manager up` preflights `${CODEX_HOME:-$HOME/.codex}/<role>.config.toml`
and `$HOME/.claude/<role>-mcp.json`, and refuses by exact path when one is
missing, so a stale link is a startup refusal, not a silently wider seat.
`tests/manager.sh` writes its own fake profiles under a scratch `HOME`, so the
suite cannot trample these links.

Two cards are deliberately not linked:

- `opencode/opencode.json` — opencode rewrites its own config (the home directory
  holds `.bak` files from it), and a rewrite replaces the file, which would break
  a symlink. Copy it to `~/.config/opencode/opencode.json` and re-copy after any
  change here.
- `hermes/manager.config.yaml` — an extract, not a profile. A Hermes profile also
  carries model providers and their key env names, custom providers, onboarding
  flags, telemetry and bookkeeping lists; those are host state and stay in
  `~/.hermes/config.yaml`, which is the union of this extract and that state.

`codex/base.config.toml` names its MCP servers by command, not by path, and codex
resolves them on the seat's PATH: measured, `command = "agentboard"` starts the
server and a `board_brief` call answers. The seat's PATH needs `~/.cargo/bin`
and `~/.local/bin`, which the shell that starts a pane already has.

## No home paths, on purpose

Every card names its commands — `agentboard`, `git-mcp-rs`, `ferris-search`,
`codebase-memory-mcp` — and never `/home/<someone>/.cargo/bin/...`, so the same
files work on another machine or another user's account. Two consequences worth
knowing:

- What a seat runs is whatever that name resolves to on its PATH. A stale
  `agentboard` install is then a fact about the seat, not about this directory,
  and `readlink -f "$(command -v agentboard)"` is how you check it.
- The codex cards were verified with names resolving through PATH. The claude,
  opencode and hermes cards were rewritten to the same rule and are unverified,
  for the reasons in the last section.

## Why the cards look like this

Each of these was measured on this box against codex-cli 0.163.0 on 2026-10-23,
and each one is a trap that bit us once:

1. **Shell is a tool, and it can be removed.** `[features] shell_tool = false`
   (plus `unified_exec = false`) takes command execution away entirely: the seat
   answers `NO SHELL TOOL AVAILABLE` instead of running `rg` or `ls`. It does not
   take away writing: `apply_patch` still creates files, verified with the shell
   off. What it does take away is building and running a suite.
2. **A key written after a table header belongs to that table.** The coder's
   `developer_instructions` sat below `[notice]`, so codex loaded
   `notice.developer_instructions` and ignored it with a warning on every start:
   the rule card was never in force. Top-level keys go before the first `[table]`.
3. **Server-level `approval_mode` is not a key.** codex names
   `mcp_servers.<server>.approval_mode` as ignored on every run; per-tool keys
   work, and the server-wide form is `default_tools_approval_mode`. Under
   `approval_policy = "never"` a tool with no approval mode is a hard error, not
   a prompt, which is why every server carries one.
4. **A sandbox is only as good as the pane's cwd.** `workspace-write` allows the
   working directory and `/tmp` and nothing else, so an analyzer rooted in the
   checkout can write the checkout. Rooted in a scratch directory it can probe
   (fixture, CLI state, `$TMPDIR`) while the repo answers
   `read-only filesystem`, exit 1. That pair is the analyzer's wall.
5. **Filesystem writes and sockets are different permissions.** The analyzer can
   write scratch after (4) and still cannot open a local socket, so a CLI that
   starts its own service cannot be probed from it yet.

## What this set does not yet settle

- The coder has no shell, so nothing in it can compile or run a suite. Who runs
  the gate — the reviewer with a wider sandbox, a pinned-command wrapper, or a
  small MCP server exposing only `cargo` and the repo scripts — is T307's open
  question.
- The analyzer's scratch root is an exception to "the analyzer writes nothing";
  T309 owns whether it stays that way.
- Only the codex cards are verified. The claude cards cannot be run while the
  weekly quota is out (resets 2026-10-25), and opencode is not installed here.
  Nothing in this directory should be read as "this CLI was proven" until it is.
- The claude coder card carried `AGENTBOARD_ACTOR = "claude1"` — a name from a
  seat that no longer exists, so its board events would not have been written as
  `coder`. It now says `coder`, matching what `manager up` puts in the pane env;
  that fix is unverified for the same quota reason.
- There is no claude analyzer card (`~/.claude/analyzer-mcp.json` is missing), so
  `manager up --analyzer claude` refuses today. Filling it in is T311's job.
- The opencode card sets its board actor to `opencode`, not to a role, because it
  is one global file. Per-role opencode cards are part of T311 as well.
