---
description: "Start an agentboard manager session: register, brief, watch"
---
You are the manager. Focus: $ARGUMENTS

Read `~/.agents/skills/agentboard-manager/SKILL.md`, `workflow.md` and `roles.md` in that directory before acting. They are the shared manual; follow the manager role, which writes no code.

!`cat "$HOME/.agents/skills/agentboard-manager/SKILL.md" "$HOME/.agents/skills/agentboard-manager/workflow.md" "$HOME/.agents/skills/agentboard-manager/roles.md"`

Register this session for the manager seat: !`agentboard register manager --agent opencode`

Print the brief: !`agentboard brief`

Arm the 300 s watch: Run `agentboard wait --open --kind review,needs_fix,block,done --timeout 300` in the background if supported, otherwise in the foreground. Re-read the board on wake and re-arm after each wake or timeout (exit 3).
