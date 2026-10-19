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
        conn.execute_batch(SCHEMA).map_err(|e| format!("schema: {e}"))?;
        migrate(&conn)
    });
    ready.clone()?;
    Ok(conn)
}

/// Bring a v1 database (title/spec/status/result/created/updated) up to v2
/// (description/goal/report/note + created_at/started_at/completed_at).
fn migrate(conn: &Connection) -> Result<(), String> {
    let cols: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(tasks)").map_err(|e| e.to_string())?;
        stmt.query_map([], |r| r.get::<_, String>(1))
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect()
    };
    if cols.iter().any(|c| c == "description") {
        return Ok(());
    }
    conn.execute_batch(
        "BEGIN IMMEDIATE;
         ALTER TABLE tasks ADD COLUMN description TEXT NOT NULL DEFAULT '';
         ALTER TABLE tasks ADD COLUMN goal TEXT NOT NULL DEFAULT '';
         ALTER TABLE tasks ADD COLUMN report TEXT;
         ALTER TABLE tasks ADD COLUMN note TEXT;
         ALTER TABLE tasks ADD COLUMN created_at INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE tasks ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE tasks ADD COLUMN started_at INTEGER;
         ALTER TABLE tasks ADD COLUMN completed_at INTEGER;
         UPDATE tasks SET description=spec, goal=spec, report=result,
                          created_at=created, updated_at=updated;
         UPDATE tasks SET status=CASE status WHEN 'running' THEN 'working'
                              WHEN 'blocked' THEN 'blocking'
                              WHEN 'triage' THEN 'ready' ELSE status END;
         ALTER TABLE tasks DROP COLUMN spec;
         ALTER TABLE tasks DROP COLUMN result;
         ALTER TABLE tasks DROP COLUMN created;
         ALTER TABLE tasks DROP COLUMN updated;
         COMMIT;",
    )
    .map_err(|e| format!("migrate v1->v2: {e}"))
}

/// Canonical statuses. Old names stay accepted as aliases so in-flight agents
/// that still say running/blocked keep working.
pub fn norm(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "triage" | "ready" => Some("ready"),
        "running" | "working" | "started" => Some("working"),
        "blocked" | "blocking" => Some("blocking"),
        "review" => Some("review"),
        "done" => Some("done"),
        "dropped" => Some("dropped"),
        _ => None,
    }
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
        "ready" | "triage" => "rdy",
        "working" | "running" => "wrk",
        "blocking" | "blocked" => "blk",
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

pub fn add(actor: &str, title: &str, description: &str, goal: &str, status: &str, assignee: Option<&str>, priority: i64, parent: Option<i64>) -> Result<String, String> {
    let conn = open()?;
    let now = now_ms();
    conn.execute(
        "INSERT INTO tasks(title,description,goal,status,assignee,priority,parent,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)",
        params![title, description, goal, status, assignee, priority, parent, now, now],
    )
    .map_err(|e| format!("add: {e}"))?;
    let id = conn.last_insert_rowid();
    log_event(&conn, id, actor, "add", title)?;
    Ok(format!("{} {status}", short(id)))
}

struct Row {
    id: i64,
    status: String,
    assignee: Option<String>,
    needs: Option<String>,
    title: String,
    description: String,
    goal: String,
    lease_until: Option<i64>,
    updated_at: i64,
    created_at: i64,
    started_at: Option<i64>,
    completed_at: Option<i64>,
    report: Option<String>,
    note: Option<String>,
    commit_ref: Option<String>,
    priority: i64,
    parent: Option<i64>,
}

const COLS: &str = "id,status,assignee,needs,title,description,goal,lease_until,updated_at,created_at,started_at,completed_at,report,note,commit_ref,priority,parent";

fn read_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok(Row {
        id: r.get(0)?,
        status: r.get(1)?,
        assignee: r.get(2)?,
        needs: r.get(3)?,
        title: r.get(4)?,
        description: r.get(5)?,
        goal: r.get(6)?,
        lease_until: r.get(7)?,
        updated_at: r.get(8)?,
        created_at: r.get(9)?,
        started_at: r.get(10)?,
        completed_at: r.get(11)?,
        report: r.get(12)?,
        note: r.get(13)?,
        commit_ref: r.get(14)?,
        priority: r.get(15)?,
        parent: r.get(16)?,
    })
}

