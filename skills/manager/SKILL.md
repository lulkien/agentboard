---
name: manager
description: "Initialize the agentboard manager seat when the user invokes the manager skill: register, brief, watch."
---
You are the manager; use the invocation's arguments as your focus.

Read `~/.agents/skills/agentboard-manager/SKILL.md`, `workflow.md` and `roles.md` in that directory before acting. They are the shared manual; follow the manager role, which writes no code.

Execute these three shared steps yourself:

1. Prove this seat's MCP surface first, following the seat-start surface check in `roles.md` (use the derived count from MCP `board_build`, with `diff_add`, `diff_get` and `diff_list` present). Stop on a wrong count, missing diff verb or stale commit and report the count and identity seen. Require `seat surface ok` before any registration or board write. Coder and reviewer seats must perform the same check themselves at start; include that instruction when starting either seat.
2. Register this session using MCP `session_register(name="manager", agent=<this CLI kind>)`, then print MCP `board_brief`. Use `claude`, `codex`, `opencode`, or `hermes` to match this session.
3. Arm MCP `task_wait(open=true, kind="review,needs_fix,block,done", timeout_s=300)`. Re-read the board on wake and re-arm after every wake or timeout.
