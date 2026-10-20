---
name: agentboard-manager
description: "Use when managing coding agents: file, dispatch, watch, gate."
version: 1.1.0
author: Hermes Agent
license: MIT
platforms: [linux, macos]
metadata:
  hermes:
    tags: [agentboard, herdr, manager, delegation, review, dispatch, wait]
    related_skills: [team-manager, claude-code, opencode]
---
# Being the manager

Two ledgers, never swap jobs:

- **agentboard** holds *work*: status, prompt (the row), report, event log.
- **herdr** holds *process*: tabs, panes, live agents, sessions.

Manager owns both ledgers, writes **no code**. No patch, no plugin, no shell-driven edit. Hands: board writes, dispatch, watching, reading panes, re-running gates, git-state resets, conversation with user. Every edit made by worker. "Review the code" = commission reviewer, not fix it.

One override: user says do independent row yourself, now — and it touches files no worker/reviewer holds — do it. Pick docs-only row, files disjoint from in-flight task, never interrupt working pane for it, record exception in row: `status T<n> working "<manager> writing it directly"`, then `report` with evidence, then `status T<n> done "committed as <sha>"`. Put "no reviewer round ran" in report so ledger not imply gate that never happened. Row editing files coder is inside = not independent — queue it.

Only time to interrupt working pane: **doc or design row whose spec you rewrite**: work so far is prose headed wrong way, no code/reviewer holds it, finishing costs second full round. `herdr agent send-keys <name> esc`, then `agentboard status T<n> ready "<why>"` to drop lease, land both fields, dispatch again. Never on code row mid-edit: tree keeps whatever half it wrote.

Repo housekeeping = code work too, even one line: `.gitignore` entry, manifest bump, CI yaml, README correction code contradicts. Test not "is it trivial" but "is it the repo's" — file row, worker edits, so every write in tree has author on board. File coder is mid-edit in = one exception worth not touching at all.

This skill = operating loop. Pane mechanics + step-by-step procedure: `team-manager` skill, whose `references/herdr-mechanics.md` carries pane depth (`herdr --skill` stays authority shipped with binary). Machine setup, MCP configs, wrapper bins: `references/machine-provisioning.md`. Costly mistakes: `references/pitfalls.md`. Reproducing worker's "the engine cannot do this" claim: `references/worker-claim-probes.md`.

Same job as short operating procedure — topology, epic/task recipes, dispatch, review loop, renew cycle, recovery table — is `team-manager` skill. Use it for commands in order; this file = reasoning + traps.

## When to Use

- You manage coder + reviewer team, work lives on board.
- Work needs filing, dispatching, watching, or report verifying.
- Review round to commission, or findings to turn into fix task.
- Worker blocked, silent, or task behind live lease.
- About to poll board by hand — stop, use `wait`.
- NEW machine must run team (claude/agentboard/MCP configs, wrapper bins, verification) — `references/machine-provisioning.md`.
- Board must start fresh — plain reset or schema switch ("Keeping the tool current").

## The one rule that shapes everything

**Task row is the prompt.** `description` = context (repo, files, situation, what to build); `goal` = how to do it + how judged finished. `claim` returns both **verbatim and uncapped**, so worker gets prompt in one call. Never stage prompt text in file — no `prompt.txt`, no `desc.txt`, no `"$(cat
scratch.md)"` handed to `add`. Row is only copy; user treats second copy as defect even if deleted after. Long, backticked, multi-paragraph text → MCP `task_add` / `task_edit` with JSON argument, sidesteps shell quoting; CLI form for short one-liners.

Name work, not agent in seat. Workflow, doc, row agent-agnostic (`docs/review-loop.md`, "Review: review-loop.md"), since coder/reviewer pair changes when quota runs out; name current agent only inside one clearly labelled setup example in body.

Dispatch therefore one line, board owns text:

```bash
agentboard dispatch T15 --to coder2
# dispatched to coder2 via herdr: Use the agentboard MCP tools: claim task T15 with the
# claim tool, read it with the get tool (full=true), then execute it. Do not shell out
# to the agentboard binary for board writes.
```

Line names MCP tools on purpose: CLI-shaped line (`Claim T8, then task_get
id=8`) taught workers to run binary in bash step instead of calling tool. If pane still gets old wording, installed binary predates change — check `strings ~/.cargo/bin/agentboard | rg 'claim tool'` before blaming row.

## The loop

0. **Orient before acting.** `agentboard brief`, `agentboard ls`,
   `agentboard sessions`, `herdr agent list`. Never dispatch into board or pane not looked at; stale `working` or pane rooted in wrong repo otherwise invisible until work comes back wrong.
