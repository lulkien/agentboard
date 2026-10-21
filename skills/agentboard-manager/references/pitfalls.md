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
- **A long note goes over MCP; keep CLI notes to a sentence.** A multi-kilobyte double-quoted
  argument to `agentboard note` is fragile for the same reason dispatch wording is: the quoting
  survives but the call can be killed mid-write, so the row keeps nothing of a ruling you
  believed you had recorded — and a ruling that never landed is worse than none, because the
  worker is already acting on the version it was dispatched with. Put anything longer than a
  line through `task_update(note=…)` with a JSON string, and confirm it with `get --full`
  instead of trusting the ack line.
- **Never poll `board.db` from a script.** A python loop over `updated_at` is a second
  implementation of board semantics that silently drifts from the schema. `wait` is
  the supported long-poll: selectors, `--status`, `--kind`, exit 0 match / 3 timeout.
- **A wait that joins two sources needs a stated precedence, and the arm that already matched
  wins.** `wait --agent` races the board arm against a herdr subscription; propagating the
  agent-side error before the board's result is consulted throws away a match the operator
  already had, so a row that did move reports as a bare timeout (`agent.list timed out`, exit 1,
  empty stdout). Settle it in the code and in a test that fails on the old ordering: once the
  board arm has fired it decides the exit; when nothing matched, the agent error decides and
  says which side failed. Either way the degraded side is printed — on stderr, naming the agent
  — because a watch that silently lost half its inputs reads as a healthy one.
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
- **`not_done` is never a worker's message.** It means the reopen cap was hit and only
  the store sets it, so a row parked there to say "I need a human" is indistinguishable
  from a capped one and your triage (drop it, or file a replacement) reads the wrong story.
  A `not_done` with no needs_fix trail behind it is a defect in the report: say so, move
  the row back, and put the rule in the next brief — a worker asking for a decision uses
  `blocking` plus `ask`. **Then check whether the tool let it.** A status the protocol
  reserves for the store is settable whenever the status name is parsed like any other, so
  a worker succeeding at it is evidence of an enforcement gap, not only of a wrong row:
  file the refusal (one line naming the status to use instead, the store's own path left
  untouched) as its own row, because every script and reviewer downstream reads a
  caller-set value as the store's verdict.
- **A report is a claim, not evidence.** Re-run the named gates in that tree and put
  the output in your reply. A worker that never claimed can still report success.
- **A behaviour change invalidates the verdict.** After a fix round, commission a new
  review rather than citing the old approve.

## The review chain

- **A move to `review` with no diff since the last claim is refused, and the refusal is the
  remedy.** `T<n> cannot move to review: no diff recorded since it was last claimed` names
  `agentboard diff add T<n>`. The gate reads the events table (a `diff` event newer than the
  newest `claim` event), so a diff from an earlier round does not satisfy a later one, and a
  reopened row needs a fresh diff. A round that changed nothing records a body saying so and
  why; the board checks only that a row exists, so the reviewer, not the board, rejects a
  body that is neither a diff nor a stated reason.
- **Never file the review row by hand.** The board files it when a parentless row moves to
  `review` and reuses the newest non-dropped child of that row on every resubmission, so a
  hand-filed child is adopted as the review row and a second one is never added beside it.
  `children T<impl>` names the real one; the ack of the move prints `(review row T<n>)`.
  Dispatch the reviewer at that row, never at the implement row.
- **The cap is final and never resets.** Three moves into `needs_fix` per row; the fourth is
  refused (exit 1, or a tool error over MCP) and the row lands `not_done` with the caller's
  body kept in an event. An open parent lands `not_done` with it, one level only; a
  closed parent is left alone and the capped row gets a note naming it. So a capped fix
  row leaves its review row `not_done` and the implement row at `review`. `status T<n> ready`
  re-arms nothing, because the count stays and the next `needs_fix` move is refused again:
  drop the rows or file a replacement implement row. A reviewer who tries to work round the
  cap is outside its brief.
