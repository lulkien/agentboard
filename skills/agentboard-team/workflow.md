# How a task moves

One task is one row. There is no enforced chain and no lease: the status and the
assignee say who has it, and the feed says what just happened. The board refuses
stale writes, so two agents cannot silently overwrite each other.

## Statuses and who moves them

| Status | Means | Who sets it |
| --- | --- | --- |
| `todo` | ready, nobody working | manager (creation) |
| `planning` | analyst is planning the row | manager (starts the analyst) |
| `implementing` | claimed and being worked | the holder (`task_claim`), or the analyst/manager handing to the coder |
| `blocked` | cannot continue, question on the row | the holder |
| `reviewing` | work finished, waiting on a reviewer | the coder |
| `done` | verified, evidence on the row | the reviewer, or the coder when the task names no reviewer |
| `canceled` | dropped on purpose | whoever owns the decision, with a reason in a comment |

A coder never closes its own work when the task names a reviewer; it moves the row
to `reviewing` and assigns the reviewer. A reviewer either closes the row or moves it
back to `implementing` and assigns the coder, with findings in a comment on the same
row. Reopening means reusing the row, never filing a second one.

## Transitions

The board enforces only the revision guard, the holder rule and the assignee rule
(`planning`, `implementing` and `blocked` need an assignee); who may move a row is
this contract, not a store check.

| From to | Who | Call |
| --- | --- | --- |
| `todo` to `planning` | manager, when it starts the analyst | `task_update(status="planning", assignee="analyst")` |
| `planning` to `implementing` | analyst, or manager | `task_update(status="implementing", assignee="coder")` |
| `todo` to `implementing` | coder | `task_claim` |
| `implementing` to `reviewing` | coder, or manager with `force`+`reason` | `task_update(status="reviewing", assignee="reviewer")` |
| `reviewing` to `done` | reviewer | `task_update(status="done")` |
| `reviewing` to `implementing` | reviewer (reopen) | `task_update(status="implementing", assignee="coder")` |
| `planning`/`implementing`/`reviewing` to `blocked` | the holder | comment the question, then `status="blocked"` |
| `blocked` to its stage | manager after the answer | `task_update(status=<stage>)` |
| open to `todo` | the holder, or manager with `release`+`force`+`reason` | `release` |
| open to `canceled` | manager | `task_update(status="canceled")` |

The names `in_progress` and `review` are still accepted as input and read as
`implementing` and `reviewing`.

## The loop

1. Manager files one row per unit of work: `task_add` with a title, a `description`
   (context), a `goal` (the command and the output that proves it), and an
   assignee when the owner is known.
2. The seat claims it: `task_claim(id)`. The row now reads `implementing` and that
   seat's name.
3. The seat works, then comments what it changed and what it verified. Comments are
   cheap; the row is the audit trail.
4. Handoff: `task_update(id, if_revision, status="reviewing", assignee="reviewer")`.
   The event carries `recipient`, which wakes the reviewer.
5. Reviewer reads the work, then either `status="done"` with the evidence comment or
   `status="implementing"`, `assignee="coder"` and its findings.
6. Manager reads `board_brief` for groups and counts, `task_epic` for an umbrella's
   direct-child progress, and `task_list` for what is where. Close or drop the
   direct children before closing their umbrella. The manager does not chase
   individual panes while the board is moving. Before creating or reparenting a
   child under a closed umbrella, the manager explicitly reopens the parent, then
   retries. The board refuses the attachment and never auto-reopens the parent.
   Handing out a done or canceled row works the same way: reopen the ancestors
   from the top, then change the row's status and assignee in one update. An
   assignee change that leaves the row closed is refused.

## Waiting without spinning

Waiting belongs to the manager, not the seats. The manager blocks on the rows it
cares about instead of polling:

```bash
agentboard wait 27 --status reviewing --timeout 300    # returns when T27 is review
agentboard wait 12 --epic                           # returns when every child is terminal
agentboard wait 27 --status reviewing,done,blocked --assignee --timeout 300   # also returns on a handoff
```

There is no MCP wait tool: waits run only through the binary. Add `--assignee`
(or a name) when the act-point is a handoff. One wait is at most
300 seconds: the CLI refuses `--timeout` above 300 or below 1, so a manager parks
the wait in the background and re-arms a fresh one when it exits 3. The verdict's
first line is `matched:` or `timeout:` and it writes nothing. A seat never waits:
it does its row, reports it and stops, and the manager prompts it with the next
one. Do not build a feed loop in a seat; that design is retired.

When the manager does read the feed, read it *unfiltered*: only an assignment carries
`recipient`, so `--recipient "$AGENTBOARD_ACTOR"` would never see the comment or
status change that answers a question. Read the page, then act on what it names.

## Questions and blockers

A missing requirement is not a guess:

1. `task_comment(id, body=<one narrow question>)`
2. `task_update(id, if_revision, status="blocked")`
3. report and stop; the manager brings the answer back as a comment from the owner
   of the decision, and wakes you to continue.

## Evidence

`done` means a named check ran and its output is on the row. Put the command and
the line it printed in a comment, in the same call that moves the status when the
tool allows a comment. A claim without evidence is a reopened row later.

## Quota

When a holder disappears, the manager reassigns with `force` and a reason, and the
row keeps its comments so the replacement can resume. See the manual's handoff
section for the exact call.
