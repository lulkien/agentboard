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
    let s = session(false)?;
    let task = block_on(fetch(&s.db, id))?;
    let text = line(id);
    if task.status != "ready" && !force {
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
    let t = candidate(to)?;
    let target = resolve(
        a.as_ref().map(|(n, c)| (n.as_str(), *c)),
        t.as_ref().map(|(n, c)| (n.as_str(), *c)),
        id,
    )?;
    let warning = match task.epic_id {
        Some(e) => {
            let epic = block_on(fetch_epic(&s.db, e))?;
            epic_warning(id, e, &epic.path, &target, live.cwd(&target))
        }
        None => None,
    };
    drop(s);
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
    let w = session(true)?;
    block_on(async {
        let mut tx = Txn::new();
        let idp = tx.p(id);
        tx.event(&idp, actor, "dispatch", &format!("{target} via {path}"));
        tx.run(&w.db, "$seq").await.map(|_| ())
    })?;
    out.push_str(&format!("dispatched to {target} via {path}: {text}"));
    Ok(out)
}
