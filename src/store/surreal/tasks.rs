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
//! Rows are loaded here and normalised to the plain `text::TaskRow` before they are
//! rendered, so the text comes from the same functions as the sqlite build.

use super::{Db, block_on, db_err, session};
use crate::clip::{CAP_DESCRIPTION, CAP_GOAL, field};
use crate::store::board::{ClaimError, NewTask, Patch, TaskEdit};
use crate::store::text::{
    self, EpicRow, EventRow, MAX_CHAIN, Submission, TaskRow, cap_cascade_note, cap_note,
    cap_refusal, cap_skipped_note, cascades_to, chain_refusal, epic_gap, field_names, gate_refusal,
    held_text, norm_branch, normalize_path, reopen_after, review_description, review_filed_note,
    review_goal, review_resubmitted_note, review_title, text_change, value_change, workdir_text,
};
use crate::store::{DEFAULT_TTL_S, Ref, STATUS_LIST, norm, now_ms, short, short_epic};
use serde::Deserialize;
use serde_json::{Map, Value, json};

/// A task row. `tid` is the integer record key (`meta::id(id)`).
#[derive(Deserialize, Clone, Debug)]
pub(super) struct Row {
    pub(super) tid: i64,
    pub(super) status: String,
    pub(super) assignee: Option<String>,
    pub(super) needs: Option<String>,
    pub(super) title: String,
    pub(super) description: String,
    pub(super) goal: String,
    /// Absent on a record written before the column existed: reads as unbound.
    /// (The field is added to an existing board by the idempotent DEFINE in the
    /// schema; no record is rewritten and nothing is refused.)
    #[serde(default)]
    pub(super) path: String,
    /// Absent on such a record: reads as `HEAD`.
    #[serde(default = "head_branch")]
    pub(super) branch: String,
    pub(super) lease_until: Option<i64>,
    pub(super) updated_at: i64,
    pub(super) created_at: i64,
    pub(super) started_at: Option<i64>,
    pub(super) completed_at: Option<i64>,
    pub(super) report: Option<String>,
    pub(super) note: Option<String>,
    pub(super) commit_ref: Option<String>,
    pub(super) priority: i64,
    pub(super) epic_id: Option<i64>,
    pub(super) parent_id: Option<i64>,
    /// Absent on a record written before the field existed: reads as never reopened.
    #[serde(default)]
    pub(super) reopen_count: i64,
}

const SELECT_TASK: &str = "SELECT meta::id(id) AS tid, * FROM tasks";

/// The `branch` a record written before the field existed reads as.
fn head_branch() -> String {
    "HEAD".to_owned()
}

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

pub(super) async fn fetch(db: &Db, id: i64) -> Result<Row, String> {
    fetch_opt(db, id)
        .await?
        .ok_or_else(|| format!("no such task {}", short(id)))
}

