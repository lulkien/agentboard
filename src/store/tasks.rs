use super::{DEFAULT_TTL_S, STATUS_LIST, fetch, get, log_event, norm, now_ms, open, short};
use crate::clip::{CAP_DESCRIPTION, CAP_GOAL, CAP_LAST, clip_marked, field};
use rusqlite::{OptionalExtension, params};
use std::time::Duration;

pub fn add(actor: &str, title: &str, description: &str, goal: &str, status: &str, assignee: Option<&str>, priority: i64, parent: Option<i64>) -> Result<String, String> {
    let conn = open()?;
    if let Some(p) = parent {
        fetch(&conn, p).map_err(|_| format!("add: no such parent task {}", short(p)))?;
    }
    // A task created already `working` belongs to its creator unless named.
    let assignee = assignee.or_else(|| (status == "working").then_some(actor));
    let now = now_ms();
    // A `working` task needs a lease, or a dead holder could never be reclaimed.
    let lease = (status == "working").then(|| now + DEFAULT_TTL_S * 1000);
    let started = (status == "working").then_some(now);
    conn.execute(
        "INSERT INTO tasks(title,description,goal,status,assignee,priority,parent,created_at,updated_at,lease_until,started_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
        params![title, description, goal, status, assignee, priority, parent, now, now, lease, started],
    )
    .map_err(|e| format!("add: {e}"))?;
    let id = conn.last_insert_rowid();
    log_event(&conn, id, actor, "add", title)?;
    Ok(format!("{} {status}", short(id)))
}

/// Claims a specific task, or the next ready one. Atomic: BEGIN IMMEDIATE + guarded UPDATE.
pub fn claim(actor: &str, id: Option<i64>, ttl_s: i64) -> Result<String, String> {
    let mut conn = open()?;
    let now = now_ms();
    let lease = now + ttl_s * 1000;
    for _ in 0..8 {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
        let cand: Option<(i64, String)> = {
            let (sql, arg): (String, Vec<i64>) = match id {
                Some(x) => (format!("SELECT id,status FROM tasks WHERE id=? AND (status='ready' OR (lease_until IS NOT NULL AND lease_until < ?))"), vec![x, now]),
                None => ("SELECT id,status FROM tasks WHERE status='ready' ORDER BY priority, id LIMIT 1".to_owned(), vec![]),
            };
            let mut stmt = tx.prepare(&sql).map_err(|e| e.to_string())?;
            let found = stmt
                .query_row(rusqlite::params_from_iter(arg.iter()), |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })
                .optional()
                .map_err(|e| e.to_string())?;
            found
        };
        let Some((task, _st)) = cand else {
            tx.commit().map_err(|e| e.to_string())?;
            return match id {
                Some(x) => {
                    // Distinguish "does not exist" from "somebody is on it".
                    let conn2 = open()?;
                    match fetch(&conn2, x) {
                        Ok(r) => Ok(format!(
                            "{} {} by {}{}, not claimable",
                            short(x),
                            r.status,
                            r.assignee.as_deref().unwrap_or("nobody"),
                            if r.lease_until.is_some() { " (lease)" } else { "" }
                        )),
                        Err(_) => Err(format!("no such task {}", short(x))),
                    }
                }
                None => Ok("no ready tasks".to_owned()),
            };
        };
        // The guard compares the OLD lease against `now`; comparing it against
        // the new lease would let a live claim be stolen.
        let changed = tx
            .execute(
                "UPDATE tasks SET status='working', assignee=?, lease_until=?, updated_at=?, started_at=COALESCE(started_at,?) WHERE id=? AND (status='ready' OR (lease_until IS NOT NULL AND lease_until < ?))",
                params![actor, lease, now, now, task, now],
            )
            .map_err(|e| e.to_string())?;
        if changed != 1 {
            tx.rollback().map_err(|e| e.to_string())?;
            continue;
        }
        log_event(&tx, task, actor, "claim", "")?;
        let r = fetch(&tx, task)?;
        tx.commit().map_err(|e| e.to_string())?;
        let mut out = format!("{} working (lease {}m) {}\n", short(r.id), ttl_s / 60, r.title);
        out.push_str(&field("description", &r.description, CAP_DESCRIPTION, false, &short(r.id)));
        out.push_str(&field("goal", &r.goal, CAP_GOAL, false, &short(r.id)));
        return Ok(out.trim_end().to_owned());
    }
    Err("claim contention: retry".to_owned())
}

