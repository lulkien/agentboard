//! Epics: a project-scoped grouping of tasks with its own id space (E1, E2...).
//! An epic is never claimed, leased or dispatched; it names the project (`path`)
//! the work belongs to.

use super::render::{recent_events, status_summary, when};
use super::{Subject, field_names, log_event, now_ms, open, short_epic, text_change};
use crate::clip::{CAP_DESCRIPTION, CAP_GOAL, clip, clip_marked};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Component, Path, PathBuf};

pub(super) struct EpicRow {
    pub id: i64,
    pub title: String,
    pub description: String,
    pub goal: String,
    pub status: String,
    pub path: String,
    pub created_at: i64,
    pub completed_at: Option<i64>,
}

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

const STATUS_LIST: &str = "open|done|dropped";

fn norm_status(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "open" => Some("open"),
        "done" => Some("done"),
        "dropped" => Some("dropped"),
        _ => None,
    }
}

/// Absolute, lexically clean project path: `~` and `~/x` expand against `home`,
/// a relative path resolves against `cwd`, `.`/`..` collapse, no trailing slash.
/// The path need not exist (an epic may be written before the repo).
pub(super) fn normalize_path(raw: &str, cwd: &Path, home: Option<&str>) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    let expanded: PathBuf = match (raw, home) {
        ("~", Some(h)) => PathBuf::from(h),
        (r, Some(h)) if r.starts_with("~/") => Path::new(h).join(&r[2..]),
        (r, _) => PathBuf::from(r),
    };
    let abs = if expanded.is_absolute() {
        expanded
    } else {
        cwd.join(expanded)
    };
    let mut out = PathBuf::new();
    for c in abs.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out.to_string_lossy().into_owned()
}

/// The marker `epic get` and `brief` show for a path that is not on disk.
pub(super) fn path_note(path: &str) -> &'static str {
    if !path.is_empty() && !Path::new(path).exists() {
        " (does not exist)"
    } else {
        ""
    }
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
    let tx = conn.transaction().map_err(|e| e.to_string())?;
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
            norm_status(s).ok_or_else(|| format!("bad epic status {s:?} ({STATUS_LIST})"))?
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
        out.push_str(&format!(
            "{} {want} {done}/{total} done {}{} {}\n",
            short_epic(id),
            if path.is_empty() { "-" } else { &path },
            path_note(&path),
            clip(&title, 60)
        ));
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
    let tag = short_epic(e.id);
    let mut out = format!("{tag} {}\ntitle: {}\n", e.status, e.title);
    if e.path.is_empty() {
        out.push_str("path: -\n");
    } else {
        out.push_str(&format!("path: {}{}\n", e.path, path_note(&e.path)));
    }
    out.push_str(&format!("created: {}\n", when(e.created_at)));
    if let Some(c) = e.completed_at {
        out.push_str(&format!("completed: {}\n", when(c)));
    }
    if let Some(line) = status_summary(&conn, "epic_id", e.id, "tasks")? {
        out.push_str(&line);
        out.push('\n');
    }
    let hint = format!("`agentboard epic get {tag} --full` or epic_get id={tag} full=true");
    for (label, value, cap) in [
        ("description", &e.description, CAP_DESCRIPTION),
        ("goal", &e.goal, CAP_GOAL),
    ] {
        if value.is_empty() {
            continue;
        }
        let shown = if full {
            value.clone()
        } else {
            clip_marked(value, cap, &hint)
        };
        out.push_str(&format!("{label}:\n{shown}\n"));
    }
    out.push_str(&recent_events(&conn, "epic_id", e.id, full, &hint)?);
    Ok(out.trim_end().to_owned())
}

/// Opens, closes or drops an epic. Closing is the manager's call, never automatic.
pub fn epic_status(
    actor: &str,
    id: i64,
    status: &str,
    note: Option<&str>,
) -> Result<String, String> {
    let want =
        norm_status(status).ok_or_else(|| format!("bad epic status {status:?} ({STATUS_LIST})"))?;
    let mut conn = open()?;
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
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

/// New spec text for an epic; `None` leaves a field alone. An empty `path`
/// clears it.
pub struct EpicEdit<'a> {
    pub title: Option<&'a str>,
    pub description: Option<&'a str>,
    pub goal: Option<&'a str>,
    pub path: Option<&'a str>,
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
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let cur = fetch_epic(&tx, id)?;
    let mut diffs = Vec::new();
    let title = text_change(&mut diffs, "title", &cur.title, e.title);
    let description = text_change(&mut diffs, "description", &cur.description, e.description);
    let goal = text_change(&mut diffs, "goal", &cur.goal, e.goal);
    let path = text_change(&mut diffs, "path", &cur.path, path.as_deref());
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

#[cfg(test)]
mod tests {
    use super::normalize_path;
    use std::path::Path;

    const CWD: &str = "/work/here";

    fn n(raw: &str) -> String {
        normalize_path(raw, Path::new(CWD), Some("/home/u"))
    }

    #[test]
    fn paths_are_absolute_clean_and_slashless() {
        assert_eq!(n("/srv/app/"), "/srv/app");
        assert_eq!(n("/srv//app/./x/../y"), "/srv/app/y");
        assert_eq!(n("~"), "/home/u");
        assert_eq!(n("~/code/app/"), "/home/u/code/app");
        assert_eq!(n("proj"), "/work/here/proj");
        assert_eq!(n("../other/"), "/work/other");
        assert_eq!(n("."), "/work/here");
        assert_eq!(n("/"), "/");
        assert_eq!(n("  "), "");
        assert_eq!(n("/../.."), "/", "cannot climb above the root");
    }

    #[test]
    fn tilde_without_home_is_just_a_relative_name() {
        assert_eq!(
            normalize_path("~/x", Path::new(CWD), None),
            "/work/here/~/x"
        );
    }
}
