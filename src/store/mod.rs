//! Storage + operations. A build has exactly one backend, chosen by Cargo feature:
//! `sqlite` (default) or `surreal`. `board::Board` is the seam both implement; the
//! free functions here forward to the compiled one, so `main.rs` and `server.rs`
//! do not know which it is.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

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

/// Where the board lives for this build, resolved exactly as the store resolves it:
/// the SQLite board file, or the data directory the local SurrealDB server owns.
/// `--db`, then `$AGENTBOARD_DB`, then the state dir. (The sqlite function, below, is
/// also the answer for the unsupported feature sets: `main.rs` has already stopped
/// those with a `compile_error!`.)
#[cfg(all(feature = "surreal", not(feature = "sqlite")))]
pub fn board_path() -> Result<PathBuf, String> {
    surreal::Settings::from_env().map(|s| s.dir)
}

#[cfg(not(all(feature = "surreal", not(feature = "sqlite"))))]
pub fn board_path() -> Result<PathBuf, String> {
    Ok(db_path())
}

#[cfg(not(all(feature = "surreal", not(feature = "sqlite"))))]
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
        "not_done" | "not-done" | "not done" | "nd" => Some("not_done"),
        "review" => Some("review"),
        "done" => Some("done"),
        "dropped" => Some("dropped"),
        _ => None,
    }
}

/// Canonical task status names, for error messages.
pub const STATUS_LIST: &str = "ready|working|blocking|not_done|needs_fix|review|done|dropped";

/// Canonical epic status names, for error messages.
pub const EPIC_STATUS_LIST: &str = "open|done|dropped";

/// An epic's status: `open`, `done` or `dropped`.
pub fn norm_status(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "open" => Some("open"),
        "done" => Some("done"),
        "dropped" => Some("dropped"),
        _ => None,
    }
}

pub fn short(id: i64) -> String {
    format!("T{id}")
}

pub fn short_epic(id: i64) -> String {
    format!("E{id}")
}

pub fn short_diff(id: i64) -> String {
    format!("D{id}")
}

/// A diff id: `D3` or `d3`. A task or epic id is refused with a pointer to the right
/// verb, so a mixed-up id never silently hits the wrong table.
pub fn parse_diff_id(raw: &str) -> Result<i64, String> {
    let t = raw.trim();
    if t.starts_with(['T', 't', '#', 'E', 'e']) {
        return Err(format!(
            "{t} is not a diff id; this takes D3 (`agentboard diff ls {t}` lists a task's diffs)"
        ));
    }
    t.strip_prefix(['D', 'd'])
        .and_then(|n| n.parse::<i64>().ok())
        .ok_or_else(|| format!("bad diff id {raw:?} (want D3)"))
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

pub mod board;
pub mod bridge;
mod proc;
pub mod text;
pub mod waiting;

pub use board::{Board, ClaimError, EpicEdit, NewTask, Outcome, Patch, TaskEdit, Wait};

// Exactly one backend is supported. With both or neither feature, `main.rs` raises
// a `compile_error!`; `stub::StubBoard` stands in for `BACKEND` there so that is
// the only error rustc prints.
#[cfg(all(feature = "sqlite", not(feature = "surreal")))]
mod sqlite;
#[cfg(not(any(
    all(feature = "sqlite", not(feature = "surreal")),
    all(feature = "surreal", not(feature = "sqlite"))
)))]
mod stub;
#[cfg(all(feature = "surreal", not(feature = "sqlite")))]
mod surreal;

/// The one backend this build compiled in.
#[cfg(all(feature = "sqlite", not(feature = "surreal")))]
const BACKEND: sqlite::SqliteBoard = sqlite::SqliteBoard;
#[cfg(all(feature = "surreal", not(feature = "sqlite")))]
const BACKEND: surreal::SurrealBoard = surreal::SurrealBoard;
#[cfg(not(any(
    all(feature = "sqlite", not(feature = "surreal")),
    all(feature = "surreal", not(feature = "sqlite"))
)))]
const BACKEND: stub::StubBoard = stub::StubBoard;