/// One-line timestamp for listings: "13:32Z" style is locale-safe enough here.
fn stamp(ts_ms: i64) -> String {
    let secs = ts_ms / 1000;
    let (h, m) = ((secs / 3600) % 24, (secs / 60) % 60);
    format!("{h:02}:{m:02}")
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
    match status {
        Some(s) => {
            let want = norm(s).ok_or_else(|| format!("bad status {s:?} (ready|working|blocking|review|done|dropped)"))?;
            sql.push_str(" AND status=?");
            args.push(want.to_owned());
        }
        None => sql.push_str(" AND status NOT IN ('done','dropped')"),
    }
    if let Some(a) = assignee {
        sql.push_str(" AND assignee=?");
        args.push(a.to_owned());
    }
    sql.push_str(" ORDER BY CASE status WHEN 'blocking' THEN 0 WHEN 'working' THEN 1 WHEN 'review' THEN 2 WHEN 'ready' THEN 3 ELSE 4 END, priority, id LIMIT ?");
    args.push(limit.to_string());
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows: Vec<Row> = stmt
        .query_map(rusqlite::params_from_iter(args.iter()), |r| read_row(r))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<Row>>>()
        .map_err(|e| e.to_string())?;
    if rows.is_empty() {
        return Ok("(no tasks)".to_owned());
    }
    let mut out = String::new();
    for r in &rows {
        let who = r.assignee.as_deref().unwrap_or("-");
        let flag = if r.needs.as_deref() == Some("input") { " [input]" } else { "" };
        out.push_str(&format!(
            "{} {} {:<9} {:>4} {}{}\n",
            short(r.id),
            code(&r.status),
            who,
            ago(r.updated_at),
            clip(&r.title, 64),
            flag
        ));
    }
    Ok(out.trim_end().to_owned())
}

/// Fills in any caller-supplied long field, or "-" when absent.
fn field(label: &str, value: &str, max: usize) -> String {
    if value.is_empty() {
        String::new()
    } else {
        format!("{label}:\n{}\n", clip(value, max))
    }
}

fn when(ts: i64) -> String {
    format!("{} ({}Z)", ago(ts), stamp(ts))
}

pub fn get(id: i64) -> Result<String, String> {
    let conn = open()?;
    let r = fetch(&conn, id)?;
    let mut out = format!(
        "{} {} {} priority {}\n",
        short(r.id),
        r.status,
        r.assignee.as_deref().unwrap_or("-"),
        r.priority
    );
    out.push_str(&format!("title: {}\n", r.title));
    if let Some(p) = r.parent {
        out.push_str(&format!("parent: {}\n", short(p)));
    }
    out.push_str(&format!("created: {}\n", when(r.created_at)));
    if let Some(s) = r.started_at {
        out.push_str(&format!("started: {}\n", when(s)));
    }
    if let Some(c) = r.completed_at {
        out.push_str(&format!("completed: {}\n", when(c)));
    }
    if let Some(u) = r.lease_until {
        let left = u - now_ms();
        if left > 0 {
            out.push_str(&format!("lease: {}m left\n", left / 60_000));
        }
    }
    out.push_str(&field("description", &r.description, 4000));
    out.push_str(&field("goal", &r.goal, 2000));
    out.push_str(&field("report", r.report.as_deref().unwrap_or(""), 6000));
    out.push_str(&field("note", r.note.as_deref().unwrap_or(""), 800));
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
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
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
                Some(x) => (format!("SELECT id,status FROM tasks WHERE id=? AND (status='ready' OR (lease_until IS NOT NULL AND lease_until < ?))"), vec![x, now]),
                None => ("SELECT id,status FROM tasks WHERE status='ready' ORDER BY priority, id LIMIT 1".to_owned(), vec![]),
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
        // The guard compares the OLD lease against `now`; comparing it against
        // the new lease would let a live claim be stolen.
        let changed = tx
            .execute(
                "UPDATE tasks SET status='working', assignee=?, lease_until=?, updated_at=?, started_at=COALESCE(started_at,?) WHERE id=? AND (status='ready' OR (lease_until IS NOT NULL AND lease_until < ?))",
                params![actor, lease, now, now, task, now],
            )
            .map_err(|e| e.to_string())?;
        if changed != 1 {
            tx.rollback().map_err(|e| e.to_string())?;
            continue;
        }
        log_event(&tx, task, actor, "claim", "")?;
        let r = fetch(&tx, task)?;
        tx.commit().map_err(|e| e.to_string())?;
        let mut out = format!("{} working (lease {}m) {}\n", short(r.id), ttl_s / 60, r.title);
        if !r.description.is_empty() {
            out.push_str(&format!("description:\n{}\n", clip(&r.description, 4000)));
        }
        if !r.goal.is_empty() {
            out.push_str(&format!("goal:\n{}\n", clip(&r.goal, 2500)));
        }
        return Ok(out.trim_end().to_owned());
    }
    Err("claim contention: retry".to_owned())
}

