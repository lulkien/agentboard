---
name: team-manager
description: "Use when managing coder and reviewer agents on the board."
version: 2.2.0
author: Hermes Agent
license: MIT
platforms: [linux, macos]
metadata:
  hermes:
    tags: [agentboard, herdr, manager, delegation, review, orchestration]
    related_skills: [agentboard-manager, claude-code, opencode]
---
# Managing a team from the board

Any agent can be manager or worker. Manager holds two ledgers, writes no code:

- **agentboard** owns work: row = prompt, status = truth, report = claim, events = history.
- **herdr** owns processes: tabs, panes, live agents, current activity.

Manager does: board writes, dispatch, waits, read panes, read reports, talk to human. Every edit belongs to worker, incl. one-line docs and `.gitignore` entries — belongs to repo → gets row. Test runs + installs = delegate work too: hand to reviewer or idle worker, never run from manager shell unless user asks.

Rest lives: `references/herdr-mechanics.md` for pane depth (context check, layout, start/restart, dialogs, seat swap, teardown, traps); `scripts/` for `herdr-status`, `herdr-agent-open`, `herdr-team-open`; `agentboard-manager` for long-form reasoning. `herdr --skill` ships authority — flag disagrees → binary wins.

## 1. Check ground before touching anything

```bash
test "${HERDR_ENV:-}" = 1 || echo NOT_INSIDE_HERDR
herdr status                         # server up, protocol compatible
herdr tab list; herdr pane list      # topology now, not from an earlier turn
herdr agent list                     # name, kind, status, cwd
agentboard brief; agentboard ls; agentboard sessions
```

Re-read topology same turn you act: stale pane id → `pane close` answers `pane_not_found`. Not inside herdr → say so, stop.

## 2. Topology: one manager tab, one team tab

```bash
herdr tab create --cwd <repo> --label team --env AGENTBOARD_ACTOR=<coder-actor>
herdr pane split --pane <root> --direction right --cwd <repo> \
  --env AGENTBOARD_ACTOR=<reviewer-actor> --no-focus
herdr agent start coder --kind claude --pane <p1> --timeout 180000 \
  -- --strict-mcp-config --mcp-config=$HOME/.claude/coding-mcp.json --model sonnet
herdr agent start reviewer --kind claude --pane <p2> --timeout 180000 \
  -- --strict-mcp-config --mcp-config=$HOME/.claude/reviewer-mcp.json --model sonnet
```

`~/.local/bin/herdr-team-open <repo>` does whole move: closes live coder/reviewer, creates tab, splits, answers folder trust, fixes name-from-kind. `DRY=1` prints plan; `REVIEWER_KIND=claude|opencode` picks seat; `CODER_MODEL`/`REVIEWER_MODEL` pick models.

**Check the installed helper before relying on it:** `rg -n 'AGENTBOARD_ACTOR|standalone'
~/.local/bin/herdr-team-open`. Copy lacking both predates actor rules below → claude + opencode team built with it starts reviewer without `--standalone`, no pane actor → both panes log as one name, review round can't say who wrote verdict. Then build layout by hand with block above, then register.

Rules for readable event log:

- One actor per pane. Hardcoded actor in MCP config beats pane env — strip it, let pane export own, else coder + reviewer log as one name. Claude coder configs here pin `AGENTBOARD_ACTOR=claude1`, so coder pane `--env` inert and `agentboard ls` shows `claude1`: read actor off claimed row, not session name.
- opencode panes need `--standalone` + `--env AGENTBOARD_ACTOR=…` on tab/split; pane on shared service inherits service env, both report same actor.
- Kind = seat, not vendor: `claude`, `opencode`, `codex` all work. Say which reviewed.
- Verify cwd in `herdr agent list` before dispatch; wrong repo = silent wrong tree.
- `agent start` needs pane at interactive shell prompt; never creates layout.

### Shared memory for both seats

