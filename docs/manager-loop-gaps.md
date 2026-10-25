# What the manager loop is missing

Written 2026-10-24 by the manager seat (hermes) after driving one epic end to end —
`qt-gui`, 28 board rows, three live seats on herdr panes — against `agentboard 0.2.0`.
This is an observation file, not a plan: it records what cost manual work, with the
evidence, the code that causes it, and the smallest fix I can see. Nothing here had been
implemented or filed as a row when it was written; the status note below records what has
landed since.

Where it says "today", it means: during that epic, on this box, with the board at
`~/.local/state/agentboard/lmdb`.

**Status 2026-10-25.** Gap #1 (`agentboard wait` / `task_wait`) landed as `8a11dc1`,
and gap #3 (`update --release` plus the self-describing holder refusal) as `3775ac9`.
Gaps #2, #4 and #5 are closed by decision rather than by code: the owner fixed the
design as "the board stores state and blocks, the manager is the wake, a seat does
its row and stops" (AGENTS.md, `## Shape`). Delivery into a pane, liveness joins and
a per-seat queue would each make the board a second scheduler, so they are not
wanted; a seat field on events stays unnecessary while the manager launches each
seat under its own actor name. The evidence below still describes `0.2.0` as it was.

---

## 0. What the board already has (so this file does not ask for it twice)

Verified against `agentboard 0.2.0`: 13 CLI verbs (`add ls get epic update claim comment
feed brief agents path reset serve`), 10 MCP tools (`task_add task_list task_get
task_epic task_update task_claim task_comment board_feed board_brief board_agents`).

The information a manager needs is already in the model:

- a status change is an event, and `feed --wait` blocks on it (`src/main.rs:279-300`);
- `feed --recipient` filters to a seat, and an assignment carries a recipient
  (`src/render.rs:93-94`, `skills/agentboard-team/SKILL.md:118`);
- `in_progress`/`blocked` require an assignee, and `in_progress`/`blocked`/`review`
  protect it from a foreign writer (`src/domain.rs:62-68`, `:291-298`);
- a foreign holder must be overridden with `force` + a non-empty reason
  (`src/domain.rs:285-286`).

So this is not a storage-model complaint. Everything missing is in the **loop** between
"the board recorded something" and "the seat that must act knows".

---

## 1. Gap: there is no blocking wait for a row or a status

**What happens.** The manager cannot say "block until T27 reaches review". `0.2.0` has no
`wait` verb:

```
$ agentboard wait T27 --timeout 600 --status review
error: unrecognized subcommand 'wait'
```

The only blocking primitive is the board-wide feed cursor:

```
agentboard --actor hermes feed --after 601abf84…:218 --wait 1800
```

That `1800` was accepted by `0.2.0` and silently cut to the store's 300 s ceiling
(`feed.rs` `WAIT_MAX_MS`), so each re-arm was really five minutes, not thirty. Today
`feed --wait` above 300 is refused outright, exit 2 (`4387229`); the command stays as
it was typed.

**Why that is not equivalent.** The cursor watches *any* event, not the row I care about;
it exits after the timeout, so a quiet window and a stalled seat look identical; and I
re-arm it every time, which is one agent turn per window. In the Qt GUI epic that was
roughly 30 re-arms, each a turn spent doing nothing but looking.

The MCP side is worse: `board_feed` accepts `wait`, but the MCP call itself is capped at
300 s, so a long watch cannot be one call. I tried it and got
`MCP call timed out after 300.0s`.

**Smallest fix.** `wait <ids…> [--status a,b] [--epic] [--timeout]` plus a `task_wait`
MCP tool that re-queries on every poll (not a frozen set), returning a typed outcome and
a distinct exit code for timeout — the shape `~/Projects/agentboard` had in an earlier
generation and `0.2.0` dropped. The store already exposes what it needs: `feed` can filter
by task, kind and recipient; a wait is that filter in a loop with a status predicate.

**Gate.** A row moved by another process during the wait returns its new status; a row
already in the requested status returns immediately with a "last:" label (do not pretend
causality); a timeout exits 3; an unknown status exits 1 by name.

