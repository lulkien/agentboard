use super::{COLS, Ref, Row, STATUS_LIST, fetch, norm, now_ms, open, read_row, short, short_epic};
use crate::clip::{
    CAP_DESCRIPTION, CAP_EVENT, CAP_GOAL, CAP_NOTE, CAP_REPORT, clip, clip_marked, field,
};
use rusqlite::{Connection, params};

fn code(status: &str) -> &str {
    match status {
        "ready" | "triage" => "rdy",
        "working" | "running" => "wrk",
        "blocking" | "blocked" => "blk",
        "needs_fix" => "fix",
        "review" => "rev",
        "done" => "don",
        "dropped" => "drp",
        other => other,
    }
}

pub(super) fn ago(ts_ms: i64) -> String {
    let d = (now_ms() - ts_ms).max(0) / 1000;
    match d {
        0..=59 => format!("{d}s"),
        60..=3599 => format!("{}m", d / 60),
        3600..=86_399 => format!("{}h", d / 3600),
        _ => format!("{}d", d / 86_400),
    }
}

/// One-line timestamp for listings: "13:32Z" style is locale-safe enough here.
pub(super) fn stamp(ts_ms: i64) -> String {
    let secs = ts_ms / 1000;
    let (h, m) = ((secs / 3600) % 24, (secs / 60) % 60);
    format!("{h:02}:{m:02}")
}

