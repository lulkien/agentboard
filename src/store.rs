//! Storage + operations. One SQLite file, WAL, short-immediate transactions.
//! Every operation opens its own connection: cheap, and safe under concurrency.

use rusqlite::{Connection, OptionalExtension, params};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const SCHEMA: &str = include_str!("schema.sql");
pub const DEFAULT_TTL_S: i64 = 1800;

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(0))
}

pub fn db_path() -> PathBuf {
    if let Ok(p) = std::env::var("AGENTBOARD_DB") {
        return PathBuf::from(p);
    }
    let base = std::env::var("XDG_STATE_HOME").map_or_else(
        |_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".local/state")
        },
        PathBuf::from,
    );
    base.join("agentboard/board.db")
}

fn open() -> Result<Connection, String> {
    let path = db_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("db dir {}: {e}", dir.display()))?;
    }
    let conn = Connection::open(&path).map_err(|e| format!("open {}: {e}", path.display()))?;
    conn.busy_timeout(Duration::from_millis(5000))
        .map_err(|e| e.to_string())?;
    // Per-connection and cheap. Never take a write lock here: `journal_mode=WAL`
    // and DDL on every open is what turns concurrent tool calls into
    // "database is locked".
    conn.execute_batch("PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;")
        .map_err(|e| e.to_string())?;
    let ready = INIT.get_or_init(|| {
        conn.execute_batch("PRAGMA journal_mode=WAL").map_err(|e| e.to_string())?;
        conn.execute_batch(SCHEMA).map_err(|e| format!("schema: {e}"))
    });
    ready.clone()?;
    Ok(conn)
}

/// Schema + WAL applied once per process; later opens are pure connection setup.
static INIT: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();

pub fn short(id: i64) -> String {
    format!("T{id}")
}

pub fn parse_id(raw: &str) -> Result<i64, String> {
    let t = raw.trim().trim_start_matches(['T', 't', '#']);
    t.parse::<i64>().map_err(|_| format!("bad task id {raw:?} (want T3 or 3)"))
}

fn code(status: &str) -> &str {
    match status {
        "triage" => "tri",
        "ready" => "rdy",
        "running" => "run",
        "blocked" => "blk",
        "review" => "rev",
        "done" => "don",
        "dropped" => "drp",
        other => other,
    }
}

fn ago(ts_ms: i64) -> String {
    let d = (now_ms() - ts_ms).max(0) / 1000;
    match d {
        0..=59 => format!("{d}s"),
        60..=3599 => format!("{}m", d / 60),
        3600..=86_399 => format!("{}h", d / 3600),
        _ => format!("{}d", d / 86_400),
    }
}

fn clip(s: &str, max: usize) -> String {
    let one_line = s.replace('\n', " ⏎ ");
    if one_line.chars().count() <= max {
        return one_line;
    }
    let cut: String = one_line.chars().take(max).collect();
    format!("{cut}…")
}

