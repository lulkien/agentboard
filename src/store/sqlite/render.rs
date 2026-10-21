use super::{COLS, Ref, Row, STATUS_LIST, fetch, norm, now_ms, open, read_row, short, short_epic};
use crate::store::text::{self, BriefData, BriefEpic, EventRow};
use rusqlite::{Connection, params};

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
    sql.push_str(" ORDER BY CASE status WHEN 'blocking' THEN 0 WHEN 'not_done' THEN 1 WHEN 'needs_fix' THEN 2 WHEN 'working' THEN 3 WHEN 'review' THEN 4 WHEN 'ready' THEN 5 ELSE 6 END, priority, id LIMIT ?");
    args.push(limit.to_string());
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows: Vec<Row> = stmt
        .query_map(rusqlite::params_from_iter(args.iter()), read_row)
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<Row>>>()
        .map_err(|e| e.to_string())?;
    Ok(text::list_text(&rows))
}

/// `full` returns description, goal, report and note unclipped and verbatim.
pub fn get(id: i64, full: bool) -> Result<String, String> {
    let conn = open()?;
    let r = fetch(&conn, id)?;
    let epic = r
        .epic_id
        .map(|e| super::epics::fetch_epic(&conn, e))
        .transpose()?;
    let summary = status_summary(&conn, "parent_id", r.id, "children")?;
    let events = recent_events(&conn, "task_id", r.id, full, &text::task_hint(r.id))?;
    let newest = super::diffs::newest_diff(&conn, r.id)?;
    Ok(text::get_text(
        &r,
        epic.as_ref(),
        summary.as_deref(),
        newest.as_ref(),
        &events,
        full,
    ))
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
    let evs: Vec<EventRow> = stmt
        .query_map(params![id], |e| {
            Ok(EventRow {
                seq: e.get(0)?,
                actor: e.get(1)?,
                kind: e.get(2)?,
                body: e.get(3)?,
                ts: e.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(text::events_text(&evs, full, hint))
}

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
    Ok(text::summary_text(label, &counts))
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
            "SELECT {COLS} FROM tasks WHERE {col}=? ORDER BY id"
        ))
        .map_err(|e| e.to_string())?;
    let rows: Vec<Row> = stmt
        .query_map(params![id], read_row)
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(text::children_text(&label, &rows))
}

/// The 8 most recent completions, newest first. Rows completed in the same
/// millisecond come out by id, so the list (and which 8 survive the LIMIT) never
/// depends on scan order.
fn recent_dones(conn: &Connection, cutoff: i64) -> Result<Vec<(i64, Option<String>)>, String> {
    let mut stmt = conn
        .prepare("SELECT id,report FROM tasks WHERE status='done' AND updated_at>=? ORDER BY updated_at DESC, id LIMIT 8")
        .map_err(|e| e.to_string())?;
    stmt.query_map(params![cutoff], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

/// One-screen status: counts + one line per open task + recent completions.
pub fn brief(since_min: i64) -> Result<String, String> {
    let conn = open()?;
    let cutoff = now_ms() - since_min * 60_000;
    let mut stmt = conn
        .prepare("SELECT status, COUNT(*) FROM tasks WHERE status NOT IN ('done','dropped') GROUP BY status ORDER BY 1")
        .map_err(|e| e.to_string())?;
    let counts: Vec<(String, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let done_recent: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE status='done' AND updated_at>=?",
            params![cutoff],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let mut stmt = conn
        .prepare("SELECT id,title,path FROM epics WHERE status='open' ORDER BY id")
        .map_err(|e| e.to_string())?;
    let open_epics: Vec<(i64, String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let mut epics = Vec::new();
    for (id, title, path) in open_epics {
        let (done, total) = super::epics::counts(&conn, id)?;
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
        epics.push(BriefEpic {
            id,
            title,
            path,
            done,
            total,
            urgent,
        });
    }
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {COLS} FROM tasks WHERE status NOT IN ('done','dropped') AND epic_id IS NULL ORDER BY CASE status WHEN 'blocking' THEN 0 WHEN 'not_done' THEN 1 WHEN 'needs_fix' THEN 2 WHEN 'working' THEN 3 WHEN 'review' THEN 4 ELSE 5 END, priority, id LIMIT 25"
        ))
        .map_err(|e| e.to_string())?;
    let rows: Vec<Row> = stmt
        .query_map([], read_row)
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let dones = recent_dones(&conn, cutoff)?;
    Ok(text::brief_text(&BriefData {
        since_min,
        counts,
        done_recent,
        epics,
        rows,
        dones,
    }))
}

#[cfg(test)]
mod tests {
    use super::recent_dones;

    #[test]
    fn done_rows_with_the_same_updated_at_list_by_id_and_the_limit_keeps_the_lowest_ids() {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        c.execute_batch(crate::store::sqlite::SCHEMA).unwrap();
        let add = |id: i64, at: i64| {
            c.execute(
                "INSERT INTO tasks(id,title,status,report,created_at,updated_at) VALUES(?,?,'done',?,?,?)",
                rusqlite::params![id, format!("t{id}"), format!("r{id}"), at, at],
            )
            .unwrap();
        };
        // ten rows done in the same millisecond, inserted out of id order, then a newer one
        for id in [5, 2, 9, 1, 7, 10, 3, 8, 4, 6] {
            add(id, 1000);
        }
        add(11, 2000);
        let got: Vec<i64> = recent_dones(&c, 0)
            .unwrap()
            .into_iter()
            .map(|d| d.0)
            .collect();
        // newest first, then ties by id; the LIMIT of 8 cuts ids 7..10 off the tie group
        assert_eq!(got, [11, 1, 2, 3, 4, 5, 6, 7]);
    }
}
