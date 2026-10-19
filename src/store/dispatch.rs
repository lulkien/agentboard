use super::render::stamp;
use super::tasks::{Patch, apply};
use super::{fetch, log_event, now_ms, open, short};
use rusqlite::{OptionalExtension, params};

/// The line a worker pane receives. Fixed wording: the pane runs claim itself.
fn line(id: i64) -> String {
    format!(
        "Claim {}, then task_get id={id} full=true, and execute it.",
        short(id)
    )
}

/// Submits `text` to a registered pane: herdr first, tmux as fallback. Returns
/// the path that worked so the operator can see which one ran.
fn deliver(name: &str, text: &str) -> Result<&'static str, String> {
    let herdr = std::process::Command::new("herdr")
        .args(["agent", "prompt", name, text])
        .stdin(std::process::Stdio::null())
        .output();
    let herdr_err = match herdr {
        Ok(o) if o.status.success() => return Ok("herdr"),
        Ok(o) => format!("herdr: {}", String::from_utf8_lossy(&o.stderr).trim()),
        Err(e) => format!("herdr: {e}"),
    };
    super::sessions::send(name, text)
        .map(|_| "tmux")
        .map_err(|tmux_err| format!("dispatch to {name} failed ({herdr_err}; {tmux_err})"))
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
    let registered = |name: &str| -> Result<bool, String> {
        conn.query_row("SELECT 1 FROM sessions WHERE name=?", params![name], |_| {
            Ok(())
        })
        .optional()
        .map(|r| r.is_some())
        .map_err(|e| e.to_string())
    };
    let target = match task.assignee.as_deref() {
        Some(a) if registered(a)? => a.to_owned(),
        _ => match to {
            Some(t) if registered(t)? => t.to_owned(),
            Some(t) => {
                return Err(format!(
                    "no registered session {t:?} (agentboard register <name>)"
                ));
            }
            None => {
                return Err(format!(
                    "{} has no registered assignee; pass --to <session>",
                    short(id)
                ));
            }
        },
    };
    drop(conn);
    let mut out = String::new();
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
        id,
        actor,
        "dispatch",
        &format!("{target} via {path}"),
    )?;
    out.push_str(&format!("dispatched to {target} via {path}: {text}"));
    Ok(out)
}
