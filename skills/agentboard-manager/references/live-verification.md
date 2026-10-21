# Verifying work against the live process

Depth for the moments when the board's status is not the evidence: an install that
reached nobody, a live round only the manager can run, and a submission whose body
is not the change.

## An install does not reach a running agent

`cargo install` replaces the file on PATH. Every agent pane spawned its MCP server
as a child process at session start and keeps that inode until the agent restarts —
`/clear` does not respawn it. So the board surface a working pane calls can be several
installs old while the file on disk is current.

Prove which build a pane is running, without asking it:

```bash
for p in $(pgrep -f "^$HOME/.cargo/bin/agentboard"); do
  printf '%s -> %s\n' "$p" "$(readlink /proc/$p/exe 2>/dev/null)"
done
```

`(deleted)` after the path means that process holds a replaced binary. Your own
Hermes-side MCP server shows the same and is harmless — you drive the board by CLI.
The worker and reviewer panes are the ones that matter.

Symptom to recognise: the worker's `task_get` output lacks the newest fields, its move
to `review` is accepted with no diff row, and no review row is auto-filed. That is a
stale pane, not a broken gate. Restart each agent in its pane (exit the agent, then
`herdr agent start <name> --kind … --pane <id> -- <flags>`), re-check the `/proc` links,
and re-dispatch. An install that changes the board surface is not finished until that
restart has happened.

## Who runs the live round

A sandboxed worker's harness refuses live-process work — starting real agents, touching
the real socket — and it is right to. Do not read that as a failure, and do not add a
permission rule to the user's agent config on your own initiative.

The loop that works: the worker builds against a stub and says in its report that the
live round is the open item; the **manager** runs the one guarded command from the repo
(`AGENTBOARD_HERDR_LIVE=1 bash tests/…`) and pastes the output onto the row; the worker
records it and moves the row on. The manager runs the command, never writes the code:
authorship stays in the worker's row, evidence on the board.

Run the repo's own guarded script rather than writing a one-off live probe. The shape
that survives review: gated on an env var so a bare run skips, its own scratch tab,
agents named `scratch-*` so they can never collide with the manager's seats, every other
agent and pane compared before and after, and it closes only the pane ids it printed.
A failed round must clean up its own panes too, or the next run starts on top of a
half-open team and the leftover panes look like the tool's bug.

Expect the live round to fail for reasons a stub cannot model. The useful ones are the
failures the stub's own assumption hid:

- a target pane that is not yet a shell refuses `agent.start` (`agent_pane_busy`) — a
  startup window, not a permanent property of that kind of pane. Do not turn one failed
  observation into an invariant in a comment, a doc or an epic: observe the same path
  again after the fix succeeds before writing it down as a rule.
- an agent started in a brand-new directory can stop on its own first-run/trust dialog
  and never reach a prompt (`agent_status: blocked`, `launch_pending: true`). A lifecycle
  verb that cannot reach a prompt must refuse and print what is on the screen; one that
  answers the dialog for the operator must print that it did, naming the directory, and
  must answer only when the dialog names that exact directory as a whole path token
  (both boundaries non-path characters). A security decision taken silently is a defect
  even when the decision itself is defensible.
- "at a prompt" is a claim about the screen, not about status: `idle` plus a marker on
  the last lines is defensible, `idle` alone is not.

## A submission's body must be the change

A diff row carrying a stat summary ("14 lines: files touched, gates run") is not a
submission. Protocol refuses it and the reviewer will refuse the round over it, which
costs a whole round. Check the body yourself before dispatching the review: if it names
files instead of showing hunks, send it back with the recording recipe rather than
letting the review discover it. The cheap tell is a line count — `diff ls` prints
`D<k> round <r> … <n> lines`, so a body of ten or twenty lines for a change whose
`git diff | wc -c` runs to tens of kilobytes is a summary, and a real submission of
that change runs to hundreds or thousands of lines. One command, before the review.

Recording a large change, once each file:

```bash
git add -N <new/files>          # intent-to-add makes new files appear in git diff
git diff > /tmp/change.diff     # complete change, new files included exactly once
agentboard diff add T<n> --body-file /tmp/change.diff
```

`--body-file` is the CLI's designed path for a body too large to inline; approving it for
a worker is not a break of "board writes go through MCP", it is the structured path for
size. The MCP tool takes only an inline body, which is why a 40 KB change has no MCP
shape — say so and let the CLI verb carry it.

