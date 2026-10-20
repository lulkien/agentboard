//! Task operations on the SurrealDB backend: the port of `sqlite/tasks.rs` plus the
//! task halves of `sqlite/render.rs` (`list`, `get`, `children`). Same rendered text,
//! same ids, same events.
//!
//! Locking. Every write opens a write session, which takes the cross-process write
//! lock before it reads anything and holds it until the operation returns, so the
//! read-modify-write is one critical section. Each write is also one server-side
//! transaction (`BEGIN`..`COMMIT`), so a crash cannot leave an update without its
//! event or an id spent without its row. Reads take no lock: they see a write whole
//! or not at all.
//!
//! The rendering helpers below (`code`, `ago`, `stamp`, `when`, the change-diff
//! helpers) are copies of the pure ones in `sqlite/`; hoisting them into a shared
//! module is listed in the migration doc's debt table.

use super::{Db, block_on, db_err, session};
use crate::clip::{
    CAP_DESCRIPTION, CAP_EVENT, CAP_GOAL, CAP_NOTE, CAP_REPORT, clip, clip_marked, field,
};
use crate::store::board::{ClaimError, NewTask, Patch, TaskEdit};
use crate::store::{DEFAULT_TTL_S, Ref, norm, now_ms, short, short_epic};
use serde::Deserialize;
use serde_json::{Map, Value, json};

const STATUS_LIST: &str = "ready|working|blocking|needs_fix|review|done|dropped";

/// A task row. `tid` is the integer record key (`meta::id(id)`).
#[derive(Deserialize, Clone, Debug)]
pub(super) struct Row {
    tid: i64,
    status: String,
    assignee: Option<String>,
    needs: Option<String>,
    title: String,
    description: String,
    goal: String,
    lease_until: Option<i64>,
    updated_at: i64,
    created_at: i64,
    started_at: Option<i64>,
    completed_at: Option<i64>,
    report: Option<String>,
    note: Option<String>,
    commit_ref: Option<String>,
    priority: i64,
    epic_id: Option<i64>,
    parent_id: Option<i64>,
}

const SELECT_TASK: &str = "SELECT meta::id(id) AS tid, * FROM tasks";

pub(super) async fn fetch_opt(db: &Db, id: i64) -> Result<Option<Row>, String> {
    let mut r = db
        .query(format!(
            "{SELECT_TASK} WHERE id = type::thing('tasks', $id)"
        ))
        .bind(("id", id))
        .await
        .map_err(|e| db_err(&e))?;
    let rows: Vec<Row> = r.take(0).map_err(|e| db_err(&e))?;
    Ok(rows.into_iter().next())
}

async fn fetch(db: &Db, id: i64) -> Result<Row, String> {
    fetch_opt(db, id)
        .await?
        .ok_or_else(|| format!("no such task {}", short(id)))
}

async fn select_rows(db: &Db, tail: &str, binds: Value) -> Result<Vec<Row>, String> {
    let mut r = db
        .query(format!("{SELECT_TASK} {tail}"))
        .bind(binds)
        .await
        .map_err(|e| db_err(&e))?;
    r.take(0).map_err(|e| db_err(&e))
}

/// One write transaction: statements are appended with their values bound as
/// parameters, then `run` commits and returns the value of a closing `RETURN`.
pub(super) struct Txn {
    sql: String,
    binds: Map<String, Value>,
}

impl Txn {
    pub(super) fn new() -> Self {
        Self {
            sql: "BEGIN TRANSACTION;\n".to_owned(),
            binds: Map::new(),
        }
    }

    /// Binds a value and returns its `$pN` name.
    pub(super) fn p(&mut self, v: impl Into<Value>) -> String {
        let name = format!("p{}", self.binds.len());
        self.binds.insert(name.clone(), v.into());
        format!("${name}")
    }

    pub(super) fn raw(&mut self, s: &str) {
        self.sql.push_str(s);
        self.sql.push('\n');
    }

    /// Allocates the next event seq and records an event on a task (`task` is a
    /// SurrealQL expression for the task id). The seq is left in `$seq`.
    fn event(&mut self, task: &str, actor: &str, kind: &str, body: &str) {
        self.event_on("task_id", task, actor, kind, body);
    }

