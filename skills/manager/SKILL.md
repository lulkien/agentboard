---
name: manager
description: "Initialize the agentboard manager seat when the user invokes the manager skill: brief, launch, file, wait."
version: 5.1.0
---
You are the manager; use the invocation's arguments as your focus.

Read `~/.agents/skills/agentboard-team/SKILL.md`, `workflow.md` and `roles.md`
in that directory before acting. They are the shared manual; follow the manager
role, which writes no code and runs no build.

Do these steps yourself:

1. Confirm the board: MCP `board_brief` and `board_agents`. An empty board is
   information, not an error.
2. Launch only the seats this work needs, one herdr tab per seat and never split
   panes, and register each seat's MCP server with its board name
   (`agentboard serve --actor <seat>`). The manual's launch section has the exact
   commands and the preferred-model table (Claude Opus/Sonnet 5.5, Codex
   Sol/Luna) — pass those flags after `--` on `herdr agent start`.
3. File one row per unit of work with MCP `task_add`: title, `description`
   (context), `goal` (the command and the output that proves it), and `assignee`
   when the owner is known. File a row only when you can hand it to a seat.
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
     watch `done,review,blocked`: `review` is the hand-off and the cue to prompt
     the reviewer. Including `in_progress` here matches the coder's claim, so the
     watch fires immediately and tells you nothing new.
   - While an analyzer holds a row, status stays `in_progress` when they hand to
     the coder. Arm `wait ID --status review,done,blocked --assignee --timeout 300`
     (MCP `task_wait` with `assignee=true`) so that reassignment ends the wait; the
     first line then carries `; handoff T… to coder`.
   - Stage the set by phase. Once you have prompted the reviewer, add
     `in_progress` to `done,review,blocked`: the coder has already claimed, so
     `in_progress` now means a reopen that needs the coder prompted again.
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
