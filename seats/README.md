# Seat cards

How each CLI seat reaches the board, kept in the repo so the checkout is the source
of truth and a review can read the whole surface without logging into a home
directory. The skills in `../skills/` say what a seat may do; these cards say how
its client gets a board identity.

| Path | Seat | Loaded by |
|---|---|---|
| `~/.local/share/agentboard/seats/<seat>/codex/` | analyst, coder, manager, reviewer | `CODEX_HOME=... codex -p <seat>` |
| `codex/<seat>.config.toml` | analyst, coder, manager, reviewer | generated into each isolated role home; legacy `~/.codex` links remain for compatibility |
| `codex/rules/agentboard.rules` | legacy shared Codex home | marked block inside `${CODEX_HOME:-~/.codex}/rules/default.rules`; isolated role homes do not copy host rules |
| `claude/<seat>-mcp.json` | analyst, coder, manager, reviewer | merged into role `mcp.json`; legacy card remains at `~/.claude/<seat>-mcp.json` |
| `grok/agentboard-mcp.toml` | grok (all seats) | rendered into private role homes by `install.py` |
| `systemd/agentboard.service` | shared HTTP daemon | linked to `~/.config/systemd/user/agentboard.service` |

## Install

    python3 skills/install.py --force
    ./install.py --force

For Codex, it reads `~/.codex/config.toml` and `~/.codex/auth.json` without
changing them, validates both, then creates a private role home at
`~/.local/share/agentboard/seats/<seat>/codex`. Only those two base files and the
role profile are copied; rules, sessions, logs, caches and SQLite files are not.
The home is mode `0700`, copied files are mode `0600`, and each role profile
overlays its own agentboard URL and permission policy. Launches set `CODEX_HOME`
to this role home. Existing `${CODEX_HOME:-~/.codex}/<seat>.config.toml` links and
the shared rules block remain managed for compatibility, but launches do not use
them. If `CODEX_HOME` points at a generated role home, compatibility writes stay
under `~/.codex`; the installer removes legacy profile links and managed rules it
finds in that restricted home. Missing base auth refuses generation, and a forced
refresh removes stale private snapshots when the base auth file has been revoked.
Generated role homes are not replaced without `--force`; an unmanaged home is
refused and must be moved aside. Claude role homes are generated at
`~/.local/share/agentboard/seats/<seat>/claude`; install canonical skills first
with `python3 skills/install.py`. The legacy `~/.claude/<seat>-mcp.json` files
remain generated for older launch commands. Repo card links
from older installs are safely replaced; `--force` replaces foreign links and
unmarked regular files at the named seat destinations. Directories and other
non-regular destinations are refused before any install writes. Each Codex seat
profile carries its own tool list and permissions. `--force` replaces only the
named destinations. A full install also links `seats/start-agentboard-manager` to
`~/.local/bin/start-agentboard-manager`; `--grok-only` manages only Grok role
homes and does not install the launcher. Use `--skip-grok` to install or refresh
everything else without reading or changing `~/.grok`; it is mutually exclusive
with `--grok-only`.
Use `--agent codex,codewhale,claude,grok` to install only the named clients; names
are exact lowercase values and cannot be repeated. Selected runs preflight only
those clients, and do not manage the shared launcher or systemd unit. A provider
override is accepted with the default full install or when `codewhale` is selected.
The user unit is linked, never enabled or started. Start the daemon before launching
HTTP seats and before merging these cards into a live checkout. Its default address
is `127.0.0.1:8765`; check `/api/health` for the board path and instance. The unit
uses the default board path. If the manager uses `AGENTBOARD_DB`, give the unit the
same absolute path with a systemd user drop-in. Stop the daemon before resetting
or migrating the board, then start it again.

Every role profile sets only `default_tools_approval_mode = "approve"` for
`codebase-memory-mcp`; the server itself is the copied base `~/.codex/config.toml` entry
that `codebase-memory-mcp install` writes. Without that entry Codex refuses to load
any seat profile (`invalid transport` in `mcp_servers.codebase-memory-mcp`); without
the approval, `approval_policy = "never"` refuses every call.

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
person's window only when it carries both labels at launch,
`--label herdr.space=<space-id> --label herdr.tab=<tab-id>`, or gets
`hc label <address> herdr.space=<space-id> herdr.tab=<tab-id>` afterwards; either
way the person asks for it, and `herdr-app do agent.focus --terminal <address>` then
shows it. The manager copies both ids from its own terminal with
`hc get <its address> --json` (the `labels` field). The launcher never touches the
window, so it also runs on a headless machine.

The legacy shared Codex rules are a block, not a link, because Codex appends its own allowed
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

