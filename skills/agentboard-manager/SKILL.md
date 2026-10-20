---
name: agentboard-manager
description: "Use when managing coding agents: file, dispatch, watch, gate."
version: 1.0.0
author: Hermes Agent
license: MIT
platforms: [linux, macos]
metadata:
  hermes:
    tags: [agentboard, herdr, manager, delegation, review, dispatch, wait]
    related_skills: [herdr, claude-code, opencode]
---

# Being the manager

Two ledgers, and they never swap jobs:

- **agentboard** holds the *work*: status, the prompt (the row), the report, the event log.
- **herdr** holds the *process*: tabs, panes, live agents, their sessions.

The manager owns both ledgers and writes **no code**. Not a patch, not a plugin, not
a shell-driven edit. Its hands are: board writes, dispatch, watching, reading panes,
re-running gates, git-state resets, and the conversation with the user. Every edit is
made by a worker. "Review the code" means commissioning the reviewer, not fixing it.

Repo housekeeping is code work too, including when it is one line: a `.gitignore`
entry, a manifest bump, a CI yaml, a README correction the code contradicts. The test
is not "is it trivial" but "is it the repo's" — file a row and let a worker make the
edit, so every write in the tree has an author on the board. A file the coder is
already in the middle of is the one exception worth not touching at all.

This skill is the operating loop. Pane mechanics: the `herdr` skill (and `herdr
--skill`, the authority shipped with the binary). Machine setup, MCP configs and
wrapper bins: `references/machine-provisioning.md`. Costly mistakes:
`references/pitfalls.md`.

## When to Use

- You are the manager of a coder + reviewer team and the work lives on the board.
- A piece of work needs filing, dispatching, watching, or its report needs verifying.
- A review round has to be commissioned, or findings turned into a fix task.
- A worker is blocked, silent, or its task sits behind a live lease.
- You are about to poll the board by hand — stop and use `wait`.
- A NEW machine must run the team (claude/agentboard/MCP configs, wrapper bins,
  verification) — `references/machine-provisioning.md`.

## The one rule that shapes everything

**The task row is the prompt.** `description` is the context (repo, files, the
situation, what to build); `goal` is how to do it plus how it is judged finished.
`claim` returns both **verbatim and uncapped**, so the worker receives its prompt in
one call. A scratch `prompt.txt` beside the task is a second copy that drifts — there
is no reason to write one.

Dispatch is therefore one line, and the board owns the text:

```bash
agentboard dispatch T15 --to coder2
# dispatched to coder2 via herdr: Claim T15, then task_get id=15 full=true, and execute it.
```

## The loop

0. **Orient before acting.** `agentboard brief`, `agentboard ls`,
   `agentboard sessions`, `herdr agent list`. Never dispatch into a board or a pane
   you have not looked at; a stale `working` or a pane rooted in the wrong repo is
   otherwise invisible until the work comes back wrong.
1. **Epic first, then the work under it.** `agentboard epic add "<outcome>" [--path <repo>]
   [--description D] [--goal G]` creates the container, then tasks go in with
   `--epic E<n>`. Epics and tasks are two tables with two id spaces: epics print `E1`,
   tasks `T1`, and the prefix decides which table a verb is talking to, so `--epic E3`
   and `--parent T7` never mean the same thing. A task may sit under an epic **and** be
   a subtask of another task (the fix task under the review task that found it); a
   subtask inherits its parent's epic, and a disagreement, a cycle, a self-parent or a
   cross-epic parent is refused. Re-pointing a task is `agentboard parent T<n> <T<m>|none`.
   An epic is never claimable, leaseable or dispatchable — every task verb refuses an E
   id and names the epic verb you wanted.

   Give the epic its **project path** (`--path`, `~` and relative paths resolved, existence
   not required). It is the one property no task has, and it is what lets dispatch answer
   "is this pane even in the right repo": when the live target's cwd falls outside the
   epic's path, `dispatch` prints a warning naming both and proceeds. Warn, not refuse —
   the manager decides, but only if it was told.