    /// The same for an event on an epic (`col` is `epic_id`).
    pub(super) fn event_on(
        &mut self,
        col: &str,
        subject: &str,
        actor: &str,
        kind: &str,
        body: &str,
    ) {
        let (a, k, b, ts) = (self.p(actor), self.p(kind), self.p(body), self.p(now_ms()));
        self.raw("LET $seq = (UPSERT counters:event SET n = (n ?? 0) + 1 RETURN VALUE n)[0];");
        self.raw(&format!(
            "CREATE type::thing('events', $seq) SET {col} = {subject}, actor = {a}, kind = {k}, body = {b}, ts = {ts};"
        ));
    }

    pub(super) async fn run(mut self, db: &Db, ret: &str) -> Result<Value, String> {
        self.raw(&format!("RETURN {ret};\nCOMMIT TRANSACTION;"));
        let mut res = db
            .query(self.sql)
            .bind(Value::Object(self.binds))
            .await
            .map_err(|e| db_err(&e))?;
        if let Some((_, e)) = res.take_errors().into_iter().min_by_key(|(i, _)| *i) {
            return Err(db_err(&e));
        }
        let last = res.num_statements().saturating_sub(1);
        let v: Option<Value> = res.take(last).map_err(|e| db_err(&e))?;
        Ok(v.unwrap_or(Value::Null))
    }
}

/// `field = $pN` for a value, `field = NONE` for absent.
pub(super) fn assign(tx: &mut Txn, sets: &mut Vec<String>, name: &str, v: Option<Value>) {
    match v {
        Some(v) => {
            let p = tx.p(v);
            sets.push(format!("{name} = {p}"));
        }
        None => sets.push(format!("{name} = NONE")),
    }
}

pub(super) fn int_of(v: &Value, what: &str) -> Result<i64, String> {
    v.as_i64()
        .ok_or_else(|| format!("board database: no {what} came back ({v})"))
}

// ---- pure helpers (copies of the sqlite ones) --------------------------------------

fn code(status: &str) -> &str {
    match status {
        "ready" | "triage" => "rdy",
        "working" | "running" => "wrk",
        "blocking" | "blocked" => "blk",
        "needs_fix" => "fix",
        "review" => "rev",
        "done" => "don",
        "dropped" => "drp",
        other => other,
    }
}

pub(super) fn ago(ts_ms: i64) -> String {
    let d = (now_ms() - ts_ms).max(0) / 1000;
    match d {
        0..=59 => format!("{d}s"),
        60..=3599 => format!("{}m", d / 60),
        3600..=86_399 => format!("{}h", d / 3600),
        _ => format!("{}d", d / 86_400),
    }
}

fn stamp(ts_ms: i64) -> String {
    let secs = ts_ms / 1000;
    let (h, m) = ((secs / 3600) % 24, (secs / 60) % 60);
    format!("{h:02}:{m:02}")
}

pub(super) fn when(ts: i64) -> String {
    format!("{} ({}Z)", ago(ts), stamp(ts))
}

pub(super) fn text_change<'a>(
    diffs: &mut Vec<String>,
    name: &str,
    old: &'a str,
    new: Option<&'a str>,
) -> &'a str {
    let value = new.unwrap_or(old);
    if value != old {
        diffs.push(format!(
            "{name} {}->{}",
            old.chars().count(),
            value.chars().count()
        ));
    }
    value
}

pub(super) fn value_change<'a>(
    diffs: &mut Vec<String>,
    name: &str,
    old: &'a str,
    new: Option<&'a str>,
) -> &'a str {
    let value = new.unwrap_or(old);
    if value != old {
        diffs.push(format!(
            "{name} \"{}\"->\"{}\"",
            clip(old, 80),
            clip(value, 80)
        ));
    }
    value
}

pub(super) fn field_names(diffs: &[String]) -> String {
    let names: Vec<&str> = diffs
        .iter()
        .filter_map(|d| d.split_whitespace().next())
        .collect();
    names.join(", ")
}

/// The refusal for a task somebody holds: who, the lease, and the manager's way out.
fn held_text(r: &Row) -> String {
    format!(
        "{} {} by {}{} — manager: agentboard status {} ready",
        short(r.tid),
        r.status,
        r.assignee.as_deref().unwrap_or("nobody"),
        r.lease_until.map_or(String::new(), |l| format!(
            " (lease {}m left, until {}Z)",
            ((l - now_ms()).max(0) + 59_999) / 60_000,
            stamp(l)
        )),
        short(r.tid)
    )
}

