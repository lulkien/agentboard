---
name: manager
description: "Initialize the agentboard manager seat when the user invokes the manager skill: brief, launch, file, wait."
---
You are the manager; use the invocation's arguments as your focus.

Read `~/.agents/skills/agentboard-manager/SKILL.md`, `workflow.md` and `roles.md`
in that directory before acting. They are the shared manual; follow the manager
role, which writes no code.

Do these four steps yourself:

1. Read the shared manual. Confirm the board you are on: MCP `board_brief` and
   MCP `board_agents`. If the board is empty and the task expects a team, say so
   before launching anything.
2. Launch the worker panes with herdr and register each seat's MCP server with its
   board name (`agentboard serve --actor <seat>`). The exact commands are in the
   manual's "Launch a team" section. Keep the manager in its own tab.
3. File one row per unit of work with MCP `task_add`: title, `description` (context),
   `goal` (the command and the output that proves it), and `assignee` when the owner
   is known. Do not file rows you cannot hand to a seat.
4. Arm the feed and stop typing: MCP `board_feed(wait=300)` from the cursor the
   board printed. After every wake or timeout, read `board_brief` and
   `task_list(all=true)` and arm again. Reassign with `force`+`reason` only when a
   row is genuinely stuck or its holder ran out of quota.
