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

Manager owns both ledgers, writes **no code**. No patch, no plugin, no shell-driven edit. Hands: board writes, dispatch, watching, reading panes, re-running gates, git-state resets, talking with user. Worker makes every edit. "Review the code" = commission reviewer, not fix it.

One override: user says do independent row yourself, now, and it touches files no worker/reviewer holds → do it. Pick docs-only row, files disjoint from in-flight task, never interrupt working pane for it. Record exception in row: `status T<n> working "<manager> writing it directly"`, then `report` with evidence, then `status T<n> done "committed as <sha>"`. Put "no reviewer round ran" in report so ledger not imply gate that never ran. Row editing files coder is inside = not independent, queue it.

Only time to interrupt working pane: **doc or design row whose spec you rewrite**: work so far is prose going wrong way, no code/reviewer holds it, finishing costs second full round. `herdr agent send-keys <name> esc`, then `agentboard status T<n> ready "<why>"` to drop lease, land both fields, dispatch again. Never on code row mid-edit: tree keeps whatever half it wrote.

Repo housekeeping = code work too, even one line: `.gitignore` entry, manifest bump, CI yaml, README correction code contradicts. Test not "is it trivial" but "is it the repo's": file row, worker edits, so every write in tree has author on board. One exception: file coder is mid-edit in, don't touch at all.

This skill = operating loop. Pane mechanics + step-by-step procedure: `team-manager` skill, whose `references/herdr-mechanics.md` has pane depth (`herdr --skill` stays authority shipped with binary). Machine setup, MCP configs, wrapper bins: `references/machine-provisioning.md`. Costly mistakes: `references/pitfalls.md`. Reproducing worker's "the engine cannot do this" claim: `references/worker-claim-probes.md`.

Same job as short procedure (topology, epic/task recipes, dispatch, review loop, renew cycle, recovery table) = `team-manager` skill. Use it for commands in order; this file = reasoning + traps.

## When to Use

- You manage coder + reviewer team, work lives on board.
- Work needs filing, dispatching, watching, or report verifying.
- Review round to commission, or findings to turn into fix task.
- Worker blocked, silent, or task behind live lease.
- About to poll board by hand: stop, use `wait`.
- NEW machine must run team (claude/agentboard/MCP configs, wrapper bins, verification): `references/machine-provisioning.md`.
- Board must start fresh: plain reset or schema switch ("Keeping the tool current").

## The one rule that shapes everything

**Task row is the prompt.** `description` = context (repo, files, situation, what to build); `goal` = how to do it + how judged finished. `claim` returns both **verbatim and uncapped**, so worker gets prompt in one call. Never stage prompt text in file: no `prompt.txt`, no `desc.txt`, no `"$(cat
scratch.md)"` handed to `add`. Row is only copy; user treats second copy as defect even if deleted after. Long, backticked, multi-paragraph text → MCP `task_add` / `task_edit` with JSON argument, avoids shell quoting; CLI form for short one-liners.

Name work, not agent in seat. Workflow, doc, row stay agent-agnostic (`docs/review-loop.md`, "Review: review-loop.md"), since coder/reviewer pair changes when quota runs out; name current agent only in one clearly labelled setup example in body.

So dispatch = one line, board owns text:

```bash
agentboard dispatch T15 --to coder2
# dispatched to coder2 via herdr: Use the agentboard MCP tools: claim task T15 with the
# claim tool, read it with the get tool (full=true), then execute it. Do not shell out
# to the agentboard binary for board writes.
```

Line names MCP tools on purpose: CLI-shaped line (`Claim T8, then task_get
id=8`) taught workers to run binary in bash instead of calling tool. Pane still gets old wording → installed binary predates change; check `strings ~/.cargo/bin/agentboard | rg 'claim tool'` before blaming row.

## The loop

0. **Orient before acting.** `agentboard brief`, `agentboard ls`,
   `agentboard sessions`, `herdr agent list`. Never dispatch into board or pane you haven't looked at; stale `working` or pane in wrong repo stays invisible until work comes back wrong.
1. **Epic first, then work under it.** `agentboard epic add "<outcome>" [--path <repo>]
   [--description D] [--goal G]` creates container, then tasks go in with
   `--epic E<n>`. Epics and tasks: two tables, two id spaces: epics print `E1`, tasks `T1`, prefix picks table, so `--epic E3` and `--parent T7` never mean same thing. Task may sit under epic **and** be subtask of another task (fix task under review task that found it); subtask inherits parent's epic; disagreement, cycle, self-parent, cross-epic parent refused. Re-point: `agentboard parent T<n> <T<m>|none`.
   Epic never claimable, leaseable, dispatchable; every task verb refuses E id and names epic verb you wanted.

   Give epic its **project path** (`--path`, `~` and relative paths resolved, existence not required). Only property no task has; lets dispatch answer "is pane in right repo": live target's cwd outside epic path → `dispatch` prints warning naming both, proceeds. Warn, not refuse: manager decides, but only if told.