1. **Epic first, then work under it.** `agentboard epic add "<outcome>" [--path <repo>]
   [--description D] [--goal G]` creates container, then tasks go in with
   `--epic E<n>`. Epics and tasks: two tables, two id spaces: epics print `E1`, tasks `T1`, prefix decides which table verb talks to, so `--epic E3` and `--parent T7` never mean same thing. Task may sit under epic **and** be subtask of another task (fix task under review task that found it); subtask inherits parent's epic; disagreement, cycle, self-parent, cross-epic parent refused. Re-point: `agentboard parent T<n> <T<m>|none`.
   Epic never claimable, leaseable, dispatchable — every task verb refuses E id and names epic verb you wanted.

   Give epic its **project path** (`--path`, `~` and relative paths resolved, existence not required). Only property no task has; lets dispatch answer "is pane in right repo": live target's cwd outside epic's path → `dispatch` prints warning naming both, proceeds. Warn, not refuse — manager decides, but only if told.
2. **Write row so worker can self-verify.** Goal must be executable: "`cargo test` prints 4 passed", "`rg 'unwrap()' src/` returns nothing", "these three commands and their output pasted into the report". "Implement X" not a goal. File long or backtick-heavy text over MCP (`task_add` with JSON string) so shell quoting never eats span. Spec long enough for five numbered findings + checks can break single call (tool answers `calls[0] requires a
   'name'`, nothing written): file row first with one-line description, then land body with `task_edit` on `description` and again on `goal`. Two edits cost two `edit` events; rejected call costs whole spec.

   **Resolve stated constraint's physical shape before filing.** User line like "still use the db file" = two possible specs: path contract (`AGENTBOARD_DB`, `--db`, `path`) or one physical file on disk. Embedded store may only offer directory, so encode reading you can implement, say in row which one, put impossible reading in reply as question — never file spec asking for shape engine cannot produce. Same for tech: user names crate or database → scope row to that one; no bake-off of alternatives; cheaper option idea goes to user, not worker's row.

   **Two documents disagreeing = doc bug in authority, not a section.** Playbook contradicts wire document agents read first (PROTOCOL.md) → fix wire document, delete "divergence" note from playbook; page documenting split keeps split alive, every worker picks whichever half it read. Ask for one concrete numbered workflow, one step per action, plus who-moves-what status ownership, and doc owning diagrams pointed at from both sides.

   Diagrams = section of markdown file: `mermaid` fences inside `.md`, never standalone `.mmd` in tree; exactly one page owns them, every other page links. Page already keeps copy → delete copy in same change that adds owner page — two diagram sources drift like two prose sources. Markdown rendering is part of doc correctness, review as output: escaped backtick inside code span splits it into code, stray backslashes and code — use double-backtick delimiter when content has backticks; no markdown renderer installed → verify by counting delimiter runs on line, not eyeballing.

   Change touching every call site — argv rewrite, rename, signature change — row also carries **parity contract**: which behaviours must not move, traps that break them, test counts before and after. "Refactor X to Y" invites silent behaviour change; enumerated contract does not. CLI rewrites: `references/cli-rewrite-parity.md`.

   **File list in spec is grep-verified, not remembered.** "`dispatch.rs` and
   `render.rs` are untouched, they only go through the trait" is a claim; two files still calling old driver mean surreal/stub build cannot compile — reviewer found exactly that after order approved. Before row names files unaffected or fixes their order, run `rg -n '<old driver>|Command::output' src/` per file, let hit count write list.

   Counts in goal (`cli.sh 39/39`, `202 passed`) go stale once anyone adds test; worker then "fails" target already exceeded or trims tests to hit number. Quote command + "current totals", or run suite at filing time — never remembered count.

   **`description` and `goal` both the prompt: correct in one pass.** Reviewer or user can overrule half a spec; easy to land new decision in `description` while old ruling sits in `goal`. Worker then blocks — correctly — asking which is authority, round spent on question you created. Before every dispatch, re-read `agentboard get T<n> --full`, check goal's required proofs match description's decision; decision changes → edit both fields same sitting, say in goal which section binding.

   Spec is **row you can amend**: `agentboard edit T<n> [--title T] [--description|--spec D]
   [--goal G] [--priority N] [--assignee A]` (same for `E<n>`, with `--path`), one `edit` event per field, `unchanged` and no write when text same. Fix row instead of refiling — refiling costs id and history. Guard = lease: row held by working agent refuses edit (exit 1, same `lease Nm left — manager: agentboard status T<n> ready` remedy as `claim`), since worker already reading text. So: amend freely while `ready`; while `working`, wait for delivery (worker can `task_get full=true` again) or reset with `status T<n> ready` and re-dispatch. Short fields record old→new values in event; description and goal record lengths only — big spec edit not restorable, so prefer refiling when change structural, not correction.
