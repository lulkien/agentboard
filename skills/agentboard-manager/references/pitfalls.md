# Pitfalls that cost a session

One rule per item, with the failure it prevents. Read this before the first dispatch
on a new machine.

## Board

- **The manager must not `claim`.** `claim` takes the lease, so the worker's claim is
  then refused and it works on a task nobody can see it holding. Use
  `status T<n> working "<note>"` for intent.
- **A refusal is an error, not a reply — and that is the point.** An explicit-id `claim`
  refusal and a `dispatch` refusal both exit 1, and over MCP the same text comes back as a
  tool error. Read it as *this row is held* plus the remedy in the text (`agentboard status
  T<n> ready`), not as *the board is broken*: retrying a refusal blind, or reporting the
  tool as down, sends the session chasing a bug that is a lease. The id-less probe is the
  exception — `claim` with no id on an empty board exits 0 with `no ready tasks`.
- **Paste the things a task says to keep; never retype them from memory.** A retyped list
  silently drops or misspells entries (`*.db-wal` becoming `*.db-wu`), and the worker then
  either preserves the wrong thing or, obeying the typo, deletes the real rule. Put the
  file's current contents in the row and require a diff in which every original line
  survives.
- **Read rows with `full`/`--full` when *you* are the reader.** `get` and `ls` fold
  newlines and clip at a cap with a marker; `claim` is the one path that is verbatim
  and uncapped. Reading a clipped spec and judging the work from it is how a manager
  reviews something other than what was asked.
- **Never poll `board.db` from a script.** A python loop over `updated_at` is a second
  implementation of board semantics that silently drifts from the schema. `wait` is
  the supported long-poll: selectors, `--status`, `--kind`, exit 0 match / 3 timeout.
- **`updated_at` moves without an event** (heartbeat). A watcher keyed on `updated_at`
  wakes on a heartbeat and tells you nothing; key the watcher on the thing you care
  about (`--status`, `--kind`).
- **A parent link can be corrected after the fact.** `agentboard parent T<n> <T<m>|none`
  re-points a subtask on any status, `done` included, and logs `edit: parent -> T<m>`; the
  epic is re-checked (a cross-epic parent is refused) and `children` then answers from
  both ends. So a row filed without its cause is a one-command repair, not a refile — but
  file it right the first time with `--parent T<review>` when the row exists to answer a
  review. Check `agentboard --help` for the spelling this build has, rather than assuming.
- **`--force` steals.** It resets a working task to `ready`; two agents on one file is
  the outcome. Use it only after a deliberate decision, and prefer the refusal text
  (`agentboard status T<n> ready`) when the holder is simply dead.
- **Never inline the spec in the dispatch line.** The reviewer reads the row; a prompt
  that carries the text gives the coder and the reviewer different specs.
- **A report is a claim, not evidence.** Re-run the named gates in that tree and put
  the output in your reply. A worker that never claimed can still report success.
- **A behaviour change invalidates the verdict.** After a fix round, commission a new
  review rather than citing the old approve.

## Delivery: calling an external command on the dispatcher's behalf

- **Every external call needs a deadline.** `Command::output()` waits forever, so one
  wedged `herdr` (or `tmux`) hangs the manager while the worker looks busy. Spawn, poll
  `try_wait`, kill at the deadline, and treat the timeout as a delivery failure that falls
  through to the next path.
- **Bound the stderr drain too.** After the kill, wait a short fixed time
  (`recv_timeout`) for the reader thread and walk away: a grandchild holding the pipe
  (`sh -c 'sleep 30'`) keeps it open long after its parent died, and a plain `wait_with_output`
  re-hangs on exactly the case the deadline was meant to fix.
- **Killing the child is not killing the work.** Without a process group a grandchild
  survives as an orphan. Either accept it explicitly or `setpgid` + `kill(-pgid)`; do not
  assume the deadline cleaned up.
- **Name the path and the reason, in the output and the event.** "exit 0" cannot tell you
  whether herdr delivered or the fallback did, and a fallback whose cause is dropped is
  unfixable later. `w1 via tmux (herdr: timed out after 200ms)` in both places.
- **Deadline both branches.** Fixing the primary path leaves the fallback undeadlined
  when it shares a helper with another verb (`sessions::send` backs `send` and the tmux
  fallback), so the same hang survives one layer down. Say so when you close the task.
- **Make the deadline testable.** A test-only env override
  (`AGENTBOARD_DELIVER_TIMEOUT_MS`) proves the kill path in milliseconds; without it the
  test either sleeps the real timeout or never covers it.

## herdr

