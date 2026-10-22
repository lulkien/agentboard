//! The SQLite backend, with transactional schema checks and additive migrations.
//! One SQLite file, WAL, short-immediate transactions. Every operation opens its
//! own connection: cheap, and safe under concurrency.

use super::board::{Board, ClaimError, EpicEdit, NewTask, Outcome, Patch, TaskEdit, Wait};
use super::bridge;
use super::text::SessionRow;
use super::{DEFAULT_TTL_S, Ref, STATUS_LIST, db_path, norm, now_ms, short, short_epic};
use rusqlite::{Connection, OptionalExtension, params};
use std::time::Duration;

pub const SCHEMA: &str = include_str!("../../schema.sql");

pub(super) fn open() -> Result<Connection, String> {
    let path = db_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("db dir {}: {e}", dir.display()))?;
    }
    let conn = Connection::open(&path).map_err(|e| format!("open {}: {e}", path.display()))?;
    conn.busy_timeout(Duration::from_millis(5000))
        .map_err(|e| e.to_string())?;
    // Per-connection settings are cheap. The schema check takes a writer lock
    // only for initialization or an older version. The WAL switch uses no
    // explicit transaction lock and runs only when the mode is not already WAL.
    conn.execute_batch("PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;")
        .map_err(|e| e.to_string())?;
    check_schema(&conn)?;
    let mode: String = conn
        .pragma_query_value(None, "journal_mode", |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if mode != "wal" {
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
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(e) => return Err(e.to_string()),
            }
        }
    }
    Ok(conn)
}

fn version(conn: &Connection) -> Result<Option<i64>, String> {
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='board_stamp' AND type='table')",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !exists {
        return Ok(None);
    }
    conn.query_row("SELECT generation FROM board_stamp WHERE id=1", [], |r| {
        r.get(0)
    })
    .optional()
    .map_err(|e| e.to_string())
}

fn compatible(found: Option<i64>) -> Result<(), String> {
    if found.is_some_and(|v| v > crate::ident::GENERATION) {
        return Err(format!(
            "board schema mismatch: board schema version {}; binary is older than the board (understands version {}); use a newer binary",
            found.unwrap(),
            crate::ident::GENERATION
        ));
    }
    if found.is_some_and(|v| v < 1) {
        return Err("board schema mismatch: unsupported schema version; manual step: inspect the board offline".into());
    }
    Ok(())
}

