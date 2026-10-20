//! `brief` on the SurrealDB backend: the port of `sqlite/render.rs::brief`. Loads
//! plain values and hands them to `text::brief_text`, the same renderer the sqlite
//! build uses. A read: it takes no lock.

use super::epics::counts;
use super::tasks::select_rows;
use super::{block_on, db_err, session};
use crate::store::now_ms;
use crate::store::text::{self, BriefData, BriefEpic, TaskRow};
use serde::Deserialize;
use serde_json::json;

pub fn brief(since_min: i64) -> Result<String, String> {
    let s = session(false)?;
    block_on(async {
        let cutoff = now_ms() - since_min * 60_000;
        #[derive(Deserialize)]
        struct Count {
            status: String,
            n: i64,
        }
        let mut r = s
            .db
            .query("SELECT status, count() AS n FROM tasks WHERE status NOT IN ['done', 'dropped'] GROUP BY status")
            .await
            .map_err(|e| db_err(&e))?;
        let counts_rows: Vec<Count> = r.take(0).map_err(|e| db_err(&e))?;
        // ordered by status name, byte order, as the sqlite `ORDER BY 1`
        let mut open_counts: Vec<(String, i64)> =
            counts_rows.into_iter().map(|c| (c.status, c.n)).collect();
        open_counts.sort();

        #[derive(Deserialize)]
        struct Done {
            tid: i64,
            report: Option<String>,
        }
        let mut r = s
            .db
            .query("SELECT meta::id(id) AS tid, report, updated_at FROM tasks WHERE status = 'done' AND updated_at >= $cutoff ORDER BY updated_at DESC, tid")
            .bind(("cutoff", cutoff))
            .await
            .map_err(|e| db_err(&e))?;
        let done_rows: Vec<Done> = r.take(0).map_err(|e| db_err(&e))?;
        let done_recent = i64::try_from(done_rows.len()).unwrap_or(i64::MAX);
        let dones: Vec<(i64, Option<String>)> = done_rows
            .into_iter()
            .take(8)
            .map(|d| (d.tid, d.report))
            .collect();

        // One line per open epic: progress, the project path, and only the counts a
        // manager must act on.
        #[derive(Deserialize)]
        struct E {
            tid: i64,
            title: String,
            path: String,
        }
        let mut r = s
            .db
            .query("SELECT meta::id(id) AS tid, title, path FROM epics WHERE status = 'open' ORDER BY tid")
            .await
            .map_err(|e| db_err(&e))?;
        let open_epics: Vec<E> = r.take(0).map_err(|e| db_err(&e))?;
        let mut epics = Vec::new();
        for e in open_epics {
            let (done, total) = counts(&s.db, e.tid).await?;
            #[derive(Deserialize)]
            struct St {
                status: String,
            }
            let mut r =
                s.db.query("SELECT status FROM tasks WHERE epic_id = $id")
                    .bind(("id", e.tid))
                    .await
                    .map_err(|e| db_err(&e))?;
            let sts: Vec<St> = r.take(0).map_err(|e| db_err(&e))?;
            let urgent: Vec<String> = ["blocking", "needs_fix"]
                .iter()
                .filter_map(|want| {
                    let n = sts.iter().filter(|t| t.status == *want).count();
                    (n > 0).then(|| format!("{n} {want}"))
                })
                .collect();
            epics.push(BriefEpic {
                id: e.tid,
                title: e.title,
                path: e.path,
                done,
                total,
                urgent,
            });
        }

        let mut rows = select_rows(
            &s.db,
            "WHERE status NOT IN ['done', 'dropped'] AND epic_id = NONE",
            json!({}),
        )
        .await?;
        rows.sort_by_key(|r| (text::brief_rank(&r.status), r.priority, r.tid));
        rows.truncate(25);
        let rows: Vec<TaskRow> = rows.into_iter().map(TaskRow::from).collect();

        Ok(text::brief_text(&BriefData {
            since_min,
            counts: open_counts,
            done_recent,
            epics,
            rows,
            dones,
        }))
    })
}