Both seats read/write one Hindsight bank over its MCP endpoint → reviewer recalls coder decisions instead of re-deriving. Setup, config shapes, session-id handshake: `hindsight-local-hermes` → `references/mcp-clients.md`.

- One bank per repo/team, `mission` naming domain; team facts out of user's personal bank.
- Add same "recall at task start, retain decisions before `review`" block to `~/.claude/CLAUDE.md` and `~/.config/opencode/AGENTS.md` → rule survives seat swap + quota-driven agent change.
- MCP config loads at process start: restart pane to pick up, only at cycle boundary. Pane mid-row keeps old toolset — configure, restart after round closes.
- Prove it: idle pane calls `recall` on seeded fact, pastes line. Config on disk, listed server name, or agent's own "available" answer ≠ evidence bank answers.

## 3. Board setup

```bash
agentboard path                      # board location; check backend build
agentboard --actor <manager> register coder --agent claude
agentboard --actor <manager> register reviewer --agent claude
agentboard sessions                  # both "up"
```

Register **agent name** herdr reports, never pane id. Stale row shadows dispatch: `agentboard unregister <name>` or `agentboard sessions prune`.

`agentboard path` names backend — read it whenever board looks wrong: **directory** = surreal build, **`.db` file** = sqlite. Switching backend/board doesn't reach running panes — their MCP servers are old-binary processes, keep writing old board until panes renewed. Stop team, switch, renew, register again.

Two surreal traps, each costs a session when re-learned. `agentboard path` only prints settings — daemon starts on first real call, so prove liveness with `agentboard brief` + listening port, not `path`. Daemon needs `surreal` binary on `PATH` or at `~/.surrealdb/surreal` (symlink into `~/.local/bin`), else every call returns one-line install hint that looks like broken board.

Install lines, env vars, version gate, lazy serving, stop command, switch-a-host checklist: `agentboard-manager` → `references/backend-install.md`.

## 4. Epic, task, subtask

```bash
agentboard --actor <manager> epic add "<outcome>" --path /abs/repo --goal "<when done>"
agentboard --actor <manager> add "<title>" --epic E4 --assignee <coder> \
  --description "<repo, files, situation, constraints>" --goal "<executable checks>"
```

- `description` = context; `goal` = how + how judged done. Goal can't fail = not goal: name commands + text they must print.
- One row per unit. Fix belongs to review that found it: `--parent T7` (epic inherited). `agentboard children T7` then shows review cost.
- Two id spaces: `E<n>` → epic verbs + `--epic`, `T<n>` → task verbs + `--parent`.
- Amend, don't refile: `agentboard edit T7 --goal …` while `ready`; live lease refuses (worker reading text).
- Long text via MCP (`task_add`/`task_edit` JSON), never scratch `.txt` for wrapper to `cat`. Second prompt copy drifts.

### Filing a plan document into epics

Plan file on disk (`.hermes/plans/*.md`) → epics + task rows. Order avoiding rework:

1. **Pick the authoritative file first.** Sibling drafts on one topic happen; `diff` before filing. One stating epic order + dispatch intent wins; never mix numbering between drafts, file from one, name it in reply.
2. **One epic per mergeable outcome**, not per heading. `--path` = repo root, `--description` = plan file path, what epic changes, acceptance line, plan's evidence-level list; `--goal` = done state + inter-epic ordering ("needs E1+E2"). Epic = what later session reads before dispatch; ordering only in plan file dies with it.
3. **Priority bands one per epic** (10, 20, 30…, tasks +1 within) → dispatch order = plan order, no re-reading plan.
4. **Verify every path and line ref before it lands in a row:** `test -e` each file, `rg -n '<symbol>'` each `file:line`. Plan refs go stale as tree moves; row pointing at deleted function sends worker hunting.
5. **A plan section that says "must be run, not inferred" becomes its own epic** with one verification task, blocked by rest, naming commands + pasted output.
6. **Prove the filing:** `epic_list` (done/total per epic) + `task_list` — ids continue from board's last, so fresh board may start at `T2`; never assume `T1` exists, never re-file because id looked odd.