pub fn list(status: Option<&str>, assignee: Option<&str>, limit: usize) -> Result<String, String> {
    let conn = open()?;
    let mut sql = format!("SELECT {COLS} FROM tasks WHERE 1=1");
    let mut args: Vec<String> = Vec::new();
    match status {
        Some(s) => {
            let want = norm(s).ok_or_else(|| format!("bad status {s:?} ({STATUS_LIST})"))?;
            sql.push_str(" AND status=?");
            args.push(want.to_owned());
        }
        None => sql.push_str(" AND status NOT IN ('done','dropped')"),
    }
    if let Some(a) = assignee {
        sql.push_str(" AND assignee=?");
        args.push(a.to_owned());
    }
    sql.push_str(" ORDER BY CASE status WHEN 'blocking' THEN 0 WHEN 'needs_fix' THEN 1 WHEN 'working' THEN 2 WHEN 'review' THEN 3 WHEN 'ready' THEN 4 ELSE 5 END, priority, id LIMIT ?");
    args.push(limit.to_string());
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows: Vec<Row> = stmt
        .query_map(rusqlite::params_from_iter(args.iter()), read_row)
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<Row>>>()
        .map_err(|e| e.to_string())?;
    if rows.is_empty() {
        return Ok("(no tasks)".to_owned());
    }
    let mut out = String::new();
    for r in &rows {
        let who = r.assignee.as_deref().unwrap_or("-");
        let flag = if r.needs.as_deref() == Some("input") {
            " [input]"
        } else {
            ""
        };
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

pub(super) fn when(ts: i64) -> String {
    format!("{} ({}Z)", ago(ts), stamp(ts))
}

/// `full` returns description, goal, report and note unclipped and verbatim.
pub fn get(id: i64, full: bool) -> Result<String, String> {
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
    if let Some(e) = r.epic_id {
        let epic = super::epics::fetch_epic(&conn, e)?;
        out.push_str(&format!(
            "epic: {} {}{}\n",
            short_epic(e),
            clip(&epic.title, 60),
            if epic.path.is_empty() {
                String::new()
            } else {
                format!(" [{}]", epic.path)
            }
        ));
    }
    if let Some(p) = r.parent_id {
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
    if let Some(line) = status_summary(&conn, "parent_id", r.id, "children")? {
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str(&field(
        "description",
        &r.description,
        CAP_DESCRIPTION,
        full,
        &short(r.id),
    ));
    out.push_str(&field("goal", &r.goal, CAP_GOAL, full, &short(r.id)));
    out.push_str(&field(
        "report",
        r.report.as_deref().unwrap_or(""),
        CAP_REPORT,
        full,
        &short(r.id),
    ));
    out.push_str(&field(
        "note",
        r.note.as_deref().unwrap_or(""),
        CAP_NOTE,
        full,
        &short(r.id),
    ));
    if let Some(c) = &r.commit_ref {
        out.push_str(&format!("commit: {c}\n"));
    }
    let t = short(r.id);
    out.push_str(&recent_events(
        &conn,
        "task_id",
        r.id,
        full,
        &format!("`agentboard get {t} --full` or task_get id={t} full=true"),
    )?);
    Ok(out.trim_end().to_owned())
}

/// The last 8 events of a task or an epic (`col` is `task_id` or `epic_id`) as an
/// `events:` block, oldest first; bodies are clipped with a marker unless `full`.
pub(super) fn recent_events(
    conn: &Connection,
    col: &str,
    id: i64,
    full: bool,
    hint: &str,
) -> Result<String, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT seq,actor,kind,body,ts FROM events WHERE {col}=? ORDER BY seq DESC LIMIT 8"
        ))
        .map_err(|e| e.to_string())?;
    let evs: Vec<(i64, String, String, String, i64)> = stmt
        .query_map(params![id], |e| {
            Ok((e.get(0)?, e.get(1)?, e.get(2)?, e.get(3)?, e.get(4)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    if evs.is_empty() {
        return Ok(String::new());
    }
    let mut out = String::from("events:\n");
    for (seq, actor, kind, body, ts) in evs.iter().rev() {
        let text = if full {
            body.clone()
        } else {
            clip_marked(body, CAP_EVENT, hint)
        };
        out.push_str(&format!("  #{seq} {actor} {kind} {}: {text}\n", ago(*ts)));
    }
    Ok(out)
}

/// Canonical status order for summaries: most urgent first.
const STATUS_ORDER: [&str; 7] = [
    "blocking",
    "needs_fix",
    "working",
    "review",
    "ready",
    "done",
    "dropped",
];

/// `children: 4 (2 done, 1 working, 1 needs_fix)`, or None when there are none.
/// `col` is `parent_id` (subtasks) or `epic_id` (an epic's tasks).
pub(super) fn status_summary(
    conn: &Connection,
    col: &str,
    id: i64,
    label: &str,
) -> Result<Option<String>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT status, COUNT(*) FROM tasks WHERE {col}=? GROUP BY status"
        ))
        .map_err(|e| e.to_string())?;
    let counts: Vec<(String, i64)> = stmt
        .query_map(params![id], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let total: i64 = counts.iter().map(|(_, n)| n).sum();
    if total == 0 {
        return Ok(None);
    }
    let mut parts = Vec::new();
    for st in STATUS_ORDER {
        if let Some((_, n)) = counts.iter().find(|(s, _)| s == st) {
            parts.push(format!("{n} {st}"));
        }
    }
    Ok(Some(format!("{label}: {total} ({})", parts.join(", "))))
}

/// One compact line per task, ordered by id: id, status, assignee, age, title.
/// For a task these are its subtasks (`parent_id`); for an epic, every task in it.
pub fn children(of: Ref) -> Result<String, String> {
    let conn = open()?;
    let (label, col, id) = match of {
        Ref::Task(id) => {
            fetch(&conn, id)?;
            (short(id), "parent_id", id)
        }
        Ref::Epic(id) => {
            super::epics::fetch_epic(&conn, id)?;
            (short_epic(id), "epic_id", id)
        }
    };
    let mut stmt = conn
        .prepare(&format!(
            "SELECT id,status,assignee,updated_at,title FROM tasks WHERE {col}=? ORDER BY id"
        ))
        .map_err(|e| e.to_string())?;
    let rows: Vec<(i64, String, Option<String>, i64, String)> = stmt
        .query_map(params![id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    if rows.is_empty() {
        return Ok(format!("{label} has no children"));
    }
    let mut out = String::new();
    for (id, status, assignee, upd, title) in rows {
        out.push_str(&format!(
            "{} {} {:<9} {:>4} {}\n",
            short(id),
            code(&status),
            assignee.as_deref().unwrap_or("-"),
            ago(upd),
            clip(&title, 64)
        ));
    }
    Ok(out.trim_end().to_owned())
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
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE status='done' AND updated_at>=?",
            params![cutoff],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let mut head: Vec<String> = counts
        .iter()
        .map(|(s, n)| format!("{n} {}", code(s)))
        .collect();
    head.push(format!("{done} done/{}m", since_min));
    out.push_str(&head.join(", "));
    out.push('\n');
    // One line per open epic: progress, the project path, and only the counts a
    // manager must act on. Its tasks are one `children E<n>` away. Dropped tasks
    // leave the denominator; closing an epic is the manager's call, never automatic.
    let mut stmt = conn
        .prepare("SELECT id,title,path FROM epics WHERE status='open' ORDER BY id")
        .map_err(|e| e.to_string())?;
    let epics: Vec<(i64, String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    for (id, title, path) in epics {
        let (done_n, total) = super::epics::counts(&conn, id)?;
        let urgent: Vec<String> = ["blocking", "needs_fix"]
            .iter()
            .filter_map(|st| {
                let n: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM tasks WHERE epic_id=? AND status=?",
                        params![id, st],
                        |r| r.get(0),
                    )
                    .unwrap_or(0);
                (n > 0).then(|| format!("{n} {st}"))
            })
            .collect();
        out.push_str(&format!(
            "EPIC {} {done_n}/{total} done{} {}{} {}\n",
            short_epic(id),
            if urgent.is_empty() {
                String::new()
            } else {
                format!(" [{}]", urgent.join(", "))
            },
            if path.is_empty() { "-" } else { &path },
            super::epics::path_note(&path),
            clip(&title, 60)
        ));
    }
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {COLS} FROM tasks WHERE status NOT IN ('done','dropped') AND epic_id IS NULL ORDER BY CASE status WHEN 'blocking' THEN 0 WHEN 'needs_fix' THEN 1 WHEN 'working' THEN 2 WHEN 'review' THEN 3 ELSE 4 END, priority, id LIMIT 25"
        ))
        .map_err(|e| e.to_string())?;
    let rows: Vec<Row> = stmt
        .query_map([], read_row)
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    for r in &rows {
        let wait = if r.needs.as_deref() == Some("input") {
            "ON INPUT"
        } else {
            ""
        };
        out.push_str(&format!(
            "{} {} {} {} {}{}\n",
            short(r.id),
            code(&r.status),
            r.assignee.as_deref().unwrap_or("-"),
            ago(r.updated_at),
            clip(&r.title, 60),
            if wait.is_empty() {
                String::new()
            } else {
                format!(" [{wait}]")
            }
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
            out.push_str(&format!(
                "  {} {}\n",
                short(id),
                clip(res.as_deref().unwrap_or(""), 80)
            ));
        }
    }
    Ok(out.trim_end().to_owned())
}
