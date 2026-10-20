---
name: team-manager
description: "Use when any agent manages a coder and a reviewer through agentboard and herdr: layout, dispatch, review loop, renew cycle."
version: 2.0.0
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

- **agentboard** owns work: row = prompt, status = truth, report = claim, events =
  history.
- **herdr** owns processes: tabs, panes, live agents, current activity.

Manager does: board writes, dispatch, waits, read panes, re-run gates, talk to human.
Every edit belongs to worker, incl. one-line docs and `.gitignore` entries — belongs to
repo → gets row.

Rest lives: `references/herdr-mechanics.md` for pane depth (context check, layout,
start/restart, dialogs, seat swap, teardown, traps); `scripts/` for `herdr-status`,
`herdr-agent-open`, `herdr-team-open`; `agentboard-manager` for long-form reasoning.
`herdr --skill` ships authority — flag disagrees → binary wins.

## 1. Check ground before touching anything

```bash
test "${HERDR_ENV:-}" = 1 || echo NOT_INSIDE_HERDR
herdr status                         # server up, protocol compatible
herdr tab list; herdr pane list      # topology now, not from an earlier turn
herdr agent list                     # name, kind, status, cwd
agentboard brief; agentboard ls; agentboard sessions
```

Re-read topology same turn you act: stale pane id → `pane close` answers
`pane_not_found`. Not inside herdr → say so, stop.

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

`~/.local/bin/herdr-team-open <repo>` does whole move: closes live coder/reviewer,
creates tab, splits, answers folder trust, fixes name-from-kind. `DRY=1` prints plan;
`REVIEWER_KIND=claude|opencode` picks seat; `CODER_MODEL`/`REVIEWER_MODEL` pick models.

Rules for readable event log:

- One actor per pane. Hardcoded actor in MCP config beats pane env — strip it, let pane
  export own, else coder + reviewer log as one name.
- opencode panes need `--standalone` + `--env AGENTBOARD_ACTOR=…` on tab/split; pane on
  shared service inherits service env, both report same actor.
- Kind = seat, not vendor: `claude`, `opencode`, `codex` all work. Say which reviewed.
- Verify cwd in `herdr agent list` before dispatch; wrong repo = silent wrong tree.
- `agent start` needs pane at interactive shell prompt; never creates layout.

## 3. Board setup

```bash
agentboard path                      # board location; check backend build
agentboard --actor <manager> register coder --agent claude
agentboard --actor <manager> register reviewer --agent claude
agentboard sessions                  # both "up"
```

Register **agent name** herdr reports, never pane id. Stale row shadows dispatch:
`agentboard unregister <name>` or `agentboard sessions prune`.

## 4. Epic, task, subtask

```bash
agentboard --actor <manager> epic add "<outcome>" --path /abs/repo --goal "<when done>"
agentboard --actor <manager> add "<title>" --epic E4 --assignee <coder> \
  --description "<repo, files, situation, constraints>" --goal "<executable checks>"
```

- `description` = context; `goal` = how + how judged done. Goal can't fail = not goal:
  name commands + text they must print.
- One row per unit. Fix belongs to review that found it: `--parent T7` (epic inherited).
  `agentboard children T7` then shows review cost.
- Two id spaces: `E<n>` → epic verbs + `--epic`, `T<n>` → task verbs + `--parent`.
- Amend, don't refile: `agentboard edit T7 --goal …` while `ready`; live lease refuses
  (worker reading text).
- Long text via MCP (`task_add`/`task_edit` JSON), never scratch `.txt` for wrapper to
  `cat`. Second prompt copy drifts.

## 5. Dispatch and wait

```bash
agentboard --actor <manager> dispatch T7 --to coder      # refuses non-ready row
agentboard --actor <manager> wait T7 --status review,needs_fix,blocking --timeout 1800
```

- Dispatch sends id, not spec: pane claims via MCP tools, reads row itself.
- One row per pane. Second dispatch into working pane = stray mid-task turn: queue next
  row as `ready`.
- Arm background watcher with notify before ending turn. No polling loops.
- `wait --status` matches **current state**: row already in status returns at once.
  Watch status not yet reached, or `--kind`. Blocked delegate shows as `ask` —
  `wait --open --kind ask`.
- Dispatch not done until watcher armed.

## 6. Review loop

**The coder tests its own behaviour; the reviewer reviews logic.** The worker that made
the change runs every gate its goal names — build, suites, and a throwaway-board proof
for behaviour changes — and pastes the real output into `report` before it sets `review`;
a row that arrives with a gate unrun is reopened, not accepted. **Every executable step
belongs to the coder**, not just tests: `cargo install`, a build for the target,
`devtool_build` / `devtool_deploy_target`, flashing a board, running the binary there and
driving the flow that proves it. The reviewer is a team lead: it reads the diff and the
pasted evidence, reviews the logic and the edge cases, and files feedback — it does not
build, deploy, install, flash or drive hardware, and it does not re-run the coder's suites
as a second gate. It may still run its own small probe or one targeted command to chase an
edge case it suspects; that is the job, and it is nothing like a full pass. Nothing is
duplicated for ceremony — a second full run mostly teaches the author to trust somebody
else's transcript.

