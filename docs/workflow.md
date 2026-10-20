# Workflow

The loop every piece of reviewable work runs. The prose that governs it is
[PROTOCOL.md](../PROTOCOL.md); these diagrams are the same rules in picture form, kept
here so the prose and the pictures cannot drift apart.

One epic holds an implement task and a review task. A review that finds problems files a
fix task as a child of the review task. The coder stops at `review`; only the reviewer
sets a fix task `done`, and only the reviewer closes the review task.

## The loop

```mermaid
flowchart TD
    subgraph MGR["Manager"]
        A["epic_add with path"]
        B["task_add implement task, epic=E"]
        C["dispatch coder"]
        D["read report: task_get or task_wait"]
        E["task_add review task, epic=E"]
        F["dispatch reviewer"]
        Z["re-run the named gates, then epic_update done"]
    end

    subgraph COD["Coder"]
        I1["task_claim, do the work, note decisions"]
        I2["task_update report, status=review"]
        FX["task_claim fix task, fix only those findings"]
        F2["task_update report per finding, status=review"]
    end

    subgraph REV["Reviewer"]
        R0["task_claim review task, read diff and report"]
        R1{"findings?"}
        RD["task_update report, status=done on the review task"]
        RF["task_add fix task, parent=review task"]
        RN["task_update status=needs_fix on the review task"]
        X1["re-check its own findings"]
        X2{"every finding closed?"}
        XF["task_update fix task done, review task done"]
        XN["note the open findings on the fix task"]
        XI["task_update status=needs_fix on the fix task"]
    end

    A --> B
    B --> C
    C --> I1
    I1 --> I2
    I2 --> D
    D --> E
    E --> F
    F --> R0
    R0 --> R1
    R1 -- none --> RD
    RD --> Z
    R1 -- yes --> RF
    RF --> RN
    RN --> FX
    FX --> F2
    F2 --> X1
    X1 --> X2
    X2 -- yes --> XF
    XF --> Z
    X2 -- no --> XN
    XN --> XI
    XI --> FX
```

## Fix-task lifecycle

```mermaid
stateDiagram-v2
    [*] --> ready : reviewer files it under the review task
    ready --> working : coder claims
    working --> review : coder reports (fixed, awaiting re-check)
    review --> needs_fix : reviewer reopens, note lists what is open
    needs_fix --> working : coder claims again
    review --> done : reviewer sets done with the evidence
    needs_fix --> done : reviewer sets done once the findings close
    working --> dropped : manager drops the row
    done --> [*]
```

## Who moves what

| Status move | Who |
|---|---|
| `ready`/`needs_fix` -> `working` -> `review` | coder |
| `review` -> `done` or `needs_fix`, on a fix task or a review task | reviewer |
| `ready` (re-arm a crashed task), `dropped`, `done` on the implement task, epic status | manager |

`review` on a fix task reads as "fixed, waiting for the reviewer". There is no `fixed`
status. A fix task reaching `done` is the reviewer's signature, and its `report` carries
the evidence.
