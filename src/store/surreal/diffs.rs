//! Diffs on the SurrealDB backend: the port of `sqlite/diffs.rs`. Append-only, so a
//! write is one insert plus its event in one transaction under the write lock, and
//! nothing here edits or deletes a row.

use super::tasks::{Txn, fetch, select_rows};
use super::{Db, block_on, db_err, session};
use crate::store::text::{self, DiffRow};
use crate::store::{Ref, now_ms, short, short_diff};
use serde::Deserialize;
use serde_json::json;

/// A diff row. `did` is the integer record key (`meta::id(id)`).
#[derive(Deserialize)]
struct Row {
    did: i64,
    task_id: i64,
    round: i64,
    body: String,
    created_at: i64,
}

impl From<Row> for DiffRow {
    fn from(r: Row) -> Self {
        Self {
            id: r.did,
            task_id: r.task_id,
            round: r.round,
            body: r.body,
            created_at: r.created_at,
        }
    }
}

const SELECT_DIFF: &str = "SELECT meta::id(id) AS did, task_id, round, body, created_at FROM diffs";

async fn select(db: &Db, tail: &str, binds: serde_json::Value) -> Result<Vec<DiffRow>, String> {
    let mut r = db
        .query(format!("{SELECT_DIFF} {tail}"))
        .bind(binds)
        .await
        .map_err(|e| db_err(&e))?;
    let rows: Vec<Row> = r.take(0).map_err(|e| db_err(&e))?;
    Ok(rows.into_iter().map(DiffRow::from).collect())
}

/// The newest diff of a task, for the line `get` shows.
pub(super) async fn newest_diff(db: &Db, task: i64) -> Result<Option<DiffRow>, String> {
    Ok(select(
        db,
        "WHERE task_id = $id ORDER BY did DESC LIMIT 1",
        json!({"id": task}),
    )
    .await?
    .into_iter()
    .next())
}

pub fn diff_add(actor: &str, task: i64, body: &str) -> Result<String, String> {
    if body.trim().is_empty() {
        return Err(text::EMPTY_DIFF.to_owned());
    }
    let s = session(true)?;
    block_on(record(&s.db, actor, task, body))
}

/// Inserts the row and its event in one transaction. The caller holds the write lock.
pub(super) async fn record(db: &Db, actor: &str, task: i64, body: &str) -> Result<String, String> {
    let round = fetch(db, task).await?.reopen_count + 1;
    // The write lock is held, so nobody else moves the counter between this read and
    // the transaction; the event body needs the id, which is known only now.
    let mut r = db
        .query("SELECT VALUE n FROM counters:diff")
        .await
        .map_err(|e| db_err(&e))?;
    let last: Vec<i64> = r.take(0).map_err(|e| db_err(&e))?;
    let id = last.first().copied().unwrap_or(0) + 1;
    let mut tx = Txn::new();
    let idp = tx.p(id);
    tx.raw(&format!("UPSERT counters:diff SET n = {idp};"));
    let content = tx.p(json!({
        "task_id": task, "round": round, "body": body, "created_at": now_ms(),
    }));
    tx.raw(&format!(
        "CREATE type::thing('diffs', {idp}) CONTENT {content};"
    ));
    let taskp = tx.p(task);
    tx.event(&taskp, actor, "diff", &text::diff_event(id, round, body));
    tx.run(db, "$seq")
        .await
        .map_err(|e| super::with_context("diff add", &e))?;
    Ok(format!(
        "{} ({} round {round})",
        short_diff(id),
        short(task)
    ))
}

pub fn diff_get(id: i64, full: bool) -> Result<String, String> {
    let s = session(false)?;
    block_on(show(&s.db, id, full))
}

pub(super) async fn show(db: &Db, id: i64, full: bool) -> Result<String, String> {
    let d = select(db, "WHERE meta::id(id) = $id", json!({"id": id})).await?;
    let d = d
        .first()
        .ok_or_else(|| format!("no such diff {}", short_diff(id)))?;
    Ok(text::diff_text(d, full))
}

/// A task's diffs, or every diff of an epic's tasks, newest first.
pub fn diff_list(of: Ref) -> Result<String, String> {
    let s = session(false)?;
    block_on(listing(&s.db, of))
}

pub(super) async fn listing(db: &Db, of: Ref) -> Result<String, String> {
    let (rows, with_task) = match of {
        Ref::Task(id) => {
            fetch(db, id).await?;
            (
                select(
                    db,
                    "WHERE task_id = $id ORDER BY did DESC",
                    json!({"id": id}),
                )
                .await?,
                false,
            )
        }
        Ref::Epic(id) => {
            super::epics::fetch_epic(db, id).await?;
            let ids: Vec<i64> = select_rows(db, "WHERE epic_id = $id", json!({"id": id}))
                .await?
                .into_iter()
                .map(|r| r.tid)
                .collect();
            (
                select(
                    db,
                    "WHERE task_id IN $ids ORDER BY did DESC",
                    json!({"ids": ids}),
                )
                .await?,
                true,
            )
        }
    };
    Ok(text::diff_list_text(&rows, with_task))
}
