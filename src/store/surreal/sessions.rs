//! Sessions on the SurrealDB backend: the port of the registry half of
//! `sqlite/sessions.rs` (`sessions`, `prune`, `unregister`, `register_session`).
//! `tail` and `send` are unchanged and still stubbed here. Liveness asks tmux and
//! herdr through the shared bounded runner in `proc.rs`.
//!
//! `Liveness` and its tmux/herdr helpers are copies of the sqlite ones; the hoist
//! into a shared module happens with the dispatch port (it needs them too).

use super::tasks::{Txn, ago, fetch_opt};
use super::{block_on, db_err, session};
use crate::store::proc;
use crate::store::{now_ms, short};
use serde::Deserialize;
use std::process::Command;

/// One registry row as `sessions` lists it. `cwd` is stored but not shown.
#[derive(Deserialize)]
struct SessionRow {
    name: String,
    agent: String,
    task_id: Option<i64>,
    seen: i64,
}

async fn rows(db: &super::Db) -> Result<Vec<SessionRow>, String> {
    let mut r = db
        .query("SELECT meta::id(id) AS name, agent, task_id, seen FROM sessions ORDER BY seen DESC, name")
        .await
        .map_err(|e| db_err(&e))?;
    r.take(0).map_err(|e| db_err(&e))
}

/// Which panes answer right now. A row is only a claim that a pane exists; this
/// is the check. tmux is asked per name, herdr once, and only if tmux cannot
/// vouch for a name.
struct Liveness {
    herdr: std::cell::OnceCell<Option<Vec<HerdrAgent>>>,
}

impl Liveness {
    fn new() -> Self {
        Self {
            herdr: std::cell::OnceCell::new(),
        }
    }

    /// `None` when herdr cannot answer (missing, hung, bad exit), `Some(empty)`
    /// when it answers with no agents: the difference decides whether a prune is safe.
    fn herdr(&self) -> Option<&[HerdrAgent]> {
        self.herdr.get_or_init(herdr_agents).as_deref()
    }

    fn alive(&self, name: &str, agent: &str) -> bool {
        tmux_alive(name)
            || self
                .herdr()
                .is_some_and(|a| a.iter().any(|h| h.name == name && h.kind == agent))
    }
}

pub fn sessions() -> Result<String, String> {
    let s = session(false)?;
    let names = block_on(rows(&s.db))?;
    if names.is_empty() {
        return Ok("(no sessions registered)".to_owned());
    }
    let live = Liveness::new();
    let mut out = String::new();
    for SessionRow {
        name,
        agent,
        task_id,
        seen,
    } in names
    {
        let alive = live.alive(&name, &agent);
        out.push_str(&format!(
            "{name} {agent} {} {} {}\n",
            if alive { "up" } else { "gone" },
            task_id.map_or("-".to_owned(), short),
            ago(seen)
        ));
    }
    Ok(out.trim_end().to_owned())
}

/// Deletes rows whose pane is not alive. Rows never expire on their own, and a
/// dead row captures a dispatch to its name, so this is the supported cleanup.
/// Refuses when neither tmux nor herdr answers: every row would look dead.
/// A dry run deletes nothing, so it takes no write lock.
pub fn prune(dry_run: bool) -> Result<String, String> {
    let s = session(!dry_run)?;
    let all = block_on(rows(&s.db))?;
    let live = Liveness::new();
    let herdr_up = live.herdr().is_some();
    if !herdr_up && !tmux_up() {
        return Err("prune: neither tmux nor herdr answers, so no session can be judged dead; start the session host, or remove one row with `agentboard unregister <name>`".to_owned());
    }
    let why = if herdr_up {
        "not a tmux session, not a herdr agent"
    } else {
        "not a tmux session (herdr unreachable: checked against tmux only)"
    };
    let (verb, done) = if dry_run {
        ("would remove", "would prune")
    } else {
        ("removed", "pruned")
    };
    let (mut out, mut dead, mut kept) = (String::new(), 0, 0);
    for r in &all {
        if live.alive(&r.name, &r.agent) {
            kept += 1;
            continue;
        }
        if !dry_run {
            delete(&s.db, &r.name)?;
        }
        dead += 1;
        out.push_str(&format!("{verb} {} ({}): {why}\n", r.name, r.agent));
    }
    out.push_str(&if dead == 0 {
        format!("nothing to prune ({kept} live)")
    } else {
        format!("{done} {dead}, kept {kept}")
    });
    Ok(out)
}

