# Herdr mechanics

Herdr = terminal workspace manager for coding agents. Socket API over workspaces, tabs, panes, recognized agents. This file = mechanics: context checks, layout, start/restart agents, dialogs, teardown, traps. Verbs: SKILL.md. Wire format: `PROTOCOL.md`.

`herdr --skill` prints authoritative text shipped with installed binary. File and binary disagree on flag → binary wins.

## Verify context first

```bash
test "${HERDR_ENV:-}" = 1 || echo NOT_INSIDE_HERDR
agentboard manager status    # board sessions joined with live agents, read-only
agentboard manager pane list # tabs/panes now
herdr status                 # server running, protocol compatible
herdr pane layout --pane "$HERDR_PANE_ID"
herdr agent list             # name, agent kind, agent_status, pane_id
```

`HERDR_ENV` not 1 → say not inside Herdr, stop. `herdr --skill` prints authoritative skill text; installed binary = authority for flags.

## Prompt stalls on an open overlay

`agent_prompt_stalled` ("no observed working or blocked state within 5000 ms; current status is idle") usually = pane had overlay open (usage/stats, recap, permission dialog), prompt never reached input line. Read pane, send `escape` to close, re-send prompt, confirm board task flips to `working` before trusting delegation.

## Prompting agents from the shell

`herdr agent prompt <name> '...'` is shell command: apostrophe in text ends quote early, rest fails with `syntax error near unexpected token`. Don't fix by staging prompt in file and passing `"$(cat file)"` — second prompt copy is exactly what board exists to prevent. Work goes out via `agentboard dispatch`, which reads row. Hand-typed line stays one line, no apostrophes, or waits for row.

## Topology: manager tab, team tab

Manager in own tab. Coding agents share NEW tab with exactly two panes (coder left, reviewer right). Never split manager's tab, never put workers in manager's tab.

`agentboard manager up --repo <repo> [--coder K] [--reviewer K]` does whole move in one verb: refuses while a `coder`/`reviewer` is live, creates fresh tab rooted in repo, splits into two panes, starts coder (default claude) and reviewer (default opencode), answers folder-trust dialog, forces the names, registers both by name. `manager down` is the reverse. This file covers only what those verbs do not.

Create worker panes from own pane only when user asks for something other than team tab. Don't invent workspaces.

```bash
agentboard manager pane split <pane> --right --cwd <repo>    # new pane next to <pane>
agentboard manager pane split <pane> --down --cwd <repo>
```

`manager pane split --help` names the arguments. `agent start` needs pane at interactive shell prompt — never creates layout.

## Start the agents

One actor per pane, so task ledger tells them apart:

```bash
herdr agent start coder --kind claude --pane wB:p5 --timeout 120000 -- \
  --strict-mcp-config --mcp-config=$HOME/.claude/coding-mcp.json \
  --append-system-prompt-file=$HOME/.claude/caveman-full.md --model sonnet

herdr agent start reviewer --kind opencode --pane wB:p6 --timeout 120000
```

opencode reads own MCP servers from `~/.config/opencode/opencode.json`, so no `--mcp-config` args. Board actor NOT in that file — with two opencode panes actor must come from pane env, so set at layout time and start each pane `--standalone`:

```bash
herdr tab create --cwd <repo> --label team --env AGENTBOARD_ACTOR=opencode-coder
herdr pane split --pane <root> --direction right --cwd <repo> --no-focus --env AGENTBOARD_ACTOR=opencode-reviewer
herdr agent start coder    --kind opencode --pane <root>   --timeout 180000 -- --standalone
herdr agent start reviewer --kind opencode --pane <split>  --timeout 180000 -- --standalone
```

`--env` exists on `tab create` AND `pane split`, lands on pane's shell, agent inherits. `--standalone` matters: pane attached to opencode's shared background service runs under service's env, not pane's → both panes report same actor. Claude reviewer instead = `--kind claude` with `--mcp-config=$HOME/.claude/reviewer-mcp.json`; use only when opencode out of quota, say so in close-out.

`--kind` accepts claude, opencode, codex, gemini, others. Agent args after `--`.

`agent start` can answer `agent_not_ready` while TUI draws startup/plugin dialog; agent shows `blocked`, any prompt call fails. Read pane (`herdr pane read <pane> --lines 20`), answer with key names (`herdr agent send-keys <pane|name> enter`, `down`):

- Folder trust, first start in repo: "Is this a project you created or one you trust?" cursor on **No, exit**, so `down enter` picks "Yes, I trust this folder".
- Read outside working directories: fires per file outside pane cwd — why pane rooted in repo A can't review repo B. `enter` takes highlighted "keep allowing reads outside working directories" option.

