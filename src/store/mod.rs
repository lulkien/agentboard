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
        conn.execute_batch("PRAGMA journal_mode=WAL")
            .map_err(|e| e.to_string())?;
        conn.execute_batch(SCHEMA)
            .map_err(|e| format!("schema: {e}"))?;
        migrate(&conn)?;
        *done = true;
    }
    drop(done);
    Ok(conn)
}

/// Bring a v1 database (title/spec/status/result/created/updated) up to v2
/// (description/goal/report/note + created_at/started_at/completed_at).
///
/// The version check runs inside the write transaction, so two processes
/// opening a v1 file at once cannot both try to ALTER it. A failed batch leaves
/// the transaction open; dropping the connection rolls it back (DDL is
/// transactional in SQLite), so no partial schema survives.
fn migrate(conn: &Connection) -> Result<(), String> {
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| format!("migrate v1->v2: {e}"))?;
    let cols: Vec<String> = {
        let mut stmt = conn
            .prepare("PRAGMA table_info(tasks)")
            .map_err(|e| e.to_string())?;
        stmt.query_map([], |r| r.get::<_, String>(1))
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect()
    };
    if cols.iter().any(|c| c == "description") {
        return conn
            .execute_batch("COMMIT")
            .map_err(|e| format!("migrate v1->v2: {e}"));
    }
    conn.execute_batch(
        "ALTER TABLE tasks ADD COLUMN description TEXT NOT NULL DEFAULT '';
         ALTER TABLE tasks ADD COLUMN goal TEXT NOT NULL DEFAULT '';
         ALTER TABLE tasks ADD COLUMN report TEXT;
         ALTER TABLE tasks ADD COLUMN note TEXT;
         ALTER TABLE tasks ADD COLUMN created_at INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE tasks ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE tasks ADD COLUMN started_at INTEGER;
         ALTER TABLE tasks ADD COLUMN completed_at INTEGER;
         UPDATE tasks SET description=spec, goal=spec, report=result,
                          created_at=created, updated_at=updated;
         UPDATE tasks SET status=CASE status WHEN 'running' THEN 'working'
                              WHEN 'blocked' THEN 'blocking'
                              WHEN 'triage' THEN 'ready' ELSE status END;
         ALTER TABLE tasks DROP COLUMN spec;
         ALTER TABLE tasks DROP COLUMN result;
         ALTER TABLE tasks DROP COLUMN created;
         ALTER TABLE tasks DROP COLUMN updated;
         COMMIT;",
    )
    .map_err(|e| format!("migrate v1->v2: {e}"))
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

pub fn parse_id(raw: &str) -> Result<i64, String> {
    let t = raw.trim().trim_start_matches(['T', 't', '#']);
    t.parse::<i64>()
        .map_err(|_| format!("bad task id {raw:?} (want T3 or 3)"))
}

fn log_event(
    conn: &Connection,
    task: i64,
    actor: &str,
    kind: &str,
    body: &str,
) -> Result<i64, String> {
    conn.execute(
        "INSERT INTO events(task_id,actor,kind,body,ts) VALUES(?,?,?,?,?)",
        params![task, actor, kind, body, now_ms()],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
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
    parent: Option<i64>,
}

const COLS: &str = "id,status,assignee,needs,title,description,goal,lease_until,updated_at,created_at,started_at,completed_at,report,note,commit_ref,priority,parent";

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
        parent: r.get(16)?,
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
mod proc;
mod render;
mod sessions;
mod tasks;
mod wait;

pub use dispatch::dispatch;
pub use render::{brief, children, get, list};
pub use sessions::{register_session, send, sessions, tail};
pub use tasks::{
    ClaimError, NewTask, Patch, add, answer, apply, ask, block, claim, finish, heartbeat, note,
    report,
};
pub use wait::{Wait, wait};