2. **Write the row so the worker can self-verify.** A goal must be executable:
   "`cargo test` prints 4 passed", "`rg 'unwrap()' src/` returns nothing", "these
   three commands and their output pasted into the report". "Implement X" is not a
   goal. File long or backtick-heavy text over MCP (`task_add` with a JSON string) so
   shell quoting never eats a span.

   For a change that touches every call site — an argv rewrite, a rename, a signature
   change — the row also carries a **parity contract**: which behaviours must not
   move, the traps that break them, and the test counts before and after. "Refactor X
   to Y" invites a silent behaviour change; an enumerated contract does not. CLI
   rewrites specifically: `references/cli-rewrite-parity.md`.

   Counts in a goal (`cli.sh 39/39`, `202 passed`) go stale the moment anyone adds a
   test, and a worker then either "fails" a target that was already exceeded or trims
   tests to hit a number. Quote the command and "current totals", or run the suite at
   filing time — never a count you remember.

   A spec is a **row you can amend**: `agentboard edit T<n> [--title T] [--description|--spec D]
   [--goal G] [--priority N] [--assignee A]` (and the same for `E<n>`, with `--path`), one
   `edit` event per field, `unchanged` and no write when the text is the same. Fix the row
   instead of refiling it — refiling costs the id and the history. The guard is the lease:
   a row held by a working agent refuses the edit (exit 1, the same
   `lease Nm left — manager: agentboard status T<n> ready` remedy as `claim`), because the
   worker is already reading that text. So: amend freely while `ready`; while `working`,
   either wait for the delivery (the worker can `task_get full=true` again) or reset with
   `status T<n> ready` and re-dispatch. Short fields record old→new values in the event,
   description and goal record lengths only — a big spec edit is not restorable, so still
   prefer refiling when the change is structural rather than a correction.
3. **Dispatch with the verb**, not a hand-typed sentence:
   `agentboard dispatch T<n> --to <session>`. It refuses a task that is not `ready`
   (exit 1, no event, no write), `--force` resets through the normal status->ready
   path first, `--print` mutates nothing, delivery is `herdr agent prompt` under a
   deadline with a tmux fallback, and the `dispatch` event is written only after a
   successful send. The deadline matters more than it looks: an undeadlined
   `Command::output()` against a wedged pane hangs the manager, not the worker.   Read the
   event body afterwards — it names the path that delivered and the reason for any
   fallback (`w1 via tmux (herdr: timed out after 200ms)`), which "exit 0" does not.

   **One row per pane at a time.** `herdr agent prompt` submits into the live agent's
   conversation, so a dispatch to a pane that is still `working` becomes an extra user
   turn in the middle of the task it is doing: it either drops the second instruction or
   answers it with the first task's context. Queue the next row as `ready` and dispatch it
   when the pane reports — parallel work is a second pane, never a second dispatch.

   If dispatch fails naming an agent you did not pass (`agent_not_found: reviewer` while
   you said `--to reviewer2`), the task's **assignee** had a registration row that
   shadows your target. Resolution now prefers a *live* target — assignee-if-live, then
   `--to`-if-live — and otherwise refuses with one error naming every candidate and
   saying the assignee's row is stale. `agentboard sessions` marks such rows `gone`.
   Clean them up with the verbs instead of touching the DB: `agentboard unregister <name>`
   for one, `agentboard sessions prune [--dry-run]` for the rest. Prune refuses when
   neither tmux nor herdr can answer — a box that cannot see sessions must not delete the
   registry — so run it where herdr is up. Only if a verb is genuinely missing do you
   edit `board.db`, and then with python's stdlib (`sqlite3`, no CLI binary on this box)
   and a line in the reply saying you did; a manager that reaches for the DB routinely has
   stopped respecting the board.
