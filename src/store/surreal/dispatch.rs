//! `dispatch` on the SurrealDB backend: the port of `sqlite/dispatch.rs`. The target
//! choice, the refusal and warning text and the pane delivery are the shared ones in
//! `bridge`; this file loads the task, the registry rows and the epic, and writes the
//! `--force` reset and the dispatch event, each inside a write session (the write
//! lock held across the write). The reads take no lock.

use super::epics::fetch_epic;
use super::tasks::{Txn, apply, fetch};
use super::{Db, block_on, db_err, session};
use crate::store::board::Patch;
use crate::store::bridge::{
    Candidate, Liveness, deliver, epic_warning, line, not_ready_text, resolve,
};
use crate::store::short;
use serde::Deserialize;

/// What the registry says about a session name: its agent type, if a row exists.
async fn registered_agent(db: &Db, name: &str) -> Result<Option<String>, String> {
    #[derive(Deserialize)]
    struct A {
        agent: String,
    }
    let mut r = db
        .query("SELECT agent FROM sessions WHERE id = type::thing('sessions', $name)")
        .bind(("name", name.to_owned()))
        .await
        .map_err(|e| db_err(&e))?;
    let rows: Vec<A> = r.take(0).map_err(|e| db_err(&e))?;
    Ok(rows.into_iter().next().map(|a| a.agent))
}

async fn chain(db: &Db, id: i64) -> Result<crate::store::bridge::DispatchChain, String> {
    let task = fetch(db, id).await?;
    let parent = task.parent_id;
    let mut root = task;
    let mut depth = 1;
    while let Some(id) = root.parent_id {
        root = fetch(db, id).await?;
        depth += 1;
        if depth > 3 {
            return Err("chain too deep".into());
        }
    }
    // New-chain status can be on any row; terminal chain events preserve the
    // shape after close/reject replaces those statuses.
    let mut shape = db.query("SELECT VALUE true FROM tasks WHERE status IN ['needs_impl','needs_review'] AND (meta::id(id)=$root OR parent_id=$root OR parent_id IN (SELECT VALUE meta::id(id) FROM tasks WHERE parent_id=$root)) LIMIT 1; SELECT VALUE true FROM events WHERE task_id=$root AND kind IN ['chain_close','chain_reopen','chain_reject'] LIMIT 1;")
        .bind(("root", root.tid)).await.map_err(|e| db_err(&e))?;
    let statuses: Vec<bool> = shape.take(0).map_err(|e| db_err(&e))?;
    let history: Vec<bool> = shape.take(1).map_err(|e| db_err(&e))?;
    let new_chain = !statuses.is_empty() || !history.is_empty();
    #[derive(Deserialize)]
    struct Claim {
        seq: i64,
        actor: String,
    }
    let mut result = db.query("SELECT meta::id(id) AS seq,actor FROM events WHERE task_id=$parent AND kind='claim' ORDER BY seq DESC LIMIT 1").bind(("parent",parent)).await.map_err(|e| db_err(&e))?;
    let claims: Vec<Claim> = result.take(0).map_err(|e| db_err(&e))?;
    let mut result = db
        .query("SELECT VALUE meta::id(id) FROM tasks WHERE parent_id=$parent")
        .bind(("parent", parent))
        .await
        .map_err(|e| db_err(&e))?;
    let children: Vec<i64> = result.take(0).map_err(|e| db_err(&e))?;
    let mut result = db.query("SELECT meta::id(id) AS seq FROM events WHERE kind='dispatch' AND task_id IN $children ORDER BY seq DESC LIMIT 1").bind(("children",children.clone())).await.map_err(|e| db_err(&e))?;
    #[derive(Deserialize)]
    struct Dispatch {
        seq: i64,
    }
    let dispatched: Vec<Dispatch> = result.take(0).map_err(|e| db_err(&e))?;
    Ok(crate::store::bridge::DispatchChain {
        root_reporter: root.reporter,
        new_chain,
        depth,
        parent_claim: claims.into_iter().next().map(|c| (c.seq, c.actor)),
        children: children.len(),
        dispatched: dispatched.first().map(|d| d.seq),
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
    let mut s = session(true)?;
    let task = block_on(fetch(&s.db, id))?;
    let facts = block_on(chain(&s.db, id))?;
    let expected = crate::store::bridge::dispatch_authority(actor, id, to, &facts)?;
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
            let agent = block_on(registered_agent(&s.db, n))?;
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
            let epic = block_on(fetch_epic(&s.db, e))?;
            epic_warning(id, e, &epic.path, &target, live.cwd(&target))
        }
        None => None,
    };
    if expected.is_none() {
        drop(s);
        s = session(false)?;
    }
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
    if expected.is_none() {
        drop(s);
        s = session(true)?;
    }
    block_on(async {
        let mut tx = Txn::new();
        let idp = tx.p(id);
        tx.event(&idp, actor, "dispatch", &format!("{target} via {path}"));
        tx.run(&s.db, "$seq").await.map(|_| ())
    })?;
    out.push_str(&format!("dispatched to {target} via {path}: {text}"));
    Ok(out)
}
