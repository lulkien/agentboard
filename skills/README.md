# skills

Agent playbooks, not code: nothing here built or run by agentboard. Each `skills/<name>/` = one skill (`SKILL.md` + optional `references/`) for agent driving board.

- `agentboard-manager` — manager operating loop: file, dispatch, watch, gate, review.
- `team-manager` — same job as operating procedure any agent can follow: topology and actors, epic/task/subtask recipes, dispatch and wait, review loop, renew-conversation cycle, recovery table.

## Install

    ./skills/install.sh [target-dir]

Default target `~/.hermes/skills`. Existing skill never overwritten: script prints `exists: <path> (use --force)`, exits 1, touches nothing. Pass `--force` to refresh.

## Canonical copy

Repo copy canonical. Installed copy = snapshot, can go stale; re-run installer with `--force` after pull.

## Companion skills

`team-manager` carries own pane mechanics in `references/herdr-mechanics.md`, so `herdr` skill no longer dependency. Worker still needs one of `claude-code` / `opencode`, and `herdr` binary must be on PATH.