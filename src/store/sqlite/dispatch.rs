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
    let task = fetch(&conn, id)?;
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
            let agent = registered_agent(&conn, n)?;
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
            let epic = super::epics::fetch_epic(&conn, e)?;
            epic_warning(id, e, &epic.path, &target, live.cwd(&target))
        }
        None => None,
    };
    drop(conn);
    let mut out = warning.unwrap_or_default();
    if task.status != "ready" {
        // Reset before sending: the pane claims immediately and must find it ready.
        apply(
            actor,
            id,
            &Patch {
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
    let conn = open()?;
    log_event(
        &conn,
        Subject::Task(id),
        actor,
        "dispatch",
        &format!("{target} via {path}"),
    )?;
    out.push_str(&format!("dispatched to {target} via {path}: {text}"));
    Ok(out)
}