2. **Write row so worker can self-verify.** Goal must be executable: "`cargo test` prints 4 passed", "`rg 'unwrap()' src/` returns nothing", "these three commands and their output pasted into the report". "Implement X" not a goal. File long or backtick-heavy text over MCP (`task_add` with JSON string) so shell quoting never eats span. Spec long enough (five numbered findings + checks) can break single call: file row first with one-line description, then land body with `task_edit` on `description` and again on `goal`. Two edits = two `edit` events.

   **Entries in one `calls[]` are independent.** Entry missing `name` rejected (`calls[0] requires a 'name'`) while siblings write, so error names one call, hides ones that landed. Re-list (`epic_list`, `task_list`) after any batched error before retry, else batch filed twice.

   **Resolve stated constraint's physical shape before filing.** User line like "still use the db file" = two possible specs: path contract (`AGENTBOARD_DB`, `--db`, `path`) or one physical file on disk. Embedded store may only offer directory, so encode reading you can implement, say in row which one, put impossible reading in reply as question. Never file spec asking for shape engine can't produce. Same for tech: user names crate or database → scope row to that one; no bake-off; cheaper-option idea goes to user, not worker's row.

   **Two documents disagreeing = doc bug in authority, not a section.** Playbook contradicts wire doc agents read first (PROTOCOL.md) → fix wire doc, delete "divergence" note from playbook; page documenting split keeps split alive, each worker picks half it read. Ask for one concrete numbered workflow, one step per action, plus who-moves-what status ownership, and one doc owning diagrams, pointed at from both sides.

   Diagrams = section of markdown file: `mermaid` fences inside `.md`, never standalone `.mmd` in tree; exactly one page owns them, others link. Page already holds copy → delete copy in same change adding owner page; two diagram sources drift like two prose sources. Markdown rendering is part of doc correctness, review as output: escaped backtick inside code span splits it into code, stray backslashes and code. Use double-backtick delimiter when content has backticks; no renderer installed → verify by counting delimiter runs on line, not eyeballing.

   Change touching every call site (argv rewrite, rename, signature change) → row also carries **parity contract**: which behaviours must not move, traps that break them, test counts before and after. "Refactor X to Y" invites silent behaviour change; enumerated contract doesn't. CLI rewrites: `references/cli-rewrite-parity.md`.

   **File list in spec is grep-verified, not remembered.** "`dispatch.rs` and
   `render.rs` are untouched, they only go through the trait" is a claim; two files still calling old driver = surreal/stub build can't compile. Reviewer found exactly that after order approved. Before row names files unaffected or fixes their order, run `rg -n '<old driver>|Command::output' src/` per file, let hit count write list.

   Counts in goal (`cli.sh 39/39`, `202 passed`) go stale once anyone adds test; worker then "fails" target already exceeded or trims tests to hit number. Quote command + "current totals", or run suite at filing time. Never remembered count.

   **`description` and `goal` both the prompt: correct in one pass.** Reviewer or user can overrule half a spec; easy to land new decision in `description` while old ruling sits in `goal`. Worker then blocks (correctly) asking which is authority, round spent on question you made. Before each dispatch, re-read `agentboard get T<n> --full`, check goal's proofs match description's decision; decision changes → edit both fields same sitting, say in goal which section binds.

   **Row starting from manager's own uncommitted edits must list them, file by file, line by line.** Worker inheriting dirty tree can't tell your half-edit from own code: duplicate validation, dead guard, comment on wrong branch all read as coder's work, reviewer then files your leftover as finding against row ("redundant guard, remove line N"). Say what you touched and that fmt already reformatted some, so round starts from known state. Expect fix row for duplication you seeded.

   Spec is **row you can amend**: `agentboard edit T<n> [--title T] [--description|--spec D]
   [--goal G] [--priority N] [--assignee A]` (same for `E<n>`, with `--path`), one `edit` event per field, `unchanged` and no write when text same. Fix row instead of refiling; refiling costs id and history. Guard = lease: row held by working agent refuses edit (exit 1, same `lease Nm left — manager: agentboard status T<n> ready` remedy as `claim`), since worker already reading text. So: amend freely while `ready`; while `working`, wait for delivery (worker can `task_get full=true` again) or reset with `status T<n> ready` and re-dispatch. Short fields record old→new in event; description and goal record lengths only. Big spec edit not restorable, so refile when change structural, not correction.
