//! Diffs on the SurrealDB backend: the port of `sqlite/diffs.rs`. Append-only, so a
//! write is one insert plus its event in one transaction under the write lock, and
//! nothing here edits or deletes a row. An evidence-only row names a patch; it is
//! not one.

use super::tasks::{Txn, fetch, select_rows};
use super::{Db, block_on, db_err, session};
use crate::store::text::{self, DiffRow, Submission};
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
    reviews: Option<i64>,
}

impl From<Row> for DiffRow {
    fn from(r: Row) -> Self {
        Self {
            id: r.did,
            task_id: r.task_id,
            round: r.round,
            body: r.body,
            created_at: r.created_at,
            reviews: r.reviews,
        }
    }
}

const SELECT_DIFF: &str =
    "SELECT meta::id(id) AS did, task_id, round, body, created_at, reviews FROM diffs";

async fn select(db: &Db, tail: &str, binds: serde_json::Value) -> Result<Vec<DiffRow>, String> {
    let mut r = db
        .query(format!("{SELECT_DIFF} {tail}"))
        .bind(binds)
        .await
        .map_err(|e| db_err(&e))?;
    let rows: Vec<Row> = r.take(0).map_err(|e| db_err(&e))?;
    Ok(rows.into_iter().map(DiffRow::from).collect())
}

/// The newest plain diff of a task, for the line `get` shows. Evidence-only rows are
/// not patches and are skipped.
pub(super) async fn newest_diff(db: &Db, task: i64) -> Result<Option<DiffRow>, String> {
    Ok(select(
        db,
        "WHERE task_id = $id AND reviews = NONE ORDER BY did DESC LIMIT 1",
        json!({"id": task}),
    )
    .await?
    .into_iter()
    .next())
}

pub fn diff_add(
    actor: &str,
    task: i64,
    body: &str,
    reviews: Option<i64>,
) -> Result<String, String> {
    if body.trim().is_empty() {
        return Err(if reviews.is_some() {
            text::EMPTY_EVIDENCE.to_owned()
        } else {
            text::EMPTY_DIFF.to_owned()
        });
    }
    let s = session(true)?;
    block_on(async {
        if let Some(named) = reviews {
            check_evidence(&s.db, task, named).await?;
        }
        record(&s.db, actor, task, body, reviews).await
    })
}

/// Refuses an evidence-only submission unless the named diff is the newest patch
/// recorded on the submitting row. An earlier patch, a diff on another row and an
/// evidence-only row are all refused by name.
pub(super) async fn check_evidence(db: &Db, task: i64, named: i64) -> Result<(), String> {
    let rows = select(db, "WHERE meta::id(id) = $id", json!({"id": named})).await?;
    let diff = rows
        .first()
        .ok_or_else(|| format!("no such diff {}", short_diff(named)))?;
    if diff.task_id != task {
        return Err(text::evidence_not_on_row(task, named, diff.task_id));
    }
    if diff.reviews.is_some() {
        return Err(text::evidence_not_a_patch(task, named));
    }
    let Some(newest) = newest_diff(db, task).await? else {
        return Err(format!(
            "diff add --evidence-only: no patch recorded on {}",
            short(task)
        ));
    };
    if newest.id != named {
        return Err(text::evidence_stale(task, named, newest.id));
    }
    Ok(())
}

/// Inserts the row and its event in one transaction. The caller holds the write lock.
pub(super) async fn record(
    db: &Db,
    actor: &str,
    task: i64,
    body: &str,
    reviews: Option<i64>,
) -> Result<String, String> {
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
    let mut content = json!({
        "task_id": task, "round": round, "body": body, "created_at": now_ms(),
    });
    if let Some(named) = reviews {
        content["reviews"] = named.into();
    }
    let content = tx.p(content);
    tx.raw(&format!(
        "CREATE type::thing('diffs', {idp}) CONTENT {content};"
    ));
    let taskp = tx.p(task);
    tx.event(
        &taskp,
        actor,
        "diff",
        &text::diff_event(id, round, body, reviews),
    );
    tx.run(db, "$seq")
        .await
        .map_err(|e| super::with_context("diff add", &e))?;
    Ok(format!(
        "{} ({} round {round})",
        short_diff(id),
        short(task)
    ))
}

/// The newest submission recorded after the task's newest claim, if any. A plain
/// diff wins over an evidence-only row: the patch stays the artifact of record.
pub(super) async fn submission_since_claim(
    db: &Db,
    task: i64,
) -> Result<Option<Submission>, String> {
    let claim = super::tasks::last_event_seq(db, task, "claim")
        .await?
        .unwrap_or(0);
    let mut r = db
        .query(
            "SELECT VALUE meta::id(id) FROM events WHERE task_id = $id AND kind = 'diff' AND meta::id(id) > $claim",
        )
        .bind(json!({"id": task, "claim": claim}))
        .await
        .map_err(|e| db_err(&e))?;
    let seqs: Vec<i64> = r.take(0).map_err(|e| db_err(&e))?;
    let since = seqs.len() as i64;
    if since == 0 {
        return Ok(None);
    }
    // One `diff` event is logged per diff row, so the newest `since` rows are the
    // submissions of this round.
    let rows = select(
        db,
        &format!("WHERE task_id = $id ORDER BY did DESC LIMIT {since}"),
        json!({"id": task}),
    )
    .await?;
    if let Some(patch) = rows.iter().find(|r| r.reviews.is_none()) {
        return Ok(Some(Submission::patch(patch.clone())));
    }
    let Some(record) = rows.first() else {
        return Ok(None);
    };
    let named = record.reviews.expect("evidence row names its diff");
    check_evidence(db, task, named).await?;
    let diff = select(db, "WHERE meta::id(id) = $id", json!({"id": named}))
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| format!("no such diff {}", short_diff(named)))?;
    Ok(Some(Submission {
        record: record.clone(),
        diff,
    }))
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
