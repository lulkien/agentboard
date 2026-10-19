//! MCP surface: 11 tools, text-only results, short acks.
//!
//! Result shape is deliberate: reads are one line per task, writes answer with
//! `ok T7 …` instead of echoing the record. Agents pay tokens per byte.

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::schemars::JsonSchema;
use rmcp::{ErrorData, ServerHandler, tool, tool_handler, tool_router};
use serde::Deserialize;

use crate::store;

const NAME: &str = "agentboard";
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn actor() -> String {
    store::default_actor()
}

fn text(s: String) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(s)])
}

fn err(e: String) -> ErrorData {
    ErrorData::invalid_params(e, None)
}

/// Runs a blocking store call (tmux subprocess) off the async runtime threads.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f).await.map_err(|e| format!("task join: {e}"))?
}

macro_rules! run {
    ($e:expr) => {
        $e.map(text).map_err(err)
    };
}

#[derive(Clone)]
pub struct Board {
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddArgs {
    /// One-line task title.
    pub title: String,
    /// Everything the worker needs: repo path, files, situation, constraints.
    #[serde(default)]
    pub description: String,
    /// How the work is done and how it is judged finished (definition of done).
    #[serde(default)]
    pub goal: String,
    /// Initial status (default ready).
    #[serde(default)]
    pub status: Option<String>,
    /// Who should do it (profile or tmux session name).
    #[serde(default)]
    pub assignee: Option<String>,
    /// Lower runs first. Default 100.
    #[serde(default)]
    pub priority: Option<i64>,
    /// Parent task id, e.g. T3.
    #[serde(default)]
    pub parent: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListArgs {
    /// ready|working|blocking|review|done|dropped. Omit for all open work.
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub assignee: Option<String>,
    /// Max rows (default 20).
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct IdArgs {
    /// Task id, T7 or 7.
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClaimArgs {
    /// Specific task, or omit to take the next ready one.
    #[serde(default)]
    pub id: Option<String>,
    /// Lease minutes; expired leases are reclaimable by others (default 30).
    #[serde(default)]
    pub ttl_min: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateArgs {
    pub id: String,
    /// ready|working|blocking|review|done|dropped. Omit to keep the current one.
    #[serde(default)]
    pub status: Option<String>,
    /// Worker's findings or outcome. Stored as the task report.
    #[serde(default)]
    pub report: Option<String>,
    /// Free-form progress note / decision / question answer.
    #[serde(default)]
    pub note: Option<String>,
    /// Commit or PR reference.
    #[serde(default)]
    pub commit: Option<String>,
    /// Refresh the claim lease (holder only, while working). Combines with other fields.
    #[serde(default)]
    pub heartbeat: bool,
    /// Lease minutes for heartbeat or a move to working (default 30, max 720).
    #[serde(default)]
    pub ttl_min: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AskArgs {
    pub id: String,
    /// The question. Sets status=blocking, needs=input until someone answers.
    pub question: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WaitArgs {
    pub id: String,
    /// Server-side long-poll in seconds (default 60, max 300). No client polling.
    #[serde(default)]
    pub timeout_s: Option<u64>,
    /// Last event seq you already saw (#N). Omit to wait for the next event.
    #[serde(default)]
    pub after_seq: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BriefArgs {
    /// Window for the "done" list, minutes (default 120).
    #[serde(default)]
    pub since_min: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TailArgs {
    /// tmux session name.
    pub name: String,
    /// Lines of scrollback (default 40, max 400).
    #[serde(default)]
    pub lines: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SendArgs {
    pub name: String,
    /// Text typed into the pane, followed by Enter.
    pub text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RegisterArgs {
    pub name: String,
    /// claude|hermes|human
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
}

#[tool_router]
impl Board {
    #[must_use]
    pub fn new() -> Self {
        Self { tool_router: Self::tool_router() }
    }

    #[tool(name = "task_add", description = "Create a task. description holds everything the worker needs (repo path, files, situation); goal holds how to do it and the definition of done. Returns its short id (T7).", annotations(title = "Add task", read_only_hint = false, destructive_hint = false))]
    async fn task_add(&self, Parameters(a): Parameters<AddArgs>) -> Result<CallToolResult, ErrorData> {
        let parent = match a.parent.as_deref() {
            Some(p) => Some(store::parse_id(p).map_err(err)?),
            None => None,
        };
        let status = match a.status.as_deref() {
            Some(raw) => store::norm(raw).ok_or_else(|| err(format!("bad status {raw:?}")))?,
            None => "ready",
        };
        run!(store::add(&actor(), &a.title, &a.description, &a.goal, status, a.assignee.as_deref(), a.priority.unwrap_or(100), parent))
    }

    #[tool(name = "task_list", description = "Open tasks, one compact line each: id, status, assignee, title.", annotations(title = "List tasks", read_only_hint = true))]
    async fn task_list(&self, Parameters(a): Parameters<ListArgs>) -> Result<CallToolResult, ErrorData> {
        run!(store::list(a.status.as_deref(), a.assignee.as_deref(), a.limit.unwrap_or(20).clamp(1, 200)))
    }

    #[tool(name = "task_get", description = "Full task record: status, timestamps, description, goal, report, note, recent events. Manager reads this to collect a worker's result.", annotations(title = "Get task", read_only_hint = true))]
    async fn task_get(&self, Parameters(a): Parameters<IdArgs>) -> Result<CallToolResult, ErrorData> {
        let id = store::parse_id(&a.id).map_err(err)?;
        run!(store::get(id))
    }

    #[tool(name = "task_claim", description = "Atomically claim a task (or the next ready one) and return its description + goal. Claim expires after ttl_min; heartbeat to extend.", annotations(title = "Claim task", read_only_hint = false, destructive_hint = false))]
    async fn task_claim(&self, Parameters(a): Parameters<ClaimArgs>) -> Result<CallToolResult, ErrorData> {
        let id = match a.id.as_deref() {
            Some(s) => Some(store::parse_id(s).map_err(err)?),
            None => None,
        };
        let ttl = a.ttl_min.unwrap_or(30).clamp(1, 720);
        run!(store::claim(&actor(), id, ttl * 60))
    }

    #[tool(name = "task_update", description = "Write back to the board: status (ready|working|blocking|review|done|dropped), report (your findings/outcome), note (progress), commit. Use status=done when finished. Returns a short ack.", annotations(title = "Update task", read_only_hint = false, destructive_hint = false))]
    async fn task_update(&self, Parameters(a): Parameters<UpdateArgs>) -> Result<CallToolResult, ErrorData> {
        let id = store::parse_id(&a.id).map_err(err)?;
        let who = actor();
        let ttl_s = a.ttl_min.unwrap_or(30).clamp(1, 720) * 60;
        let beat = if a.heartbeat { Some(store::heartbeat(&who, id, ttl_s).map_err(err)?) } else { None };
        if a.status.is_none() && a.report.is_none() && a.note.is_none() && a.commit.is_none() {
            return match beat {
                Some(ack) => Ok(text(ack)),
                None => Err(err("nothing to do: pass status, report, note, commit or heartbeat".to_owned())),
            };
        }
        let patch = store::Patch {
            status: a.status.as_deref(),
            report: a.report.as_deref(),
            note: a.note.as_deref(),
            commit: a.commit.as_deref(),
            needs: None,
            ttl_s,
        };
        run!(store::apply(&who, id, &patch))
    }

    #[tool(name = "task_ask", description = "Block on a human decision. Sets status=blocking/needs=input; the answer arrives as an event. Follow with task_wait, passing the #seq from the ack as after_seq. Ask ONE narrow question per call: compound questions get partial answers.", annotations(title = "Ask and block", read_only_hint = false, destructive_hint = false))]
    async fn task_ask(&self, Parameters(a): Parameters<AskArgs>) -> Result<CallToolResult, ErrorData> {
        let id = store::parse_id(&a.id).map_err(err)?;
        run!(store::ask(&actor(), id, &a.question))
    }

    #[tool(name = "task_wait", description = "Server-side long-poll: returns as soon as the task gets an answer or new event. Prefer this over repeated task_get.", annotations(title = "Wait for change", read_only_hint = true))]
    async fn task_wait(&self, Parameters(a): Parameters<WaitArgs>) -> Result<CallToolResult, ErrorData> {
        let id = store::parse_id(&a.id).map_err(err)?;
        let ms = a.timeout_s.unwrap_or(60).clamp(1, 300) * 1000;
        run!(store::wait(id, a.after_seq, ms).await)
    }

    #[tool(name = "board_brief", description = "Team status in one screen: counts, open tasks ordered by urgency, recent completions. Use for standup/reporting.", annotations(title = "Board brief", read_only_hint = true, idempotent_hint = true))]
    async fn board_brief(&self, Parameters(a): Parameters<BriefArgs>) -> Result<CallToolResult, ErrorData> {
        run!(store::brief(a.since_min.unwrap_or(120).clamp(1, 20_160)))
    }

    #[tool(name = "session_list", description = "Registered agent sessions (tmux) and what task each holds.", annotations(title = "List sessions", read_only_hint = true))]
    async fn session_list(&self, Parameters(_): Parameters<Empty>) -> Result<CallToolResult, ErrorData> {
        run!(blocking(store::sessions).await)
    }

    #[tool(name = "session_register", description = "Register this session's tmux name so the board can map task -> pane.", annotations(title = "Register session", read_only_hint = false, destructive_hint = false))]
    async fn session_register(&self, Parameters(a): Parameters<RegisterArgs>) -> Result<CallToolResult, ErrorData> {
        let task = match a.task_id.as_deref() {
            Some(t) => Some(store::parse_id(t).map_err(err)?),
            None => None,
        };
        run!(store::register_session(&a.name, a.agent.as_deref().unwrap_or("agent"), a.cwd.as_deref().unwrap_or(""), task))
    }

    #[tool(name = "session_tail", description = "Last N lines of a tmux pane. Cheap way to see what an agent is actually doing.", annotations(title = "Tail pane", read_only_hint = true))]
    async fn session_tail(&self, Parameters(a): Parameters<TailArgs>) -> Result<CallToolResult, ErrorData> {
        let lines = a.lines.unwrap_or(40).clamp(1, 400);
        run!(blocking(move || store::tail(&a.name, lines)).await)
    }

    #[tool(name = "session_send", description = "Type a line into a tmux pane and press Enter (steer another agent).", annotations(title = "Send keys", read_only_hint = false, destructive_hint = true))]
    async fn session_send(&self, Parameters(a): Parameters<SendArgs>) -> Result<CallToolResult, ErrorData> {
        run!(blocking(move || store::send(&a.name, &a.text)).await)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Empty {}

impl Default for Board {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Board {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::new(ServerCapabilities::builder().enable_tools().build());
        info.server_info = Implementation::new(NAME, VERSION);
        info
    }
}
