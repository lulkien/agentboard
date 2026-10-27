---
name: agentboard-manager
description: "Initialize the agentboard manager seat (Claude, Codex, Hermes or Grok): brief, launch, file, wait in the background."
version: 6.0.0
---
You are the manager; use the invocation's arguments as your focus.

Read `~/.agents/skills/agentboard-team/SKILL.md`, `workflow.md` and `roles.md`
in that directory before acting. They are the shared manual; follow the manager
role, which writes no code and runs no build.

Do these steps yourself:

When starting yourself from a herdr pane at its shell prompt, use the installed
`start-agentboard-manager claude|codex|grok|hermes` helper. It renames the tab and starts
the selected manager after the pane is ready; `seats/README.md` documents the
requirements and launch behavior.

1. Confirm the board: MCP `board_brief` and `board_agents`. An empty board is
   information, not an error.
2. Launch only the seats this work needs, one herdr tab per seat and never split
   panes, and register each seat's MCP server with its board name
   (`agentboard serve --actor <seat>`). The manual's launch section has the exact
   commands and the preferred-model table (Claude Opus/Sonnet 5.5 with seat
   effort, Codex Sol/Luna with profile effort) — pass those flags after `--` on
   `herdr agent start`.
   Grok Build as the manager: register with `seats/install.py` (or the
   compatibility wrapper `seats/grok/register-mcp.sh`), launch
   with `-- --trust --always-approve`, and reach the MCP tools as
   `agentboard__<tool>` through `use_tool`; `grok.md` in this directory has the
   recipe, probes and traps.
   Hermes uses the host MCP configuration in `~/.hermes/config.yaml` (see
   `seats/README.md`); configure its agentboard actor as `manager`. The launcher
   starts Hermes with `--yolo` and sends `/agentboard-manager` through herdr.
3. File one row per unit of work with MCP `task_add`: title, `description`
   (context), `goal` (the command and the output that proves it), and `assignee`
   when the owner is known. File a row only when you can hand it to a seat.
   Before you hand out a row the human names, `task_get` it. If it is `done` or
   `canceled`:
   - Same objective: reopen and assign in one write,
     `task_update(id, if_revision, status="todo", assignee=<seat>, comment=<the
     human's request>)`. The seat's `task_claim` then starts it. The board refuses
     an assignee change that leaves the row closed (`cannot change assignee`).
   - Different work: file a new row and name the old one in its description.
   - Unclear which: ask the human before writing anything.
   - `canceled`: revive it only when the human explicitly asks.
   - Closed parent: the board refuses with `reopen it before reopening`. Reopen
     the closed ancestors from the highest down (`todo`, or `implementing` when you
     hold it), then reopen and assign the row.
4. Wake, then wait. You are the only wake. The board stores state and blocks on it;
   it never schedules, dispatches or wakes anyone. A seat does its row, reports it
   and stops rather than polling, so every wake after that is yours to give: when a
   seat reports, prompt the seat that holds the next row by name with
   `herdr agent prompt coder 'claim your assigned row, report, stop'`.

   - Never block in the foreground. Send `herdr agent prompt` without `--wait`,
     then arm a background watch with completion notification and come back when
     it fires. A foreground sleep, a poll loop or a blocking prompt is a bug in
     the manager, not diligence.
   - Watch act-points, not the row's current state. While a coder holds a row,
     watch `done,reviewing,blocked`: `reviewing` is the hand-off and the cue to prompt
     the reviewer. Including `implementing` here matches the coder's claim, so the
     watch fires immediately and tells you nothing new.
   - While an analyst holds a row (`planning`), the hand to the coder moves it to
     `implementing` with the coder as assignee. Arm `wait ID --status reviewing,done,blocked --assignee --timeout 300`
     so that reassignment ends the wait; the
     first line then carries `; handoff T… to coder`.
   - Stage the set by phase. Once you have prompted the reviewer, add
     `implementing` to `done,reviewing,blocked`: the coder has already claimed, so
     `implementing` now means a reopen that needs the coder prompted again.
   - Keep one watcher per row. Kill the old arm before arming a new one, because
     two watchers wake you twice for one event.
   - Use a 300 s long-poll arm (`wait ID --status CSV --timeout 300`, and add
     `--assignee` when the act-point is a handoff) in the background. Exit 3 means
     nothing matched; re-arm. Exit 0 means the row reached an act-point. The CLI
     accepts only 1 to 300 seconds: it refuses `--timeout` above 300 and refuses
     zero, so zero is no escape hatch.
   - Never prompt a seat mid-turn. Queue its next row and prompt when the seat
     frees; a live turn is not a mailbox.

   `board_feed(wait=300)` stays for seeing what moved on the whole board; run
   that watch in the background too and re-arm it from the cursor it prints.
   Reassign with
   `force` and a `reason` only when a row is genuinely stuck or its holder ran out of
   quota.
5. Judge a claim from the seat's reported output plus the board and git state.
   Send a doubt to the reviewer as a row; never fix it yourself and never close a
   reviewer's row. Commit locally only when the owner says so, and never push.
6. A big goal is an umbrella row: file it assigned to yourself, file its children
   with `parent`, read its progress with MCP `task_epic` (CLI `epic ID`), and close
   it only when every child is terminal. The board refuses an epic close while a
   child is open.
   When an epic completes, end its seats' agent sessions before the next epic
   starts and never carry their conversations over. Start fresh sessions only
   for the seats the next epic needs, one tab per seat; leave the others stopped.