/// Deletes one row; returns how many existed (0 or 1).
fn delete(db: &super::Db, name: &str) -> Result<usize, String> {
    block_on(async {
        let mut r = db
            .query("DELETE type::thing('sessions', $name) RETURN BEFORE")
            .bind(("name", name.to_owned()))
            .await
            .map_err(|e| db_err(&e))?;
        // Only the row count matters; the record id cannot be read as plain JSON.
        #[derive(Deserialize)]
        struct Gone {}
        let gone: Vec<Gone> = r.take(0).map_err(|e| db_err(&e))?;
        Ok(gone.len())
    })
}

pub fn unregister(name: &str) -> Result<String, String> {
    let s = session(true)?;
    if delete(&s.db, name)? == 0 {
        return Err(format!("no registered session {name:?}"));
    }
    Ok(format!("ok session {name} removed"))
}

pub fn register_session(
    name: &str,
    agent: &str,
    cwd: &str,
    task: Option<i64>,
) -> Result<String, String> {
    let s = session(true)?;
    block_on(async {
        if let Some(t) = task {
            fetch_opt(&s.db, t)
                .await?
                .ok_or_else(|| format!("no such task {}", short(t)))?;
        }
        // An upsert: registering a name again replaces the row, as the sqlite
        // `ON CONFLICT DO UPDATE` does.
        let mut tx = Txn::new();
        let (n, a, c, t, seen) = (
            tx.p(name),
            tx.p(agent),
            tx.p(cwd),
            task.map_or_else(|| "NONE".to_owned(), |t| tx.p(t)),
            tx.p(now_ms()),
        );
        tx.raw(&format!(
            "UPSERT type::thing('sessions', {n}) SET agent = {a}, cwd = {c}, task_id = {t}, seen = {seen}, status = 'up';"
        ));
        tx.run(&s.db, "true").await?;
        Ok(format!("ok session {name} up"))
    })
}

fn tmux_alive(name: &str) -> bool {
    proc::run(Command::new("tmux").args(["has-session", "-t", name]))
        .is_ok_and(|o| o.status.success())
}

/// One entry of `herdr agent list`.
struct HerdrAgent {
    name: String,
    kind: String,
}

/// A tmux server is up and answering (a `has-session` miss alone cannot tell
/// "no such session" from "no tmux at all").
fn tmux_up() -> bool {
    proc::run(Command::new("tmux").arg("list-sessions")).is_ok_and(|o| o.status.success())
}

/// Live herdr agents. `None`: herdr did not answer (missing binary, hang, bad
/// exit). Bad JSON is an answer with no agents. Bounded by the shared runner.
fn herdr_agents() -> Option<Vec<HerdrAgent>> {
    match proc::run(Command::new("herdr").args(["agent", "list"])) {
        Ok(o) if o.status.success() => Some(parse_herdr_agents(&o.stdout)),
        _ => None,
    }
}

/// Parses the `herdr agent list` envelope (`.result.agents[]`). Agents without a
/// `name` (unnamed panes) are skipped.
fn parse_herdr_agents(json: &[u8]) -> Vec<HerdrAgent> {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(json) else {
        return Vec::new();
    };
    v["result"]["agents"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    Some(HerdrAgent {
                        name: x["name"].as_str()?.to_owned(),
                        kind: x["agent"].as_str().unwrap_or_default().to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::parse_herdr_agents;

    #[test]
    fn herdr_agents_parse_and_unnamed_panes_are_skipped() {
        let json = br#"{"result":{"agents":[{"name":"w1","agent":"claude"},{"agent":"claude"},{"name":"w2"}]}}"#;
        let got = parse_herdr_agents(json);
        let names: Vec<_> = got
            .iter()
            .map(|h| (h.name.as_str(), h.kind.as_str()))
            .collect();
        assert_eq!(names, [("w1", "claude"), ("w2", "")]);
        assert!(parse_herdr_agents(b"not json").is_empty());
    }
}