- Tool names, never home paths: `git` and `ferris-search` resolve on the Claude
  seat's `PATH`.
- Claude, Codex and generated Grok role cards connect to
  `http://127.0.0.1:8765/mcp/<seat>`. The URL names the actor on every request.
- The board tool set is the twelve tools of this build: `task_add`, `task_list`,
  `task_get`, `task_epic`, `task_update`, `task_claim`, `task_comment`,
  `board_feed`, `board_brief`, `board_agents`, `board_project_delete`,
  `board_agent_delete`.
- One actor per card, named by the seat-specific MCP URL.

## Codex

Each seat profile defines its own approval policy, filesystem permissions, and
board tool list. Launchers point `CODEX_HOME` at that seat's generated home, so
the reviewer and analyst do not inherit command rules from the shared host
`~/.codex/rules/default.rules`.

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
- reviewer: `reviewer-scratch` extends `:read-only`, grants writes only under
  `/tmp/agentboard/scratch`, and enables networking behind `network_proxy` for
  loopback test listeners while blocking outbound access. Its rule block allows
  only `rtk git diff/log/show/status`. Host command rules with `decision="allow"`
  run outside every sandbox, so keep the repo rule block narrow. The reviewer can
  run `bash scripts/gate.sh --prebuilt` against the coder's release binary; see
  `skills/agentboard-reviewer/SKILL.md` for the scratch environment and suite
  commands. Cargo is prohibited by reviewer instructions, not refused by the
  sandbox: the host `~/.codex/rules/default.rules` cargo allow rule runs outside
  every sandbox. The RTK pre-tool rewrite's effect on that match is unverified.
  Other rtk reads (`rtk ls`, `rtk read`, `rtk find`, `rtk rg`) run inside the
  read-only sandbox without an allow rule; an allow rule would run them outside it,
  where `rtk find -exec` and `rtk rg --pre` can write.
  Dashboard browser verification remains Claude-only.

## Claude

`install.py` generates `~/.local/share/agentboard/seats/<seat>/claude` for each
role. Set `CLAUDE_CONFIG_DIR` to that directory and use its `mcp.json` with
`--strict-mcp-config`, so only the generated merged MCP card is loaded. Base
`~/.claude/settings.json`, `.credentials.json`, and `~/.claude.json` are read-only;
global settings and credentials are copied with private permissions, and user
MCP entries are merged with the role card taking precedence. Session history,
caches, logs, project settings/approvals and plugins are not copied. Canonical
agentboard seat skills are linked from `~/.agents/skills` after running
`skills/install.py`. User plugin enablement settings are retained, but plugin
records and bundle files are not copied; install user plugins separately in
each role home. The legacy card is retained for old launch commands.
When base authentication is withdrawn, rerun with `--force` to remove copied
`.credentials.json` snapshots whose bytes still match the installer's private
SHA-256 ownership marker. Unmarked credentials, including logins created in a
role home and credentials changed since copying, are preserved and a warning
asks you to inspect and remove them manually if stale. The installer never
quarantines or deletes credentials whose ownership it cannot prove.
Generated files are mode 0600, output lists server names only. A differing
generated file is preserved unless `--force` is supplied;
`--force` replaces an unmarked regular file or foreign symlink at that exact seat
destination without following its target. Directories and other non-regular
destinations are refused. Example:
`env CLAUDE_CONFIG_DIR=~/.local/share/agentboard/seats/manager/claude claude --mcp-config ~/.local/share/agentboard/seats/manager/claude/mcp.json --strict-mcp-config`.

## Codewhale