4. **Arm the watcher before ending the turn.** This is the step that gets skipped and
   it is not optional: no polling loops in python, no reading `board.db` by hand.

   ```bash
   agentboard wait T<n> --timeout 1800 --status review,blocking,needs_fix
   # exit 0 = matched (prints `matched:` + the task's status + the new event lines)
   # exit 3 = timeout (prints `timeout:` + one status line per watched task + a tail)
   ```

   A **review** task is the exception: the reviewer marks its own row `review` when it
   finishes, exactly as a coder would, so watch `done,review,needs_fix,blocking` for it —
   otherwise the wait runs the full timeout while the verdict sits on the board (a 40-minute
   timeout spent on a review that landed 10 minutes in).

   Run it as a background job with notify. `--open --kind ask` is the other useful
   shape: it wakes when *anyone* on the board needs a human. A blocked delegate shows
   up as `ask`, which the CLI accepts as an alias of the real `block` event.

   A dispatch is not finished until its watcher is armed, and any question about a
   delegate's progress — from the user, or from your own doubt — is answered by reading
   the board, never by asking the pane or guessing from elapsed time. Ending a turn on
   "it's probably still working" is how a finished task gets reported as running.

   Facts that matter when reading its output:

   - `--open` re-queries each poll, so a task created mid-wait is picked up (verified:
     matched 204 ms after the row appeared). Explicit ids and `--epic` stay frozen —
     they name work, not a predicate.
   - A `--status`-only match on a task already in that status returns at once, and
     prints its most recent event prefixed `last:` — that event is context, not the
     cause of the match.
   - On timeout it hands back every watched task's status plus a clipped tail of the
     freshest one, so a silent timeout is still evidence.
   - `--status`/`--kind` names are validated: a typo exits 1 instead of quietly
     matching nothing. `reclaim` is not a kind (it is logged as `claim`).
   - For a `--status`-only match, `--after` is a no-op by definition: the match is
     current state, not an event.
5. **On wake, read the row, then re-run the gates yourself.** Completion is the board
   `report` **plus your own** runs, with the real command output in your reply to the
   user. Never the pane's word "done", never the report's claims. `agent_status: idle`
   means "ready for input", not "finished".

   **Green once is not green.** A suite that asserts anything time-shaped — a relative
   age in a rendered row (`created: 0s`, `lease: 59m left`), a boundary between two
   calls, a TTL countdown — passes or fails by where the second hand was. Run the suite
   three to five times before believing it, and when a reviewer reports a suite that is
   "N/N but flaky", treat the flake as a defect of the same severity as a failing test:
   the next session inherits a gate that cannot be trusted.
6. **One fresh review task per review round**, in a pane separate from the coder
   (standing choice: the `claude-reviewer` wrapper, actor `reviewer`), dispatched the
   same way. The verdict goes on the board
   as its report. A `needs_fix` verdict becomes the next coder task — and that fix task
   is a **subtask of the review that produced it**: file it with
   `--parent T<review>` (the epic is inherited from the parent), so `children T<review>`
   answers "what did this review cost" instead of leaving the link only in the prose.
   Same for anything else a task spawns: the child records its cause, not just its epic.
   Filed it flat by mistake? `agentboard parent T<n> T<m>` repairs the link afterwards —
   it is accepted on any status, `done` included, logs `edit: parent -> T<m>`, and
   `children` then answers from both ends. A wrong link never needs a refile.

   Ask for a **revert proof** on behaviour fixes: the author reverts their own fix,
   shows the named test fails, then restores it. A test that passes on the broken code
   is decoration, and the review round is the only place this gets caught.

   Revert-proof is necessary but not sufficient — the harness can be the thing that is
   broken in two ways that both look green:

   - **The assertion accepts success.** A helper written as `try: return "ACCEPTED: " +
     call(...) except: return str(e)` then handed to a substring check passes whether the
     call succeeded or was refused. Assert the *shape* (an error was raised, exit code is
     non-zero), never a string that both paths can produce.
   - **The assertion measures the wrong thing.** A hang test that only times the caller
     still passes when the kill is deleted — the stub survives, the caller returns fast.
     Assert the observable: the pid is gone, the event count, the exit code, the row state.

   So demand two mutations per guard: revert the fix (does it fail?) and force the
   failure path to succeed (does it still fail?). Anything that survives either one is
   untested, not passing.

   A verdict belongs to the reviewer that produced it. If the review role changes hands
   (a different vendor or model takes the seat), the accumulated commits get one fresh
   pass from the reviewer of record before anything is called merge-ready — the earlier
   approve is another reviewer's opinion, not a gate you can cite. Say on the board which
   reviewer ran, so a later session can tell the two apart.

   A **required** fix from the reviewer of record jumps the queue. The user's next feature
   does not outrank a merge gate: land the fix, re-verify, commit, then resume the queue —
   everything built after it would sit on an unapproved base, and the finding is cheapest
   to fix while the reviewer's evidence is fresh. Optional findings queue like any other work.
