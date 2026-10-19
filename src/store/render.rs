use super::{COLS, Row, STATUS_LIST, fetch, norm, now_ms, open, read_row, short};
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

fn when(ts: i64) -> String {
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
    if let Some(line) = children_summary(&conn, r.id)? {
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
    let mut stmt = conn
        .prepare(
            "SELECT seq,actor,kind,body,ts FROM events WHERE task_id=? ORDER BY seq DESC LIMIT 8",
        )
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
        let t = short(r.id);
        for (seq, actor, kind, body, ts) in evs.iter().rev() {
            let text = if full {
                body.clone()
            } else {
                clip_marked(
                    body,
                    CAP_EVENT,
                    &format!("`agentboard get {t} --full` or task_get id={t} full=true"),
                )
            };
            out.push_str(&format!("  #{seq} {actor} {kind} {}: {text}\n", ago(*ts)));
        }
    }
    Ok(out.trim_end().to_owned())
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

/// `children: 4 (2 done, 1 working, 1 needs_fix)`, or None for a leaf task.
fn children_summary(conn: &Connection, id: i64) -> Result<Option<String>, String> {
    let mut stmt = conn
        .prepare("SELECT status, COUNT(*) FROM tasks WHERE parent=? GROUP BY status")
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
    Ok(Some(format!("children: {total} ({})", parts.join(", "))))
}

/// One compact line per direct child, ordered by id: id, status, assignee, age, title.
pub fn children(parent: i64) -> Result<String, String> {
    let conn = open()?;
    fetch(&conn, parent)?;
    let mut stmt = conn
        .prepare("SELECT id,status,assignee,updated_at,title FROM tasks WHERE parent=? ORDER BY id")
        .map_err(|e| e.to_string())?;
    let rows: Vec<(i64, String, Option<String>, i64, String)> = stmt
        .query_map(params![parent], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    if rows.is_empty() {
        return Ok(format!("{} has no children", short(parent)));
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
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {COLS} FROM tasks WHERE status NOT IN ('done','dropped') ORDER BY CASE status WHEN 'blocking' THEN 0 WHEN 'needs_fix' THEN 1 WHEN 'working' THEN 2 WHEN 'review' THEN 3 ELSE 4 END, priority, id LIMIT 25"
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
    // A task with children is an epic. Report progress only while some child is
    // still open and the epic itself is not closed; dropped children do not
    // count toward the total. Closing the epic is the manager's call, never automatic.
    let mut stmt = conn
        .prepare(
            "SELECT p.id, p.title, SUM(c.status='done'), SUM(c.status!='dropped') FROM tasks p JOIN tasks c ON c.parent=p.id
             WHERE p.status NOT IN ('done','dropped')
             GROUP BY p.id HAVING SUM(c.status NOT IN ('done','dropped')) > 0 ORDER BY p.id",
        )
        .map_err(|e| e.to_string())?;
    let epics: Vec<(i64, String, i64, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    for (id, title, done_n, total) in epics {
        out.push_str(&format!(
            "EPIC {} {done_n}/{total} done {}\n",
            short(id),
            clip(&title, 60)
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
