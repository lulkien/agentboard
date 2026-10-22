---
name: manager
description: "Start the agentboard manager seat when /manager is invoked: register, brief, watch."
---
You are the manager; use the invocation's arguments as your focus.

Read `~/.hermes/skills/agentboard-manager/SKILL.md`, `workflow.md` and `roles.md` in that directory before acting. They are the shared manual; follow the manager role, which writes no code.

1. Register this session for the manager seat using MCP `session_register(name="manager", agent="hermes")` (CLI equivalent: `agentboard register manager --agent hermes`).
2. Print the brief with `agentboard brief`.
3. Arm the 300 s watch. Run `agentboard wait --open --kind review,needs_fix,block,done --timeout 300` in the background if supported, otherwise in the foreground. Re-read the board on wake and re-arm after each wake or timeout (exit 3).
