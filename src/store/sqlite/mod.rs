//! The SQLite backend: today's rusqlite code, unchanged, behind the `sqlite` feature.
//! One SQLite file, WAL, short-immediate transactions. Every operation opens its
//! own connection: cheap, and safe under concurrency.

use super::board::{Board, ClaimError, EpicEdit, NewTask, Outcome, Patch, TaskEdit, Wait};
use super::bridge;
use super::text::SessionRow;
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
    check_schema(&conn)?;
    // A failed init is retried on the next open instead of being cached for the
    // life of a long-running MCP server.
    let mut done = INIT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !*done {
        // First: a refused file is left exactly as it was (no WAL switch, no DDL).
        check_schema(&conn)?;
        // SQLite's journal-mode transition can return BUSY immediately even
        // with busy_timeout, when another cold opener is switching to WAL.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            match conn.execute_batch("PRAGMA journal_mode=WAL") {
                Ok(()) => break,
                Err(e)
                    if matches!(
                        e.sqlite_error_code(),
                        Some(
                            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                        )
                    ) && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        conn.execute_batch(SCHEMA)
            .map_err(|e| format!("schema: {e}"))?;
        conn.execute(
            "UPDATE board_stamp SET opened_by=? WHERE id=1",
            [crate::ident::stamp()],
        )
        .map_err(|e| e.to_string())?;
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
    if !tasks.iter().any(|c| c == "reopen_count") {
        let p = db_path();
        return Err(format!(
            "this board file predates task reopen_count and cannot be used; move it aside and start fresh (there is no migration): mv {0} {0}.old",
            p.display()
        ));
    }
    check_stamp(conn)
}

fn check_stamp(conn: &Connection) -> Result<(), String> {
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='board_stamp')",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let found: Option<(i64, String)> = if exists {
        conn.query_row(
            "SELECT generation, opened_by FROM board_stamp WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
    } else {
        None
    };
    if found
        .as_ref()
        .is_some_and(|(v, _)| *v == crate::ident::GENERATION)
    {
        return Ok(());
    }
    let board = found.map_or_else(
        || "unstamped".to_owned(),
        |(v, b)| format!("generation={v} {b}"),
    );
    Err(format!(
        "board surface mismatch: board {board}; client generation={} {}; file {}; no migration",
        crate::ident::GENERATION,
        crate::ident::stamp(),
        db_path().display()
    ))
}

fn stamp_writer(conn: &Connection, build: &str) -> Result<(), String> {
    conn.execute("UPDATE board_stamp SET last_writer=? WHERE id=1", [build])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Schema + WAL applied once per process; later opens still check the stamp.
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
    let seq = conn.last_insert_rowid();
    stamp_writer(conn, &crate::ident::stamp())?;
    Ok(seq)
}

pub(super) use super::text::{TaskRow as Row, field_names, text_change, value_change};

const COLS: &str = "id,status,assignee,needs,title,description,goal,path,branch,lease_until,updated_at,created_at,started_at,completed_at,report,note,commit_ref,priority,epic_id,parent_id,reopen_count";

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
        reopen_count: r.get(20)?,
    })
}

fn fetch(conn: &Connection, id: i64) -> Result<Row, String> {
    let sql = format!("SELECT {COLS} FROM tasks WHERE id=?");
    conn.query_row(&sql, params![id], read_row)
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no such task {}", short(id)))
}