## 5. Dispatch and wait

```bash
agentboard --actor <manager> dispatch T7 --to coder      # refuses non-ready row
agentboard --actor <manager> wait T7 --status review,needs_fix,blocking --timeout 300
```

- Dispatch sends id, not spec: pane claims via MCP tools, reads row itself.
- One row per pane. Second dispatch into working pane = stray mid-task turn: queue next row as `ready`.
- Arm background watcher with notify before ending turn. No polling loops.
- **Default `--timeout 300`, not 1800.** Hour-long watch too long for this user. Exit 3 = stall check: read pane once (`herdr agent wait <pane> --until idle --timeout 1`), push only idle pane, re-arm same wait. Loop = manager's job — never wrap poller around `agentboard wait`.
- **`agentboard answer` writes the board only — it does not type into the pane.** After answering blocked row, prompt idle pane same breath (`herdr agent prompt
  <name> "Manager answered on T<n>: <ruling>. Finish it: task_update report + status
  review."`), else worker sits at prompt while row reads `working`, nothing moves. `note` = ledger, not delivery.
- `wait --status` matches **current state**: row already in status returns at once. Watch status not yet reached, or `--kind`. Blocked delegate shows as `ask` — `wait --open --kind ask`.
- **Never put a row that already reached a watched status in the set.** Sibling already `done` (or review row already in `needs_fix`) makes whole wait return first poll → wake reports history, nothing learned. Watch only in-flight rows, only statuses none holds yet.
- **Multi-row watches wake instantly on a stale status.** `wait T10 T11 T2 --status
  ...needs_fix` matches moment any watched row already holds one of those, prints last event prefixed `last:` — history, not new verdict. Before arming, read `agentboard ls`, drop every status any watched row holds; open review round usually means `--status done,blocking` only. On wake, re-read `agentboard ls`/`get` before acting; never re-dispatch on notification alone.
- Dispatch not done until watcher armed.

## 6. Review loop

**The coder tests its own behaviour; the reviewer reviews logic.** Worker that made change runs every gate goal names — build, suites, throwaway-board proof for behaviour changes — pastes real output into `report` before setting `review`; row arriving with gate unrun → reopened, not accepted. **Every executable step belongs to the coder**, not just tests: `cargo install`, target build, `devtool_build` / `devtool_deploy_target`, flashing board, running binary there, driving proving flow. Reviewer = team lead: reads diff + pasted evidence, reviews logic + edge cases, files feedback — no build, deploy, install, flash, hardware driving, no re-running coder's suites as second gate. May run own small probe or one targeted command to chase suspected edge case; that's the job, not a full pass. Nothing duplicated for ceremony — second full run mostly teaches author to trust someone else's transcript.

```
task  ->  review  ->  fix  ->  re-check  ->  done  ->  renew
```

1. Worker claims, works, tests itself, pastes gate lines into `report`, sets `review`.
2. Manager reads report, files review task under same epic, dispatches reviewer.
3. Reviewer reads diff for logic, edge cases, evidence: clean → review task `done`; findings → files fix task itself (`parent=<review task>`), sets own row `needs_fix`.
4. Coder fixes, answers each finding in `report`, sets fix task `review`. Coder never sets fix task `done`, never closes review task.
5. Reviewer re-checks own findings. All closed → fix task `done` with evidence, then review task `done`. Still open → `note` naming it, fix task `needs_fix`, loop.
6. Manager checks report vs goal, closes task + epic when every row `done` or `dropped` — any gate re-run goes to idle worker, not manager shell, never back to coder's reviewer.

Status ownership: coder `ready`/`needs_fix` → `working` → `review`; reviewer `review` → `done`|`needs_fix` on both kinds; manager `ready` (re-arm crashed row), `dropped`, `done` on implement task, epic status.

