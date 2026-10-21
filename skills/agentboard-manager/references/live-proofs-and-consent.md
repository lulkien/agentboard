# Live proofs, and actions taken on the operator's behalf

Some phases cannot be proved by a stub. That is not a reason to skip the proof, and
not a reason to let a worker fake it.

## When a worker is barred from the proof

A sandboxed worker's permission classifier refuses live-process work (real terminal
panes, real agent sessions, real devices). The worker is right to stop and say so —
narrating a proof it did not run is worse than a red round. The manager's move:

- Read the refusal, then run **that one command** from the manager's shell and paste
  the output back onto the row for the worker to record. This is the narrow exception
  to "the manager runs no suites": a proof the worker is barred from, executed once,
  with the artifact landing in the row's report. It is not a licence to start running
  the repo's gates.
- Tell the worker in the row note, in advance, what will happen if the classifier
  refuses: paste the refusal, say "live round blocked by classifier", stop — never
  work around the sandbox, and never re-file the row.
- Expect a loop: fix → "ready for the live round" → manager runs it → output back. Two
  or three iterations are normal and each one finds something a stub cannot.

## What a stub cannot prove

A stub server encodes the code's own assumptions about the surface.

- A stub that answers `agent.start` on a pane, or a fixture whose dialog text the code
author wrote, proves the code's handling — not that the vendor behaves that way.
  Evidence of this class: a split tab root was assumed dead for `agent.start` and the
  stub agreed, while the real surface refused on a different condition and accepted on
  the next attempt; a fresh-directory seat sat `blocked` with `launch_pending` on its
  own trust dialog while every stub said it reached a prompt.
- So: when a phase's proof is "the stub says so", the row is not done until one live
  round exercises it, and the live round's output belongs in the report verbatim.
- When a live round fails, send the failure back with the exact output and the file
  line it contradicts, and ask for the real states (what does the surface say before,
  during, after) rather than a plausible ordering. "Make it work" invites a retry loop;
  "read the state and encode the rule with a comment naming the observed behaviour"
  produces a fix that survives the next herdr version.

## Actions the tool takes for the operator must be visible

- Answering a consent surface (a folder-trust dialog, a first-run wizard) is legitimate
  when the tool cannot work unattended otherwise — but never silently. Print one line
  naming the subject and the decision: which path was trusted, which keys were sent,
  which agent was force-stopped and what its status was. A silent acceptance of a
  security prompt is a defect even when the outcome is correct.
- Never press keys on a dialog whose subject cannot be identified. If the screen does
  not name the folder being trusted, refuse and print what it said; answering the wrong
  dialog is worse than stopping.
- Keep the refusal path informative: on timeout, print the agent's status, its
  `launch_pending` flag and a tail of the screen, so the operator can answer it by hand
  instead of guessing what the tool saw.

## Reading the board instead of the tool's word

- A status the protocol reserves for the board can still be written by a caller. When a
  row lands a status that contradicts its own event list (a `not_done` with no
  `needs_fix` moves and `reopens: 0`), believe the events, treat it as a caller-declared
  state, and file the enforcement gap rather than triaging a phantom.
