# CLI role capabilities: evidence and gaps

Evidence snapshot: 2026-10-23, T306 analysis, T307 notes, and T349 report.
T306 manager answer #2102 accepts historical observations and explicit gaps;
T351 documents that plan without changing policy or running new probes.
Commands below are evidence records, not instructions to run against a live seat.

The intended policies are analyzer: read the checkout and probe in scratch;
coder: write owned workspace files, no arbitrary shell, restricted MCP;
reviewer: read the checkout under the review rules in `../PROTOCOL.md`.
These policies and the cards do not prove that every restriction is enforced.

| State | Meaning |
|---|---|
| MEASURED | Runtime behavior observed on this box, within the stated scope. Historical manager observations are labeled manager-reported, not independently reproduced by this documentation worker. |
| READ-NOT-RUN | Local help, configuration, parser, evaluator, or source evidence only. Parsing a key or evaluating a rule is never runtime denial proof. |
| NOT MEASURED | Runtime evidence missing, blocked, or unavailable; the reason is recorded. It does not mean impossible. |

“Independently observed” below means observed by the T306 analyzer, not rerun
for T351. Relative card paths are under `seats/`. Source rows can be read with
`task_get(id="T306"|"T307"|"T349", full=true)` (one id per call).
Where a source gives no complete invocation or transcript, that absence is
explicit; no reconstructed command is presented as an actual invocation.

## Versions and observation limits

| CLI | Version evidence (T306 unless stated otherwise) | Runtime limits |
|---|---|---|
| Codex | Independently observed: `codex --version` → `codex-cli 0.163.0`. | Historical runtime evidence below is accepted by manager #2102. Nested Codex sandbox startup failed its app-server socket-directory check, even with writable `XDG_RUNTIME_DIR`; that is not a role denial proof. |
| Claude Code | Independently observed: `claude --version` → `2.1.307 (Claude Code)`. | NOT MEASURED: manager #2102 reports `weekly limit`, reset 2026-10-25. An earlier no-tool print baseline timed out (exit 124); timeout is not denial evidence. |
| OpenCode | Independently observed: `XDG_DATA_HOME=/tmp/t306/xdg /home/kiewn/.opencode/bin/opencode --version` → `opencode v2.1.1`. | NOT MEASURED: debug config service startup reports `EPERM listen`; independent `socket.socket()` raises `PermissionError [Errno 1] Operation not permitted`. The binary was absent from PATH but exists at that absolute path. T349 and the older README's “not installed” statement are superseded by this narrower observation. |
| Hermes | Version NOT MEASURED: `hermes --version` hits a read-only `.install.lock` error. With scratch `HERMES_HOME` and `HERMES_DISABLE_LAZY_INSTALLS=1`: `no dependency environment is committed for this install` (complete scratch invocation not supplied). | Manager #2102 excludes probing its own sole runtime. No repair, install, or live worker-profile probe was performed. |

## Codex evidence records

These identifiers carry the command/tool, available output, provenance, and
limits used by the role matrix.