Re-check `agent list` for `idle` before sending work. Startup dialog delayed readiness → herdr may name agent after kind: `herdr agent rename <pane_id> <name>` targets pane id. `agent_name_taken` = previous agent in that pane still running.

## Renew a pane's conversation between work cycles

Cycle: task -> review -> fix -> done, then renew: next task starts in cleared context so pane doesn't carry last row's state. `agentboard manager renew [--names A,B]` sends `/clear`, waits for `idle`, and prints the prompt marker it found; it refuses a working or blocked seat and a screen with no marker. By hand only when `renew` refused and you must see why: `agentboard manager agent read <name>` shows the screen, `manager agent keys <name> esc` closes an overlay, then run `renew` again.

Rules to avoid damage:

- Renew only pane at its prompt. `working` pane mid-task, or one with review in flight, loses thread; renew after row closed.
- Open overlay (usage/stats with `d to day`, recap, permission dialog) swallows text: send `escape` first, read pane, then `/clear`.
- Coder and reviewer renewed separately, own cycles, not together.
- Claude Code only. Same probe before assuming key exists in another agent's TUI: read pane, try text, read again, confirm prompt empty.

## Drive work

```bash
herdr agent prompt coder 'Claim task T37 ..., run its goal, task_update status review. Reply one line.' --wait --timeout 900000
herdr agent read coder                 # or: herdr pane read <pane_id> --lines 20
herdr agent wait coder --timeout 120000
```

`--wait` returns when agent settles; `agent_status` is idle, working, blocked, done or unknown. `done`/`idle` = "ready for input", not proof of completion: take completion from task ledger + `herdr agent prompt ... --wait` output together.

**Run `agent prompt ... --wait` as a background job, never in the foreground.** Call blocks until agent settles, can outlast terminal tool's timeout: foreground call dies exit 124 while prompt already submitted, so exit code says nothing about delivery. Re-sending double-prompts already-working agent — breaks one-row-per-pane rule. Background with notify, take truth from `herdr agent read <name>` and board row status.

`blocked` = approval or question dialog. Read pane, answer with key names:

```bash
herdr agent send-keys coder down down enter
```

## Ask a working agent for status without interrupting it

`herdr agent send-keys` takes key names only (`invalid_key` for text), so question goes via `pane send-text`, Enter via `send-keys`:

```bash
herdr pane send-text wB:p5 '/btw Status in one line: which step is running, ETA?'
herdr pane send-keys wB:p5 enter
sleep 25; herdr pane read wB:p5 --lines 14
```

Claude Code's `/btw` answers as side question without cancelling turn — right way to handle "ask agent what it's doing, taking too long" while row `working`. Plain text in input box queues as *next* user turn; agent may answer with current task context or drop it — use only to augment work. `send-keys <name> esc` = hard interrupt: ends turn, pane returns to prompt, board row keeps lease until `agentboard status T<n> ready "<why>"` clears it.

## One project per agent session

Work switches repo → close running agent, open fresh session rooted in that repo. Reusing pane with other project's cwd makes every read a cross-directory permission dialog ("Read outside the working directories") and leaves agent with wrong project context.

Whole team on a new repo: `agentboard manager down`, then `agentboard manager up --repo <repo>`. One agent only (manual — no verb replaces a single-seat reopen):

1. Find agent's pane + foreground process group (`herdr pane process-info --pane <id>`), `kill -TERM -<pgid>`, wait for shell prompt, then `agentboard manager pane close <id>`. Leftover pane = dead screen space + one more place agent gets started by mistake.
2. `agentboard manager pane split <existing-pane> --down --cwd <repo>` for new pane. Name the pane to split explicitly; never rely on the manager's own current pane.
3. `agentboard manager agent start <name> --kind <kind> --pane <new> -- <role's MCP config args>`.
4. Re-check name with `manager agent list` — herdr may name it after kind when startup dialog delayed readiness; `manager agent rename` fixes it.
5. `agentboard register <name> --agent <kind>` with that name.

## Restart an agent with a different config, model or env

Running agent keeps MCP config and model it started with; swapping either needs restart. Model change = same sequence, different `--model` (`sonnet` routine, `opus` design or hard implementation); expensive models chosen per phase, not per team member:

1. `herdr agent send-keys <name> ctrl+c` — may not quit TUI; send-keys rejects text (`invalid_key` for `/exit`): key names only (esc, enter, down, ctrl+c).
2. Get foreground process group: `herdr pane process-info --pane <pane_id>` → `.result.process_info.foreground_process_group_id`.
3. `kill -TERM -<pgid>`; pane falls back to shell prompt.
4. `herdr agent start <name> --kind <kind> --pane <same-pane> ...` with new config args.

