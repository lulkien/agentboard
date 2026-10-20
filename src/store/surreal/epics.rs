//! Epics on the SurrealDB backend: the port of `sqlite/epics.rs`. Same locking rules
//! as `tasks.rs`: a write holds the write lock for the whole read-modify-write and is
//! one server-side transaction; a read takes no lock.
//!
//! `normalize_path` and `path_note` are copies of the sqlite ones (see the debt table
//! in docs/surrealdb-migration.md).

use super::tasks::{
    Txn, assign, field_names, int_of, recent_events, status_summary, text_change, value_change,
    when,
};
use super::{Db, block_on, db_err, session};
use crate::clip::{CAP_DESCRIPTION, CAP_GOAL, clip, clip_marked};
use crate::store::board::EpicEdit;
use crate::store::{now_ms, short_epic};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Component, Path, PathBuf};

#[derive(Deserialize)]
pub(super) struct EpicRow {
    tid: i64,
    pub title: String,
    description: String,
    goal: String,
    status: String,
    pub path: String,
    created_at: i64,
    completed_at: Option<i64>,
}

const SELECT_EPIC: &str = "SELECT meta::id(id) AS tid, * FROM epics";

pub(super) async fn fetch_epic(db: &Db, id: i64) -> Result<EpicRow, String> {
    let mut r = db
        .query(format!(
            "{SELECT_EPIC} WHERE id = type::thing('epics', $id)"
        ))
        .bind(("id", id))
        .await
        .map_err(|e| db_err(&e))?;
    let rows: Vec<EpicRow> = r.take(0).map_err(|e| db_err(&e))?;
    rows.into_iter()
        .next()
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
fn normalize_path(raw: &str, cwd: &Path, home: Option<&str>) -> String {
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

/// The marker `epic get` shows for a path that is not on disk.
fn path_note(path: &str) -> &'static str {
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
    let s = session(true)?;
    let id = block_on(async {
        let now = now_ms();
        let mut tx = Txn::new();
        tx.raw("LET $id = (UPSERT counters:epic SET n = (n ?? 0) + 1 RETURN VALUE n)[0];");
        let c = tx.p(json!({
            "title": title, "description": description, "goal": goal, "path": path,
            "created_at": now, "updated_at": now,
        }));
        tx.raw(&format!("CREATE type::thing('epics', $id) CONTENT {c};"));
        tx.event_on("epic_id", "$id", actor, "add", title);
        tx.run(&s.db, "$id")
            .await
            .map_err(|e| format!("epic add: {e}"))
            .and_then(|v| int_of(&v, "epic id"))
    })?;
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

/// (done, not-dropped) task counts of an epic: dropped work leaves the denominator.
async fn counts(db: &Db, epic: i64) -> Result<(i64, i64), String> {
    #[derive(Deserialize)]
    struct S {
        status: String,
    }
    let mut r = db
        .query("SELECT status FROM tasks WHERE epic_id = $id")
        .bind(("id", epic))
        .await
        .map_err(|e| db_err(&e))?;
    let rows: Vec<S> = r.take(0).map_err(|e| db_err(&e))?;
    let done = rows.iter().filter(|t| t.status == "done").count();
    let live = rows.iter().filter(|t| t.status != "dropped").count();
    Ok((
        i64::try_from(done).unwrap_or(i64::MAX),
        i64::try_from(live).unwrap_or(i64::MAX),
    ))
}

pub fn epic_list(status: Option<&str>) -> Result<String, String> {
    let want = match status {
        Some(s) => {
            norm_status(s).ok_or_else(|| format!("bad epic status {s:?} ({STATUS_LIST})"))?
        }
        None => "open",
    };
    let s = session(false)?;
    block_on(async {
        let mut r =
            s.db.query(format!("{SELECT_EPIC} WHERE status = $s ORDER BY tid"))
                .bind(("s", want))
                .await
                .map_err(|e| db_err(&e))?;
        let rows: Vec<EpicRow> = r.take(0).map_err(|e| db_err(&e))?;
        if rows.is_empty() {
            return Ok(format!("no {want} epics"));
        }
        let mut out = String::new();
        for e in rows {
            let (done, total) = counts(&s.db, e.tid).await?;
            out.push_str(&format!(
                "{} {want} {done}/{total} done {}{} {}\n",
                short_epic(e.tid),
                if e.path.is_empty() { "-" } else { &e.path },
                path_note(&e.path),
                clip(&e.title, 60)
            ));
        }
        Ok(out.trim_end().to_owned())
    })
}

/// `full` returns description and goal unclipped, verbatim.
pub fn epic_get(id: i64, full: bool) -> Result<String, String> {
    let s = session(false)?;
    block_on(async {
        let e = fetch_epic(&s.db, id).await?;
        let tag = short_epic(e.tid);
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
        if let Some(line) = status_summary(&s.db, "epic_id", e.tid, "tasks").await? {
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
        out.push_str(&recent_events(&s.db, "epic_id", e.tid, full, &hint).await?);
        Ok(out.trim_end().to_owned())
    })
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
    let s = session(true)?;
    block_on(async {
        let cur = fetch_epic(&s.db, id).await?;
        let now = now_ms();
        let completed = (want != "open").then(|| cur.completed_at.unwrap_or(now));
        let mut tx = Txn::new();
        let mut sets = Vec::new();
        assign(&mut tx, &mut sets, "status", Some(want.into()));
        assign(&mut tx, &mut sets, "updated_at", Some(now.into()));
        assign(
            &mut tx,
            &mut sets,
            "completed_at",
            completed.map(Value::from),
        );
        let idp = tx.p(id);
        tx.raw(&format!(
            "UPDATE type::thing('epics', {idp}) SET {};",
            sets.join(", ")
        ));
        let kind = if want == "done" { "done" } else { "edit" };
        tx.event_on("epic_id", &idp, actor, kind, note.unwrap_or(""));
        let seq = int_of(&tx.run(&s.db, "$seq").await?, "event seq")?;
        Ok(format!("{} {want} #{seq}", short_epic(id)))
    })
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
    let s = session(true)?;
    block_on(async {
        let cur = fetch_epic(&s.db, id).await?;
        let mut diffs = Vec::new();
        let title = value_change(&mut diffs, "title", &cur.title, e.title);
        let description = text_change(&mut diffs, "description", &cur.description, e.description);
        let goal = text_change(&mut diffs, "goal", &cur.goal, e.goal);
        let path = value_change(&mut diffs, "path", &cur.path, path.as_deref());
        if diffs.is_empty() {
            return Ok(format!("{} unchanged", short_epic(id)));
        }
        let mut tx = Txn::new();
        let mut sets = Vec::new();
        assign(&mut tx, &mut sets, "title", Some(title.into()));
        assign(&mut tx, &mut sets, "description", Some(description.into()));
        assign(&mut tx, &mut sets, "goal", Some(goal.into()));
        assign(&mut tx, &mut sets, "path", Some(path.into()));
        assign(&mut tx, &mut sets, "updated_at", Some(now_ms().into()));
        let idp = tx.p(id);
        tx.raw(&format!(
            "UPDATE type::thing('epics', {idp}) SET {};",
            sets.join(", ")
        ));
        tx.event_on("epic_id", &idp, actor, "edit", &diffs.join(", "));
        let seq = int_of(&tx.run(&s.db, "$seq").await?, "event seq")?;
        let mut out = format!("{} edited {} #{seq}", short_epic(id), field_names(&diffs));
        if diffs.iter().any(|d| d.starts_with("path ")) && !path_note(path).is_empty() {
            out.push_str(&format!(
                "\nwarning: path {path} does not exist (yet); the epic is edited anyway"
            ));
        }
        Ok(out)
    })
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
    fn paths_are_absolute_clean_and_need_not_exist() {
        assert_eq!(n(""), "");
        assert_eq!(n("   "), "");
        assert_eq!(n("/a/b/../c/./d/"), "/a/c/d");
        assert_eq!(n("repo"), "/work/here/repo");
        assert_eq!(n("../repo"), "/work/repo");
        assert_eq!(n("~"), "/home/u");
        assert_eq!(n("~/x/../y"), "/home/u/y");
    }
}
