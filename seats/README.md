# Seat cards

How each CLI seat reaches the board, kept in the repo so the checkout is the source
of truth and a review can read the whole surface without logging into a home
directory. The skills in `../skills/` say what a seat may do; these cards say how
its client gets a board identity.

| Path | Seat | Loaded by |
|---|---|---|
| `codex/<seat>.config.toml` | analyst, coder, manager, reviewer | `codex -p <seat>` |
| `codex/rules/agentboard.rules` | every codex seat | a marked block inside `$CODEX_HOME/rules/default.rules` |
| `codex/base.config.toml` | every codex seat | merged into `$CODEX_HOME/config.toml` |
| `claude/<seat>-mcp.json` | analyst, coder, manager, reviewer | `claude --mcp-config <file>` |
| `opencode/agentboard-mcp.sh` | any opencode seat | the global opencode config's `agentboard` entry |
| `grok/register-mcp.sh` | grok (manager) | `grok mcp add --scope user` into `~/.grok/config.toml` |

## Install

    ./seats/install.py --force

It links each codex profile to `${CODEX_HOME:-~/.codex}/<seat>.config.toml` and
each claude card to `~/.claude/<seat>-mcp.json`. The codex base config is not a
link: it is merged into `$CODEX_HOME/config.toml`, where host state (`[tui]`,
`[projects.*].trust_level`, `[notice]`) also lives. `install.py` refuses an existing
file that is not a symlink into this repo, and `--force` replaces only the named
destinations.

The Codex rules are a block, not a link, because Codex appends its own allowed
commands to `$CODEX_HOME/rules/default.rules`. The block is the whole of
`codex/rules/agentboard.rules`, from its `# BEGIN AGENTBOARD RULES` line to its
`# END AGENTBOARD RULES` line:

- No block yet (or no file): it is appended, and every byte already there stays.
- One well-formed block: left alone, reported `unchanged`, with a note if it differs
  from the repo. `--force` rewrites just those lines where they stand.
- Duplicate, missing, reversed or whitespace-padded markers: refused, `--force`
  included, since guessing the span could delete your rules. Fix the file by hand.
- A symlink or non-regular `default.rules` is refused and never written through.
- Line endings (CRLF or LF) and file mode are kept; the write is a same-directory
  temp plus rename, and it aborts if the file changed meanwhile. Install with
  Codex closed.
- An old `rules/agentboard.rules` link into this repo is removed; a foreign link goes
  only under `--force`; a regular file is kept and reported.
- To uninstall, delete the marked lines by hand; there is no uninstall verb.

## What every card carries

- Commands, never home paths: `agentboard`, `git-mcp-rs`, `ferris-search`,
  `codebase-memory-mcp` resolve on the seat's `PATH`. Check the one that matters
  with `readlink -f "$(command -v agentboard)"`.
- The board tool set is the ten tools of this build: `task_add`, `task_list`,
  `task_get`, `task_epic`, `task_update`, `task_claim`, `task_comment`,
  `board_feed`, `board_brief`, `board_agents`.
- One actor per card. A base config must never carry `AGENTBOARD_ACTOR`: every
  seat that reads it would write under that name.

## Codex

`approval_policy = "never"` and per-role `sandbox_mode`, with the per-tool
`approval_mode = "approve"` in the base config. Under `never` a tool with no
approval mode is a hard error, and so is a tool left on `auto`: a seat's board reads
(`board_feed`, `board_brief`, `task_epic`) must be `approve`. The base lists every
board tool.

- analyst: `workspace-write`, opened in a scratch directory, so it reads the repo
  and writes only its probe area.
- coder: `danger-full-access` with the shell on (owner correction 2026-10-23: builds
  and tests are this seat's job; prefer MCP for reading and searching).
- manager: `workspace-write`.
- reviewer: `read-only` with no shell (owner directive: reading and judging only).

## Claude

One `--mcp-config` card per seat, because a Claude seat's actor comes from that
file. `analyst-mcp.json`, `coder-mcp.json`, `manager-mcp.json` and
`reviewer-mcp.json` differ only in `AGENTBOARD_ACTOR`. A Claude manager starts with
`claude --mcp-config ~/.claude/manager-mcp.json`.

## Grok

`grok/register-mcp.sh` writes the server into `~/.grok/config.toml` with
`AGENTBOARD_ACTOR=<actor>` (default `grok`), because Grok keeps one user config and
takes its actor from that entry. `grok mcp doctor agentboard` is the proof: it starts
the server, completes the handshake and lists the tool catalog. A Grok seat needs
`--trust` (project `AGENTS.md` and project skills load only in a trusted folder) and
`--always-approve` (otherwise a tool call waits on a prompt nobody is watching); both
go after herdr's `--`. `skills/agentboard-manager/grok.md` carries the full launch recipe.

## Not shipped here

- `opencode` keeps one global `~/.config/opencode/opencode.json`, so it can carry
  only one board actor. Its `agentboard` entry therefore runs
  `opencode/agentboard-mcp.sh`, which takes the actor from `AGENTBOARD_SEAT` in the
  pane environment (`herdr tab create --env AGENTBOARD_SEAT=coder`) and falls back
  to `opencode`. A seat started without that variable writes as `opencode`.
- The Hermes manager reads `~/.hermes/config.yaml`, which is host state: it holds
  model providers and keys as well as MCP servers.

## Verified or not

The codex cards are checked with `codex -p <seat> mcp list`, which loads the
profile and prints the servers and env that profile resolves. The claude cards are
not exercised here: the account quota decides whether a live seat can be started,
and a card that has not been run is not a proof.