Reviewer approval = claim, not proof. Read artifact yourself — reviewers approved docs whose diagrams contradicted documented rule. Manager findings → new fix task, never direct edit.

**After the verdict, check the implement row actually moved.** Reviewers close own row + child fix rows, routinely leave task under review in `review`. Read `agentboard ls`: review row `done` + implement row not → close it yourself (`task_update` status `done`, note naming reviewer, round, what it ran, commit sha) instead of prompting for paperwork already in pane. Same when implement row's diff landed in own round.

**A red test you did not cause is an attribution question, not a fix row.** Run same case from older build (stale `oe-workdir/build` or pre-change binary), find commit with `git log -S '<a log string from that path>'`. Failure predates diff on table → no fix row, no test hacking by worker: name row whose change makes it pass (epic replacing semantics), write that expectation into THAT row's goal → redness has owner, current round stays focused.

**Micro-findings ride the next row.** Stale comment, doc line contradicting change, one dead symbol — reviewer reports but doesn't file → fold into later row's description ("same round: fix <file:line>, it still says …") instead of full dispatch-review cycle for one line. State in reply which row adopted it.

**A known red suite is carried, not re-argued.** Once failing test attributed to later epic, write in goal of every round running suite ("expect exactly this one failure, it is <row>'s semantics"), say it in review row too. Else each reviewer burns first minutes re-attributing same red, or reports it as current diff regression → round bounces.

**`git status` is part of a clean verdict.** Reviewer editing tree = unowned change — no row, no ledger author, missing from report — grading own diff. Revert it, or adopt via fix row assigned to coder, naming in row that manager adopted reviewer's edit so ledger has author. Test-shaped version: failing test made to pass by changing mock proves nothing about production class — test never touched it. Require contract asserted against real implementation same round — say which change (production or mock) is being proven.

**The verdict goes in the row's `report` field.** Status move to `done`/`needs_fix` with empty report hides verdict in reviewer scrollback → row unauditable, manager reconstructs from pane. Say it in row you write ("verdict in the report field, not only in your pane"); ignored anyway → paste verdict yourself from pane via `task_update` (report + note naming process defect) instead of another prompting round, repeat complaint in next review row until it sticks.

**A row that lands another row's scope leaves that row a duplicate.** Workers often implement next task's API while doing theirs; don't dispatch deprived row as filed. Amend same sitting to what's actually left (tests, removing now-dead path, caller that must change), or drop it, say so in reply.

Demand mutation checks on guards: test must fail when guard removed; guard disableable by env var in release build ≠ guard. Freeze tree while reviewer reads; moving diff voids verdict.

## 7. Renew pane between cycles

```bash
herdr pane send-text coder "/clear"
herdr pane send-keys coder enter
herdr pane read coder --lines 8          # expect empty prompt
```

- Only pane at prompt. Clearing `working` pane loses row thread.
- `/clear` renews conversation only. Does NOT reload `--mcp-config`, `--append-system-prompt-file` or any other start flag — MCP server added to config needs group kill + `herdr agent start` in same pane (see §2, shared memory). Tell it was missed: worker reports "no <tool> tools in this session" on row whose config on disk lists them.
- Fix task from review = SAME cycle: not renewal point.
- Open stats/recap/approval overlay swallows text: `herdr agent send-keys coder esc` first, read pane, then `/clear`.
- Coder + reviewer renew separately, own cycles.

## 8. Ask, interrupt, restart

```bash
herdr pane send-text coder '/btw one line: which step now, ETA?'   # side question
herdr pane send-keys coder enter
herdr agent send-keys coder esc                                    # hard interrupt
```

