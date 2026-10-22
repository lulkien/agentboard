# Workflow

The loop every piece of reviewable work runs. The prose that governs it is
[PROTOCOL.md](../PROTOCOL.md); these diagrams are the same rules in picture form, kept
here so the prose and the pictures cannot drift apart. This page owns every diagram:
other pages link here and carry none.

One epic holds a chain of exactly three levels: an implement row, its review row (a child
of the implement row) and a fix row (a child of the review row). There is no fourth level.
The board files the review row itself when the implement row moves to `review`, and it
refuses that move unless a submission was recorded since the last claim: a diff, or an evidence-only row for a re-run round whose tree did not change. A review that finds
problems files one fix row, reused across rounds. The coder stops at `review`; only the
reviewer sets a fix row `done` and closes the review row; only the manager closes the
implement row.

## The loop

```mermaid
flowchart TD
    subgraph MGR["Manager"]
        A["epic_add with path"]
        B["task_add implement row, epic=E"]
        C["dispatch coder"]
        D["read report: task_get or task_wait"]
        F["dispatch reviewer to the review row"]
        Z["re-run the named gates, task_update done on the implement row, then epic_update done"]
    end

    subgraph COD["Coder"]
        I1["task_claim, do the work, note decisions"]
        I2["diff_add the change, or evidence-only for a re-run"]
        I3["task_update report, status=review"]
        FX["task_claim fix row, fix only those findings"]
        F1["diff_add the fix"]
        F2["task_update report per finding, status=review"]
    end

    subgraph BRD["Board"]
        G{"diff or evidence-only recorded since the last claim?"}
        GR["refused, exit 1, names both forms"]
        AF["files the review row, or notes the new submission; the brief names the diff to read"]
    end

    subgraph REV["Reviewer"]
        R0["task_claim review row, read only the diff the brief names"]
        R1{"findings?"}
        RD["task_update report, status=done on the review row"]
        RF["task_add fix row, parent=review row"]
        RN["task_update status=needs_fix on the review row"]
        X1["re-check its own findings against the new diff"]
        X2{"every finding closed?"}
        XF["task_update fix row done, review row done"]
        XN["note the open findings on the fix row"]
        XI["task_update status=needs_fix on the fix row"]
    end

    A --> B
    B --> C
    C --> I1
    I1 --> I2
    I2 --> I3
    I3 --> G
    G -- no --> GR
    GR --> I2
    G -- yes --> AF
    AF --> D
    D --> F
    F --> R0
    R0 --> R1
    R1 -- none --> RD
    RD --> Z
    R1 -- yes --> RF
    RF --> RN
    RN --> FX
    FX --> F1
    F1 --> F2
    F2 --> X1
    X1 --> X2
    X2 -- yes --> XF
    XF --> Z
    X2 -- no --> XN
    XN --> XI
    XI --> FX
```

## Fix-row lifecycle

```mermaid
stateDiagram-v2
    [*] --> ready : reviewer files it under the review row
    ready --> working : coder claims
    working --> review : coder records a diff, then reports
    review --> needs_fix : reviewer reopens, note lists what is open
    needs_fix --> working : coder claims again
    review --> done : reviewer sets done with the evidence
    review --> not_done : the reviewer's fourth move into needs_fix is refused
    working --> dropped : manager drops the row
    not_done --> dropped : manager drops it
    done --> [*]
    dropped --> [*]
```

## The reopen cap

```mermaid
flowchart TD
    M["a row moves into needs_fix"] --> N{"reopen count is already 3?"}
    N -- no --> OK["count goes up by one, the row lands needs_fix"]
    N -- yes --> CAP["refused: CLI exit 1, MCP tool error, one line naming the cap and not_done"]
    CAP --> L["the row lands not_done, the caller body is kept in an event, reopens: 4/3 cap hit"]
    L --> P{"does the row have a parent?"}
    P -- no --> E["nothing else moves"]
    P -- yes --> Q{"is the parent open: ready, working, blocking, needs_fix or review?"}
    Q -- yes --> PN["the parent lands not_done in the same transaction, with its own event naming the child"]
    Q -- no --> PS["the parent is left as it is, the capped row gets a note naming the parent and its status"]
    PN --> H["the grandparent is not touched; a capped fix row leaves the implement row at review"]
    PS --> H
    E --> U["the manager drops the rows or files a replacement implement row"]
    H --> U
```

## Who moves what

| Status move | Who |
|---|---|
| `ready`/`needs_fix` -> `working` -> `review` (a move to `review` needs a diff or evidence-only submission since the last claim) | coder |
| `review` -> `done` or `needs_fix`, on a fix row or a review row | reviewer |
| `ready` (re-arm a crashed row), `dropped`, `done` on the implement row, epic status, clearing a `not_done` row | manager |
| `not_done`, and filing the review row | the board |

`review` on a fix row reads as "fixed, waiting for the reviewer". There is no `fixed`
status. A fix row reaching `done` is the reviewer's signature, and its `report` carries
the evidence.

Closing an epic once every row is `done` or `dropped` is the manager's policy, not a
guard: `epic_update` does not check the rows, so a `not_done` row blocks closure only
because the manager does not close over it.
