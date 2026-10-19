//! Storage + operations. One SQLite file, WAL, short-immediate transactions.
//! Every operation opens its own connection: cheap, and safe under concurrency.

use rusqlite::{Connection, OptionalExtension, params};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const SCHEMA: &str = include_str!("../schema.sql");
pub const DEFAULT_TTL_S: i64 = 1800;

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(0))
}

static DB_OVERRIDE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
static ACTOR_OVERRIDE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Process-wide `--db` / `--actor` from the CLI. Kept in statics instead of
/// `std::env::set_var`, which is unsound once the tokio runtime has threads.
pub fn set_overrides(db: Option<String>, actor: Option<String>) {
    if let Some(d) = db {
        let _ = DB_OVERRIDE.set(PathBuf::from(d));
    }
    if let Some(a) = actor {
        let _ = ACTOR_OVERRIDE.set(a);
    }
}

/// Actor name: `--actor`, else `$AGENTBOARD_ACTOR`, else "agent".
pub fn default_actor() -> String {
    ACTOR_OVERRIDE
        .get()
        .cloned()
        .or_else(|| std::env::var("AGENTBOARD_ACTOR").ok())
        .unwrap_or_else(|| "agent".to_owned())
}

pub fn db_path() -> PathBuf {
    if let Some(p) = DB_OVERRIDE.get() {
        return p.clone();
    }
    if let Ok(p) = std::env::var("AGENTBOARD_DB") {
        return PathBuf::from(p);
    }
    let base = std::env::var("XDG_STATE_HOME").map_or_else(
        |_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".local/state")
        },
        PathBuf::from,
    );
    base.join("agentboard/board.db")
}

fn open() -> Result<Connection, String> {
    let path = db_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("db dir {}: {e}", dir.display()))?;
    }
    let conn = Connection::open(&path).map_err(|e| format!("open {}: {e}", path.display()))?;
    conn.busy_timeout(Duration::from_millis(5000))
        .map_err(|e| e.to_string())?;
    // Per-connection and cheap. Never take a write lock here: `journal_mode=WAL`
    // and DDL on every open is what turns concurrent tool calls into
    // "database is locked".
    conn.execute_batch("PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;")
        .map_err(|e| e.to_string())?;
    // A failed init is retried on the next open instead of being cached for the
    // life of a long-running MCP server.
    let mut done = INIT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !*done {
        // First: a refused file is left exactly as it was (no WAL switch, no DDL).
        check_schema(&conn)?;
        conn.execute_batch("PRAGMA journal_mode=WAL")
            .map_err(|e| e.to_string())?;
        conn.execute_batch(SCHEMA)
            .map_err(|e| format!("schema: {e}"))?;
        *done = true;
    }
    drop(done);
    Ok(conn)
}

/// Refuses a board file written before epics existed. There is no migration by
/// design (history is expendable): half-migrating a live board is worse than
/// asking the operator to move the file aside.
fn check_schema(conn: &Connection) -> Result<(), String> {
    let cols = |table: &str| -> Result<Vec<String>, String> {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(|e| e.to_string())?;
        stmt.query_map([], |r| r.get::<_, String>(1))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())
    };
    let tasks = cols("tasks")?;
    if tasks.is_empty() {
        return Ok(()); // fresh file: the schema creates everything
    }
    let old = tasks.iter().any(|c| c == "parent")
        || !tasks.iter().any(|c| c == "epic_id")
        || cols("epics")?.is_empty()
        || !cols("events")?.iter().any(|c| c == "epic_id");
    if old {
        let p = db_path();
        return Err(format!(
            "this board file predates epics and cannot be used; move it aside and start fresh (there is no migration): mv {0} {0}.old",
            p.display()
        ));
    }
    Ok(())
}

/// Canonical status names, for error messages.
const STATUS_LIST: &str = "ready|working|blocking|needs_fix|review|done|dropped";

/// Canonical statuses. Old names stay accepted as aliases so in-flight agents
/// that still say running/blocked keep working.
pub fn norm(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "triage" | "ready" => Some("ready"),
        "running" | "working" | "started" => Some("working"),
        "blocked" | "blocking" => Some("blocking"),
        "needs_fix" | "needs-fix" | "need fix" | "needs fix" | "need-fix" | "need_fix" | "fix" => {
            Some("needs_fix")
        }
        "review" => Some("review"),
        "done" => Some("done"),
        "dropped" => Some("dropped"),
        _ => None,
    }
}

/// Schema + WAL applied once per process; later opens are pure connection setup.
static INIT: std::sync::Mutex<bool> = std::sync::Mutex::new(false);

pub fn short(id: i64) -> String {
    format!("T{id}")
}

pub fn short_epic(id: i64) -> String {
    format!("E{id}")
}

/// A task id: `T3`, `t3`, `#3` or `3`. An epic id is refused with a pointer to the
/// right verb, so a mixed-up id never silently hits the wrong table.
pub fn parse_id(raw: &str) -> Result<i64, String> {
    let t = raw.trim();
    if t.starts_with(['E', 'e']) {
        return Err(format!(
            "{t} is an epic id; this takes a task id (T3 or 3). Epics: `agentboard epic get {t}` / `children {t}`"
        ));
    }
    t.trim_start_matches(['T', 't', '#'])
        .parse::<i64>()
        .map_err(|_| format!("bad task id {raw:?} (want T3 or 3)"))
}