| ID | File and key | Observation and available output | Source and limit |
|---|---|---|---|
| C1 | `codex/analyzer.config.toml`: `sandbox_mode="workspace-write"`, `approval_policy="never"`; loaded by `codex -p analyzer`, cwd in scratch | T306 independently used read-only shell and scratch Python in the outer analyzer seat. Python expression `Path('/tmp/t306/write-proof').write_text('scratch writable\n')`; readback `scratch writable`. Complete shell wrapper/readback command not supplied. | T306 report: evidence of shell availability and scratch writes in that seat, not a per-command allowlist. |
| C2 | `codex/analyzer.config.toml`: `sandbox_mode="workspace-write"` plus scratch cwd | Manager-reported write probe of `/home/kiewn/Projects/agentboard/PROBE_SHOULD_FAIL.txt` → `read-only filesystem`, exit 1. Historical complete command unspecified. | T306 manager #2090 observation, retained in T306 plan and #2102; not independently reproduced. Cwd matters: rooting workspace-write in the checkout would allow checkout writes. |
| C3 | `codex/coder.config.toml`: `[features] shell_tool=false`, `unified_exec=false` | Manager-reported live `codex -p coder` and scratch `codex exec -p coder`: requests for `rg --version` and `ls -l` → `NO SHELL TOOL AVAILABLE`. Full prompt/argv unspecified. Linked profile later reports `shell tool: no`. | T307 #2075 and T349 report. Observed tool availability, not OS denial of every possible execution channel. |
| C4 | `codex/base.config.toml`: `mcp_servers.<server>.tools.<tool>.approval_mode`; role card `approval_policy="never"` | Manager-reported analyzer calls `task_add` / `task_dispatch` → `MCP tool call requires approval, but approval policy is never`. Allowlisted calls use `approval_mode="approve"`. | T307 #2067; per-tool form honored per #2075 and T349, accepted by T306 #2102. Exact failed call arguments and success transcripts unavailable. This proves a general approval mechanism, not each role's complete verb policy. |
| C5 | `codex/coder.config.toml`: shell flags as C3, `sandbox_mode="danger-full-access"`, top-level `developer_instructions` | Manager-reported `apply_patch` created `probe_write.txt` with shell disabled; no full patch payload or tool response supplied. Through the role symlink, the seat quoted `SEARCH AND READ` and reported `shell tool: no`. | T349 report. Confirms file writing and linked coder-card loading. No measured workspace-only file boundary: “never write outside repository” is an instruction, not an established filesystem wall. |
| C6 | `$CODEX_HOME/<role>.config.toml` → `codex/<role>.config.toml`; shared `$CODEX_HOME/config.toml` | Manager-reported `readlink -f` resolves coder/analyzer/reviewer/manager links into `seats/`; full invocations/output paths unspecified. `codex --help` says role config layers over base user config (T306). | T349 link observation; coder runtime load is C5. Reviewer runtime enforcement is not inferred from links or analyzer behavior. |
| C7 | `$CODEX_HOME/config.toml` / role override: `mcp_servers.<s>.enabled`, `enabled_tools`, `disabled_tools` | T306 parser command below returns `enabled:false`, `enabled_tools:["allowed"]`, `disabled_tools:["denied"]`. | READ-NOT-RUN. Analyzer-profile parser acceptance in 0.163.0; filtering/disable at runtime and coder/reviewer role application remain unmeasured. |

C7's actual command (no server runtime probe):

```sh
codex -p analyzer -c 'mcp_servers.t306_probe.command="true"' -c 'mcp_servers.t306_probe.enabled=false' -c 'mcp_servers.t306_probe.enabled_tools=["allowed"]' -c 'mcp_servers.t306_probe.disabled_tools=["denied"]' mcp get t306_probe --json
```

## Role × capability × CLI matrix

### Codex 0.163.0

“MEASURED mechanism” in the approval rows refers only to C4's historical
analyzer observation. It does not claim a separate runtime probe for each role.
Parser-only profile loading stays READ-NOT-RUN even when the parser succeeded.

