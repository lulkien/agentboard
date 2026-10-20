//! `wait`: server-side long-poll over a set of tasks. One connection and one
//! query per poll, whatever the size of the set.

use super::render::get;
use super::{STATUS_LIST, fetch, norm, open, parse_epic_id, parse_id, short};
use crate::clip::{CAP_LAST, clip_marked};
use crate::store::board::{Outcome, Wait};
use rusqlite::{Connection, params};
use std::time::{Duration, Instant};

/// First-line markers: MCP has no exit codes, so the verdict is in the text.
const MATCHED: &str = "matched:";
const TIMEOUT: &str = "timeout:";

/// Every `kind` the store writes. `ask` is not one: it logs a `block` event.
const KINDS: [&str; 11] = [
    "add",
    "claim",
    "report",
    "note",
    "block",
    "answer",
    "review",
    "done",
    "needs_fix",
    "edit",
    "dispatch",
];

fn norm_kind(raw: &str) -> Option<&'static str> {
    let k = raw.trim().to_ascii_lowercase();
    let k = if k == "ask" { "block" } else { k.as_str() };
    KINDS.into_iter().find(|x| *x == k)
}

/// `a,b,c` -> canonical names. An empty list or an unknown name is an error,
/// never a filter that silently matches nothing.
fn csv(
    raw: &str,
    what: &str,
    canon: fn(&str) -> Option<&'static str>,
    list: &str,
) -> Result<Vec<&'static str>, String> {
    let items: Vec<&str> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if items.is_empty() {
        return Err(format!("wait: empty --{what} list ({list})"));
    }
    items
        .into_iter()
        .map(|i| canon(i).ok_or_else(|| format!("wait: bad {what} {i:?} ({list})")))
        .collect()
}

/// The watched set at call time: explicit ids, direct children of an epic, every
/// open task. A missing id or epic is an error, as is an empty set without
/// `--open` (`--open` is a predicate: tasks may appear while waiting).
fn expand(
    conn: &Connection,
    ids: &[i64],
    epic: Option<i64>,
    open: bool,
) -> Result<Vec<i64>, String> {
    let mut out = ids.to_vec();
    for id in ids {
        fetch(conn, *id)?;
    }
    let mut column = |sql: &str, arg: &[i64]| -> Result<(), String> {
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(arg), |r| r.get::<_, i64>(0))
            .map_err(|e| e.to_string())?;
        for id in rows {
            out.push(id.map_err(|e| e.to_string())?);
        }
        Ok(())
    };
    if let Some(e) = epic {
        super::epics::fetch_epic(conn, e)?;
        column("SELECT id FROM tasks WHERE epic_id=?", &[e])?;
    }
    if open {
        column(
            "SELECT id FROM tasks WHERE status NOT IN ('done','dropped')",
            &[],
        )?;
    }
    out.sort_unstable();
    out.dedup();
    if out.is_empty() && !open {
        return Err("wait: selectors match no tasks".to_owned());
    }
    Ok(out)
}

struct Ev {
    seq: i64,
    actor: String,
    kind: String,
    body: String,
}

impl Ev {
    fn line(&self) -> String {
        // The delta is what the caller asked for (answers live here): never clipped.
        format!("#{} {} {}: {}", self.seq, self.actor, self.kind, self.body)
    }
}

struct Seen {
    id: i64,
    status: String,
    assignee: Option<String>,
    updated_at: i64,
    /// Events with seq > the start watermark.
    events: Vec<Ev>,
}

impl Seen {
    fn header(&self) -> String {
        format!(
            "{} {} {}",
            short(self.id),
            self.status,
            self.assignee.as_deref().unwrap_or("-")
        )
    }
}

/// The poll query. For `--open` a task that finishes while we wait stays watched
/// (`IN set`, or `id > ?2` for one created since), so its `done` event is not
/// lost to the predicate.
fn poll_sql(watched: &[i64], open: bool) -> String {
    let set = watched
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let scope = if open {
        format!("(t.id IN ({set}) OR t.id > ?2 OR t.status NOT IN ('done','dropped'))")
    } else {
        format!("t.id IN ({set})")
    };
    format!(
        "SELECT t.id,t.status,t.assignee,t.updated_at,e.seq,e.actor,e.kind,e.body \
         FROM tasks t LEFT JOIN events e ON e.task_id=t.id AND e.seq>?1 WHERE {scope} ORDER BY t.id,e.seq"
    )
}