/// An epic id: `E3` or `e3`. The prefix is required (a bare number is a task).
pub fn parse_epic_id(raw: &str) -> Result<i64, String> {
    let t = raw.trim();
    if t.starts_with(['T', 't', '#']) {
        return Err(format!(
            "{t} is a task id; an epic id looks like E3 (`agentboard epic ls` lists them)"
        ));
    }
    t.strip_prefix(['E', 'e'])
        .and_then(|n| n.parse::<i64>().ok())
        .ok_or_else(|| format!("bad epic id {raw:?} (want E3)"))
}

/// Either kind of id, for the few verbs that take both (`children`).
pub enum Ref {
    Task(i64),
    Epic(i64),
}

pub fn parse_ref(raw: &str) -> Result<Ref, String> {
    if raw.trim().starts_with(['E', 'e']) {
        parse_epic_id(raw).map(Ref::Epic)
    } else {
        parse_id(raw).map(Ref::Task)
    }
}

/// What an event is about. One timeline serves both; the schema CHECK makes
/// exactly one of the two columns set.
#[derive(Clone, Copy)]
enum Subject {
    Task(i64),
    Epic(i64),
}

fn log_event(
    conn: &Connection,
    on: Subject,
    actor: &str,
    kind: &str,
    body: &str,
) -> Result<i64, String> {
    let (task, epic) = match on {
        Subject::Task(id) => (Some(id), None),
        Subject::Epic(id) => (None, Some(id)),
    };
    conn.execute(
        "INSERT INTO events(task_id,epic_id,actor,kind,body,ts) VALUES(?,?,?,?,?,?)",
        params![task, epic, actor, kind, body, now_ms()],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

/// The value to store for a text field an edit may set, noting `name old->new`
/// (character lengths) in `diffs` when it moves.
fn text_change<'a>(
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

/// Like `text_change` for the short fields (title, assignee, path): the event
/// records `name "old"->"new"`, each clipped to 80 chars, so a same-length
/// change is visible and the old text is recoverable.
fn value_change<'a>(
    diffs: &mut Vec<String>,
    name: &str,
    old: &'a str,
    new: Option<&'a str>,
) -> &'a str {
    let value = new.unwrap_or(old);
    if value != old {
        diffs.push(format!(
            "{name} \"{}\"->\"{}\"",
            crate::clip::clip(old, 80),
            crate::clip::clip(value, 80)
        ));
    }
    value
}

/// `title, goal` out of `["title 12->15", "goal 3->9"]`.
fn field_names(diffs: &[String]) -> String {
    let names: Vec<&str> = diffs
        .iter()
        .filter_map(|d| d.split_whitespace().next())
        .collect();
    names.join(", ")
}

struct Row {
    id: i64,
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

const COLS: &str = "id,status,assignee,needs,title,description,goal,lease_until,updated_at,created_at,started_at,completed_at,report,note,commit_ref,priority,epic_id,parent_id";

fn read_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok(Row {
        id: r.get(0)?,
        status: r.get(1)?,
        assignee: r.get(2)?,
        needs: r.get(3)?,
        title: r.get(4)?,
        description: r.get(5)?,
        goal: r.get(6)?,
        lease_until: r.get(7)?,
        updated_at: r.get(8)?,
        created_at: r.get(9)?,
        started_at: r.get(10)?,
        completed_at: r.get(11)?,
        report: r.get(12)?,
        note: r.get(13)?,
        commit_ref: r.get(14)?,
        priority: r.get(15)?,
        epic_id: r.get(16)?,
        parent_id: r.get(17)?,
    })
}

fn fetch(conn: &Connection, id: i64) -> Result<Row, String> {
    let sql = format!("SELECT {COLS} FROM tasks WHERE id=?");
    conn.query_row(&sql, params![id], read_row)
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no such task {}", short(id)))
}

mod dispatch;
mod epics;
mod proc;
mod render;
mod sessions;
mod tasks;
mod wait;

pub use dispatch::dispatch;
pub use epics::{EpicEdit, epic_add, epic_edit, epic_get, epic_list, epic_status};
pub use render::{brief, children, get, list};
pub use sessions::{prune, register_session, send, sessions, tail, unregister};
pub use tasks::{
    ClaimError, NewTask, Patch, TaskEdit, add, answer, apply, ask, block, claim, edit, finish,
    heartbeat, note, report, set_parent,
};
pub use wait::{Wait, wait};

#[cfg(test)]
mod tests {
    use super::{Connection, SCHEMA, check_schema};

    #[test]
    fn fresh_and_current_files_pass_the_schema_check() {
        let c = Connection::open_in_memory().unwrap();
        check_schema(&c).unwrap();
        c.execute_batch(SCHEMA).unwrap();
        check_schema(&c).unwrap();
    }

    #[test]
    fn a_pre_epics_board_is_refused_untouched() {
        for ddl in [
            // the old shape: tasks.parent, no epics, events without epic_id
            "CREATE TABLE tasks(id INTEGER PRIMARY KEY, title TEXT, parent INTEGER);
             CREATE TABLE events(seq INTEGER PRIMARY KEY, task_id INTEGER);",
            // tasks already migrated but no epics table
            "CREATE TABLE tasks(id INTEGER PRIMARY KEY, epic_id INTEGER, parent_id INTEGER);
             CREATE TABLE events(seq INTEGER PRIMARY KEY, task_id INTEGER, epic_id INTEGER);",
        ] {
            let c = Connection::open_in_memory().unwrap();
            c.execute_batch(ddl).unwrap();
            let err = check_schema(&c).unwrap_err();
            assert!(
                err.contains("predates epics") && err.contains("mv "),
                "{err}"
            );
            let tables: i64 = c
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(tables <= 2, "the check must not create anything: {tables}");
        }
    }
}