| Role | Capability | State | File/key and evidence; boundary |
|---|---|---|---|
| Analyzer | Shell access inside workspace-write sandbox | MEASURED | `codex/analyzer.config.toml`, `sandbox_mode`; C1: read-only shell and scratch Python available in the outer seat. No command allowlist proven. |
| Analyzer | Scratch write / checkout read boundary | MEASURED | Same file/key plus scratch cwd; C1 readback `scratch writable`; C2 manager-reported repo write `read-only filesystem`, exit 1. |
| Coder | Shell tool removal | MEASURED | `codex/coder.config.toml`, `[features] shell_tool`, `unified_exec`; C3 manager-reported `NO SHELL TOOL AVAILABLE`, later `shell tool: no`. |
| Coder | File write while shell is off | MEASURED | Same file, shell flags and `sandbox_mode="danger-full-access"`; C5 manager-reported `apply_patch` created `probe_write.txt`. Workspace confinement NOT MEASURED. |
| Reviewer | Read-only filesystem | READ-NOT-RUN | `codex/reviewer.config.toml`, `sandbox_mode="read-only"`, `approval_policy="never"`; read card only. Analyzer's different sandbox observation does not prove reviewer enforcement. |
| Analyzer | Load role-specific configuration | READ-NOT-RUN | `$CODEX_HOME/analyzer.config.toml` → `codex/analyzer.config.toml`; C6 help/link evidence and C7 actual `codex -p analyzer … mcp get` parser success. No separate load-isolation proof. |
| Coder | Load role-specific configuration | MEASURED | `$CODEX_HOME/coder.config.toml` → `codex/coder.config.toml`, top-level `developer_instructions` and shell flags; C5 manager-reported `SEARCH AND READ`, `shell tool: no`. |
| Reviewer | Load role-specific configuration | READ-NOT-RUN | `$CODEX_HOME/reviewer.config.toml` → `codex/reviewer.config.toml`; C6 link/help and card evidence only. |
| Analyzer | Per-tool MCP approval gate | MEASURED mechanism; role policy incomplete | `codex/base.config.toml`, `tools.<tool>.approval_mode`, analyzer `approval_policy="never"`; C4 manager-reported `task_add`/`task_dispatch` hard error. Complete role read/write subset NOT MEASURED. |
| Coder | Per-tool MCP approval gate | MEASURED mechanism; NOT MEASURED role subset | Same base keys and coder `approval_policy="never"`; C4 general mechanism and T307/T349 reports. No independently observed coder-specific verb-denial inventory. |
| Reviewer | Per-tool MCP approval gate | MEASURED mechanism; NOT MEASURED role subset | Same base keys and reviewer `approval_policy="never"`; C4 general mechanism only. No reviewer-specific runtime denial proof. |
| Analyzer | Individual MCP tool filtering / server disable | READ-NOT-RUN | `$CODEX_HOME/config.toml` / role override, `mcp_servers.<s>.enabled`, `enabled_tools`, `disabled_tools`; C7 parser output. Runtime filtering NOT MEASURED. |
| Coder | Individual MCP tool filtering / server disable | READ-NOT-RUN | Same candidate override keys, `codex/coder.config.toml`; C7 is analyzer parser evidence, not a coder runtime test. |
| Reviewer | Individual MCP tool filtering / server disable | READ-NOT-RUN | Same candidate override keys, `codex/reviewer.config.toml`; C7 is analyzer parser evidence, not a reviewer runtime test. |
| Analyzer | Git MCP write denial / exactly required server set | NOT MEASURED | `codex/base.config.toml`, `mcp_servers.*`, `default_tools_approval_mode="approve"`: broad shared grants, no measured role filtering. |
| Coder | Git MCP write denial / exactly required server set | NOT MEASURED | Same base keys; shell-off alone does not restrict git MCP writes. Coder's developer instruction is not runtime denial evidence. |
| Reviewer | Git MCP write denial / exactly required server set | NOT MEASURED | Same base keys; no measured reviewer server/tool inventory enforcing its role. |

The shared Codex base lists four servers: agentboard, git-mcp-rs, ferris-search,
codebase-memory-mcp. T307 requests five including hindsight; “exactly five” is
not established. Shared `default_tools_approval_mode="approve"` grants tools
broadly and must not be described as enforced read-only git access.

### Claude Code 2.1.307

T306's local `claude --help` is READ-NOT-RUN evidence for
`--allowedTools` / `--disallowedTools` (example `Bash(git *)`), `--settings`,
and `--permission-prompts none` (“anything that would prompt” denied, per help).
Settings keys `permissions.allow` / `permissions.deny` and MCP server allow/deny
key spellings remain candidate/unverified here. Weekly quota prevents runtime
proof of strict role allowlisting or fail-closed behavior.