```
task  ->  review  ->  fix  ->  re-check  ->  done  ->  renew
```

1. Worker claims, works, tests it itself, pastes the gate lines into `report`, sets `review`.
2. Manager reads report, files review task under same epic, dispatches reviewer.
3. Reviewer reads the diff for logic, edge cases and the evidence: clean → review task
   `done`; findings → files fix task itself (`parent=<review task>`), sets own row `needs_fix`.
4. Coder fixes, answers each finding in `report`, sets fix task `review`. Coder never
   sets fix task `done`, never closes review task.
5. Reviewer re-checks own findings. All closed → fix task `done` with evidence, then
   review task `done`. Still open → `note` naming it, fix task `needs_fix`, loop.
6. Manager checks the report against the goal, then closes task + epic when every row
   `done` or `dropped` — any re-run of a gate goes to an idle worker, not the manager's
   shell, and never back to the coder's reviewer.

Status ownership: coder `ready`/`needs_fix` → `working` → `review`; reviewer `review` →
`done`|`needs_fix` on both kinds; manager `ready` (re-arm crashed row), `dropped`, `done`
on implement task, epic status.

Reviewer approval = claim, not proof. Read artifact yourself — reviewers approved docs
whose diagrams contradicted documented rule. Manager findings → new fix task, never
direct edit.

Demand mutation checks on guards: test must fail when guard removed; guard disableable
by env var in release build ≠ guard. Freeze tree while reviewer reads; moving diff
voids verdict.

## 7. Renew pane between cycles

```bash
herdr pane send-text coder "/clear"
herdr pane send-keys coder enter
herdr pane read coder --lines 8          # expect empty prompt
```

- Only pane at prompt. Clearing `working` pane loses row thread.
- Fix task from review = SAME cycle: not renewal point.
- Open stats/recap/approval overlay swallows text: `herdr agent send-keys coder esc`
  first, read pane, then `/clear`.
- Coder + reviewer renew separately, own cycles.

## 8. Ask, interrupt, restart

```bash
herdr pane send-text coder '/btw one line: which step now, ETA?'   # side question
herdr pane send-keys coder enter
herdr agent send-keys coder esc                                    # hard interrupt
```

- `/btw` answers without cancelling turn — use for "what doing" while row `working`.
  Plain text queues as *next* user turn.
- `agent prompt … --wait` goes in background job: foreground can die at exit 124 while
  prompt already submitted; re-send double-prompts.
- `blocked` = dialog, not hang: read pane, answer with key names
  (`send-keys coder down enter`), re-check `agent list` for `idle`.
- Swap seat mid-round on quota out: kill each TUI by process group, kill old kind's
  background service too, reuse panes with new kind, re-register sessions, then
  `agentboard status T<n> ready "<why>"` to free orphaned row, re-dispatch.
- Restart with other config/model: same sequence; confirm via
  `herdr pane process-info --pane <p>` → `foreground_processes[].argv`, not start reply.

## 9. Recovery, in payoff order

| Symptom | First move |
|---|---|
| `agent T7 working by X (lease 24m left)` | row held: `agentboard status T7 ready` clears, re-dispatch |
| `agent_prompt_stalled` | overlay/dialog up: read pane, `esc`, re-send |
| pane `working`, row `ready` | read pane; turn ended without board write |
| `agent_not_ready` / `blocked` at start | startup dialog: folder trust (`down enter`), read-outside-cwd (`enter`) |
| `agent_name_taken` | previous agent still runs: kill its group, then start |
| row claimed twice | guard broken: stop, fix before next row |
| stray background servers | `pgrep -af '<name>'` — process name may differ from binary |
| `pkill -f <pattern>` kills own shell | bracket it: `pkill -f '[s]urreal db start'` |
| helper script "fix did nothing" | two copies: `diff -q ~/.local/bin/<h> scripts/<h>`, then `bash -n` |

Close team only when asked: kill each pane's foreground process group, close panes,
close tab, then `herdr tab list` / `herdr agent list` to prove — never claim teardown
without fresh listing.

## 10. One cycle, start to finish

1. `herdr status`, `agentboard brief`, `herdr agent list` — orient.
2. Row written with executable goal, right `--epic`/`--parent`.
3. Dispatch, arm watcher before ending turn.
4. On wake: read report, re-run named gates yourself.
5. Review task filed + dispatched; tree frozen while reviewer reads.
6. Fix tasks closed by reviewer — not coder, not manager.
7. Commit explicit paths only, on explicit ok; never push.
8. Renew both panes, then start next row.