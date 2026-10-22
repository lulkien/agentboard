//! The board seam. `Board` is every operation `main.rs` and `server.rs` call, with
//! the argument and return types they use. One implementation is compiled per
//! build: `sqlite::SqliteBoard` or `surreal::SurrealBoard`.

use super::text::SessionRow;

/// A task as the caller describes it; `add` supplies id, timestamps and lease.
pub struct NewTask<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub goal: &'a str,
    pub status: &'a str,
    pub assignee: Option<&'a str>,
    pub priority: i64,
    pub epic: Option<i64>,
    pub parent: Option<i64>,
    /// Project root; normalised like an epic's. Empty or absent = not repo-bound.
    pub path: Option<&'a str>,
    /// Git ref the worker starts on; empty or absent = `HEAD`.
    pub branch: Option<&'a str>,
}

/// New spec text for a task; `None` leaves a field alone. An empty `assignee`
/// or `path` clears it; an empty `branch` means `HEAD`.
pub struct TaskEdit<'a> {
    pub epic: Option<i64>,
    pub title: Option<&'a str>,
    pub description: Option<&'a str>,
    pub goal: Option<&'a str>,
    pub priority: Option<i64>,
    pub assignee: Option<&'a str>,
    pub path: Option<&'a str>,
    pub branch: Option<&'a str>,
}

/// Why a claim failed. `Held` is the legitimate refusal (somebody holds the
/// task); the MCP layer reports it differently from a bad request or a DB error.
pub enum ClaimError {
    Held(String),
    Other(String),
}

impl From<String> for ClaimError {
    fn from(e: String) -> Self {
        Self::Other(e)
    }
}

impl std::fmt::Display for ClaimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (Self::Held(m) | Self::Other(m)) = self;
        f.write_str(m)
    }
}

/// One transactional edit of a task. Status moves, report, note, commit and
/// lease all land together or not at all.
pub struct Patch<'a> {
    pub edit: Option<&'a TaskEdit<'a>>,
    pub status: Option<&'a str>,
    pub report: Option<&'a str>,
    pub note: Option<&'a str>,
    pub commit: Option<&'a str>,
    pub needs: Option<&'a str>,
    pub ttl_s: i64,
}

/// New spec text for an epic; `None` leaves a field alone. An empty `path`
/// clears it.
pub struct EpicEdit<'a> {
    pub status: Option<&'a str>,
    pub note: Option<&'a str>,
    pub title: Option<&'a str>,
    pub description: Option<&'a str>,
    pub goal: Option<&'a str>,
    pub path: Option<&'a str>,
}

/// What `wait` returned. `text` starts with `matched:` or `timeout:`.
pub struct Outcome {
    pub text: String,
    pub timed_out: bool,
}

pub struct Wait<'a> {
    pub ids: &'a [String],
    pub epic: Option<&'a str>,
    pub open: bool,
    /// CSV of statuses; the task's CURRENT status must be in it.
    pub status: Option<&'a str>,
    /// CSV of event kinds.
    pub kind: Option<&'a str>,
    pub after: Option<i64>,
    pub timeout_ms: u64,
}

/// Board operations. Methods return the rendered text the adapters print, as the
/// free functions in `store` always did. `wait` is async because it sleeps
/// between polls; the rest are sync.
pub trait Board {
    fn add(&self, actor: &str, task: &NewTask<'_>) -> Result<String, String>;
    fn edit(&self, actor: &str, id: i64, e: &TaskEdit<'_>) -> Result<String, ClaimError>;
    fn set_parent(&self, actor: &str, id: i64, parent: Option<i64>) -> Result<String, String>;
    fn claim(&self, actor: &str, id: Option<i64>, ttl_s: i64) -> Result<String, ClaimError>;
    fn apply(&self, actor: &str, id: i64, p: &Patch<'_>) -> Result<String, String>;
    fn note(&self, actor: &str, id: i64, body: &str) -> Result<String, String>;
    fn report(&self, actor: &str, id: i64, body: &str) -> Result<String, String>;
    fn ask(&self, actor: &str, id: i64, question: &str) -> Result<String, String>;
    fn answer(&self, actor: &str, id: i64, text: &str) -> Result<String, String>;
    fn finish(
        &self,
        actor: &str,
        id: i64,
        status: &str,
        summary: &str,
        commit: Option<&str>,
    ) -> Result<String, String>;
    fn block(&self, actor: &str, id: i64, reason: &str) -> Result<String, String>;
    fn heartbeat(&self, actor: &str, id: i64, ttl_s: i64) -> Result<String, String>;

    fn list(
        &self,
        status: Option<&str>,
        assignee: Option<&str>,
        limit: usize,
    ) -> Result<String, String>;
    fn get(&self, id: i64, full: bool) -> Result<String, String>;
    fn children(&self, of: super::Ref) -> Result<String, String>;
    fn brief(&self, since_min: i64) -> Result<String, String>;

    /// Records a diff for a task: append-only, round = the task's `reopen_count + 1`.
    /// `reviews` names a diff: the row is then an evidence-only submission for a
    /// re-run round, never a patch.
    fn diff_add(
        &self,
        actor: &str,
        task: i64,
        body: &str,
        reviews: Option<i64>,
    ) -> Result<String, String>;
    fn diff_get(&self, id: i64, full: bool) -> Result<String, String>;
    /// A task's diffs, or every diff of an epic's tasks, newest first.
    fn diff_list(&self, of: super::Ref) -> Result<String, String>;

    fn epic_add(
        &self,
        actor: &str,
        title: &str,
        description: &str,
        goal: &str,
        path: Option<&str>,
    ) -> Result<String, String>;
    fn epic_list(&self, status: Option<&str>) -> Result<String, String>;
    fn epic_get(&self, id: i64, full: bool) -> Result<String, String>;
    fn epic_status(
        &self,
        actor: &str,
        id: i64,
        status: &str,
        note: Option<&str>,
    ) -> Result<String, String>;
    fn epic_edit(&self, actor: &str, id: i64, e: &EpicEdit<'_>) -> Result<String, String>;

    fn sessions(&self) -> Result<String, String>;
    fn session_rows(&self) -> Result<Vec<SessionRow>, String>;
    fn prune(&self, dry_run: bool) -> Result<String, String>;
    fn unregister(&self, name: &str) -> Result<String, String>;
    fn register_session(
        &self,
        name: &str,
        agent: &str,
        cwd: &str,
        task: Option<i64>,
    ) -> Result<String, String>;
    fn tail(&self, name: &str, lines: usize) -> Result<String, String>;
    fn send(&self, name: &str, text: &str) -> Result<String, String>;

    fn dispatch(
        &self,
        actor: &str,
        id: i64,
        to: Option<&str>,
        force: bool,
        print_only: bool,
    ) -> Result<String, String>;
    async fn wait(&self, w: Wait<'_>) -> Result<Outcome, String>;
}
