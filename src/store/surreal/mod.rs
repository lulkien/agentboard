//! The SurrealDB backend, compiled only under the `surreal` feature: settings,
//! auto-serve, the write lock, the connect path with its schema bootstrap, and one
//! module per concern (`tasks`, `epics`, `sessions`, `render`, `dispatch`, `wait`)
//! implementing every `Board` operation.

mod diffs;
mod dispatch;
mod epics;
mod lock;
mod render;
mod serve;
mod sessions;
mod tasks;
pub(super) mod upgrade;
mod wait;

use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::{Client, Ws};
use surrealdb::opt::auth::Root;
use tokio::runtime::Runtime;

use super::board::{Board, ClaimError, EpicEdit, NewTask, Outcome, Patch, TaskEdit, Wait};
use super::bridge;
use super::text::SessionRow;

/// Every board operation, on SurrealDB.
pub struct SurrealBoard;

impl Board for SurrealBoard {
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
        tasks::list(status, assignee, limit)
    }
    fn get(&self, id: i64, full: bool) -> Result<String, String> {
        tasks::get(id, full)
    }
    fn children(&self, of: super::Ref) -> Result<String, String> {
        tasks::children(of)
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
    fn diff_list(&self, of: super::Ref) -> Result<String, String> {
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

/// Bumped when a change to the schema below is NOT readable by the previous
/// version. Adding a field with a DEFAULT is compatible — old records read with
/// the default and no board is refused — so `tasks.path` / `tasks.branch` did
/// not move this marker.
pub const SCHEMA_VERSION: i64 = crate::ident::GENERATION;
const NAMESPACE: &str = "agentboard";
const DATABASE: &str = "board";

const DEFAULT_URL: &str = "127.0.0.1:18720";

/// Where and as whom to reach the server. Parsed once, purely, from the
/// environment so bad values fail with one line before anything is started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// `HOST:PORT`, loopback only; also the address the server is told to bind.
    pub url: String,
    pub user: String,
    pub pass: String,
    /// The `surrealkv://` data directory; the write lock lives inside it.
    pub dir: PathBuf,
}

impl Settings {
    /// `get` reads one environment variable; `db` is `--db` / `AGENTBOARD_DB` when
    /// set; `state_base` is the state directory that `db_path` uses.
    pub fn parse(
        get: &dyn Fn(&str) -> Option<String>,
        db: Option<PathBuf>,
        state_base: &Path,
    ) -> Result<Self, String> {
        let url = get("AGENTBOARD_SURREAL_URL").unwrap_or_else(|| DEFAULT_URL.to_owned());
        check_url(&url)?;
        let user = get("AGENTBOARD_SURREAL_USER").unwrap_or_else(|| "root".to_owned());
        if user.trim().is_empty() {
            return Err(
                "AGENTBOARD_SURREAL_USER is empty: want a non-empty user name (default root)"
                    .into(),
            );
        }
        let pass = get("AGENTBOARD_SURREAL_PASS").unwrap_or_else(|| "root".to_owned());
        if pass.is_empty() {
            return Err(
                "AGENTBOARD_SURREAL_PASS is empty: want a non-empty password (default root)".into(),
            );
        }
        let dir = db.unwrap_or_else(|| state_base.join("agentboard/surreal"));
        Ok(Self {
            url,
            user,
            pass,
            dir,
        })
    }

    /// From the process environment and the `--db` override, as `db_path` does.
    pub fn from_env() -> Result<Self, String> {
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let db = super::DB_OVERRIDE
            .get()
            .cloned()
            .or_else(|| get("AGENTBOARD_DB").map(PathBuf::from));
        let base = get("XDG_STATE_HOME").map_or_else(
            || PathBuf::from(get("HOME").unwrap_or_else(|| ".".into())).join(".local/state"),
            PathBuf::from,
        );
        Self::parse(&get, db, &base)
    }

    #[cfg(test)]
    pub fn defaults_for_test(dir: &str, url: &str) -> Self {
        Self {
            url: url.to_owned(),
            user: "root".into(),
            pass: "secret".into(),
            dir: PathBuf::from(dir),
        }
    }
}

fn check_url(raw: &str) -> Result<(), String> {
    let bad = || {
        format!(
            "AGENTBOARD_SURREAL_URL {raw:?}: want HOST:PORT on loopback (127.0.0.1, localhost or [::1]), e.g. {DEFAULT_URL}"
        )
    };
    let (host, port) = raw.rsplit_once(':').ok_or_else(bad)?;
    if !matches!(host, "127.0.0.1" | "localhost" | "[::1]") {
        return Err(bad());
    }
    match port.parse::<u16>() {
        Ok(p) if p > 0 => Ok(()),
        _ => Err(bad()),
    }
}

/// A server that is not 2.7.x is refused: the client crate and the schema are
/// written for it, and a stale server from another build must never be written to.
pub fn check_version(url: &str, body: &str) -> Result<(), String> {
    if body.trim().starts_with("surrealdb-2.7.") {
        Ok(())
    } else {
        Err(format!(
            "board server at {url} is {:?}, want surrealdb 2.7.x. Stop it, or point AGENTBOARD_SURREAL_URL at another port.",
            body.trim()
        ))
    }
}

/// The refusal for a database that is not a current agentboard board.
pub fn schema_refusal(found: Option<i64>, s: &Settings) -> String {
    let found = found.map_or("no schema marker".to_owned(), |v| {
        format!("schema version {v}")
    });
    format!(
        "board schema mismatch: found {found}, want version {SCHEMA_VERSION}. Board {} at {}. Owner cutover: stop every seat and server (pkill -f 'bind {}'); run agentboard board upgrade --owner with the same --db and AGENTBOARD_SURREAL_URL; reinstall; restart seats; run python3 scripts/check-seat-surface.py. No automatic upgrade; unsupported generations are refused.",
        s.dir.display(),
        s.url,
        s.url
    )
}

/// The one shared runtime. The sync surface `block_on`s it per call.
fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

/// Idempotent: `IF NOT EXISTS` on every definition, so a second run changes nothing.
/// Re-applied by every write session, not only at first bootstrap: that is how a field
/// added to this block reaches a board that already exists (see `session_with`).
/// Ids are integers, as in the SQLite schema; `events` carries the SQLite CHECK
/// that exactly one of `task_id` / `epic_id` is set.
const SCHEMA: &str = "
DEFINE TABLE IF NOT EXISTS meta SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS version ON meta TYPE int;
DEFINE FIELD IF NOT EXISTS build ON meta TYPE string DEFAULT '';

DEFINE TABLE IF NOT EXISTS counters SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS n ON counters TYPE int;

DEFINE TABLE IF NOT EXISTS epics SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS title ON epics TYPE string;
DEFINE FIELD IF NOT EXISTS description ON epics TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS goal ON epics TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS status ON epics TYPE string DEFAULT 'open';
DEFINE FIELD IF NOT EXISTS path ON epics TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS created_at ON epics TYPE int;
DEFINE FIELD IF NOT EXISTS updated_at ON epics TYPE int;
DEFINE FIELD IF NOT EXISTS completed_at ON epics TYPE option<int>;

DEFINE TABLE IF NOT EXISTS tasks SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS board ON tasks TYPE string DEFAULT 'main';
DEFINE FIELD IF NOT EXISTS title ON tasks TYPE string;
DEFINE FIELD IF NOT EXISTS description ON tasks TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS goal ON tasks TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS path ON tasks TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS branch ON tasks TYPE string DEFAULT 'HEAD';
DEFINE FIELD IF NOT EXISTS status ON tasks TYPE string DEFAULT 'ready';
DEFINE FIELD IF NOT EXISTS assignee ON tasks TYPE option<string>;
DEFINE FIELD IF NOT EXISTS priority ON tasks TYPE int DEFAULT 100;
DEFINE FIELD IF NOT EXISTS epic_id ON tasks TYPE option<int>;
DEFINE FIELD IF NOT EXISTS parent_id ON tasks TYPE option<int>;
DEFINE FIELD IF NOT EXISTS reopen_count ON tasks TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS needs ON tasks TYPE option<string>;
DEFINE FIELD IF NOT EXISTS report ON tasks TYPE option<string>;
DEFINE FIELD IF NOT EXISTS note ON tasks TYPE option<string>;
DEFINE FIELD IF NOT EXISTS commit_ref ON tasks TYPE option<string>;
DEFINE FIELD IF NOT EXISTS lease_until ON tasks TYPE option<int>;
DEFINE FIELD IF NOT EXISTS created_at ON tasks TYPE int;
DEFINE FIELD IF NOT EXISTS updated_at ON tasks TYPE int;
DEFINE FIELD IF NOT EXISTS started_at ON tasks TYPE option<int>;
DEFINE FIELD IF NOT EXISTS completed_at ON tasks TYPE option<int>;
DEFINE INDEX IF NOT EXISTS tasks_status ON tasks FIELDS status, priority;
DEFINE INDEX IF NOT EXISTS tasks_lease ON tasks FIELDS lease_until;
DEFINE INDEX IF NOT EXISTS tasks_assignee ON tasks FIELDS assignee, status;
DEFINE INDEX IF NOT EXISTS tasks_epic ON tasks FIELDS epic_id;
DEFINE INDEX IF NOT EXISTS tasks_parent ON tasks FIELDS parent_id;

DEFINE TABLE IF NOT EXISTS events SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS task_id ON events TYPE option<int>;
DEFINE FIELD IF NOT EXISTS epic_id ON events TYPE option<int>
    ASSERT ($value != NONE) != ($this.task_id != NONE);
DEFINE FIELD IF NOT EXISTS actor ON events TYPE string;
DEFINE FIELD IF NOT EXISTS kind ON events TYPE string;
DEFINE FIELD IF NOT EXISTS body ON events TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS ts ON events TYPE int;
DEFINE INDEX IF NOT EXISTS ev_task ON events FIELDS task_id;
DEFINE INDEX IF NOT EXISTS ev_epic ON events FIELDS epic_id;

DEFINE TABLE IF NOT EXISTS diffs SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS task_id ON diffs TYPE int;
DEFINE FIELD IF NOT EXISTS round ON diffs TYPE int;
DEFINE FIELD IF NOT EXISTS body ON diffs TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS created_at ON diffs TYPE int;
DEFINE INDEX IF NOT EXISTS diffs_task ON diffs FIELDS task_id;

DEFINE TABLE IF NOT EXISTS sessions SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS agent ON sessions TYPE string;
DEFINE FIELD IF NOT EXISTS cwd ON sessions TYPE option<string>;
DEFINE FIELD IF NOT EXISTS task_id ON sessions TYPE option<int>;
DEFINE FIELD IF NOT EXISTS seen ON sessions TYPE int;
DEFINE FIELD IF NOT EXISTS status ON sessions TYPE string DEFAULT 'up';
";

/// A `DEFAULT` applies when a row is created, never to a row already stored, and
/// SCHEMAFULL refuses any write to a row that lacks a non-optional field. So the
/// `DEFINE FIELD f ON t TYPE <not option> DEFAULT d` lines of `SCHEMA` also yield,
/// per table, one `UPDATE t SET f = f ?? d, ... WHERE f IS NONE OR ...` run right
/// after the definitions: a field added to `SCHEMA` reaches the rows that predate it
/// with no further code. One statement per table, because a row is validated whole
/// after each update and would still lack the table's other new fields.
fn backfill() -> String {
    let mut tables: Vec<(&str, Vec<(&str, &str)>)> = Vec::new();
    for l in SCHEMA.lines() {
        let Some(rest) = l.strip_prefix("DEFINE FIELD IF NOT EXISTS ") else {
            continue;
        };
        let Some((field, rest)) = rest.split_once(" ON ") else {
            continue;
        };
        let Some((table, rest)) = rest.split_once(" TYPE ") else {
            continue;
        };
        let Some((ty, default)) = rest.split_once(" DEFAULT ") else {
            continue;
        };
        if ty.starts_with("option<") {
            continue;
        }
        let default = default.trim_end_matches(';');
        match tables.iter_mut().find(|(t, _)| *t == table) {
            Some((_, fields)) => fields.push((field, default)),
            None => tables.push((table, vec![(field, default)])),
        }
    }
    tables
        .iter()
        .map(|(table, fields)| {
            let set: Vec<_> = fields
                .iter()
                .map(|(f, d)| format!("{f} = {f} ?? {d}"))
                .collect();
            let missing: Vec<_> = fields.iter().map(|(f, _)| format!("{f} IS NONE")).collect();
            format!(
                "UPDATE {table} SET {} WHERE {};\n",
                set.join(", "),
                missing.join(" OR ")
            )
        })
        .collect()
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Bootstrap {
    /// An empty database: tables defined and the marker written.
    Created,
    /// A current board: definitions re-applied, nothing changed.
    Already,
}

pub type Db = Surreal<Client>;

/// A connection without a held lock. Used by the tests; the verbs use `Session`.
#[cfg(test)]
pub struct Conn {
    pub db: Db,
    pub bootstrap: Bootstrap,
    /// The pid of the server this call started, or `None` if one was already up.
    pub spawned_pid: Option<u32>,
}

#[derive(Deserialize)]
struct Marker {
    version: i64,
}

/// Whether a driver error says the connection to the server went away mid-request
/// (the server was killed or stopped), as opposed to the database refusing a query.
fn connection_lost(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    ["connection reset", "connection closed", "broken pipe"]
        .iter()
        .any(|p| m.contains(p))
}

/// `what: err`, except that an outage keeps its `board server at ...` line whole: the
/// one-line outage shape is the contract, whatever verb met it.
fn with_context(what: &str, err: &str) -> String {
    if err.starts_with("board server at ") {
        err.to_owned()
    } else {
        format!("{what}: {err}")
    }
}

/// A database error as one line. A connection lost mid-request is an outage, so it
/// takes the same `board server at URL: ...` shape as a server that cannot be reached.
fn db_err(e: &surrealdb::Error) -> String {
    let msg = e.to_string();
    if connection_lost(&msg)
        && let Ok(s) = Settings::from_env()
    {
        return format!("board server at {}: connection lost: {msg}", s.url);
    }
    format!("board database: {msg}")
}

/// Brings the database to the current schema, or refuses it. The caller holds
/// the write lock, so two processes never bootstrap at once.
async fn bootstrap(db: &Db, s: &Settings) -> Result<Bootstrap, String> {
    let mut r = db
        .query("SELECT version FROM meta:schema")
        .await
        .map_err(|e| db_err(&e))?;
    let marker: Vec<Marker> = r.take(0).map_err(|e| db_err(&e))?;
    let found = marker.first().map(|m| m.version);
    if let Some(v) = found
        && v != SCHEMA_VERSION
    {
        return Err(schema_refusal(Some(v), s));
    }
    if found.is_none() {
        let mut r = db.query("INFO FOR DB").await.map_err(|e| db_err(&e))?;
        let info: Option<serde_json::Value> = r.take(0).map_err(|e| db_err(&e))?;
        let tables = info
            .as_ref()
            .and_then(|v| v.get("tables"))
            .and_then(|t| t.as_object())
            .map_or(0, serde_json::Map::len);
        if tables > 0 {
            return Err(schema_refusal(None, s));
        }
    }
    db.query(SCHEMA)
        .await
        .and_then(surrealdb::Response::check)
        .map_err(|e| db_err(&e))?;
    db.query(backfill())
        .await
        .and_then(surrealdb::Response::check)
        .map_err(|e| db_err(&e))?;
    if found.is_some() {
        return Ok(Bootstrap::Already);
    }
    db.query("CREATE meta:schema SET version = $v, build = $build")
        .bind(("v", SCHEMA_VERSION))
        .bind(("build", crate::ident::stamp()))
        .await
        .and_then(surrealdb::Response::check)
        .map_err(|e| db_err(&e))?;
    Ok(Bootstrap::Created)
}

/// Serve if needed, check the version, then connect, sign in and bootstrap.
#[cfg(test)]
pub fn connect(s: &Settings) -> Result<Conn, String> {
    connect_with(s, &serve::Timing::default(), None, lock::DEFAULT_WAIT)
}

/// `connect` with the budgets and the server binary injectable for tests. Takes
/// no lock beyond the first bootstrap.
#[cfg(test)]
pub fn connect_with(
    s: &Settings,
    t: &serve::Timing,
    bin: Option<&Path>,
    lock_wait: Duration,
) -> Result<Conn, String> {
    let sess = session_with(s, t, bin, lock_wait, false)?;
    Ok(Conn {
        db: sess.db,
        bootstrap: sess.bootstrap,
        spawned_pid: sess.spawned_pid,
    })
}

/// A connected, bootstrapped database handle. A write session also holds the
/// cross-process write lock until it is dropped: the whole read-modify-write
/// runs inside it. A read session holds none, because every write is one
/// server-side transaction, so a read sees a write whole or not at all.
pub struct Session {
    pub db: Db,
    // Read by the tests only.
    #[cfg_attr(not(test), allow(dead_code))]
    pub bootstrap: Bootstrap,
    #[cfg_attr(not(test), allow(dead_code))]
    pub spawned_pid: Option<u32>,
    _lock: Option<lock::WriteLock>,
}

/// A session with settings and budgets from the environment (what a verb uses).
pub fn session(write: bool) -> Result<Session, String> {
    session_with(
        &Settings::from_env()?,
        &serve::Timing::default(),
        None,
        lock::DEFAULT_WAIT,
        write,
    )
}

/// Runs a future on the shared runtime from the sync surface. The CLI's `main`
/// is itself async, and `block_on` panics inside a runtime, so when the caller is
/// already on one the future runs on a short-lived helper thread instead.
pub fn block_on<T: Send>(fut: impl std::future::Future<Output = T> + Send) -> T {
    if tokio::runtime::Handle::try_current().is_ok() {
        std::thread::scope(|s| {
            s.spawn(|| runtime().block_on(fut))
                .join()
                .unwrap_or_else(|e| std::panic::resume_unwind(e))
        })
    } else {
        runtime().block_on(fut)
    }
}

fn session_with(
    s: &Settings,
    t: &serve::Timing,
    bin: Option<&Path>,
    lock_wait: Duration,
    write: bool,
) -> Result<Session, String> {
    let served = serve::ensure_up(s, t, bin)?;
    let (_, body) = serve::http_get(&s.url, "/version", t.probe.max(Duration::from_secs(1)))
        .map_err(|e| serve::unavailable(s, &format!("/version failed ({e}) for")))?;
    check_version(&s.url, &body)?;
    std::fs::create_dir_all(&s.dir).map_err(|e| format!("data dir {}: {e}", s.dir.display()))?;
    // A write takes the lock before it reads anything, so nothing it reads can
    // change under it.
    let mut guard = if write {
        Some(lock::acquire(&s.dir, lock_wait)?)
    } else {
        None
    };
    // A server can answer /health and /version and still never finish the
    // WebSocket upgrade; every phase is bounded.
    let db = bounded(s, t, open_db(s))?;
    let found = bounded(s, t, marker_version(&db))?;
    let bootstrap = match found {
        // The marker only says which schema generation the board is, not that every
        // field of this build exists on it. Every write session (the lock is already
        // held, so two processes never define at once) re-runs SCHEMA: `IF NOT EXISTS`
        // makes it a no-op except for a field added since the board was created, so
        // any call that writes is guaranteed to carry a new field. A read session
        // skips the pass (it would pay a DDL round trip and, unlocked, race a
        // writer); a record written before a field existed reads back through the
        // serde defaults on its row type.
        Some(v) if v == SCHEMA_VERSION && write => bounded(s, t, bootstrap(&db, s))?,
        Some(v) if v == SCHEMA_VERSION => Bootstrap::Already,
        Some(v) => {
            let build = bounded(s, t, async {
                let mut r = db
                    .query("SELECT VALUE build FROM meta:schema")
                    .await
                    .map_err(|e| db_err(&e))?;
                let builds: Vec<Option<String>> = r.take(0).map_err(|e| db_err(&e))?;
                Ok(builds
                    .into_iter()
                    .next()
                    .flatten()
                    .unwrap_or_else(|| "unstamped".to_owned()))
            })?;
            return Err(format!(
                "{} board build={build}; client build={}",
                schema_refusal(Some(v), s),
                crate::ident::stamp()
            ));
        }
        None => {
            // First use of this database: bootstrap under the lock so two
            // processes never define the schema at once.
            if guard.is_none() {
                guard = Some(lock::acquire(&s.dir, lock_wait)?);
            }
            bounded(s, t, bootstrap(&db, s))?
        }
    };
    if write {
        bounded(s, t, async {
            db.query("UPSERT meta:opened SET version=$v, build=$build")
                .bind(("v", SCHEMA_VERSION))
                .bind(("build", crate::ident::stamp()))
                .await
                .and_then(surrealdb::Response::check)
                .map_err(|e| db_err(&e))?;
            Ok(())
        })?;
    }
    Ok(Session {
        db,
        bootstrap,
        spawned_pid: served.spawned_pid,
        _lock: if write { guard } else { None },
    })
}

fn bounded<T: Send>(
    s: &Settings,
    t: &serve::Timing,
    fut: impl std::future::Future<Output = Result<T, String>> + Send,
) -> Result<T, String> {
    block_on(async {
        match tokio::time::timeout(t.connect, fut).await {
            Ok(r) => r,
            Err(_) => Err(serve::unavailable(
                s,
                &format!(
                    "did not complete the WebSocket handshake within {} ms for",
                    t.connect.as_millis()
                ),
            )),
        }
    })
}

async fn open_db(s: &Settings) -> Result<Db, String> {
    let db = Surreal::new::<Ws>(s.url.as_str())
        .await
        .map_err(|e| format!("board server at {}: {e}", s.url))?;
    db.signin(Root {
        username: &s.user,
        password: &s.pass,
    })
    .await
    .map_err(|e| format!("board server at {}: sign-in failed: {e}", s.url))?;
    db.use_ns(NAMESPACE)
        .use_db(DATABASE)
        .await
        .map_err(|e| db_err(&e))?;
    Ok(db)
}

async fn marker_version(db: &Db) -> Result<Option<i64>, String> {
    let mut r = db
        .query("SELECT version FROM meta:schema")
        .await
        .map_err(|e| db_err(&e))?;
    let marker: Vec<Marker> = r.take(0).map_err(|e| db_err(&e))?;
    Ok(marker.first().map(|m| m.version))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command, Stdio};
    use std::time::Instant;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k| m.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())
    }

    #[test]
    fn an_outage_line_keeps_its_shape_under_a_verb_prefix() {
        assert_eq!(
            with_context("add", "board server at 127.0.0.1:1: connection lost: x"),
            "board server at 127.0.0.1:1: connection lost: x"
        );
        assert_eq!(
            with_context("add", "board database: x"),
            "add: board database: x"
        );
    }

    #[test]
    fn a_connection_lost_mid_request_is_an_outage_not_a_database_error() {
        assert!(connection_lost("I/O error: connection reset"));
        assert!(connection_lost("Connection closed by peer"));
        assert!(connection_lost("Broken pipe (os error 32)"));
        assert!(!connection_lost(
            "The query was not executed due to a failed transaction"
        ));
    }

    #[test]
    fn settings_defaults_and_overrides() {
        let base = Path::new("/state");
        let s = Settings::parse(&env(&[]), None, base).unwrap();
        assert_eq!(s.url, "127.0.0.1:18720");
        assert_eq!((s.user.as_str(), s.pass.as_str()), ("root", "root"));
        assert_eq!(s.dir, Path::new("/state/agentboard/surreal"));
        let s = Settings::parse(
            &env(&[
                ("AGENTBOARD_SURREAL_URL", "localhost:9"),
                ("AGENTBOARD_SURREAL_USER", "ab"),
                ("AGENTBOARD_SURREAL_PASS", "pw"),
            ]),
            Some(PathBuf::from("/x/data")),
            base,
        )
        .unwrap();
        assert_eq!(
            (s.url.as_str(), s.user.as_str(), s.pass.as_str()),
            ("localhost:9", "ab", "pw")
        );
        assert_eq!(s.dir, Path::new("/x/data"), "--db / AGENTBOARD_DB wins");
    }

    #[test]
    fn bad_settings_name_the_variable_and_the_shape() {
        let base = Path::new("/state");
        for url in [
            "",
            "nohost",
            "0.0.0.0:18720",
            "example.com:80",
            "127.0.0.1:0",
            "127.0.0.1:99999",
            "127.0.0.1:x",
        ] {
            let e =
                Settings::parse(&env(&[("AGENTBOARD_SURREAL_URL", url)]), None, base).unwrap_err();
            assert!(
                e.starts_with("AGENTBOARD_SURREAL_URL ")
                    && e.contains("want HOST:PORT on loopback"),
                "{url}: {e}"
            );
            assert!(!e.contains('\n'));
        }
        let e = Settings::parse(&env(&[("AGENTBOARD_SURREAL_USER", " ")]), None, base).unwrap_err();
        assert!(e.starts_with("AGENTBOARD_SURREAL_USER is empty"), "{e}");
        let e = Settings::parse(&env(&[("AGENTBOARD_SURREAL_PASS", "")]), None, base).unwrap_err();
        assert!(e.starts_with("AGENTBOARD_SURREAL_PASS is empty"), "{e}");
    }

    #[test]
    fn only_a_2_7_server_is_accepted() {
        check_version("127.0.0.1:1", "surrealdb-2.7.0").unwrap();
        check_version("127.0.0.1:1", "surrealdb-2.7.3\n").unwrap();
        for bad in [
            "surrealdb-2.6.9",
            "surrealdb-3.3.4",
            "surrealdb-2.70.0",
            "",
            "nginx",
        ] {
            let e = check_version("127.0.0.1:1", bad).unwrap_err();
            assert!(
                e.contains("want surrealdb 2.7.x") && !e.contains('\n'),
                "{bad}: {e}"
            );
        }
    }

    #[tokio::test]
    async fn block_on_works_when_called_from_inside_a_runtime() {
        // The CLI's `main` is async; `runtime().block_on` here would panic with
        // "Cannot start a runtime from within a runtime".
        assert_eq!(block_on(async { 7 }), 7);
    }

    #[test]
    fn the_pkill_hint_matches_the_command_line_the_server_is_started_with() {
        let s = Settings::defaults_for_test("/tmp/ab-hint/data", "127.0.0.1:18720");
        let line = schema_refusal(Some(99), &s);
        let pattern = line
            .split_once("pkill -f '")
            .and_then(|(_, rest)| rest.split_once('\''))
            .map(|(p, _)| p)
            .expect("the refusal names a pkill pattern");
        let cmdline = serve::server_args(&s).join(" ");
        assert!(
            cmdline.contains(pattern),
            "pkill pattern {pattern:?} does not match the server command line {cmdline:?}"
        );
    }

    // ---- tests against a real server -------------------------------------------

    struct TestServer {
        child: Child,
        settings: Settings,
        root: PathBuf,
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn scratch(tag: &str) -> PathBuf {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let d = std::env::temp_dir().join(format!("ab-surreal-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn binary() -> Option<PathBuf> {
        serve::find_binary(
            std::env::var("PATH").ok().as_deref(),
            std::env::var("HOME").ok().as_deref(),
        )
    }

    fn kill9(pid: u32) {
        Command::new("kill")
            .args(["-9", &pid.to_string()])
            .status()
            .unwrap();
    }

    fn fresh_settings(tag: &str) -> (PathBuf, Settings) {
        let root = scratch(tag);
        let s = Settings::defaults_for_test(
            root.join("data").to_str().unwrap(),
            &format!("127.0.0.1:{}", free_port()),
        );
        (root, s)
    }

    /// The `surreal` binary, or `None` after printing a SKIP line. With
    /// `AGENTBOARD_REQUIRE_SURREAL` set (and not `0`) a missing binary panics
    /// instead: the surreal host must never pass these tests by skipping them.
    fn binary_or_skip(test: &str) -> Option<PathBuf> {
        let found = binary();
        if found.is_none() {
            let why = "no `surreal` binary in PATH or ~/.surrealdb/surreal";
            if std::env::var("AGENTBOARD_REQUIRE_SURREAL").is_ok_and(|v| !v.is_empty() && v != "0")
            {
                panic!("{test}: AGENTBOARD_REQUIRE_SURREAL is set but {why}");
            }
            println!("SKIP {test}: {why}");
        }
        found
    }

    /// A throwaway server on a random port with its own data dir, or `None` after
    /// printing why the test cannot run (see `binary_or_skip`).
    fn start_server(test: &str) -> Option<TestServer> {
        let bin = binary_or_skip(test)?;
        let (root, settings) = fresh_settings(test);
        let child = Command::new(bin)
            .args(["db", "start", "--no-banner", "--log", "warn", "--bind"])
            .arg(&settings.url)
            .args(["--username", &settings.user, "--password", &settings.pass])
            .arg(format!("surrealkv://{}", settings.dir.display()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let server = TestServer {
            child,
            settings,
            root,
        };
        let t = Instant::now();
        while !serve::healthy(&server.settings.url, Duration::from_millis(300)) {
            assert!(
                t.elapsed() < Duration::from_secs(15),
                "test server never came up"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        Some(server)
    }

    fn table_names(c: &Conn) -> Vec<String> {
        runtime().block_on(async {
            let info: Option<serde_json::Value> =
                c.db.query("INFO FOR DB").await.unwrap().take(0).unwrap();
            let mut names: Vec<String> = info.unwrap()["tables"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
            names.sort();
            names
        })
    }

    #[test]
    fn bootstrap_is_idempotent() {
        let Some(srv) = start_server("idempotent") else {
            return;
        };
        let first = connect(&srv.settings).unwrap();
        assert_eq!(first.bootstrap, Bootstrap::Created);
        assert!(first.spawned_pid.is_none(), "a server was already up");
        let tables = table_names(&first);
        let second = connect(&srv.settings).unwrap();
        assert_eq!(second.bootstrap, Bootstrap::Already);
        assert_eq!(
            table_names(&second),
            tables,
            "second run changed the tables"
        );
        let marker: Vec<Marker> = runtime().block_on(async {
            second
                .db
                .query("SELECT version FROM meta:schema")
                .await
                .unwrap()
                .take(0)
                .unwrap()
        });
        assert_eq!(marker.len(), 1, "exactly one marker record");
        assert_eq!(marker[0].version, SCHEMA_VERSION);
        assert_eq!(
            tables,
            [
                "counters", "diffs", "epics", "events", "meta", "sessions", "tasks"
            ]
        );
        println!(
            "bootstrap run 1: {:?}, run 2: {:?}; tables {tables:?}; marker version {}",
            first.bootstrap, second.bootstrap, marker[0].version
        );
        // The lock file sits inside the data dir and did not upset the server.
        assert!(lock::lock_path(&srv.settings.dir).is_file());
    }

    #[test]
    fn diffs_round_trip_with_the_round_from_the_reopen_count() {
        use super::diffs::{listing, newest_diff, record, show};
        use crate::store::Ref;
        let Some(srv) = start_server("diffs_round_trip") else {
            return;
        };
        let c = connect(&srv.settings).unwrap();
        let db = &c.db;
        runtime().block_on(async {
            db.query("CREATE tasks:1 SET title = 't', created_at = 1, updated_at = 1, reopen_count = 2")
                .await
                .unwrap()
                .check()
                .unwrap();
            assert_eq!(listing(db, Ref::Task(1)).await.unwrap(), "(no diffs)");
            assert_eq!(record(db, "a", 1, "one\ntwo\n").await.unwrap(), "D1 (T1 round 3)");
            let long = "x".repeat(5000);
            assert_eq!(record(db, "a", 1, &long).await.unwrap(), "D2 (T1 round 3)");
            let l = listing(db, Ref::Task(1)).await.unwrap();
            let lines: Vec<_> = l.lines().collect();
            assert!(lines[0].starts_with("D2 round 3 ") && lines[0].ends_with(" 1 lines"), "{l}");
            assert!(lines[1].starts_with("D1 round 3 ") && lines[1].ends_with(" 2 lines"), "{l}");
            let clipped = show(db, 2, false).await.unwrap();
            assert!(clipped.contains("[truncated: 4000 of 5000 chars;"), "{clipped}");
            let full = show(db, 2, true).await.unwrap();
            assert!(full.ends_with(&long) && !full.contains("truncated"));
            assert!(show(db, 1, true).await.unwrap().ends_with("one\ntwo\n"));
            assert_eq!(newest_diff(db, 1).await.unwrap().unwrap().id, 2);
            assert!(show(db, 9, false).await.unwrap_err().contains("no such diff D9"));
            assert!(record(db, "a", 9, "x").await.unwrap_err().contains("no such task T9"));
            let mut r = db
                .query("SELECT body, meta::id(id) AS seq FROM events WHERE task_id = 1 AND kind = 'diff' ORDER BY seq LIMIT 1")
                .await
                .unwrap();
            let ev: Vec<serde_json::Value> = r.take(0).unwrap();
            let ev = ev[0]["body"].as_str().map(str::to_owned);
            assert_eq!(ev.as_deref(), Some("D1 round 3 2 lines"));
        });
    }

    #[test]
    fn the_review_gate_and_the_review_row_filed_once() {
        use super::diffs::record;
        use super::tasks::{Txn, diff_since_claim, fetch, file_review};
        let Some(srv) = start_server("review_gate") else {
            return;
        };
        let c = connect(&srv.settings).unwrap();
        let db = &c.db;
        runtime().block_on(async {
            db.query(
                "CREATE epics:1 SET title = 'e', created_at = 1, updated_at = 1; \
                 CREATE tasks:1 SET title = 'impl', path = '/p', branch = 'feat/x', priority = 7, epic_id = 1, created_at = 1, updated_at = 1; \
                 UPSERT counters:task SET n = 1",
            )
            .await
            .unwrap()
            .check()
            .unwrap();
            let claim = |tag: &'static str| async move {
                let mut tx = Txn::new();
                let one = tx.p(1);
                tx.event(&one, "a", tag, "");
                tx.run(db, "$seq").await.unwrap();
            };
            assert!(!diff_since_claim(db, 1).await.unwrap(), "no diff at all");
            record(db, "a", 1, "first").await.unwrap();
            assert!(diff_since_claim(db, 1).await.unwrap(), "no claim yet: any diff counts");
            claim("claim").await;
            assert!(!diff_since_claim(db, 1).await.unwrap(), "the old diff predates the new claim");
            record(db, "a", 1, "second").await.unwrap();
            assert!(diff_since_claim(db, 1).await.unwrap());
            claim("claim").await;
            assert!(!diff_since_claim(db, 1).await.unwrap(), "every later claim resets it");

            let implement = fetch(db, 1).await.unwrap();
            let mut tx = Txn::new();
            let review = file_review(db, &mut tx, "a", &implement, Some("the report")).await.unwrap();
            tx.run(db, "true").await.unwrap();
            assert_eq!(review, 2);
            let r = fetch(db, 2).await.unwrap();
            assert_eq!(
                (r.title.as_str(), r.status.as_str(), r.parent_id, r.epic_id),
                ("Review impl", "ready", Some(1), Some(1))
            );
            assert_eq!((r.path.as_str(), r.branch.as_str(), r.priority), ("/p", "feat/x", 7));
            assert!(r.assignee.is_none());
            assert!(r.description.contains("the report") && r.description.contains("D2"));
            let before = r.description.clone();
            record(db, "a", 1, "third").await.unwrap();
            let mut tx = Txn::new();
            assert_eq!(file_review(db, &mut tx, "a", &implement, Some("again")).await.unwrap(), 2);
            tx.run(db, "true").await.unwrap();
            let mut q = db.query("SELECT count() FROM tasks WHERE parent_id = 1 GROUP ALL").await.unwrap();
            let n: Vec<serde_json::Value> = q.take(0).unwrap();
            assert_eq!(n[0]["count"], 1, "no second review row");
            assert_eq!(fetch(db, 2).await.unwrap().description, before, "description untouched");
            let mut q = db
                .query("SELECT body FROM events WHERE task_id = 2 AND kind = 'note'")
                .await
                .unwrap();
            let notes: Vec<serde_json::Value> = q.take(0).unwrap();
            assert!(notes[0]["body"].as_str().unwrap().contains("D3"), "{notes:?}");

            // Renamed between rounds: still the same review row, and the note lands on it.
            db.query("UPDATE tasks:1 SET title = 'impl renamed'").await.unwrap().check().unwrap();
            record(db, "a", 1, "fourth").await.unwrap();
            let renamed = fetch(db, 1).await.unwrap();
            let mut tx = Txn::new();
            assert_eq!(file_review(db, &mut tx, "a", &renamed, None).await.unwrap(), 2);
            tx.run(db, "true").await.unwrap();
            let mut q = db.query("SELECT count() FROM tasks WHERE parent_id = 1 GROUP ALL").await.unwrap();
            let n: Vec<serde_json::Value> = q.take(0).unwrap();
            assert_eq!(n[0]["count"], 1, "a rename must not file a second review row");
            let mut q = db
                .query("SELECT body FROM events WHERE task_id = 2 AND kind = 'note' AND string::contains(body, 'D4')")
                .await
                .unwrap();
            let hit: Vec<serde_json::Value> = q.take(0).unwrap();
            assert_eq!(hit.len(), 1, "the note lands on the original row");
        });
    }

    #[test]
    fn a_write_session_adds_a_field_the_existing_board_lacks() {
        let Some(srv) = start_server("additive_field") else {
            return;
        };
        let c = connect(&srv.settings).unwrap();
        let fields = |c: &Conn| -> String {
            runtime().block_on(async {
                let info: Option<serde_json::Value> =
                    c.db.query("INFO FOR TABLE tasks")
                        .await
                        .unwrap()
                        .take(0)
                        .unwrap();
                info.unwrap()["fields"].to_string()
            })
        };
        assert!(fields(&c).contains("\"branch\""), "fresh board has branch");
        // Make it a board from before the field existed: marker intact, field gone.
        runtime().block_on(async {
            c.db.query(
                "REMOVE FIELD branch ON tasks; REMOVE FIELD path ON tasks; REMOVE FIELD board ON tasks; REMOVE FIELD reopen_count ON tasks; \
                 CREATE tasks:98 SET title = 'old', created_at = 1, updated_at = 1",
            )
            .await
                .unwrap()
                .check()
                .unwrap();
        });
        assert!(!fields(&c).contains("\"branch\""), "field removed");
        // A read session leaves the board alone ...
        let r = connect(&srv.settings).unwrap();
        assert_eq!(r.bootstrap, Bootstrap::Already);
        assert!(!fields(&r).contains("\"branch\""), "a read must not define");
        // ... a write session defines it, and a row then keeps the value.
        let w = session_with(
            &srv.settings,
            &serve::Timing::default(),
            None,
            lock::DEFAULT_WAIT,
            true,
        )
        .unwrap();
        assert_eq!(w.bootstrap, Bootstrap::Already);
        assert!(
            fields(&c).contains("\"branch\""),
            "write session re-defined branch"
        );
        let kept: Option<String> = runtime().block_on(async {
            w.db.query("CREATE tasks:99 SET title = 't', created_at = 1, updated_at = 1, branch = 'feat/x'")
                .await
                .unwrap()
                .check()
                .unwrap();
            let mut r =
                w.db.query("SELECT VALUE branch FROM tasks:99")
                    .await
                    .unwrap();
            r.take(0).unwrap()
        });
        assert_eq!(kept.as_deref(), Some("feat/x"));
        // The row written while the fields were absent takes an ordinary write and
        // reads back each field's default.
        let old: Vec<serde_json::Value> = runtime().block_on(async {
            w.db.query("UPDATE tasks:98 SET status = 'working'")
                .await
                .unwrap()
                .check()
                .expect("a row older than the fields accepts a write");
            let mut r =
                w.db.query("SELECT status, board, path, branch, reopen_count FROM tasks:98")
                    .await
                    .unwrap();
            r.take(0).unwrap()
        });
        assert_eq!(
            old[0],
            serde_json::json!({"status": "working", "board": "main", "path": "", "branch": "HEAD", "reopen_count": 0})
        );
    }

    #[test]
    fn surface_stamp_tracks_writers_and_refuses_a_running_stale_client() {
        let Some(srv) = start_server("surface-stamp") else {
            return;
        };
        let c = connect(&srv.settings).unwrap();
        runtime().block_on(async {
            for build in [
                "version=first tools=23 commit=one",
                "version=second tools=23 commit=two",
            ] {
                let mut tx = tasks::Txn::new();
                tx.raw("UPSERT sessions:aware SET agent='new', seen=0, status='up';");
                tx.run_as(&c.db, "true", build).await.unwrap();
                let mut r =
                    c.db.query("SELECT VALUE build FROM meta:writer")
                        .await
                        .unwrap();
                assert_eq!(r.take::<Vec<String>>(0).unwrap(), vec![build]);
            }
            c.db.query("UPSERT sessions:legacy SET agent='old', seen=0, status='up'")
                .await
                .unwrap()
                .check()
                .unwrap();
            let mut r =
                c.db.query("SELECT VALUE build FROM meta:writer")
                    .await
                    .unwrap();
            assert_eq!(
                r.take::<Vec<String>>(0).unwrap(),
                vec!["version=second tools=23 commit=two"]
            );
            let mut tx = tasks::Txn::new();
            tx.raw("UPSERT sessions:aware SET agent='new', seen=0, status='up';");
            tx.run(&c.db, "true").await.unwrap();
            let mut r =
                c.db.query("SELECT VALUE build FROM meta:writer")
                    .await
                    .unwrap();
            assert_eq!(
                r.take::<Vec<String>>(0).unwrap(),
                vec![crate::ident::stamp()]
            );
            c.db.query("UPDATE meta:schema SET version=99, build='future tools=99 commit=future'")
                .await
                .unwrap()
                .check()
                .unwrap();
        });
        // Reopening a write session is precisely what each operation of an
        // already-running MCP process does; no bootstrap or mutation may follow.
        let err = session_with(
            &srv.settings,
            &serve::Timing::default(),
            None,
            lock::DEFAULT_WAIT,
            true,
        )
        .err()
        .unwrap();
        assert!(
            err.contains("schema version 99") && err.contains("want version 2"),
            "{err}"
        );
        assert!(
            err.contains("board build=future tools=99 commit=future")
                && err.contains(&crate::ident::stamp()),
            "{err}"
        );
    }

    #[test]
    fn a_wrong_or_missing_marker_is_refused_with_the_exact_line() {
        let Some(srv) = start_server("marker") else {
            return;
        };
        let c = connect(&srv.settings).unwrap();
        runtime().block_on(async {
            c.db.query("UPDATE meta:schema SET version = 99")
                .await
                .unwrap()
                .check()
                .unwrap();
        });
        let err = connect(&srv.settings).err().unwrap();
        assert_eq!(
            err,
            format!(
                "{} board build={}; client build={}",
                schema_refusal(Some(99), &srv.settings),
                crate::ident::stamp(),
                crate::ident::stamp()
            )
        );
        println!("wrong-marker refusal: {err}");
        // tables but no marker at all: also refused, differently worded
        runtime().block_on(async {
            c.db.query("DELETE meta:schema")
                .await
                .unwrap()
                .check()
                .unwrap();
        });
        let err = connect(&srv.settings).err().unwrap();
        assert!(err.contains("found no schema marker"), "{err}");
        println!("missing-marker refusal: {err}");
    }

    #[test]
    fn a_wait_cannot_outlive_a_server_that_stops_answering() {
        let Some(srv) = start_server("waithang") else {
            return;
        };
        connect(&srv.settings).unwrap(); // bootstrap the board
        let (tx, rx) = std::sync::mpsc::channel();
        let settings = srv.settings.clone();
        std::thread::spawn(move || {
            // Waits for 30 s; every database call gets a 700 ms budget.
            let w = crate::store::board::Wait {
                ids: &[],
                epic: None,
                open: true,
                status: Some("done"),
                kind: None,
                after: None,
                timeout_ms: 30_000,
            };
            let r = block_on(wait::wait_on(&settings, w, Duration::from_millis(700)));
            let _ = tx.send(r.err());
        });
        std::thread::sleep(Duration::from_millis(500));
        // Freeze the server: the WebSocket stays open and silent, as after a crash.
        let pid = srv.child.id().to_string();
        Command::new("kill").args(["-STOP", &pid]).status().unwrap();
        let started = Instant::now();
        let got = rx.recv_timeout(Duration::from_secs(6));
        Command::new("kill").args(["-CONT", &pid]).status().unwrap();
        let err = got
            .expect("the wait must return within seconds, not outlive its 30 s timeout")
            .expect("a wait against a frozen server must fail");
        println!(
            "wait on a frozen server, after {:?}: {err}",
            started.elapsed()
        );
        assert!(
            err.starts_with(
                "board server unavailable: did not answer within 700 ms while waiting for `surreal db start"
            ),
            "{err}"
        );
        assert!(
            err.contains("Install: curl -sSf https://install.surrealdb.com"),
            "{err}"
        );
        assert!(!err.contains('\n'));
    }

    #[test]
    fn auto_serve_starts_a_server_and_a_killed_one_gives_one_line() {
        if binary_or_skip("auto_serve").is_none() {
            return;
        }
        let (root, s) = fresh_settings("autoserve");
        // Nothing answers on this fresh port, so connect starts a server itself.
        let c = connect(&s).expect("auto-serve should start a server and connect");
        assert_eq!(c.bootstrap, Bootstrap::Created);
        let pid = c.spawned_pid.expect("connect spawned the server");
        println!("auto-serve: spawned surreal pid {pid} on {}", s.url);
        // Already up now: a second connect must not spawn.
        let again = connect(&s).unwrap();
        assert!(again.spawned_pid.is_none());
        // Kill the server. With a binary that cannot start, connect is one line.
        kill9(pid);
        let t = Instant::now();
        while serve::healthy(&s.url, Duration::from_millis(200)) {
            assert!(t.elapsed() < Duration::from_secs(5), "server did not die");
            std::thread::sleep(Duration::from_millis(50));
        }
        let quick = serve::Timing {
            probe: Duration::from_millis(100),
            poll_every: Duration::from_millis(20),
            deadline: Duration::from_millis(500),
            connect: Duration::from_millis(500),
        };
        let err = connect_with(
            &s,
            &quick,
            Some(Path::new("/nonexistent/surreal")),
            Duration::from_secs(1),
        )
        .err()
        .unwrap();
        println!("server-absent line (after kill): {err}");
        assert!(
            err.starts_with("board server unavailable: spawn /nonexistent/surreal"),
            "{err}"
        );
        assert!(
            err.contains("Install: curl -sSf https://install.surrealdb.com"),
            "{err}"
        );
        assert!(!err.contains('\n'));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn two_starters_race_and_both_end_up_connected() {
        if binary_or_skip("race").is_none() {
            return;
        }
        let (root, s) = fresh_settings("race");
        let hs: Vec<_> = (0..2)
            .map(|_| {
                let s = s.clone();
                std::thread::spawn(move || {
                    serve::ensure_up(&s, &serve::Timing::default(), None)
                        .map(|served| served.spawned_pid)
                })
            })
            .collect();
        let pids: Vec<Option<u32>> = hs.into_iter().map(|h| h.join().unwrap().unwrap()).collect();
        println!("race: both starters returned Ok; spawned pids {pids:?}");
        assert_eq!(
            pids.iter().flatten().count(),
            1,
            "exactly one starter spawns; the other waits and finds it up"
        );
        assert!(serve::healthy(&s.url, Duration::from_millis(500)));
        let c = connect(&s).unwrap();
        assert!(matches!(c.bootstrap, Bootstrap::Created));
        for p in pids.into_iter().flatten() {
            kill9(p);
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Answers /health and /version like a real server, accepts the WebSocket
    /// connection and then says nothing, as a wedged server would.
    fn wedged_server() -> (
        u16,
        std::thread::JoinHandle<()>,
        std::sync::mpsc::Sender<()>,
    ) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (stop, stopped) = std::sync::mpsc::channel::<()>();
        let h = std::thread::spawn(move || {
            let mut held = Vec::new();
            while stopped.try_recv().is_err() {
                let Ok((mut c, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                c.set_nonblocking(false).unwrap();
                c.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut buf = [0u8; 2048];
                let n = c.read(&mut buf).unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).into_owned();
                let body = if head.starts_with("GET /health") {
                    Some("OK")
                } else if head.starts_with("GET /version") {
                    Some("surrealdb-2.7.0")
                } else {
                    None
                };
                match body {
                    Some(b) => {
                        let _ = write!(
                            c,
                            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{b}",
                            b.len()
                        );
                    }
                    None => held.push(c), // the upgrade: never answered
                }
            }
        });
        (port, h, stop)
    }

    #[test]
    fn a_server_that_wedges_after_health_cannot_hang_connect() {
        let (port, h, stop) = wedged_server();
        let root = scratch("wedged");
        let s = Settings::defaults_for_test(
            root.join("data").to_str().unwrap(),
            &format!("127.0.0.1:{port}"),
        );
        let t = serve::Timing {
            probe: Duration::from_millis(200),
            poll_every: Duration::from_millis(20),
            deadline: Duration::from_millis(500),
            connect: Duration::from_millis(700),
        };
        // Run it on a thread so a missing timeout fails the test instead of hanging it.
        let (tx, rx) = std::sync::mpsc::channel();
        let s2 = s.clone();
        std::thread::spawn(move || {
            let r = connect_with(&s2, &t, None, Duration::from_secs(1));
            let _ = tx.send(r.err());
        });
        let started = Instant::now();
        let err = rx
            .recv_timeout(Duration::from_secs(3))
            .expect("connect must return within 3 s against a wedged server")
            .expect("connect cannot succeed against a server that never upgrades");
        println!("wedged-handshake line after {:?}: {err}", started.elapsed());
        assert!(
            err.starts_with("board server unavailable: did not complete the WebSocket handshake within 700 ms for `surreal db start"),
            "{err}"
        );
        assert!(
            err.contains("Install: curl -sSf https://install.surrealdb.com"),
            "{err}"
        );
        assert!(!err.contains('\n'));
        let _ = stop.send(());
        let _ = h.join();
        let _ = std::fs::remove_dir_all(&root);
    }
}
