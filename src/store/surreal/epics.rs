//! Epics on the SurrealDB backend: the port of `sqlite/epics.rs`. Same locking rules
//! as `tasks.rs`: a write holds the write lock for the whole read-modify-write and is
//! one server-side transaction; a read takes no lock.
//!
//! Rows are loaded here and normalised to the plain `text::EpicRow`; the paths and the
//! text come from the shared `text` module, as in the sqlite build.

use super::tasks::{Txn, assign, int_of, recent_events, status_summary};
use super::{Db, block_on, db_err, session};
use crate::store::board::EpicEdit;
use crate::store::text::{self, field_names, normalize_path, path_note, text_change, value_change};
use crate::store::{EPIC_STATUS_LIST, norm_status, now_ms, short_epic};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
pub(super) struct EpicRow {
    pub(super) tid: i64,
    pub(super) title: String,
    pub(super) description: String,
    pub(super) goal: String,
    pub(super) status: String,
    pub(super) path: String,
    pub(super) created_at: i64,
    pub(super) completed_at: Option<i64>,
}

impl From<EpicRow> for text::EpicRow {
    fn from(e: EpicRow) -> Self {
        Self {
            id: e.tid,
            title: e.title,
            description: e.description,
            goal: e.goal,
            status: e.status,
            path: e.path,
            created_at: e.created_at,
            completed_at: e.completed_at,
        }
    }
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
            .map_err(|e| super::with_context("epic add", &e))
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
pub(super) async fn counts(db: &Db, epic: i64) -> Result<(i64, i64), String> {
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
            norm_status(s).ok_or_else(|| format!("bad epic status {s:?} ({EPIC_STATUS_LIST})"))?
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
            out.push_str(&text::epic_list_line(&e.into(), want, done, total));
        }
        Ok(out.trim_end().to_owned())
    })
}

/// `full` returns description and goal unclipped, verbatim.
pub fn epic_get(id: i64, full: bool) -> Result<String, String> {
    let s = session(false)?;
    block_on(async {
        let e: text::EpicRow = fetch_epic(&s.db, id).await?.into();
        let summary = status_summary(&s.db, "epic_id", e.id, "tasks").await?;
        let events = recent_events(&s.db, "epic_id", e.id, full, &text::epic_hint(e.id)).await?;
        Ok(text::epic_text(&e, summary.as_deref(), &events, full))
    })
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