---

## 2. Gap: an assignment is recorded, but nobody is woken

**What happens.** `assignee` is set, the event carries `recipient`, and then nothing
happens. `skills/coder/SKILL.md:22` says the assignment "carries recipient `reviewer` and
wakes it" — that sentence describes a convention, not code: there is no `herdr`, `wake` or
`deliver` anywhere under `src/`.

**Evidence.** Every handoff in the epic needed me to do this by hand, on the pane:

```
herdr agent prompt reviewer 'T27 is in review and assigned to you (revision 5) …'
```

Five times, for coder→reviewer and reviewer→coder, plus three times to start the next
queued row. When I did not, the row sat in `review` with an assignee and no reader — the
owner noticed before the board did ("hmm, why coder is done and reviewer is also done, why
the workflow not moving forward").

**Smallest fix, in order of honesty.** (a) make `agents` report the live seats with their
pane ids (it reports actors' last activity, not whether a seat is alive); (b) an explicit
`--notify <seat>` on update, or a manager-mode `dispatch <T>` that resolves assignee →
pane → `herdr agent prompt`; (c) at minimum, delete the "wakes it" sentence from the
skills, because a seat that trusts it will stop watching and the work will park.

**Gate.** On a scratch board with two fake seats, an assignment to seat B produces a
prompt on B's pane within N seconds, and an assignment to a seat with no pane refuses by
name rather than silently recording.

---

## 3. Gap: assignment names a seat, and a dead seat strands the row

**What happens.** The coder's skill tells it to hand off with
`task_update(…, status="review", assignee="reviewer")`. That is a **name**, not a live
process. When the reviewer seat is replaced — in this epic, opencode ran out of quota and
a hermes seat took over — the new seat is refused by the holder check:

```
Held { holder: "reviewer" }   // src/domain.rs:293-295
```

It happened three times: T26 refused twice and had to post its PASS verdict as a comment
and then block on a clarify asking who should move the row; T27 refused once. Each time I
recovered with the designed escape — `force` + reason:

```
agentboard update 26 --if-revision 3 --assignee hermes --force \
  --reason "the retired reviewer name still held the row, which blocked the reviewer seat"
```

That works and leaves an audit trail, which is good. The defect is that it was needed at
all, and that the *reviewer* had to ask a human/manager how to proceed, mid-review.

**Correction on my own earlier claim.** I told the owner that `--status review` hardcodes
the assignee `reviewer`. That is wrong: with an explicit actor and no assignee, `review`
assigns the writer (`src/domain.rs:247-249`); the literal `reviewer` comes from the skill's
instruction. The binary is not the culprit here — the *convention* is, because it names a
seat identity that can die.

**Smallest fix.** One of: resolve `--assignee` against the live roster and refuse an
unknown one without `--force` (partially there: the CLI has a "register an unknown target
assignee" flag, so the concept exists); or let a seat that holds the same *role* take over
without a force; or add `--release` so a dead holder's row returns to `ready` in one verb.

**Gate.** Take a row to `review` with assignee `reviewer`, then write to it as `hermes`
without force: today it must refuse (prove that), and after the fix it must either succeed
because the roster says `reviewer` is not live, or refuse with a message that names a
recoverable verb instead of only `--force`.

---

## 4. Gap: "working" does not mean "alive"

A row in `in_progress` with a lease tells the manager nothing about whether the seat is
still there. During the epic I distinguished the two by reading panes by hand:

```
herdr agent get coder          # agent_status: idle|working|blocked
herdr agent read coder --lines 20
```

Two real cases that the board could not tell apart from progress: a codex seat that hit
its usage limit while holding a claimed row (the pane printed "You've hit your usage limit";
the board still said `in_progress`), and an opencode reviewer that fell back to a free
model and went idle with three rows in `review`. Both times the recovery was a pane
inspection, not a board query.

**Smallest fix.** `brief` should name in-progress/blocked rows whose holder has no live
session, or whose lease is older than some threshold with no event since the claim. The
lease already exists (`lease_until`, `started_at`, TTL 1800 s in the store); what is
missing is the join to liveness and the sentence in `brief`.

**Gate.** Start a row in `in_progress` under a seat, kill that seat, run `brief`: the row
must appear under "holders gone / stalled", not under "in flight".

---

## 5. Gap: two seats on one actor name write indistinguishable history

`AGENTBOARD_ACTOR` is per process, and the hermes seats in this epic (coder and reviewer)
both ran as `hermes` because that is what the binary registers. The event log then cannot
tell a review verdict from a code report by the actor field, and the manager has to instruct
seats to sign comments by hand:

> "Name yourself 'reviewer seat' in every comment and verdict you write"

That is a human convention covering for a missing field. It worked — but an audit trail
that depends on the author remembering a signature is not an audit trail.

**Smallest fix.** Either per-seat actor names enforced at registration (a pane-backed seat
gets its own name even when the CLI is the same), or a `seat` field on events alongside
`actor`, written from the seat registration.

**Gate.** Two seats of the same agent kind, two actors or two `seat` values, each write one
comment; `feed` shows which seat wrote which without reading the body.

---

## 6. What I do *not* think is missing (guard against overbuilding)

- **No new store or schema.** Feed + recipients + lease + revision already carry everything
  above; the gaps are predicates and joins, not tables.
- **No board-side scheduler.** Choosing what to work on next is a manager judgement
  ("one unit of work in flight"), and `claim` with no id already answers "what is ready".
- **No queue per seat.** The priority field plus `claim` was enough to run 24 units in
  order; what hurt was never ordering, it was notification.
- **No commit/push verbs.** The repo rule (seats never commit) belongs in the skills, and
  it held: no seat committed during the epic, the manager did, once, on the owner's word.

---

## 7. Smallest useful change set, ranked by manual work removed

| # | change | removes | rough size |
|---|---|---|---|
| 1 | `wait <ids> --status … --timeout` + `task_wait` MCP tool with per-poll re-query | ~30 re-arms per epic, each a turn | one store fn + two adapters + tests |
| 2 | assignment delivery (roster ← pane ids, then prompt the assignee) | 5–8 hand prompts per epic, and every "why is it not moving" | `agents` liveness join + one dispatch verb |
| 3 | assignee liveness / `--release` | force+reason recoveries, reviewer blocked mid-review | predicate on update + one verb |
| 4 | stall surfacing in `brief` | pane inspection to tell idle from working | one join, one sentence |
| 5 | seat attribution on events | hand-written signatures in comments | actor or `seat` plumbing |

If only one of these is ever done, it should be **#1**, because it is the one that turns
the manager from a poller into a waiter. If two, add **#2**, because #1 without #2 means
the manager wakes up and still has to hand-prompt the next seat — which is exactly the
loop I ran for this whole epic.

---

## Appendix: the exact evidence collected

| observation | command / text |
|---|---|
| no wait verb | `agentboard wait T27 --timeout 600 --status review` → `error: unrecognized subcommand 'wait'` |
| MCP wait capped | `board_feed(after=…, wait=3000)` → `MCP call timed out after 300.0s` |
| holder refusal | `Held { holder }` from `src/domain.rs:291-298`; surfaced to the seat as "the board refused my field write twice ('task is he…')" |
| managed override | `agentboard update 26 --if-revision 3 --assignee hermes --force --reason "…"` → ok, revision 4 |
| dead seat still `in_progress` | codex pane: "You've hit your usage limit … try again at 7:08 PM"; board: `T27 in_progress` |
| idle reviewer with queued rows | `herdr agent get reviewer` → `idle` while T23/T24/T25 sat in `review` |
| assignment wording that promises a wake | `skills/coder/SKILL.md:22` — "That event carries recipient `reviewer` and wakes it." (no such code in `src/`) |
| status→assignee rule | `src/domain.rs:247-249` — `Review` with an explicit actor and no assignee assigns the actor, not a literal `reviewer` |
| protected fields | `src/domain.rs:62-68` — `requires_assignee` = in_progress/blocked; `protects_assignee` = + review |
