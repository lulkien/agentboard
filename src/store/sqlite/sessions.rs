use super::{fetch, now_ms, open};
use crate::store::bridge::{Liveness, tmux_up};
use crate::store::short;
use crate::store::text::ago;
use rusqlite::params;

/// One registry row as `sessions` lists it. `cwd` is stored but not shown.
struct SessionRow {
    name: String,
    agent: String,
    task: Option<i64>,
    seen: i64,
}

/// Newest first; rows seen in the same millisecond come out by name (SQLite BINARY
/// collation: byte order), so the listing never depends on insertion order.
fn rows(conn: &rusqlite::Connection) -> Result<Vec<SessionRow>, String> {
    let mut stmt = conn
        .prepare("SELECT name,agent,task_id,seen FROM sessions ORDER BY seen DESC, name")
        .map_err(|e| e.to_string())?;
    stmt.query_map([], |r| {
        Ok(SessionRow {
            name: r.get(0)?,
            agent: r.get(1)?,
            task: r.get(2)?,
            seen: r.get(3)?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<rusqlite::Result<Vec<_>>>()
    .map_err(|e| e.to_string())
}

pub fn sessions() -> Result<String, String> {
    let conn = open()?;
    let names = rows(&conn)?;
    if names.is_empty() {
        return Ok("(no sessions registered)".to_owned());
    }
    let live = Liveness::new();
    let mut out = String::new();
    for SessionRow {
        name,
        agent,
        task,
        seen,
    } in names
    {
        let alive = live.alive(&name, &agent);
        out.push_str(&format!(
            "{name} {agent} {} {} {}\n",
            if alive { "up" } else { "gone" },
            task.map_or("-".to_owned(), short),
            ago(seen)
        ));
    }
    Ok(out.trim_end().to_owned())
}

/// Deletes rows whose pane is not alive. Rows never expire on their own, and a
/// dead row captures a dispatch to its name, so this is the supported cleanup.
/// Refuses when neither tmux nor herdr answers: every row would look dead.
pub fn prune(dry_run: bool) -> Result<String, String> {
    let conn = open()?;
    let all = rows(&conn)?;
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
            conn.execute("DELETE FROM sessions WHERE name=?", params![r.name])
                .map_err(|e| e.to_string())?;
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

pub fn unregister(name: &str) -> Result<String, String> {
    let conn = open()?;
    let n = conn
        .execute("DELETE FROM sessions WHERE name=?", params![name])
        .map_err(|e| e.to_string())?;
    if n == 0 {
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
    let conn = open()?;
    if let Some(t) = task {
        fetch(&conn, t)?;
    }
    conn.execute(
        "INSERT INTO sessions(name,agent,cwd,task_id,seen,status) VALUES(?,?,?,?,?,'up')
         ON CONFLICT(name) DO UPDATE SET agent=excluded.agent, cwd=excluded.cwd, task_id=excluded.task_id, seen=excluded.seen, status='up'",
        params![name, agent, cwd, task, now_ms()],
    )
    .map_err(|e| e.to_string())?;
    Ok(format!("ok session {name} up"))
}

#[cfg(test)]
mod tests {
    use super::rows;

    #[test]
    fn rows_with_the_same_seen_list_by_name_in_byte_order() {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        c.execute_batch(crate::store::sqlite::SCHEMA).unwrap();
        // inserted out of order, all seen in the same millisecond, one newer row
        for name in ["mid", "émile", "alpha", "Zed"] {
            c.execute(
                "INSERT INTO sessions(name,agent,seen) VALUES(?,?,?)",
                rusqlite::params![name, "claude", 1000],
            )
            .unwrap();
        }
        c.execute(
            "INSERT INTO sessions(name,agent,seen) VALUES('newest','claude',2000)",
            [],
        )
        .unwrap();
        let got: Vec<String> = rows(&c).unwrap().into_iter().map(|r| r.name).collect();
        // newest first, then ties by byte order: uppercase before lowercase, non-ASCII last
        assert_eq!(got, ["newest", "Zed", "alpha", "mid", "émile"]);
    }
}