pub fn add(actor: &str, task: &NewTask<'_>) -> Result<String, String> {
    BACKEND.add(actor, task)
}
pub fn edit(actor: &str, id: i64, e: &TaskEdit<'_>) -> Result<String, ClaimError> {
    BACKEND.edit(actor, id, e)
}
pub fn set_parent(actor: &str, id: i64, parent: Option<i64>) -> Result<String, String> {
    BACKEND.set_parent(actor, id, parent)
}
pub fn claim(actor: &str, id: Option<i64>, ttl_s: i64) -> Result<String, ClaimError> {
    BACKEND.claim(actor, id, ttl_s)
}
pub fn apply(actor: &str, id: i64, p: &Patch<'_>) -> Result<String, String> {
    BACKEND.apply(actor, id, p)
}
pub fn note(actor: &str, id: i64, body: &str) -> Result<String, String> {
    BACKEND.note(actor, id, body)
}
pub fn report(actor: &str, id: i64, body: &str) -> Result<String, String> {
    BACKEND.report(actor, id, body)
}
pub fn ask(actor: &str, id: i64, question: &str) -> Result<String, String> {
    BACKEND.ask(actor, id, question)
}
pub fn answer(actor: &str, id: i64, text: &str) -> Result<String, String> {
    BACKEND.answer(actor, id, text)
}
pub fn finish(
    actor: &str,
    id: i64,
    status: &str,
    summary: &str,
    commit: Option<&str>,
) -> Result<String, String> {
    BACKEND.finish(actor, id, status, summary, commit)
}
pub fn block(actor: &str, id: i64, reason: &str) -> Result<String, String> {
    BACKEND.block(actor, id, reason)
}
pub fn heartbeat(actor: &str, id: i64, ttl_s: i64) -> Result<String, String> {
    BACKEND.heartbeat(actor, id, ttl_s)
}
pub fn list(status: Option<&str>, assignee: Option<&str>, limit: usize) -> Result<String, String> {
    BACKEND.list(status, assignee, limit)
}
pub fn get(id: i64, full: bool) -> Result<String, String> {
    BACKEND.get(id, full)
}
pub fn children(of: Ref) -> Result<String, String> {
    BACKEND.children(of)
}
pub fn brief(since_min: i64) -> Result<String, String> {
    BACKEND.brief(since_min)
}
pub fn diff_add(actor: &str, task: i64, body: &str) -> Result<String, String> {
    BACKEND.diff_add(actor, task, body)
}
pub fn diff_get(id: i64, full: bool) -> Result<String, String> {
    BACKEND.diff_get(id, full)
}
pub fn diff_list(of: Ref) -> Result<String, String> {
    BACKEND.diff_list(of)
}
pub fn epic_add(
    actor: &str,
    title: &str,
    description: &str,
    goal: &str,
    path: Option<&str>,
) -> Result<String, String> {
    BACKEND.epic_add(actor, title, description, goal, path)
}
pub fn epic_list(status: Option<&str>) -> Result<String, String> {
    BACKEND.epic_list(status)
}
pub fn epic_get(id: i64, full: bool) -> Result<String, String> {
    BACKEND.epic_get(id, full)
}
pub fn epic_status(
    actor: &str,
    id: i64,
    status: &str,
    note: Option<&str>,
) -> Result<String, String> {
    BACKEND.epic_status(actor, id, status, note)
}
pub fn epic_edit(actor: &str, id: i64, e: &EpicEdit<'_>) -> Result<String, String> {
    BACKEND.epic_edit(actor, id, e)
}
pub fn sessions() -> Result<String, String> {
    BACKEND.sessions()
}
pub fn session_rows() -> Result<Vec<text::SessionRow>, String> {
    BACKEND.session_rows()
}
pub fn prune(dry_run: bool) -> Result<String, String> {
    BACKEND.prune(dry_run)
}
pub fn unregister(name: &str) -> Result<String, String> {
    BACKEND.unregister(name)
}
pub fn register_session(
    name: &str,
    agent: &str,
    cwd: &str,
    task: Option<i64>,
) -> Result<String, String> {
    BACKEND.register_session(name, agent, cwd, task)
}
pub fn tail(name: &str, lines: usize) -> Result<String, String> {
    BACKEND.tail(name, lines)
}
pub fn send(name: &str, text: &str) -> Result<String, String> {
    BACKEND.send(name, text)
}
pub fn dispatch(
    actor: &str,
    id: i64,
    to: Option<&str>,
    force: bool,
    print_only: bool,
) -> Result<String, String> {
    BACKEND.dispatch(actor, id, to, force, print_only)
}
pub async fn wait(w: Wait<'_>) -> Result<Outcome, String> {
    BACKEND.wait(w).await
}
