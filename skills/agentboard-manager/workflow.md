# How a row moves

Authority: `PROTOCOL.md`. Diagrams: `docs/workflow.md`. This page is the short form; if it disagrees with `PROTOCOL.md`, `PROTOCOL.md` wins and this page is the bug.

## The chain

Exactly three levels: **implement -> review -> fix**.

- The review row is a child of the implement row. The fix row is a child of the review row. One fix row per review, reused every round.
- A child of a level-3 row, or a re-parent that reaches level 4, is refused (`chain too deep`, exit 1).
- The board files the review row. Never file one by hand. The first move of a parentless row into `review` creates `Review <title>`. A resubmission reuses it.

## The seat's surface, before the loop

A seat asserts the surface it acts with before it works. One command: `agentboard board-build`. One expected number: the `tools=` count it prints, which must equal the number of agentboard tools this seat's own session offers, with the commit at HEAD and no `(stale, ...)` or `(unverified, ...)` suffix. The catalog form of the check is in `roles.md`.

No match, no work: stop, do not claim, do not submit. Report the count and commit the seat holds and the count and commit the command printed. Why: a stale surface silently applies the rules it was built with and reports success, which is worse than a crash. The board refuses the write as well, with `board surface mismatch` naming both counts (`PROTOCOL.md`, Per-write tool count).

## One round

1. Manager files the implement row (`add --epic E`) with an executable goal and dispatches it. `dispatch --to <session>` is silently overridden by the row's assignee: a live registered assignee wins and the command still prints success. To hand the row to another pane, run `edit <id> --assignee reviewer` first, then dispatch bare.
2. Coder: `task_claim`, work, `diff_add`, `task_update report= status=review`.
3. Manager reads `children T<impl>` for the review row and dispatches the reviewer to it. The tree stays frozen while the reviewer reads.
4. Reviewer reads the newest diff. Clean: review row `done`. Findings: it files the fix row (`--parent` = review row) and sets the review row `needs_fix`.
5. Coder fixes only the listed findings, `diff_add` on the fix row, answers each finding in `report`, sets the fix row `review`.
6. Reviewer re-checks its own findings. All closed: fix row `done`, then review row `done`. Any open: `note` naming it, fix row `needs_fix`, back to 5.
7. Manager reads the report against the goal, closes the implement row `done`, and commits explicit paths only after the owner authorizes it; never pushes.

The coder's round ends at `review`, with a diff; it never ends at a commit. Coder and reviewer seats never commit, push, or install. The reviewer reads the diff, not the history: a commit buys the round nothing and costs the team a history it did not agree to. Committing belongs to the manager, with the owner's authorization.

## The diff gate

A move into `review` is refused unless a diff row was recorded on that task after its newest claim. The refusal names `agentboard diff add T<n>`. The body is the complete change itself, verbatim. A round that changed nothing records a body saying so and why. The board never reads the body; the reviewer rejects one that is neither a diff nor a stated no-change reason.

## The reopen cap

Three moves into `needs_fix` per row, never reset. The fourth is refused (exit 1) and the row lands `not_done`. An open parent lands `not_done` in the same transaction, one level only. A capped fix row stops its review row; the implement row stays `review` for the manager. `get` shows `reopens: N/3`.

`not_done` is not claimable and nothing re-arms it. The manager drops the rows (`status T<n> dropped`) or files a replacement implement row. `status ready` keeps the count.

## Status meanings and owners

| Status | Meaning |
|---|---|
| `ready` | claimable |
| `working` | claimed, lease live |
| `blocking` | waiting on a human (`task_ask`) |
| `review` | on an implement row: ready for the reviewer. On a fix row: fixed, awaiting re-check |
| `needs_fix` | the reviewer sent it back with findings |
| `not_done` | reopen cap hit; set by the board only |
| `done` | verified |
| `dropped` | abandoned |

| Who | Moves |
|---|---|
| Coder | `ready`/`needs_fix` -> `working` -> `review`. Nothing else. |
| Reviewer | `review` -> `done` or `needs_fix`, on review and fix rows. Never on the implement row. |
| Manager | `ready` (re-arm a crashed row), `dropped`, `done` on the implement row, epic status. |
| Board | `not_done`, only through the cap. |

A row arriving at `not_done` from a worker was not capped: read `reopens:` first. A worker that needs a human uses `blocking` and `task_ask`.

## Epic boundary

Close the epic when every row is `done` or `dropped`; a `not_done` row blocks closure. This is policy, not a board check. Then commit, then `manager renew`. A fix row is the same cycle: not a renewal point. `renew` does not reload MCP config or start flags: a changed config needs `manager down` and `manager up`.

One epic in flight (owner rule, 2026-10-22): the manager finishes every row of the current epic (`done` or `dropped`) and commits its work before filing or dispatching the next epic's first implementation row. A defect belonging to a later epic may be filed there and left `ready`: filing is not starting. Only a defect that blocks the current epic's remaining rows joins the current epic for work now; its own epic keeps waiting. Renewal clears the panes; this boundary clears the queue before the team changes context.

## Watching and gating

- Never put an implement row and its review row in one `wait` set: the first status change on either wakes you on the wrong row. Watch the row you just dispatched, with statuses it has not reached.
- A gate that cleans is not a gate. A `clean` step can delete the artifact the gate was meant to inspect and still exit 0. Run read-only gates first; run a wipe as its own step and re-check timestamps before quoting a result.

## Retiring a shell path

A verb replaces a shell path only when the old one is gone from the host too. A repo delete leaves a runnable copy on `PATH`. Closing step: move, do not delete, any leftover wrapper (`mkdir -p ~/.hermes/cache/scratch/retired-helpers-$(date +%Y%m%dT%H%M%S) && mv ~/.local/bin/herdr-* <that dir>/`), then prove it: `ls ~/.local/bin/herdr-*` returns nothing.
