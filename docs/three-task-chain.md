# The three-row chain

This is the design note behind the primary workflow: three seats, three rows, one
unit of work. Wire rules live in [PROTOCOL.md](../PROTOCOL.md).
[workflow.md](workflow.md) owns the diagram and the status-ownership table. This
page explains the shape and the division of responsibility; it does not repeat the
wire rules.

## Seats and rows

The manager owns the board and the team, but does not do the work. Three worker
seats do one unit of work, each on one row. The chain is exactly three levels:

| Row | Filed by | Parent | Status path |
|---|---|---|---|
| Analyze | manager | none | `ready` or `working` -> `needs_impl` -> `done` or `not_done` |
| Implement | analyzer | analyze row | `ready`, `needs_fix` or `needs_impl` -> `working` -> `needs_review` -> `done` or `not_done` |
| Review | coder | implement row | `ready` -> `review` -> `done`, `not_done` or `needs_fix` |

Each row has at most one child. The deepest row is the review, and it never
takes a child of its own. Rework therefore reopens the implement row; the chain
never grows a fourth row.
The row kinds are identified by their statuses: any `needs_impl` or
`needs_review` in a chain selects this workflow, and recorded chain events keep
that identity after terminal statuses replace the published ones.

## What each seat reads and writes

| Seat | Reads | Writes | Prohibitions |
|---|---|---|---|
| Manager | epic, board stage, reports, gates | one analyze root; dispatch to rescue a stalled stage; epic status; commit after the owner agrees | never files an implement or review row; never chases a seat by hand; never edits the tree |
| Analyzer | analyze row, source, history, tests | plan in the analyze row, status `needs_impl`, the implement child, dispatch to `coder` | never edits source or implements; never files the review row; never dispatches outside the next seat |
| Coder | implement row, the analyze plan, source | the implementation, its diff, status `needs_review`, the review child, dispatch to `reviewer` | never extends the plan; never files an analyze row; never closes the chain; never commits or installs |
| Reviewer | review row, the newest patch, the analyze plan, description and goal | a verdict on its review row, status `review`, then a close or `needs_fix` | never edits, builds or tests; never files a row; never changes the implement row directly; never reopens twice for the same defect |

A seat writes only its own row. The manager's exception is a dispatch rescue, and
the board records who dispatched what.

## One round

1. The manager files exactly one analyze root and waits. It does not file the
   implement or review rows.
2. The analyzer claims the root, reads the real source, and puts the plan in its
   `report`. Publishing the plan is `task_update(report=<plan>, status="needs_impl")`.
   The analyzer then files the implement child and dispatches it to `coder`.
3. The coder claims the implement row and implements the plan exactly. The coder
   records the complete change with `diff_add`, then files the review child. The
   review description must name the current patch (`D<n>`) and the analyze plan
   (`T<n>`). Filing the review child atomically publishes the implement row as
   `needs_review`. The coder dispatches that row to `reviewer`.
4. The reviewer claims the review row and reads the newest patch against the plan.
   It records its verdict with `status="review"`.
5. Pass: the reviewer sets the review row `done`. The board closes all three rows
   in one transaction and stores the verdict on the review row.
6. Fail: the reviewer sets the review row `needs_fix` with a reason. The board
   reopens the implement row as `needs_fix`, puts the review row in `needs_fix`,
   keeps the analyze row at `needs_impl`, and counts the reopen on the implement
   row. The coder reclaims the same implement row, records the next diff, and
   publishes `needs_review`; this refreshes the same review row to `ready`. Never
   file a second review row.

The diff history is append-only. Round 2 adds a diff; it does not replace round 1.
A re-run round whose tree did not change records an evidence-only submission that
names the newest patch, exactly as [PROTOCOL.md](../PROTOCOL.md) describes.

## The reopen rule

Rework is a status change on the rows that already exist. The reviewer's
`needs_fix` on the review row is the only rejection door; it moves the implement
row to `needs_fix` and increments its reopen count. The implement row is reusable
through every round and keeps the same review child.

The cap is three reopens. The fourth rejection is refused and converts all three
rows to `not_done`; the cap is final. A capped chain is not re-armed by a status
edit. The manager drops it or files a new analyze root with a fresh count.

A closed chain is final through the normal status setter. The store also has a
manager-only reopen for a chain closed by mistake, with a required reason, a
different identity from the one that recorded the close, and no recorded commit.
It counts against the same cap. The current CLI and MCP surface do not expose
that store operation, so a close cannot be undone through a door this binary
offers yet.

## The close rule

The close is issued on the review row. A close to `done` or `not_done` needs
four things: the analyze row still at `needs_impl`, both later rows at
`needs_review` or `review`, a verdict, and a recorded diff. The acting identity
must differ from the identity that recorded the newest diff: the seat that did
the work may not declare its own work good. The close is one transaction over all
three rows; a failure leaves every row where it was. The board validates the
three-row state, the verdict and the identity, not which row's id carried the
call.

The reviewer owns the verdict in the loop. The manager may close only when it is
not the identity that recorded the newest diff, and it still needs the review
row's verdict. After a close, the manager re-runs the named gates against the
tree, commits the explicit paths the owner authorized, and then closes the epic
once every row is `done` or `dropped`. Committing is the manager's act, not the
reviewer's.

## The manager's narrow role

The manager's job on a chain is to file the analyze root, keep the team alive,
and read the result. It does not file implement or review rows, it does not move
their statuses, and it does not chase a seat by hand. A stalled stage is handled
by reading the board and dispatching the row to a live seat; a dead seat is a
lease and dispatch problem, not a reason to type into another pane.

## Failure modes

| Failure | What it looks like | What to do |
|---|---|---|
| Stalled chain | a stage row is `ready` or `working` with no movement, or a handoff was never dispatched | read the row and its lease; re-dispatch to the live seat; the manager may force a reset |
| Wrong close | a verdict is missing, the closer recorded the newest diff, or a gate was not run | the close is refused by name; if it already closed, no CLI/MCP reopen door exists, so file a new analyze root, or have a store operator reopen before any commit |
| Cap hit | the fourth rejection lands `not_done` on all three rows | stop the loop; drop the chain or file a new analyze root; do not try to reset the count |
| Dead seat | a lease expires or the pane is gone | dispatch the row to a live seat; do not edit the row on the seat's behalf |
| Fork | a second child is attempted, or a second review row is filed | the board refuses it by name; reuse the existing row |

The diagram is in [workflow.md](workflow.md#the-three-row-chain). The exact
status list, refusal text and truncation rules are in
[PROTOCOL.md](../PROTOCOL.md).