6. Confirm new args took: `herdr pane process-info --pane <pane>` prints `foreground_processes[].argv` — check `--model` and `--mcp-config`, don't trust start reply.

## Swap the team's agent kind mid-task

Quota runs out mid-round: user says "kill opencode, use claude". Panes survive; seat changes. Order matters — board and panes each hold half the state.

1. Read board first: working row's id, whether reviewer row mid-verdict.
2. Kill each TUI by process group (`process-info` → `kill -TERM -<pgid>`), then kill old kind's background service too — opencode's `opencode ... serve --service` outlives panes, later start would inherit dead service's env. Match pattern with bracket (`pgrep -af '[o]pencode.exe'`) or `pkill -f` matches issuing shell and SIGTERMs own command.
3. Confirm every pane back at interactive shell prompt (`herdr pane read <pane>
   --lines 3`), then reuse panes: `herdr agent start coder --kind claude --pane <p1>
   ... --mcp-config=$HOME/.claude/coding-mcp.json`, reviewer pane with `reviewer-mcp.json`. Reuse keeps layout and cwd; fresh tab is for new repo.
4. Row old seat held now orphaned, still `working` with live lease: `agentboard status T<n> ready "<why>"` clears lease so new seat can claim. Leaving it → row waits out lease while fresh agent asks for work.
5. Re-register sessions with new kind — `agentboard register coder --agent claude`, same for `reviewer` — else `sessions` reports `up` for kind that no longer exists and dispatch resolution picks stale row.
6. Re-dispatch orphaned row, re-arm watcher. Old agent's partial work still in tree; say in row (or reply) whether new seat starts over or continues.

Verdict belongs to reviewer that produced it: review role changed vendor mid-round → new reviewer reads same row but approval = second opinion, not inherited gate. Note which reviewer signed.

## Close the team down

Work finished → user wants panes gone, not idle. `agentboard manager down` (add `--force` only on the user's say-so) closes both panes and unregisters the sessions. Then verify, don't assume: `agentboard manager pane list` and `manager tab list` must show only manager tab, `manager agent list` no named agents, `manager status` no registered rows. Closing tab's last pane can leave empty tab (or herdr opens fresh one, rooted at parent dir) — close that pane too, else workspace unclean.

Only a single hand-built seat needs the manual kill: `herdr pane process-info --pane <p>` → `result.process_info.foreground_process_group_id`, `kill -TERM -<pgid>` (negative pid = whole group), then `agentboard manager pane close <p>`. Pane's own `foreground_processes[]` can be empty for an idle agent, so empty array proves nothing; surviving login shell after the group kill is normal.

## Pitfalls

- Agent names must match `[a-z][a-z0-9_-]{0,31}`, unique among live agents; name follows pane occupant, frees when agent exits.
- `herdr agent list` shows `name: null` for panes with no recognized agent; target by `pane_id`.
- `herdr pane run <pane> '<cmd>'` prints nothing to caller — output appears in pane. To verify env or one-off command, run then `herdr pane read <pane> --lines 4`, else empty result misread as "variable unset".
- `pane read` takes pane id positionally (`herdr pane read wB:p6 --lines 20`); `--pane` is for `process-info`/`layout`, not `read`.
- Herdr pane ≠ tmux session. Anything checking liveness with `tmux has-session` reports agents gone; match name **and** agent kind against `herdr agent list` instead, with bounded subprocess.
- Probe/run agents with throwaway state DB or directory, answer every dialog you cause — blocked pane silently stops pipeline.
- Parse herdr JSON with `python3 -c`, never `python3 - <<'PY'`: heredoc replaces stdin, so `herdr ... | python3 - <<'PY'` pipeline feeds script itself, command output never arrives — parse prints nothing while command looks fine.
- `agentboard manager status` joins board sessions with live agents read-only in one call. Run before changing topology instead of chaining five commands and misreading stale pane id.
- **Re-read topology in the same turn you act on it.** Pane list from earlier turn goes stale once user or dialog changes it, so `herdr pane close <pane>` can answer `pane_not_found` and "cleaned up" report describes changed state. Close, re-list, state what's actually there; never retry close blind, never claim teardown without fresh `herdr tab list` / `herdr agent list` output.
- **The installed `agentboard` is what agents run.** A verb missing from `~/.cargo/bin/agentboard` is a stale install, not a missing feature: rebuild-and-install the binary, then retry.