// ---- writes -------------------------------------------------------------------------

/// The epic a new task lands in: the explicit one, else the parent's. An
/// explicit epic that disagrees with the parent's is an error: a subtask lives in
/// its parent's epic.
async fn resolve_epic(
    db: &Db,
    epic: Option<i64>,
    parent: Option<i64>,
) -> Result<Option<i64>, String> {
    let parent_epic = match parent {
        Some(p) => {
            let row = fetch(db, p)
                .await
                .map_err(|_| format!("add: no such parent task {}", short(p)))?;
            Some((p, row.epic_id))
        }
        None => None,
    };
    if let Some(e) = epic {
        super::epics::fetch_epic(db, e)
            .await
            .map_err(|_| format!("add: no such epic {}", short_epic(e)))?;
    }
    match (epic, parent_epic) {
        (Some(e), Some((p, pe))) if pe != Some(e) => Err(format!(
            "add: parent {} belongs to {}, not {} (a subtask lives in its parent's epic)",
            short(p),
            pe.map_or("no epic".to_owned(), short_epic),
            short_epic(e)
        )),
        (None, Some((_, pe))) => Ok(pe),
        (e, _) => Ok(e),
    }
}

pub fn add(actor: &str, task: &NewTask<'_>) -> Result<String, String> {
    let NewTask {
        title,
        description,
        goal,
        status,
        assignee,
        priority,
        epic,
        parent,
    } = *task;
    let s = session(true)?;
    block_on(async {
        let epic = resolve_epic(&s.db, epic, parent).await?;
        // A task created already `working` belongs to its creator unless named.
        let assignee = assignee.or_else(|| (status == "working").then_some(actor));
        let now = now_ms();
        // A `working` task needs a lease, or a dead holder could never be reclaimed.
        let lease = (status == "working").then(|| now + DEFAULT_TTL_S * 1000);
        let started = (status == "working").then_some(now);
        let mut content = json!({
            "title": title, "description": description, "goal": goal, "status": status,
            "priority": priority, "created_at": now, "updated_at": now,
        });
        let o = content.as_object_mut().expect("object");
        for (k, v) in [
            ("assignee", assignee.map(Value::from)),
            ("epic_id", epic.map(Value::from)),
            ("parent_id", parent.map(Value::from)),
            ("lease_until", lease.map(Value::from)),
            ("started_at", started.map(Value::from)),
        ] {
            if let Some(v) = v {
                o.insert(k.to_owned(), v);
            }
        }
        // Allocate the id and insert the row in one transaction, under the lock.
        let mut tx = Txn::new();
        tx.raw("LET $id = (UPSERT counters:task SET n = (n ?? 0) + 1 RETURN VALUE n)[0];");
        let c = tx.p(content);
        tx.raw(&format!("CREATE type::thing('tasks', $id) CONTENT {c};"));
        tx.event("$id", actor, "add", title);
        let id = tx
            .run(&s.db, "$id")
            .await
            .map_err(|e| format!("add: {e}"))
            .and_then(|v| int_of(&v, "task id"))?;
        Ok(format!("{} {status}", short(id)))
    })
}

