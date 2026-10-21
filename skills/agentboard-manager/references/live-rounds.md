# Proving lifecycle verbs against the real muxer

A stub server proves framing, refusal paths and deadlines. It cannot prove the
pane/agent state machine, because the stub implements your assumption about
that machine. Every lifecycle verb (`up`, `down`, `renew`, `agent start`) needs one round against the real socket before a row closes.

## The guarded live round

Ship it as a script in the repo, not as ad-hoc commands, so the proof is
repeatable and its scope is auditable:

- gate on one env var (`…_LIVE=1`); without it the script prints `skipped` and
exits 0, so a normal suite run never touches the live session;
- name every agent it creates with its own prefix (`scratch-…`), so it can
never match the manager's real `coder`/`reviewer`;
- point the agents it starts at a throwaway board via `--env` (both the data
path and the server URL), never the live board;
- snapshot every other agent and pane before and after, and assert they are
identical; close only the pane ids the verb printed;
- check for leftovers AFTER attempting a forced teardown, and run that
teardown on the failure path too — a check that runs before cleanup fails for
the wrong reason and leaves the tab behind;
- assert the negative too: a `down` without `--force` on a working agent exits
3, closes nothing, and leaves the registration in place.

## Who runs it

A sandboxed worker's permission classifier can refuse a command that drives a
live multiplexer. That refusal is correct behaviour, not a bug to route around:
the worker must paste it and stop, and the **manager runs that one command** and
hands the verbatim output back for the report. Never let a worker "work around"
a classifier denial, and never accept a lifecycle row whose only proof is a stub.

Before running it: confirm the release binary is newer than the newest source
file (`stat -c '%y %n'` on both). A stale binary turns the round into a proof of
last round's code. Keep the tree frozen from the run until the reviewer reads
it — the manager's output is the evidence the review cites.

## What the real surface taught, and the rules that follow

- **`agent.start` needs an idle shell, and a fresh pane is not one yet.** A pane
  created by `tab.create` or `pane.split` can answer
  `agent_pane_busy: … is not an available shell` for a moment. Retry inside one
  named budget, and retry only on that error class — never widen it to any
  error, and never let the retry spend the caller's start timeout.
- **A brand-new repo puts the agent on its folder-trust dialog**, which shows as
  `agent_status=blocked` with `launch_pending=true` and no prompt. So: a verb
  that opens a team must END AT A PROMPT, or refuse and print what is on the
  screen; `renew` must demand a prompt marker on screen (an idle status is not
  proof that context was cleared); and a seat that answers a trust dialog must
  print that it did, naming the directory, because a security decision taken
  for the operator is never silent.
- **Match the directory as a whole path token.** Substring matching on the repo
  path accepts a dialog for a different project whose path merely ends with the
  same text. Require a non-path character on BOTH sides of the match; when the
  dialog does not name that exact path, refuse and send no keys. Failing closed
  (refuse, print the screen) is the correct direction when a layout changes.
- **Register session rows only after every seat settled**, and unwind rows and
  panes together on failure. Registering seat-by-seat inside the start loop
  leaves a ghost row (registry entry, closed pane) when the second seat fails.
  The test that proves the fix must fail on the SECOND seat.
- **A refusal is a feature.** `refusing to stop <name>: status=working` (exit 3)
lives in the library so every stop path goes through it; `--force` is the only
way past it, and it must actually stop a blocked/launching seat and close its
panes.
