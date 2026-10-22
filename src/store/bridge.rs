//! The pane bridges and the pure dispatch logic, shared by both backends: which
//! tmux sessions and herdr agents are alive, reading and typing into a pane, and
//! how a dispatch picks its target and words its refusals. Nothing here touches a
//! board database; the stores load the rows and pass plain values in.

use crate::clip::clip;
use crate::store::proc;
use crate::store::short;
use std::process::Command;

/// Authorization facts loaded under the backend's dispatch lock. Assignee is
/// deliberately absent: only a recorded claim grants a seat authority.
pub struct DispatchChain {
    pub root_reporter: String,
    pub depth: usize,
    pub new_chain: bool,
    pub parent_claim: Option<(i64, String)>,
    pub children: usize,
    pub dispatched: Option<i64>,
}

pub fn dispatch_refusal(id: i64, target: Option<&str>, reason: &str) -> String {
    format!(
        "dispatch {} to {} refused: {reason}; allowed: root reporter: any row/target; analyze claimant -> implement child to coder; implement claimant -> review child to reviewer; legacy implement claimant -> review child to reviewer; legacy review claimant -> fix child to coder",
        short(id),
        target.unwrap_or("(unspecified)")
    )
}

pub fn dispatch_authority(
    actor: &str,
    id: i64,
    target: Option<&str>,
    chain: &DispatchChain,
) -> Result<Option<&'static str>, String> {
    if !actor.trim().is_empty() && actor == chain.root_reporter {
        return Ok(None);
    }
    let refusal = |reason: &str| dispatch_refusal(id, target, reason);
    let expected = match (chain.new_chain, chain.depth) {
        (true, 2) | (false, 3) => "coder",
        (true, 3) | (false, 2) => "reviewer",
        _ => return Err(refusal("outside the next-seat mapping")),
    };
    let Some((claim, holder)) = &chain.parent_claim else {
        return Err(refusal("parent has no recorded claim"));
    };
    if actor.trim().is_empty() || actor != holder {
        return Err(refusal("actor did not claim the parent"));
    }
    if target.is_some_and(|target| target != expected) {
        return Err(refusal("wrong next-seat target"));
    }
    if chain.children != 1 || chain.dispatched.is_some_and(|seq| seq > *claim) {
        return Err(refusal(
            "handoff already dispatched or chain has multiple children",
        ));
    }
    Ok(Some(expected))
}

/// Which panes answer right now. A row is only a claim that a pane exists; this
/// is the check. tmux is asked per name, herdr once, and only if tmux cannot
/// vouch for a name.
pub struct Liveness {
    herdr: std::cell::OnceCell<Option<Vec<HerdrAgent>>>,
}

impl Liveness {
    pub fn new() -> Self {
        Self {
            herdr: std::cell::OnceCell::new(),
        }
    }

    /// `None` when herdr cannot answer (missing, hung, bad exit), `Some(empty)`
    /// when it answers with no agents: the difference decides whether a prune is safe.
    pub fn herdr(&self) -> Option<&[HerdrAgent]> {
        self.herdr.get_or_init(herdr_agents).as_deref()
    }

    /// The working directory herdr reports for a named agent. tmux reports none,
    /// and an unreachable herdr or an agent without a cwd is "unknown", never a
    /// reason to warn.
    pub fn cwd(&self, name: &str) -> Option<String> {
        self.herdr()?
            .iter()
            .find(|h| h.name == name)
            .and_then(|h| h.cwd.clone())
    }

    /// The pane_id herdr reports for a named agent. `None` if herdr unreachable,
    /// the agent doesn't exist, or the agent has no pane_id.
    pub fn pane_id(&self, name: &str) -> Option<String> {
        self.herdr()?
            .iter()
            .find(|h| h.name == name)
            .and_then(|h| h.pane_id.clone())
    }

    pub fn alive(&self, name: &str, agent: &str) -> bool {
        tmux_alive(name)
            || self
                .herdr()
                .is_some_and(|a| a.iter().any(|h| h.name == name && h.kind == agent))
    }
}

pub fn tmux_alive(name: &str) -> bool {
    proc::run(Command::new("tmux").args(["has-session", "-t", name]))
        .is_ok_and(|o| o.status.success())
}

/// One entry of `herdr agent list`.
#[derive(Debug, PartialEq)]
pub struct HerdrAgent {
    pub name: String,
    pub kind: String,
    pub cwd: Option<String>,
    pub pane_id: Option<String>,
}