- **`herdr-agent-open` from the manager shell destroys the old agent and starts
  nothing.** It splits the *current* pane, which needs `HERDR_PANE_ID`; the kill
  happens first, the split dies after. Create the tab and panes by id instead.
- **Register sessions by herdr agent name, never pane id.** The liveness check
  compares names, so a pane-id row registers fine and then reads `gone` forever, and
  `dispatch` cannot address it.
- **The task assignee is an MCP actor** (`claude1`, `opencode`), not a session name.
  Assignee-to-session resolution usually misses; `--to <session>` is what names the
  pane.
- **A stale session row outranks `--to`.** Resolution takes the registered assignee
  before the explicit target and never asks whether that session is alive, so a row left
  by a pane you closed hours ago captures the dispatch: `--to reviewer2` reports
  `agent_not_found: reviewer`. `agentboard sessions` is where you see it (`gone`, with the
  task it held); register the live agent under that name or delete the row before
  dispatching. Check this the moment a dispatch fails on a name you did not type.
- **`blocked` is ambiguous.** It can be a permission dialog, or `agent start` exiting
  non-zero while the agent sits happily at its prompt. Read the pane and check
  `agent list` before killing anything — a restart costs the session and can drop a
  claimed task.
- **`agent_name_taken` outlives a killed process group.** Close the pane; the name
  stays owned otherwise.
- **Two teams need distinct names** (`coder2`/`reviewer2`). The lifecycle rule binds a
  name, so a second project under new names satisfies it without stopping the first.
- **A pane is closed only on an explicit stop**, never as cleanup, never while
  `working` or `blocked`.
- **Check `cwd` in `agent list` before dispatching** — a pane rooted in the wrong repo
  is silent until the diff comes back in the wrong project.

## Gates

- **A guard must fail when the thing it guards is broken.** Two shapes pass on broken
  code: an assertion that accepts the success path (a helper returning `"ACCEPTED: " +
  result` when the call succeeded, then checked for a substring that both paths produce),
  and one that measures the wrong observable (timing the caller instead of asserting the
  stub's pid is gone). Prove every guard twice — revert the fix, and force the failure
  path to succeed. A guard that survives either one is decoration, and only a reviewer
  who mutates the code will notice.
- **Never assert on wall-clock text in a rendered row.** `get`/`get --full` prints relative
  ages and a live lease (`created: 0s`, `lease: 59m left`), so two renders compared for
  equality — the natural way to prove "the refused edit changed nothing" — differ whenever
  a second or minute boundary falls between the calls, and a lease assertion after a
  TTL change is 59 or 60 depending on the same luck. Strip those lines before comparing,
  assert on the event timeline or stored values, or assert a range. A suite that is green
  on the run you happened to watch proves nothing; repeat it three to five times.
- **Every suite drives a built binary.** `tests/mcp_client.py`, `tests/cli.sh` and
  `tests/dispatch.sh` all execute `target/release/agentboard`, so a stale build makes a
  green run test code you did not change. Rebuild first, or the green is meaningless.
- **Test delivery with stubs, never a live pane.** Two-line `herdr`/`tmux` scripts on PATH
  that exit 0, exit non-zero, or sleep are how the fallback and deadline get proven; a real
  dispatch types into someone's working agent and cannot be replayed.
- **Build the fixture before arming a background mover.** A mover that writes into a DB you
  then `rm -rf` in the next call races itself into a false "feature broken" verdict — create
  the scratch DB first, arm the mover second, and never clean up the directory until the
  probe has printed its result.
- **Baseline claims need a baseline.** "Clippy still 8 warnings" is only true against
  HEAD: `git worktree add /tmp/head HEAD`, run the same command there, compare counts
  *and* sites. A **lower** count is legitimate (warnings get fixed incidentally) — but check
  the vanished sites are really gone and no `#[allow]` was added; a silenced warning is not
  a fixed one.
- **Watch for output-shape coupling.** Exit codes decided by sniffing a marker string
  (`text.starts_with("timeout:")`) work until someone adds a leading line; prefer a
  typed outcome when the signature is already changing.
- **A diff is a rendering; the file is the evidence.** A hunk that looks like it
  introduced a garbage line (`+2 -0` at the end of a README) can be the printer merging
  or wrapping lines. Read the file itself (`sed -n`, `cat -A`) before filing a defect
  against the worker: a false finding costs a round and teaches the worker that your
  reports are not worth reading closely.
- **A comparison against something outside the tree expires.** `diff -r` of a vendored
  copy, an installed skill dir, or a generated file is true only at the instant it ran —
  the external side keeps moving while the worker works. Re-run it when you verify, and
  never accept "diff empty" as a property of the commit.