/// One transactional edit of a task. Status moves, report, note, commit and
/// lease all land together or not at all.
pub struct Patch<'a> {
    pub status: Option<&'a str>,
    pub report: Option<&'a str>,
    pub note: Option<&'a str>,
    pub commit: Option<&'a str>,
    pub needs: Option<&'a str>,
    pub ttl_s: i64,
}

pub fn apply(actor: &str, id: i64, p: &Patch<'_>) -> Result<String, String> {
    let mut conn = open()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
    let cur = fetch(&tx, id)?;
    let now = now_ms();
    let status = match p.status {
        Some(raw) => norm(raw).ok_or_else(|| format!("bad status {raw:?} ({STATUS_LIST})"))?,
        None => cur.status.as_str(),
    };
    let moved = status != cur.status;
    if let Some(rp) = p.report {
        tx.execute("UPDATE tasks SET report=? WHERE id=?", params![rp, id]).map_err(|e| e.to_string())?;
    }
    if let Some(n) = p.note.filter(|n| !n.trim().is_empty()) {
        tx.execute("UPDATE tasks SET note=? WHERE id=?", params![n, id]).map_err(|e| e.to_string())?;
    }
    if let Some(c) = p.commit {
        tx.execute("UPDATE tasks SET commit_ref=? WHERE id=?", params![c, id]).map_err(|e| e.to_string())?;
    }
    let needs = p.needs.map(str::to_owned).or_else(|| (status == "blocking").then(|| "input".to_owned()));
    let ttl = if p.ttl_s > 0 { p.ttl_s } else { DEFAULT_TTL_S };
    match status {
        "working" => {
            // Only entering `working` (or a missing lease) sets one: plain notes and
            // reports must not keep a dead holder's lease alive. Heartbeat extends.
            let lease = match cur.lease_until {
                Some(old) if !moved => old,
                Some(old) => old.max(now + ttl * 1000),
                None => now + ttl * 1000,
            };
            tx.execute(
                "UPDATE tasks SET status=?, needs=?, updated_at=?, lease_until=?, assignee=COALESCE(assignee,?), started_at=COALESCE(started_at,?), completed_at=NULL WHERE id=?",
                params![status, needs, now, lease, actor, now, id],
            )
        }
        "done" | "dropped" => tx.execute(
            "UPDATE tasks SET status=?, needs=?, updated_at=?, lease_until=NULL, completed_at=COALESCE(completed_at,?) WHERE id=?",
            params![status, needs, now, now, id],
        ),
        "ready" => tx.execute(
            "UPDATE tasks SET status=?, needs=?, updated_at=?, lease_until=NULL, started_at=NULL, completed_at=NULL WHERE id=?",
            params![status, needs, now, id],
        ),
        // `needs_fix` waits on someone else's fix, so it holds no lease. Same for
        // blocking and review.
        _ => tx.execute(
            "UPDATE tasks SET status=?, needs=?, updated_at=?, lease_until=NULL WHERE id=?",
            params![status, needs, now, id],
        ),
    }
    .map_err(|e| e.to_string())?;
    let kind = match (moved, status) {
        (true, "done") => "done",
        (true, "review") => "review",
        (true, "blocking") => "block",
        (true, "needs_fix") => "needs_fix",
        (true, "working") => {
            match cur.status.as_str() {
                "blocking" => "answer",
                "ready" => "claim",
                _ => "edit",
            }
        }
        (true, _) => "edit",
        (false, _) => {
            if p.report.is_some() { "report" } else { "note" }
        }
    };
    let body = p.note.or(p.report).unwrap_or("");
    let seq = log_event(&tx, id, actor, kind, body)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(format!("{} {status} #{seq}", short(id)))
}

pub fn note(actor: &str, id: i64, body: &str) -> Result<String, String> {
    apply(actor, id, &Patch { status: None, report: None, note: Some(body), commit: None, needs: None, ttl_s: 0 })
}

