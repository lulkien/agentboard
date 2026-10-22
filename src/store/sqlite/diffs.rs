//! Diffs: what a coder submitted for review, kept verbatim and append-only. There is
//! no edit and no delete here, so a row outlives the task being dropped. A reviewer
//! reads the newest patch; an evidence-only row names that patch and is never one.

use super::{Ref, Subject, fetch, log_event, now_ms, open};
use crate::store::text::{self, DiffRow, Submission};
use crate::store::{short, short_diff};
use rusqlite::{Connection, OptionalExtension, params};

const COLS: &str = "id,task_id,round,body,created_at,reviews";

pub(super) fn read_diff(r: &rusqlite::Row<'_>) -> rusqlite::Result<DiffRow> {
    Ok(DiffRow {
        id: r.get(0)?,
        task_id: r.get(1)?,
        round: r.get(2)?,
        body: r.get(3)?,
        created_at: r.get(4)?,
        reviews: r.get(5)?,
    })
}

/// The newest plain diff of a task, for the line `get` shows. Evidence-only rows are
/// not patches and are skipped.
pub(super) fn newest_diff(conn: &Connection, task: i64) -> Result<Option<DiffRow>, String> {
    conn.query_row(
        &format!(
            "SELECT {COLS} FROM diffs WHERE task_id=? AND reviews IS NULL ORDER BY id DESC LIMIT 1"
        ),
        params![task],
        read_diff,
    )
    .optional()
    .map_err(|e| e.to_string())
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
    let mut conn = open()?;
    let tx = super::write_transaction(&mut conn)?;
    if let Some(named) = reviews {
        check_evidence(&tx, task, named)?;
    }
    let out = record(&tx, actor, task, body, reviews)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(out)
}