| Role | Capability | State | File/key and evidence gap |
|---|---|---|---|
| Analyzer | Builtin tool denial / file editing | NOT MEASURED | No analyzer card exists (T349); help flags above are candidates, not a measured role policy. Weekly limit. |
| Analyzer | Shell command restrictions | NOT MEASURED | No analyzer settings file; `--allowedTools` / `--disallowedTools` help expression only. Weekly limit. |
| Analyzer | MCP tool/server restrictions | NOT MEASURED | No `claude/analyzer-mcp.json`; server permission keys unverified. Weekly limit. |
| Coder | Builtin tool denial / file editing | NOT MEASURED | `claude/coding-mcp.json`, `mcpServers` is connection config; no validated builtin permission keys. Weekly limit. |
| Coder | Shell command restrictions | NOT MEASURED | Same connection card supplies no proven shell policy; help flags and candidate `permissions.allow` / `deny` are READ-NOT-RUN only. Weekly limit. |
| Coder | MCP tool/server restrictions | NOT MEASURED | Same card's `mcpServers` does not prove per-tool/server denial; permission key spellings unverified. Weekly limit. |
| Reviewer | Builtin tool denial / file editing | NOT MEASURED | `claude/reviewer-mcp.json`, `mcpServers` is connection config; no validated builtin permission keys. Weekly limit. |
| Reviewer | Shell command restrictions | NOT MEASURED | Same connection card; command-pattern help only, candidate settings keys unverified. Weekly limit. |
| Reviewer | MCP tool/server restrictions | NOT MEASURED | Same card's `mcpServers`; no live deny/fail-closed observation or validated permission keys. Weekly limit. |

### OpenCode v2.1.1

READ-NOT-RUN evidence: `opencode/opencode.json` has global `permission.bash`
with `"*":"allow"`, `"*grep *":"deny"`, `"*grep":"deny"`. This copied
pattern map is not a strict allowlist. Runtime matching order, default deny,
per-agent merge, and tool restrictions are unmeasured. The requested
`agent.<name>.permission` and tool-name rules are candidate mechanisms, not
established enforcement. EPERM service startup blocks the role probes (T306).

| Role | Capability | State | File/key and evidence gap |
|---|---|---|---|
| Analyzer | Shell/bash restrictions | NOT MEASURED | `opencode/opencode.json`, global `permission.bash`; per-agent permission behavior unverified; EPERM startup. |
| Analyzer | Tools/edit/webfetch restrictions | NOT MEASURED | Same file; requested `agent.<name>.permission` and tool-name rules unverified; EPERM startup. |
| Analyzer | Per-agent MCP disable | NOT MEASURED | Same file, `mcp` connection data; per-agent disable key/behavior not established; EPERM startup. |
| Coder | Shell/bash restrictions | NOT MEASURED | Same file, global `permission.bash`; per-agent merge/default deny unverified; EPERM startup. |
| Coder | Tools/edit/webfetch restrictions | NOT MEASURED | Same file; requested `agent.<name>.permission` and tool-name rules unverified; EPERM startup. |
| Coder | Per-agent MCP disable | NOT MEASURED | Same file, `mcp`; no measured per-agent disable mechanism; EPERM startup. |
| Reviewer | Shell/bash restrictions | NOT MEASURED | Same file, global `permission.bash`; no reviewer runtime denial proof; EPERM startup. |
| Reviewer | Tools/edit/webfetch restrictions | NOT MEASURED | Same file; requested per-agent/tool rules unverified; EPERM startup. |
| Reviewer | Per-agent MCP disable | NOT MEASURED | Same file, `mcp`; no measured per-agent disable mechanism; EPERM startup. |

Per-agent MCP disable is neither proven possible nor proven impossible here.
The global card's board actor is `opencode`, not a measured per-role identity.

### Hermes (version NOT MEASURED)

