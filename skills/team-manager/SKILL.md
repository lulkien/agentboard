---
name: team-manager
description: "Use when any agent manages a coder and a reviewer through agentboard and herdr: file epics and tasks, dispatch, run the review loop, renew panes."
version: 1.0.0
author: Hermes Agent
license: MIT
platforms: [linux, macos]
metadata:
  hermes:
    tags: [agentboard, herdr, manager, delegation, review, orchestration]
    related_skills: [agentboard-manager, herdr, claude-code, opencode]
---

# Managing a team from the board

Any agent can be the manager, and any agent can be a worker. The manager holds two
ledgers and writes no code:

- **agentboard** owns the work: the row is the prompt, the status is the truth, the
  report is the claim, the event log is the history.
- **herdr** owns the processes: tabs, panes, live agents, what they are doing now.

The manager's hands are board writes, dispatch, waits, reading panes, re-running
gates, and talking to the human. Every edit is made by a worker, including
one-line docs and `.gitignore` entries — if it belongs to the repo, it gets a row.

`agentboard-manager` is the long-form playbook with the reasoning. This skill is the
operating procedure: commands, order, and the traps that cost time.

## 1. Topology: one manager tab, one team tab

```bash
herdr tab list                     # find the manager pane; never split it
herdr tab create --cwd <repo> --label team --env AGENTBOARD_ACTOR=<coder-actor>
herdr pane split --pane <root> --direction right --cwd <repo> \
  --env AGENTBOARD_ACTOR=<reviewer-actor> --no-focus
herdr agent start coder --kind claude --pane <p1> --timeout 180000 \
  -- --strict-mcp-config --mcp-config=$HOME/.claude/coding-mcp.json --model sonnet
herdr agent start reviewer --kind claude --pane <p2> --timeout 180000 \
  -- --strict-mcp-config --mcp-config=$HOME/.claude/reviewer-mcp.json --model sonnet
```

Rules that decide whether the log is readable later:

- One actor per pane. The actor comes from the MCP config's `environment` block, and a
  hardcoded value there beats the pane env — remove it and let each pane export its own,
  or the event log cannot tell coder from reviewer.
- Pane kind is a seat, not a vendor: `claude`, `opencode`, `codex` all work. Pick whichever
  has quota, and say in the close-out which one reviewed.
- Verify cwd in `herdr agent list` before dispatching. A pane rooted in the wrong repo is
  the one failure that silently produces work in the wrong tree.

```bash
herdr agent list                     # name, kind, status, cwd
herdr pane read <pane> --lines 20    # what it is actually doing
herdr agent prompt <name> "<text>"   # one turn, submitted atomically
```

## 2. Board setup

```bash
agentboard path                      # where the board lives; check the backend build
agentboard --actor <manager> register coder --agent claude
agentboard --actor <manager> register reviewer --agent claude
agentboard sessions                  # both must say "up"
```

Register the **agent name** as herdr reports it, never a pane id. A stale row shadows a
dispatch: `agentboard unregister <name>` or `agentboard sessions prune` removes it.

## 3. Epic, task, subtask

```bash
agentboard --actor <manager> epic add "<outcome>" --path /abs/repo --goal "<when it is done>"
agentboard --actor <manager> add "<title>" --epic E4 --assignee <coder> \
  --description "<repo, files, situation, constraints>" --goal "<executable checks>"
```

- `description` is context; `goal` is how to do it plus how it is judged finished. A goal
  that cannot fail is not a goal: name the commands and the text they must print.
- One row per unit of work. A fix belongs to the review that found it:
  `--parent T7` (the epic is inherited). `agentboard children T7` then answers what that
  review cost.
- Two id spaces: `E<n>` goes to epic verbs and `--epic`, `T<n>` to task verbs and
  `--parent`. The wrong one is refused by design.
- Amend instead of refiling: `agentboard edit T7 --goal ...` while the row is `ready`;
  a live lease refuses the edit (the worker is reading that text).
- Long text goes in over MCP (`task_add`/`task_edit` with JSON), never into a scratch
  `.txt` that a wrapper `cat`s. A scratch copy is a second prompt that drifts.

## 4. Dispatch and wait