`install.py` generates one home per seat, `~/.local/share/agentboard/seats/<seat>/codewhale/`,
holding `config.toml` (from `seats/codewhale/<seat>.config.toml`) and `mcp.json`
(the same `/mcp/<seat>` URL as the Claude card). Effort, yolo and sandbox live in the
config, so they are per seat. The installer reads `~/.codewhale/mcp.json` and
`~/.codewhale/config.toml` as optional, read-only bases; it preserves other MCP
servers and top-level fields, while the generated agentboard entry wins. User
provider/model settings carry into each seat, with seat policy, hooks and MCP path
overriding them. `--codewhale-provider NAME` overrides a base provider. Neither
base is rewritten, and credentials are omitted from generated TOML. If a base is
absent, that file uses the existing defaults. Clean managed outputs refresh when
their bases change; hand-edited outputs remain protected by the normal `--force`
rule. If a role-first home is absent, install safely copies regular files from
the legacy `~/.local/share/agentboard/codewhale/<seat>/` home, then regenerates
its owned config and MCP files. The legacy home is read-only: it is never
removed, moved or linked. Unsafe entries refuse before any destination is
published. If the role-first home exists, it is refreshed there and unrelated
content is retained; the legacy home is left alone. `--force` replaces unmarked regular files or foreign symlinks at exactly
`config.toml` or `mcp.json`; it never follows the link, and other symlinks or
non-regular entries in a seat home remain refused. Malformed bases fail preflight
before installed files change. Every seat
sets `reasoning_effort = "medium"` and `yolo = true` and none sets `approval_policy`
(`never` and `auto` deny board calls unattended). Analyst and reviewer add
`sandbox_mode = "workspace-write"`, the reviewer also `sandbox_network_access = true`
(all-or-nothing network, weaker than Codex's loopback-only proxy). Launch those two
with `-C <scratch dir>` so the repository stays read-only.

A hand-edited generated file is kept unless `--force`; clean managed files refresh
when their bases or templates change. Without `--force`, unmarked regular Codewhale
seat config and MCP files and foreign seat-file links are refused; `--force`
replaces those exact entries with regular generated files and never follows link
targets. Directories and other nonregular destinations remain refused. Generated
files and auth artifacts use mode `0600`. The installer uses Codewhale's
value-free `auth list` inventory and
`auth print-api-key` to transfer every runtime-effective API key it can export
through Codewhale's auth CLI into each seat's credential store, including keys for
providers other than the selected one. Missing providers are skipped; OAuth/session
grants and other non-exportable routes are not transferred. Inventory or export
errors refuse before seat files change. Source keys are read-only, and key rotation
is picked up on rerun. A removed source key does not revoke an existing seat key.
Credentials stay out of generated config. The selected base provider, or
`--codewhale-provider NAME`, still controls `config.toml`; the flag overrides the
base provider. With no exportable keys, the installer keeps the no-auth default.
Manual key setup remains available:

    codewhale auth print-api-key --provider P | CODEWHALE_HOME=<home> codewhale auth set --provider P --api-key-stdin

Launch shape (`hc run` forwards no environment, so set it with `env`):

    hc run --bg --name coder --cwd <repo> -- env CODEWHALE_HOME=<home> codewhale --skip-onboarding --fresh --disable subagents

Codewhale does not report status itself, so every generated home appends the hooks
from `seats/codewhale/status-hooks.toml`: session start reports `idle`,
message submit `working`, turn end `done`, as OSC 7501 on the tty. A seat is
ready when `hc wait <seat>` returns; use `hc send --wait` and retry the first send
while hc answers `still starting`. The board stays the authority. Homes written
before the hooks existed need `./install.py --force` to pick them up.

## Grok

`./install.py` leaves the legacy `~/.grok` home unchanged and generates private
homes at `~/.local/share/agentboard/seats/<seat>/grok` for manager, analyst,
coder and reviewer. Each config uses the fixed URL `/mcp/<seat>` and copies only
safe user settings plus `auth.json` and `mcp_credentials.json` when present.
The role home is mode 0700 and copied credentials are mode 0600. Unrelated role
config is preserved on `--force`. Credential files are refreshed
or withdrawn only while they still match the installer's recorded snapshot;
diverged or independently created credentials stay in place. The installer
refreshes only its marked agentboard table in a role config.
Base `auth.json` and `mcp_credentials.json` must each be valid JSON objects. Only
those files are copied as credentials. `~/.grok` is read-only and is never
rewritten.

Launch a worker with a role-specific home and fixed MCP actor:

    env GROK_HOME="$HOME/.local/share/agentboard/seats/analyst/grok" herdr agent start analyst --kind grok --pane "$PANE_ANALYST" -- --trust --always-approve --sandbox read-only --deny Edit
    hc run --bg --name reviewer --cwd "$REPO" -- env GROK_HOME="$HOME/.local/share/agentboard/seats/reviewer/grok" grok --trust --always-approve --sandbox read-only --deny Edit

Use the matching `/agentboard-<seat>` prompt. Coder uses no added read-only
policy; analyst and reviewer use the documented read-only sandbox and Edit deny.

Only `grok mcp doctor agentboard` has been proven: it starts the server, completes
the handshake and lists the tool catalog. A real session load has not been proven.
A Grok seat needs
`--trust` (project `AGENTS.md` and project skills load only in a trusted folder) and
`--always-approve` (otherwise a tool call waits on a prompt nobody is watching); both
go after herdr's `--` or the `--` that ends `hc run`'s own options.
`skills/agentboard-manager/SKILL.md` carries the full launch recipe.

## Verified or not

The installer suite checks generated merged cards, private file modes, idempotence,
safe link replacement and input refusal in isolated homes.
The coder report for the HTTP cutover records live Claude and Codex writes against
a scratch daemon; card shape alone is not transport proof.