/// A tmux server is up and answering (a `has-session` miss alone cannot tell
/// "no such session" from "no tmux at all").
pub fn tmux_up() -> bool {
    proc::run(Command::new("tmux").arg("list-sessions")).is_ok_and(|o| o.status.success())
}

/// Live herdr agents as (name, agent type). `None`: herdr did not answer (missing
/// binary, hang, bad exit). Bad JSON is an answer with no agents. Bounded by the
/// shared runner (deadline, group kill).
fn herdr_agents() -> Option<Vec<HerdrAgent>> {
    match proc::run(Command::new("herdr").args(["agent", "list"])) {
        Ok(o) if o.status.success() => Some(parse_herdr_agents(&o.stdout)),
        _ => None,
    }
}

/// Parses the `herdr agent list` envelope (`.result.agents[]`). Agents without a
/// `name` (unnamed panes) are skipped.
pub(crate) fn parse_herdr_agents(json: &[u8]) -> Vec<HerdrAgent> {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(json) else {
        return Vec::new();
    };
    v["result"]["agents"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    Some(HerdrAgent {
                        name: x["name"].as_str()?.to_owned(),
                        kind: x["agent"].as_str().unwrap_or_default().to_owned(),
                        cwd: x["cwd"].as_str().map(str::to_owned),
                        pane_id: x["pane_id"].as_str().map(str::to_owned),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Trim, drop empty, keep newest N lines, cap to 6000 bytes at a char boundary.
/// Used by both tmux and herdr tail paths so they cannot drift.
pub(crate) fn trim_cap_tail(text: &str, lines: usize) -> String {
    let mut rows: Vec<&str> = text
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .collect();
    while rows.len() > lines {
        rows.remove(0);
    }
    let joined = rows.join("\n");
    if joined.len() <= 6000 {
        return joined;
    }
    // Keep the newest 6000 bytes, moving the cut forward to a char boundary:
    // TUI panes are full of multibyte box-drawing characters.
    let mut cut = joined.len() - 6000;
    while !joined.is_char_boundary(cut) {
        cut += 1;
    }
    joined[cut..].to_owned()
}

/// Last N lines of a pane. Tries herdr first (if the name is a live herdr agent
/// with a pane_id), then falls back to tmux. Errors name both bridges.
pub fn tail(name: &str, lines: usize) -> Result<String, String> {
    let live = Liveness::new();

    // herdr path: only if the name is a live herdr agent with a pane_id.
    if let Some(pane_id) = live.pane_id(name) {
        let out = proc::run(Command::new("herdr").args([
            "pane",
            "read",
            &pane_id,
            "--lines",
            &lines.to_string(),
        ]))
        .map_err(|e| format!("herdr: {e}"))?;
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            return Ok(trim_cap_tail(&text, lines));
        }
        let herdr_err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        // Fall through to tmux, but remember the herdr error.
        let tmux_result = try_tmux_tail(name, lines);
        match tmux_result {
            Ok(msg) => return Ok(msg),
            Err(e) => return Err(format!("no pane {name}: herdr: {herdr_err}, tmux: {e}")),
        }
    }

    // No herdr pane_id for this name. Try tmux, but include herdr in the error
    // if it also fails, so the user sees both bridges were tried.
    let tmux_result = try_tmux_tail(name, lines);
    match tmux_result {
        Ok(msg) => Ok(msg),
        Err(e) => Err(format!(
            "no pane {name}: herdr: not a herdr agent with pane_id, tmux: {e}"
        )),
    }
}

fn try_tmux_tail(name: &str, lines: usize) -> Result<String, String> {
    let out = proc::run(Command::new("tmux").args([
        "capture-pane",
        "-p",
        "-t",
        name,
        "-S",
        &format!("-{lines}"),
    ]))
    .map_err(|e| format!("tmux: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        return Err(format!("tmux: {err}"));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(trim_cap_tail(&text, lines))
}

/// Send text to a pane. Tries herdr first (if the name is a live herdr agent
/// with a pane_id), then falls back to tmux. Errors name both bridges.
/// Priority: herdr wins when a name is both a tmux session and a herdr agent.
pub fn send(name: &str, text: &str) -> Result<String, String> {
    let live = Liveness::new();

    // herdr path: only if the name is a live herdr agent with a pane_id.
    if let Some(pane_id) = live.pane_id(name) {
        // herdr pane send-text <pane_id> <text>
        let out = proc::run(Command::new("herdr").args(["pane", "send-text", &pane_id, text]))
            .map_err(|e| format!("herdr: {e}"))?;
        if !out.status.success() {
            let herdr_err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            // Fall through to tmux, but remember the herdr error.
            let tmux_result = try_tmux_send(name, text);
            match tmux_result {
                Ok(msg) => return Ok(msg),
                Err(e) => return Err(format!("no pane {name}: herdr: {herdr_err}, tmux: {e}")),
            }
        }
        // herdr pane send-keys <pane_id> enter
        let out = proc::run(Command::new("herdr").args(["pane", "send-keys", &pane_id, "enter"]))
            .map_err(|e| format!("herdr: {e}"))?;
        if !out.status.success() {
            let herdr_err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            let tmux_result = try_tmux_send(name, text);
            match tmux_result {
                Ok(msg) => return Ok(msg),
                Err(e) => return Err(format!("no pane {name}: herdr: {herdr_err}, tmux: {e}")),
            }
        }
        return Ok(format!("ok sent {} bytes to {name}", text.len()));
    }

    // No herdr pane_id for this name. Try tmux, but include herdr in the error
    // if it also fails, so the user sees both bridges were tried.
    let tmux_result = try_tmux_send(name, text);
    match tmux_result {
        Ok(msg) => Ok(msg),
        Err(e) => Err(format!(
            "no pane {name}: herdr: not a herdr agent with pane_id, tmux: {e}"
        )),
    }
}

fn try_tmux_send(name: &str, text: &str) -> Result<String, String> {
    // `-l --` types the text literally: without `--` a line starting with "-"
    // is parsed as a tmux option, and without `-l` words like "Up" are key names.
    for keys in [&["-l", "--", text][..], &["Enter"][..]] {
        let out = proc::run(
            Command::new("tmux")
                .args(["send-keys", "-t", name])
                .args(keys),
        )
        .map_err(|e| format!("tmux: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            return Err(format!("tmux send-keys failed for {name}: {err}"));
        }
    }
    Ok(format!("ok sent {} bytes to {name}", text.len()))
}

// ---- dispatch ---------------------------------------------------------------------

/// The line a worker pane receives. Fixed wording: the pane uses MCP tools.
pub fn line(id: i64) -> String {
    format!(
        "Use the agentboard MCP tools: claim task {} with the claim tool, read it with the get tool (full=true), then execute it. Do not shell out to the agentboard binary for board writes.",
        short(id)
    )
}

/// Runs `herdr agent prompt` under the shared deadline. `Err` is the one-line
/// reason herdr did not deliver: spawn failure, bad exit, or timeout.
fn herdr_prompt(name: &str, text: &str) -> Result<(), String> {
    let out = proc::run(Command::new("herdr").args(["agent", "prompt", name, text]))?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stderr = stderr.trim();
    Err(if stderr.is_empty() {
        out.status.to_string()
    } else {
        clip(stderr, 160)
    })
}

/// Submits `text` to a registered pane: herdr first, tmux as fallback. Returns
/// the path that worked; a tmux win names why herdr did not, so the event and the
/// printed line explain the fallback.
pub fn deliver(name: &str, text: &str) -> Result<String, String> {
    let herdr_err = match herdr_prompt(name, text) {
        Ok(()) => return Ok("herdr".to_owned()),
        Err(reason) => reason,
    };
    send(name, text)
        .map(|_| format!("tmux (herdr: {herdr_err})"))
        .map_err(|tmux_err| format!("dispatch to {name} failed (herdr: {herdr_err}; {tmux_err})"))
}

/// What a session name is worth as a dispatch target.
#[derive(Clone, Copy)]
pub enum Candidate {
    /// No row: never registered.
    Missing,
    /// A row exists but no pane answers to the name.
    Stale,
    Live,
}

impl Candidate {
    /// `agent` is the registered agent type for `name`, or `None` without a row.
    pub fn of(agent: Option<&str>, live: &Liveness, name: &str) -> Self {
        match agent {
            None => Self::Missing,
            Some(a) if live.alive(name, a) => Self::Live,
            Some(_) => Self::Stale,
        }
    }
}

/// Picks the pane to prompt: the task's assignee if its pane is live, else
/// `--to` if live. A registry row alone proves nothing (rows never expire), so a
/// dead assignee row must not capture the dispatch.
pub fn resolve(
    assignee: Option<(&str, Candidate)>,
    to: Option<(&str, Candidate)>,
    id: i64,
) -> Result<String, String> {
    if let Some((n, Candidate::Live)) = &assignee {
        return Ok((*n).to_owned());
    }
    if let Some((n, Candidate::Live)) = &to {
        return Ok((*n).to_owned());
    }
    let mut why = Vec::new();
    if let Some((n, Candidate::Stale)) = &assignee {
        why.push(format!(
            "{} has assignee {n:?} but that session row is stale: no live pane answers to it (`agentboard sessions prune` removes dead rows)",
            short(id)
        ));
    }
    match &to {
        Some((n, Candidate::Stale)) => why.push(format!(
            "session {n:?} is registered but not live (stale row; `agentboard sessions prune`)"
        )),
        Some((n, _)) => why.push(format!(
            "no registered session {n:?} (agentboard register <name>)"
        )),
        None if why.is_empty() => why.push(format!(
            "{} has no registered assignee; pass --to <session>",
            short(id)
        )),
        None => why.push("pass --to <live session>".to_owned()),
    }
    Err(why.join("; "))
}

/// The refusal for dispatching a task that is not `ready` without `--force`.
pub fn not_ready_text(
    id: i64,
    status: &str,
    assignee: Option<&str>,
    lease_until: Option<i64>,
) -> String {
    let lease = lease_until.map_or("none".to_owned(), |l| {
        format!(
            "{}m left, until {}Z",
            (l - crate::store::now_ms()).max(0) / 60_000,
            crate::store::text::stamp(l)
        )
    });
    format!(
        "{} is {status}, not ready (assignee {}, lease {lease}); --force resets it to ready first",
        short(id),
        assignee.unwrap_or("nobody")
    )
}

/// Warn, never refuse: only when the epic names a path and the pane reports a
/// cwd, and the cwd is not that path or under it.
pub fn epic_warning(
    id: i64,
    epic_id: i64,
    epic_path: &str,
    target: &str,
    cwd: Option<String>,
) -> Option<String> {
    cwd.filter(|cwd| !epic_path.is_empty() && !std::path::Path::new(cwd).starts_with(epic_path))
        .map(|cwd| {
            format!(
                "warning: {} belongs to epic {} rooted at {epic_path}, but session {target:?} reports cwd {cwd} (outside it); dispatching anyway\n",
                short(id),
                crate::store::short_epic(epic_id)
            )
        })
}

#[cfg(test)]
mod tests {
    use super::{HerdrAgent, parse_herdr_agents};

    #[test]
    fn parses_herdr_agent_list_and_skips_unnamed() {
        let sample = br#"{"id":"cli:agent:list","result":{"agents":[{"agent":"hermes","pane_id":"wB:p1"},{"agent":"claude","name":"coder","cwd":"/work/app","pane_id":"wB:p5"},{"agent":"codex","name":"nocwd"}]},"type":"agent_list"}"#;
        assert_eq!(
            parse_herdr_agents(sample),
            vec![
                HerdrAgent {
                    name: "coder".to_owned(),
                    kind: "claude".to_owned(),
                    cwd: Some("/work/app".to_owned()),
                    pane_id: Some("wB:p5".to_owned()),
                },
                HerdrAgent {
                    name: "nocwd".to_owned(),
                    kind: "codex".to_owned(),
                    cwd: None,
                    pane_id: None,
                },
            ]
        );
        assert!(parse_herdr_agents(b"not json").is_empty());
        assert!(parse_herdr_agents(b"{}").is_empty());
    }

    #[test]
    fn parses_herdr_agent_list_with_pane_id_and_skips_unnamed() {
        // Agent without name is skipped, agent without pane_id has None
        let sample = br#"{"result":{"agents":[{"agent":"claude","name":"coder","pane_id":"w1:p1"},{"agent":"codex","name":"reviewer"}]}}"#;
        let agents = parse_herdr_agents(sample);
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].name, "coder");
        assert_eq!(agents[0].pane_id, Some("w1:p1".to_owned()));
        assert_eq!(agents[1].name, "reviewer");
        assert_eq!(agents[1].pane_id, None);
    }

    #[test]
    fn trim_cap_tail_keeps_newest_lines_and_caps_bytes() {
        use super::trim_cap_tail;
        // 10 lines, keep 5
        let text =
            "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10\n";
        let out = trim_cap_tail(text, 5);
        assert_eq!(out.lines().count(), 5);
        assert!(out.starts_with("line 6"));
        assert!(out.ends_with("line 10"));
        // Cap test
        let long = "x".repeat(7000);
        let out = trim_cap_tail(&long, 10);
        assert_eq!(out.len(), 6000);
    }
}