READ-NOT-RUN evidence: `hermes/manager.config.yaml` is a manager extract,
not three worker profiles. `platform_toolsets.cli` includes `terminal` and
`file`; `mcp_servers.<s>.enabled` is present. T306 read installed
`model_tools.py`, `_select_tool_names`, subtracting `disabled_toolsets`.
The installed source's full path is not supplied. Catalog selection does not
prove dispatch denial or OS filesystem confinement. Source-visible command
matching is described in the allowlist table below. No live profile probe was
authorized against the manager's sole runtime (#2102).

| Role | Capability | State | File/key and evidence gap |
|---|---|---|---|
| Analyzer | Toolset/terminal availability | NOT MEASURED | Manager extract `hermes/manager.config.yaml`, `platform_toolsets.cli`; source `disabled_toolsets` only. No analyzer profile/runtime probe. |
| Analyzer | Filesystem restriction | NOT MEASURED | Same extract has no measured worker filesystem boundary; no validated isolation key. Manager runtime excluded. |
| Analyzer | MCP filtering | NOT MEASURED | Same extract, `mcp_servers.<s>.enabled` read only; no per-role tool-filter/dispatch proof. Manager runtime excluded. |
| Analyzer | Shell command restrictions | NOT MEASURED | Installed `tools/approval_floors.py`, `approvals.deny`, `command_allowlist` source only; no analyzer profile probe. |
| Coder | Toolset/terminal availability | NOT MEASURED | Manager extract, `platform_toolsets.cli`; source `disabled_toolsets` only. No coder profile/runtime probe. |
| Coder | Filesystem restriction | NOT MEASURED | Manager extract; no measured worker filesystem boundary or validated isolation key. Manager runtime excluded. |
| Coder | MCP filtering | NOT MEASURED | Manager extract, `mcp_servers.<s>.enabled` read only; no coder filtering/dispatch proof. Manager runtime excluded. |
| Coder | Shell command restrictions | NOT MEASURED | Installed `tools/approval_floors.py`, `approvals.deny`, `command_allowlist` source only; no coder profile probe. |
| Reviewer | Toolset/terminal availability | NOT MEASURED | Manager extract, `platform_toolsets.cli`; source `disabled_toolsets` only. No reviewer profile/runtime probe. |
| Reviewer | Filesystem restriction | NOT MEASURED | Manager extract; no measured worker filesystem boundary or validated isolation key. Manager runtime excluded. |
| Reviewer | MCP filtering | NOT MEASURED | Manager extract, `mcp_servers.<s>.enabled` read only; no reviewer filtering/dispatch proof. Manager runtime excluded. |
| Reviewer | Shell command restrictions | NOT MEASURED | Installed `tools/approval_floors.py`, `approvals.deny`, `command_allowlist` source only; no reviewer profile probe. |

## Command allowlists: expression versus enforcement

One row per CLI. None of the following parser/help/source results establishes
a strict runtime default-deny allowlist for all three roles.

| CLI | State and mechanism | Evidence and limit (T306) |
|---|---|---|
| Codex | READ-NOT-RUN evaluator: scratch `/tmp/t306-policy/probe.rules`, `prefix_rule(pattern=…, decision=…)`; runtime strict allowlisting under `approval_policy="never"` NOT MEASURED | `codex execpolicy --help` and `codex execpolicy check --help` expose `--rules PATH COMMAND…`. Actual checks and available output below. No checked command was executed. Chosen T307 fallback is shell removal, so builds/suites/scripts are also unavailable through those tools. |
| Claude | READ-NOT-RUN help: `claude --help`, `--allowedTools`, `--disallowedTools`, `--settings`, `--permission-prompts none`; strict role behavior NOT MEASURED | Help shows `Bash(git *)` and says anything that would prompt is denied with `none`. Candidate settings `permissions.allow` / `deny` and MCP server allow/deny keys remain unvalidated; weekly limit blocks runtime proof. |
| OpenCode | READ-NOT-RUN card: `opencode/opencode.json`, `permission.bash`; runtime strict allowlisting NOT MEASURED | Map expresses patterns (`*` allow plus grep denials), not default-deny allowlisting. Ordered-looking map alone does not prove matching order, per-agent merge, or bash/edit/webfetch enforcement; EPERM startup. |
| Hermes | READ-NOT-RUN installed source: `tools/approval_floors.py`, `approvals.deny`, `command_allowlist`; runtime strict allowlisting NOT MEASURED | Source reads a deny glob list; `_command_matches_permanent_allowlist` reads `command_allowlist` and rejects compound shell operators for that shortcut. Full installed path not supplied. No live deny/matching or isolation proof; manager runtime excluded. |

