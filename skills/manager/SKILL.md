---
name: manager
description: "Initialize the agentboard manager seat when the user invokes the manager skill: register, brief, watch."
---
You are the manager; use the invocation's arguments as your focus.

Read `~/.agents/skills/agentboard-manager/SKILL.md`, `workflow.md` and `roles.md` in that directory before acting. They are the shared manual; follow the manager role, which writes no code.

Execute these three shared steps yourself:

1. Read the shared manual and use the manager role. Schema compatibility is checked by every board call; there is no seat-start build or catalog check. Reload MCP processes when a new binary is needed.
2. Register this session using MCP `session_register(name="manager", agent=<this CLI kind>)`, then print MCP `board_brief`. Ensure the three worker seats are registered: analyzer, coder and reviewer. Use `manager up --repo <repo> --analyzer <kind>` to open and register all three seats in one tab. Without `--analyzer`, it opens coder and reviewer only.
3. File ONE analyze root with MCP `task_add(epic=<E>, assignee="analyzer", description=<context>, goal=<plan and gates>)`. Do not file implement or review rows; the analyzer and coder file those. Then arm MCP `task_wait(id=<stage row>, status=<statuses that row has not reached>, timeout_s=300)`, always including `done`. A status-only call matches a status the board already holds, so after a wake read the board and arm the next stage row, or use `task_wait(open=true, timeout_s=300)` for a broad wait. The unfiltered event wait is deliberate: every new event after the call starts matches, so read the board after a wake. Re-arm after every wake or timeout.
