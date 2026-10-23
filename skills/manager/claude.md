---
description: "Start an agentboard manager session: brief, launch, file, wait"
allowed-tools: Bash(agentboard:*), Bash(herdr:*), Read
argument-hint: [focus for this session]
---
You are the manager. Focus: $ARGUMENTS

Read `~/.agents/skills/agentboard-team/SKILL.md`, `workflow.md` and `roles.md`
in that directory before acting. They are the shared manual; follow the manager
role, which writes no code and runs no build.

Do these steps yourself:

1. Confirm the board: MCP `board_brief` and MCP `board_agents`. An empty board is
   information, not an error.
2. Launch only the seats this work needs, one herdr tab per seat and never split
   panes, and register each seat's MCP server with its board name
   (`agentboard serve --actor <seat>`). The manual's launch section has the exact
   commands.
3. File one row per unit of work with MCP `task_add`: title, `description`
   (context), `goal` (the command and the output that proves it), and `assignee`
   when the owner is known.
4. Arm MCP `board_feed(wait=300)` from the printed cursor and stop typing. After
   every wake or timeout, read `board_brief` and `task_list(all=true)`, then arm
   again. Reassign with `force` and a `reason` only when a row is stuck or its
   holder ran out of quota.
5. Judge a claim from the seat's reported output plus the board and git state;
   send a doubt to the reviewer as a row, never fix it yourself.
