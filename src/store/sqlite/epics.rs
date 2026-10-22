//! Epics: a project-scoped grouping of tasks with its own id space (E1, E2...).
//! An epic is never claimed, leased or dispatched; it names the project (`path`)
//! the work belongs to.

use super::render::{recent_events, status_summary};
use super::{Subject, field_names, log_event, now_ms, open, short_epic, text_change, value_change};
use crate::store::board::EpicEdit;
use crate::store::text::{self, EpicRow, normalize_path, path_note};
use crate::store::{EPIC_STATUS_LIST, norm_status};
use rusqlite::{Connection, OptionalExtension, params};

pub(super) fn fetch_epic(conn: &Connection, id: i64) -> Result<EpicRow, String> {
    conn.query_row(
        "SELECT id,title,description,goal,status,path,created_at,completed_at FROM epics WHERE id=?",
        params![id],
        |r| {
            Ok(EpicRow {
                id: r.get(0)?,
                title: r.get(1)?,
                description: r.get(2)?,
                goal: r.get(3)?,
                status: r.get(4)?,
                path: r.get(5)?,
                created_at: r.get(6)?,
                completed_at: r.get(7)?,
            })
        },
    )
    .optional()
    .map_err(|e| e.to_string())?
    .ok_or_else(|| format!("no such epic {}", short_epic(id)))
}

pub fn epic_add(
    actor: &str,
    title: &str,
    description: &str,
    goal: &str,
    path: Option<&str>,
) -> Result<String, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("cwd: {e}"))?;
    let home = std::env::var("HOME").ok();
    let path = normalize_path(path.unwrap_or(""), &cwd, home.as_deref());
    let mut conn = open()?;
    let tx = super::write_transaction(&mut conn)?;
    let now = now_ms();
    tx.execute(
        "INSERT INTO epics(title,description,goal,path,created_at,updated_at) VALUES(?,?,?,?,?,?)",
        params![title, description, goal, path, now, now],
    )
    .map_err(|e| format!("epic add: {e}"))?;
    let id = tx.last_insert_rowid();
    log_event(&tx, Subject::Epic(id), actor, "add", title)?;
    tx.commit().map_err(|e| e.to_string())?;
    let mut out = format!("{} open", short_epic(id));
    if !path.is_empty() {
        out.push_str(&format!(" {path}"));
        if !path_note(&path).is_empty() {
            out.push_str(&format!(
                "\nwarning: path {path} does not exist (yet); the epic is created anyway"
            ));
        }
    }
    Ok(out)
}

pub fn epic_list(status: Option<&str>) -> Result<String, String> {
    let want = match status {
        Some(s) => {
            norm_status(s).ok_or_else(|| format!("bad epic status {s:?} ({EPIC_STATUS_LIST})"))?
        }
        None => "open",
    };
    let conn = open()?;
    let mut stmt = conn
        .prepare("SELECT id,title,path FROM epics WHERE status=? ORDER BY id")
        .map_err(|e| e.to_string())?;
    let rows: Vec<(i64, String, String)> = stmt
        .query_map(params![want], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    if rows.is_empty() {
        return Ok(format!("no {want} epics"));
    }
    let mut out = String::new();
    for (id, title, path) in rows {
        let (done, total) = counts(&conn, id)?;
        let e = EpicRow {
            id,
            title,
            description: String::new(),
            goal: String::new(),
            status: want.to_owned(),
            path,
            created_at: 0,
            completed_at: None,
        };
        out.push_str(&text::epic_list_line(&e, want, done, total));
    }
    Ok(out.trim_end().to_owned())
}

/// (done, not-dropped) task counts of an epic: dropped work leaves the denominator.
pub(super) fn counts(conn: &Connection, epic: i64) -> Result<(i64, i64), String> {
    conn.query_row(
        "SELECT COALESCE(SUM(status='done'),0), COALESCE(SUM(status!='dropped'),0) FROM tasks WHERE epic_id=?",
        params![epic],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .map_err(|e| e.to_string())
}

/// `full` returns description and goal unclipped, verbatim.
pub fn epic_get(id: i64, full: bool) -> Result<String, String> {
    let conn = open()?;
    let e = fetch_epic(&conn, id)?;
    let summary = status_summary(&conn, "epic_id", e.id, "tasks")?;
    let events = recent_events(&conn, "epic_id", e.id, full, &text::epic_hint(e.id))?;
    Ok(text::epic_text(&e, summary.as_deref(), &events, full))
}

/// Opens, closes or drops an epic. Closing is the manager's call, never automatic.
pub fn epic_status(
    actor: &str,
    id: i64,
    status: &str,
    note: Option<&str>,
) -> Result<String, String> {
    let want = norm_status(status)
        .ok_or_else(|| format!("bad epic status {status:?} ({EPIC_STATUS_LIST})"))?;
    let mut conn = open()?;
    let tx = super::write_transaction(&mut conn)?;
    let cur = fetch_epic(&tx, id)?;
    let now = now_ms();
    let completed = (want != "open").then(|| cur.completed_at.unwrap_or(now));
    tx.execute(
        "UPDATE epics SET status=?, updated_at=?, completed_at=? WHERE id=?",
        params![want, now, completed, id],
    )
    .map_err(|e| e.to_string())?;
    let kind = if want == "done" { "done" } else { "edit" };
    let seq = log_event(&tx, Subject::Epic(id), actor, kind, note.unwrap_or(""))?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(format!("{} {want} #{seq}", short_epic(id)))
}

/// Amends an epic's spec. Epics are never leased, so there is no lease guard.
/// Logs one `edit` event naming each changed field with its old/new length;
/// nothing is written when no value moved.
pub fn epic_edit(actor: &str, id: i64, e: &EpicEdit<'_>) -> Result<String, String> {
    if e.title.is_none() && e.description.is_none() && e.goal.is_none() && e.path.is_none() {
        return Err("nothing to edit: pass title, description, goal or path".to_owned());
    }
    if e.title.is_some_and(|t| t.trim().is_empty()) {
        return Err("title cannot be empty".to_owned());
    }
    let cwd = std::env::current_dir().map_err(|e| format!("cwd: {e}"))?;
    let home = std::env::var("HOME").ok();
    let path = e.path.map(|p| normalize_path(p, &cwd, home.as_deref()));
    let mut conn = open()?;
    let tx = super::write_transaction(&mut conn)?;
    let cur = fetch_epic(&tx, id)?;
    let mut diffs = Vec::new();
    let title = value_change(&mut diffs, "title", &cur.title, e.title);
    let description = text_change(&mut diffs, "description", &cur.description, e.description);
    let goal = text_change(&mut diffs, "goal", &cur.goal, e.goal);
    let path = value_change(&mut diffs, "path", &cur.path, path.as_deref());
    if diffs.is_empty() {
        return Ok(format!("{} unchanged", short_epic(id)));
    }
    tx.execute(
        "UPDATE epics SET title=?, description=?, goal=?, path=?, updated_at=? WHERE id=?",
        params![title, description, goal, path, now_ms(), id],
    )
    .map_err(|e| e.to_string())?;
    let seq = log_event(&tx, Subject::Epic(id), actor, "edit", &diffs.join(", "))?;
    tx.commit().map_err(|e| e.to_string())?;
    let mut out = format!("{} edited {} #{seq}", short_epic(id), field_names(&diffs));
    if diffs.iter().any(|d| d.starts_with("path ")) && !path_note(path).is_empty() {
        out.push_str(&format!(
            "\nwarning: path {path} does not exist (yet); the epic is edited anyway"
        ));
    }
    Ok(out)
}