/// One transactional edit of a task. Status moves, report, note, commit and
/// lease all land together or not at all.
pub struct Patch<'a> {
    pub status: Option<&'a str>,
    pub report: Option<&'a str>,
    pub note: Option<&'a str>,
    pub commit: Option<&'a str>,
    pub needs: Option<&'a str>,
    pub ttl_s: i64,
}

pub fn apply(actor: &str, id: i64, p: &Patch<'_>) -> Result<String, String> {
    let mut conn = open()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
    let cur = fetch(&tx, id)?;
    let now = now_ms();
    let status = match p.status {
        Some(raw) => norm(raw).ok_or_else(|| format!("bad status {raw:?} (ready|working|blocking|review|done|dropped)"))?,
        None => cur.status.as_str(),
    };
    let moved = status != cur.status;
    if let Some(rp) = p.report {
        tx.execute("UPDATE tasks SET report=? WHERE id=?", params![rp, id]).map_err(|e| e.to_string())?;
    }
    if let Some(n) = p.note.filter(|n| !n.trim().is_empty()) {
        tx.execute("UPDATE tasks SET note=? WHERE id=?", params![n, id]).map_err(|e| e.to_string())?;
    }
    if let Some(c) = p.commit {
        tx.execute("UPDATE tasks SET commit_ref=? WHERE id=?", params![c, id]).map_err(|e| e.to_string())?;
    }
    let needs = p.needs.map(str::to_owned).or_else(|| (status == "blocking").then(|| "input".to_owned()));
    let ttl = if p.ttl_s > 0 { p.ttl_s } else { DEFAULT_TTL_S };
    match status {
        "working" => {
            let lease = cur.lease_until.map_or(now + ttl * 1000, |old| old.max(now + ttl * 1000));
            tx.execute(
                "UPDATE tasks SET status=?, needs=?, updated_at=?, lease_until=?, started_at=COALESCE(started_at,?), completed_at=NULL WHERE id=?",
                params![status, needs, now, lease, now, id],
            )
        }
        "done" | "dropped" => tx.execute(
            "UPDATE tasks SET status=?, needs=?, updated_at=?, lease_until=NULL, completed_at=COALESCE(completed_at,?) WHERE id=?",
            params![status, needs, now, now, id],
        ),
        "ready" => tx.execute(
            "UPDATE tasks SET status=?, needs=?, updated_at=?, lease_until=NULL, started_at=NULL, completed_at=NULL WHERE id=?",
            params![status, needs, now, id],
        ),
        _ => tx.execute(
            "UPDATE tasks SET status=?, needs=?, updated_at=?, lease_until=NULL WHERE id=?",
            params![status, needs, now, id],
        ),
    }
    .map_err(|e| e.to_string())?;
    let kind = match (moved, status) {
        (true, "done") => "done",
        (true, "review") => "review",
        (true, "blocking") => "block",
        (true, "working") => {
            match cur.status.as_str() {
                "blocking" => "answer",
                "ready" => "claim",
                _ => "edit",
            }
        }
        (true, _) => "edit",
        (false, _) => {
            if p.report.is_some() { "report" } else { "note" }
        }
    };
    let body = p.note.or(p.report).unwrap_or("");
    let seq = log_event(&tx, id, actor, kind, body)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(format!("{} {status} #{seq}", short(id)))
}