7. **Freeze the tree while a reviewer reads it.** Do not dispatch the next coder round
   until the verdict lands: a diff that moves under the reviewer produces findings
   against code that no longer exists, and the whole verdict is thrown away. A
   behaviour change after a review makes the old verdict stale evidence — commission
   a second round rather than citing it. Two rows that would edit the same file (a docs
   task and a feature task both touching `README.md`) are serialized for the same reason:
   say in the spec which row owns the shared file, and dispatch the other one after.
8. **Commit only on an explicit user ok**, locally, never pushed. The manager commits;
   workers are told not to. `git add` explicit paths — never `-A`, which sweeps the
   workers' scratch files into the commit.

   **Re-read git state at report time.** The user has a shell in the same repo and can
   push or commit from it while you are working, so a `git status` from earlier in the
   session is not evidence about now: before telling them "N commits ahead, unpushed" or
   "HEAD is X", run `git status -sb` and `git ls-remote origin master` and compare — or
   say nothing about push state. A stale ahead-count is a false claim about the tree.

## Board surface the manager uses

| Command | What it is for |
|---|---|
| `brief` | counts, open tasks by urgency, recent completions — the standup; groups open epics with their path and `[N blocking, N needs_fix]` |
| `ls [--status S] [--assignee A]` | compact open list |
| `epic add "<title>" [--path P] [--description D] [--goal G]` | the container: its own table and id space, never leaseable |
| `epic ls [--status open\|done\|dropped]` / `epic get E<n> [--full]` / `epic status E<n> <s> [note]` | read and close epics |
| `children E<n>` / `children T<n>` | the epic's tasks, or one task's subtasks — two different questions |
| `parent T<n> <T<m>\|none` | re-point a subtask's parent (epic inheritance is re-checked) |
| `add "<title>" --description D --goal G [--assignee A] [--epic E] [--parent T] [--priority N]` | file a row; `--spec` is an alias of `--description` |
| `get T<n> [--full]` | the row; `--full` for verbatim description/goal/report/note/events. Plain mode folds to one line and clips **with an explicit marker** |
| `status T<n> <s> [note]` | manager moves status without claiming. `ready` clears the lease and re-arms a crashed task |
| `note` / `report` | progress / outcome, without moving status |
| `ask` / `answer` | block on a human, resume with a fresh lease |
| `dispatch T<n> [--to S] [--force] [--print]` | hand the row to a pane (see step 3) |
| `wait [T…] [--epic E<n>] [--open] [--status csv] [--kind csv] [--after SEQ] [--timeout SEC]` | block until the board moves (see step 4); `--epic` takes an E id and selects every task under it, subtasks included |
| `sessions` / `register <agent-name> --agent claude\|opencode [--task T]` | map task -> pane; **register the herdr agent name, never the pane id** |
| `sessions prune [--dry-run]` / `unregister <name>` | delete rows no live pane answers to; rows never expire on their own |

Statuses: `ready working blocking needs_fix review done dropped`; epic statuses:
`open done dropped`.

Two refusals worth recognising on sight:

```text
T7 working by claude (lease 24m left, until 13:23Z) — manager: agentboard status T7 ready
T9 is review, not ready (assignee opencode, lease none); --force resets it to ready first
```

The first is `claim` telling you a lease is live; the remedy in the text works
(it clears the lease), so a crashed holder costs one command, not a 30-minute TTL.

Both are **errors, not replies**: an explicit-id `claim` refusal and a `dispatch`
refusal exit 1, and over MCP the same text arrives as a tool error. So an error from
`claim` means *someone holds this row* — read the text and apply the remedy — not
*the board is broken*; do not retry it blind and do not report the tool as down. The
id-less probe is the opposite case and stays a success: `claim` with no id on an empty
board exits 0 and prints `no ready tasks`, which is an answer, not a failure.

