use super::render::stamp;
use super::{
    DEFAULT_TTL_S, STATUS_LIST, Subject, fetch, log_event, norm, now_ms, open, short, short_epic,
};
use crate::clip::{CAP_DESCRIPTION, CAP_GOAL, field};
use rusqlite::{OptionalExtension, params};

/// A task as the caller describes it; `add` supplies id, timestamps and lease.
pub struct NewTask<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub goal: &'a str,
    pub status: &'a str,
    pub assignee: Option<&'a str>,
    pub priority: i64,
    pub epic: Option<i64>,
    pub parent: Option<i64>,
}

/// The epic a new task lands in: the explicit one, else the parent's. An
/// explicit epic that disagrees with the parent's is an error: a subtask lives in
/// its parent's epic.
fn resolve_epic(
    conn: &rusqlite::Connection,
    epic: Option<i64>,
    parent: Option<i64>,
) -> Result<Option<i64>, String> {
    let parent_epic = match parent {
        Some(p) => {
            let row =
                fetch(conn, p).map_err(|_| format!("add: no such parent task {}", short(p)))?;
            Some((p, row.epic_id))
        }
        None => None,
    };
    if let Some(e) = epic {
        super::epics::fetch_epic(conn, e)
            .map_err(|_| format!("add: no such epic {}", short_epic(e)))?;
    }
    match (epic, parent_epic) {
        (Some(e), Some((p, pe))) if pe != Some(e) => Err(format!(
            "add: parent {} belongs to {}, not {} (a subtask lives in its parent's epic)",
            short(p),
            pe.map_or("no epic".to_owned(), short_epic),
            short_epic(e)
        )),
        (None, Some((_, pe))) => Ok(pe),
        (e, _) => Ok(e),
    }
}

pub fn add(actor: &str, task: &NewTask<'_>) -> Result<String, String> {
    let NewTask {
        title,
        description,
        goal,
        status,
        assignee,
        priority,
        epic,
        parent,
    } = *task;
    let mut conn = open()?;
    // Read the parent/epic and insert in one write transaction: the invariant
    // must hold for the row that lands, not for the row we looked at.
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let epic = resolve_epic(&tx, epic, parent)?;
    // A task created already `working` belongs to its creator unless named.
    let assignee = assignee.or_else(|| (status == "working").then_some(actor));
    let now = now_ms();
    // A `working` task needs a lease, or a dead holder could never be reclaimed.
    let lease = (status == "working").then(|| now + DEFAULT_TTL_S * 1000);
    let started = (status == "working").then_some(now);
    tx.execute(
        "INSERT INTO tasks(title,description,goal,status,assignee,priority,epic_id,parent_id,created_at,updated_at,lease_until,started_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
        params![title, description, goal, status, assignee, priority, epic, parent, now, now, lease, started],
    )
    .map_err(|e| format!("add: {e}"))?;
    let id = tx.last_insert_rowid();
    log_event(&tx, Subject::Task(id), actor, "add", title)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(format!("{} {status}", short(id)))
}

/// Re-parents an existing task (`None` clears it). The only write that can make
/// a cycle, so the walk up the ancestors happens here, in the same transaction.
pub fn set_parent(actor: &str, id: i64, parent: Option<i64>) -> Result<String, String> {
    let mut conn = open()?;
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let task = fetch(&tx, id)?;
    if let Some(p) = parent {
        if p == id {
            return Err(format!("{} cannot be its own parent", short(id)));
        }
        let new_parent = fetch(&tx, p)?;
        if new_parent.epic_id != task.epic_id {
            return Err(format!(
                "cross-epic parent: {} belongs to {} but {} belongs to {}",
                short(id),
                task.epic_id.map_or("no epic".to_owned(), short_epic),
                short(p),
                new_parent.epic_id.map_or("no epic".to_owned(), short_epic)
            ));
        }
        // Walk up from the new parent: meeting `id` means `id` is its ancestor.
        let mut at = new_parent.parent_id;
        let mut steps = 0;
        while let Some(a) = at {
            if a == id || steps > 10_000 {
                return Err(format!(
                    "cycle: {} is already an ancestor of {}",
                    short(id),
                    short(p)
                ));
            }
            at = fetch(&tx, a)?.parent_id;
            steps += 1;
        }
    }
    tx.execute(
        "UPDATE tasks SET parent_id=?, updated_at=? WHERE id=?",
        params![parent, now_ms(), id],
    )
    .map_err(|e| e.to_string())?;
    let body = parent.map_or("parent cleared".to_owned(), |p| {
        format!("parent -> {}", short(p))
    });
    log_event(&tx, Subject::Task(id), actor, "edit", &body)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(parent.map_or(format!("{} no parent", short(id)), |p| {
        format!("{} parent {}", short(id), short(p))
    }))
}

/// Why a claim failed. `Held` is the legitimate refusal (somebody holds the
/// task); the MCP layer reports it differently from a bad request or a DB error.
pub enum ClaimError {
    Held(String),
    Other(String),
}

impl From<String> for ClaimError {
    fn from(e: String) -> Self {
        Self::Other(e)
    }
}

impl std::fmt::Display for ClaimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (Self::Held(m) | Self::Other(m)) = self;
        f.write_str(m)
    }
}