/// Amends a task's spec. A task a worker holds (`working` with a live lease) is
/// refused: that worker is reading this text. Logs one `edit` event naming each
/// changed field with its old/new length; nothing is written when no value moved.
pub fn edit(actor: &str, id: i64, e: &TaskEdit<'_>) -> Result<String, ClaimError> {
    if e.title.is_none()
        && e.description.is_none()
        && e.goal.is_none()
        && e.priority.is_none()
        && e.assignee.is_none()
    {
        return Err(String::from(
            "nothing to edit: pass title, description, goal, priority or assignee",
        )
        .into());
    }
    if e.title.is_some_and(|t| t.trim().is_empty()) {
        return Err(String::from("title cannot be empty").into());
    }
    let s = session(true)?;
    block_on(async {
        let cur = fetch(&s.db, id).await?;
        if cur.status == "working" && cur.lease_until.is_some_and(|l| l >= now_ms()) {
            return Err(ClaimError::Held(held_text(&cur)));
        }
        let mut diffs = Vec::new();
        let title = value_change(&mut diffs, "title", &cur.title, e.title);
        let description = text_change(&mut diffs, "description", &cur.description, e.description);
        let goal = text_change(&mut diffs, "goal", &cur.goal, e.goal);
        let assignee = value_change(
            &mut diffs,
            "assignee",
            cur.assignee.as_deref().unwrap_or(""),
            e.assignee,
        );
        let priority = e.priority.unwrap_or(cur.priority);
        if priority != cur.priority {
            diffs.push(format!("priority {}->{priority}", cur.priority));
        }
        if diffs.is_empty() {
            return Ok(format!("{} unchanged", short(id)));
        }
        let mut tx = Txn::new();
        let mut sets = Vec::new();
        assign(&mut tx, &mut sets, "title", Some(title.into()));
        assign(&mut tx, &mut sets, "description", Some(description.into()));
        assign(&mut tx, &mut sets, "goal", Some(goal.into()));
        assign(
            &mut tx,
            &mut sets,
            "assignee",
            (!assignee.is_empty()).then(|| assignee.into()),
        );
        assign(&mut tx, &mut sets, "priority", Some(priority.into()));
        assign(&mut tx, &mut sets, "updated_at", Some(now_ms().into()));
        let idp = tx.p(id);
        tx.raw(&format!(
            "UPDATE type::thing('tasks', {idp}) SET {};",
            sets.join(", ")
        ));
        tx.event(&idp, actor, "edit", &diffs.join(", "));
        let seq = int_of(&tx.run(&s.db, "$seq").await?, "event seq")?;
        Ok(format!(
            "{} edited {} #{seq}",
            short(id),
            field_names(&diffs)
        ))
    })
}

/// Re-parents an existing task (`None` clears it). The only write that can make
/// a cycle, so the walk up the ancestors happens here, under the same lock.
pub fn set_parent(actor: &str, id: i64, parent: Option<i64>) -> Result<String, String> {
    let s = session(true)?;
    block_on(async {
        let task = fetch(&s.db, id).await?;
        if let Some(p) = parent {
            if p == id {
                return Err(format!("{} cannot be its own parent", short(id)));
            }
            let new_parent = fetch(&s.db, p).await?;
            if new_parent.epic_id != task.epic_id {
                return Err(format!(
                    "cross-epic parent: {} belongs to {} but {} belongs to {}",
                    short(id),
                    task.epic_id.map_or("no epic".to_owned(), short_epic),
                    short(p),
                    new_parent.epic_id.map_or("no epic".to_owned(), short_epic)
                ));
            }
            // Walk up from the new parent: meeting `id` means `id` is its ancestor.
            let mut at = new_parent.parent_id;
            let mut steps = 0;
            while let Some(a) = at {
                if a == id || steps > 10_000 {
                    return Err(format!(
                        "cycle: {} is already an ancestor of {}",
                        short(id),
                        short(p)
                    ));
                }
                at = fetch(&s.db, a).await?.parent_id;
                steps += 1;
            }
        }
        let mut tx = Txn::new();
        let mut sets = Vec::new();
        assign(&mut tx, &mut sets, "parent_id", parent.map(Value::from));
        assign(&mut tx, &mut sets, "updated_at", Some(now_ms().into()));
        let idp = tx.p(id);
        tx.raw(&format!(
            "UPDATE type::thing('tasks', {idp}) SET {};",
            sets.join(", ")
        ));
        let body = parent.map_or("parent cleared".to_owned(), |p| {
            format!("parent -> {}", short(p))
        });
        tx.event(&idp, actor, "edit", &body);
        tx.run(&s.db, "$seq").await?;
        Ok(parent.map_or(format!("{} no parent", short(id)), |p| {
            format!("{} parent {}", short(id), short(p))
        }))
    })
}

