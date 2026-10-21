//! The SQLite backend: today's rusqlite code, unchanged, behind the `sqlite` feature.
//! One SQLite file, WAL, short-immediate transactions. Every operation opens its
//! own connection: cheap, and safe under concurrency.

use super::board::{Board, ClaimError, EpicEdit, NewTask, Outcome, Patch, TaskEdit, Wait};
use super::bridge;
use super::{DEFAULT_TTL_S, Ref, STATUS_LIST, db_path, norm, now_ms, short, short_epic};
use rusqlite::{Connection, OptionalExtension, params};
use std::time::Duration;

pub const SCHEMA: &str = include_str!("../../schema.sql");

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
    // A board written before tasks carried a repo path and a branch is refused
    // the same way: the columns cannot be added to an existing file, and a
    // half-migrated board is worse than a fresh one.
    if !(tasks.iter().any(|c| c == "path") && tasks.iter().any(|c| c == "branch")) {
        let p = db_path();
        return Err(format!(
            "this board file predates task path/branch and cannot be used; move it aside and start fresh (there is no migration): mv {0} {0}.old",
            p.display()
        ));
    }
    Ok(())
}

/// Schema + WAL applied once per process; later opens are pure connection setup.
static INIT: std::sync::Mutex<bool> = std::sync::Mutex::new(false);

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

pub(super) use super::text::{TaskRow as Row, field_names, text_change, value_change};

const COLS: &str = "id,status,assignee,needs,title,description,goal,path,branch,lease_until,updated_at,created_at,started_at,completed_at,report,note,commit_ref,priority,epic_id,parent_id";

fn read_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok(Row {
        id: r.get(0)?,
        status: r.get(1)?,
        assignee: r.get(2)?,
        needs: r.get(3)?,
        title: r.get(4)?,
        description: r.get(5)?,
        goal: r.get(6)?,
        path: r.get(7)?,
        branch: r.get(8)?,
        lease_until: r.get(9)?,
        updated_at: r.get(10)?,
        created_at: r.get(11)?,
        started_at: r.get(12)?,
        completed_at: r.get(13)?,
        report: r.get(14)?,
        note: r.get(15)?,
        commit_ref: r.get(16)?,
        priority: r.get(17)?,
        epic_id: r.get(18)?,
        parent_id: r.get(19)?,
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
mod render;
mod sessions;
mod tasks;
mod wait;

/// The SQLite implementation of the board seam: each method calls the module
/// function of the same name, so the SQL is exactly what it was before the split.
pub struct SqliteBoard;

impl Board for SqliteBoard {
    fn add(&self, actor: &str, task: &NewTask<'_>) -> Result<String, String> {
        tasks::add(actor, task)
    }
    fn edit(&self, actor: &str, id: i64, e: &TaskEdit<'_>) -> Result<String, ClaimError> {
        tasks::edit(actor, id, e)
    }
    fn set_parent(&self, actor: &str, id: i64, parent: Option<i64>) -> Result<String, String> {
        tasks::set_parent(actor, id, parent)
    }
    fn claim(&self, actor: &str, id: Option<i64>, ttl_s: i64) -> Result<String, ClaimError> {
        tasks::claim(actor, id, ttl_s)
    }
    fn apply(&self, actor: &str, id: i64, p: &Patch<'_>) -> Result<String, String> {
        tasks::apply(actor, id, p)
    }
    fn note(&self, actor: &str, id: i64, body: &str) -> Result<String, String> {
        tasks::note(actor, id, body)
    }
    fn report(&self, actor: &str, id: i64, body: &str) -> Result<String, String> {
        tasks::report(actor, id, body)
    }
    fn ask(&self, actor: &str, id: i64, question: &str) -> Result<String, String> {
        tasks::ask(actor, id, question)
    }
    fn answer(&self, actor: &str, id: i64, text: &str) -> Result<String, String> {
        tasks::answer(actor, id, text)
    }
    fn finish(
        &self,
        actor: &str,
        id: i64,
        status: &str,
        summary: &str,
        commit: Option<&str>,
    ) -> Result<String, String> {
        tasks::finish(actor, id, status, summary, commit)
    }
    fn block(&self, actor: &str, id: i64, reason: &str) -> Result<String, String> {
        tasks::block(actor, id, reason)
    }
    fn heartbeat(&self, actor: &str, id: i64, ttl_s: i64) -> Result<String, String> {
        tasks::heartbeat(actor, id, ttl_s)
    }

    fn list(
        &self,
        status: Option<&str>,
        assignee: Option<&str>,
        limit: usize,
    ) -> Result<String, String> {
        render::list(status, assignee, limit)
    }
    fn get(&self, id: i64, full: bool) -> Result<String, String> {
        render::get(id, full)
    }
    fn children(&self, of: Ref) -> Result<String, String> {
        render::children(of)
    }
    fn brief(&self, since_min: i64) -> Result<String, String> {
        render::brief(since_min)
    }

    fn epic_add(
        &self,
        actor: &str,
        title: &str,
        description: &str,
        goal: &str,
        path: Option<&str>,
    ) -> Result<String, String> {
        epics::epic_add(actor, title, description, goal, path)
    }
    fn epic_list(&self, status: Option<&str>) -> Result<String, String> {
        epics::epic_list(status)
    }
    fn epic_get(&self, id: i64, full: bool) -> Result<String, String> {
        epics::epic_get(id, full)
    }
    fn epic_status(
        &self,
        actor: &str,
        id: i64,
        status: &str,
        note: Option<&str>,
    ) -> Result<String, String> {
        epics::epic_status(actor, id, status, note)
    }
    fn epic_edit(&self, actor: &str, id: i64, e: &EpicEdit<'_>) -> Result<String, String> {
        epics::epic_edit(actor, id, e)
    }

    fn sessions(&self) -> Result<String, String> {
        sessions::sessions()
    }
    fn prune(&self, dry_run: bool) -> Result<String, String> {
        sessions::prune(dry_run)
    }
    fn unregister(&self, name: &str) -> Result<String, String> {
        sessions::unregister(name)
    }
    fn register_session(
        &self,
        name: &str,
        agent: &str,
        cwd: &str,
        task: Option<i64>,
    ) -> Result<String, String> {
        sessions::register_session(name, agent, cwd, task)
    }
    fn tail(&self, name: &str, lines: usize) -> Result<String, String> {
        bridge::tail(name, lines)
    }
    fn send(&self, name: &str, text: &str) -> Result<String, String> {
        bridge::send(name, text)
    }

    fn dispatch(
        &self,
        actor: &str,
        id: i64,
        to: Option<&str>,
        force: bool,
        print_only: bool,
    ) -> Result<String, String> {
        dispatch::dispatch(actor, id, to, force, print_only)
    }
    async fn wait(&self, w: Wait<'_>) -> Result<Outcome, String> {
        wait::wait(w).await
    }
}

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
    fn a_board_without_task_path_and_branch_is_refused_untouched() {
        // Epics-era shape, but before tasks carried a repo path and a branch.
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE tasks(id INTEGER PRIMARY KEY, epic_id INTEGER, parent_id INTEGER);
             CREATE TABLE epics(id INTEGER PRIMARY KEY);
             CREATE TABLE events(seq INTEGER PRIMARY KEY, task_id INTEGER, epic_id INTEGER);",
        )
        .unwrap();
        let err = check_schema(&c).unwrap_err();
        assert!(
            err.contains("predates task path/branch") && err.contains("mv "),
            "{err}"
        );
        let tables: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 3, "the check must not create anything");
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