mod diffs;
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

    fn diff_add(&self, actor: &str, task: i64, body: &str) -> Result<String, String> {
        diffs::diff_add(actor, task, body)
    }
    fn diff_get(&self, id: i64, full: bool) -> Result<String, String> {
        diffs::diff_get(id, full)
    }
    fn diff_list(&self, of: Ref) -> Result<String, String> {
        diffs::diff_list(of)
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
    fn session_rows(&self) -> Result<Vec<SessionRow>, String> {
        sessions::session_rows()
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
    fn schema_generation_matches_rust() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(SCHEMA).unwrap();
        let generation: i64 = c
            .query_row("SELECT generation FROM board_stamp WHERE id=1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(generation, crate::ident::GENERATION);
    }

    #[test]
    fn first_use_hides_partial_schema_and_rolls_back_interrupted_init() {
        let path = std::env::temp_dir().join(format!(
            "agentboard-init-{}-{}.db",
            std::process::id(),
            super::now_ms()
        ));
        // Stop after tasks exists, before events and the stamp. This is the
        // exact window that used to expose an unusable board to another open.
        let (prefix, tail) = SCHEMA
            .split_once("CREATE TABLE IF NOT EXISTS events(")
            .unwrap();
        let writer = Connection::open(&path).unwrap();
        writer.execute_batch(prefix).unwrap();
        let reader_path = path.clone();
        std::thread::spawn(move || {
            let reader = Connection::open(reader_path).unwrap();
            check_schema(&reader).unwrap();
            let tables: i64 = reader
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(tables, 0, "partial init must be invisible");
        })
        .join()
        .unwrap();
        // Closing without COMMIT models an interrupted first initialization.
        drop(writer);
        let retry = Connection::open(&path).unwrap();
        check_schema(&retry).unwrap();
        assert_eq!(
            retry
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0,
            "interrupted DDL must roll back"
        );
        retry.execute_batch(prefix).unwrap();
        retry
            .execute_batch(&format!("CREATE TABLE IF NOT EXISTS events({tail}"))
            .unwrap();
        check_schema(&retry).unwrap();
        drop(retry);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn surface_stamp_refuses_missing_and_future_generation_without_ddl() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(SCHEMA).unwrap();
        c.execute(
            "UPDATE board_stamp SET generation=99, opened_by='future tools=99 commit=future'",
            [],
        )
        .unwrap();
        let err = check_schema(&c).unwrap_err();
        assert!(
            err.contains("board generation=99 future tools=99 commit=future"),
            "{err}"
        );
        assert!(
            err.contains("client generation=2 agentboard") && err.contains("file "),
            "{err}"
        );
        assert_eq!(
            c.query_row("SELECT generation FROM board_stamp", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            99
        );
        c.execute("DROP TABLE board_stamp", []).unwrap();
        assert!(check_schema(&c).unwrap_err().contains("board unstamped"));
        assert_eq!(
            c.query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='board_stamp'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }

    #[test]
    fn participating_writers_move_the_stamp_but_legacy_writes_are_undetectable() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(SCHEMA).unwrap();
        let read = || {
            c.query_row("SELECT last_writer FROM board_stamp", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap()
        };
        for build in [
            "agentboard 0.1.0 tools=23 commit=first",
            "agentboard 0.1.0 tools=23 commit=second",
        ] {
            super::stamp_writer(&c, build).unwrap();
            assert_eq!(read(), build);
            check_schema(&c).unwrap();
            assert_eq!(read(), build, "reads preserve writer");
        }
        // Pre-stamp clients never call stamp_writer. Their writes cannot be attributed.
        c.execute(
            "INSERT INTO sessions(name,agent,seen) VALUES('legacy','old',0)",
            [],
        )
        .unwrap();
        assert_eq!(read(), "agentboard 0.1.0 tools=23 commit=second");
        c.execute(
            "INSERT INTO tasks(id,title,created_at,updated_at) VALUES(1,'test',0,0)",
            [],
        )
        .unwrap();
        super::log_event(&c, super::Subject::Task(1), "test", "note", "stamp path").unwrap();
        assert_eq!(read(), crate::ident::stamp());
    }

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
    fn a_board_without_reopen_count_is_refused_untouched() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE tasks(id INTEGER PRIMARY KEY, epic_id INTEGER, parent_id INTEGER, path TEXT, branch TEXT);
             CREATE TABLE epics(id INTEGER PRIMARY KEY);
             CREATE TABLE events(seq INTEGER PRIMARY KEY, task_id INTEGER, epic_id INTEGER);",
        )
        .unwrap();
        let err = check_schema(&c).unwrap_err();
        assert!(
            err.contains("predates task reopen_count") && err.contains("mv "),
            "{err}"
        );
        println!("{err}");
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