fn log_event(conn: &Connection, task: i64, actor: &str, kind: &str, body: &str) -> Result<i64, String> {
    conn.execute(
        "INSERT INTO events(task_id,actor,kind,body,ts) VALUES(?,?,?,?,?)",
        params![task, actor, kind, body, now_ms()],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

pub fn add(actor: &str, title: &str, spec: &str, assignee: Option<&str>, priority: i64, parent: Option<i64>) -> Result<String, String> {
    let conn = open()?;
    let now = now_ms();
    conn.execute(
        "INSERT INTO tasks(title,spec,assignee,priority,parent,created,updated) VALUES(?,?,?,?,?,?,?)",
        params![title, spec, assignee, priority, parent, now, now],
    )
    .map_err(|e| format!("add: {e}"))?;
    let id = conn.last_insert_rowid();
    log_event(&conn, id, actor, "add", title)?;
    Ok(format!("{} ready", short(id)))
}

struct Row {
    id: i64,
    status: String,
    assignee: Option<String>,
    needs: Option<String>,
    title: String,
    spec: String,
    lease_until: Option<i64>,
    updated: i64,
    result: Option<String>,
    commit_ref: Option<String>,
    priority: i64,
}

const COLS: &str = "id,status,assignee,needs,title,spec,lease_until,updated,result,commit_ref,priority";

fn read_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok(Row {
        id: r.get(0)?,
        status: r.get(1)?,
        assignee: r.get(2)?,
        needs: r.get(3)?,
        title: r.get(4)?,
        spec: r.get(5)?,
        lease_until: r.get(6)?,
        updated: r.get(7)?,
        result: r.get(8)?,
        commit_ref: r.get(9)?,
        priority: r.get(10)?,
    })
}

fn fetch(conn: &Connection, id: i64) -> Result<Row, String> {
    let sql = format!("SELECT {COLS} FROM tasks WHERE id=?");
    conn.query_row(&sql, params![id], |r| read_row(r))
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no such task {}", short(id)))
}

pub fn list(status: Option<&str>, assignee: Option<&str>, limit: usize) -> Result<String, String> {
    let conn = open()?;
    let mut sql = format!("SELECT {COLS} FROM tasks WHERE 1=1");
    let mut args: Vec<String> = Vec::new();
    if let Some(s) = status {
        sql.push_str(" AND status=?");
        args.push(s.to_owned());
    } else {
        sql.push_str(" AND status NOT IN ('done','dropped')");
    }
    if let Some(a) = assignee {
        sql.push_str(" AND assignee=?");
        args.push(a.to_owned());
    }
    sql.push_str(" ORDER BY CASE status WHEN 'running' THEN 0 WHEN 'blocked' THEN 1 WHEN 'review' THEN 2 WHEN 'triage' THEN 3 ELSE 4 END, priority, id LIMIT ?");
    args.push(limit.to_string());
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows: Vec<Row> = stmt
        .query_map(rusqlite::params_from_iter(args.iter()), |r| read_row(r))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    if rows.is_empty() {
        return Ok("(no tasks)".to_owned());
    }
    let mut out = String::new();
    for r in &rows {
        let who = r.assignee.as_deref().unwrap_or("-");
        let flag = if r.needs.as_deref() == Some("input") { "!input" } else { "" };
        out.push_str(&format!(
            "{} {} {:<8} {}{}\n",
            short(r.id),
            code(&r.status),
            who,
            clip(&r.title, 64),
            if flag.is_empty() { String::new() } else { format!(" [{flag}]") }
        ));
    }
    Ok(out.trim_end().to_owned())
}

pub fn get(id: i64) -> Result<String, String> {
    let conn = open()?;
    let r = fetch(&conn, id)?;
    let mut out = format!(
        "{} {} {} {}\n",
        short(r.id),
        r.status,
        r.assignee.as_deref().unwrap_or("-"),
        r.title
    );
    if !r.spec.is_empty() {
        out.push_str(&format!("spec: {}\n", clip(&r.spec, 1500)));
    }
    if let Some(u) = r.lease_until {
        let left = u - now_ms();
        if left > 0 {
            out.push_str(&format!("lease: {} left\n", ago(now_ms() - left)));
        }
    }
    if let Some(res) = &r.result {
        out.push_str(&format!("result: {}\n", clip(res, 500)));
    }
    if let Some(c) = &r.commit_ref {
        out.push_str(&format!("commit: {c}\n"));
    }
    let mut stmt = conn
        .prepare("SELECT seq,actor,kind,body,ts FROM events WHERE task_id=? ORDER BY seq DESC LIMIT 8")
        .map_err(|e| e.to_string())?;
    let evs: Vec<(i64, String, String, String, i64)> = stmt
        .query_map(params![r.id], |e| {
            Ok((e.get(0)?, e.get(1)?, e.get(2)?, e.get(3)?, e.get(4)?))
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    if !evs.is_empty() {
        out.push_str("events:\n");
        for (seq, actor, kind, body, ts) in evs.iter().rev() {
            out.push_str(&format!("  #{seq} {actor} {kind} {}: {}\n", ago(*ts), clip(body, 220)));
        }
    }
    Ok(out.trim_end().to_owned())
}

/// Claims a specific task, or the next ready one. Atomic: BEGIN IMMEDIATE + guarded UPDATE.
pub fn claim(actor: &str, id: Option<i64>, ttl_s: i64) -> Result<String, String> {
    let mut conn = open()?;
    let now = now_ms();
    let lease = now + ttl_s * 1000;
    for _ in 0..8 {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
        let cand: Option<(i64, String)> = {
            let (sql, arg): (String, Vec<i64>) = match id {
                Some(x) => (format!("SELECT id,status FROM tasks WHERE id=? AND (status IN ('ready','triage') OR (status='running' AND COALESCE(lease_until,0) < ?))"), vec![x, now]),
                None => ("SELECT id,status FROM tasks WHERE status IN ('ready','triage') ORDER BY priority, id LIMIT 1".to_owned(), vec![]),
            };
            let mut stmt = tx.prepare(&sql).map_err(|e| e.to_string())?;
            let found = stmt
                .query_row(rusqlite::params_from_iter(arg.iter()), |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })
                .optional()
                .map_err(|e| e.to_string())?;
            found
        };
        let Some((task, _st)) = cand else {
            tx.commit().map_err(|e| e.to_string())?;
            return match id {
                Some(x) => {
                    // Distinguish "does not exist" from "somebody is on it".
                    let conn2 = open()?;
                    match fetch(&conn2, x) {
                        Ok(r) => Ok(format!(
                            "{} {} by {} (lease), not claimable",
                            short(x),
                            r.status,
                            r.assignee.as_deref().unwrap_or("nobody")
                        )),
                        Err(_) => Err(format!("no such task {}", short(x))),
                    }
                }
                None => Ok("no ready tasks".to_owned()),
            };
        };
        let changed = tx
            .execute(
                "UPDATE tasks SET status='running', assignee=?, lease_until=?, updated=? WHERE id=? AND COALESCE(lease_until,0) < ?",
                params![actor, lease, now, task, lease],
            )
            .map_err(|e| e.to_string())?;
        if changed != 1 {
            tx.rollback().map_err(|e| e.to_string())?;
            continue;
        }
        log_event(&tx, task, actor, "claim", "")?;
        let r = fetch(&tx, task)?;
        tx.commit().map_err(|e| e.to_string())?;
        let mut out = format!("{} running (lease {}m)\n{}", short(r.id), ttl_s / 60, r.title);
        if !r.spec.is_empty() {
            out.push_str(&format!("\n{}", clip(&r.spec, 3000)));
        }
        return Ok(out.trim_end().to_owned());
    }
    Err("claim contention: retry".to_owned())
}

fn set_status(actor: &str, id: i64, status: &str, needs: Option<&str>, kind: &str, body: &str, result: Option<&str>, commit: Option<&str>) -> Result<String, String> {
    let conn = open()?;
    fetch(&conn, id)?;
    let now = now_ms();
    conn.execute(
        "UPDATE tasks SET status=?, needs=?, updated=?, lease_until=CASE WHEN ?='running' THEN lease_until ELSE NULL END, result=COALESCE(?, result), commit_ref=COALESCE(?, commit_ref) WHERE id=?",
        params![status, needs, now, status, result, commit, id],
    )
    .map_err(|e| e.to_string())?;
    let seq = log_event(&conn, id, actor, kind, body)?;
    Ok(format!("{} {status} #{seq}", short(id)))
}

pub fn note(actor: &str, id: i64, body: &str) -> Result<String, String> {
    let conn = open()?;
    fetch(&conn, id)?;
    let seq = log_event(&conn, id, actor, "note", body)?;
    Ok(format!("ok {} #{seq}", short(id)))
}

pub fn ask(actor: &str, id: i64, question: &str) -> Result<String, String> {
    set_status(actor, id, "blocked", Some("input"), "ask", question, None, None)
}

pub fn answer(actor: &str, id: i64, text: &str) -> Result<String, String> {
    let ack = set_status(actor, id, "running", None, "answer", text, None, None)?;
    // A resumed task gets a fresh lease, otherwise another agent can steal it.
    heartbeat(actor, id, DEFAULT_TTL_S)?;
    Ok(ack)
}

/// Arbitrary status moves for reopen/drop: ready|triage|dropped.
pub fn set_status_public(actor: &str, id: i64, status: &str, note: &str) -> Result<String, String> {
    match status {
        "ready" | "triage" | "dropped" => set_status(actor, id, status, None, "edit", note, None, None),
        other => Err(format!("cannot set status {other:?} here")),
    }
}

pub fn finish(actor: &str, id: i64, status: &str, summary: &str, commit: Option<&str>) -> Result<String, String> {
    let kind = if status == "done" { "done" } else { "review" };
    set_status(actor, id, status, None, kind, summary, Some(summary), commit)
}

pub fn block(actor: &str, id: i64, reason: &str) -> Result<String, String> {
    set_status(actor, id, "blocked", None, "block", reason, None, None)
}

pub fn heartbeat(_actor: &str, id: i64, ttl_s: i64) -> Result<String, String> {
    let conn = open()?;
    let lease = now_ms() + ttl_s * 1000;
    let n = conn
        .execute("UPDATE tasks SET lease_until=? WHERE id=? AND status='running'", params![lease, id])
        .map_err(|e| e.to_string())?;
    if n != 1 {
        return Err(format!("{} not running", short(id)));
    }
    Ok(format!("ok {} lease {}m", short(id), ttl_s / 60))
}

/// One-screen status: counts + one line per open task + recent completions.
pub fn brief(since_min: i64) -> Result<String, String> {
    let conn = open()?;
    let cutoff = now_ms() - since_min * 60_000;
    let mut out = String::new();
    let mut stmt = conn
        .prepare("SELECT status, COUNT(*) FROM tasks WHERE status NOT IN ('done','dropped') GROUP BY status ORDER BY 1")
        .map_err(|e| e.to_string())?;
    let counts: Vec<(String, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    let done: i64 = conn
        .query_row("SELECT COUNT(*) FROM tasks WHERE status='done' AND updated>=?", params![cutoff], |r| r.get(0))
        .unwrap_or(0);
    let mut head: Vec<String> = counts.iter().map(|(s, n)| format!("{n} {}", code(s))).collect();
    head.push(format!("{done} done/{}m", since_min));
    out.push_str(&head.join(", "));
    out.push('\n');
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {COLS} FROM tasks WHERE status NOT IN ('done','dropped') ORDER BY CASE status WHEN 'blocked' THEN 0 WHEN 'running' THEN 1 WHEN 'review' THEN 2 ELSE 3 END, priority, id LIMIT 25"
        ))
        .map_err(|e| e.to_string())?;
    let rows: Vec<Row> = stmt
        .query_map([], |r| read_row(r))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    for r in &rows {
        let wait = if r.needs.as_deref() == Some("input") { "ON INPUT" } else { "" };
        out.push_str(&format!(
            "{} {} {} {} {}{}\n",
            short(r.id),
            code(&r.status),
            r.assignee.as_deref().unwrap_or("-"),
            ago(r.updated),
            clip(&r.title, 60),
            if wait.is_empty() { String::new() } else { format!(" [{wait}]") }
        ));
    }
    let mut stmt = conn
        .prepare("SELECT id,result FROM tasks WHERE status='done' AND updated>=? ORDER BY updated DESC LIMIT 8")
        .map_err(|e| e.to_string())?;
    let dones: Vec<(i64, Option<String>)> = stmt
        .query_map(params![cutoff], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    if !dones.is_empty() {
        out.push_str("done:\n");
        for (id, res) in dones {
            out.push_str(&format!("  {} {}\n", short(id), clip(res.as_deref().unwrap_or(""), 80)));
        }
    }
    Ok(out.trim_end().to_owned())
}

/// Blocks server-side (no agent-side polling) until the task changes or the timeout hits.
///
/// `after` is the last event seq the caller already saw; without it we take the
/// current watermark, which is correct for "wait for the next thing to happen".
pub async fn wait(id: i64, after: Option<i64>, timeout_ms: u64) -> Result<String, String> {
    let start_ev: i64 = {
        let conn = open()?;
        fetch(&conn, id)?;
        match after {
            Some(seq) => seq,
            None => conn
                .query_row("SELECT COALESCE(MAX(seq),0) FROM events WHERE task_id=?", params![id], |r| r.get(0))
                .map_err(|e| e.to_string())?,
        }
    };
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        {
            let conn = open()?;
            let r = fetch(&conn, id)?;
            let mut stmt = conn
                .prepare("SELECT seq,actor,kind,body FROM events WHERE task_id=? AND seq>? ORDER BY seq")
                .map_err(|e| e.to_string())?;
            let evs: Vec<(i64, String, String, String)> = stmt
                .query_map(params![id, start_ev], |e| Ok((e.get(0)?, e.get(1)?, e.get(2)?, e.get(3)?)))
                .map_err(|e| e.to_string())?
                .filter_map(Result::ok)
                .collect();
            if !evs.is_empty() {
                let mut out = format!("{} {}\n", short(id), r.status);
                for (seq, actor, kind, body) in evs {
                    out.push_str(&format!("#{seq} {actor} {kind}: {}\n", clip(&body, 400)));
                }
                return Ok(out.trim_end().to_owned());
            }
            if std::time::Instant::now() >= deadline {
                // Never return blind: hand back the tail so the caller can decide.
                return Ok(format!(
                    "{} {} unchanged after {}s\nlast: {}",
                    short(id),
                    r.status,
                    timeout_ms / 1000,
                    clip(&get(id).unwrap_or_default(), 400)
                ));
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

pub fn sessions() -> Result<String, String> {
    let conn = open()?;
    let names: Vec<(String, String, Option<String>, Option<i64>, i64)> = {
        let mut stmt = conn
            .prepare("SELECT name,agent,cwd,task_id,seen FROM sessions ORDER BY seen DESC")
            .map_err(|e| e.to_string())?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect()
    };
    if names.is_empty() {
        return Ok("(no sessions registered)".to_owned());
    }
    let mut out = String::new();
    for (name, agent, _cwd, task, seen) in names {
        let alive = tmux_alive(&name);
        out.push_str(&format!(
            "{name} {agent} {} {} {}\n",
            if alive { "up" } else { "gone" },
            task.map_or("-".to_owned(), short),
            ago(seen)
        ));
    }
    Ok(out.trim_end().to_owned())
}

pub fn register_session(name: &str, agent: &str, cwd: &str, task: Option<i64>) -> Result<String, String> {
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
    std::process::Command::new("tmux")
        .args(["has-session", "-t", name])
        .status()
        .is_ok_and(|s| s.success())
}

/// Last N lines of a tmux pane, trimmed and capped: the cheap way to see what an
/// agent is doing without asking it for a status report.
pub fn tail(name: &str, lines: usize) -> Result<String, String> {
    let out = std::process::Command::new("tmux")
        .args(["capture-pane", "-p", "-t", name, "-S", &format!("-{lines}")])
        .output()
        .map_err(|e| format!("tmux: {e}"))?;
    if !out.status.success() {
        return Err(format!("tmux: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut rows: Vec<&str> = text.lines().map(str::trim_end).filter(|l| !l.is_empty()).collect();
    while rows.len() > lines {
        rows.remove(0);
    }
    let joined = rows.join("\n");
    Ok(if joined.len() > 6000 { joined[joined.len() - 6000..].to_owned() } else { joined })
}

pub fn send(name: &str, text: &str) -> Result<String, String> {
    let st = std::process::Command::new("tmux")
        .args(["send-keys", "-t", name, text, "Enter"])
        .status()
        .map_err(|e| format!("tmux: {e}"))?;
    if !st.success() {
        return Err(format!("tmux send-keys failed for {name}"));
    }
    Ok(format!("ok sent {} bytes to {name}", text.len()))
}
