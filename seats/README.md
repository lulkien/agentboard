# Seat setup

How each CLI seat reaches the board. One global `agentboard` MCP entry per client
(`http://127.0.0.1:8765/mcp`) serves every seat; the seat's name is the
`AGENTBOARD_ACTOR` the seat is launched with, sent as `Authorization: Bearer <actor>`.
The skills in `../skills/` say what a seat may do; nothing in the launch enforces it.

| Path | What | Used by |
|---|---|---|
| `agentboard-manager` | the manager launcher | linked to `~/.local/bin/agentboard-manager` on a full install |
| `systemd/agentboard.service` | shared HTTP daemon | linked to `~/.config/systemd/user/agentboard.service` |

## Install

    ./install.py [--agent AGENT[,AGENT...]] [--force] [--skip-binary]

Run it from anywhere; it lives at the repository root and is the only installer.
`./install.py -h` prints the same usage. Those three flags are the only options.
Once every preflight check passes, it runs `cargo install --path <repo> --locked` to
build and install `~/.cargo/bin/agentboard`; a failed build exits 1 with nothing
else written. `--skip-binary` skips the build. Then it:

- links the six repo skills into `~/.agents/skills` (see `../skills/README.md` for
  which client reads them from where; `~/.claude/skills` is not linked yet);
- on a full install, links the launcher and the user unit (it never enables or
  starts the unit);
- registers one global `agentboard` entry per selected client through the client's
  own command, so no live config is hand-edited: `claude mcp add-json -s user`
  (after `claude mcp remove -s user agentboard`, since Claude refuses a second add),
  `codex mcp add --url ... --bearer-token-env-var AGENTBOARD_ACTOR`,
  `grok mcp add -s user --transport http ... -H 'Authorization: Bearer ${AGENTBOARD_ACTOR}'`
  and `codewhale mcp add --url ... --bearer-token-env-var AGENTBOARD_ACTOR`. A rerun
  replaces the entry and leaves other servers alone. A client missing from `PATH` gets
  a `note:` line; a failing add exits 1 with the client's output. The entry resolves
  `AGENTBOARD_ACTOR` at launch: a seat started without it has no usable agentboard
  server, which is why every role skill's first step calls `board_brief`.

`--force` replaces foreign skill, launcher and unit links; a real directory at a
skill destination is always refused.

### agy (Antigravity CLI)

agy expands no variable in an MCP entry, so its `agentboard` entry is the stdio shim
`agy/agentboard-stdio`: `agy mcp remove agentboard` (a missing entry is fine), then
`agy mcp add agentboard <repo>/seats/agy/agentboard-stdio`, which leaves exactly one
entry and replaces an older `serverUrl` one. The shim inherits the launch environment,
reads `AGENTBOARD_ACTOR` (exit 2 with `agentboard-stdio: set AGENTBOARD_ACTOR` when it is
empty), and POSTs each JSON-RPC line to `${AGENTBOARD_URL:-http://127.0.0.1:8765/mcp}`
with `Authorization: Bearer $AGENTBOARD_ACTOR`. A daemon refusal or a refused connection
comes back as a JSON-RPC error for that request id. agy does not read `~/.agents/skills`,
so the installer links `~/.gemini/config/skills/<skill>` to the canonical
`~/.agents/skills/<skill>` links. It never writes `~/.gemini/antigravity-cli/settings.json`
or onboarding state. `python3 tests/agy_shim.py` proves the shim against a scratch daemon.

Launch an agy seat as its board actor, with the entry prompt in `-i` (so no typing
helper runs):

    env AGENTBOARD_ACTOR=<seat> herdr agent start <seat> --kind agy --pane "$PANE" -- --dangerously-skip-permissions --model gemini-4-neon-high -i '/agentboard-<seat> T<id>'
    hc run --bg --name <seat> --cwd "$REPO" -- env AGENTBOARD_ACTOR=<seat> agy --dangerously-skip-permissions --model gemini-4-neon-high -i '/agentboard-<seat> T<id>'