/// Claims a specific task, or the next ready one. Under the write lock the
/// candidate pick and the guarded `UPDATE` cannot interleave with another claim;
/// the guard (`status='ready'` or an expired lease) is also in the `UPDATE`
/// itself, so a row that stopped being claimable is never handed out.
pub fn claim(actor: &str, id: Option<i64>, ttl_s: i64) -> Result<String, ClaimError> {
    let s = session(true)?;
    block_on(async {
        let now = now_ms();
        let lease = now + ttl_s * 1000;
        for _ in 0..8 {
            let cand: Option<i64> = match id {
                Some(x) => match fetch_opt(&s.db, x).await? {
                    None => return Err(format!("no such task {}", short(x)).into()),
                    Some(r) => {
                        let stale = r.lease_until.is_some_and(|l| l < now);
                        if r.status == "ready" || stale {
                            Some(x)
                        } else {
                            // A refusal is an error: `agentboard claim T7 && ...` must not
                            // read "somebody else has it" as "claimed".
                            return Err(ClaimError::Held(held_text(&r)));
                        }
                    }
                },
                None => {
                    let mut r = s
                        .db
                        .query("SELECT meta::id(id) AS tid, priority FROM tasks WHERE status = 'ready' ORDER BY priority, tid LIMIT 1")
                        .await
                        .map_err(|e| db_err(&e))?;
                    #[derive(Deserialize)]
                    struct Pick {
                        tid: i64,
                    }
                    let v: Vec<Pick> = r.take(0).map_err(|e| db_err(&e))?;
                    v.first().map(|p| p.tid)
                }
            };
            let Some(task) = cand else {
                return Ok("no ready tasks".to_owned());
            };
            // The guard compares the OLD lease against `now`; comparing it against
            // the new lease would let a live claim be stolen.
            let mut tx = Txn::new();
            let (idp, a, l, n) = (tx.p(task), tx.p(actor), tx.p(lease), tx.p(now));
            tx.raw(&format!(
                "LET $won = (UPDATE type::thing('tasks', {idp}) SET status = 'working', assignee = {a}, lease_until = {l}, updated_at = {n}, started_at = started_at ?? {n} WHERE status = 'ready' OR (lease_until != NONE AND lease_until < {n}) RETURN VALUE meta::id(id));"
            ));
            tx.raw("IF array::len($won) = 1 {");
            tx.event(&idp, actor, "claim", "");
            tx.raw("};");
            let won = tx.run(&s.db, "array::len($won)").await?;
            if won.as_i64() != Some(1) {
                continue;
            }
            let r = fetch(&s.db, task).await?;
            let mut out = format!(
                "{} working (lease {}m) {}\n",
                short(r.tid),
                ttl_s / 60,
                r.title
            );
            // claim is the worker's entry point, not a display path: the prompt
            // arrives byte-for-byte (newlines, fences) and uncapped.
            out.push_str(&field(
                "description",
                &r.description,
                CAP_DESCRIPTION,
                true,
                &short(r.tid),
            ));
            out.push_str(&field("goal", &r.goal, CAP_GOAL, true, &short(r.tid)));
            return Ok(out);
        }
        Err("claim contention: retry".to_owned().into())
    })
}