Every round, a fix round included, records the **complete** change again, so the newest
diff reproduces the tree: the reviewer reads only the newest diff, so a delta would hide
the rest of the change from the verdict. Earlier `D` rows stay as history; say on the row
which id is live and which are stale records of the same work, so a later session does
not compare an old body with the current tree and call identical work a mismatch.

Do **not** append a second pass with `git diff --no-index /dev/null <file>`: with
intent-to-add entries the file is already in `git diff`, and the append duplicates every
new file. The record then contains the whole change but `git apply` fails on it.

Verify the record the way a reviewer will, not by hashing the rendering:

```bash
# regenerate the submission from the tree, then compare bytes with the recorded body
agentboard diff get D<n> --full | tail -n +2 | head -c -1 > /tmp/recorded.diff
diff -u /tmp/recorded.diff /tmp/regenerated.diff && echo identical
```

The render prepends a header line and appends one trailing newline, so a raw
`sha256sum` of `diff get --full` is off by one byte against the file. Strip both, or
better, regenerate from the tree and diff: the property that matters is that the
recorded bytes reproduce the tree.

## Watching a submission, not a status

A status wait on a row that already sits in a watched status matches on the first poll:
an arm watching `--status review,...` for a row already in `review` fires at once and
reports nothing new. `--after <seq>` does not save it — for a status-only match the
sequence filter is a no-op.

When what you are waiting for is a submission rather than a transition, wait on the kind:

```bash
agentboard wait T<n> --kind diff --after <last-seq> --timeout 300
```

When a row is already at `review` and its review child is what moves, watch the review
row alone: `children T<impl>` names it, and watching parent and child together guarantees
the parent matches first.

## Two statuses a worker must not choose

`not_done` means the reopen cap was hit and is the store's to set. A worker that wants a
decision does not declare itself `not_done` — it takes `blocking` with `needs=input` and
asks. A caller-set `not_done` is a board defect: no script can tell it apart from a capped
row, so the manager's triage (drop it, or file a replacement) stops being decidable.

`done` from a worker is a claim about its own gates; `review` is where a second pair of
eyes starts. A worker that sets `done` on its own implement row has closed a gate it was
not given.

## When the installed skill copy is ahead

`skills/install.sh --force` overwrites the installed copy, so the direction of drift
decides whether installing ships knowledge or destroys it. Check first:

```bash
wc -l skills/<name>/SKILL.md ~/.hermes/skills/<name>/SKILL.md
diff -rq skills/<name> ~/.hermes/skills/<name>
```

Installed longer, with reference files the repo lacks, means the installed copy is
ahead — the manager's own post-turn skill pass writes there. Do not install over it,
and do not let a worker: a worker that proves the install on a scratch target and
reports the drift is behaving correctly, not stalling.

Resolve it by adopting installed → repo, then reinstalling:

```bash
cp -a ~/.hermes/skills/<name>/. skills/<name>/
bash skills/install.sh --force
diff -r skills/<name> ~/.hermes/skills/<name>   # empty both ways
```

The adoption moves the tree under any diff a worker has already recorded. After it: tell
the worker to re-record the diff complete (new files via `git add -N`, once each), and
tell the reviewer which `D<n>` is live and which numbers are stale records of the same
work. A reviewer comparing a pre-adoption diff with the post-adoption tree refuses the
round for a mismatch the manager created.

## Watching a submission, not a status

A status wait on a row that already sits in a watched status matches on the first poll:
an arm watching `--status review,...` for a row already in `review` fires at once and
reports nothing new. `--after <seq>` does not save it — for a status-only match the
sequence filter is a no-op.

When what you are waiting for is a submission rather than a transition, wait on the kind:

```bash
agentboard wait T<n> --kind diff --after <last-seq> --timeout 300
```

When a row is already at `review` and its review child is what moves, watch the review
row alone: `children T<impl>` names it, and watching parent and child together guarantees
the parent matches first.

## Two statuses a worker must not choose

`not_done` means the reopen cap was hit and is the store's to set. A worker that wants a
decision does not declare itself `not_done` — it takes `blocking` with `needs=input` and
asks. A caller-set `not_done` is a board defect: no script can tell it apart from a capped
row, so the manager's triage (drop it, or file a replacement) stops being decidable.

`done` from a worker is a claim about its own gates; `review` is where a second pair of
eyes starts. A worker that sets `done` on its own implement row has closed a gate it was
not given.