/// One query for the whole set: every watched task, joined to its new events.
/// `open_from` is `Some(max task id at start)` for `--open`: the set then also
/// takes every currently open task and every task created since.
fn poll(
    conn: &Connection,
    sql: &str,
    start: i64,
    open_from: Option<i64>,
) -> Result<Vec<Seen>, String> {
    let mut stmt = conn.prepare_cached(sql).map_err(|e| e.to_string())?;
    let mut rows = match open_from {
        Some(max_id) => stmt.query(params![start, max_id]),
        None => stmt.query(params![start]),
    }
    .map_err(|e| e.to_string())?;
    let mut out: Vec<Seen> = Vec::new();
    while let Some(r) = rows.next().map_err(|e| e.to_string())? {
        let id: i64 = r.get(0).map_err(|e| e.to_string())?;
        if out.last().is_none_or(|t| t.id != id) {
            out.push(Seen {
                id,
                status: r.get(1).map_err(|e| e.to_string())?,
                assignee: r.get(2).map_err(|e| e.to_string())?,
                updated_at: r.get(3).map_err(|e| e.to_string())?,
                events: Vec::new(),
            });
        }
        if let Some(seq) = r.get::<_, Option<i64>>(4).map_err(|e| e.to_string())? {
            let ev = Ev {
                seq,
                actor: r.get(5).map_err(|e| e.to_string())?,
                kind: r.get(6).map_err(|e| e.to_string())?,
                body: r.get(7).map_err(|e| e.to_string())?,
            };
            out.last_mut().expect("pushed above").events.push(ev);
        }
    }
    Ok(out)
}