/// Refuses an evidence-only submission unless the named diff is the newest patch
/// recorded on the submitting row. An earlier patch, a diff on another row and an
/// evidence-only row are all refused by name.
pub(super) fn check_evidence(conn: &Connection, task: i64, named: i64) -> Result<(), String> {
    let diff = conn
        .query_row(
            &format!("SELECT {COLS} FROM diffs WHERE id=?"),
            params![named],
            read_diff,
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no such diff {}", short_diff(named)))?;
    if diff.task_id != task {
        return Err(text::evidence_not_on_row(task, named, diff.task_id));
    }
    if diff.reviews.is_some() {
        return Err(text::evidence_not_a_patch(task, named));
    }
    let Some(newest) = newest_diff(conn, task)? else {
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

/// Inserts the row and its event on `conn`, which is the caller's write transaction.
pub(super) fn record(
    conn: &Connection,
    actor: &str,
    task: i64,
    body: &str,
    reviews: Option<i64>,
) -> Result<String, String> {
    let round = fetch(conn, task)?.reopen_count + 1;
    conn.execute(
        "INSERT INTO diffs(task_id,round,body,created_at,reviews) VALUES(?,?,?,?,?)",
        params![task, round, body, now_ms(), reviews],
    )
    .map_err(|e| format!("diff add: {e}"))?;
    let id = conn.last_insert_rowid();
    log_event(
        conn,
        Subject::Task(task),
        actor,
        "diff",
        &text::diff_event(id, round, body, reviews),
    )?;
    Ok(format!(
        "{} ({} round {round})",
        short_diff(id),
        short(task)
    ))
}

/// The newest submission recorded after the task's newest claim, if any. A plain
/// diff wins over an evidence-only row: the patch stays the artifact of record.
/// Called inside the gate's write transaction.
pub(super) fn submission_since_claim(
    conn: &Connection,
    task: i64,
) -> Result<Option<Submission>, String> {
    let since: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events WHERE task_id=?1 AND kind='diff' AND seq > COALESCE((SELECT MAX(seq) FROM events WHERE task_id=?1 AND kind='claim'),0)",
            params![task],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if since <= 0 {
        return Ok(None);
    }
    // One `diff` event is logged per diff row, so the newest `since` rows are the
    // submissions of this round.
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {COLS} FROM diffs WHERE task_id=? ORDER BY id DESC LIMIT ?"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![task, since], read_diff)
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    if let Some(patch) = rows.iter().find(|r| r.reviews.is_none()) {
        return Ok(Some(Submission::patch(patch.clone())));
    }
    let Some(record) = rows.first() else {
        return Ok(None);
    };
    let named = record.reviews.expect("evidence row names its diff");
    let diff = conn
        .query_row(
            &format!("SELECT {COLS} FROM diffs WHERE id=?"),
            params![named],
            read_diff,
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no such diff {}", short_diff(named)))?;
    check_evidence(conn, task, named)?;
    Ok(Some(Submission {
        record: record.clone(),
        diff,
    }))
}

pub fn diff_get(id: i64, full: bool) -> Result<String, String> {
    show(&open()?, id, full)
}

fn show(conn: &Connection, id: i64, full: bool) -> Result<String, String> {
    let d = conn
        .query_row(
            &format!("SELECT {COLS} FROM diffs WHERE id=?"),
            params![id],
            read_diff,
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no such diff {}", short_diff(id)))?;
    Ok(text::diff_text(&d, full))
}

/// A task's diffs, or every diff of an epic's tasks, newest first.
pub fn diff_list(of: Ref) -> Result<String, String> {
    listing(&open()?, of)
}

fn listing(conn: &Connection, of: Ref) -> Result<String, String> {
    let (sql, id, with_task) = match of {
        Ref::Task(id) => {
            fetch(conn, id)?;
            (
                format!("SELECT {COLS} FROM diffs WHERE task_id=? ORDER BY id DESC"),
                id,
                false,
            )
        }
        Ref::Epic(id) => {
            super::epics::fetch_epic(conn, id)?;
            (
                format!(
                    "SELECT {COLS} FROM diffs WHERE task_id IN (SELECT id FROM tasks WHERE epic_id=?) ORDER BY id DESC"
                ),
                id,
                true,
            )
        }
    };
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![id], read_diff)
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(text::diff_list_text(&rows, with_task))
}

#[cfg(test)]
mod tests {
    use super::super::SCHEMA;
    use super::*;

    #[test]
    fn diffs_round_trip_with_the_round_from_the_reopen_count() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(SCHEMA).unwrap();
        let now = now_ms();
        c.execute(
            "INSERT INTO tasks(title,created_at,updated_at,reopen_count) VALUES('t',?,?,2)",
            params![now, now],
        )
        .unwrap();
        assert_eq!(listing(&c, Ref::Task(1)).unwrap(), "(no diffs)");
        assert_eq!(
            record(&c, "a", 1, "one\ntwo\n", None).unwrap(),
            "D1 (T1 round 3)"
        );
        let long = "x".repeat(5000);
        assert_eq!(record(&c, "a", 1, &long, None).unwrap(), "D2 (T1 round 3)");
        let l = listing(&c, Ref::Task(1)).unwrap();
        let lines: Vec<_> = l.lines().collect();
        assert!(
            lines[0].starts_with("D2 round 3 ") && lines[0].ends_with(" 1 lines"),
            "{l}"
        );
        assert!(
            lines[1].starts_with("D1 round 3 ") && lines[1].ends_with(" 2 lines"),
            "{l}"
        );
        let clipped = show(&c, 2, false).unwrap();
        assert!(
            clipped.contains("[truncated: 4000 of 5000 chars;"),
            "{clipped}"
        );
        let full = show(&c, 2, true).unwrap();
        assert!(full.ends_with(&long) && !full.contains("truncated"));
        assert!(show(&c, 1, true).unwrap().ends_with("one\ntwo\n"));
        assert_eq!(newest_diff(&c, 1).unwrap().unwrap().id, 2);
        assert!(show(&c, 9, false).unwrap_err().contains("no such diff D9"));
        assert!(
            record(&c, "a", 9, "x", None)
                .unwrap_err()
                .contains("no such task T9")
        );
        let ev: String = c
            .query_row(
                "SELECT body FROM events WHERE task_id=1 AND kind='diff' ORDER BY seq LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ev, "D1 round 3 2 lines");
    }

    #[test]
    fn an_evidence_only_row_is_marked_and_never_the_newest_patch() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(SCHEMA).unwrap();
        let now = now_ms();
        c.execute(
            "INSERT INTO tasks(title,created_at,updated_at) VALUES('t',?,?)",
            params![now, now],
        )
        .unwrap();
        record(&c, "a", 1, "a patch", None).unwrap();
        record(&c, "a", 1, "re-ran: cargo test ok", Some(1)).unwrap();
        let newest = newest_diff(&c, 1).unwrap().unwrap();
        assert_eq!(
            (newest.id, newest.reviews),
            (1, None),
            "evidence is not a patch"
        );
        let l = listing(&c, Ref::Task(1)).unwrap();
        assert!(l.starts_with("D2 evidence-only -> D1 "), "{l}");
        assert!(l.contains("D1 round 1 "), "{l}");
        let shown = show(&c, 2, true).unwrap();
        assert!(shown.contains("evidence-only, reviews D1"), "{shown}");
        assert!(shown.contains("not a diff"), "{shown}");
        let ev: String = c
            .query_row(
                "SELECT body FROM events WHERE task_id=1 AND kind='diff' ORDER BY seq DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ev, "D2 round 1 evidence-only -> D1 1 lines");
    }
}