T306's scratch rules:

```python
prefix_rule(pattern=["cargo","test"], decision="allow")
prefix_rule(pattern=["rg"], decision="forbidden")
```

| Actual evaluator command | Available output |
|---|---|
| `codex execpolicy check --rules /tmp/t306-policy/probe.rules --pretty cargo test` | `matchedPrefix ["cargo","test"]`, decision `"allow"` |
| `codex execpolicy check --rules /tmp/t306-policy/probe.rules --pretty rg --version` | `matchedPrefix ["rg"]`, decision `"forbidden"` |
| `codex execpolicy check --rules /tmp/t306-policy/probe.rules --pretty ls` | `{"matchedRules": []}` |

An unmatched evaluator result proves neither runtime denial nor runtime allow.
`codex features list` reports `exec_permission_approvals` as `under development
false`, `hooks` as `stable true`, `request_rule` as `removed false`, and
`shell_tool` / `unified_exec` as `stable true`. Feature presence does not prove
a hard allowlist. No feature changes or hook trust bypasses were performed.

## NOT POSSIBLE: narrow findings within this evidence

These findings concern the named mechanism/version or removed tool path;
they are separate from the NOT MEASURED role restrictions above.

| Mechanism / scope | Evidence-backed limitation | Evidence and alternative |
|---|---|---|
| Codex 0.163.0, `mcp_servers.<s>.approval_mode` | This key cannot enforce server approval: it is ignored. | Manager-reported T307 #2075 / T349: startup names ignored keys for git-mcp-rs, ferris-search, codebase-memory-mcp. Full warning transcript unavailable. Per-tool `tools.<t>.approval_mode` is measured (C4); `default_tools_approval_mode` is accepted, manager-reported. |
| Codex 0.163.0, per-tool `approval_mode="deny"` | `deny` is not an accepted variant. | T306 actual `mcp get` parser error: `unknown variant deny, expected one of auto, prompt, writes, approve` (complete invocation not supplied). This does not rule out `disabled_tools`; C7 accepts that filter key. |
| Codex coder, both shell tools removed | Cargo, suites and repo scripts cannot be invoked through those removed shell tools. | C3 / T307 observed `NO SHELL TOOL AVAILABLE`. Gate relocation is T307's work; this document does not assign builds to the read-only documentary reviewer. |
| Codex shell flags alone as a filesystem-write prohibition | Removing shell does not remove all file-write capability. | C5 / T349 manager-reported `apply_patch` creation of `probe_write.txt` is the counterexample. No workspace-only coder wall was established. |
| Codex 0.163.0, `request_rule` feature | Cannot be treated as an available supported feature. | T306 `codex features list`: `request_rule` is `removed false`. No feature enablement attempted. |

The initial supposition that some CLI categorically cannot deny individual MCP
tools remains unproven. No such impossible CLI/capability pair was established.
All other unanswered capabilities stay NOT MEASURED, with their blockers above.

## Documentary validation scope

T351 changes only this matrix and its README link. Review compares the newest
recorded diff with T306's plan, T307's full note/events, and T349's full report:
all three roles for all four CLIs, four command-allowlist rows, provenance and
available output for measured claims, and separate NOT POSSIBLE / NOT MEASURED
findings. No builds, suites, installs, configuration changes, or fresh live
CLI/service/sandbox probes belong to this documentation round.
