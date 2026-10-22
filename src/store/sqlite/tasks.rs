use super::{
    DEFAULT_TTL_S, STATUS_LIST, Subject, fetch, field_names, log_event, norm, now_ms, open, short,
    short_epic, text_change, value_change,
};
use crate::clip::{CAP_DESCRIPTION, CAP_GOAL, field};
use crate::store::board::{ClaimError, NewTask, Patch, TaskEdit};
use crate::store::text::{
    MAX_CHAIN, Submission, cap_cascade_note, cap_note, cap_refusal, cap_skipped_note, cascades_to,
    chain_refusal, epic_gap, gate_refusal, held_text, norm_branch, normalize_path, reopen_after,
    review_description, review_filed_note, review_goal, review_resubmitted_note, review_title,
    workdir_text,
};
use rusqlite::{OptionalExtension, params};

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

/// Levels from `id` up to its root, itself included: a parentless task is level 1.
fn chain_level(conn: &rusqlite::Connection, id: i64) -> Result<usize, String> {
    let mut level = 1;
    let mut at = fetch(conn, id)?.parent_id;
    while let Some(a) = at {
        if level > 10_000 {
            return Err(format!("cycle above {}", short(id)));
        }
        level += 1;
        at = fetch(conn, a)?.parent_id;
    }
    Ok(level)
}