pub fn apply(actor: &str, id: i64, p: &Patch<'_>) -> Result<String, String> {
    let s = session(true)?;
    block_on(async {
        let cur = fetch(&s.db, id).await?;
        let now = now_ms();
        let status = match p.status {
            Some(raw) => norm(raw).ok_or_else(|| format!("bad status {raw:?} ({STATUS_LIST})"))?,
            None => cur.status.as_str(),
        };
        let moved = status != cur.status;
        let mut tx = Txn::new();
        let mut sets: Vec<String> = Vec::new();
        if let Some(rp) = p.report {
            assign(&mut tx, &mut sets, "report", Some(rp.into()));
        }
        if let Some(n) = p.note.filter(|n| !n.trim().is_empty()) {
            assign(&mut tx, &mut sets, "note", Some(n.into()));
        }
        if let Some(c) = p.commit {
            assign(&mut tx, &mut sets, "commit_ref", Some(c.into()));
        }
        let needs = p
            .needs
            .map(str::to_owned)
            .or_else(|| (status == "blocking").then(|| "input".to_owned()));
        let ttl = if p.ttl_s > 0 { p.ttl_s } else { DEFAULT_TTL_S };
        assign(&mut tx, &mut sets, "status", Some(status.into()));
        assign(&mut tx, &mut sets, "needs", needs.map(Value::from));
        assign(&mut tx, &mut sets, "updated_at", Some(now.into()));
        let nowp = tx.p(now);
        match status {
            "working" => {
                // Only entering `working` (or a missing lease) sets one: plain notes and
                // reports must not keep a dead holder's lease alive. Heartbeat extends.
                let lease = match cur.lease_until {
                    Some(old) if !moved => old,
                    Some(old) => old.max(now + ttl * 1000),
                    None => now + ttl * 1000,
                };
                assign(&mut tx, &mut sets, "lease_until", Some(lease.into()));
                let a = tx.p(actor);
                sets.push(format!("assignee = assignee ?? {a}"));
                sets.push(format!("started_at = started_at ?? {nowp}"));
                sets.push("completed_at = NONE".to_owned());
            }
            "done" | "dropped" => {
                sets.push("lease_until = NONE".to_owned());
                sets.push(format!("completed_at = completed_at ?? {nowp}"));
            }
            "ready" => {
                sets.push("lease_until = NONE".to_owned());
                sets.push("started_at = NONE".to_owned());
                sets.push("completed_at = NONE".to_owned());
            }
            // `needs_fix` waits on someone else's fix, so it holds no lease. Same for
            // blocking and review.
            _ => sets.push("lease_until = NONE".to_owned()),
        }
        let kind = match (moved, status) {
            (true, "done") => "done",
            (true, "review") => "review",
            (true, "blocking") => "block",
            (true, "needs_fix") => "needs_fix",
            (true, "working") => match cur.status.as_str() {
                "blocking" => "answer",
                "ready" => "claim",
                _ => "edit",
            },
            (true, _) => "edit",
            (false, _) => {
                if p.report.is_some() {
                    "report"
                } else {
                    "note"
                }
            }
        };
        let body = p.note.or(p.report).unwrap_or("");
        let idp = tx.p(id);
        tx.raw(&format!(
            "UPDATE type::thing('tasks', {idp}) SET {};",
            sets.join(", ")
        ));
        tx.event(&idp, actor, kind, body);
        let seq = int_of(&tx.run(&s.db, "$seq").await?, "event seq")?;
        Ok(format!("{} {status} #{seq}", short(id)))
    })
}

fn patch<'a>() -> Patch<'a> {
    Patch {
        status: None,
        report: None,
        note: None,
        commit: None,
        needs: None,
        ttl_s: 0,
    }
}

pub fn note(actor: &str, id: i64, body: &str) -> Result<String, String> {
    apply(
        actor,
        id,
        &Patch {
            note: Some(body),
            ..patch()
        },
    )
}

/// Writes the worker's report without moving the task.
pub fn report(actor: &str, id: i64, body: &str) -> Result<String, String> {
    apply(
        actor,
        id,
        &Patch {
            report: Some(body),
            ..patch()
        },
    )
}

pub fn ask(actor: &str, id: i64, question: &str) -> Result<String, String> {
    apply(
        actor,
        id,
        &Patch {
            status: Some("blocking"),
            note: Some(question),
            needs: Some("input"),
            ..patch()
        },
    )
}

pub fn answer(actor: &str, id: i64, text: &str) -> Result<String, String> {
    // Resume with a fresh lease in the same transaction: no steal window.
    apply(
        actor,
        id,
        &Patch {
            status: Some("working"),
            note: Some(text),
            ttl_s: DEFAULT_TTL_S,
            ..patch()
        },
    )
}

/// Arbitrary status moves for reopen/drop.
pub fn finish(
    actor: &str,
    id: i64,
    status: &str,
    summary: &str,
    commit: Option<&str>,
) -> Result<String, String> {
    apply(
        actor,
        id,
        &Patch {
            status: Some(status),
            report: Some(summary),
            commit,
            ..patch()
        },
    )
}

pub fn block(actor: &str, id: i64, reason: &str) -> Result<String, String> {
    apply(
        actor,
        id,
        &Patch {
            status: Some("blocking"),
            note: Some(reason),
            ..patch()
        },
    )
}