/// Writes the worker's report without moving the task.
pub fn report(actor: &str, id: i64, body: &str) -> Result<String, String> {
    apply(actor, id, &Patch { status: None, report: Some(body), note: None, commit: None, needs: None, ttl_s: 0 })
}

pub fn ask(actor: &str, id: i64, question: &str) -> Result<String, String> {
    apply(actor, id, &Patch { status: Some("blocking"), report: None, note: Some(question), commit: None, needs: Some("input"), ttl_s: 0 })
}

pub fn answer(actor: &str, id: i64, text: &str) -> Result<String, String> {
    // Resume with a fresh lease in the same transaction: no steal window.
    apply(actor, id, &Patch { status: Some("working"), report: None, note: Some(text), commit: None, needs: None, ttl_s: DEFAULT_TTL_S })
}

/// Arbitrary status moves for reopen/drop.

pub fn finish(actor: &str, id: i64, status: &str, summary: &str, commit: Option<&str>) -> Result<String, String> {
    apply(actor, id, &Patch { status: Some(status), report: Some(summary), note: None, commit, needs: None, ttl_s: 0 })
}

pub fn block(actor: &str, id: i64, reason: &str) -> Result<String, String> {
    apply(actor, id, &Patch { status: Some("blocking"), report: None, note: Some(reason), commit: None, needs: None, ttl_s: 0 })
}

pub fn heartbeat(actor: &str, id: i64, ttl_s: i64) -> Result<String, String> {
    let conn = open()?;
    let lease = now_ms() + ttl_s * 1000;
    // Only the holder may extend: a stale owner must not refresh a lease that
    // another agent reclaimed after expiry.
    let n = conn
        .execute(
            "UPDATE tasks SET lease_until=?, updated_at=? WHERE id=? AND status='working' AND assignee=?",
            params![lease, now_ms(), id, actor],
        )
        .map_err(|e| e.to_string())?;
    if n != 1 {
        return Err(format!("{} not working or not held by {actor}", short(id)));
    }
    Ok(format!("ok {} lease {}m", short(id), ttl_s / 60))
}

/// Blocks server-side (no agent-side polling) until the task changes or the timeout hits.
///
/// `after` is the last event seq the caller already saw; without it we take the
/// current watermark, which is correct for "wait for the next thing to happen".
pub async fn wait(id: i64, after: Option<i64>, timeout_ms: u64) -> Result<String, String> {
    let start_ev: i64 = {
        let conn = open()?;
        fetch(&conn, id)?;
        match after {
            Some(seq) => seq,
            None => conn
                .query_row("SELECT COALESCE(MAX(seq),0) FROM events WHERE task_id=?", params![id], |r| r.get(0))
                .map_err(|e| e.to_string())?,
        }
    };
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        {
            let conn = open()?;
            let r = fetch(&conn, id)?;
            let mut stmt = conn
                .prepare("SELECT seq,actor,kind,body FROM events WHERE task_id=? AND seq>? ORDER BY seq")
                .map_err(|e| e.to_string())?;
            let evs: Vec<(i64, String, String, String)> = stmt
                .query_map(params![id, start_ev], |e| Ok((e.get(0)?, e.get(1)?, e.get(2)?, e.get(3)?)))
                .map_err(|e| e.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?;
            if !evs.is_empty() {
                let mut out = format!("{} {}\n", short(id), r.status);
                for (seq, actor, kind, body) in evs {
                    // The delta is what the caller asked for (answers live here): never clipped.
                    out.push_str(&format!("#{seq} {actor} {kind}: {body}\n"));
                }
                return Ok(out.trim_end().to_owned());
            }
            if std::time::Instant::now() >= deadline {
                // Never return blind: hand back the tail so the caller can decide.
                return Ok(format!(
                    "{} {} unchanged after {}s\nlast: {}",
                    short(id),
                    r.status,
                    timeout_ms / 1000,
                    clip_marked(&get(id, false).unwrap_or_default(), CAP_LAST, &format!("task_get id={} for the record", short(id)))
                ));
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
