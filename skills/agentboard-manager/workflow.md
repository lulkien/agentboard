# How a row moves

Authority: `PROTOCOL.md`. Design: `docs/three-task-chain.md`. Diagrams:
`docs/workflow.md`. This page is the short form; if it disagrees with
`PROTOCOL.md`, `PROTOCOL.md` wins and this page is the bug.

## The chain

Exactly three levels: **analyze -> implement -> review**.

- The manager files the analyze root. The analyzer publishes `needs_impl`, files
  the implement child, and dispatches it to the coder.
- The coder implements the plan exactly, records the diff on the implement row,
  and files the review child. The review description must identify both the
  round's patch (`D<n>`) and its analyze plan (`T<n>`). Filing the review child
  publishes the implement row as `needs_review` in the same transaction; the
  coder dispatches it to the reviewer.
- The reviewer claims the review row, reads the newest patch against the plan,
  and records a verdict with `status="review"`.
- The review row has no children, ever. Rework reopens the implement row and
  reuses the same review row. A second child is refused with `new chain T<n>
  already has child T<m>; only one child is allowed`. A fourth level is refused
  with `chain too deep` and the line `a review row takes no child; reopen the
  implement row`.

## One round

1. Manager files one analyze root (`task_add --epic E`) with an executable goal.
   Do not file the implement row or the review row.
2. Analyzer: claim the root, read the source, write the plan, publish it with
   `task_update report=<plan> status=needs_impl`, file the implement child
   (`--parent T<analyze>`), and dispatch it to the coder.
3. Coder: claim the implement row, implement the plan exactly, `diff_add` the
   complete change, file the review child naming the diff and plan, and dispatch
   it to the reviewer. The board refuses the review row if the coder did not file
   it or if it names no diff and plan.
4. Reviewer: claim the review row, read the newest patch (`diff_list`,
   `diff_get`), and set `status=review` with the verdict in `report`.
5. Pass: set the review row `done`. All three rows close together in one
   transaction. The identity that recorded the newest diff cannot close.
6. Fail: set the review row `needs_fix` with a reason. The implement row reopens
   as `needs_fix`, the review row also lands there, and the analyze row stays
   `needs_impl`. On the next round the coder takes the implement row again and
   adds a diff; its `needs_review` publication returns the existing review row to
   `ready`, clearing the old verdict and lease, and keeps one review row per
   chain.
7. Manager: re-run the gates the goal names, read the report, and commit explicit
   paths after the owner authorizes it. Then close the epic. The manager never
   moves the worker statuses for it.

## The diff gate

A move into `needs_review` (or legacy `review`) is refused unless a submission
row was recorded since the newest claim. Two forms satisfy it:

- a **diff**: the complete change itself, verbatim;
- an **evidence-only** submission for a re-run round whose tree did not change:
  `diff_add(id, body=<what you ran and its output>, reviews="D<n>")` (CLI:
  `agentboard diff add T<n> --evidence-only --reviews D<n> --body-file -`). It
  must name the newest patch of this row (same row, no re-parent). The row is
  marked `evidence-only` and is never read as a patch: `diff_get` labels it, and
  the review brief names the patch under review.

A real diff is preferred when both exist. The refusal names both forms. The board
never reads a body; a plain diff whose body is prose is still not admissible — a
round with no tree change records evidence-only instead.

## The reopen cap

Three reopens per implement row, never reset. The fourth rejection is refused
(exit 1) and all three rows land `not_done`. The same review row is reused every
round; a second review row is refused. `get` shows `reopens: N/3`.

`not_done` is not claimable and nothing re-arms it. The manager drops the rows
(`status T<n> dropped`) or files a replacement analyze root with a fresh count.
`status ready` keeps the count.

## Close and reopen

A close is `task_update(status="done"|"not_done")` on the review row after the
verdict. It needs the analyze row at `needs_impl`, the implement and review rows
at `needs_review` or `review`, a verdict, a recorded diff, and an acting identity
different from whoever recorded the newest diff. The close is atomic.

A close made by mistake cannot be undone by the normal status setter: it returns
`closed chain is final; use chain reopen with a different identity and a reason`.
The store has a manager-only reopen with a required reason, a different identity
from the closer, no recorded commit, and the same cap. The current CLI and MCP
surface does not expose that operation; say so rather than teaching a verb the
binary lacks.

## Status meanings and owners

| Status | Meaning |
|---|---|
| `ready` | claimable |
| `working` | claimed, lease live |
| `blocking` | waiting on a human (`task_update status=blocking note=<question>`) |
| `needs_impl` | the analyze plan is published, or the implement row was reopened for coding |
| `needs_review` | the implement row is submitted for review |
| `review` | the reviewer has the row and the verdict is being recorded |
| `needs_fix` | the reviewer rejected this round; the implement row is claimable again |
| `not_done` | the reviewer's terminal abandon verdict, or the chain stopped at the cap |
| `done` | verified |
| `dropped` | abandoned |

| Who | Moves |
|---|---|
| Manager | files the analyze root, re-dispatches a stranded stage, sets epic status, commits after the owner agrees |
| Analyzer | `ready`/`working` -> `needs_impl`; files the implement child; dispatches the coder |
| Coder | `ready`/`needs_fix`/`needs_impl` -> `working` -> `needs_review` (a claimed `working` row publishes directly); records the diff; files the review child; dispatches the reviewer |
| Reviewer | review row `ready` -> `review` -> `done`, `not_done` or `needs_fix` |
| Board | the three-row close, the rejection, and the cap-induced `not_done` |

A `not_done` row with `reopens: N/3` below the cap is the reviewer's abandon
verdict, not the cap. A worker that needs a human uses
`task_update status=blocking note=<question>`.

## Epic boundary

Close the epic when every row is `done` or `dropped`; a `not_done` row blocks
closure. This is policy, not a board check. Then commit, then `manager renew`.

One epic in flight (owner rule, 2026-10-22): the manager finishes every row of the
current epic (`done` or `dropped`) and commits its work before filing or
dispatching the next epic's first analyze row. A defect belonging to a later epic
may be filed there and left `ready`: filing is not starting. Only a defect that
blocks the current epic's remaining rows joins the current epic for work now; its
own epic keeps waiting. Renewal clears the panes; this boundary clears the queue
before the team changes context.

## Watching and gating

- Never put rows from two stages in one `wait` set: the first status change on
  either wakes you on the wrong row. Watch the stage row you just filed or
  dispatched, with statuses it has not reached.
- The manager's watch set includes `needs_impl` and `needs_review` as well as
  `needs_fix`, `blocking` and `done`; a chain waiting on the next seat is not
  stalled just because nothing is `done` yet. A status-only wait returns at once
  when the row already holds a listed status, so arm statuses the stage has not
  reached, or filter on events.
- A gate that cleans is not a gate. A `clean` step can delete the artifact the
  gate was meant to inspect and still exit 0. Run read-only gates first; run a
  wipe as its own step and re-check timestamps before quoting a result.

## Retiring a shell path

A verb replaces a shell path only when the old one is gone from the host too. A repo delete leaves a runnable copy on `PATH`. Closing step: move, do not delete, any leftover wrapper (`mkdir -p ~/.hermes/cache/scratch/retired-helpers-$(date +%Y%m%dT%H%M%S) && mv ~/.local/bin/herdr-* <that dir>/`), then prove it: `ls ~/.local/bin/herdr-*` returns nothing.