pub fn heartbeat(actor: &str, id: i64, ttl_s: i64) -> Result<String, String> {
    let s = session(true)?;
    block_on(async {
        let lease = now_ms() + ttl_s * 1000;
        // Only the holder may extend: a stale owner must not refresh a lease that
        // another agent reclaimed after expiry.
        let held = fetch_opt(&s.db, id)
            .await?
            .is_some_and(|r| r.status == "working" && r.assignee.as_deref() == Some(actor));
        if !held {
            return Err(format!("{} not working or not held by {actor}", short(id)));
        }
        let mut tx = Txn::new();
        let (idp, l, n) = (tx.p(id), tx.p(lease), tx.p(now_ms()));
        tx.raw(&format!(
            "UPDATE type::thing('tasks', {idp}) SET lease_until = {l}, updated_at = {n};"
        ));
        tx.run(&s.db, "true").await?;
        Ok(format!("ok {} lease {}m", short(id), ttl_s / 60))
    })
}

// ---- reads --------------------------------------------------------------------------

pub fn list(status: Option<&str>, assignee: Option<&str>, limit: usize) -> Result<String, String> {
    let s = session(false)?;
    block_on(async {
        let mut cond = String::new();
        let mut binds = Map::new();
        match status {
            Some(st) => {
                let want = norm(st).ok_or_else(|| format!("bad status {st:?} ({STATUS_LIST})"))?;
                cond.push_str("status = $status");
                binds.insert("status".into(), want.into());
            }
            None => cond.push_str("status NOT IN ['done', 'dropped']"),
        }
        if let Some(a) = assignee {
            cond.push_str(" AND assignee = $assignee");
            binds.insert("assignee".into(), a.into());
        }
        let mut rows = select_rows(&s.db, &format!("WHERE {cond}"), Value::Object(binds)).await?;
        let rank = |st: &str| match st {
            "blocking" => 0,
            "needs_fix" => 1,
            "working" => 2,
            "review" => 3,
            "ready" => 4,
            _ => 5,
        };
        rows.sort_by_key(|r| (rank(&r.status), r.priority, r.tid));
        rows.truncate(limit);
        if rows.is_empty() {
            return Ok("(no tasks)".to_owned());
        }
        let mut out = String::new();
        for r in &rows {
            let who = r.assignee.as_deref().unwrap_or("-");
            let flag = if r.needs.as_deref() == Some("input") {
                " [input]"
            } else {
                ""
            };
            out.push_str(&format!(
                "{} {} {:<9} {:>4} {}{}\n",
                short(r.tid),
                code(&r.status),
                who,
                ago(r.updated_at),
                clip(&r.title, 64),
                flag
            ));
        }
        Ok(out.trim_end().to_owned())
    })
}

/// `full` returns description, goal, report and note unclipped and verbatim.
pub fn get(id: i64, full: bool) -> Result<String, String> {
    let s = session(false)?;
    block_on(async {
        let r = fetch(&s.db, id).await?;
        let mut out = format!(
            "{} {} {} priority {}\n",
            short(r.tid),
            r.status,
            r.assignee.as_deref().unwrap_or("-"),
            r.priority
        );
        out.push_str(&format!("title: {}\n", r.title));
        if let Some(e) = r.epic_id {
            let epic = super::epics::fetch_epic(&s.db, e).await?;
            out.push_str(&format!(
                "epic: {} {}{}\n",
                short_epic(e),
                clip(&epic.title, 60),
                if epic.path.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", epic.path)
                }
            ));
        }
        if let Some(p) = r.parent_id {
            out.push_str(&format!("parent: {}\n", short(p)));
        }
        out.push_str(&format!("created: {}\n", when(r.created_at)));
        if let Some(st) = r.started_at {
            out.push_str(&format!("started: {}\n", when(st)));
        }
        if let Some(c) = r.completed_at {
            out.push_str(&format!("completed: {}\n", when(c)));
        }
        if let Some(u) = r.lease_until {
            let left = u - now_ms();
            if left > 0 {
                out.push_str(&format!("lease: {}m left\n", left / 60_000));
            }
        }
        if let Some(line) = status_summary(&s.db, "parent_id", r.tid, "children").await? {
            out.push_str(&line);
            out.push('\n');
        }
        let t = short(r.tid);
        out.push_str(&field(
            "description",
            &r.description,
            CAP_DESCRIPTION,
            full,
            &t,
        ));
        out.push_str(&field("goal", &r.goal, CAP_GOAL, full, &t));
        out.push_str(&field(
            "report",
            r.report.as_deref().unwrap_or(""),
            CAP_REPORT,
            full,
            &t,
        ));
        out.push_str(&field(
            "note",
            r.note.as_deref().unwrap_or(""),
            CAP_NOTE,
            full,
            &t,
        ));
        if let Some(c) = &r.commit_ref {
            out.push_str(&format!("commit: {c}\n"));
        }
        out.push_str(
            &recent_events(
                &s.db,
                "task_id",
                r.tid,
                full,
                &format!("`agentboard get {t} --full` or task_get id={t} full=true"),
            )
            .await?,
        );
        Ok(out.trim_end().to_owned())
    })
}

