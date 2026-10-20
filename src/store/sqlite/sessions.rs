use super::render::ago;
use super::{fetch, now_ms, open, short};
use crate::store::proc;
use rusqlite::params;
use std::process::Command;

/// One registry row as `sessions` lists it. `cwd` is stored but not shown.
struct SessionRow {
    name: String,
    agent: String,
    task: Option<i64>,
    seen: i64,
}

/// Newest first; rows seen in the same millisecond come out by name (SQLite BINARY
/// collation: byte order), so the listing never depends on insertion order.
fn rows(conn: &rusqlite::Connection) -> Result<Vec<SessionRow>, String> {
    let mut stmt = conn
        .prepare("SELECT name,agent,task_id,seen FROM sessions ORDER BY seen DESC, name")
        .map_err(|e| e.to_string())?;
    stmt.query_map([], |r| {
        Ok(SessionRow {
            name: r.get(0)?,
            agent: r.get(1)?,
            task: r.get(2)?,
            seen: r.get(3)?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<rusqlite::Result<Vec<_>>>()
    .map_err(|e| e.to_string())
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
    fn herdr(&self) -> Option<&[HerdrAgent]> {
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

pub fn sessions() -> Result<String, String> {
    let conn = open()?;
    let names = rows(&conn)?;
    if names.is_empty() {
        return Ok("(no sessions registered)".to_owned());
    }
    let live = Liveness::new();
    let mut out = String::new();
    for SessionRow {
        name,
        agent,
        task,
        seen,
    } in names
    {
        let alive = live.alive(&name, &agent);
        out.push_str(&format!(
            "{name} {agent} {} {} {}\n",
            if alive { "up" } else { "gone" },
            task.map_or("-".to_owned(), short),
            ago(seen)
        ));
    }
    Ok(out.trim_end().to_owned())
}

/// Deletes rows whose pane is not alive. Rows never expire on their own, and a
/// dead row captures a dispatch to its name, so this is the supported cleanup.
/// Refuses when neither tmux nor herdr answers: every row would look dead.
pub fn prune(dry_run: bool) -> Result<String, String> {
    let conn = open()?;
    let all = rows(&conn)?;
    let live = Liveness::new();
    let herdr_up = live.herdr().is_some();
    if !herdr_up && !tmux_up() {
        return Err("prune: neither tmux nor herdr answers, so no session can be judged dead; start the session host, or remove one row with `agentboard unregister <name>`".to_owned());
    }
    let why = if herdr_up {
        "not a tmux session, not a herdr agent"
    } else {
        "not a tmux session (herdr unreachable: checked against tmux only)"
    };
    let (verb, done) = if dry_run {
        ("would remove", "would prune")
    } else {
        ("removed", "pruned")
    };
    let (mut out, mut dead, mut kept) = (String::new(), 0, 0);
    for r in &all {
        if live.alive(&r.name, &r.agent) {
            kept += 1;
            continue;
        }
        if !dry_run {
            conn.execute("DELETE FROM sessions WHERE name=?", params![r.name])
                .map_err(|e| e.to_string())?;
        }
        dead += 1;
        out.push_str(&format!("{verb} {} ({}): {why}\n", r.name, r.agent));
    }
    out.push_str(&if dead == 0 {
        format!("nothing to prune ({kept} live)")
    } else {
        format!("{done} {dead}, kept {kept}")
    });
    Ok(out)
}

pub fn unregister(name: &str) -> Result<String, String> {
    let conn = open()?;
    let n = conn
        .execute("DELETE FROM sessions WHERE name=?", params![name])
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Err(format!("no registered session {name:?}"));
    }
    Ok(format!("ok session {name} removed"))
}

pub fn register_session(
    name: &str,
    agent: &str,
    cwd: &str,
    task: Option<i64>,
) -> Result<String, String> {
    let conn = open()?;
    if let Some(t) = task {
        fetch(&conn, t)?;
    }
    conn.execute(
        "INSERT INTO sessions(name,agent,cwd,task_id,seen,status) VALUES(?,?,?,?,?,'up')
         ON CONFLICT(name) DO UPDATE SET agent=excluded.agent, cwd=excluded.cwd, task_id=excluded.task_id, seen=excluded.seen, status='up'",
        params![name, agent, cwd, task, now_ms()],
    )
    .map_err(|e| e.to_string())?;
    Ok(format!("ok session {name} up"))
}

fn tmux_alive(name: &str) -> bool {
    proc::run(Command::new("tmux").args(["has-session", "-t", name]))
        .is_ok_and(|o| o.status.success())
}

/// One entry of `herdr agent list`.
#[derive(Debug, PartialEq)]
struct HerdrAgent {
    name: String,
    kind: String,
    cwd: Option<String>,
    pane_id: Option<String>,
}

/// A tmux server is up and answering (a `has-session` miss alone cannot tell
/// "no such session" from "no tmux at all").
fn tmux_up() -> bool {
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
fn parse_herdr_agents(json: &[u8]) -> Vec<HerdrAgent> {
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
fn trim_cap_tail(text: &str, lines: usize) -> String {
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

#[cfg(test)]
mod tests {
    use super::{HerdrAgent, parse_herdr_agents, rows};

    #[test]
    fn rows_with_the_same_seen_list_by_name_in_byte_order() {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        c.execute_batch(crate::store::sqlite::SCHEMA).unwrap();
        // inserted out of order, all seen in the same millisecond, one newer row
        for name in ["mid", "émile", "alpha", "Zed"] {
            c.execute(
                "INSERT INTO sessions(name,agent,seen) VALUES(?,?,?)",
                rusqlite::params![name, "claude", 1000],
            )
            .unwrap();
        }
        c.execute(
            "INSERT INTO sessions(name,agent,seen) VALUES('newest','claude',2000)",
            [],
        )
        .unwrap();
        let got: Vec<String> = rows(&c).unwrap().into_iter().map(|r| r.name).collect();
        // newest first, then ties by byte order: uppercase before lowercase, non-ASCII last
        assert_eq!(got, ["newest", "Zed", "alpha", "mid", "émile"]);
    }

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