3. **Dispatch with verb**, not hand-typed sentence:
   `agentboard dispatch T<n> --to <session>`. Refuses task not `ready` (exit 1, no event, no write), `--force` resets through normal status->ready path first, `--print` mutates nothing, delivery is `herdr agent prompt` under deadline with tmux fallback, `dispatch` event written only after successful send. Deadline matters more than looks: undeadlined `Command::output()` against wedged pane hangs manager, not worker.   Read event body after — names delivering path and reason for any fallback (`w1 via tmux (herdr: timed out after 200ms)`), which "exit 0" does not.

   **One row per pane at a time.** `herdr agent prompt` submits into live agent's conversation, so dispatch to pane still `working` = extra user turn mid-task: drops second instruction or answers it with first task's context. Queue next row as `ready`, dispatch when pane reports — parallel work = second pane, never second dispatch.

   Dispatch fails naming agent you did not pass (`agent_not_found: reviewer` while you said `--to reviewer2`) → task's **assignee** had registration row shadowing target. Resolution now prefers *live* target — assignee-if-live, then `--to`-if-live — else refuses with one error naming every candidate, saying assignee's row stale. `agentboard sessions` marks such rows `gone`. Clean with verbs, not DB: `agentboard unregister <name>` for one, `agentboard sessions prune [--dry-run]` for rest. Prune refuses when neither tmux nor herdr answers — box that cannot see sessions must not delete registry — so run where herdr up. Only if verb genuinely missing, edit `board.db`, with python stdlib (`sqlite3`, no CLI binary on this box) and line in reply saying you did; manager reaching for DB routinely has stopped respecting board.
4. **Arm watcher before ending turn.** Step that gets skipped, not optional: no polling loops in python, no reading `board.db` by hand.

   ```bash
   agentboard wait T<n> --timeout 1800 --status review,blocking,needs_fix
   # exit 0 = matched (prints `matched:` + the task's status + the new event lines)
   # exit 3 = timeout (prints `timeout:` + one status line per watched task + a tail)
   ```

   **Review** task is exception: reviewer marks own row `review` when finished, like coder, so watch `done,review,needs_fix,blocking` for it — else wait runs full timeout while verdict sits on board (40-minute timeout spent on review landed 10 minutes in).

   Run as background job with notify. `--open --kind ask` other useful shape: wakes when *anyone* on board needs human. Blocked delegate shows as `ask`, CLI accepts as alias of real `block` event.

   **A short timeout is a stall check, and the manager is the loop — not a script.** `--timeout 300` is a fine choice for a task that should move within minutes: exit 3 means the row has not moved, so read the pane once (`herdr agent wait <pane> --until idle --timeout 1`) and push with `herdr agent prompt` only if it is idle; then arm the same wait again. What must not happen is wrapping that in a poller: a python watchdog and a shell supervisor were both written for this and both were worse than the command — one keyed pane state by agent kind and pushed a *busy* agent four times, the other needed a shared-board filter so it did not drive this repo's coder at another project's rows. `agentboard wait` already re-queries, already prints the offending status, and already has the exit codes; anything layered on top is a second, weaker implementation of it.

   Dispatch not finished until watcher armed; any question about delegate progress — from user or own doubt — answered by reading board, never asking pane or guessing from elapsed time. Ending turn on "it's probably still working" is how finished task gets reported as running.

   Facts that matter reading its output:

   - `--open` re-queries each poll, so a task created mid-wait is picked up (verified:
     matched 204 ms after the row appeared). Explicit ids and `--epic` stay frozen —
     they name work, not a predicate.
   - A `--status`-only match on a task already in that status returns at once, and
     prints its most recent event prefixed `last:` — that event is context, not the
     cause of the match. So never add a status the row already holds to the watch set:
     re-watching a `review` row matches on the first poll and the watcher exits having
     seen nothing new. Watch the row you just dispatched, with only the statuses it is
     heading toward.
   - A match is a snapshot, and the notification arrives later than the event: a row can
     already be `done` when the message says `needs_fix`. Re-read `agentboard ls` and
     `agentboard get T<n>` on every wake-up before acting, and never re-dispatch or reopen
     on the strength of the notification alone.
   - On timeout it hands back every watched task's status plus a clipped tail of the
     freshest one, so a silent timeout is still evidence.
   - `--status`/`--kind` names are validated: a typo exits 1 instead of quietly
     matching nothing. `reclaim` is not a kind (it is logged as `claim`).
   - For a `--status`-only match, `--after` is a no-op by definition: the match is
     current state, not an event.
