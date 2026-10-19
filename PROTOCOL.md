# Working the board

Conventions the tool schemas do not encode. Drop this file in the repo (or paste
the rules into `CLAUDE.md` / `AGENTS.md`) once — it is the whole protocol.

1. **The board is the only status channel.** Do not write `NOTES.md`,
   `STATUS.md`, `TASK.md` or a summary file. If you want the next agent to know
   something, `task_update(note=...)`.
2. **Start with `task_claim` and no id.** Take the next ready task. If it says
   `no ready tasks`, stop — do not invent work.
3. **One task at a time per session.** Finish or block it before claiming again.
4. **Notes are for the next reader, not a diary.** Decisions, surprises,
   commands worth rerunning. Not "now reading the file".
5. **Never guess a missing requirement.** `task_ask` with ONE narrow question,
   then `task_wait(after_seq=<#seq from the ack>)`. If the answer only covers
   part of what you asked, ask again — do not fill the gap yourself.
6. **`done` means verified.** Run the check the spec names and put its actual
   output in `result`. If there is no check, say how you verified it.
7. **`review` when a human should look** (API changes, anything irreversible).
   `done` when the verification is enough on its own.
8. **Long task? `task_update(heartbeat=true)`** before the lease runs out
   (30 min default) or another session will reclaim the work.
9. **Touch only your task.** Do not claim, block or complete someone else's;
   `session_tail` to see what another pane is doing instead.

## Cost note

The full tool surface is ~1.8k tokens per request; `task_claim` returns the spec
so a follow-up `task_get` is never needed. Writes ack in one line on purpose.