#[derive(Debug)]
struct Column {
    name: String,
    ty: String,
    required: bool,
    default: Option<String>,
    key: bool,
}
fn columns(conn: &Connection, table: &str) -> Result<Vec<Column>, String> {
    conn.prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| e.to_string())?
        .query_map([], |r| {
            Ok(Column {
                name: r.get(1)?,
                ty: r.get(2)?,
                required: r.get(3)?,
                default: r.get(4)?,
                key: r.get::<_, i64>(5)? != 0,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

/// BEGIN IMMEDIATE is SQLite's cross-process board write lock. Re-read the
/// version after taking it; definitions, defaults and version commit together.
fn check_schema(conn: &Connection) -> Result<(), String> {
    let found = version(conn)?;
    compatible(found)?;
    if found == Some(crate::ident::GENERATION) {
        return Ok(());
    }
    conn.execute_batch("BEGIN IMMEDIATE").map_err(|e| {
        format!("schema migration lock: {e}; manual step: release the board writer and retry")
    })?;
    let result = migrate_locked(conn);
    match result {
        Ok(notice) => {
            conn.execute_batch("COMMIT").map_err(|e| e.to_string())?;
            if let Some((v, definitions, rows)) = notice {
                crate::store::migrated(v, definitions, rows);
            }
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

fn validate_plan(schema: &str) -> Result<(), String> {
    let sql = schema
        .lines()
        .map(|line| line.split("--").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    for statement in sql.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        if ![
            "CREATE TABLE IF NOT EXISTS ",
            "CREATE INDEX IF NOT EXISTS ",
            "INSERT OR IGNORE INTO board_stamp(",
        ]
        .iter()
        .any(|prefix| statement.starts_with(prefix))
            && statement != "BEGIN IMMEDIATE"
            && statement != "COMMIT"
        {
            return Err(format!(
                "board schema mismatch: non-additive migration statement {statement}; manual step: provide an offline migration"
            ));
        }
    }
    Ok(())
}

fn migrate_locked(conn: &Connection) -> Result<Option<(i64, usize, usize)>, String> {
    let found = version(conn)?;
    compatible(found)?;
    if found == Some(crate::ident::GENERATION) {
        return Ok(None);
    }
    let existing: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if found.is_none() && existing != 0 {
        return Err("board schema mismatch: no schema version; manual step: inspect and stamp the legacy board offline".into());
    }
    validate_plan(SCHEMA)?;
    let expected = Connection::open_in_memory().map_err(|e| e.to_string())?;
    expected.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    let tables = [
        "epics",
        "tasks",
        "events",
        "diffs",
        "sessions",
        "board_stamp",
    ];
    let mut additions = Vec::new();
    let mut backfilled = 0;
    let mut fills = Vec::new();
    for table in tables {
        let old = columns(conn, table)?;
        if old.is_empty() {
            continue;
        }
        let mut missing = Vec::new();
        for column in columns(&expected, table)? {
            if let Some(default) = &column.default {
                missing.push(if old.iter().any(|c| c.name == column.name) {
                    format!("{} IS NULL", column.name)
                } else {
                    "1".to_owned()
                });
                fills.push(format!(
                    "UPDATE {table} SET {}={default} WHERE {} IS NULL",
                    column.name, column.name
                ));
            }
            if let Some(previous) = old.iter().find(|c| c.name == column.name) {
                if previous.ty != column.ty {
                    return Err(format!(
                        "board schema mismatch: non-additive type change {table}.{}; manual step: reconcile field types offline",
                        column.name
                    ));
                }
            } else {
                if column.key || (column.required && column.default.is_none()) {
                    return Err(format!(
                        "board schema mismatch: non-additive field {table}.{} has no default; manual step: supply an offline migration",
                        column.name
                    ));
                }
                let default = column
                    .default
                    .map_or(String::new(), |d| format!(" DEFAULT {d}"));
                additions.push(format!(
                    "ALTER TABLE {table} ADD COLUMN {} {}{}{default};",
                    column.name,
                    column.ty,
                    if column.required { " NOT NULL" } else { "" }
                ));
            }
        }
        if !missing.is_empty() {
            backfilled += conn
                .query_row(
                    &format!(
                        "SELECT count(*) FROM {table} WHERE {}",
                        missing.join(" OR ")
                    ),
                    [],
                    |r| r.get::<_, usize>(0),
                )
                .map_err(|e| e.to_string())?;
        }
    }
    let before: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type IN ('table','index')",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    for sql in &additions {
        conn.execute_batch(sql).map_err(|e| e.to_string())?;
    }
    conn.execute_batch(
        &SCHEMA
            .replace("BEGIN IMMEDIATE;", "")
            .replace("COMMIT;", ""),
    )
    .map_err(|e| format!("schema migration: {e}"))?;
    for fill in fills {
        conn.execute(&fill, [])
            .map_err(|e| format!("schema backfill: {e}"))?;
    }
    conn.execute(
        "UPDATE board_stamp SET generation=? WHERE id=1",
        [crate::ident::GENERATION],
    )
    .map_err(|e| e.to_string())?;
    let after: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type IN ('table','index')",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(found.map(|v| (v, additions.len() + (after - before) as usize, backfilled)))
}

fn write_transaction(conn: &mut Connection) -> Result<rusqlite::Transaction<'_>, String> {
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let found = version(&tx)?;
    compatible(found)?;
    if found != Some(crate::ident::GENERATION) {
        return Err("board schema mismatch: schema changed during operation; retry".into());
    }
    Ok(tx)
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
    let seq = conn.last_insert_rowid();

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

    fn diff_add(
        &self,
        actor: &str,
        task: i64,
        body: &str,
        reviews: Option<i64>,
    ) -> Result<String, String> {
        diffs::diff_add(actor, task, body, reviews)
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
    fn interrupted_definitions_roll_back() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("BEGIN IMMEDIATE; CREATE TABLE partial(id INTEGER);")
            .unwrap();
        c.execute_batch("ROLLBACK").unwrap();
        check_schema(&c).unwrap();
        assert_eq!(super::version(&c).unwrap(), Some(crate::ident::GENERATION));
        assert!(super::columns(&c, "partial").unwrap().is_empty());
    }

    #[test]
    fn non_additive_statements_are_named() {
        for statement in [
            "DROP TABLE tasks;",
            "ALTER TABLE tasks DROP COLUMN title;",
            "UPDATE tasks SET title='rewrite';",
        ] {
            let error = super::validate_plan(statement).unwrap_err();
            assert!(error.contains("non-additive") && error.contains("manual step"));
        }
        super::validate_plan(SCHEMA).unwrap();
    }

    #[test]
    fn existing_missing_defaults_are_backfilled_only_when_null() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(&SCHEMA.replace(
            "branch       TEXT NOT NULL DEFAULT 'HEAD'",
            "branch       TEXT DEFAULT 'HEAD'",
        ))
        .unwrap();
        c.execute_batch("UPDATE board_stamp SET generation=1; INSERT INTO tasks(title,branch,created_at,updated_at) VALUES('missing',NULL,1,1),('present','feat/keep',1,1);").unwrap();
        check_schema(&c).unwrap();
        let values: Vec<String> = c
            .prepare("SELECT branch FROM tasks ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(values, ["HEAD", "feat/keep"]);
    }

    #[test]
    fn older_schema_migrates_without_rewriting_present_values() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(&SCHEMA.replace("  reviews    INTEGER", "  old_reviews INTEGER"))
            .unwrap();
        c.execute_batch("UPDATE board_stamp SET generation=1; INSERT INTO tasks(title,branch,created_at,updated_at) VALUES('preserved','feat/keep',1,1);").unwrap();
        check_schema(&c).unwrap();
        assert_eq!(super::version(&c).unwrap(), Some(crate::ident::GENERATION));
        let branch: String = c
            .query_row("SELECT branch FROM tasks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(branch, "feat/keep");
        assert!(
            super::columns(&c, "diffs")
                .unwrap()
                .iter()
                .any(|c| c.name == "reviews")
        );
        check_schema(&c).unwrap();
    }

    #[test]
    fn failed_version_bump_rolls_back_definitions() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(&SCHEMA.replace("  reviews    INTEGER", "  old_reviews INTEGER"))
            .unwrap();
        c.execute_batch("UPDATE board_stamp SET generation=1; CREATE TRIGGER fail_version BEFORE UPDATE OF generation ON board_stamp BEGIN SELECT RAISE(ABORT,'simulated interruption'); END;").unwrap();
        assert!(check_schema(&c).is_err());
        assert_eq!(super::version(&c).unwrap(), Some(1));
        assert!(
            !super::columns(&c, "diffs")
                .unwrap()
                .iter()
                .any(|c| c.name == "reviews")
        );
        c.execute_batch("DROP TRIGGER fail_version").unwrap();
        check_schema(&c).unwrap();
        assert_eq!(super::version(&c).unwrap(), Some(2));
    }

    #[test]
    fn future_version_is_refused_on_the_next_call() {
        let c = Connection::open_in_memory().unwrap();
        check_schema(&c).unwrap();
        c.execute("UPDATE board_stamp SET generation=3", [])
            .unwrap();
        assert!(
            check_schema(&c)
                .unwrap_err()
                .contains("binary is older than the board")
        );
        assert_eq!(super::version(&c).unwrap(), Some(3));
    }

    #[test]
    fn non_additive_type_change_rolls_back() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(&SCHEMA.replace("title        TEXT", "title        INTEGER"))
            .unwrap();
        c.execute("UPDATE board_stamp SET generation=1", [])
            .unwrap();
        let error = check_schema(&c).unwrap_err();
        assert!(
            error.contains("non-additive type change") && error.contains("manual step"),
            "{error}"
        );
        assert_eq!(super::version(&c).unwrap(), Some(1));
    }

    #[test]
    fn fresh_and_current_files_pass_the_schema_check() {
        let c = Connection::open_in_memory().unwrap();
        check_schema(&c).unwrap();
        c.execute_batch(SCHEMA).unwrap();
        check_schema(&c).unwrap();
    }
}