**The manager does not `claim`.** A manager claim takes the lease and the coder's
claim is then refused. Use `status T<n> working "<note>"` to show intent and let the
worker claim.

## MCP equivalents

When driving the board over MCP instead of a shell: `board_brief`, `task_list`,
`task_add`, `task_get` (`full: true`), `task_claim`, `task_update` (status / report /
note / commit), `task_edit`, `task_ask` + `task_wait`, `task_children`, `epic_add`,
`epic_list`, `epic_get`, `epic_update`, `epic_edit`, `session_list`.
`task_wait` carries the same selectors as the CLI `wait` and, having no exit codes,
puts `matched:` or `timeout:` on the first line — branch on that. There is no MCP
`parent` verb: re-pointing a subtask over MCP is a CLI call.

Workers must have `task_claim` and `task_get` in their allowed toolset; without them
they work and report success on a task they never claimed.

## herdr: the minimum a manager needs

Full pane mechanics and start flags: the `herdr` skill. What the loop needs:

```bash
herdr tab list | pane list | agent list          # orientation, JSON envelopes
herdr tab create --cwd <repo> --label <name>     # -> root_pane.pane_id
herdr pane split --pane <id> --direction right --cwd <repo> --no-focus
herdr agent start coder2 --kind claude --pane <id> --timeout 120000 -- \
  --strict-mcp-config --mcp-config=$HOME/.claude/coding-mcp.json \
  --append-system-prompt-file=$HOME/.claude/caveman-full.md --model sonnet
herdr agent start reviewer2 --kind claude --pane <id> --timeout 120000 -- \
  --strict-mcp-config --mcp-config=$HOME/.claude/reviewer-mcp.json \
  --append-system-prompt-file=$HOME/.claude/caveman-full.md --model sonnet
herdr agent prompt <name> "<one line>"           # atomic submit, one turn
herdr agent read <name> --lines 30               # what is it actually doing
herdr pane close <id> ; herdr tab close <id>     # teardown, on an explicit stop only
```

- **Names are the address.** Distinct names let two teams run at once (`coder2`,
  `reviewer2`); `agent_name_taken` cannot be cleared without closing that pane.
- **A pane is never closed to free a name or as cleanup.** Not while `working` or
  `blocked`, and only when the user asks.
- **`blocked` is usually a dialog or a TUI at its prompt**, not a hung agent: read the
  pane before touching it. An opencode reviewer may sit on "allow web search?" — a
  local diff review does not need it.
- **Do not run `herdr-agent-open` from the manager shell.** It splits the *current*
  pane, which needs `HERDR_PANE_ID`; it kills the old agent first and then dies at the
  split, leaving a project with no agent. Create the tab and panes explicitly by id.
- Verify `cwd` in `agent list` before dispatching. A pane in the wrong directory is
  the one failure that silently produces work in the wrong repo.

## Keeping the tool current

- `agentboard` runs from two binaries: `~/.cargo/bin/agentboard` (installed — what your shell and the
  agent MCP configs execute) and `./target/release/agentboard` (the checkout). After changing the
  tool, `cargo install --path . --locked`, or the verbs you just built are missing from the binary
  you are dispatching with.
- `tests/mcp_client.py` drives `target/release/agentboard`. Rebuild before running it, or a green
  suite is testing week-old code.
- **A schema change needs a switch, not a migration.** The board has no migrate path: an
  old file is refused before any DDL with one line naming `mv <db> <db>.old`, and the file
  is left byte-identical. History is expendable there, so the switch is a wipe — but the
  order matters, because the agents' MCP servers are processes of the *old* binary:

  1. stop every agent and close the panes (nothing may hold the board) — including your own
  MCP board server, which is equally a process of the old binary: after the switch, drive the
  board by CLI until this session restarts, or every MCP board call fails against a schema it
  does not know; 2. `cargo install`
  the new binary, 3. move `board.db` (plus `-wal`/`-shm`) into an archive directory, 4. run
  one read (`agentboard brief`) so the *new* binary creates the schema, 5. re-register the
  teams by name, 6. refile the rows still open — every id died in the wipe, so the queue is
  rebuilt from the specs you still hold.

  Skipping step 1 is the failure: an old-binary server answering a request recreates the
  old schema in the new file, and the new binary then refuses its own board.