pub fn note(actor: &str, id: i64, body: &str) -> Result<String, String> {
    apply(actor, id, &Patch { status: None, report: None, note: Some(body), commit: None, needs: None, ttl_s: 0 })
}

/// Writes the worker's report without moving the task.
pub fn report(actor: &str, id: i64, body: &str) -> Result<String, String> {
    apply(actor, id, &Patch { status: None, report: Some(body), note: None, commit: None, needs: None, ttl_s: 0 })
}

pub fn ask(actor: &str, id: i64, question: &str) -> Result<String, String> {
    apply(actor, id, &Patch { status: Some("blocking"), report: None, note: Some(question), commit: None, needs: Some("input"), ttl_s: 0 })
}

pub fn answer(actor: &str, id: i64, text: &str) -> Result<String, String> {
    // Resume with a fresh lease in the same transaction: no steal window.
    apply(actor, id, &Patch { status: Some("working"), report: None, note: Some(text), commit: None, needs: None, ttl_s: DEFAULT_TTL_S })
}

/// Arbitrary status moves for reopen/drop.

pub fn finish(actor: &str, id: i64, status: &str, summary: &str, commit: Option<&str>) -> Result<String, String> {
    apply(actor, id, &Patch { status: Some(status), report: Some(summary), note: None, commit, needs: None, ttl_s: 0 })
}

pub fn block(actor: &str, id: i64, reason: &str) -> Result<String, String> {
    apply(actor, id, &Patch { status: Some("blocking"), report: None, note: Some(reason), commit: None, needs: None, ttl_s: 0 })
}

pub fn heartbeat(_actor: &str, id: i64, ttl_s: i64) -> Result<String, String> {
    let conn = open()?;
    let lease = now_ms() + ttl_s * 1000;
    let n = conn
        .execute("UPDATE tasks SET lease_until=?, updated_at=? WHERE id=? AND status='working'", params![lease, now_ms(), id])
        .map_err(|e| e.to_string())?;
    if n != 1 {
        return Err(format!("{} not working", short(id)));
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
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let done: i64 = conn
        .query_row("SELECT COUNT(*) FROM tasks WHERE status='done' AND updated_at>=?", params![cutoff], |r| r.get(0))
        .unwrap_or(0);
    let mut head: Vec<String> = counts.iter().map(|(s, n)| format!("{n} {}", code(s))).collect();
    head.push(format!("{done} done/{}m", since_min));
    out.push_str(&head.join(", "));
    out.push('\n');
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {COLS} FROM tasks WHERE status NOT IN ('done','dropped') ORDER BY CASE status WHEN 'blocking' THEN 0 WHEN 'working' THEN 1 WHEN 'review' THEN 2 ELSE 3 END, priority, id LIMIT 25"
        ))
        .map_err(|e| e.to_string())?;
    let rows: Vec<Row> = stmt
        .query_map([], |r| read_row(r))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    for r in &rows {
        let wait = if r.needs.as_deref() == Some("input") { "ON INPUT" } else { "" };
        out.push_str(&format!(
            "{} {} {} {} {}{}\n",
            short(r.id),
            code(&r.status),
            r.assignee.as_deref().unwrap_or("-"),
            ago(r.updated_at),
            clip(&r.title, 60),
            if wait.is_empty() { String::new() } else { format!(" [{wait}]") }
        ));
    }
    let mut stmt = conn
        .prepare("SELECT id,report FROM tasks WHERE status='done' AND updated_at>=? ORDER BY updated_at DESC LIMIT 8")
        .map_err(|e| e.to_string())?;
    let dones: Vec<(i64, Option<String>)> = stmt
        .query_map(params![cutoff], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
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
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?;
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
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?
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
