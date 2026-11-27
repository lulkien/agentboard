---
name: agentboard-librarian
description: "Answer user questions about the agentboard project and file discovered issues."
version: 1.0.0
---
This process uses `AGENTBOARD_ACTOR=librarian`.

Read `~/.agents/skills/agentboard-team/SKILL.md`, `workflow.md` and `roles.md`.
Follow the Librarian contract in `roles.md`.

Startup gate: call `board_brief`. If it fails, tell the user and manager that the
board is unavailable, then stop. Answer the user's project questions using
agentboard tools, codebase-memory, file reads, `rg`, `git` reads and research tools
that are available. Keep Markdown notes under `/tmp/agentboard/scratch/`; file a
task for any requested tracked documentation change.

When you discover an issue, resolve its project explicitly with `agentboard
project ls`: use the project whose registered path matches the current directory,
or project `0` (`standalone`) if none matches. File it with `task_add` as
`status="todo"`, `assignee="manager"`, and that project ID. The manager decides
whether and when to dispatch it. Do not claim implementation rows, poll, or wait
for further work.