- **A submission is recorded from a file, never retyped.** A change too large to paste
  (thousands of lines) cannot be re-typed verbatim, so use `diff add T<n> --body-file
  <path>`: the file form of the CLI verb is the sanctioned path for a big body, even for a
  worker told to write the board over MCP — say in the row that you authorised it. Three
  traps make the identity proof lie: `git add -N` already puts new files into `git diff
  HEAD`, so appending `diff --no-index /dev/null <file>` records each new file twice (all
  bytes present, `git apply` fails on the duplicate); `diff get --full` renders a trailing
  newline the stored body does not have, so a one-byte digest difference is the render, not
  corruption — compare with the last byte dropped; and a deep `tree` path is not a
  substitute for content. Prove it by regenerating the submission from the tree and diffing
  that against the recorded body, never against the worker's own scratch file.
- **Epic closure is policy, not a guard.** `epic_update done` accepts an epic with `not_done`
  rows in it. Close an epic only when every row is `done` or `dropped`, drop or refile
  `not_done` rows first, and call it policy whenever you quote the rule; a document that
  says "the board blocks it" describes code that does not exist.
- **Clear both panes at each epic boundary.** The last row of an epic closed, the verdict in
  and the commit landed: renew coder and reviewer before the next epic's first row, not only
  between tasks. Never while a pane is `working`.
