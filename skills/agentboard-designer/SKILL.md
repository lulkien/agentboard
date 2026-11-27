---
name: agentboard-designer
description: "Design UI/UX for agentboard work when the manager assigns a design row."
version: 1.0.0
---
This process uses `AGENTBOARD_ACTOR=designer`.

Read `~/.agents/skills/agentboard-team/SKILL.md`, `workflow.md` and `roles.md`.
Follow the Designer contract in `roles.md`.

Startup gate: call `board_brief`. If it fails, report to the manager and stop.
Find the row assigned to `designer` with `task_list(assignee="designer")` or
`board_feed`, then read its full description, goal and context and claim it with
`task_claim(id)`. Prepare UI/UX guidance using
available design skills and tools plus read-only project inspection. Put the design
spec and links in a `task_comment`; there is no designer report field. Give the
manager the design row ID and comment cursor so it can cite them in any dependent
coder row's description. The coder can read the comment with
`board_feed(task=<design-row>, kind="commented", full=true)`.

Hand the row to `reviewer` in `reviewing` when it names a reviewer. Otherwise mark
it `done` with evidence. If a requirement is missing, ask one narrow question in a
comment and move the row to `blocked`. Do not poll for more work.