```bash
agentboard --actor <manager> dispatch T7 --to coder      # refuses a non-ready row
agentboard --actor <manager> wait T7 --status review,needs_fix,blocking --timeout 1800
```

- Dispatch sends the row's id, not the row's text: the pane claims with its MCP tools and
  reads `description`/`goal` itself. Never retype the spec into the prompt.
- One row per pane at a time. A second dispatch into a working pane becomes a stray turn
  in the middle of the work: queue the next row as `ready`.
- Arm the watcher in the background with notify before ending the turn. No polling loops.
- `wait --status` matches **current state**, so watching a row already in that status
  returns at once. Watch a status the row is not in yet, or `--kind`.
- A dispatch is not finished until its watcher is armed; a blocked delegate arrives as
  `ask`, which is worth watching with `wait --open --kind ask`.

## 5. The review loop

```
task  ->  review  ->  fix  ->  re-check  ->  done  ->  renew
```

1. Worker claims, works, finishes with `report` and `status=review`.
2. Manager reads the report, files a review task under the same epic, dispatches the
   reviewer.
3. Reviewer reads the diff and the report: clean → review task `done`; findings → files
   the fix task itself (`parent=<review task>`) and sets its own row `needs_fix`.
4. Coder fixes, answers each finding in `report`, sets the fix task `review`. The coder
   never sets a fix task `done`, never closes a review task.
5. Reviewer re-checks its own findings. All closed → fix task `done` with evidence, then
   review task `done`. Still open → `note` naming it, fix task `needs_fix`, loop.
6. Manager re-runs the gates in that tree before believing anything, then closes the task
   and the epic when every row is `done` or `dropped`.

Status ownership is fixed: coder `ready`/`needs_fix` → `working` → `review`; reviewer
`review` → `done`|`needs_fix` on both kinds; manager `ready` (re-arm a crashed row),
`dropped`, `done` on an implement task, and epic status.

A reviewer approval is a claim, not proof. Read the artifact yourself: two rounds of this
team's reviewers approved docs and diagrams that stated the opposite of the rule they
documented. Manager findings go on a new fix task, never into a direct edit.

Ask for mutation checks on anything that guards: the test must fail when the guard is
removed, and a guard that can be disabled by an env var in a release build is not a guard.
Freeze the tree while a reviewer reads it; a diff that moves invalidates the verdict.

## 6. Renew the pane between cycles

After `done`, before the next task, clear the pane's conversation:

```bash
herdr pane send-text coder "/clear"
herdr pane send-keys coder enter
herdr pane read coder --lines 8          # expect an empty prompt
```

- Only a pane sitting at its prompt. Clearing a `working` pane loses the thread of the
  row it is on.
- An open stats/usage overlay swallows the text: `herdr agent send-keys coder esc` first.
- Coder and reviewer are renewed separately, each on its own cycle.

## 7. Recovery, in the order it pays off

| Symptom | First move |
|---|---|
| `agent T7 working by X (lease 24m left)` | the row is held: `agentboard status T7 ready` clears it, then re-dispatch |
| pane says `working` but the row is `ready` | read the pane; a dialog may be open (`esc`, then answer) |
| dispatch stalls (`agent_prompt_stalled`) | an overlay or permission dialog is up: read, `esc`, re-send |
| row claimed twice | the guard is broken; stop and fix that before the next row |
| stray servers after a test run | `pgrep -af '<name>'` — the process name may differ from the binary name |
| `pkill -f <pattern>` kills your own shell | bracket the pattern: `pkill -f '[s]urreal db start'` |

Close the team only when the user asks: kill the foreground process group per pane, then
close the panes and the tab, and re-list to prove it.

## 8. Manager checklist for one cycle

1. `agentboard brief`, `agentboard ls`, `herdr agent list` — orient before touching anything.
2. Row written with an executable goal and the right `--epic`/`--parent`.
3. Dispatch, then arm the watcher before ending the turn.
4. On wake: read the report, re-run the named gates yourself.
5. Review task filed and dispatched; tree frozen while the reviewer reads.
6. Fix tasks closed by the reviewer, not by the coder or the manager.
7. Commit explicit paths only, on an explicit ok; never push.
8. Renew both panes, then start the next row.
