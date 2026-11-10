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
| `grok/agentboard-mcp.toml` | grok (manager) | managed block in `~/.grok/config.toml` by `install.py` |
| `systemd/agentboard.service` | shared HTTP daemon | linked to `~/.config/systemd/user/agentboard.service` |

## Install

    ./seats/install.py --force

It links each codex profile to `${CODEX_HOME:-~/.codex}/<seat>.config.toml` and
each claude card to `~/.claude/<seat>-mcp.json`. The codex base config is not a
link: it is merged into `$CODEX_HOME/config.toml`, where host state (`[tui]`,
`[projects.*].trust_level`, `[notice]`) also lives. `install.py` refuses an existing
file that is not a symlink into this repo, and `--force` replaces only the named
destinations. A full install also links `seats/start-agentboard-manager` to
`~/.local/bin/start-agentboard-manager`; `--grok-only` manages only the Grok MCP
block and does not install the launcher.
The user unit is linked, never enabled or started. Start the daemon before launching
HTTP seats and before merging these cards into a live checkout. Its default address
is `127.0.0.1:8765`; check `/api/health` for the board path and instance. The unit
uses the default board path. If the manager uses `AGENTBOARD_DB`, give the unit the
same absolute path with a systemd user drop-in. Stop the daemon before resetting
or migrating the board, then start it again.

## Start the manager in the current pane or terminal

From a herdr pane or an hc terminal at its shell prompt, run:

    start-agentboard-manager {claude|codex|grok}

The launcher picks by environment: herdr when `HERDR_ENV=1` (it wins when both
worlds are present), else hc when any `HC_*` variable is set, else it refuses and
starts nothing.

### herdr

The command renames the current tab to `manager`, then waits in the background
until the pane returns to its shell prompt before starting the selected manager
with `herdr agent start`. It requires `HERDR_ENV=1`, `HERDR_PANE_ID`, `HERDR_TAB_ID`, `herdr`,
`jq` and `setsid`. Start details and the Grok first prompt are recorded in
`${XDG_RUNTIME_DIR:-/tmp}/agentboard-manager-start.log`.

### hc

The command starts the selected manager as a new hc terminal named `manager` in
the current directory, then prints `manager <address>`. It requires
an `HC_*` variable (set inside hc), `hc` and `jq`; it needs no window and never calls
`herdr-app`. Grok cannot take its entry prompt in argv, so the launcher waits for
that terminal and sends `/agentboard-manager`; a reply that is refused or not
started is written to `${XDG_RUNTIME_DIR:-/tmp}/agentboard-manager-start.log` and is
never resent, because the screen has to be read first.

Seats start hidden and the launcher adds no label itself. A terminal joins the
person's window only when it carries `--label herdr.space=<space-id>` at launch, or
gets `hc label <address> herdr.space=<space-id>` afterwards; either way the person
asks for it, and `herdr-app do agent.focus --terminal <address>` then shows it. The
launcher never touches the window, so it also runs on a headless machine.

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

- Commands, never home paths: `git`, `ferris-search`,
  `codebase-memory-mcp` resolve on the seat's `PATH`. Check the one that matters
  with `readlink -f "$(command -v agentboard)"`.
- Claude and Codex cards connect to `http://127.0.0.1:8765/mcp/<seat>`; Grok
  connects to `http://127.0.0.1:8765/mcp/${AGENTBOARD_SEAT:-manager}`. The URL
  names the actor on every request. Keep `AGENTBOARD_SEAT` nonempty when set.
- The board tool set is the ten tools of this build: `task_add`, `task_list`,
  `task_get`, `task_epic`, `task_update`, `task_claim`, `task_comment`,
  `board_feed`, `board_brief`, `board_agents`.
- One actor per card. A base config must never carry `AGENTBOARD_ACTOR`: every
  seat that reads it would write under that name.

## Codex