/// Blocks server-side (no agent-side polling) until a watched task matches or
/// the timeout hits. Reply starts with `matched:` or `timeout:`.
///
/// Explicit ids and `--epic` children are frozen at call time (they name work);
/// `--open` is re-evaluated on every poll, so a task created mid-wait is watched.
///
/// A task matches when its current status passes `status` (if given) and, unless
/// `status` is the only filter, it has a new event passing `kind` (if given).
/// `after` is the last event seq the caller already saw; without it the start is
/// the current watermark: "wait for the next thing to happen".
pub async fn wait(w: Wait<'_>) -> Result<Outcome, String> {
    let statuses = w
        .status
        .map(|s| csv(s, "status", norm, STATUS_LIST))
        .transpose()?;
    let kinds = w
        .kind
        .map(|k| csv(k, "kind", norm_kind, &KINDS.join("|")))
        .transpose()?;
    let ids = w
        .ids
        .iter()
        .map(|s| parse_id(s))
        .collect::<Result<Vec<_>, _>>()?;
    let epic = w.epic.map(parse_epic_id).transpose()?;
    if ids.is_empty() && epic.is_none() && !w.open {
        return Err("wait: name tasks (wait T1 T2), --epic E<n> or --open".to_owned());
    }

    // Set and watermark come from the connection that polls, in one read
    // snapshot: an event landing after it has seq > start and is seen by poll #1.
    let conn = open()?;
    let (watched, start, max_id) = {
        let snap = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        let watched = expand(&snap, &ids, epic, w.open)?;
        let start = match w.after {
            Some(seq) => seq,
            None => snap
                .query_row("SELECT COALESCE(MAX(seq),0) FROM events", [], |r| r.get(0))
                .map_err(|e| e.to_string())?,
        };
        let max_id: i64 = snap
            .query_row("SELECT COALESCE(MAX(id),0) FROM tasks", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        (watched, start, max_id)
    };
    let sql = poll_sql(&watched, w.open);
    let open_from = w.open.then_some(max_id);

    let deadline = Instant::now() + Duration::from_millis(w.timeout_ms);
    loop {
        let seen = poll(&conn, &sql, start, open_from)?;
        let hits: Vec<(&Seen, Vec<&Ev>)> = seen
            .iter()
            .filter_map(|t| {
                if !statuses
                    .as_ref()
                    .is_none_or(|s| s.contains(&t.status.as_str()))
                {
                    return None;
                }
                let evs: Vec<&Ev> = t
                    .events
                    .iter()
                    .filter(|e| kinds.as_ref().is_none_or(|k| k.contains(&e.kind.as_str())))
                    .collect();
                let needs_event = kinds.is_some() || statuses.is_none();
                (!needs_event || !evs.is_empty()).then_some((t, evs))
            })
            .collect();
        if !hits.is_empty() {
            let mut out = format!("{MATCHED} {} of {} watched\n", hits.len(), seen.len());
            for (t, evs) in hits {
                out.push_str(&t.header());
                out.push('\n');
                if evs.is_empty() {
                    // Already in the wanted status with nothing new. The latest event
                    // is context only: it may postdate the move that got it here.
                    let last = conn
                        .query_row("SELECT seq,actor,kind,body FROM events WHERE task_id=? ORDER BY seq DESC LIMIT 1", params![t.id], |r| {
                            Ok(Ev { seq: r.get(0)?, actor: r.get(1)?, kind: r.get(2)?, body: r.get(3)? })
                        })
                        .ok();
                    out.extend(last.map(|e| format!("last: {}\n", e.line())));
                }
                for e in evs {
                    out.push_str(&e.line());
                    out.push('\n');
                }
            }
            return Ok(Outcome {
                text: out.trim_end().to_owned(),
                timed_out: false,
            });
        }
        if Instant::now() >= deadline {
            // Never return blind: every status, plus the tail of the freshest task.
            let mut out = format!(
                "{TIMEOUT} nothing matched after {}s ({} watched)\n",
                w.timeout_ms / 1000,
                seen.len()
            );
            for t in &seen {
                out.push_str(&t.header());
                out.push('\n');
            }
            if let Some(t) = seen.iter().max_by_key(|t| (t.updated_at, t.id)) {
                let tag = short(t.id);
                out.push_str(&format!(
                    "last ({tag}): {}",
                    clip_marked(
                        &get(t.id, false).unwrap_or_default(),
                        CAP_LAST,
                        &format!("task_get id={tag} for the record")
                    )
                ));
            }
            return Ok(Outcome {
                text: out,
                timed_out: true,
            });
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(super::super::SCHEMA).unwrap();
        // E1 holds T2 (working) and T3 (done); E2 is empty; T1 and T4 belong to no epic.
        for id in [1, 2] {
            conn.execute(
                "INSERT INTO epics(id,title,created_at,updated_at) VALUES(?,?,0,0)",
                params![id, format!("e{id}")],
            )
            .unwrap();
        }
        for (id, status, epic) in [
            (1, "working", None),
            (2, "working", Some(1)),
            (3, "done", Some(1)),
            (4, "ready", None),
        ] {
            conn.execute(
                "INSERT INTO tasks(id,title,status,epic_id,created_at,updated_at) VALUES(?,?,?,?,0,0)",
                params![id, format!("t{id}"), status, epic],
            )
            .unwrap();
        }
        conn
    }

    #[test]
    fn csv_accepts_known_names_and_aliases() {
        assert_eq!(
            csv("review, done", "status", norm, STATUS_LIST).unwrap(),
            vec!["review", "done"]
        );
        assert_eq!(
            csv("running", "status", norm, STATUS_LIST).unwrap(),
            vec!["working"]
        );
        assert_eq!(
            csv("ask,note", "kind", norm_kind, "k").unwrap(),
            vec!["block", "note"]
        );
    }

    #[test]
    fn csv_rejects_unknown_and_empty() {
        assert!(
            csv("rview", "status", norm, STATUS_LIST)
                .unwrap_err()
                .contains("bad status \"rview\"")
        );
        assert!(
            csv("note,nope", "kind", norm_kind, "k")
                .unwrap_err()
                .contains("bad kind \"nope\"")
        );
        assert!(
            csv("", "status", norm, STATUS_LIST)
                .unwrap_err()
                .contains("empty --status")
        );
        assert!(
            csv(" , ", "kind", norm_kind, "k")
                .unwrap_err()
                .contains("empty --kind")
        );
    }

    #[test]
    fn expand_unions_dedups_and_fails_fast() {
        let c = db();
        assert_eq!(expand(&c, &[4, 2, 4], None, false).unwrap(), vec![2, 4]);
        assert_eq!(
            expand(&c, &[], Some(1), false).unwrap(),
            vec![2, 3],
            "epic selects the tasks in it"
        );
        assert_eq!(
            expand(&c, &[], None, true).unwrap(),
            vec![1, 2, 4],
            "open skips done"
        );
        assert_eq!(
            expand(&c, &[3, 4], Some(1), true).unwrap(),
            vec![1, 2, 3, 4]
        );
        assert!(
            expand(&c, &[9], None, false)
                .unwrap_err()
                .contains("no such task")
        );
        assert!(
            expand(&c, &[], Some(9), false)
                .unwrap_err()
                .contains("no such epic")
        );
        assert!(
            expand(&c, &[], Some(2), false)
                .unwrap_err()
                .contains("match no tasks")
        );
    }

    fn ev(c: &Connection, task: i64, kind: &str) -> i64 {
        c.execute(
            "INSERT INTO events(task_id,actor,kind,body,ts) VALUES(?,?,?,?,0)",
            params![task, "a", kind, ""],
        )
        .unwrap();
        c.last_insert_rowid()
    }

    #[test]
    fn poll_orders_by_task_then_seq_and_honours_start() {
        let c = db();
        let (e1, e2, e3, e4) = (
            ev(&c, 4, "note"),
            ev(&c, 2, "note"),
            ev(&c, 4, "done"),
            ev(&c, 2, "review"),
        );
        let seen = poll(&c, &poll_sql(&[4, 2], false), e1 - 1, None).unwrap();
        assert_eq!(
            seen.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![2, 4],
            "task id ascending"
        );
        assert_eq!(
            seen[0].events.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![e2, e4],
            "seq ascending"
        );
        assert_eq!(
            seen[1].events.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![e1, e3]
        );
        let after = poll(&c, &poll_sql(&[4, 2], false), e3, None).unwrap();
        assert_eq!(after[0].events.len(), 1, "seq > start only");
        assert!(
            after[1].events.is_empty(),
            "a quiet task still appears, with no events"
        );
    }

    #[test]
    fn open_scope_sees_new_tasks_and_keeps_finished_ones() {
        let c = db();
        let (start, max_id) = (0, 4);
        let sql = poll_sql(&[1, 2, 4], true);
        let ids = |c: &Connection| {
            poll(c, &sql, start, Some(max_id))
                .unwrap()
                .into_iter()
                .map(|t| t.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&c), vec![1, 2, 4]);
        // Created mid-wait, then finished before the next poll: still seen.
        c.execute(
            "INSERT INTO tasks(id,title,status,created_at,updated_at) VALUES(5,'new','done',0,0)",
            [],
        )
        .unwrap();
        // Watched task finishes: stays in the set.
        c.execute("UPDATE tasks SET status='done' WHERE id=2", [])
            .unwrap();
        assert_eq!(ids(&c), vec![1, 2, 4, 5]);
        // Frozen scope never grows.
        assert_eq!(
            poll(&c, &poll_sql(&[1, 2, 4], false), start, None)
                .unwrap()
                .len(),
            3
        );
        // Re-opened old task joins an open wait that started without it.
        c.execute("UPDATE tasks SET status='ready' WHERE id=3", [])
            .unwrap();
        assert_eq!(
            poll(&c, &poll_sql(&[1], true), start, Some(max_id))
                .unwrap()
                .iter()
                .map(|t| t.id)
                .collect::<Vec<_>>(),
            vec![1, 3, 4, 5]
        );
    }
}