`agentboard-manager agy` does the same for the manager with `gemini-4-neon-medium` and
`-i /agentboard-manager`. No seat gets a `--settings` deny or sandbox flag; its limits are
written in its role skill. One-time setup a human does first: finish agy's onboarding (the
terms and data-use screen, which stops a seat at "Finish setup"), and make sure the seat's
working directory is in `trustedWorkspaces` in `~/.gemini/antigravity-cli/settings.json`,
or agy asks to trust it. The installer writes neither.

### Leftovers from older installers

Earlier installers built per-role homes at
`~/.local/share/agentboard/seats/<seat>/<client>` (and Codewhale homes at
`~/.local/share/agentboard/codewhale/<seat>`), repo-linked `~/.codex/<seat>.config.toml`
profiles, generated `~/.claude/<seat>-mcp.json` cards and a `# BEGIN AGENTBOARD RULES`
block in `~/.codex/rules/default.rules`. The installer no longer reads, writes or
deletes any of them: they hold sessions and copied credentials. Every run names each
one in a `note: leftover from an older installer` line; delete them by hand once no
seat still runs from them. The repo's per-seat cards and profiles are gone, and
`codewhale/status-hooks.toml` has no consumer until Codewhale status hooks move to
the global config.

## Start the manager in the current pane or terminal

From a herdr pane or an hc terminal at its shell prompt, run:

    agentboard-manager {codex|grok|codewhale|claude|agy}

`agentboard-manager -h` explains the choice below. The launcher starts the client with
`AGENTBOARD_ACTOR=manager` and no home variable, so the client reads the global
`agentboard` entry `install.py` registered (bearer token from `AGENTBOARD_ACTOR`).

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

The command replaces the shell in the current hc terminal with the selected
manager (`exec`), in the current directory, as the actor `manager`; it opens
no new terminal and prints nothing. It requires `HC_TERMINAL_ID` (set inside every
hc terminal), `hc`, `jq` and `setsid`; it needs no window and never calls
`herdr-app`. hc names a terminal only at `hc run --name`, so the manager keeps this
terminal's address (`$HC_TERMINAL_ID`) and no name. Grok and Codewhale cannot take
their entry prompt in argv, so a detached helper waits for this terminal and sends
`/agentboard-manager`, retrying up to ten times while hc answers `still starting`.
A reply that is refused or not started is never resent, because the screen has to
be read first; it is written to
`${XDG_RUNTIME_DIR:-/tmp}/agentboard-manager-start.log`, and you type
`/agentboard-manager` yourself.

Seats start hidden and the launcher adds no label itself. No `--label` goes on
`hc run` unless the human asks to watch that seat. The opt-in, only when the human asks:
`hc label <address> herdr.space=<space-id> herdr.tab=<tab-id>` (only when the human asks); the terminal then
joins the person's window, and `herdr-app do agent.focus --terminal <address>`
shows it. The manager copies both ids from its own terminal with
`hc get <its address> --json` (the `labels` field). The launcher never touches the
window, so it also runs on a headless machine.

## What every seat carries

- One entry, named `agentboard`, at `http://127.0.0.1:8765/mcp`; the actor is the
  bearer token, taken from `AGENTBOARD_ACTOR` by every client's own expansion.
- The board tool set is the twelve tools of this build: `task_add`, `task_list`,
  `task_get`, `task_epic`, `task_update`, `task_claim`, `task_comment`,
  `board_feed`, `board_brief`, `board_agents`, `board_project_delete`,
  `board_agent_delete`.
- Tool names, never home paths: `git` and `ferris-search` resolve on the seat's `PATH`.
- Worker launches are `env AGENTBOARD_ACTOR=<seat> <client>` plus model, effort and
  the client's unattended flag; `skills/agentboard-team/SKILL.md` and
  `skills/agentboard-manager/SKILL.md` carry the recipes. No seat gets a home
  variable, sandbox profile or deny flag.

## Verified or not

The installer suite checks the exact `mcp add` commands, a rerun against the real
CLIs (one entry, same shape, other servers kept), refusals, link handling, and that
old role homes stay byte-identical while being named as leftovers.