- Agent panes spawn their MCP server from the PATH binary at session start and keep that build until
  the pane restarts. A schema change is the dangerous case; a new flag is merely invisible.
- Parsing has its own suite, `tests/cli.sh` (exit codes, `--flag=value`, unknown-flag errors, hyphen
  values), and delivery has `tests/dispatch.sh` (refusal, `--print`, `--force`, assigned-vs-`--to`,
  fallback reason, deadline kill). Run `mcp_client.py`, `cli.sh` and `dispatch.sh` before believing a
  CLI or dispatch change.
- **Test delivery with stub binaries on PATH, never against a live pane.** A two-line `herdr`/`tmux`
  script that exits, fails, or sleeps is how you prove the fallback and the deadline in milliseconds;
  a real dispatch types into someone's working agent and cannot be replayed.
- **"Every call now goes through X" is a grep you owe.** Unifying a mechanism does not
  retire the hand-rolled copies of it: after the shared runner landed, `sessions` still
  had its own spawn-and-join with a fixed deadline, so the README
  line "the env var bounds every bridge call" and the commit message were both false —
  the path took 8 s under a stub that should have been killed at 300 ms. Before claiming
  coverage, search for the old pattern (`search_files` / `rg`: `Command::output`, the old
  constant, a second `std::process::Command` in the same module) and make the claim the
  acceptance criterion of a test, not a sentence in a commit.
- **A count written in prose is a claim to re-derive, not to copy.** A header comment
  saying "18 tools", a README's test total, a worker's report line `cli.sh 102/102` — all
  drift within one round, and quoting one back to the user is a wrong statement of fact.
  Derive it from the source at verification time (`rg -c '#\[tool\(' src/server.rs`,
  `cargo test 2>&1 | rg '^test result'`) and fix the prose where it disagrees with what
  the command prints.
- **A verb that lands retires the skill that says it is missing.** Shipping a feature is
  not finished while a playbook still teaches the workaround: after the round, grep the
  skill library for the old claim (`rg -n 'no edit verb|there is no|not yet|workaround'`
  in the skills dir), patch the sentence in place, and check the reference files too — a
  stale `pitfalls.md` bullet sends the next session down a path the tool closed.
- **When the playbook is also shipped inside the repo, edit the repo copy and install from
  it.** The repo copy is canonical and the installed one is a snapshot, so editing the
  installed copy creates drift that the next `install.sh` silently reverts: edit in the
  tree, `skills/install.sh --force`, then `diff -r` both ways. That comparison is against
  something *outside* the tree, so its result is time-shaped — a worker reporting `diff -r`
  empty means empty at the moment it looked, not at commit time. Re-run the diff yourself
  at verification and hold the commit until it matches; a copy task whose source keeps
  moving lands stale, and the stale half is the half a future session reads.

## What "done" means

1. The task is `done` on the board, with a report that names files and pastes real output.
2. The manager re-ran the named gates in that tree and they passed — the output is in
   the reply to the user.
3. A review task agrees, or its required fixes landed and a second round says so.
4. Nothing is committed until the user says commit, and nothing is pushed ever.

## Close the round: hygiene tasks

A feature round is not finished when the review says approve. File the hygiene work
before you call it done, or it accumulates until the noise hides a real warning:

- **clippy**: `cargo clean -p <crate>` then `cargo clippy --all-targets` — cached runs
  replay warnings from earlier builds and will lie to you about the count (8, 6 and 2
  were all "the same" tree at different cache states).
- **fmt**: `cargo fmt --check`; a never-formatted repo reports thousands of diff lines.
- Two separate tasks, clippy first, fmt second, and **fmt never runs while a reviewer
  is reading the tree** — a mechanical 1000-line diff buries the change under review.
  Commit them separately for the same reason: a clippy fix is reviewable, a formatting
  sweep is not.
- Add the gate to whatever the repo calls its checks (`README`, `justfile`, CI) so the
  next round inherits it instead of repeating this task.