3. **Dispatch with verb**, not hand-typed sentence:
   `agentboard dispatch T<n> --to <session>`. Refuses task not `ready` (exit 1, no event, no write), `--force` resets through normal status->ready path first, `--print` mutates nothing, delivery is `herdr agent prompt` under deadline with tmux fallback, `dispatch` event written only after successful send. Deadline matters: undeadlined `Command::output()` against wedged pane hangs manager, not worker. Read event body after: names delivery path and fallback reason (`w1 via tmux (herdr: timed out after 200ms)`), which "exit 0" doesn't.

   **One row per pane at a time.** `herdr agent prompt` submits into live agent's conversation, so dispatch to pane still `working` = extra user turn mid-task: drops second instruction or answers it with first task's context. Queue next row as `ready`, dispatch when pane reports. Parallel work = second pane, never second dispatch.

   Dispatch fails naming agent you didn't pass (`agent_not_found: reviewer` while you said `--to reviewer2`) → task's **assignee** registration row shadowed target. Resolution now prefers *live* target (assignee-if-live, then `--to`-if-live), else refuses with one error naming every candidate, saying assignee row stale. `agentboard sessions` marks such rows `gone`. Clean with verbs, not DB: `agentboard unregister <name>` for one, `agentboard sessions prune [--dry-run]` for rest. Prune refuses when neither tmux nor herdr answers (box that can't see sessions must not delete registry), so run where herdr up. Only if verb truly missing, edit `board.db` with python stdlib (`sqlite3`, no CLI binary on this box) and say so in reply; manager routinely reaching for DB has stopped respecting board.
4. **Arm watcher before ending turn.** Step that gets skipped, not optional: no python polling loops, no reading `board.db` by hand.

   ```bash
   agentboard wait T<n> --timeout 300 --status review,blocking,needs_fix
   # exit 0 = matched (prints `matched:` + the task's status + the new event lines)
   # exit 3 = timeout (prints `timeout:` + one status line per watched task + a tail)
   ```

   **Review** task is exception: reviewer marks own row `review` when finished, like coder, so watch `done,review,needs_fix,blocking` for it. Else wait runs full timeout while verdict sits on board (40-minute timeout spent on review landed in 10).

   Run as background job with notify. `--open --kind ask` also useful: wakes when *anyone* on board needs human. Blocked delegate shows as `ask`, CLI accepts as alias of real `block` event.

   **Short timeout = stall check, and manager is the loop, not a script.** `--timeout 300` fine for task that should move within minutes: exit 3 = row hasn't moved, so read pane once (`herdr agent wait <pane> --until idle --timeout 1`), push with `herdr agent prompt` only if idle, then arm same wait again. Never wrap in poller: python watchdog and shell supervisor both written for this, both worse than command. One keyed pane state by agent kind and pushed *busy* agent four times; other needed shared-board filter so it didn't drive this repo's coder at another project's rows. `agentboard wait` already re-queries, prints offending status, has exit codes; anything on top = second, weaker copy.

   Dispatch not finished until watcher armed. Any question on delegate progress (from user or own doubt) answered by reading board, never asking pane or guessing from elapsed time. Ending turn on "it's probably still working" is how finished task gets reported as running.

   Facts that matter reading its output:

   - `--open` re-queries each poll, so task created mid-wait gets picked up (verified:
     matched 204 ms after the row appeared). Explicit ids and `--epic` stay frozen —
     they name work, not a predicate.
   - `--status`-only match on task already in that status returns at once, and
     prints its most recent event prefixed `last:` — that event is context, not the
     cause of the match. So never add a status the row already holds to the watch set:
     re-watching a `review` row matches on the first poll and the watcher exits having
     seen nothing new. Watch the row you just dispatched, with only the statuses it is
     heading toward.
   - Same per id. Multi-id watch whose set includes row already sitting
     in a watched status (`wait T10 T14 T15 --status done`) matches on the first poll and
     returns `matched: 1 of 3`, so the wake you actually wanted never arrives. Put only
     rows that are still moving into the set, and after a false wake re-arm with the
     remainder rather than concluding the work was already done.
   - Match is snapshot, notification arrives later than event: row can
     already be `done` when the message says `needs_fix`. Re-read `agentboard ls` and
     `agentboard get T<n>` on every wake-up before acting, and never re-dispatch or reopen
     on the strength of the notification alone.
   - On timeout returns every watched task's status plus clipped tail of the
     freshest one, so a silent timeout is still evidence.
   - `--status`/`--kind` names validated: typo exits 1 instead of quietly
     matching nothing. `reclaim` is not a kind (it is logged as `claim`).
   - For `--status`-only match, `--after` is no-op by definition: match is
     current state, not an event.
5. **On wake, read row; delegate runs gates, not you.** User's standing rule: manager reads report against goal, checks each pasted line and count, hands any re-run to idle worker or reviewer (`herdr agent prompt`). No suites, builds, installs from manager's shell unless user asks. Completion = board `report` naming each gate with real last line. Never pane's word "done", never unbacked claim. `agent_status: idle` = "ready for input", not "finished". Evidence missing or hollow → move row back to `ready` with note naming gate, re-dispatch.

   **Coder tests own behaviour; reviewer reviews logic.** Worker that made change runs every gate goal names (build, unit and CLI suites, plus throwaway-board behaviour proof when change behavioural, `AGENTBOARD_DB=/tmp/…` style, never live board) and pastes real output into `report` before setting `review`. Row reaching `review` with gate unrun gets reopened. **Every executable step belongs to coder**, not just tests: `cargo install`, build for target, `devtool_build` / `devtool_deploy_target`, flashing board, running binary there and driving proving flow. Reviewer = team lead: reads diff and pasted evidence, reviews logic and edge cases, files feedback. Doesn't build, deploy, install, flash, drive hardware, or re-run coder's suites as second gate. May run own small probe or single targeted command to chase suspected edge case; that's the job, not full pass. Neither reviewer nor manager re-runs coder's suite routinely: duplicated cost, and pushes author to trust someone else's run.

   **Claimed platform blocker = finding until reproduced.** "CREATE does not persist", "the deserializer breaks on single-letter fields", "the crate will not compile": worker hitting wall in spike usually hit bug in own probe (wrong namespace, unawaited query, connection reused wrong). Design calling its core mechanism unverified is not a design. Run smallest independent probe that would fail if claim false, in `$TMPDIR`, before accepting report or sending to review. Recipe + traps making probe lie: `references/worker-claim-probes.md`. Then rule on claim in writing: verified, disproved with output, or unverified-with-version; file fix against doc that stated it.

   **Gate that cleans is not a gate.** Re-running worker's build must leave artifacts you came to inspect intact: `clean` flag on build tool can delete build dir, compile log, every test binary, still exit 0 with success-looking summary (`Attempted 1 tasks … all succeeded` because only task was the clean). Prefer plain build; want wipe → run as own step, re-check binary timestamps before quoting test result. Read-only gates first, destructive ones only when their output is what you need.

   **Report's own caveat is claim too: timestamp artifact.** "the unit-test targets were not compiled", "I could not configure a separate build", "only the library was built" arrive as honest self-doubt, get copied into review row as scope; check artifact instead of believing either side. Build log names output dir (cmake `--build <dir>`, cargo `--target-dir`, `-o` in compile line), and second build tree under same workdir often months old, making green build look missing: `find <dir> -name '<target>' -newermt '<diff time>'`. Fresh binary → caveat false, say so in review row so reviewer doesn't spend round on it. Old binary, no compile line → caveat real, goal not met.

   **Green report dies at worker's last edit.** Workers routinely land one-line clippy/fmt fix *after* last full suite run, then report earlier numbers; pasted count no longer describes tree you commit. Check which tree state report measured; worker admits post-run edit → fix task or reviewer runs FULL suite again, not just changed phase, since compile-level tweak can move later phase's output.

   **Suites report does NOT list = where red one hides.** Report enumerating
   "controller 36, adaptor 24, client 128, dbus 38 — all PASSED" silently chose its gate;
   list test targets build produced (`find <build dir> -name '*_tests'`), diff against names in report. Unlisted target not covered by claim, and is where collateral damage shows: suite worker never ran is one that fails. Have reviewer run missing ones before accepting round; one red → decide attribution with baseline binary before filing (failure predating diff belongs in row that removes behaviour, not fix task against this change).

   **Green once not green.** Suite asserting anything time-shaped (relative age in rendered row (`created: 0s`, `lease: 59m left`), boundary between two calls, TTL countdown) passes/fails by where second hand was. Have reviewer run it three to five times before believing; reviewer reports "N/N but flaky" → flake = defect, same severity as failing test: next session inherits untrustable gate.

   **Gate that skips not a gate.** Test needing external binary (database daemon, CLI tool) prints `SKIP <name>: not found`, exits 0, so run that matters (host that DOES have dependency) can be green asserting nothing. Give suite strict switch (`AGENTBOARD_REQUIRE_SURREAL=1`-style) turning skip into failure, run strict wherever dependency installed, add both rows to repo check table: default run and strict run. Verify strict switch honestly: hide binary (`PATH=/nonexistent
   HOME=/nonexistent`), confirm tests FAIL not skip.

   **Test-only escape hatch must not exist in release binary.** Env var disabling write lock so mutation check proves lock load-bearing = good tool and live-board hazard: if `AGENTBOARD_UNSAFE_NO_WRITE_LOCK=1` works in release, one shell export corrupts board. Require switch `cfg(test)`/debug-only, or refuse in release; check built artifact (`strings`/behaviour), not only source.
6. **One fresh review task per review round**, in pane separate from coder. Seat follows quota: Claude reviewer with `reviewer-mcp.json` while Claude has room, else second opencode pane with own actor. Dispatched same way. Verdict goes on board as its report. Scope = diff: logic, edge cases, whether coder's pasted evidence proves claim. Not re-run of coder's gates (step 5), not board-wide test pass. Findings become next coder task: fix task = **subtask of review that produced it**: file with `--parent T<review>` (epic inherited from parent), so `children T<review>` answers "what did this review cost" instead of link only in prose. Same for anything else task spawns: child records cause, not just epic. Filed flat by mistake? `agentboard parent T<n> T<m>` repairs link after, accepted on any status, `done` included, logs `edit: parent -> T<m>`, `children` then answers from both ends. Wrong link never needs refile.

   **Reviewer that edits tree is not reviewing.** `git status` after "clean" verdict is part of gate. Reviewer edit = unowned change (no row, no author in ledger, absent from report) and verdict then grades own diff. Adopt via fix row assigned to coder (say in row manager adopted reviewer's edit, so ledger has author) or revert; never close round on it. Test-shaped version: fix making failing test pass by changing mock proves nothing about production class, since test never touched class. Demand contract asserted against real implementation same round.

   **Instruction files reach pane only at agent start.** `AGENTS.md`, `CLAUDE.md`, MCP config: editing one does nothing for running pane, and `/clear` keeps old rules loaded. Restart agent at cycle boundary when new rule must bite, say rule live from next round, not this one. `needs_fix` verdict → next coder task, fix = **subtask of review that produced it** (as above): `--parent T<review>`, `children T<review>` shows cost; repair flat filing with `agentboard parent T<n> T<m>` (any status, `done` included, logs `edit: parent -> T<m>`, `children` answers both ends). Wrong link never needs refile.

   **Scope bleed: amend next row before dispatching it.** Worker given row A often implements API row B was filed for, same methods, landed early. Don't dispatch B as written or second round rewrites first; re-read B's description against diff, restate as what's still missing (tests over new surface, refusal edges, callers), naming row that landed it. Leftover often just removing old entry point: `rg -n '<old symbol>' server/ ldclient/ tests/` after round tells whether B still has work; symbol whose only callers are old tests = real row ("delete it and its press-era tests"), not duplicate. Same correction when review round asks for work another queued row owns.

   **Closing fix belongs to reviewer, not author.** Coder fills fix task, stops at `review` (no `fixed` status) with report and note. Only reviewer moves fix task on: `needs_fix` with remaining findings as note, or `done` with evidence; only done fix task lets it close review task. State in both rows: coder closing own fix self-certifies, reviewer reopens loop already shut.

   **Adjudicate findings before fix dispatched.** Reviewer reads code, not standing rule, so can file finding contradicting it ("the fix task never reaches done"); coder faithfully implements wrong thing. While fix row still `ready`, amend: `REJECTED` block naming each rejected finding with rule it breaks, then valid ones as actual work, explicit "do not apply R<n>" line. `task_edit` refuses only while lease live, so `needs_fix` review and `ready` fix both amendable; once coder claims, round spent. Read child fix row for findings too: reviewer may set review row `needs_fix` with empty report, all findings in fix task.

   **`needs_fix` verdict with no child row = yours to file.** Reviewers sometimes report finding in note and stop. Read note, file fix as child of that review with note's findings quoted and proofs to paste, dispatch; re-prompting reviewer for paperwork costs round, finding already known. Say in child's description manager filed on reviewer's behalf.

   **Empty report + `needs_fix` = read pane, then re-check `children` once.** Verdict often exists only in pane transcript: row shows `needs_fix` with nothing after colon while reviewer still writing child row. `herdr pane read <pane> --lines 200 | rg -v '^\s*$' | tail -40` gives numbered findings, and `children T<review>` a minute later usually shows fix task it filed. File child yourself only when still absent after second look. Reviewer line like "tracked separately" or "belongs in test infrastructure, tracked separately" with no id is NOT filed row: fold that finding into fix row you dispatch, named as manager addition, or it never lands.

   **Reviewer reproduces measurement, not reads it.** Round turns on numbers (contention rates, lost updates, latency) → reviewer re-runs experiment with own probe and transport, pastes own output. Different rates expected (reviewer's HTTP probe duplicated in 1 of 10 runs where coder's ws probe saw 2 of 10); direction and conclusion must match. Verdict quoting author's table back ≠ evidence.

   **Parity transcript sleeping between writes hides ordering hole.** Two backends sorting `seen DESC` and `seen DESC, name` print same transcript only because test registers rows 50 ms apart; real board sorts same-millisecond rows differently. Require same-timestamp case and one stable tiebreak in BOTH backends; "identical transcript" unproven until that case exists.

   **Two workers same vendor still need two actors.** Two opencode panes (quota fallback) → each pane own `AGENTBOARD_ACTOR` (`herdr tab create
   --env`, `herdr pane split --env`, each pane started `--standalone`), else every ledger event reads `opencode`, review can't name writer. Mechanics: `opencode` skill, "Two panes, two board actors".

   Ask for **revert proof** on behaviour fixes: author reverts own fix, shows named test fails, restores. Test passing on broken code = decoration; review round only place this caught.

   Revert-proof necessary not sufficient. Harness itself can break two ways, both look green:

   - **Assertion accepts success.** Helper written as `try: return "ACCEPTED: " +
     call(...) except: return str(e)` then handed to a substring check passes whether the
     call succeeded or was refused. Assert the *shape* (an error was raised, exit code is
     non-zero), never a string that both paths can produce.
   - **Assertion measures wrong thing.** Hang test that only times caller
     still passes when the kill is deleted — the stub survives, the caller returns fast.
     Assert the observable: the pid is gone, the event count, the exit code, the row state.
     The same trap catches a reviewer's own tooling: a "no stray process left" check run as
     `pgrep -x surreal` printed zero every time because the daemon is `surreal-db`, which is
     an all-clear the matcher could only ever produce. Verify such a check by creating the
     state it looks for, or match on something unambiguous like the data directory in `ps`.

   So demand two mutations per guard: revert fix (fails?) and force failure path to succeed (still fails?). Survives either = untested, not passing.

   **Survived mutation = finding about test, not guard.** Named test doesn't
   reach guard (earlier gate in same function already returns, or fixture leaves guarded state empty), so round's proof void until test exercises path that gets there. Remedy = NEW test for that distinct path (late timeout after re-grant, handover completing while revoke open), not same test with louder assertion. Require author to say in report which guard was redundant for originally named test and which new test now fails without it; mutation round reporting "all five guards verified" with no survived list hasn't looked.

   Verdict belongs to reviewer that produced it. Review role changes hands (different vendor/model takes seat) → accumulated commits get one fresh pass from reviewer of record before called merge-ready; earlier approve = another reviewer's opinion, not citable gate. Say on board which reviewer ran, so later session can tell apart.

   **Required** fix from reviewer of record jumps queue. User's next feature doesn't outrank merge gate: land fix, re-verify, commit, resume queue. Everything built after would sit on unapproved base; finding cheapest to fix while reviewer evidence fresh. Optional findings queue like other work.
7. **Freeze tree while reviewer reads it.** No next coder round until verdict lands: diff moving under reviewer yields findings against code gone, whole verdict wasted. Behaviour change after review makes old verdict stale; commission second round, don't cite it. Two rows editing same file (docs task and feature task both touching `README.md`) serialized for same reason: spec says which row owns shared file, dispatch other after.
8. **Commit landed round without being asked; never push.** Standing default: implement or design task reaching `done` → reviewed files committed locally, one objective per commit, sha in reply. User asking "commit it" each round = doing manager's bookkeeping. Manager commits; workers told not to. Push stays human's call, always; same for history rewrite. `git add` explicit paths, never `-A` (sweeps workers' scratch files into commit).

   **Split by objective, leave worker's in-flight edits unstaged.** Two finished rounds in one tree = two commits, explicit paths each; count table (`AGENTS.md`) moves in commit that changed tests it describes. Third change worker holds (manifest bump for dependency later epic adds) stays out of both: stage only reviewed paths, say in reply what left behind and why.

   Other half: before commit, read `git status` for files same step touches and is about to leave behind. Seam function new CLI path calls (`board_path()` in `store/mod.rs`) is part of step "CLI adapter" though step title names two other files; staging by title leaves commit that doesn't build alone. Stage every reviewed file step needed; noticed immediately → amend rather than "oops" commit.

   **No install while manifest carries unreviewed changes.** `cargo install --path
   . --locked` builds whole dependency graph, so mid-migration install compiles crate under test, pins binary from unreviewed tree. Panes keep old binary until install; behaviour/wording change not installed → state plainly, don't report as live.

   **Install = manager's step after clean verdict, never line in worker's goal.** Worker sandbox may refuse it as shared-resource change (correctly), and reviewer handed install-plus-live-board steps exercises them against LIVE board, leaving probe rows, pinning binary from unreviewed tree. Keep every gate touching `~/.cargo/bin` or live board out of worker goals; slipped in → strike from dispatched goal with `task_edit`, say on row goal supersedes that paragraph. Unreviewed install landed and board misbehaves → roll back without disturbing diff: `git worktree add --detach /tmp/ab-head <good-sha>` then `cargo install --path /tmp/ab-head --locked <backend flags>`, prove with `add --help | rg <new flag>` (absent = old surface back), `git worktree remove --force`.

   **Re-read git state at report time.** User has shell in same repo, can push/commit while you work, so earlier `git status` not evidence now: before saying "N commits ahead, unpushed" or "HEAD is X", run `git status -sb` and `git ls-remote origin master` and compare, or say nothing about push state. Stale ahead-count = false claim about tree.

## Board surface the manager uses

| Command | What it is for |
|---|---|
| `brief` | counts, open tasks by urgency, recent completions: standup; groups open epics with path and `[N blocking, N needs_fix]` |
| `ls [--status S] [--assignee A]` | compact open list |
| `epic add "<title>" [--path P] [--description D] [--goal G]` | container: own table and id space, never leaseable |
| `epic ls [--status open\|done\|dropped]` / `epic get E<n> [--full]` / `epic status E<n> <s> [note]` | read and close epics |
| `children E<n>` / `children T<n>` | epic's tasks, or one task's subtasks: two different questions |
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

First = `claim` saying lease live; remedy in text works (clears lease), so crashed holder costs one command, not 30-minute TTL.

Both **errors, not replies**: explicit-id `claim` refusal and `dispatch` refusal exit 1; over MCP same text arrives as tool error. So error from `claim` means *someone holds this row* (read text, apply remedy), not *board is broken*; no blind retry, don't report tool down. Id-less probe = opposite, stays success: `claim` with no id on empty board exits 0, prints `no ready tasks`. Answer, not failure.

**Manager does not `claim`.** Manager claim takes lease, coder's claim then refused. Use `status T<n> working "<note>"` to show intent, let worker claim.

## MCP equivalents

Driving board over MCP instead of shell: `board_brief`, `task_list`,
`task_add`, `task_get` (`full: true`), `task_claim`, `task_update` (status / report /
note / commit), `task_edit`, `task_ask` + `task_wait`, `task_children`, `epic_add`,
`epic_list`, `epic_get`, `epic_update`, `epic_edit`, `session_list`.
`task_wait` has same selectors as CLI `wait`; no exit codes, so puts `matched:` or `timeout:` on first line; branch on that. No MCP `parent` verb: re-pointing subtask over MCP = CLI call.

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

- **`herdr agent prompt <name> ... --wait` belongs in background job.** Foreground hits tool deadline, gets yielded, looks like failed delivery while prompt already submitted. Run `background=true, notify=true`; returns timeout but pane shows agent working → don't resend (injects second turn into running task). Confirm on board (`agentboard ls`, `agentboard get T<n>`).
- **`agentboard answer` writes board only, doesn't type into pane.** Resuming blocked row leaves worker at prompt not knowing answer landed; row shows `working`, nothing moves. After `answer`, push pane immediately: `herdr agent prompt <name> "Manager answered on T<n> (#<seq>): <the ruling>. Finish the row: task_update with report + status review."`, only into idle pane. Same for `note`: note = ledger, not delivery.
- **Names are the address.** Distinct names let two teams run at once (`coder2`,
  `reviewer2`); `agent_name_taken` can't clear without closing that pane.
- **Workers write to board through MCP tools, not by shelling binary.** `agentboard dispatch` used to send "Claim T8, then task_get id=8 full=true", coder answered with bash step running CLI claim: same effect, extra subprocess, unstructured output, wrong shape for agent. Name tool in row or prompt (`task_claim`, `task_get`, `task_update`), keep CLI for verbs MCP lacks and verification commands goal lists, check live pane for what worker actually did, don't trust report.
- **Reviewer approval ≠ verification: re-read artifact yourself.** Coder can leave doc/diagram describing opposite of rule it states, reviewer can pass it: two rounds approved `docs/review-loop.md` while flowchart put reviewer-owned `done` node in Coder lane. Read artifact, check against rule, not only command outputs in report. Manager findings go on new fix task (`parent=<the review task>`, assignee coder) so reviewer still closes it; never edit tree yourself.
- **Row is the prompt: write no prompt files.** Long text into `task_add`/`task_edit` as JSON over MCP, never scratch `.txt` a wrapper `cat`s. Scratch copy = second prompt that drifts; user reads it as breaking board-is-only-channel rule.
- **herdr is pane host; tmux legacy.** Build and test herdr paths first. Verb speaking only tmux = bug for this team; `parse_herdr_agents` dropping `pane_id` is what made it one (`tail`/`send` were tmux-only while `sessions`, `dispatch` and `prune` already used herdr). Leave tmux fallback untouched rather than improve; spend time on tmux only when nothing else to do.
- **Renew pane's conversation between cycles, never inside one.** Cycle = task -> review -> fix -> done; next task starts with cleared context: `herdr pane send-text <pane> "/clear"` then `herdr pane send-keys <pane> enter` (pane must sit at prompt; open stats/usage overlay eats text, so send `esc` first, read pane to confirm). Do after task closed, before next dispatch, coder and reviewer separately. Rule written into agent's instruction file not in pane until agent restarts; `/clear` alone keeps old rules. Fix task spawned by review = SAME cycle, not renewal point: clearing before fix drops context with findings, fix row only thread back. Renew when review and every fix under it `done`. Never clear pane `working` or mid-review: row = only prompt, cleared pane loses thread.
- **Never close pane to free name or as cleanup.** Not while `working` or `blocked`; only when user asks.
- **`blocked` usually dialog or TUI at prompt**, not hung agent: read pane before touching. Opencode reviewer may sit on "allow web search?"; local diff review doesn't need it.
- **Don't run `herdr-agent-open` from manager shell.** Splits *current* pane, needs `HERDR_PANE_ID`; kills old agent first, then dies at split, leaving project with no agent. Create tab and panes explicitly by id.
- Verify `cwd` in `agent list` before dispatching. Pane in wrong directory = silent work in wrong repo.

## Keeping the tool current

- `agentboard` runs from two binaries: `~/.cargo/bin/agentboard` (installed, what shell and agent MCP configs execute) and `./target/release/agentboard` (checkout). After changing tool, `cargo install --path . --locked`, else verbs just built missing from binary you dispatch with.
- **Install with backend feature host actually runs.** Once store feature-gated, bare install builds default backend, so surreal host installing without `--no-default-features --features surreal` silently gets sqlite binary: new file, empty board, agents reporting `no ready tasks` while real store untouched. Install from reviewed tree with host's feature set, prove which installed by BEHAVIOUR, not mtime/size: hand it path other build would refuse (sqlite file to directory-backed build, directory to sqlite build), require refusal. `strings` match or fresh timestamp only says something built; refusal says which backend runs. Say in reply which build live.
- `tests/mcp_client.py` drives `target/release/agentboard`. Rebuild before running, else green suite tests week-old code.
- **Storage engine swap gated on multi-process safety, not schema.** Board opened by CLI, one MCP server process per agent, suites spawning N workers; embedded engine with per-process cache can't carry that, in-process mutex or flock doesn't fix stale view. Prove cross-process atomicity and read coherence before any port written; design whose core mechanism still unverified = rejection. Probe recipe + interpretation table: `references/worker-claim-probes.md`, "Multi-process safety probe". Engine facts, measured numbers, surviving local-server-plus-lock design: `storage-engine-swap` skill, its surrealdb-2x reference.
- **Schema change needs switch, not migration.** Board has no migrate path: old file refused before any DDL with one line naming `mv <db> <db>.old`, file left byte-identical. History expendable, so switch = wipe, but order matters, agents' MCP servers are *old* binary processes:

  1. stop every agent, close panes (nothing may hold board), including own MCP board server, also old-binary process: after switch, drive board by CLI until session restarts, else every MCP board call fails on unknown schema; 2. `cargo install`
  new binary, 3. move `board.db` (plus `-wal`/`-shm`) into archive dir, 4. run one read (`agentboard brief`) so *new* binary creates schema, 5. re-register teams by name, 6. refile still-open rows; every id died in wipe, rebuild queue from specs you hold.

  Skip step 1 = failure: old-binary server answering request recreates old schema in new file, new binary then refuses own board.
- **Reset without schema change = same wipe, minus binary swap.** Resolve path first with `agentboard path`: `$AGENTBOARD_DB` (often set in MCP config env block) overrides state dir, so deleting default path can miss live board. Confirm board idle before wiping (every task `done`/`dropped`, no live lease), then `cp -p board.db board.db.bak-<ts>`, `mv board.db board.db.old-<ts>`
  (plus `-wal`/`-shm`), run one read (`agentboard brief`) so fresh schema lands, verify empty counts via python `sqlite3` (no `sqlite3` CLI on this box), only then delete `.old` duplicate: `.bak` = restore path, outlives session. Say in reply where backup sits and when to drop it.
- **Reset empties `sessions` too.** Panes keep old MCP server process (store reopens path per call, no stale handle), but registry rows gone: re-register names before dispatching, else `dispatch` answers `agent_not_found` on healthy team.
- Agent panes spawn MCP server from PATH binary at session start, keep that build until pane restarts. Schema change = dangerous case; new flag merely invisible.
- Parsing has own suite, `tests/cli.sh` (exit codes, `--flag=value`, unknown-flag errors, hyphen values); delivery has `tests/dispatch.sh` (refusal, `--print`, `--force`, assigned-vs-`--to`, fallback reason, deadline kill). Run `mcp_client.py`, `cli.sh` and `dispatch.sh` before believing CLI or dispatch change.
- **Test delivery with stub binaries on PATH, never live pane.** Two-line `herdr`/`tmux` script that exits, fails, or sleeps proves fallback and deadline in milliseconds; real dispatch types into someone's working agent, can't replay.
- **"Every call now goes through X" is a grep you owe.** Unifying mechanism doesn't retire hand-rolled copies: after shared runner landed, `sessions` still had own spawn-and-join with fixed deadline, so README line "the env var bounds every bridge call" and commit message both false; path took 8 s under stub that should die at 300 ms. Before claiming coverage, search old pattern (`search_files` / `rg`: `Command::output`, old constant, second `std::process::Command` in same module), make claim a test's acceptance criterion, not sentence in commit.
- **Count in prose = claim to re-derive, not copy.** Header comment "18 tools", README test total, worker report line `cli.sh 102/102` all drift within one round; quoting one back to user = wrong fact. Derive from source at verification (`rg -c '#\[tool\(' src/server.rs`,
  `cargo test 2>&1 | rg '^test result'`), fix prose where it disagrees with command output.
  **Behavioural claim in user-facing prose verified same way: run the verb.** "`tail`
  is tmux-only", "the bridge covers every call", "this env var bounds it" survive rounds
  because nobody executes them. Before rewriting such line in README or PROTOCOL, run
  command against live pane/process, quote output. Stale doc claims not docs problem; same defect as stale test count, one level up.
- **Verb that lands retires skill saying it's missing.** Feature not finished while playbook still teaches workaround: after round, grep skill library for old claim (`rg -n 'no edit verb|there is no|not yet|workaround'` in skills dir), patch sentence in place, check reference files too. Stale `pitfalls.md` bullet sends next session down path tool closed.
- **Playbook also shipped in repo → edit repo copy, install from it.** Repo copy canonical, installed one = snapshot; editing installed copy creates drift next `install.sh` silently reverts: edit in tree, `skills/install.sh --force`, then `diff -r` both ways. Comparison against something *outside* tree, so result time-shaped: worker reporting `diff -r` empty means empty when it looked, not at commit time. Re-run diff yourself at verification, hold commit until match; copy task whose source keeps moving lands stale, and stale half is what future session reads. Same for any
  write reaching only `~/.hermes/skills/`: skill-management or curator pass that
  can't touch repo leaves tree next `install.sh` reads behind, so mirror
  edit into `skills/<name>/` and reinstall same sitting, then diff both ways.

  **Repo copy BEHIND → `install.sh --force` destroys knowledge.** `wc -l` both
  copies first: repo file shorter → missing sections installed-only, reinstall deletes them. Then don't install: patch same block into repo
  `skills/<name>/SKILL.md` and installed `SKILL.md`, prove by extracting
  changed paragraph from each and comparing (`python3` slice, or
  `diff <(sed -n '/<anchor>/,/^$/p' a) <(sed -n '/<anchor>/,/^$/p' b)`): whole-file
  `diff -r` never comes back clean while unrelated drift exists, so paragraph is
  unit to verify. Commit repo copy, file remaining drift as row instead of
  silently loading one side.

  **Compressing either copy: compressor's backup keyed by parent-dir name + stem**
  (`backups/team-manager/SKILL.original.md`), so second tree of same skill hits
  `Backup file already exists ... Aborting to prevent data loss`, run reports failure
  after spending model call. Rename stale backup
  (`mv … SKILL.original.pre-$(date +%Y%m%dT%H%M%S).md`), rerun. Expect 1-2% on skills
  already dense: `Validation passed` ≠ saving, so quote `wc -c` before/after
  and check `rg -c '```'` unchanged.

## What "done" means

1. Task `done` on board, report names files and pastes real output.
2. Delegate (reviewer or idle worker) ran named gates in that tree, real output quoted in reply. Manager ran no suite, build or install unless user asked.
3. Review task agrees, or required fixes landed and second round says so.
4. Reviewed files committed locally (step 8), one objective per commit, sha in reply. Push and history rewrites stay human's call, always.
5. Report actually supports status. `done` whose own report lists acceptance flows unproven = failed gate wearing green tag: read report against goal; disagree → move row back to `ready` with note naming what's unproven and what blocks it (missing hardware, service that can't start, no persistent client). Never leave it `done` because worker said so, never quietly widen goal to what was proven. Blocked verification row = honest scoping; green row contradicting its evidence poisons every later "E<n> is done" claim built on it.

## Close the round: hygiene tasks

Feature round not finished when review says approve. File hygiene work before calling done, else it piles up until noise hides real warning:

- **clippy**: `cargo clean -p <crate>` then `cargo clippy --all-targets`; cached runs replay warnings from earlier builds, lie about count (8, 6 and 2 all "same" tree at different cache states).
- **fmt**: `cargo fmt --check`; never-formatted repo reports thousands of diff lines.
- Two separate tasks, clippy first, fmt second; **fmt never runs while reviewer reading tree**: mechanical 1000-line diff buries change under review. Commit separately, same reason: clippy fix reviewable, formatting sweep not.
- Add gate to whatever repo calls its checks (`README`, `justfile`, CI) so next round inherits it instead of repeating task.