/// Levels of the subtree rooted at `id`, itself included: a leaf is 1.
fn subtree_height(conn: &rusqlite::Connection, id: i64) -> Result<usize, String> {
    let mut height = 0;
    let mut level = vec![id];
    while !level.is_empty() {
        height += 1;
        if height > 10_000 {
            return Err(format!("cycle below {}", short(id)));
        }
        let mut next = Vec::new();
        let mut stmt = conn
            .prepare("SELECT id FROM tasks WHERE parent_id=?")
            .map_err(|e| e.to_string())?;
        for at in &level {
            let kids = stmt
                .query_map(params![at], |r| r.get::<_, i64>(0))
                .map_err(|e| e.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?;
            next.extend(kids);
        }
        level = next;
    }
    Ok(height)
}

/// Refuses a subtree `height` levels deep (1 for a new row) under `parent` when its
/// deepest row would pass the last level of the chain.
fn check_depth(conn: &rusqlite::Connection, parent: i64, height: usize) -> Result<(), String> {
    let level = chain_level(conn, parent)?;
    if level + height > MAX_CHAIN {
        if new_chain_root(conn, parent)?.is_some() {
            return Err(crate::store::chain::depth_refusal(parent, level, height));
        }
        return Err(chain_refusal(parent, level, height));
    }
    Ok(())
}

fn check_single_child(
    conn: &rusqlite::Connection,
    parent: i64,
    moving: Option<i64>,
) -> Result<(), String> {
    let mut statement = conn
        .prepare("SELECT id FROM tasks WHERE parent_id=? ORDER BY id")
        .map_err(|e| e.to_string())?;
    let children = statement
        .query_map(params![parent], |r| r.get::<_, i64>(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    crate::store::chain::single_child(parent, &children, moving)
}
fn check_linear(conn: &rusqlite::Connection, root: i64) -> Result<(), String> {
    check_single_child(conn, root, None)?;
    let mut statement = conn
        .prepare("SELECT id FROM tasks WHERE parent_id=?")
        .map_err(|e| e.to_string())?;
    let children = statement
        .query_map(params![root], |r| r.get::<_, i64>(0))
        .map_err(|e| e.to_string())?;
    for child in children {
        check_linear(conn, child.map_err(|e| e.to_string())?)?;
    }
    Ok(())
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
        path,
        branch,
    } = *task;
    let cwd = std::env::current_dir().map_err(|e| format!("cwd: {e}"))?;
    let home = std::env::var("HOME").ok();
    let path = normalize_path(path.unwrap_or(""), &cwd, home.as_deref());
    let branch = norm_branch(branch.unwrap_or("")).map_err(|e| format!("add: {e}"))?;
    let mut conn = open()?;
    // Read the parent/epic and insert in one write transaction: the invariant
    // must hold for the row that lands, not for the row we looked at.
    let tx = super::write_transaction(&mut conn)?;
    let epic = resolve_epic(&tx, epic, parent)?;
    crate::store::chain::initial_status(status)?;
    if let Some(p) = parent {
        check_depth(&tx, p, 1).map_err(|e| format!("add: {e}"))?;
        if new_chain_root(&tx, p)?.is_some() {
            check_single_child(&tx, p, Some(-1))?;
        }
    }
    // A task created already `working` belongs to its creator unless named.
    let assignee = assignee.or_else(|| (status == "working").then_some(actor));
    let now = now_ms();
    // A `working` task needs a lease, or a dead holder could never be reclaimed.
    let lease = (status == "working").then(|| now + DEFAULT_TTL_S * 1000);
    let started = (status == "working").then_some(now);
    tx.execute(
        "INSERT INTO tasks(title,description,goal,path,branch,status,assignee,priority,epic_id,parent_id,created_at,updated_at,lease_until,started_at,reporter) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        params![title, description, goal, path, branch, status, assignee, priority, epic, parent, now, now, lease, started, actor],
    )
    .map_err(|e| format!("add: {e}"))?;
    let id = tx.last_insert_rowid();
    log_event(&tx, Subject::Task(id), actor, "add", title)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(format!("{} {status}", short(id)))
}

/// Amends a task's spec. A task a worker holds (`working` with a live lease) is
/// refused: that worker is reading this text. Logs one `edit` event naming each
/// changed field with its old/new length; nothing is written when no value moved.
pub fn edit(actor: &str, id: i64, e: &TaskEdit<'_>) -> Result<String, ClaimError> {
    let mut conn = open()?;
    let tx = super::write_transaction(&mut conn)?;
    let out = edit_in_tx(actor, id, e, &tx)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(out)
}

fn edit_in_tx(
    actor: &str,
    id: i64,
    e: &TaskEdit<'_>,
    tx: &rusqlite::Transaction<'_>,
) -> Result<String, ClaimError> {
    if e.title.is_none()
        && e.description.is_none()
        && e.goal.is_none()
        && e.priority.is_none()
        && e.assignee.is_none()
        && e.path.is_none()
        && e.branch.is_none()
        && e.epic.is_none()
    {
        return Err(String::from(
            "nothing to edit: pass title, description, goal, priority, assignee, path, branch or epic",
        )
        .into());
    }
    if e.title.is_some_and(|t| t.trim().is_empty()) {
        return Err(String::from("title cannot be empty").into());
    }
    let cur = fetch(tx, id)?;
    if cur.status == "working" && cur.lease_until.is_some_and(|l| l >= now_ms()) {
        return Err(ClaimError::Held(held_text(&cur)));
    }
    let mut diffs = Vec::new();
    let mut family = Vec::new();
    let epic = e.epic.or(cur.epic_id);
    if epic != cur.epic_id {
        let target = super::epics::fetch_epic(tx, epic.unwrap())?;
        if target.status != "open" {
            return Err(format!(
                "cannot move {} to {}: target epic is {}",
                short(id),
                short_epic(target.id),
                target.status
            )
            .into());
        }
        if let Some(parent) = cur.parent_id {
            let row = fetch(tx, parent)?;
            if row.epic_id != epic {
                family.push((parent, row.epic_id));
            }
        }
        let mut statement = tx.prepare(
            "SELECT id, epic_id FROM tasks WHERE parent_id=? AND (epic_id IS NULL OR epic_id != ?) ORDER BY id"
        ).map_err(|e| e.to_string())?;
        let children = statement
            .query_map(params![id, target.id], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?;
        for child in children {
            family.push(child.map_err(|e| e.to_string())?);
        }
        diffs.push(format!(
            "epic {}->{}",
            cur.epic_id.map_or("none".to_owned(), short_epic),
            short_epic(target.id)
        ));
    }
    let title = value_change(&mut diffs, "title", &cur.title, e.title);
    let description = text_change(&mut diffs, "description", &cur.description, e.description);
    let goal = text_change(&mut diffs, "goal", &cur.goal, e.goal);
    let assignee = value_change(
        &mut diffs,
        "assignee",
        cur.assignee.as_deref().unwrap_or(""),
        e.assignee,
    );
    // A path is normalised the way an epic's is; an empty one clears it.
    let path_value = e.path.map(|raw| {
        let cwd = std::env::current_dir().unwrap_or_default();
        let home = std::env::var("HOME").ok();
        normalize_path(raw, &cwd, home.as_deref())
    });
    let path = value_change(&mut diffs, "path", &cur.path, path_value.as_deref());
    // Refused before any write: a branch git cannot check out must not land.
    let branch_value = match e.branch {
        Some(raw) => norm_branch(raw).map_err(|err| ClaimError::Other(format!("edit: {err}")))?,
        None => cur.branch.clone(),
    };
    let branch = value_change(
        &mut diffs,
        "branch",
        &cur.branch,
        Some(branch_value.as_str()),
    );
    let priority = e.priority.unwrap_or(cur.priority);
    if priority != cur.priority {
        diffs.push(format!("priority {}->{priority}", cur.priority));
    }
    if diffs.is_empty() {
        return Ok(format!("{} unchanged", short(id)));
    }
    tx.execute(
        "UPDATE tasks SET title=?, description=?, goal=?, path=?, branch=?, assignee=?, priority=?, epic_id=?, updated_at=? WHERE id=?",
        params![
            title,
            description,
            goal,
            path,
            branch,
            (!assignee.is_empty()).then_some(assignee),
            priority,
            epic,
            now_ms(),
            id
        ],
    )
    .map_err(|e| e.to_string())?;
    if let Some(note) = epic_gap(&family) {
        log_event(tx, Subject::Task(id), actor, "note", &note)?;
    }
    let seq = log_event(tx, Subject::Task(id), actor, "edit", &diffs.join(", "))?;
    Ok(format!(
        "{} edited {} #{seq}",
        short(id),
        field_names(&diffs)
    ))
}

/// Re-parents an existing task (`None` clears it). The only write that can make
/// a cycle, so the walk up the ancestors happens here, in the same transaction.
pub fn set_parent(actor: &str, id: i64, parent: Option<i64>) -> Result<String, String> {
    let mut conn = open()?;
    let tx = super::write_transaction(&mut conn)?;
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
        check_depth(&tx, p, subtree_height(&tx, id)?)?;
        if new_chain_root(&tx, p)?.is_some() {
            check_single_child(&tx, p, Some(id))?;
            check_linear(&tx, id)?;
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

/// Claims a specific task, or the next ready one. Atomic: BEGIN IMMEDIATE + guarded UPDATE.
pub fn claim(actor: &str, id: Option<i64>, ttl_s: i64) -> Result<String, ClaimError> {
    let mut conn = open()?;
    let now = now_ms();
    let lease = now + ttl_s * 1000;
    for _ in 0..8 {
        let tx = super::write_transaction(&mut conn)?;
        let cand: Option<(i64, String)> = {
            let (sql, arg): (String, Vec<i64>) = match id {
                Some(x) => ("SELECT id,status FROM tasks WHERE id=? AND (status='ready' OR (status='needs_impl' AND parent_id IS NOT NULL) OR (lease_until IS NOT NULL AND lease_until < ?))".to_string(), vec![x, now]),
                None => ("SELECT id,status FROM tasks WHERE status='ready' OR (status='needs_impl' AND parent_id IS NOT NULL) ORDER BY priority, id LIMIT 1".to_owned(), vec![]),
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
                        Ok(r) => Err(ClaimError::Held(held_text(&r))),
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
                "UPDATE tasks SET status='working', assignee=?, lease_until=?, updated_at=?, started_at=COALESCE(started_at,?) WHERE id=? AND (status='ready' OR (status='needs_impl' AND parent_id IS NOT NULL) OR (lease_until IS NOT NULL AND lease_until < ?))",
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
        // Where the work lives, before the prompt: a worker that checks out the
        // wrong tree wastes the whole lease.
        out.push_str(&workdir_text(&r));
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

/// Files the review row for a parentless task that just moved to review, or notes the
/// new submission on the one already filed. Returns the review row's id. Runs inside
/// `apply`'s write transaction, after the gate has proved a submission exists.
fn file_review(
    conn: &rusqlite::Connection,
    actor: &str,
    implement: &super::Row,
    report: Option<&str>,
    submission: &Submission,
) -> Result<i64, String> {
    // A parentless row's only children are its review rows (fix rows hang off those), so
    // the link is the parent alone: renaming the row between rounds must not hide it.
    let title = review_title(&implement.title);
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM tasks WHERE parent_id=? AND status != 'dropped' ORDER BY id DESC LIMIT 1",
            params![implement.id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(review) = existing {
        let note = review_resubmitted_note(implement.id, submission);
        log_event(conn, Subject::Task(review), actor, "note", &note)?;
        return Ok(review);
    }
    let now = now_ms();
    conn.execute(
        "INSERT INTO tasks(title,description,goal,path,branch,status,priority,epic_id,parent_id,created_at,updated_at,reporter) VALUES(?,?,?,?,?,'ready',?,?,?,?,?,?)",
        params![
            title,
            review_description(implement, report, submission),
            review_goal(implement.id, submission),
            implement.path,
            implement.branch,
            implement.priority,
            implement.epic_id,
            implement.id,
            now,
            now,
            actor
        ],
    )
    .map_err(|e| format!("review row: {e}"))?;
    let review = conn.last_insert_rowid();
    log_event(conn, Subject::Task(review), actor, "add", &title)?;
    log_event(
        conn,
        Subject::Task(implement.id),
        actor,
        "note",
        &review_filed_note(review),
    )?;
    Ok(review)
}

pub fn apply(actor: &str, id: i64, p: &Patch<'_>) -> Result<String, String> {
    let mut conn = open()?;
    let tx = super::write_transaction(&mut conn)?;
    if let Some(edit) = p.edit {
        edit_in_tx(actor, id, edit, &tx).map_err(|e| e.to_string())?;
    }
    let cur = fetch(&tx, id)?;
    let now = now_ms();
    let status = match p.status {
        Some(raw) => norm(raw).ok_or_else(|| format!("bad status {raw:?} ({STATUS_LIST})"))?,
        None => cur.status.as_str(),
    };
    let root = new_chain_root(&tx, id)?;
    if matches!(status, "needs_impl" | "needs_review") {
        crate::store::chain::transition(chain_level(&tx, id)?, &cur.status, status)?;
        if status == "needs_impl" && cur.parent_id.is_none() {
            check_linear(&tx, id)?;
        }
    }
    if status == "needs_review" && root.is_none() {
        return Err("analyze plan is not needs_impl; cannot skip to needs_review".into());
    }
    if let Some(root) = root {
        if status == "needs_fix" && chain_level(&tx, id)? != 3 {
            return Err("only the review row may reject a new chain with needs_fix".into());
        }
        if matches!(fetch(&tx, root)?.status.as_str(), "done" | "not_done")
            && p.status.is_some_and(|s| s != cur.status)
        {
            return Err(
                "closed chain is final; use chain reopen with a different identity and a reason"
                    .into(),
            );
        }
        let action = match status {
            "done" | "not_done" if p.status.is_some() => Some(crate::store::chain::Action::Close {
                status,
                verdict: p.report.or(p.note).unwrap_or(""),
            }),
            "needs_fix" if chain_level(&tx, id)? == 3 && p.status.is_some() => {
                Some(crate::store::chain::Action::Reject {
                    reason: p.note.or(p.report).unwrap_or(""),
                })
            }
            _ => None,
        };
        if let Some(action) = action {
            if p.commit.is_some() || p.needs.is_some() {
                return Err(
                    "chain operation cannot combine commit or needs; record those separately"
                        .into(),
                );
            }
            let result = chain_in_tx(&tx, actor, root, action)?;
            tx.commit().map_err(|e| e.to_string())?;
            return crate::store::chain::finish(result);
        }
    }
    let moved = status != cur.status;
    // A move into review needs a submission recorded since the last claim (a diff, or
    // evidence for a re-run round): refused before anything is written, so a reviewer
    // never gets an empty review.
    let submit = moved && matches!(status, "review" | "needs_review");
    let submission = if submit {
        super::diffs::submission_since_claim(
            &tx,
            if root.is_some() && chain_level(&tx, id)? == 3 {
                cur.parent_id.unwrap()
            } else {
                id
            },
        )?
    } else {
        None
    };
    if submit && submission.is_none() {
        return Err(gate_refusal(id));
    }
    // A move into needs_fix is a reopen; the one over the cap lands not_done instead.
    let reopen = moved && status == "needs_fix";
    let capped = reopen && reopen_after(cur.reopen_count).1;
    let status = if capped { "not_done" } else { status };
    if reopen {
        tx.execute(
            "UPDATE tasks SET reopen_count=reopen_count+1 WHERE id=?",
            params![id],
        )
        .map_err(|e| e.to_string())?;
    }
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
            _ => "claim",
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
    if capped {
        log_event(&tx, Subject::Task(id), actor, kind, &cap_note(body))?;
        // One level up only: the grandparent is the manager's to look at.
        if let Some(parent) = cur.parent_id {
            let above = fetch(&tx, parent)?;
            if cascades_to(&above.status) {
                tx.execute(
                    "UPDATE tasks SET status='not_done', needs=NULL, lease_until=NULL, updated_at=? WHERE id=?",
                    params![now, parent],
                )
                .map_err(|e| e.to_string())?;
                log_event(
                    &tx,
                    Subject::Task(parent),
                    actor,
                    "edit",
                    &cap_cascade_note(id),
                )?;
            } else {
                log_event(
                    &tx,
                    Subject::Task(id),
                    actor,
                    "note",
                    &cap_skipped_note(parent, &above.status),
                )?;
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        return Err(cap_refusal(id));
    }
    let seq = log_event(&tx, Subject::Task(id), actor, kind, body)?;
    // Only a parentless row files a review row: a fix row or a review row never does.
    let review = if submit && cur.parent_id.is_none() {
        let report = p.report.or(cur.report.as_deref());
        match submission.as_ref() {
            Some(submission) => Some(file_review(&tx, actor, &cur, report, submission)?),
            // The gate refused above; keeping this branch means deleting the gate
            // still lets the status move, which the mutation proof checks.
            None => None,
        }
    } else {
        None
    };
    tx.commit().map_err(|e| e.to_string())?;
    let filed = review.map_or(String::new(), |r| format!(" (review row {})", short(r)));
    Ok(format!("{} {status} #{seq}{filed}", short(id)))
}

pub fn note(actor: &str, id: i64, body: &str) -> Result<String, String> {
    apply(
        actor,
        id,
        &Patch {
            edit: None,
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
            edit: None,
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
            edit: None,
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
            edit: None,
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
            edit: None,
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
            edit: None,
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
    let mut db = open()?;
    let conn = super::write_transaction(&mut db)?;
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

    conn.commit().map_err(|e| e.to_string())?;
    Ok(format!("ok {} lease {}m", short(id), ttl_s / 60))
}

fn chain_snapshot(
    tx: &rusqlite::Transaction<'_>,
    root: i64,
) -> Result<crate::store::chain::Snapshot, String> {
    let mut rows = vec![fetch(tx, root)?];
    let mut index = 0;
    while index < rows.len() {
        if rows.len() > 3 {
            return Err(
                "chain requires exactly analyze -> implement -> review, with no review children"
                    .into(),
            );
        }
        let mut statement = tx
            .prepare("SELECT id FROM tasks WHERE parent_id=? ORDER BY id")
            .map_err(|e| e.to_string())?;
        let ids = statement
            .query_map(params![rows[index].id], |r| r.get::<_, i64>(0))
            .map_err(|e| e.to_string())?;
        for id in ids {
            rows.push(fetch(tx, id.map_err(|e| e.to_string())?)?);
        }
        index += 1;
    }
    let ids = rows
        .iter()
        .map(|r| r.id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let diff_actor = tx.query_row(&format!("SELECT actor FROM events WHERE kind='diff' AND task_id IN ({ids}) ORDER BY seq DESC LIMIT 1"), [], |r| r.get(0)).optional().map_err(|e| e.to_string())?;
    let close_actor = tx.query_row("SELECT actor FROM events WHERE kind='chain_close' AND task_id=? ORDER BY seq DESC LIMIT 1", params![root], |r| r.get(0)).optional().map_err(|e| e.to_string())?;
    Ok(crate::store::chain::Snapshot {
        rows,
        diff_actor,
        close_actor,
    })
}

fn chain_in_tx(
    tx: &rusqlite::Transaction<'_>,
    actor: &str,
    root: i64,
    action: crate::store::chain::Action<'_>,
) -> Result<(String, bool), String> {
    let snapshot = chain_snapshot(tx, root)?;
    let changes = crate::store::chain::plan(actor, &snapshot, action)?;
    for (index, change) in changes.iter().enumerate() {
        let closed = matches!(change.status.as_str(), "done" | "not_done");
        tx.execute("UPDATE tasks SET status=?, reopen_count=?, lease_until=NULL, completed_at=?, updated_at=?, note=? WHERE id=?", params![change.status, change.count, closed.then(now_ms), now_ms(), change.body, change.id]).map_err(|e| e.to_string())?;
        if index == 2 && matches!(change.kind, "chain_reopen" | "chain_reject") {
            tx.execute(
                "UPDATE tasks SET report=NULL WHERE id=?",
                params![change.id],
            )
            .map_err(|e| e.to_string())?;
        }
        if index == 2 && change.kind == "chain_close" {
            tx.execute(
                "UPDATE tasks SET report=? WHERE id=?",
                params![change.body, change.id],
            )
            .map_err(|e| e.to_string())?;
        }
        log_event(
            tx,
            Subject::Task(change.id),
            actor,
            change.kind,
            &change.body,
        )?;
        #[cfg(test)]
        if index == 0 {
            crate::store::chain::tests::pause_close();
        }
    }
    Ok((
        crate::store::chain::summary(&changes),
        changes[1].count > crate::store::text::MAX_REOPENS,
    ))
}

pub fn chain_action(
    actor: &str,
    root: i64,
    action: crate::store::chain::Action<'_>,
) -> Result<String, String> {
    let mut conn = open()?;
    let tx = super::write_transaction(&mut conn)?;
    let result = chain_in_tx(&tx, actor, root, action)?;
    tx.commit().map_err(|e| e.to_string())?;
    crate::store::chain::finish(result)
}

fn new_chain_root(tx: &rusqlite::Connection, id: i64) -> Result<Option<i64>, String> {
    let mut row = fetch(tx, id)?;
    for _ in 0..MAX_CHAIN {
        match row.parent_id {
            Some(parent) => row = fetch(tx, parent)?,
            None => {
                let closed: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE task_id=? AND kind IN ('chain_close','chain_reject'))", params![row.id], |r| r.get(0)).map_err(|e| e.to_string())?;
                return Ok((row.status == "needs_impl" || closed).then_some(row.id));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::super::SCHEMA;
    use super::*;
    use crate::store::sqlite::diffs::{check_evidence, record, submission_since_claim};

    fn board() -> rusqlite::Connection {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        c.execute_batch(SCHEMA).unwrap();
        c.execute(
            "INSERT INTO epics(title,created_at,updated_at) VALUES('e',1,1)",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO tasks(title,path,branch,priority,epic_id,created_at,updated_at) VALUES('impl','/p','feat/x',7,1,1,1)",
            [],
        )
        .unwrap();
        c
    }

    fn patch(c: &rusqlite::Connection, id: i64) -> Submission {
        Submission::patch(super::super::diffs::newest_diff(c, id).unwrap().unwrap())
    }

    #[test]
    fn the_gate_wants_a_submission_after_the_newest_claim_event() {
        let c = board();
        assert!(
            submission_since_claim(&c, 1).unwrap().is_none(),
            "no submission at all"
        );
        record(&c, "a", 1, "first", None).unwrap();
        assert!(
            submission_since_claim(&c, 1).unwrap().is_some(),
            "no claim yet: any diff counts"
        );
        log_event(&c, Subject::Task(1), "a", "claim", "").unwrap();
        assert!(
            submission_since_claim(&c, 1).unwrap().is_none(),
            "the old diff predates the new claim"
        );
        record(&c, "a", 1, "second", None).unwrap();
        assert!(submission_since_claim(&c, 1).unwrap().is_some());
        log_event(&c, Subject::Task(1), "a", "claim", "").unwrap();
        assert!(
            submission_since_claim(&c, 1).unwrap().is_none(),
            "every later claim resets it"
        );
    }

    #[test]
    fn evidence_only_requires_the_newest_patch_of_the_same_row() {
        let c = board();
        record(&c, "a", 1, "the patch", None).unwrap();
        // implement T1 -> review T2 -> fix T3
        c.execute(
            "INSERT INTO tasks(title,parent_id,created_at,updated_at) VALUES('rev',1,1,1)",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO tasks(title,parent_id,created_at,updated_at) VALUES('fix',2,1,1)",
            [],
        )
        .unwrap();
        // An ancestor's patch is in the chain but not on this row: no re-parent.
        let err = check_evidence(&c, 3, 1).unwrap_err();
        assert!(
            err.contains("D1 is recorded on T1, not on T3") && err.contains("no re-parent"),
            "{err}"
        );
        record(&c, "a", 3, "the fix patch", None).unwrap();
        check_evidence(&c, 3, 2).unwrap();
        // A claim after the patch makes the evidence row this round's submission;
        // without it the gate would prefer the patch itself.
        log_event(&c, Subject::Task(3), "a", "claim", "").unwrap();
        record(&c, "a", 3, "re-ran: cargo test ok", Some(2)).unwrap();
        let s = submission_since_claim(&c, 3).unwrap().unwrap();
        assert!(s.evidence_only());
        assert_eq!((s.record.id, s.diff.id), (3, 2));
        // Another evidence-only row is not a patch.
        let err = check_evidence(&c, 3, 3).unwrap_err();
        assert!(
            err.contains("D3 is evidence-only, not a patch") && err.contains("--reviews"),
            "{err}"
        );
        // A newer patch supersedes D2.
        record(&c, "a", 3, "the real patch", None).unwrap();
        let err = check_evidence(&c, 3, 2).unwrap_err();
        assert!(
            err.contains("D2 is not the newest patch of T3") && err.contains("D4"),
            "{err}"
        );
        // An unrelated row's diff is refused by name too.
        c.execute(
            "INSERT INTO tasks(title,created_at,updated_at) VALUES('other',1,1)",
            [],
        )
        .unwrap();
        record(&c, "a", 4, "other patch", None).unwrap();
        let err = check_evidence(&c, 3, 5).unwrap_err();
        assert!(err.contains("D5 is recorded on T4, not on T3"), "{err}");
        assert!(
            check_evidence(&c, 3, 99)
                .unwrap_err()
                .contains("no such diff D99")
        );
        // A plain diff recorded later is still preferred; the patch is the artifact.
        record(&c, "a", 3, "another plain patch", None).unwrap();
        let s = submission_since_claim(&c, 3).unwrap().unwrap();
        assert!(!s.evidence_only());
        assert_eq!((s.record.id, s.diff.id), (6, 6));
    }

    #[test]
    fn a_review_row_is_filed_once_and_a_resubmission_only_notes_it() {
        let c = board();
        record(&c, "a", 1, "d1", None).unwrap();
        let implement = fetch(&c, 1).unwrap();
        let review = file_review(&c, "a", &implement, Some("the report"), &patch(&c, 1)).unwrap();
        assert_eq!(review, 2);
        let r = fetch(&c, 2).unwrap();
        assert_eq!(
            (r.title.as_str(), r.status.as_str(), r.parent_id, r.epic_id),
            ("Review impl", "ready", Some(1), Some(1))
        );
        assert_eq!(
            (r.path.as_str(), r.branch.as_str(), r.priority),
            ("/p", "feat/x", 7)
        );
        assert!(r.assignee.is_none());
        assert!(r.description.contains("the report") && r.description.contains("D1"));
        let before = r.description.clone();
        record(&c, "a", 1, "d2", None).unwrap();
        assert_eq!(
            file_review(&c, "a", &implement, Some("again"), &patch(&c, 1)).unwrap(),
            2
        );
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM tasks WHERE parent_id=1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(n, 1, "no second review row");
        assert_eq!(
            fetch(&c, 2).unwrap().description,
            before,
            "description untouched"
        );
        let note: String = c
            .query_row(
                "SELECT body FROM events WHERE task_id=2 AND kind='note'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(note.contains("D2"), "{note}");

        // Renamed between rounds: still the same review row, and the note lands on it.
        c.execute("UPDATE tasks SET title='impl renamed' WHERE id=1", [])
            .unwrap();
        record(&c, "a", 1, "d3", None).unwrap();
        let renamed = fetch(&c, 1).unwrap();
        assert_eq!(
            file_review(&c, "a", &renamed, None, &patch(&c, 1)).unwrap(),
            2
        );
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM tasks WHERE parent_id=1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(n, 1, "a rename must not file a second review row");
        let notes: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM events WHERE task_id=2 AND kind='note' AND body LIKE '%D3%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(notes, 1);

        // A dropped review row is not reused: the next submission files a fresh one.
        c.execute("UPDATE tasks SET status='dropped' WHERE id=2", [])
            .unwrap();
        assert_eq!(
            file_review(&c, "a", &renamed, None, &patch(&c, 1)).unwrap(),
            3
        );
    }
}
