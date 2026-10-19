use super::proc;
use super::render::ago;
use super::{fetch, now_ms, open, short};
use rusqlite::params;
use std::process::Command;

/// One registry row as `sessions` lists it. `cwd` is stored but not shown.
struct SessionRow {
    name: String,
    agent: String,
    task: Option<i64>,
    seen: i64,
}

pub fn sessions() -> Result<String, String> {
    let conn = open()?;
    let names: Vec<SessionRow> = {
        let mut stmt = conn
            .prepare("SELECT name,agent,task_id,seen FROM sessions ORDER BY seen DESC")
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
        .map_err(|e| e.to_string())?
    };
    if names.is_empty() {
        return Ok("(no sessions registered)".to_owned());
    }
    let herdr = herdr_agents();
    let mut out = String::new();
    for SessionRow {
        name,
        agent,
        task,
        seen,
    } in names
    {
        let alive = tmux_alive(&name) || herdr.iter().any(|(n, a)| *n == name && *a == agent);
        out.push_str(&format!(
            "{name} {agent} {} {} {}\n",
            if alive { "up" } else { "gone" },
            task.map_or("-".to_owned(), short),
            ago(seen)
        ));
    }
    Ok(out.trim_end().to_owned())
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

/// Live herdr agents as (name, agent type). Missing binary, hang, bad exit or bad
/// JSON all mean "none". Bounded by the shared runner (deadline, group kill).
fn herdr_agents() -> Vec<(String, String)> {
    match proc::run(Command::new("herdr").args(["agent", "list"])) {
        Ok(o) if o.status.success() => parse_herdr_agents(&o.stdout),
        _ => Vec::new(),
    }
}

/// Parses the `herdr agent list` envelope (`.result.agents[]`). Agents without a
/// `name` (unnamed panes) are skipped.
fn parse_herdr_agents(json: &[u8]) -> Vec<(String, String)> {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(json) else {
        return Vec::new();
    };
    v["result"]["agents"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    Some((
                        x["name"].as_str()?.to_owned(),
                        x["agent"].as_str().unwrap_or_default().to_owned(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Last N lines of a tmux pane, trimmed and capped: the cheap way to see what an
/// agent is doing without asking it for a status report.
pub fn tail(name: &str, lines: usize) -> Result<String, String> {
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
        return Err(format!(
            "tmux: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
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
        return Ok(joined);
    }
    // Keep the newest 6000 bytes, moving the cut forward to a char boundary:
    // TUI panes are full of multibyte box-drawing characters.
    let mut cut = joined.len() - 6000;
    while !joined.is_char_boundary(cut) {
        cut += 1;
    }
    Ok(joined[cut..].to_owned())
}

pub fn send(name: &str, text: &str) -> Result<String, String> {
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
            return Err(format!(
                "tmux send-keys failed for {name}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }
    Ok(format!("ok sent {} bytes to {name}", text.len()))
}

#[cfg(test)]
mod tests {
    use super::parse_herdr_agents;

    #[test]
    fn parses_herdr_agent_list_and_skips_unnamed() {
        let sample = br#"{"id":"cli:agent:list","result":{"agents":[{"agent":"hermes","pane_id":"wB:p1"},{"agent":"claude","name":"coder","pane_id":"wB:p5"}],"type":"agent_list"}}"#;
        assert_eq!(
            parse_herdr_agents(sample),
            vec![("coder".to_owned(), "claude".to_owned())]
        );
        assert!(parse_herdr_agents(b"not json").is_empty());
        assert!(parse_herdr_agents(b"{}").is_empty());
    }
}