/// The last 8 events of a task as an `events:` block, oldest first; bodies are
/// clipped with a marker unless `full`.
pub(super) async fn recent_events(
    db: &Db,
    col: &str,
    id: i64,
    full: bool,
    hint: &str,
) -> Result<String, String> {
    #[derive(Deserialize)]
    struct Ev {
        seq: i64,
        actor: String,
        kind: String,
        body: String,
        ts: i64,
    }
    let mut r = db
        .query(format!("SELECT meta::id(id) AS seq, actor, kind, body, ts FROM events WHERE {col} = $id ORDER BY seq DESC LIMIT 8"))
        .bind(("id", id))
        .await
        .map_err(|e| db_err(&e))?;
    let evs: Vec<Ev> = r.take(0).map_err(|e| db_err(&e))?;
    if evs.is_empty() {
        return Ok(String::new());
    }
    let mut out = String::from("events:\n");
    for e in evs.iter().rev() {
        let text = if full {
            e.body.clone()
        } else {
            clip_marked(&e.body, CAP_EVENT, hint)
        };
        out.push_str(&format!(
            "  #{} {} {} {}: {text}\n",
            e.seq,
            e.actor,
            e.kind,
            ago(e.ts)
        ));
    }
    Ok(out)
}

/// Canonical status order for summaries: most urgent first.
const STATUS_ORDER: [&str; 7] = [
    "blocking",
    "needs_fix",
    "working",
    "review",
    "ready",
    "done",
    "dropped",
];

/// `children: 4 (2 done, 1 working, 1 needs_fix)`, or None when there are none.
pub(super) async fn status_summary(
    db: &Db,
    col: &str,
    id: i64,
    label: &str,
) -> Result<Option<String>, String> {
    #[derive(Deserialize)]
    struct Count {
        status: String,
        n: i64,
    }
    let mut r = db
        .query(format!(
            "SELECT status, count() AS n FROM tasks WHERE {col} = $id GROUP BY status"
        ))
        .bind(("id", id))
        .await
        .map_err(|e| db_err(&e))?;
    let counts: Vec<Count> = r.take(0).map_err(|e| db_err(&e))?;
    let total: i64 = counts.iter().map(|c| c.n).sum();
    if total == 0 {
        return Ok(None);
    }
    let mut parts = Vec::new();
    for st in STATUS_ORDER {
        if let Some(c) = counts.iter().find(|c| c.status == st) {
            parts.push(format!("{} {st}", c.n));
        }
    }
    Ok(Some(format!("{label}: {total} ({})", parts.join(", "))))
}

/// One compact line per task, ordered by id: id, status, assignee, age, title.
/// For a task these are its subtasks (`parent_id`); for an epic, every task in it.
pub fn children(of: Ref) -> Result<String, String> {
    let s = session(false)?;
    block_on(async {
        let (label, col, id) = match of {
            Ref::Task(id) => {
                fetch(&s.db, id).await?;
                (short(id), "parent_id", id)
            }
            Ref::Epic(id) => {
                super::epics::fetch_epic(&s.db, id).await?;
                (short_epic(id), "epic_id", id)
            }
        };
        let rows = select_rows(
            &s.db,
            &format!("WHERE {col} = $id ORDER BY tid"),
            json!({"id": id}),
        )
        .await?;
        if rows.is_empty() {
            return Ok(format!("{label} has no children"));
        }
        let mut out = String::new();
        for r in rows {
            out.push_str(&format!(
                "{} {} {:<9} {:>4} {}\n",
                short(r.tid),
                code(&r.status),
                r.assignee.as_deref().unwrap_or("-"),
                ago(r.updated_at),
                clip(&r.title, 64)
            ));
        }
        Ok(out.trim_end().to_owned())
    })
}
