# Agentboard seats versus one coding agent

## Question and scope

For the same repository task, model family and final grader, does the agentboard
seat team produce higher quality than one coding agent, and at what cost? A null
or negative result is valid. Every arm's final tree receives the same hidden
acceptance and regression checks; no primary measure is exclusive to the team.
Slice 1 uses four historical tasks and Claude sessions. It does not claim to
measure the general value of multi-agent systems.

## Arms

| Arm | Sessions and configuration |
| --- | --- |
| A-sonnet | One `claude -p` coding session, Sonnet 5.5, medium effort. |
| A-opus | One `claude -p` coding session, Opus 5.5, high effort; controls for the team's Opus seats. |
| B-team | Analyst Opus/high, coder Sonnet/medium, reviewer Opus/high. Each wake is a fresh `claude -p` session using its seat card and the scratch board. Analyst and reviewer cannot edit files. |

The deterministic Python manager uses zero model tokens. Manager cost is excluded,
so B's measured cost is a lower bound. If B already misses a cost threshold at
that lower bound, the result is robust; otherwise label it provisional because
LLM manager overhead was not measured. An LLM-manager arm and Codex arms are
later work, not part of this slice.

All arms use built-in tools only. The only permitted MCP server is the scratch
agentboard, and only for B. Runs use the same environment and PATH, serial
execution, `--dangerously-skip-permissions`, and an identical `AGENTS.md`-first
prompt preamble. No hooks, user `CLAUDE.md`, Hindsight or codebase-memory MCP are
available. Each session has a proposed `$6` budget and each task-arm cell a
proposed `$20` ceiling; these are owner-review proposals, not product limits.
Record arm order.

## Task corpus

The frozen slice uses four repository commits, with each base equal to the
historical parent state for the task. Prompts describe observed contracts and
avoid implementation hints. `bench/preflight.py` proves each hidden grader fails
on base and passes on reference, while every declared library regression passes
on both.

| ID | Kind | Base → reference | Prompt source |
| --- | --- | --- | --- |
| t1-actor-guard | Surgical store fix | `a89cb4df26b6` → `8744808b6e11` | Authored actor-roster bug report. |
| t2-claim-doc | Protocol documentation | `aca8d68dd68e` → `a93399507406` | T321/T318 item 6; prompt omits the corrected wording. |
| t3-wait-after | CLI and docs, multi-file | `8744808b6e11` → `f61ab674893b` | Authored re-armed wait requirement. |
| t4-map-usage | Store, adapters and docs | `59994cf64b90` → `4ceb1f979fea` | T308 description and goal verbatim. |

The reported substitute for a task that fails preflight is
`8add61acbebf` (base `567aed40d20b`); a substitution must be documented before
scoring. `task.json` records source commits, prompt edits, build, acceptance,
regression commands and allowed paths. Slice 1's runnable how-to is intentionally
left for C2.

## Isolation

Each arm and repetition gets a clean tree exported from the task base, initialized
as a new repository with one synthetic base commit. It receives no reference
history. B gets one fresh scratch `AGENTBOARD_DB` and loopback daemon per run,
stopped afterward. Every child process receives the scratch database path and
seat cards point only at the scratch port. Live board state is never used.

Sessions emit stream JSON with verbose events. The harness checks the `system/init`
event's MCP servers and tools against the arm allowlist. A violation makes the
run invalid; rerun it and do not score it. Use `--safe-mode` with strict explicit
MCP config. If safe mode removes that server, C2 may use settings with hooks
disabled; record the winning mechanism. Scan tool inputs for accesses to the real
repository outside `skills/` and to `bench/corpus/`; report any leak flag.

## Quality measures

Primary per-run PASS requires a successful build, hidden acceptance, and every
declared regression command. Regression commands are PASS-to-PASS: they must pass
on both base and reference. Report pass count by arm out of tasks × repetitions;
when repetitions are at least two, also report pass^k (all repetitions pass).

Secondary, unscored measures are formatting and clippy hygiene, changed files and
lines, out-of-scope paths, uncommitted leftovers and the leak flag. B-only
diagnostics are seat sessions, retries, reopen count (reviewing-to-implementing
events), and reviewer finding count and severity. An optional blinded human rubric
scores correctness, scope, tests and docs from 0–3; it is not part of the primary
verdict. The primary score is execution-based rather than LLM-as-judge.

## Cost measures

Capture per-session input, output, cache-creation and cache-read tokens,
`total_cost_usd`, duration, turns and assistant `tool_use` count from stream JSON.
Sum these per run and retain a B per-seat breakdown. Measure wall-clock from the
first session start to grading start. USD is the primary cost scalar and is the
CLI's API-equivalent amount even for subscription use; disclose that basis.

## Proposed verdict table

Thresholds are proposals for owner review, not a hard service-level objective.
For each baseline A, compute `ΔQ = PASS(B) − PASS(A)` and `M = USD(B) / USD(A)`.

| ΔQ | Cost multiple M | Verdict |
| --- | --- | --- |
| `< 0` | any | worse |
| `≤ 0` | `≥ 3` | posture FAIL |
| `= 0` | `≤ 1.5` | tied, slight premium |
| `= 0` | `1.5–3` | tied, premium M (owner judges) |
| `> 0` | `≤ 3` | better |
| `> 0` | `> 3` | better but expensive |

Posture FAIL means a large cost increase without a quality gain. With fewer than
three repetitions per cell, label every verdict `directional (n=…)`.

## Report format

Write `docs/benchmark-runs/<run-id>.md` and its `.json` summary. The header records
harness commit, Claude version, models and effort, repetitions, caps, isolation
mechanism and the excluded-manager note. Include one table per task with arm,
acceptance/regression PASS, hygiene, out-of-scope count, outcome, sessions, tool
calls, all four token counts, USD, wall-clock and B diagnostics. Add aggregate
PASS, cost, tokens and wall-clock, Pareto note, and verdict lines for B versus both
single-agent baselines. `report.py --from <results.json>` must regenerate the
Markdown byte for byte. Raw JSONL and trees stay in scratch.

## Threats to validity

- Four tasks and one repetition provide little statistical power; individual
  tasks may not represent later work.
- Excluding manager tokens makes team cost a lower bound and may change a close
  cost comparison.
- Safe-mode and strict MCP configuration may differ from production seats.
- Prompt underspecification can make a task fail for reasons unrelated to seat
  coordination; preserve authored prompts and disclose every edit.
- Model, CLI and pricing drift make results time-specific; record exact versions
  and reported USD values.
- Historical references can leak through cached context or tools. Fresh exported
  trees, restricted tools and the leak scan reduce but cannot eliminate this risk.
- Fixed historical commits may have idiosyncratic tests, dependencies or code
  paths. Preflight validates graders; it does not prove external generality.

## Prior work (reported, not independently verified)

The T384/T385 research notes report that MCP product benchmarks measure MCP tool
use rather than orchestration; that small coding tasks can incur substantially
higher multi-agent costs with little quality gain; that subagents can hurt short
SWE-bench-style work; and that parallelizable work or clean-context review may
benefit from teams. These are design context, not evidence produced by this
benchmark. Slice 1 uses their execution-grader and token-efficiency ideas without
claiming to reproduce those studies.
