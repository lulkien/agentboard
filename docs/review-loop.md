# Review loop

This document describes the review loop used by this project. [PROTOCOL.md](../PROTOCOL.md) is the authority for it, and [workflow.md](workflow.md) carries the same rules as diagrams.

## Topology

One `herdr` tab rooted in the repository. Two panes:

- **coder pane** (left): exports `AGENTBOARD_ACTOR=opencode-coder`
- **reviewer pane** (right): exports `AGENTBOARD_ACTOR=opencode-reviewer`

Each pane runs its own `agentboard` MCP server. The actor name is recorded in every event so the log names the writer.

> **Current setup**: two opencode panes, `AGENTBOARD_ACTOR=opencode-coder` / `opencode-reviewer`, because the Claude quota is exhausted; any coder/reviewer pair works.

## The loop

The chain is three levels, **implement → review → fix**, and no fourth: the review row is a child of the implement row, the fix row is a child of the review row, and the board refuses a child of a level-3 row. One fix row serves a review across every round.

1. **Manager** creates an epic (`E<n>`) with `epic_add` and an implement row with `epic=E<n>`, and dispatches the coder.
2. **Coder** claims the implement row, works, writes progress with `note`, records the change with `diff_add`, then finishes with `task_update(report=..., status=review)`. The board refuses that move unless a submission was recorded after the newest claim, and the refusal names both forms: a diff, or an evidence-only submission for a re-run round whose tree did not change (`diff_add(..., reviews="D<n>")`, naming the newest patch of this row).
3. **The board** files the review row when a parentless row moves to `review`: one child, `ready`, unassigned, inheriting epic, path, branch and priority, with the report, the diff id the reviewer must read, and one line saying whether the round is a diff or evidence-only. A second submission of the same implement row reuses it and notes the new submission on it. A fix row or a review row never files another.
4. **Manager** reads the report (via `task_get` or `board_brief`, or `task_wait` for server-side blocking) and dispatches the reviewer to the review row.
5. **Reviewer** claims the review row, reads only the newest patch (`diff_list`, `diff_get`) and either:
   - Sets it `done` with a report (no changes needed), or
   - Files one fix row with `parent=<review row id>` (inherits the epic) and sets the review row `status=needs_fix`.
6. **Coder** claims the fix row, fixes the listed findings, records `diff_add` on the fix row, writes `report` with the verdict per finding, and sets the fix row `status=review` (this is the "fixed" state; there is no `fixed` status).
7. **Reviewer** re-checks the new diff against its own findings:
   - If all checks pass: sets the fix row `done` (report carries the evidence), then sets the review row `done`.
   - If findings remain: writes a `note` listing them on the fix row and sets the fix row `status=needs_fix` (the review row stays `needs_fix`). The same fix row is reused.
8. The loop returns to step 6 until the reviewer closes the review row, or the reopen cap stops it (below).
9. **Manager** re-runs the gates the goal names, closes the implement row `done` (only the manager does), and closes the epic with `epic_update(status=done)` once every row is `done` or `dropped`. That last part is policy: `epic_update` does not check the rows, and a `not_done` row blocks closure only because the manager does not close over it.

Only the reviewer sets a fix row `done`. The coder never sets a fix row `done` and never closes a review row. Only the manager closes an implement row.

### Reviewer rules

1. The cap is final; re-scoping belongs to the manager.
2. Review the new diff only: the diff the brief names (the newest patch, or the patch an evidence-only submission re-runs).
3. Classify every finding blocker, follow-up or nit. An uncategorised observation is not a finding.
4. Never re-report a closed finding without new evidence quoted from the current diff.

The board does not verify a diff body, so the reviewer also rejects a prose body submitted as a diff. A round with no tree change records an evidence-only submission instead; it is labelled as such and names the patch to read.

## Fix-row lifecycle

```
ready → working (coder claims) → review (coder records a diff and reports; "fixed" state) → needs_fix (reviewer reopens with note) → working (coder claims again) → done (reviewer sets)
```

States are exactly those in `src/schema.sql`:

```
ready | working | blocking | not_done | needs_fix | review | done | dropped
```

`not_done` is set by the board, not by an agent. The reopen cap is 3: a row's fourth move into `needs_fix` is refused with exit 1 (a tool error over MCP) and the row lands `not_done` instead, the caller's body kept in an event on the row. An open parent of that row (`ready`, `working`, `blocking`, `needs_fix` or `review`) lands `not_done` in the same transaction, one level up only; a `done`, `dropped` or `not_done` parent is left as it is and the capped row gets a note naming it. So a capped fix row stops its review row and leaves the implement row at `review` for the manager. `reopen_count` is never reset, so a `not_done` row is not re-armed: the manager drops the rows, or files a replacement implement row with a fresh count.

The `review` state on a fix row means "fixed, awaiting re-check". The `needs_fix` state means "re-check found remaining work". The reviewer drives the transition from `review` to `done` (on the fix row) or `needs_fix` (on the fix row), and subsequently closes the review row `done`. The coder drives `ready` → `working` → `review`.

## Where text lives

| Field      | Purpose                                               |
|------------|-------------------------------------------------------|
| `description` | Context: repo path, files, situation, constraints |
| `goal`        | How to do it + definition of done                   |
| `report`      | Outcome: findings, verdict per finding, evidence    |
| `note`        | Last progress line (history lives in events)        |

No `NOTES.md` / `STATUS.md` files. The board is the only status channel.

## Diagrams

The loop, the fix-row lifecycle and the reopen cap are drawn in [workflow.md](workflow.md), together with the status-ownership table. That page owns every diagram; this one carries none, so the prose here and the pictures cannot drift.

## Wait mechanism

- **Board-level wait**: `task_wait` (MCP) or `agentboard wait` (CLI) blocks server-side until watched tasks match a status/kind filter. Returns only the delta. No polling by hand.
- **Pane-level watch**: `agentboard tail` / `session_tail` read a herdr pane first (`herdr pane read`), and fall back to tmux; `send`/`session_send` do the same with `herdr pane send-text`. Either bridge's failure is reported with the bridge that failed.
- Never read the board's storage file directly. The board and the pane are the two authoritative sources.

## Status reference

From the `tasks` table in `src/schema.sql`:

```sql
status TEXT NOT NULL DEFAULT 'ready', -- ready|working|blocking|not_done|needs_fix|review|done|dropped
```

Aliases accepted on input (matching `src/store/mod.rs` `norm()`): `running`→`working`, `started`→`working`, `blocked`→`blocking`, `triage`→`ready`, `need fix`/`needs-fix`/`need-fix`/`need_fix`/`needs fix`/`fix`→`needs_fix`, `not-done`/`not done`/`nd`→`not_done`.