`approval_policy = "never"` and per-role filesystem permissions, configured with
`sandbox_mode` or a named permissions profile, plus per-tool
`approval_mode = "approve"` in the base config. Under `never` a tool with no
approval mode is a hard error, and so is a tool left on `auto`: a seat's board reads
(`board_feed`, `board_brief`, `task_epic`) must be `approve`. The base lists every
board tool.

- analyst: the `seat-scratch` permission profile keeps the repository readable
  and permits writes only under `/tmp/agentboard/scratch`; create that directory
  with mode `700` before launching the seat, which stays rooted in the repository.
- coder: `danger-full-access` with the shell on (owner correction 2026-10-23: builds
  and tests are this seat's job; prefer MCP for reading and searching). Writes
  outside the repo are limited to `/tmp/agentboard/scratch/` for scratch
  boards, logs and screenshots. It also
  creates its task branch and makes the one commit that branch carries; it never
  pushes or merges into the working branch.
- manager: `workspace-write`.
- reviewer: `read-only` with the shell on for git reads only. The rule block allows
  `git diff`, `git log`, `git show` and `git status`; the sandbox refuses every
  write. Rules live per `CODEX_HOME`, not per profile, so the coder and manager's
  git write subcommands are in the same file; the reviewer cannot use them because
  its sandbox is read-only. The profile also cannot create scratch boards or run
  tests. It remains unchanged because Codex's documented
  `sandbox_workspace_write.network_access` option is a boolean for outbound
  network access; it has no loopback-only host allowlist, so it cannot permit the
  local dashboard while denying other network access. The expanded verification
  and local dashboard procedure apply to Claude reviewers; Codex reviewers rely
  on the coder's report and screenshots.

## Claude

One `--mcp-config` card per seat. `analyst-mcp.json`, `coder-mcp.json`,
`manager-mcp.json` and `reviewer-mcp.json` use distinct `/mcp/<seat>` URLs, which
name the actor for each request. A Claude manager starts with
`claude --mcp-config ~/.claude/manager-mcp.json`.

## Grok

`./seats/install.py` installs the marked block from `seats/grok/agentboard-mcp.toml`
into `~/.grok/config.toml`. The command is the portable `agentboard` and must be on
the PATH Grok inherits. Its actor comes from `${AGENTBOARD_SEAT:-manager}`: unset
uses `manager`, and a supplied value passes through unchanged. Never set
`AGENTBOARD_SEAT` to an empty string. Grok is manager-only. In herdr, name a second manager's pane with
`herdr tab create --env AGENTBOARD_SEAT=<name>`. In hc the name rides the argv of
its own terminal, because `hc run` has no environment flag:

    hc run --bg --name manager-2 -- env AGENTBOARD_SEAT=<name> grok --trust --always-approve

An empty `AGENTBOARD_SEAT` passes through as empty, and agentboard refuses to write
with `actor required: pass --actor or set AGENTBOARD_ACTOR`.

The installer preserves the rest of the Grok config, its line endings and mode. It
refuses malformed TOML, malformed markers, symlinked/non-regular config, and an
unmarked `[mcp_servers.agentboard]` table (including one written by `grok mcp add`),
even with `--force`. Review and remove or convert that table by hand before retrying;
the installer never removes user text.

Only `grok mcp doctor agentboard` has been proven: it starts the server, completes
the handshake and lists the tool catalog. A real session load has not been proven.
A Grok seat needs
`--trust` (project `AGENTS.md` and project skills load only in a trusted folder) and
`--always-approve` (otherwise a tool call waits on a prompt nobody is watching); both
go after herdr's `--` or the `--` that ends `hc run`'s own options.
`skills/agentboard-manager/SKILL.md` carries the full launch recipe.

## Verified or not

The installer suite checks the linked URL cards and user unit in isolated homes.
The coder report for the HTTP cutover records live Claude and Codex writes against
a scratch daemon; card shape alone is not transport proof.