pub(super) async fn select_rows(db: &Db, tail: &str, binds: Value) -> Result<Vec<Row>, String> {
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
    pub(super) fn event(&mut self, task: &str, actor: &str, kind: &str, body: &str) {
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
        let guard = format!(
            "LET $version = (SELECT VALUE version FROM meta:schema)[0];
            IF $version != {} {{ THROW 'board schema mismatch: version changed during operation; retry with a compatible binary'; }};\n",
            super::SCHEMA_VERSION
        );
        self.sql.insert_str("BEGIN TRANSACTION;\n".len(), &guard);
        self.raw(&format!("RETURN {ret};\nCOMMIT TRANSACTION;"));
        let mut res = db
            .query(self.sql)
            .bind(Value::Object(self.binds))
            .await
            .map_err(|e| db_err(&e))?;
        if let Some((_, e)) = res.take_errors().into_iter().min_by_key(|(i, e)| {
            // A THROW marks earlier statements as rolled back. Preserve the
            // named refusal rather than the generic transaction-failed wrapper.
            (
                e.to_string()
                    .contains("The query was not executed due to a failed transaction"),
                *i,
            )
        }) {
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

impl From<Row> for TaskRow {
    /// Normalised at the store boundary: the renderer sees plain values.
    fn from(r: Row) -> Self {
        Self {
            id: r.tid,
            status: r.status,
            assignee: r.assignee,
            needs: r.needs,
            title: r.title,
            description: r.description,
            goal: r.goal,
            path: r.path,
            branch: r.branch,
            lease_until: r.lease_until,
            updated_at: r.updated_at,
            created_at: r.created_at,
            started_at: r.started_at,
            completed_at: r.completed_at,
            report: r.report,
            note: r.note,
            commit_ref: r.commit_ref,
            priority: r.priority,
            epic_id: r.epic_id,
            parent_id: r.parent_id,
            reopen_count: r.reopen_count,
        }
    }
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

/// Levels from `id` up to its root, itself included: a parentless task is level 1.
async fn chain_level(db: &Db, id: i64) -> Result<usize, String> {
    let mut level = 1;
    let mut at = fetch(db, id).await?.parent_id;
    while let Some(a) = at {
        if level > 10_000 {
            return Err(format!("cycle above {}", short(id)));
        }
        level += 1;
        at = fetch(db, a).await?.parent_id;
    }
    Ok(level)
}

/// Levels of the subtree rooted at `id`, itself included: a leaf is 1.
async fn subtree_height(db: &Db, id: i64) -> Result<usize, String> {
    let mut height = 0;
    let mut level = vec![id];
    while !level.is_empty() {
        height += 1;
        if height > 10_000 {
            return Err(format!("cycle below {}", short(id)));
        }
        let rows = select_rows(db, "WHERE parent_id IN $ids", json!({ "ids": level })).await?;
        level = rows.into_iter().map(|r| r.tid).collect();
    }
    Ok(height)
}

/// Refuses a subtree `height` levels deep (1 for a new row) under `parent` when its
/// deepest row would pass the last level of the chain.
async fn check_depth(db: &Db, parent: i64, height: usize) -> Result<(), String> {
    let level = chain_level(db, parent).await?;
    if level + height > MAX_CHAIN {
        return Err(chain_refusal(parent, level, height));
    }
    Ok(())
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
        path,
        branch,
    } = *task;
    let cwd = std::env::current_dir().map_err(|e| format!("cwd: {e}"))?;
    let home = std::env::var("HOME").ok();
    let path = normalize_path(path.unwrap_or(""), &cwd, home.as_deref());
    let branch = norm_branch(branch.unwrap_or("")).map_err(|e| format!("add: {e}"))?;
    let s = session(true)?;
    block_on(async {
        let epic = resolve_epic(&s.db, epic, parent).await?;
        if let Some(p) = parent {
            check_depth(&s.db, p, 1)
                .await
                .map_err(|e| super::with_context("add", &e))?;
        }
        // A task created already `working` belongs to its creator unless named.
        let assignee = assignee.or_else(|| (status == "working").then_some(actor));
        let now = now_ms();
        // A `working` task needs a lease, or a dead holder could never be reclaimed.
        let lease = (status == "working").then(|| now + DEFAULT_TTL_S * 1000);
        let started = (status == "working").then_some(now);
        let mut content = json!({
            "title": title, "description": description, "goal": goal,
            "path": path, "branch": branch, "status": status,
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
            .map_err(|e| super::with_context("add", &e))
            .and_then(|v| int_of(&v, "task id"))?;
        Ok(format!("{} {status}", short(id)))
    })
}

/// Amends a task's spec. A task a worker holds (`working` with a live lease) is
/// refused: that worker is reading this text. Logs one `edit` event naming each
/// changed field with its old/new length; nothing is written when no value moved.
pub fn edit(actor: &str, id: i64, e: &TaskEdit<'_>) -> Result<String, ClaimError> {
    let s = session(true)?;
    block_on(async {
        let mut tx = Txn::new();
        let (_, fields) = edit_in_tx(&s.db, &mut tx, actor, id, e).await?;
        let Some(fields) = fields else {
            return Ok(format!("{} unchanged", short(id)));
        };
        let seq = int_of(&tx.run(&s.db, "$seq").await?, "event seq")?;
        Ok(format!("{} edited {fields} #{seq}", short(id)))
    })
}

async fn edit_in_tx(
    db: &Db,
    tx: &mut Txn,
    actor: &str,
    id: i64,
    e: &TaskEdit<'_>,
) -> Result<(Row, Option<String>), ClaimError> {
    if e.title.is_none()
        && e.description.is_none()
        && e.goal.is_none()
        && e.priority.is_none()
        && e.assignee.is_none()
        && e.path.is_none()
        && e.branch.is_none()
        && e.epic.is_none()
    {
        return Err(String::from(
            "nothing to edit: pass title, description, goal, priority, assignee, path, branch or epic",
        )
        .into());
    }
    if e.title.is_some_and(|t| t.trim().is_empty()) {
        return Err(String::from("title cannot be empty").into());
    }
    let mut cur = fetch(db, id).await?;
    if cur.status == "working" && cur.lease_until.is_some_and(|l| l >= now_ms()) {
        return Err(ClaimError::Held(held_text(&TaskRow::from(cur.clone()))));
    }
    let mut diffs = Vec::new();
    let mut family = Vec::new();
    let epic = e.epic.or(cur.epic_id);
    if epic != cur.epic_id {
        let target = super::epics::fetch_epic(db, epic.unwrap()).await?;
        if target.status != "open" {
            return Err(format!(
                "cannot move {} to {}: target epic is {}",
                short(id),
                short_epic(target.tid),
                target.status
            )
            .into());
        }
        if let Some(parent) = cur.parent_id {
            let row = fetch(db, parent).await?;
            if row.epic_id != epic {
                family.push((parent, row.epic_id));
            }
        }
        let mut result = db.query("SELECT *, meta::id(id) AS tid FROM tasks WHERE parent_id=$id AND (epic_id IS NONE OR epic_id != $epic) ORDER BY tid")
            .bind(("id", id)).bind(("epic", target.tid)).await.map_err(|e| db_err(&e))?;
        let children: Vec<Row> = result.take(0).map_err(|e| db_err(&e))?;
        family.extend(children.into_iter().map(|row| (row.tid, row.epic_id)));
        diffs.push(format!(
            "epic {}->{}",
            cur.epic_id.map_or("none".to_owned(), short_epic),
            short_epic(target.tid)
        ));
    }
    let title = value_change(&mut diffs, "title", &cur.title, e.title);
    let description = text_change(&mut diffs, "description", &cur.description, e.description);
    let goal = text_change(&mut diffs, "goal", &cur.goal, e.goal);
    let assignee = value_change(
        &mut diffs,
        "assignee",
        cur.assignee.as_deref().unwrap_or(""),
        e.assignee,
    );
    // A path is normalised the way an epic's is; an empty one clears it. A
    // branch git could not check out is refused before any write.
    let path_value = e.path.map(|raw| {
        let cwd = std::env::current_dir().unwrap_or_default();
        let home = std::env::var("HOME").ok();
        normalize_path(raw, &cwd, home.as_deref())
    });
    let path = value_change(&mut diffs, "path", &cur.path, path_value.as_deref());
    let branch_value = match e.branch {
        Some(raw) => norm_branch(raw).map_err(|err| ClaimError::Other(format!("edit: {err}")))?,
        None => cur.branch.clone(),
    };
    let branch = value_change(
        &mut diffs,
        "branch",
        &cur.branch,
        Some(branch_value.as_str()),
    );
    let priority = e.priority.unwrap_or(cur.priority);
    if priority != cur.priority {
        diffs.push(format!("priority {}->{priority}", cur.priority));
    }
    if diffs.is_empty() {
        return Ok((cur, None));
    }
    let mut sets = Vec::new();
    assign(tx, &mut sets, "title", Some(title.into()));
    assign(tx, &mut sets, "description", Some(description.into()));
    assign(tx, &mut sets, "goal", Some(goal.into()));
    assign(tx, &mut sets, "path", Some(path.into()));
    assign(tx, &mut sets, "branch", Some(branch.into()));
    assign(
        tx,
        &mut sets,
        "assignee",
        (!assignee.is_empty()).then(|| assignee.into()),
    );
    assign(tx, &mut sets, "priority", Some(priority.into()));
    assign(tx, &mut sets, "epic_id", epic.map(Value::from));
    assign(tx, &mut sets, "updated_at", Some(now_ms().into()));
    let idp = tx.p(id);
    tx.raw(&format!(
        "UPDATE type::thing('tasks', {idp}) SET {};",
        sets.join(", ")
    ));
    if let Some(note) = epic_gap(&family) {
        tx.event(&idp, actor, "note", &note);
    }
    tx.event(&idp, actor, "edit", &diffs.join(", "));
    cur.title = title.to_owned();
    cur.description = description.to_owned();
    cur.goal = goal.to_owned();
    cur.path = path.to_owned();
    cur.branch = branch.to_owned();
    cur.assignee = (!assignee.is_empty()).then(|| assignee.to_owned());
    cur.priority = priority;
    cur.epic_id = epic;
    Ok((cur, Some(field_names(&diffs))))
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
            check_depth(&s.db, p, subtree_height(&s.db, id).await?).await?;
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
                            return Err(ClaimError::Held(held_text(&TaskRow::from(r.clone()))));
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
            // Where the work lives, before the prompt: a worker that checks out
            // the wrong tree wastes the whole lease.
            out.push_str(&workdir_text(&TaskRow::from(r.clone())));
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

/// Adds to `tx` the review row for a parentless task that just moved to review, or a
/// note on the one already filed. Returns the review row's id. The write lock is held,
/// so the counter read here cannot move before the transaction runs.
pub(super) async fn file_review(
    db: &Db,
    tx: &mut Txn,
    actor: &str,
    implement: &Row,
    report: Option<&str>,
    submission: &Submission,
) -> Result<i64, String> {
    // A parentless row's only children are its review rows (fix rows hang off those), so
    // the link is the parent alone: renaming the row between rounds must not hide it.
    let title = review_title(&implement.title);
    let existing = select_rows(
        db,
        "WHERE parent_id = $id AND status != 'dropped' ORDER BY tid DESC LIMIT 1",
        json!({"id": implement.tid}),
    )
    .await?
    .into_iter()
    .next();
    if let Some(review) = existing {
        let rp = tx.p(review.tid);
        tx.event(
            &rp,
            actor,
            "note",
            &review_resubmitted_note(implement.tid, submission),
        );
        return Ok(review.tid);
    }
    let mut r = db
        .query("SELECT VALUE n FROM counters:task")
        .await
        .map_err(|e| db_err(&e))?;
    let last: Vec<i64> = r.take(0).map_err(|e| db_err(&e))?;
    let review = last.first().copied().unwrap_or(0) + 1;
    let now = now_ms();
    let as_row = TaskRow::from(implement.clone());
    let report = report.or(implement.report.as_deref());
    let mut content = json!({
        "title": title,
        "description": review_description(&as_row, report, submission),
        "goal": review_goal(implement.tid, submission),
        "path": implement.path, "branch": implement.branch, "status": "ready",
        "priority": implement.priority, "parent_id": implement.tid,
        "created_at": now, "updated_at": now,
    });
    if let Some(e) = implement.epic_id {
        content["epic_id"] = e.into();
    }
    let rp = tx.p(review);
    tx.raw(&format!("UPSERT counters:task SET n = {rp};"));
    let c = tx.p(content);
    tx.raw(&format!("CREATE type::thing('tasks', {rp}) CONTENT {c};"));
    tx.event(&rp, actor, "add", &title);
    let ip = tx.p(implement.tid);
    tx.event(&ip, actor, "note", &review_filed_note(review));
    Ok(review)
}

/// The `seq` of the newest event of `kind` on a task, if any.
pub(super) async fn last_event_seq(db: &Db, id: i64, kind: &str) -> Result<Option<i64>, String> {
    #[derive(Deserialize)]
    struct Seq {
        seq: i64,
    }
    let mut r = db
        .query("SELECT meta::id(id) AS seq FROM events WHERE task_id = $id AND kind = $kind ORDER BY seq DESC LIMIT 1")
        .bind(("id", id))
        .bind(("kind", kind.to_owned()))
        .await
        .map_err(|e| db_err(&e))?;
    let rows: Vec<Seq> = r.take(0).map_err(|e| db_err(&e))?;
    Ok(rows.first().map(|s| s.seq))
}

pub fn apply(actor: &str, id: i64, p: &Patch<'_>) -> Result<String, String> {
    let s = session(true)?;
    block_on(async {
        let mut tx = Txn::new();
        let cur = if let Some(edit) = p.edit {
            edit_in_tx(&s.db, &mut tx, actor, id, edit)
                .await
                .map_err(|e| e.to_string())?
                .0
        } else {
            fetch(&s.db, id).await?
        };
        let now = now_ms();
        let status = match p.status {
            Some(raw) => norm(raw).ok_or_else(|| format!("bad status {raw:?} ({STATUS_LIST})"))?,
            None => cur.status.as_str(),
        };
        let moved = status != cur.status;
        // A move into review needs a submission recorded since the last claim (a diff,
        // or evidence for a re-run round): refused before anything is written, so a
        // reviewer never gets an empty review.
        let submit = moved && status == "review";
        let submission = if submit {
            super::diffs::submission_since_claim(&s.db, id).await?
        } else {
            None
        };
        if submit && submission.is_none() {
            return Err(gate_refusal(id));
        }
        // A move into needs_fix is a reopen; the one over the cap lands not_done instead.
        let reopen = moved && status == "needs_fix";
        let capped = reopen && reopen_after(cur.reopen_count).1;
        let status = if capped { "not_done" } else { status };
        let mut sets: Vec<String> = Vec::new();
        if reopen {
            sets.push("reopen_count = (reopen_count ?? 0) + 1".to_owned());
        }
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
                _ => "claim",
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
        if capped {
            tx.event(&idp, actor, kind, &cap_note(body));
            // One level up only: the grandparent is the manager's to look at.
            if let Some(parent) = cur.parent_id {
                let above = fetch(&s.db, parent).await?;
                if cascades_to(&above.status) {
                    let pp = tx.p(parent);
                    let np = tx.p(now);
                    tx.raw(&format!(
                        "UPDATE type::thing('tasks', {pp}) SET status = 'not_done', needs = NONE, lease_until = NONE, updated_at = {np};"
                    ));
                    tx.event(&pp, actor, "edit", &cap_cascade_note(id));
                } else {
                    tx.event(
                        &idp,
                        actor,
                        "note",
                        &cap_skipped_note(parent, &above.status),
                    );
                }
            }
            tx.run(&s.db, "$seq").await?;
            return Err(cap_refusal(id));
        }
        tx.event(&idp, actor, kind, body);
        tx.raw("LET $main = $seq;");
        // Only a parentless row files a review row: a fix row or a review row never does.
        let mut review = None;
        if submit && cur.parent_id.is_none() {
            // The gate refused above; keeping the None case means deleting the gate
            // still lets the status move, which the mutation proof checks.
            if let Some(submission) = submission.as_ref() {
                review =
                    Some(file_review(&s.db, &mut tx, actor, &cur, p.report, submission).await?);
            }
        }
        let seq = int_of(&tx.run(&s.db, "$main").await?, "event seq")?;
        let filed = review.map_or(String::new(), |r| format!(" (review row {})", short(r)));
        Ok(format!("{} {status} #{seq}{filed}", short(id)))
    })
}

fn patch<'a>() -> Patch<'a> {
    Patch {
        edit: None,
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
            edit: None,
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
            edit: None,
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
            edit: None,
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
            edit: None,
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
            edit: None,
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
            edit: None,
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
        rows.sort_by_key(|r| (text::list_rank(&r.status), r.priority, r.tid));
        rows.truncate(limit);
        let rows: Vec<TaskRow> = rows.into_iter().map(TaskRow::from).collect();
        Ok(text::list_text(&rows))
    })
}

/// `full` returns description, goal, report and note unclipped and verbatim.
pub fn get(id: i64, full: bool) -> Result<String, String> {
    let s = session(false)?;
    block_on(async {
        let r = fetch(&s.db, id).await?;
        let epic = match r.epic_id {
            Some(e) => Some(super::epics::fetch_epic(&s.db, e).await?),
            None => None,
        };
        let summary = status_summary(&s.db, "parent_id", r.tid, "children").await?;
        let events = recent_events(&s.db, "task_id", r.tid, full, &text::task_hint(r.tid)).await?;
        let newest = super::diffs::newest_diff(&s.db, r.tid).await?;
        Ok(text::get_text(
            &TaskRow::from(r),
            epic.map(EpicRow::from).as_ref(),
            summary.as_deref(),
            newest.as_ref(),
            &events,
            full,
        ))
    })
}

/// The last 8 events of a task or an epic (`col` is `task_id` or `epic_id`) as an
/// `events:` block, oldest first; bodies are clipped with a marker unless `full`.
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
    let evs: Vec<EventRow> = evs
        .into_iter()
        .map(|e| EventRow {
            seq: e.seq,
            actor: e.actor,
            kind: e.kind,
            body: e.body,
            ts: e.ts,
        })
        .collect();
    Ok(text::events_text(&evs, full, hint))
}

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
    let counts: Vec<(String, i64)> = counts.into_iter().map(|c| (c.status, c.n)).collect();
    Ok(text::summary_text(label, &counts))
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
        let rows: Vec<TaskRow> = rows.into_iter().map(TaskRow::from).collect();
        Ok(text::children_text(&label, &rows))
    })
}