/// Claims a specific task, or the next ready one. Atomic: BEGIN IMMEDIATE + guarded UPDATE.
pub fn claim(actor: &str, id: Option<i64>, ttl_s: i64) -> Result<String, ClaimError> {
    let mut conn = open()?;
    let now = now_ms();
    let lease = now + ttl_s * 1000;
    for _ in 0..8 {
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let cand: Option<(i64, String)> = {
            let (sql, arg): (String, Vec<i64>) = match id {
                Some(x) => ("SELECT id,status FROM tasks WHERE id=? AND (status='ready' OR (lease_until IS NOT NULL AND lease_until < ?))".to_string(), vec![x, now]),
                None => ("SELECT id,status FROM tasks WHERE status='ready' ORDER BY priority, id LIMIT 1".to_owned(), vec![]),
            };
            let mut stmt = tx.prepare(&sql).map_err(|e| e.to_string())?;

            stmt.query_row(rusqlite::params_from_iter(arg.iter()), |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })
            .optional()
            .map_err(|e| e.to_string())?
        };
        let Some((task, _st)) = cand else {
            tx.commit().map_err(|e| e.to_string())?;
            return match id {
                Some(x) => {
                    // Distinguish "does not exist" from "somebody is on it".
                    let conn2 = open()?;
                    match fetch(&conn2, x) {
                        // A refusal is an error: `agentboard claim T7 && ...` must not
                        // read "somebody else has it" as "claimed".
                        Ok(r) => Err(ClaimError::Held(format!(
                            "{} {} by {}{} — manager: agentboard status {} ready",
                            short(x),
                            r.status,
                            r.assignee.as_deref().unwrap_or("nobody"),
                            r.lease_until.map_or(String::new(), |l| format!(
                                " (lease {}m left, until {}Z)",
                                ((l - now_ms()).max(0) + 59_999) / 60_000,
                                stamp(l)
                            )),
                            short(x)
                        ))),
                        Err(_) => Err(format!("no such task {}", short(x)).into()),
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
        log_event(&tx, Subject::Task(task), actor, "claim", "")?;
        let r = fetch(&tx, task)?;
        tx.commit().map_err(|e| e.to_string())?;
        let mut out = format!(
            "{} working (lease {}m) {}\n",
            short(r.id),
            ttl_s / 60,
            r.title
        );
        // claim is the worker's entry point, not a display path: the prompt
        // arrives byte-for-byte (newlines, fences) and uncapped.
        out.push_str(&field(
            "description",
            &r.description,
            CAP_DESCRIPTION,
            true,
            &short(r.id),
        ));
        out.push_str(&field("goal", &r.goal, CAP_GOAL, true, &short(r.id)));
        return Ok(out);
    }
    Err("claim contention: retry".to_owned().into())
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
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let cur = fetch(&tx, id)?;
    let now = now_ms();
    let status = match p.status {
        Some(raw) => norm(raw).ok_or_else(|| format!("bad status {raw:?} ({STATUS_LIST})"))?,
        None => cur.status.as_str(),
    };
    let moved = status != cur.status;
    if let Some(rp) = p.report {
        tx.execute("UPDATE tasks SET report=? WHERE id=?", params![rp, id])
            .map_err(|e| e.to_string())?;
    }
    if let Some(n) = p.note.filter(|n| !n.trim().is_empty()) {
        tx.execute("UPDATE tasks SET note=? WHERE id=?", params![n, id])
            .map_err(|e| e.to_string())?;
    }
    if let Some(c) = p.commit {
        tx.execute("UPDATE tasks SET commit_ref=? WHERE id=?", params![c, id])
            .map_err(|e| e.to_string())?;
    }
    let needs = p
        .needs
        .map(str::to_owned)
        .or_else(|| (status == "blocking").then(|| "input".to_owned()));
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
        (true, "working") => match cur.status.as_str() {
            "blocking" => "answer",
            "ready" => "claim",
            _ => "edit",
        },
        (true, _) => "edit",
        (false, _) => {
            if p.report.is_some() {
                "report"
            } else {
                "note"
            }
        }
    };
    let body = p.note.or(p.report).unwrap_or("");
    let seq = log_event(&tx, Subject::Task(id), actor, kind, body)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(format!("{} {status} #{seq}", short(id)))
}

pub fn note(actor: &str, id: i64, body: &str) -> Result<String, String> {
    apply(
        actor,
        id,
        &Patch {
            status: None,
            report: None,
            note: Some(body),
            commit: None,
            needs: None,
            ttl_s: 0,
        },
    )
}

/// Writes the worker's report without moving the task.
pub fn report(actor: &str, id: i64, body: &str) -> Result<String, String> {
    apply(
        actor,
        id,
        &Patch {
            status: None,
            report: Some(body),
            note: None,
            commit: None,
            needs: None,
            ttl_s: 0,
        },
    )
}

pub fn ask(actor: &str, id: i64, question: &str) -> Result<String, String> {
    apply(
        actor,
        id,
        &Patch {
            status: Some("blocking"),
            report: None,
            note: Some(question),
            commit: None,
            needs: Some("input"),
            ttl_s: 0,
        },
    )
}

pub fn answer(actor: &str, id: i64, text: &str) -> Result<String, String> {
    // Resume with a fresh lease in the same transaction: no steal window.
    apply(
        actor,
        id,
        &Patch {
            status: Some("working"),
            report: None,
            note: Some(text),
            commit: None,
            needs: None,
            ttl_s: DEFAULT_TTL_S,
        },
    )
}

/// Arbitrary status moves for reopen/drop.
pub fn finish(
    actor: &str,
    id: i64,
    status: &str,
    summary: &str,
    commit: Option<&str>,
) -> Result<String, String> {
    apply(
        actor,
        id,
        &Patch {
            status: Some(status),
            report: Some(summary),
            note: None,
            commit,
            needs: None,
            ttl_s: 0,
        },
    )
}

pub fn block(actor: &str, id: i64, reason: &str) -> Result<String, String> {
    apply(
        actor,
        id,
        &Patch {
            status: Some("blocking"),
            report: None,
            note: Some(reason),
            commit: None,
            needs: None,
            ttl_s: 0,
        },
    )
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
