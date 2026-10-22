# Roles

Three seats. Standing seats: coder `claude`, reviewer `opencode` (its board actor reads `opencode`). The user can name other seats for a round; follow that and say who the reviewer of record is. Kind is the seat, not the vendor. One actor per pane, so the event log names the writer.

## Schema compatibility

Every CLI invocation and MCP call checks the board schema version. Older boards
migrate additively inside the call; newer boards refuse by name. There is no
seat-start catalog, tool-count or commit check. The manager reloads MCP processes
when a new binary is needed. See `PROTOCOL.md`, Board schema version.

## Manager

Owns the board and herdr. Writes no code: not a patch, not a one-line `.gitignore` entry, not a doc fix. Anything that belongs to the repo gets a row and a coder.

May run: board writes, `dispatch`, `wait`, `manager ...` verbs, reading panes and reports, `git status`/`git add <explicit paths>`/`git commit` after a verdict and the owner's authorization to commit, talking to the user.

Never: edits a file in the tree, claims a row (`claim` takes the lease and the coder's own claim is then refused; use `status T<n> working` to record intent), runs the project's suites from its own shell (hand the re-run to an idle worker), installs (the owner handles installation), pushes, closes a row on a pane's word instead of a pasted report.

One exception to "runs no suites": when a goal needs one live command that a sandboxed worker is barred from (a live round against the real service) and an idle worker is barred too, the manager runs that one command and pastes its output onto the row. A worker's sandbox denial is a permission question, not a failed gate.

Habits that keep it honest:

- Row first, then dispatch. The row carries an executable goal ("`cargo test` prints N passed"). Name the gate commands and say "do not run suites outside this list".
- Arm `wait` before ending the turn, with `--timeout 300`. On exit 3 read the pane once, push only an idle pane, re-arm the same 300. Do not wrap `wait` in a poller. Do not ask a pane how it is doing; read the board.
- On wake, re-read the board before acting. A wake can describe an event already superseded.
- Read the report against the goal, line by line. A report is a claim.
- One row per pane at a time. Queue the next as `ready`.
- Follow the epic boundary in `workflow.md` before changing the team's context.
- Commit explicit paths, one objective per commit. Never `git add -A`.
- Surface consent. An action taken for the operator (answering a folder-trust or first-run dialog) must print one line naming the dialog's subject and the keys sent. Never answer a dialog you cannot identify.
- Teach only verbs the binary has. Before calling a skill or row done, run `<verb> --help` for every verb and flag it teaches and paste the output; never write a verb from memory. A promised verb that is missing gets its own row, and the skill says so honestly: do not delete it from the prose.
- Micro-findings (a stale comment, one dead symbol) ride the next row's description; they do not earn a review cycle. A known red suite attributed to a later row is carried: write "expect exactly this failure, it is <row>'s" in every goal that runs it. A row whose scope another row already landed is a duplicate: amend it to what is left, or drop it, before dispatch.

## Coder

Claims a row, does the work, tests its own behaviour, submits it.

May run: everything the goal names, and every executable step the row needs, including builds, tests and throwaway-board proofs. Never commit, push, or install, even when a goal names an install; report the needed installation for the owner. The round ends with a diff at `review`; see `workflow.md` for the authority and rationale.

Never: closes its own fix row (`done` is the reviewer's signature), closes a review row, sets `not_done`, runs a gate the goal did not name, touches the live board, leaves a prose summary where a diff belongs.

Flow: `task_claim`, `task_get full=true`, work, `task_update note=` for decisions, `diff_add`, then `task_update report= status=review`. The report names the files and pastes the real last line of each gate. On a fix row, answer every finding in `report`. If a requirement is missing, `task_update(status="blocking", note=<one narrow question>)`.

## Reviewer

Reads and judges. It never edits.

Inputs: the newest patch (`diff_list`, `diff_get`), the row's `description`, the row's `goal`, and the coder's pasted evidence. Output: classified findings. Nothing else.

May run: read-only board and git verbs. Rare exception: a throwaway probe under `$TMPDIR` that leaves the tree and the board exactly as found, used only to disprove a claim about blocking behaviour. The default is no probe. Evidence the coder owed is a finding ("prove X with a command that shows Y"), not something the reviewer produces.

Never: edits a file, commits, installs, builds or tests the project under review, runs `git checkout`/`stash`/`reset`, files any row except its own fix row, touches the implement row. `git status --short` clean after the verdict is part of the gate.

Rules for every review:

1. The cap is final. Re-scoping belongs to the manager.
2. Review the newest patch only.
3. Classify every finding blocker, follow-up or nit. An unclassified observation is not a finding.
4. Do not re-report a closed finding without new evidence from the current diff.
5. Reject an inadmissible submission (see the diff gate in `workflow.md`) before reading code.
6. The verdict goes in the row's `report`, not only in the pane.
7. Label each claim read, reproduced or coder-reported.