- `/btw` answers without cancelling turn — use for "what doing" while row `working`. Plain text queues as *next* user turn.
- `agent prompt … --wait` goes in background job: foreground can die at exit 124 while prompt already submitted; re-send double-prompts.
- `blocked` = dialog, not hang: read pane, answer with key names (`send-keys coder down enter`), re-check `agent list` for `idle`.
- Vendor quota = board-wide pause, not stuck worker. Pane showing `⚠ Usage limit reached · limit resets HH:MM` keeps turn, **resumes by itself** at that time: don't kill, don't re-prompt, don't re-dispatch row. Same limit hits every `claude --print` helper at once → silently failing helper script + stopped reviewer = one cause; read pane before debugging helper. Schedule next dispatch after reset.
- Swap seat mid-round on quota out: kill each TUI by process group, kill old kind's background service too, reuse panes with new kind, re-register sessions, then `agentboard status T<n> ready "<why>"` to free orphaned row, re-dispatch.
- Restart with other config/model: same sequence; confirm via `herdr pane process-info --pane <p>` → `foreground_processes[].argv`, not start reply.

## 9. Recovery, in payoff order

| Symptom | First move |
|---|---|
| `agent T7 working by X (lease 24m left)` | row held: `agentboard status T7 ready` clears, re-dispatch |
| `agent_prompt_stalled` | overlay/dialog up: read pane, `esc`, re-send |
| pane `working`, row `ready` | read pane; turn ended without board write |
| `agent_not_ready` / `blocked` at start | startup dialog: folder trust (`down enter`), read-outside-cwd (`enter`) |
| `agent_name_taken` | previous agent still runs: kill its group, then start |
| `⚠ Usage limit reached · resets HH:MM` | vendor pause: pane self-resumes then, helpers fail till then — wait, never kill |
| board empty after a backend change | panes still run old binary's MCP server: renew panes, writes went to old board |
| `agentboard` verbs fail: `Invalid revision N for type Value`, `no such task` for a row `ls` shows, `wait` dead | surreal daemon, not store: `cp -a <data dir> <data dir>.corrupt-<ts>`, `kill -TERM $(pgrep -f 'surreal-db start' | head -1)`, then one `agentboard brief` restarts it, reads return — recipe in `agentboard-manager` → `references/backend-install.md` |
| row claimed twice | guard broken: stop, fix before next row |
| stray background servers | `pgrep -af '<name>'` — process name may differ from binary |
| `pkill -f <pattern>` kills own shell | bracket it: `pkill -f '[s]urreal db start'` |
| helper script "fix did nothing" | two copies: `diff -q ~/.local/bin/<h> scripts/<h>`, then `bash -n` |

Close team only when asked, prove with fresh listing — never claim teardown without one. `herdr agent` has no close verb; sequence:

```bash
herdr pane process-info --pane <p>   # result.process_info.foreground_process_group_id
kill -TERM -<pgid>                   # negative pid = the whole group
herdr pane close <p>                 # one per pane
herdr tab list; herdr agent list     # proof: agent gone, tab gone
```

- Read pgid from `result.process_info.foreground_process_group_id` — pane's own `foreground_processes[]` can be empty for idle agent, so empty array proves nothing.
- After group kill, pane's login shell usually survives; `pane close` removes it — surviving `bash` pid ≠ failed kill.
- Tab vanishes with last pane: `tab close` then answers `tab_not_found` = done, not broken.
- Tabs with pane but no agent not part of team; name them, ask before closing — may hold human's own shell.

## 10. One cycle, start to finish

1. `herdr status`, `agentboard brief`, `herdr agent list` — orient.
2. Row written with executable goal, right `--epic`/`--parent`.
3. Dispatch, arm watcher before ending turn.
4. On wake: read report, check each pasted gate line vs goal, delegate any re-run to idle worker or reviewer.
5. Review task filed + dispatched; tree frozen while reviewer reads.
6. Fix tasks closed by reviewer — not coder, not manager.
7. Commit explicit paths only, on explicit ok; never push.
8. Renew both panes, then start next row.