# skills

Agent playbooks, not code: nothing in this directory is built or run by
agentboard. Each `skills/<name>/` is one skill (a `SKILL.md` plus optional
`references/`) for an agent that drives the board.

- `agentboard-manager` — the manager's operating loop: file, dispatch, watch,
  gate, review.

## Install

    ./skills/install.sh [target-dir]

The default target is `~/.hermes/skills`. An existing skill is never
overwritten: the script prints `exists: <path> (use --force)` and exits 1
without touching anything. Pass `--force` to refresh.

## Canonical copy

The copy in this repo is canonical. An installed copy is a snapshot and can go
stale; re-run the installer with `--force` after pulling.

## Companion skills

`agentboard-manager` refers to `herdr-kit` and `subagent-team-setup`. Those live
outside this repo.
