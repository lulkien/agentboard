//! `wait` on the SurrealDB backend: the port of `sqlite/wait.rs`. Argument checking,
//! the match rule and the `matched:` / `timeout:` text are the shared ones in
//! `waiting`; this file selects the watched set and polls the database.
//!
//! Consistency. A wait only reads, and takes no write lock: it never blocks a writer,
//! and it sleeps between polls without holding anything. A write is one server-side
//! transaction that commits before its writer releases the lock and returns, and each
//! poll is one read transaction (tasks and their new events together) that starts
//! after that, so a poll sees every write another process has committed. The poll
//! interval is the same 200 ms as the sqlite build.

use super::epics::fetch_epic;
use super::tasks::{fetch, get};
use super::{Db, Settings, db_err, lock, serve, session_with};
use crate::store::board::{Outcome, Wait};
use crate::store::waiting::{self, Ev, Seen};
use serde::Deserialize;
use std::collections::HashMap;
use std::future::Future;
use std::time::{Duration, Instant};

#[derive(Deserialize)]
struct Tid {
    tid: i64,
}

async fn task_ids(
    db: &Db,
    tail: &str,
    bind: Option<(&'static str, i64)>,
) -> Result<Vec<i64>, String> {
    let mut q = db.query(format!("SELECT meta::id(id) AS tid FROM tasks {tail}"));
    if let Some((k, v)) = bind {
        q = q.bind((k, v));
    }
    let mut r = q.await.map_err(|e| db_err(&e))?;
    let rows: Vec<Tid> = r.take(0).map_err(|e| db_err(&e))?;
    Ok(rows.into_iter().map(|t| t.tid).collect())
}

/// The watched set at call time: explicit ids, tasks of an epic, every open task. A
/// missing id or epic is an error, as is an empty set without `--open`.
async fn expand(db: &Db, ids: &[i64], epic: Option<i64>, open: bool) -> Result<Vec<i64>, String> {
    let mut out = ids.to_vec();
    for id in ids {
        fetch(db, *id).await?;
    }
    if let Some(e) = epic {
        fetch_epic(db, e).await?;
        out.extend(task_ids(db, "WHERE epic_id = $e", Some(("e", e))).await?);
    }
    if open {
        out.extend(task_ids(db, "WHERE status NOT IN ['done', 'dropped']", None).await?);
    }
    waiting::finish_set(out, open)
}

async fn max_of(db: &Db, table: &str, key: &str) -> Result<i64, String> {
    let mut r = db
        .query(format!(
            "SELECT meta::id(id) AS {key} FROM {table} ORDER BY {key} DESC LIMIT 1"
        ))
        .await
        .map_err(|e| db_err(&e))?;
    let rows: Vec<serde_json::Value> = r.take(0).map_err(|e| db_err(&e))?;
    Ok(rows
        .first()
        .and_then(|v| v.get(key))
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0))
}

/// One read transaction for the whole set: every watched task, and its events with
/// seq > `start`. For `--open` a task that finishes while we wait stays watched
/// (`IN set`, or an id above `open_from` for one created since).
async fn poll(
    db: &Db,
    set: &[i64],
    start: i64,
    open_from: Option<i64>,
) -> Result<Vec<Seen>, String> {
    #[derive(Deserialize)]
    struct T {
        tid: i64,
        status: String,
        assignee: Option<String>,
        updated_at: i64,
    }
    #[derive(Deserialize)]
    struct E {
        seq: i64,
        task_id: i64,
        actor: String,
        kind: String,
        body: String,
    }
    let scope = if open_from.is_some() {
        "(meta::id(id) IN $set OR meta::id(id) > $max OR status NOT IN ['done', 'dropped'])"
    } else {
        "meta::id(id) IN $set"
    };
    let mut r = db
        .query(format!(
            "BEGIN TRANSACTION;\n\
             SELECT meta::id(id) AS tid, status, assignee, updated_at FROM tasks WHERE {scope} ORDER BY tid;\n\
             SELECT meta::id(id) AS seq, task_id, actor, kind, body FROM events \
               WHERE task_id IN (SELECT VALUE meta::id(id) FROM tasks WHERE {scope}) AND meta::id(id) > $start ORDER BY seq;\n\
             COMMIT TRANSACTION;"
        ))
        .bind(("set", set.to_vec()))
        .bind(("max", open_from.unwrap_or(0)))
        .bind(("start", start))
        .await
        .map_err(|e| db_err(&e))?;
    if let Some((_, e)) = r.take_errors().into_iter().min_by_key(|(i, _)| *i) {
        return Err(db_err(&e));
    }
    let n = r.num_statements();
    let tasks: Vec<T> = r.take(n - 2).map_err(|e| db_err(&e))?;
    let events: Vec<E> = r.take(n - 1).map_err(|e| db_err(&e))?;
    let mut out: Vec<Seen> = tasks
        .into_iter()
        .map(|t| Seen {
            id: t.tid,
            status: t.status,
            assignee: t.assignee,
            updated_at: t.updated_at,
            events: Vec::new(),
        })
        .collect();
    for e in events {
        if let Some(t) = out.iter_mut().find(|t| t.id == e.task_id) {
            t.events.push(Ev {
                seq: e.seq,
                actor: e.actor,
                kind: e.kind,
                body: e.body,
            });
        }
    }
    Ok(out)
}

