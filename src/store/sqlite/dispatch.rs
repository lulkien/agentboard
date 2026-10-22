use super::tasks::apply;
use super::{Subject, fetch, log_event, open, short};
use crate::store::board::Patch;
use crate::store::bridge::{
    Candidate, Liveness, deliver, epic_warning, line, not_ready_text, resolve,
};
use rusqlite::{OptionalExtension, params};

/// What the registry says about a session name: its agent type, if a row exists.
fn registered_agent(conn: &rusqlite::Connection, name: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT agent FROM sessions WHERE name=?",
        params![name],
        |r| r.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

fn chain(
    conn: &rusqlite::Connection,
    id: i64,
) -> Result<crate::store::bridge::DispatchChain, String> {
    let task = fetch(conn, id)?;
    let parent = task.parent_id;
    let mut root = task;
    let mut depth = 1;
    while let Some(id) = root.parent_id {
        root = fetch(conn, id)?;
        depth += 1;
        if depth > 3 {
            return Err("chain too deep".into());
        }
    }
    // Inspect the whole chain, including descendants of the requested row.
    // Closure/rejection events retain the new shape after terminal statuses.
    let new_chain = conn.query_row(
        "WITH RECURSIVE chain_rows(id,status) AS (SELECT id,status FROM tasks WHERE id=?1 UNION ALL SELECT t.id,t.status FROM tasks t JOIN chain_rows c ON t.parent_id=c.id) SELECT EXISTS(SELECT 1 FROM chain_rows WHERE status IN ('needs_impl','needs_review')) OR EXISTS(SELECT 1 FROM events WHERE task_id=?1 AND kind IN ('chain_close','chain_reopen','chain_reject'))",
        params![root.id], |r| r.get(0)
    ).map_err(|e| e.to_string())?;
    let parent_claim = conn.query_row("SELECT seq,actor FROM events WHERE task_id=? AND kind='claim' ORDER BY seq DESC LIMIT 1", params![parent], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(|e| e.to_string())?;
    let children = conn
        .query_row(
            "SELECT count(*) FROM tasks WHERE parent_id=?",
            params![parent],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let dispatched = conn.query_row("SELECT max(seq) FROM events WHERE kind='dispatch' AND task_id IN (SELECT id FROM tasks WHERE parent_id=?)",params![parent],|r| r.get(0)).map_err(|e| e.to_string())?;
    Ok(crate::store::bridge::DispatchChain {
        root_reporter: root.reporter,
        new_chain,
        depth,
        parent_claim,
        children,
        dispatched,
    })
}

/// Tells a pane to claim task `id`. Refuses unless the task is `ready`; `force`
/// resets it through the normal status->ready path first. `print_only` touches
/// nothing: no reset, no event, no herdr/tmux.
pub fn dispatch(
    actor: &str,
    id: i64,
    to: Option<&str>,
    force: bool,
    print_only: bool,
) -> Result<String, String> {
    let conn = open()?;
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| e.to_string())?;
    let task = fetch(&conn, id)?;
    let facts = chain(&conn, id)?;
    let expected = crate::store::bridge::dispatch_authority(actor, id, to, &facts)?;
    if expected.is_none() {
        conn.execute_batch("COMMIT").map_err(|e| e.to_string())?;
    }
    if expected.is_some() {
        crate::store::bridge::dispatch_authority(actor, id, task.assignee.as_deref(), &facts)?;
    }
    let text = line(id);
    if task.status != "ready" && (!force || expected.is_some()) {
        if expected.is_some() {
            return Err(crate::store::bridge::dispatch_refusal(
                id,
                to.or(expected),
                "task is not ready; only the root reporter may force a reset",
            ));
        }
        return Err(not_ready_text(
            id,
            &task.status,
            task.assignee.as_deref(),
            task.lease_until,
        ));
    }
    if print_only {
        return Ok(text);
    }
    let live = Liveness::new();
    let candidate = |name: Option<&str>| -> Result<Option<(String, Candidate)>, String> {
        name.map(|n| {
            let agent = registered_agent(&conn, n)?;
            Ok((n.to_owned(), Candidate::of(agent.as_deref(), &live, n)))
        })
        .transpose()
    };
    let a = candidate(task.assignee.as_deref())?;
    let t = candidate(to.or(expected))?;
    let target = resolve(
        a.as_ref().map(|(n, c)| (n.as_str(), *c)),
        t.as_ref().map(|(n, c)| (n.as_str(), *c)),
        id,
    )?;
    crate::store::bridge::dispatch_authority(actor, id, Some(&target), &facts)?;
    let warning = match task.epic_id {
        Some(e) => {
            let epic = super::epics::fetch_epic(&conn, e)?;
            epic_warning(id, e, &epic.path, &target, live.cwd(&target))
        }
        None => None,
    };
    let mut out = warning.unwrap_or_default();
    if task.status != "ready" {
        // Reset before sending: the pane claims immediately and must find it ready.
        apply(
            actor,
            id,
            &Patch {
                edit: None,
                status: Some("ready"),
                report: None,
                note: Some("dispatch --force"),
                commit: None,
                needs: None,
                ttl_s: 0,
            },
        )?;
        out.push_str(&format!(
            "--force: {} {} -> ready\n",
            short(id),
            task.status
        ));
    }
    let path = deliver(&target, &text)?;
    log_event(
        &conn,
        Subject::Task(id),
        actor,
        "dispatch",
        &format!("{target} via {path}"),
    )?;
    if expected.is_some() {
        conn.execute_batch("COMMIT").map_err(|e| e.to_string())?;
    }
    out.push_str(&format!("dispatched to {target} via {path}: {text}"));
    Ok(out)
}
