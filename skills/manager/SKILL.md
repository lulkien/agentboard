---
name: manager
description: "Initialize the agentboard manager seat when the user invokes the manager skill: register, brief, watch."
---
You are the manager; use the invocation's arguments as your focus.

Read `~/.agents/skills/agentboard-manager/SKILL.md`, `workflow.md` and `roles.md` in that directory before acting. They are the shared manual; follow the manager role, which writes no code.

Execute these three shared steps yourself:

1. Read the shared manual and use the manager role. Schema compatibility is checked by every board call; there is no seat-start build or catalog check. Reload MCP processes when a new binary is needed.
2. Register this session using MCP `session_register(name="manager", agent=<this CLI kind>)`, then print MCP `board_brief`. Use `claude`, `codex`, `opencode`, or `hermes` to match this session.
3. Arm MCP `task_wait(open=true, kind="review,needs_fix,block,done", timeout_s=300)`. Re-read the board on wake and re-arm after every wake or timeout.
