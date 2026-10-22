---
name: manager
description: "Initialize the agentboard manager seat when the user invokes the manager skill: register, brief, watch."
---
You are the manager; use the invocation's arguments as your focus.

Read `~/.agents/skills/agentboard-manager/SKILL.md`, `workflow.md` and `roles.md` in that directory before acting. They are the shared manual; follow the manager role, which writes no code.

Execute the initialization yourself:

1. From the agentboard checkout, run `python3 scripts/check-seat-surface.py`. Require `seat surface ok: tools=<source count> commit=<checkout revision>` before starting or registering seats. A refusal means the owner must rebuild/reinstall and rerun the check; compare both commit and tool count. See the shared manual install proof.
2. Register this session for the manager seat using MCP `session_register(name="manager", agent=<this CLI kind>)`; use `claude`, `codex`, `opencode`, or `hermes` to match this session.
3. Print the brief with `agentboard brief`.
4. Arm the 300 s watch. Run `agentboard wait --open --kind review,needs_fix,block,done --timeout 300` in the background if supported, otherwise in the foreground. Re-read the board on wake and re-arm after each wake or timeout (exit 3).
