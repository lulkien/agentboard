use super::render::stamp;
use super::sessions::Liveness;
use super::tasks::apply;
use super::{Subject, fetch, log_event, now_ms, open, short, short_epic};
use crate::clip::clip;
use crate::store::board::Patch;
use crate::store::proc;
use rusqlite::{OptionalExtension, params};
use std::process::Command;

/// The line a worker pane receives. Fixed wording: the pane uses MCP tools.
fn line(id: i64) -> String {
    format!(
        "Use the agentboard MCP tools: claim task {} with the claim tool, read it with the get tool (full=true), then execute it. Do not shell out to the agentboard binary for board writes.",
        short(id)
    )
}

/// Runs `herdr agent prompt` under the shared deadline. `Err` is the one-line
/// reason herdr did not deliver: spawn failure, bad exit, or timeout.
fn herdr_prompt(name: &str, text: &str) -> Result<(), String> {
    let out = proc::run(Command::new("herdr").args(["agent", "prompt", name, text]))?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stderr = stderr.trim();
    Err(if stderr.is_empty() {
        out.status.to_string()
    } else {
        clip(stderr, 160)
    })
}

/// Submits `text` to a registered pane: herdr first, tmux as fallback. Returns
/// the path that worked; a tmux win names why herdr did not, so the event and the
/// printed line explain the fallback.
fn deliver(name: &str, text: &str) -> Result<String, String> {
    let herdr_err = match herdr_prompt(name, text) {
        Ok(()) => return Ok("herdr".to_owned()),
        Err(reason) => reason,
    };
    super::sessions::send(name, text)
        .map(|_| format!("tmux (herdr: {herdr_err})"))
        .map_err(|tmux_err| format!("dispatch to {name} failed (herdr: {herdr_err}; {tmux_err})"))
}

/// What a session name is worth as a dispatch target.
enum Candidate {
    /// No row: never registered.
    Missing,
    /// A row exists but no pane answers to the name.
    Stale,
    Live,
}

impl Candidate {
    fn of(conn: &rusqlite::Connection, live: &Liveness, name: &str) -> Result<Self, String> {
        let agent: Option<String> = conn
            .query_row(
                "SELECT agent FROM sessions WHERE name=?",
                params![name],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(match agent {
            None => Self::Missing,
            Some(a) if live.alive(name, &a) => Self::Live,
            Some(_) => Self::Stale,
        })
    }
}

/// Picks the pane to prompt: the task's assignee if its pane is live, else
/// `--to` if live. A registry row alone proves nothing (rows never expire), so a
/// dead assignee row must not capture the dispatch.
fn resolve(
    conn: &rusqlite::Connection,
    live: &Liveness,
    assignee: Option<&str>,
    to: Option<&str>,
    id: i64,
) -> Result<String, String> {
    let a = assignee.map(|n| Candidate::of(conn, live, n)).transpose()?;
    if let (Some(Candidate::Live), Some(n)) = (&a, assignee) {
        return Ok(n.to_owned());
    }
    let t = to.map(|n| Candidate::of(conn, live, n)).transpose()?;
    if let (Some(Candidate::Live), Some(n)) = (&t, to) {
        return Ok(n.to_owned());
    }
    let mut why = Vec::new();
    if let (Some(Candidate::Stale), Some(n)) = (&a, assignee) {
        why.push(format!(
            "{} has assignee {n:?} but that session row is stale: no live pane answers to it (`agentboard sessions prune` removes dead rows)",
            short(id)
        ));
    }
    match (&t, to) {
        (Some(Candidate::Stale), Some(n)) => why.push(format!(
            "session {n:?} is registered but not live (stale row; `agentboard sessions prune`)"
        )),
        (Some(_), Some(n)) => why.push(format!(
            "no registered session {n:?} (agentboard register <name>)"
        )),
        _ if why.is_empty() => why.push(format!(
            "{} has no registered assignee; pass --to <session>",
            short(id)
        )),
        _ => why.push("pass --to <live session>".to_owned()),
    }
    Err(why.join("; "))
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
        let lease = task.lease_until.map_or("none".to_owned(), |l| {
            format!(
                "{}m left, until {}Z",
                (l - now_ms()).max(0) / 60_000,
                stamp(l)
            )
        });
        return Err(format!(
            "{} is {}, not ready (assignee {}, lease {lease}); --force resets it to ready first",
            short(id),
            task.status,
            task.assignee.as_deref().unwrap_or("nobody")
        ));
    }
    if print_only {
        return Ok(text);
    }
    let live = Liveness::new();
    let target = resolve(&conn, &live, task.assignee.as_deref(), to, id)?;
    // Warn, never refuse: only when the epic names a path and the pane reports a
    // cwd, and the cwd is not that path or under it.
    let warning = match task.epic_id {
        Some(e) => {
            let epic = super::epics::fetch_epic(&conn, e)?;
            live.cwd(&target)
                .filter(|cwd| {
                    !epic.path.is_empty() && !std::path::Path::new(cwd).starts_with(&epic.path)
                })
                .map(|cwd| {
                    format!(
                        "warning: {} belongs to epic {} rooted at {}, but session {target:?} reports cwd {cwd} (outside it); dispatching anyway\n",
                        short(id),
                        short_epic(e),
                        epic.path
                    )
                })
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