5. **On wake, read row, then re-run gates yourself.** Completion = board `report` **plus your own** runs, real command output in reply to user. Never pane's word "done", never report's claims. `agent_status: idle` = "ready for input", not "finished".

   **Claimed platform blocker = finding until reproduced.** "CREATE does not persist", "the deserializer breaks on single-letter fields", "the crate will not compile" — worker hitting wall in spike usually hits bug in own probe (wrong namespace, unawaited query, connection reused wrong way); design calling its core mechanism unverified is not a design. Run smallest independent probe that would fail if claim false, in `$TMPDIR`, before accepting report or sending to review. Recipe + traps that make probe lie: `references/worker-claim-probes.md`. Then rule on claim in writing: verified, disproved with output, or unverified-with-version; file fix against doc that stated it.

   **Green report dies at worker's last edit.** Workers routinely land one-line clippy/fmt fix *after* last full suite run, then report earlier numbers; pasted count no longer describes tree you commit. Read report for tree state it measured; worker admits post-run edit → fix task or reviewer carries FULL suite again — not just changed phase — since compile-level tweak can move later phase's output.

   **Green once not green.** Suite asserting anything time-shaped — relative age in rendered row (`created: 0s`, `lease: 59m left`), boundary between two calls, TTL countdown — passes/fails by where second hand was. Run suite three to five times before believing; reviewer reports "N/N but flaky" → treat flake as defect same severity as failing test: next session inherits untrustable gate.

   **Gate that skips not a gate.** Test needing external binary (database daemon, CLI tool) prints `SKIP <name>: not found` and still exits 0, so run that matters — host that DOES have dependency — can be green asserting nothing. Give suite strict switch (`AGENTBOARD_REQUIRE_SURREAL=1`-style) turning skip into failure, run strict mode wherever dependency installed, add both rows to repo's check table: default run and strict run. Verify strict switch honestly: hide binary (`PATH=/nonexistent
   HOME=/nonexistent`), confirm tests FAIL not skip.

   **Test-only escape hatch must not exist in release binary.** Env var disabling write lock so mutation check proves lock load-bearing = good tool and live-board hazard: if `AGENTBOARD_UNSAFE_NO_WRITE_LOCK=1` works in release build, one shell export corrupts board. Require switch `cfg(test)`/debug-only, or refuse in release, check in built artifact (`strings`/behaviour), not only source.
6. **One fresh review task per review round**, in pane separate from coder — seat follows quota: Claude reviewer with `reviewer-mcp.json` while Claude has room, else second opencode pane with own actor. Dispatched same way. Verdict goes on board as its report. `needs_fix` verdict becomes next coder task — fix task is **subtask of review that produced it**: file with `--parent T<review>` (epic inherited from parent), so `children T<review>` answers "what did this review cost" instead of link only in prose. Same for anything else task spawns: child records cause, not just epic. Filed flat by mistake? `agentboard parent T<n> T<m>` repairs link after — accepted on any status, `done` included, logs `edit: parent -> T<m>`, `children` then answers from both ends. Wrong link never needs refile.

   **Closing fix belongs to reviewer, not author.** Coder fills fix task, stops at `review` — no `fixed` status — with report and note. Only reviewer moves fix task on: `needs_fix` with remaining findings as note, or `done` with evidence; only done fix task lets it close review task. State in both rows: coder closing own fix self-certifies, reviewer opens loop already shut.

   **Adjudicate findings before fix dispatched.** Reviewer reads code, not standing rule, so can file finding contradicting it ("the fix task never reaches done"), coder faithfully implements wrong thing. While fix row still `ready`, amend: `REJECTED` block naming each rejected finding with rule it breaks, then valid ones as actual work, explicit "do not apply R<n>" line. `task_edit` refuses only while lease live, so `needs_fix` review and `ready` fix both amendable; once coder claims, round spent. Read child fix row for findings too — reviewer may set review row `needs_fix` with empty report, every finding in fix task.

   **`needs_fix` verdict with no child row is yours to file.** Reviewers sometimes report finding in note and stop. Read note, file fix as child of that review with note's findings quoted and proofs to paste, dispatch; re-prompting reviewer for paperwork costs round, finding already known. Say in child's description manager filed on reviewer's behalf.

   **Reviewer reproduces measurement, not reads it.** Round turns on numbers (contention rates, lost updates, latency) → reviewer re-runs experiment with own probe and transport, pastes own output. Different rates expected — one reviewer's HTTP probe duplicated in 1 of 10 runs where coder's ws probe saw 2 of 10 — direction and conclusion must match. Verdict quoting author's table back not evidence.

   **Parity transcript sleeping between writes hides ordering hole.** Two backends sorting `seen DESC` and `seen DESC, name` print same transcript only because test registers rows 50 ms apart; real board sorts same-millisecond rows differently. Require same-timestamp case and one stable tiebreak implemented in BOTH backends; "identical transcript" unproven until that case exists.

   **Two workers same vendor still need two actors.** Two opencode panes (quota fallback) → each pane own `AGENTBOARD_ACTOR` — `herdr tab create
   --env`, `herdr pane split --env`, each pane started `--standalone` — else every ledger event reads `opencode`, review round cannot name writer. Mechanics: `opencode` skill, "Two panes, two board actors".

   Ask for **revert proof** on behaviour fixes: author reverts own fix, shows named test fails, restores. Test passing on broken code = decoration; review round only place this caught.

   Revert-proof necessary not sufficient — harness itself can be broken two ways that both look green:

   - **The assertion accepts success.** A helper written as `try: return "ACCEPTED: " +
     call(...) except: return str(e)` then handed to a substring check passes whether the
     call succeeded or was refused. Assert the *shape* (an error was raised, exit code is
     non-zero), never a string that both paths can produce.
   - **The assertion measures the wrong thing.** A hang test that only times the caller
     still passes when the kill is deleted — the stub survives, the caller returns fast.
     Assert the observable: the pid is gone, the event count, the exit code, the row state.
     The same trap catches a reviewer's own tooling: a "no stray process left" check run as
     `pgrep -x surreal` printed zero every time because the daemon is `surreal-db`, which is
     an all-clear the matcher could only ever produce. Verify such a check by creating the
     state it looks for, or match on something unambiguous like the data directory in `ps`.

   So demand two mutations per guard: revert fix (fails?) and force failure path to succeed (still fails?). Survives either = untested, not passing.

   Verdict belongs to reviewer that produced it. Review role changes hands (different vendor/model takes seat) → accumulated commits get one fresh pass from reviewer of record before anything called merge-ready — earlier approve is another reviewer's opinion, not citable gate. Say on board which reviewer ran, so later session can tell apart.

   **Required** fix from reviewer of record jumps queue. User's next feature does not outrank merge gate: land fix, re-verify, commit, resume queue — everything built after would sit on unapproved base, finding cheapest to fix while reviewer's evidence fresh. Optional findings queue like other work.
7. **Freeze tree while reviewer reads it.** No next coder round until verdict lands: diff moving under reviewer yields findings against code no longer existing, whole verdict thrown away. Behaviour change after review makes old verdict stale evidence — commission second round, don't cite it. Two rows editing same file (docs task and feature task both touching `README.md`) serialized for same reason: spec says which row owns shared file, dispatch other after.
8. **Commit landed round without being asked; never push.** Standing default: implement or design task reaching `done` gets reviewed files committed locally, one objective per commit, sha in reply — user asking "commit it" every round does manager's bookkeeping. Manager commits; workers told not to. Push stays human's call, always; so does history rewrite. `git add` explicit paths — never `-A`, sweeps workers' scratch files into commit.

   **Split by objective, leave worker's in-flight edits unstaged.** Two finished rounds in one tree = two commits, explicit paths each; count table (`AGENTS.md`) moves in commit that changed tests it describes. Third change worker holds — manifest bump for dependency later epic adds — stays out of both: stage only reviewed paths, say in reply what left behind and why.

   Other half: before commit, read `git status` for files same step touches and is about to leave behind. Seam function new CLI path calls (`board_path()` living in `store/mod.rs`) is part of step "CLI adapter" even though step title names two other files; staging by title leaves commit that doesn't build alone. Stage every reviewed file step needed; amend commit rather than second "oops" commit when noticed immediately.

   **No install while manifest carries unreviewed changes.** `cargo install --path
   . --locked` builds whole dependency graph, so installing mid-migration compiles crate under test, pins binary from unreviewed tree. Panes keep old binary until install; behaviour/wording change not installed yet → state plainly, don't report as live.

   **Re-read git state at report time.** User has shell in same repo, can push/commit while you work, so earlier `git status` not evidence about now: before saying "N commits ahead, unpushed" or "HEAD is X", run `git status -sb` and `git ls-remote origin master` and compare — or say nothing about push state. Stale ahead-count = false claim about tree.

## Board surface the manager uses

| Command | What it is for |
|---|---|
| `brief` | counts, open tasks by urgency, recent completions — standup; groups open epics with path and `[N blocking, N needs_fix]` |
| `ls [--status S] [--assignee A]` | compact open list |
| `epic add "<title>" [--path P] [--description D] [--goal G]` | container: own table and id space, never leaseable |
| `epic ls [--status open\|done\|dropped]` / `epic get E<n> [--full]` / `epic status E<n> <s> [note]` | read and close epics |
| `children E<n>` / `children T<n>` | epic's tasks, or one task's subtasks — two different questions |
| `parent T<n> <T<m>\|none` | re-point subtask's parent (epic inheritance re-checked) |
| `add "<title>" --description D --goal G [--assignee A] [--epic E] [--parent T] [--priority N]` | file row; `--spec` alias of `--description` |
| `get T<n> [--full]` | the row; `--full` for verbatim description/goal/report/note/events. Plain mode folds to one line, clips **with explicit marker** |
| `status T<n> <s> [note]` | manager moves status without claiming. `ready` clears lease, re-arms crashed task |
| `note` / `report` | progress / outcome, no status move |
| `ask` / `answer` | block on human, resume with fresh lease |
| `dispatch T<n> [--to S] [--force] [--print]` | hand row to pane (see step 3) |
| `wait [T…] [--epic E<n>] [--open] [--status csv] [--kind csv] [--after SEQ] [--timeout SEC]` | block until board moves (see step 4); `--epic` takes E id, selects every task under it, subtasks included |
| `sessions` / `register <agent-name> --agent claude\|opencode [--task T]` | map task -> pane; **register herdr agent name, never pane id** |
| `sessions prune [--dry-run]` / `unregister <name>` | delete rows no live pane answers to; rows never expire alone |

Statuses: `ready working blocking needs_fix review done dropped`; epic statuses:
`open done dropped`.

Two refusals to recognise on sight:

```text
T7 working by claude (lease 24m left, until 13:23Z) — manager: agentboard status T7 ready
T9 is review, not ready (assignee opencode, lease none); --force resets it to ready first
```

First is `claim` saying lease live; remedy in text works (clears lease), so crashed holder costs one command, not 30-minute TTL.

Both **errors, not replies**: explicit-id `claim` refusal and `dispatch` refusal exit 1; over MCP same text arrives as tool error. So error from `claim` means *someone holds this row* — read text, apply remedy — not *board is broken*; no blind retry, don't report tool down. Id-less probe opposite case, stays success: `claim` with no id on empty board exits 0, prints `no ready tasks` — answer, not failure.

**Manager does not `claim`.** Manager claim takes lease, coder's claim then refused. Use `status T<n> working "<note>"` to show intent, let worker claim.

## MCP equivalents

Driving board over MCP instead of shell: `board_brief`, `task_list`,
`task_add`, `task_get` (`full: true`), `task_claim`, `task_update` (status / report /
note / commit), `task_edit`, `task_ask` + `task_wait`, `task_children`, `epic_add`,
`epic_list`, `epic_get`, `epic_update`, `epic_edit`, `session_list`.
`task_wait` carries same selectors as CLI `wait`; no exit codes, so puts `matched:` or `timeout:` on first line — branch on that. No MCP `parent` verb: re-pointing subtask over MCP = CLI call.

Workers must have `task_claim` and `task_get` in allowed toolset; without them they work and report success on task never claimed.

## herdr: the minimum a manager needs

Full pane mechanics and start flags: `team-manager` (references/herdr-mechanics.md).
What loop needs:

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

- **`herdr agent prompt <name> ... --wait` belongs in background job.** Foreground hits tool deadline, gets yielded, reads like failed delivery while prompt already submitted. Run `background=true, notify=true`; returns timeout but pane shows agent working → don't resend — resend injects second turn into running task. Confirm on board (`agentboard ls`, `agentboard get T<n>`).
- **Names are the address.** Distinct names let two teams run at once (`coder2`,
  `reviewer2`); `agent_name_taken` cannot clear without closing that pane.
- **Workers write to board through MCP tools, not by shelling binary.** `agentboard dispatch` used to send "Claim T8, then task_get id=8 full=true", coder answered with bash step running CLI claim: same effect, extra subprocess, unstructured output, wrong shape for agent. Say tool name in row or prompt (`task_claim`, `task_get`, `task_update`), keep CLI for verbs MCP lacks and verification commands goal lists, check live pane for what worker actually did, not trusting report.
- **Reviewer approval not verification: re-read artifact yourself.** Coder can leave doc/diagram describing opposite of rule it states, reviewer can pass it: two rounds approved `docs/review-loop.md` while flowchart put reviewer-owned `done` node inside Coder lane. Read artifact, check against rule, not only command outputs in report. Manager findings go on new fix task (`parent=<the review task>`, assignee coder) so reviewer still closes it; never edit tree yourself.
- **Row is the prompt: write no prompt files.** Long text into `task_add`/`task_edit` as JSON over MCP, never scratch `.txt` a wrapper `cat`s. Scratch copy = second prompt that drifts; user reads it as breaking board-is-only-channel rule.
- **herdr is pane host; tmux legacy.** Build and test herdr paths first. Verb speaking only tmux = bug for this team; `parse_herdr_agents` dropping `pane_id` is what makes it one (`tail`/`send` were tmux-only while `sessions`, `dispatch` and `prune` already used herdr). Keep tmux fallback untouched rather than improving; spend time on tmux only when nothing else to do.
- **Renew pane's conversation between cycles, never inside one.** Cycle = task -> review -> fix -> done; next task starts with cleared context: `herdr pane send-text <pane> "/clear"` then `herdr pane send-keys <pane> enter` (pane must sit at prompt; open stats/usage overlay eats text, so send `esc` first, read pane to confirm). Do after task closed, before next dispatch, coder and reviewer separately. Fix task spawned by review belongs to SAME cycle, not renewal point: clearing before fix drops context carrying findings, fix row only thread back. Renew when review and every fix under it `done`. Never clear pane `working` or mid-review: row is its only prompt, cleared pane loses thread.
- **Pane never closed to free name or as cleanup.** Not while `working` or `blocked`, only when user asks.
- **`blocked` usually dialog or TUI at prompt**, not hung agent: read pane before touching. Opencode reviewer may sit on "allow web search?" — local diff review doesn't need it.
- **Don't run `herdr-agent-open` from manager shell.** Splits *current* pane, needs `HERDR_PANE_ID`; kills old agent first, then dies at split, leaving project with no agent. Create tab and panes explicitly by id.
- Verify `cwd` in `agent list` before dispatching. Pane in wrong directory = one failure silently producing work in wrong repo.

## Keeping the tool current

- `agentboard` runs from two binaries: `~/.cargo/bin/agentboard` (installed — what shell and agent MCP configs execute) and `./target/release/agentboard` (checkout). After changing tool, `cargo install --path . --locked`, or verbs just built missing from binary you dispatch with.
- **Install with backend feature host actually runs.** Once store feature-gated, bare install builds default backend, so surreal host installing without `--no-default-features --features surreal` quietly gets sqlite binary again — new file, empty board, agents reporting `no ready tasks` while real store untouched. Install from reviewed tree with host's feature set, prove which installed by BEHAVIOUR, not mtime or size: hand it path other build would refuse (sqlite file to directory-backed build, directory to sqlite build), require refusal. `strings` match or fresh timestamp only says something built; refusal says which backend executes. Say in reply which build live.
- `tests/mcp_client.py` drives `target/release/agentboard`. Rebuild before running, or green suite tests week-old code.
- **Storage engine swap gated on multi-process safety, not schema.** Board opened by CLI, one MCP server process per agent, suites spawning N workers; embedded engine serving per-process cache cannot carry that, in-process mutex or flock doesn't fix stale view. Prove cross-process atomicity and read coherence before any port written; design whose core mechanism still marked unverified = rejection. Probe recipe + interpretation table: `references/worker-claim-probes.md`, "Multi-process safety probe". Engine facts, measured numbers, local-server-plus-lock design that survived: `storage-engine-swap` skill, its surrealdb-2x reference.
- **Schema change needs switch, not migration.** Board has no migrate path: old file refused before any DDL with one line naming `mv <db> <db>.old`, file left byte-identical. History expendable, so switch is wipe — but order matters, agents' MCP servers are processes of *old* binary:

  1. stop every agent, close panes (nothing may hold board) — including own MCP board server, equally old-binary process: after switch, drive board by CLI until session restarts, or every MCP board call fails against unknown schema; 2. `cargo install`
  new binary, 3. move `board.db` (plus `-wal`/`-shm`) into archive directory, 4. run one read (`agentboard brief`) so *new* binary creates schema, 5. re-register teams by name, 6. refile rows still open — every id died in wipe, queue rebuilt from specs you still hold.

  Skipping step 1 = failure: old-binary server answering request recreates old schema in new file, new binary then refuses own board.
- **Reset without schema change = same wipe, minus binary swap.** Resolve path first with `agentboard path`: `$AGENTBOARD_DB` (often set in MCP config's env block) overrides state dir, so deleting default path can miss live board. Confirm board idle before wiping — every task `done`/`dropped`, no live lease — then `cp -p board.db board.db.bak-<ts>`, `mv board.db board.db.old-<ts>`
  (plus `-wal`/`-shm`), run one read (`agentboard brief`) so fresh schema lands, verify empty counts through python's `sqlite3` (no `sqlite3` CLI on this box), only then delete `.old` duplicate: `.bak` is restore path, outlives session. Say in reply where backup sits and when to drop it.
- **Reset empties `sessions` too.** Panes keep old MCP server process (store reopens path per call, no stale handle), but registry rows gone — re-register names before dispatching, or `dispatch` answers `agent_not_found` on team running fine.
- Agent panes spawn MCP server from PATH binary at session start, keep that build until pane restarts. Schema change dangerous case; new flag merely invisible.
- Parsing has own suite, `tests/cli.sh` (exit codes, `--flag=value`, unknown-flag errors, hyphen values), delivery has `tests/dispatch.sh` (refusal, `--print`, `--force`, assigned-vs-`--to`, fallback reason, deadline kill). Run `mcp_client.py`, `cli.sh` and `dispatch.sh` before believing CLI or dispatch change.
- **Test delivery with stub binaries on PATH, never live pane.** Two-line `herdr`/`tmux` script that exits, fails, or sleeps proves fallback and deadline in milliseconds; real dispatch types into someone's working agent, cannot replay.
- **"Every call now goes through X" is a grep you owe.** Unifying mechanism doesn't retire hand-rolled copies: after shared runner landed, `sessions` still had own spawn-and-join with fixed deadline, so README line "the env var bounds every bridge call" and commit message both false — path took 8 s under stub that should have been killed at 300 ms. Before claiming coverage, search old pattern (`search_files` / `rg`: `Command::output`, old constant, second `std::process::Command` in same module), make claim acceptance criterion of a test, not sentence in commit.
- **Count in prose = claim to re-derive, not copy.** Header comment "18 tools", README test total, worker report line `cli.sh 102/102` — all drift within one round; quoting one back to user = wrong statement of fact. Derive from source at verification time (`rg -c '#\[tool\(' src/server.rs`,
  `cargo test 2>&1 | rg '^test result'`), fix prose where it disagrees with command output.
- **Verb that lands retires skill saying it's missing.** Feature not finished while playbook still teaches workaround: after round, grep skill library for old claim (`rg -n 'no edit verb|there is no|not yet|workaround'` in skills dir), patch sentence in place, check reference files too — stale `pitfalls.md` bullet sends next session down path tool closed.
- **Playbook also shipped inside repo → edit repo copy, install from it.** Repo copy canonical, installed one snapshot; editing installed copy creates drift next `install.sh` silently reverts: edit in tree, `skills/install.sh --force`, then `diff -r` both ways. Comparison against something *outside* tree, so result time-shaped — worker reporting `diff -r` empty means empty when it looked, not at commit time. Re-run diff yourself at verification, hold commit until match; copy task whose source keeps moving lands stale, and stale half is what future session reads.

## What "done" means

1. Task `done` on board, report names files and pastes real output.
2. The report carries the named gates' real output from that tree — tool name, counts, exit codes — and the manager reads it for completeness rather than re-running it. Owner's rule: **the manager manages.** No builds, no ctest, no parity runs, no code edits from the manager's own shell; a green claim that looks thin, stale or self-contradictory goes back as a review row or a question to the worker, not as a gate the manager runs itself.
3. Review task agrees, or required fixes landed and second round says so.
4. Reviewed files committed locally (step 8), one objective per commit, sha in reply. Push and history rewrites stay human's call, always.

## Close the round: hygiene tasks

Feature round not finished when review says approve. File hygiene work before calling done, or it accumulates until noise hides real warning:

- **clippy**: `cargo clean -p <crate>` then `cargo clippy --all-targets` — cached runs replay warnings from earlier builds, lie about count (8, 6 and 2 all "same" tree at different cache states).
- **fmt**: `cargo fmt --check`; never-formatted repo reports thousands of diff lines.
- Two separate tasks, clippy first, fmt second; **fmt never runs while reviewer reading tree** — mechanical 1000-line diff buries change under review. Commit separately, same reason: clippy fix reviewable, formatting sweep not.
- Add gate to whatever repo calls its checks (`README`, `justfile`, CI) so next round inherits it instead of repeating task.