- **A diagram node is a claim about every path that reaches it.** Arrows are drawn generic
  ("a row moves into `needs_fix`") while the node text is written for the case the author had
  in mind, so a node reading "the implement row stays at review" is false the moment the
  capped row is the *review* row — whose parent is the implement row. Whatever the prose
  scopes per case, the node must scope too: put the case in the node ("a capped fix row
  leaves the implement row at review") and, when a fix round changes which cases exist,
  re-read every node whose arrow can now carry a different row. Skipping this costs exactly
  one doc defect per round, and only in the diagram — which is why the prose passes review
  while the picture lies.

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

- **Open and close teams with `manager up` / `manager down`, not by hand.** They
  name every pane explicitly and register by name; no step splits "the current
  pane", which a manager shell cannot rely on.
- **Retiring a helper in the repo does not retire it on the machine.** An older setup
  copied the wrappers onto `PATH` (`~/.local/bin/herdr-*.sh` plus the extension-less
  symlinks), so a skill sentence saying "no shell helper exists" is false on that host
  until those copies are gone — and the dangerous one keeps being runnable, which is
  exactly the state the rewrite was meant to end. Move them, do not delete them: `mkdir -p
  ~/.hermes/cache/scratch/retired-helpers-$(date +%Y%m%dT%H%M%S) && mv ~/.local/bin/herdr-*
  <that dir>/`, then prove the claim with `ls ~/.local/bin/herdr-*` returning nothing.
  Backing up rather than deleting keeps the user's own tooling recoverable while the doc
  and the host stop disagreeing.
- **Sessions are keyed by herdr agent name.** The liveness check compares names, so
  a row registered under anything else reads `gone` forever and `dispatch` cannot
  address it. `manager up` registers the right name; `manager status` shows `gone`.
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
- **`/clear` does not respawn the pane's MCP server.** The child process survives a
  clear, so after an install a cleared pane keeps the old build and the evidence reads as
  a board defect: `diff add` missing, a move to review refused with no gate in sight, no
  auto-filed review row, an unknown-verb error for something you just installed. Prove
  which build each process runs with `readlink /proc/<pid>/exe` across `pgrep -f
  agentboard`; a `(deleted)` suffix means it still holds the replaced file. Give the pane
  a new agent rather than a reload: `/exit`, Enter on the confirmation, wait for the shell
  prompt, then `herdr agent start <name> --kind <kind> --pane <id> --timeout 120000 --
  <its usual argv>`. Do it at a cycle boundary, never while the pane is working.
- **A seat can start and still never reach a prompt.** A `claude` launched into a
  directory it has not been trusted for comes up `agent_status: blocked` with
  `launch_pending: true` and stays there, so a launcher or a renew that waits for "back at
  a prompt" reports a bare timeout that names nothing. Read `blocked` +
  `launch_pending` + `interactive_ready` from `agent list` and surface the pane's own text;
  then either make the seat non-interactive for a fresh cwd or refuse and quote it.
- **Answering another program's interactive prompt is a decision taken for the operator, so
  it is never silent — and it is only made for the object it names.** A launcher that
  auto-accepts a folder-trust (or any consent) dialog must print that it did, naming the
  target and the keys sent; a dialog answered invisibly is indistinguishable from a seat
  that started clean, and the operator never learns a trust decision was recorded for that
  path. Gate the answer on the dialog identifying exactly your target, matched as a whole
  token on BOTH sides — a path check that looks only forward accepts `/home/u/tmp/repo` for
  `repo=/tmp/repo`, which is the wrong-directory acceptance the check exists to stop — and
  when it cannot be identified or does not match, send no keys: refuse and quote the
  screen. Tests must cover the negative (a dialog naming another path, asserting the stub
  saw no send-keys) or they pin the code's own guess about the layout rather than the rule.
- **Create-then-register leaks a half ledger when the second create fails.** A setup verb
  that registers each unit inside its create loop (session row for seat 1, then seat 2)
  leaves seat 1's row behind when seat 2 fails, with its pane already closed: the board now
  claims a live session that answers nowhere, and the next dispatch resolves to it. Register
  only after every create succeeded, unregister in the failure path as well as closing
  panes, and prove it by failing the SECOND unit — a test that fails the first one never
  reaches the leak.
- **Check `cwd` in `agent list` before dispatching** — a pane rooted in the wrong repo
  is silent until the diff comes back in the wrong project.

## Gates

- **A sandbox refusal is not a workaround licence.** A worker's permission classifier
  refusing a genuinely live command (a live round against the real daemon, a flash,
  anything touching someone else's workspace) is correct behaviour: it pastes the refusal
  and stops, and the manager runs that one command instead, handing the verbatim output
  back for the report. Keep it to one command per iteration and read the refusal before
  ruling — a denied command and a failed command look identical in a report. Run that
  command to a file (`… > $TMPDIR/live.txt 2>&1`) and grep it for the line each step must
  print (`up tab`, `answered`, `renewed`, `stopped`, `ok:`): piping through `tail`
  truncates the head, which is exactly where the setup line that proves what the verb did
  lands, and re-running to see it costs a whole live round. The round's own script must
  clean up BEFORE it asserts nothing was left behind, with the cleanup on the failure path
  as well, or a deliberately refused teardown fails its leftover-pane assertion while the
  tab is still open — a self-inflicted failure that reads like a product defect.
- **A guard must fail when the thing it guards is broken.** Two shapes pass on broken
  code: an assertion that accepts the success path (a helper returning `"ACCEPTED: " +
  result` when the call succeeded, then checked for a substring that both paths produce),
  and one that measures the wrong observable (timing the caller instead of asserting the
  stub's pid is gone). Prove every guard twice — revert the fix, and force the failure
  path to succeed. A guard that survives either one is decoration, and only a reviewer
  who mutates the code will notice.
- **A test that asserts a string the code itself mints proves nothing.** When both the
  implementation and its test carry the same literal (`"… at a prompt"`, a status word the
  verb prints, an error sentence built a few lines away), the test passes for any state that
  reaches that print — including the one it was written to exclude. Require the negative
  case: the observed condition absent (screen without the prompt marker, dialog naming
  another path, status `blocked`) must produce a refusal with its own message, and that
  refusal is what the test asserts. Same trap as a declared total nobody re-derived: the
  number and the text agree because one author wrote both.
- **Never assert on wall-clock text in a rendered row.** `get`/`get --full` prints relative
  ages and a live lease (`created: 0s`, `lease: 59m left`), so two renders compared for
  equality — the natural way to prove "the refused edit changed nothing" — differ whenever
  a second or minute boundary falls between the calls, and a lease assertion after a
  TTL change is 59 or 60 depending on the same luck. Strip those lines before comparing,
  assert on the event timeline or stored values, or assert a range. A suite that is green
  on the run you happened to watch proves nothing; repeat it three to five times.
- **One observed failure is reproduced or retired — never averaged.** When a suite fails once
  in a handful of runs, the adjudication costs a loop of that test (or the whole binary) ten to
  thirty times, not one re-run: a pass on the retry is exactly what a flake looks like, so it
  proves nothing either way. If it will not reproduce, name the mechanism you suspect and record
  the ruling where a recurrence will be found again — the epic's numbered decisions — instead of
  filing a row on a rumour or leaving it as a footnote in a report. Suspect the harness first
  when parallel tests allocate resources: binding `:0` and closing the socket before the child
  uses it lets two tests adopt the same server, and the loser then sees a state word
  (`Already`) where it asserts another (`Created`).
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
- **An assertion on an error's shape breaks the first time a verb wraps it.** A check that
  every failure prints exactly one line passes until a new verb prefixes the message
  (`add: board server at …`), and from then on a random subset of the suite fails depending
  on which path happened to hit the kill — which reads as flake, gets retried, and hides
  that the error contract moved. Classify by cause once (unreachable, connection lost
  mid-request, query error) and assert the class rather than one driver's wording, then
  prove the shape through the verbs that wrap errors, not only the one the test calls. The
  same flake returns every time the schema grows if write sessions re-apply the whole DDL:
  each added table widens the window in which a killed server surfaces as a reset, so the
  widening is a finding about the write path, not about the test.

- **A playbook may only teach verbs the binary has.** The mirror of retiring a stale
  workaround: a skill written from an epic's verb list describes a surface that the
  implementation may never have reached, so every verb and flag the skill teaches is
  checked against the build that exists (`<verb> --help`, one line each) before the skill
  is called done — otherwise the next session runs a `Usage: … <COMMAND>` line and reads
  it as a broken tool. When a promised verb is genuinely missing, file it as its own row
  and leave the skill honest about the gap; quietly deleting it from the prose makes an
  unfinished epic look finished, which is the more expensive error.

## Probes and live boards

- **A throwaway board takes two settings, not one.** On a server-backed build the path
  env (`AGENTBOARD_DB`) names the data directory while the server comes from the URL env
  (`AGENTBOARD_SURREAL_URL`); a suite or probe that sets only the path talks to the
  *default* server, i.e. the owner's live board, and still prints PASS. Every fixture aims
  both at a throwaway (temp dir plus a random loopback port). The proof it did is never the
  green line: take `ls --limit 400 | wc -l` and the newest event sequence before and after
  and require them unchanged, and add one run with the URL pointed at a dead port that
  must FAIL rather than quietly fall back to the default server.
- **After a default-backend flip, the suites the row does not name are the hazard.**
  Suites written against the old path kind (a `.db` file) carry an env prefix that sets the
  path only, so the flip turns every untouched suite into a writer on the live board. Before
  calling the flip done, enumerate the test directory and the repo's gate table and give
  every suite the throwaway treatment — a gate table row whose suite cannot run on the new
  default (`rm: cannot remove …: Is a directory`) is part of the flip, not a follow-up.
- **`ls` hides the scale of the damage.** Its default limit is 20 rows, so a runaway probe
  that wrote 160 looks like 20. Count per status with `--limit 400`, and scan for real rows
  wearing a fake assignee (`T48 wrk w1`): a suite that claims "the next ready task" steals
  live work, not just ids.
- **Cleanup is three steps and `status` is not one of them for assignees.** Drop the junk
  in a loop — `for i in $(seq A B); do agentboard --actor <mgr> status "T$i" dropped "junk
  from <suite>; not real work"; done` clears 160 rows in seconds — then `status <T> ready`
  the real rows that got claimed, then `edit <T> --assignee ""`, because `status` clears
  the lease and leaves the assignee in place (a stale one shadows later dispatch
  resolution). Verify with the same `--limit 400` count and the open list, record the
  incident on the row that caused it, and file the fixture fix as its own row whose
  acceptance is the before/after-count proof.
- **A count delta is only evidence once you know who wrote the rows.** The before/after
  counts that prove a suite left the board alone are read across a window in which *you*
  also file rows, so a delta of exactly the rows the manager filed (the row under review,
  a follow-up, a nit row) is your own paperwork and not a suite write. Compare the newest
  event sequence as well, and when a delta does appear name the ids it added instead of
  asking the worker to explain it — the current `ls --limit 400` against the list from
  before the round gives the new ids in one command, and a worker that volunteers "the +1
  is the row you filed" has done that subtraction for you and should be believed only after
  you repeat it.
