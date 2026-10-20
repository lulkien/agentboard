//! A `Board` whose every operation returns the stub error. It backs the surreal
//! build until its operations are ported, and gives `BACKEND` a value in the two
//! unsupported feature sets, so the `compile_error!` in `main.rs` is the only error.

use super::Ref;
use super::board::{Board, ClaimError, EpicEdit, NewTask, Outcome, Patch, TaskEdit, Wait};

const STUB: &str = "surreal backend is not implemented yet: this build has the board seam only (see docs/surrealdb-migration.md)";

fn stub<T>() -> Result<T, String> {
    Err(STUB.to_owned())
}

pub struct StubBoard;

impl Board for StubBoard {
    fn add(&self, _actor: &str, _task: &NewTask<'_>) -> Result<String, String> {
        stub()
    }
    fn edit(&self, _actor: &str, _id: i64, _e: &TaskEdit<'_>) -> Result<String, ClaimError> {
        Err(ClaimError::Other(STUB.to_owned()))
    }
    fn set_parent(&self, _actor: &str, _id: i64, _parent: Option<i64>) -> Result<String, String> {
        stub()
    }
    fn claim(&self, _actor: &str, _id: Option<i64>, _ttl_s: i64) -> Result<String, ClaimError> {
        Err(ClaimError::Other(STUB.to_owned()))
    }
    fn apply(&self, _actor: &str, _id: i64, _p: &Patch<'_>) -> Result<String, String> {
        stub()
    }
    fn note(&self, _actor: &str, _id: i64, _body: &str) -> Result<String, String> {
        stub()
    }
    fn report(&self, _actor: &str, _id: i64, _body: &str) -> Result<String, String> {
        stub()
    }
    fn ask(&self, _actor: &str, _id: i64, _question: &str) -> Result<String, String> {
        stub()
    }
    fn answer(&self, _actor: &str, _id: i64, _text: &str) -> Result<String, String> {
        stub()
    }
    fn finish(
        &self,
        _actor: &str,
        _id: i64,
        _status: &str,
        _summary: &str,
        _commit: Option<&str>,
    ) -> Result<String, String> {
        stub()
    }
    fn block(&self, _actor: &str, _id: i64, _reason: &str) -> Result<String, String> {
        stub()
    }
    fn heartbeat(&self, _actor: &str, _id: i64, _ttl_s: i64) -> Result<String, String> {
        stub()
    }

    fn list(
        &self,
        _status: Option<&str>,
        _assignee: Option<&str>,
        _limit: usize,
    ) -> Result<String, String> {
        stub()
    }
    fn get(&self, _id: i64, _full: bool) -> Result<String, String> {
        stub()
    }
    fn children(&self, _of: Ref) -> Result<String, String> {
        stub()
    }
    fn brief(&self, _since_min: i64) -> Result<String, String> {
        stub()
    }

    fn epic_add(
        &self,
        _actor: &str,
        _title: &str,
        _description: &str,
        _goal: &str,
        _path: Option<&str>,
    ) -> Result<String, String> {
        stub()
    }
    fn epic_list(&self, _status: Option<&str>) -> Result<String, String> {
        stub()
    }
    fn epic_get(&self, _id: i64, _full: bool) -> Result<String, String> {
        stub()
    }
    fn epic_status(
        &self,
        _actor: &str,
        _id: i64,
        _status: &str,
        _note: Option<&str>,
    ) -> Result<String, String> {
        stub()
    }
    fn epic_edit(&self, _actor: &str, _id: i64, _e: &EpicEdit<'_>) -> Result<String, String> {
        stub()
    }

    fn sessions(&self) -> Result<String, String> {
        stub()
    }
    fn prune(&self, _dry_run: bool) -> Result<String, String> {
        stub()
    }
    fn unregister(&self, _name: &str) -> Result<String, String> {
        stub()
    }
    fn register_session(
        &self,
        _name: &str,
        _agent: &str,
        _cwd: &str,
        _task: Option<i64>,
    ) -> Result<String, String> {
        stub()
    }
    fn tail(&self, _name: &str, _lines: usize) -> Result<String, String> {
        stub()
    }
    fn send(&self, _name: &str, _text: &str) -> Result<String, String> {
        stub()
    }

    fn dispatch(
        &self,
        _actor: &str,
        _id: i64,
        _to: Option<&str>,
        _force: bool,
        _print_only: bool,
    ) -> Result<String, String> {
        stub()
    }
    async fn wait(&self, _w: Wait<'_>) -> Result<Outcome, String> {
        stub()
    }
}
