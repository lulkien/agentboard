# Seat cards

How each agentboard seat is started, kept in the repo so the checkout is the
source of truth and a review can read the whole surface without logging into a
home directory.

| File | Seat | Loaded by |
|---|---|---|
| `coder.config.toml` | coder | `codex -p coder` |
| `analyzer.config.toml` | analyzer | `codex -p analyzer` |
| `reviewer.config.toml` | reviewer | `codex -p reviewer` |
| `manager.config.toml` | manager | `codex -p manager` |
| `base.config.toml` | every seat | merged into `$CODEX_HOME/config.toml` |

## Install

One physical copy: these files, and links in the home directory pointing at them.

    # role cards: link, never copy, so an edit lands in the repo
    repo=$(git rev-parse --show-toplevel)
    for role in coder analyzer reviewer manager; do
      ln -sfn "$repo/seats/$role.config.toml" \
              ${CODEX_HOME:-$HOME/.codex}/$role.config.toml
    done

    # shared servers, guard rails, approvals: merge seats/base.config.toml
    # into ${CODEX_HOME:-$HOME/.codex}/config.toml (host state stays there)

`agentboard manager up` preflights `${CODEX_HOME:-$HOME/.codex}/<role>.config.toml`
and refuses by exact path when it is missing, so a stale link is a startup
refusal, not a silently wider seat.

`base.config.toml` names its MCP servers by command, not by path, and codex
resolves them on the seat's PATH: measured, `command = "agentboard"` starts the
server and a `board_brief` call answers. The seat's PATH needs `~/.cargo/bin`
and `~/.local/bin`, which the shell that starts a pane already has.

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
5. **Sandbox filesystem writes and sockets are different permissions.** The
   analyzer can write scratch after (4) and still cannot open a local socket, so
   a CLI that starts its own service cannot be probed from it yet.

## Open, and owned elsewhere

- The coder has no shell, so nothing in it can compile or run a suite. Who runs
  the gate - the reviewer with a wider sandbox, a pinned-command wrapper, or a
  small MCP server exposing only `cargo` and the repo scripts - is the open
  question on T307.
- The analyzer's scratch root is an exception to "the analyzer writes nothing";
  T309 owns whether it stays that way.
- An installer that performs the linking above (instead of this README telling a
  human to) is T311's row.
- The seats are codex-only here. The claude and opencode cards live in
  `~/.claude/*-mcp.json` and `~/.config/opencode/opencode.json` and have not been
  brought in yet.