/// A task's latest event, shown as context when it already sits in the wanted status.
async fn last_event(db: &Db, id: i64) -> Option<Ev> {
    #[derive(Deserialize)]
    struct E {
        seq: i64,
        actor: String,
        kind: String,
        body: String,
    }
    let mut r = db
        .query("SELECT meta::id(id) AS seq, actor, kind, body FROM events WHERE task_id = $id ORDER BY seq DESC LIMIT 1")
        .bind(("id", id))
        .await
        .ok()?;
    let rows: Vec<E> = r.take(0).ok()?;
    rows.into_iter().next().map(|e| Ev {
        seq: e.seq,
        actor: e.actor,
        kind: e.kind,
        body: e.body,
    })
}

/// One database call, bounded. A wait is the one long-lived verb, and a server that
/// dies or wedges under it (a killed server leaves the WebSocket silent) must end it
/// with the usual one-line error instead of outliving its own timeout. No silent
/// reconnect: the caller re-issues the wait.
async fn within<T>(
    s: &Settings,
    budget: Duration,
    fut: impl Future<Output = Result<T, String>>,
) -> Result<T, String> {
    match tokio::time::timeout(budget, fut).await {
        Ok(r) => r,
        Err(_) => Err(serve::unavailable(
            s,
            &format!(
                "did not answer within {} ms while waiting for",
                budget.as_millis()
            ),
        )),
    }
}

/// Blocks until a watched task matches or the timeout hits. Reply starts with
/// `matched:` or `timeout:`. Explicit ids and `--epic` tasks are frozen at call time;
/// `--open` is re-evaluated on every poll. `after` is the last event seq the caller
/// already saw; without it the start is the current watermark.
pub async fn wait(w: Wait<'_>) -> Result<Outcome, String> {
    let settings = Settings::from_env()?;
    wait_on(&settings, w, serve::Timing::default().connect).await
}

/// `wait` against explicit settings, every database call bounded by `budget`.
pub(super) async fn wait_on(
    settings: &Settings,
    w: Wait<'_>,
    budget: Duration,
) -> Result<Outcome, String> {
    let waiting::Parsed {
        statuses,
        kinds,
        ids,
        epic,
    } = waiting::parse(&w)?;
    let s = session_with(
        settings,
        &serve::Timing::default(),
        None,
        lock::DEFAULT_WAIT,
        false,
    )?;
    let watched = within(settings, budget, expand(&s.db, &ids, epic, w.open)).await?;
    let start = match w.after {
        Some(seq) => seq,
        None => within(settings, budget, max_of(&s.db, "events", "seq")).await?,
    };
    let open_from = if w.open {
        Some(within(settings, budget, max_of(&s.db, "tasks", "tid")).await?)
    } else {
        None
    };
    let deadline = Instant::now() + Duration::from_millis(w.timeout_ms);
    loop {
        let seen = within(settings, budget, poll(&s.db, &watched, start, open_from)).await?;
        let hits = waiting::hits(&seen, statuses.as_deref(), kinds.as_deref());
        if !hits.is_empty() {
            let mut lasts: HashMap<i64, Ev> = HashMap::new();
            for (t, evs) in &hits {
                if !evs.is_empty() {
                    continue;
                }
                let last = within(settings, budget, async {
                    Ok::<_, String>(last_event(&s.db, t.id).await)
                })
                .await?;
                if let Some(e) = last {
                    lasts.insert(t.id, e);
                }
            }
            let lookup = |id: i64| {
                lasts.get(&id).map(|e| Ev {
                    seq: e.seq,
                    actor: e.actor.clone(),
                    kind: e.kind.clone(),
                    body: e.body.clone(),
                })
            };
            return Ok(Outcome {
                text: waiting::matched_text(seen.len(), &hits, &lookup),
                timed_out: false,
            });
        }
        if Instant::now() >= deadline {
            let tail = waiting::freshest(&seen).map(|id| get(id, false).unwrap_or_default());
            return Ok(Outcome {
                text: waiting::timeout_text(w.timeout_ms, &seen, tail.as_deref()),
                timed_out: true,
            });
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
