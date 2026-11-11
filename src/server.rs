//! MCP surface: 12 tools, text-only results, short acknowledgements.
//!
//! Result shape is deliberate: reads are one line per task, writes answer with the
//! changed record plus its event cursor. Agents pay tokens per byte.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, Extensions, Implementation, ServerCapabilities, ServerConfig,
};
use rmcp::schemars::JsonSchema;
use rmcp::{ErrorData, ServerHandler, tool, tool_handler, tool_router};
use serde::Deserialize;

use crate::domain::{self, EventKind, NewTask, Status, TaskPatch};
use crate::render;
use crate::store::{
    Board as BoardHandle, ClaimOutcome, Error as StoreError, FeedFilter, ListFilter, UpdateInput,
};

const NAME: &str = "agentboard";
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Resolve the adapter's project scope without making the store depend on
/// process environment or the caller's working directory.
pub fn resolve_project(
    board: &BoardHandle,
    explicit: Option<u64>,
) -> crate::store::Result<Option<u64>> {
    if explicit.is_some() {
        return Ok(explicit);
    }
    if let Ok(raw) = std::env::var("AGENTBOARD_PROJECT") {
        let id = raw
            .parse::<u64>()
            .map_err(|_| StoreError::InvalidProjectEnv(raw.clone()))?;
        return Ok(Some(id));
    }
    let Ok(cwd) = std::env::current_dir() else {
        return Ok(None);
    };
    let cwd = normalized_path(&cwd);
    let projects = board.list_projects()?;
    Ok(projects.into_iter().find_map(|project| {
        (normalized_path(Path::new(&project.project_path)) == cwd).then_some(project.project_id)
    }))
}

/// Identity and project of one HTTP request. The daemon puts it in the request
/// extensions; stdio has none and falls back to `--actor`, the environment and cwd.
#[derive(Clone, Debug)]
pub struct RequestScope {
    pub actor: String,
    pub project: Option<u64>,
}

fn request_scope(extensions: &Extensions) -> Option<RequestScope> {
    extensions
        .get::<http::request::Parts>()?
        .extensions
        .get::<RequestScope>()
        .cloned()
}

/// Project for a call: an HTTP request never reads the daemon's environment or
/// working directory, only its argument, then the project on its URL.
fn call_project(
    board: &BoardHandle,
    explicit: Option<u64>,
    scope: Option<&RequestScope>,
) -> crate::store::Result<Option<u64>> {
    match scope {
        Some(scope) => Ok(explicit.or(scope.project)),
        None => resolve_project(board, explicit),
    }
}

fn normalized_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn text(body: String) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(body)])
}

fn err(message: String) -> ErrorData {
    ErrorData::invalid_params(message, None)
}

/// Store work is synchronous and LMDB serialises writers, so every call runs off
/// the async runtime threads.
impl From<StoreError> for ErrorData {
    fn from(error: StoreError) -> Self {
        if error.is_refusal() {
            Self::invalid_params(error.to_string(), None)
        } else {
            Self::internal_error(error.to_string(), None)
        }
    }
}

async fn blocking<T, E>(
    board: &Arc<BoardHandle>,
    f: impl FnOnce(&BoardHandle) -> Result<T, E> + Send + 'static,
) -> Result<T, ErrorData>
where
    T: Send + 'static,
    E: Into<ErrorData> + Send + 'static,
{
    let board = Arc::clone(board);
    tokio::task::spawn_blocking(move || f(&board))
        .await
        .map_err(|error| ErrorData::internal_error(format!("task join: {error}"), None))?
        .map_err(Into::into)
}

fn actor_from_env() -> Option<String> {
    std::env::var("AGENTBOARD_ACTOR")
        .ok()
        .filter(|name| !name.trim().is_empty())
}

#[derive(Clone)]
pub struct Board {
    tool_router: ToolRouter<Self>,
    board: Arc<BoardHandle>,
    /// Set by `agentboard serve --actor`; otherwise the environment decides.
    actor: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddArgs {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub goal: String,
    /// todo | planning | implementing | blocked | reviewing | done | canceled
    #[serde(default)]
    pub status: Option<String>,
    #[schemars(description = "Lower values run first.")]
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub parent: Option<u64>,
    #[serde(default)]
    pub project: Option<u64>,
    /// Register an unknown target assignee in this transaction
    #[serde(default)]
    pub new_actor: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListArgs {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub parent: Option<u64>,
    #[serde(default)]
    pub project: Option<u64>,
    /// Include done and canceled rows
    #[serde(default)]
    pub all: bool,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BriefArgs {
    /// Restrict the summary to this project ID
    #[serde(default)]
    pub project: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetArgs {
    pub id: u64,
    /// Print the full description and goal
    #[serde(default)]
    pub full: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EpicArgs {
    pub id: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProjectDeleteArgs {
    pub id: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AgentDeleteArgs {
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateArgs {
    pub id: u64,
    /// The revision the caller last read; a stale value refuses
    pub if_revision: u64,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub goal: Option<String>,
    /// Append a section to the analyst report
    #[serde(default)]
    pub analyst_report: Option<String>,
    /// Append a section to the coder report
    #[serde(default)]
    pub coder_report: Option<String>,
    /// Append a section to the reviewer report
    #[serde(default)]
    pub reviewer_report: Option<String>,
    /// Rewrite the given reports instead of appending
    #[serde(default)]
    pub replace_report: bool,
    #[serde(default)]
    pub status: Option<String>,
    #[schemars(description = "Lower values run first.")]
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub assignee: Option<String>,
    /// Clear the assignee explicitly
    #[serde(default)]
    pub clear_assignee: bool,
    #[serde(default)]
    pub parent: Option<u64>,
    #[serde(default)]
    pub clear_parent: bool,
    #[serde(default)]
    pub project: Option<u64>,
    #[serde(default)]
    pub clear_project: bool,
    /// Return the row to todo and clear its owner, in one write
    #[serde(default)]
    pub release: bool,
    /// Override another actor's protected task
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub new_actor: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClaimArgs {
    #[serde(default)]
    pub id: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CommentArgs {
    pub id: u64,
    pub body: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FeedArgs {
    #[schemars(description = "Event cursor as <instance>:<seq>; 0 replays history.")]
    #[serde(default)]
    pub after: Option<String>,
    #[schemars(description = "Event kind: created, updated, claimed, or commented.")]
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub recipient: Option<String>,
    #[serde(default)]
    pub task: Option<u64>,
    #[serde(default)]
    pub limit: Option<usize>,
    /// Block up to this many seconds for a matching event
    #[serde(default)]
    pub wait: Option<u64>,
    /// Return comment and update text untruncated
    #[serde(default)]
    pub full: bool,
}

#[tool_router]
impl Board {
    pub fn new(board: BoardHandle, actor: Option<String>) -> Self {
        Self::shared(Arc::new(board), actor)
    }

    pub fn shared(board: Arc<BoardHandle>, actor: Option<String>) -> Self {
        Self {
            tool_router: Self::tool_router(),
            board,
            actor,
        }
    }

    /// The actor this invocation writes as: the HTTP request's URL name, else
    /// `--actor`, which wins over the environment so a wrapper that passes a flag
    /// is not silently overridden by an exported variable.
    fn actor_name(&self, scope: Option<&RequestScope>) -> Result<String, String> {
        scope
            .map(|scope| scope.actor.clone())
            .or_else(|| self.actor.clone())
            .filter(|name| !name.trim().is_empty())
            .or_else(actor_from_env)
            .ok_or_else(|| {
                "actor required: start the server with --actor or set AGENTBOARD_ACTOR".to_owned()
            })
    }

    #[tool(
        name = "task_add",
        description = "Create a task; returns its record and event cursor"
    )]
    async fn task_add(
        &self,
        Parameters(args): Parameters<AddArgs>,
        extensions: Extensions,
    ) -> Result<CallToolResult, ErrorData> {
        let scope = request_scope(&extensions);
        let actor = self.actor_name(scope.as_ref()).map_err(err)?;
        let status = match args.status.as_deref() {
            Some(raw) => Status::parse(raw).map_err(|e| err(e.to_string()))?,
            None => Status::Todo,
        };
        let input = NewTask {
            title: args.title,
            description: args.description,
            goal: args.goal,
            status,
            priority: args.priority.unwrap_or(100),
            assignee: args.assignee,
            parent_id: args.parent,
            project_id: args.project,
        };
        let (added, cursor) = blocking(&self.board, move |board| -> crate::store::Result<_> {
            let added = board.add(&actor, input, args.new_actor, domain::now_ms())?;
            let instance = board.metadata()?.instance_id;
            let cursor = format!("{instance}:{}", added.event.seq);
            Ok((added, cursor))
        })
        .await?;
        Ok(text(format!(
            "ok\n{}\ncursor {}",
            render::task_record(&added.task, false),
            cursor
        )))
    }

    #[tool(
        name = "task_list",
        description = "List tasks by status, assignee, parent or project",
        annotations(read_only_hint = true)
    )]
    async fn task_list(
        &self,
        Parameters(args): Parameters<ListArgs>,
        extensions: Extensions,
    ) -> Result<CallToolResult, ErrorData> {
        let scope = request_scope(&extensions);
        let status = match args.status.as_deref() {
            Some(raw) => Some(Status::parse(raw).map_err(|e| err(e.to_string()))?),
            None => None,
        };
        let ListArgs {
            assignee,
            parent,
            project,
            all,
            limit,
            offset,
            ..
        } = args;
        let page = blocking(&self.board, move |board| {
            let project = call_project(board, project, scope.as_ref())?;
            board.list(&ListFilter {
                status,
                assignee: assignee.as_deref(),
                parent,
                project,
                include_terminal: all,
                limit: limit.unwrap_or(20),
                offset: offset.unwrap_or(0),
            })
        })
        .await?;
        let mut lines: Vec<String> = page.tasks.iter().map(render::task_line).collect();
        if lines.is_empty() {
            lines.push("no tasks".into());
        }
        lines.push(format!(
            "total {} cursor {}",
            page.total,
            page.cursor.encode()
        ));
        if page.more {
            lines.push("more true".into());
        }
        Ok(text(lines.join("\n")))
    }

    #[tool(
        name = "task_get",
        description = "One task with its revision; full text on request",
        annotations(read_only_hint = true)
    )]
    async fn task_get(
        &self,
        Parameters(args): Parameters<GetArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let task = blocking(&self.board, move |board| board.get(args.id))
            .await?
            .ok_or_else(|| err(format!("task T{} does not exist", args.id)))?;
        Ok(text(render::task_record(&task, args.full)))
    }

    #[tool(
        name = "task_epic",
        description = "One epic with direct-child status counts and progress",
        annotations(read_only_hint = true)
    )]
    async fn task_epic(
        &self,
        Parameters(args): Parameters<EpicArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let epic = blocking(&self.board, move |board| board.epic(args.id)).await?;
        Ok(text(render::epic_text(&epic)))
    }

    #[tool(
        name = "task_update",
        description = "Change fields, status or owner with an expected revision"
    )]
    async fn task_update(
        &self,
        Parameters(args): Parameters<UpdateArgs>,
        extensions: Extensions,
    ) -> Result<CallToolResult, ErrorData> {
        let scope = request_scope(&extensions);
        let actor = self.actor_name(scope.as_ref()).map_err(err)?;
        let status = match args.status.as_deref() {
            Some(raw) => Some(Status::parse(raw).map_err(|e| err(e.to_string()))?),
            None => None,
        };
        let assignee = crate::domain::set_or_clear("assignee", args.assignee, args.clear_assignee)
            .map_err(|error| err(error.to_string()))?;
        let parent = crate::domain::set_or_clear("parent", args.parent, args.clear_parent)
            .map_err(|error| err(error.to_string()))?;
        let project = crate::domain::set_or_clear("project", args.project, args.clear_project)
            .map_err(|error| err(error.to_string()))?;
        let patch = TaskPatch {
            title: args.title,
            description: args.description,
            goal: args.goal,
            analyst_report: args.analyst_report,
            coder_report: args.coder_report,
            reviewer_report: args.reviewer_report,
            replace_reports: args.replace_report,
            status,
            priority: args.priority,
            assignee,
            parent_id: parent,
            project_id: project,
        };
        let (updated, cursor) = blocking(&self.board, move |board| -> crate::store::Result<_> {
            let updated = board.update(UpdateInput {
                id: args.id,
                actor: &actor,
                expected_revision: args.if_revision,
                patch: &patch,
                release: args.release,
                force: args.force,
                force_reason: args.reason.as_deref(),
                new_actor: args.new_actor,
                comment: args.comment.as_deref(),
                now_ms: domain::now_ms(),
            })?;
            let instance = board.metadata()?.instance_id;
            let cursor = format!("{instance}:{}", updated.event.seq);
            Ok((updated, cursor))
        })
        .await?;
        Ok(text(format!(
            "ok\n{}\ncursor {}",
            render::task_record(&updated.task, false),
            cursor
        )))
    }

    #[tool(
        name = "task_claim",
        description = "Claim the next ready task or one by id; returns its work order"
    )]
    async fn task_claim(
        &self,
        Parameters(args): Parameters<ClaimArgs>,
        extensions: Extensions,
    ) -> Result<CallToolResult, ErrorData> {
        let scope = request_scope(&extensions);
        let actor = self.actor_name(scope.as_ref()).map_err(err)?;
        let outcome = blocking(&self.board, move |board| {
            board.claim(&actor, args.id, domain::now_ms())
        })
        .await?;
        if matches!(outcome, ClaimOutcome::NoWork) {
            return Ok(text("no ready tasks".into()));
        }
        Ok(text(render::claim_line(&outcome)))
    }

    #[tool(
        name = "task_comment",
        description = "Append a comment; does not change the revision"
    )]
    async fn task_comment(
        &self,
        Parameters(args): Parameters<CommentArgs>,
        extensions: Extensions,
    ) -> Result<CallToolResult, ErrorData> {
        let scope = request_scope(&extensions);
        let actor = self.actor_name(scope.as_ref()).map_err(err)?;
        let cursor = blocking(&self.board, move |board| -> crate::store::Result<_> {
            let event = board.comment(&actor, args.id, &args.body, domain::now_ms())?;
            let instance = board.metadata()?.instance_id;
            Ok(format!("{instance}:{}", event.seq))
        })
        .await?;
        Ok(text(format!("ok\ncursor {cursor}")))
    }

    #[tool(
        name = "board_feed",
        description = "Events after a cursor, filtered by task, kind or recipient; optional wait",
        annotations(read_only_hint = true)
    )]
    async fn board_feed(
        &self,
        Parameters(args): Parameters<FeedArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let kind = match args.kind.as_deref() {
            Some(raw) => Some(EventKind::parse(raw).map_err(|e| err(e.to_string()))?),
            None => None,
        };
        let FeedArgs {
            after,
            recipient,
            task,
            limit,
            wait,
            full,
            ..
        } = args;
        let page = blocking(&self.board, move |board| {
            let filter = FeedFilter {
                task,
                kind,
                recipient: recipient.as_deref(),
                after: after.as_deref(),
                limit: limit.unwrap_or(50),
            };
            match wait {
                Some(seconds) if seconds > 0 => board.wait(&filter, seconds.saturating_mul(1000)),
                _ => board.feed(&filter),
            }
        })
        .await?;
        Ok(text(render::feed_page(&page, full)))
    }

    #[tool(
        name = "board_brief",
        description = "Epic progress, unparented active tasks, then counts by status",
        annotations(read_only_hint = true)
    )]
    async fn board_brief(
        &self,
        Parameters(args): Parameters<BriefArgs>,
        extensions: Extensions,
    ) -> Result<CallToolResult, ErrorData> {
        let scope = request_scope(&extensions);
        let brief = blocking(&self.board, move |board| {
            let project = call_project(board, args.project, scope.as_ref())?;
            board.brief_for_project(10, project)
        })
        .await?;
        Ok(text(render::brief_text(&brief)))
    }

    #[tool(
        name = "board_agents",
        description = "Registered actor names, newest activity first",
        annotations(read_only_hint = true)
    )]
    async fn board_agents(&self) -> Result<CallToolResult, ErrorData> {
        let agents = blocking(&self.board, move |board| board.agents()).await?;
        if agents.is_empty() {
            return Ok(text("no actors".into()));
        }
        let lines: Vec<String> = agents.iter().map(render::agent_line).collect();
        Ok(text(lines.join("\n")))
    }

    #[tool(
        name = "board_project_delete",
        description = "Delete a project that no task carries",
        annotations(destructive_hint = true)
    )]
    async fn board_project_delete(
        &self,
        Parameters(args): Parameters<ProjectDeleteArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let project = blocking(&self.board, move |board| board.delete_project(args.id)).await?;
        Ok(text(render::deleted_project(&project)))
    }

    #[tool(
        name = "board_agent_delete",
        description = "Remove a roster name that holds no open task",
        annotations(destructive_hint = true)
    )]
    async fn board_agent_delete(
        &self,
        Parameters(args): Parameters<AgentDeleteArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let agent = blocking(&self.board, move |board| board.delete_agent(&args.name)).await?;
        Ok(text(render::deleted_agent(&agent)))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_board(name: &str) -> BoardHandle {
        let path = std::env::temp_dir().join(format!("agentboard-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        BoardHandle::open_with_options(
            &path,
            crate::store::BoardOptions {
                map_size_mb: Some(8),
            },
        )
        .expect("scratch board")
    }

    #[test]
    fn the_tool_surface_is_twelve_tools_with_valid_schemas() {
        let server = Board::new(scratch_board("surface"), None);
        let tools = server.tool_router.list_all();
        let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
        assert_eq!(
            names,
            vec![
                "board_agent_delete",
                "board_agents",
                "board_brief",
                "board_feed",
                "board_project_delete",
                "task_add",
                "task_claim",
                "task_comment",
                "task_epic",
                "task_get",
                "task_list",
                "task_update",
            ]
        );
        for tool in &tools {
            let encoded = serde_json::to_string(tool).expect("tool serializes");
            assert!(
                encoded.contains("\"inputSchema\""),
                "{} has no schema",
                tool.name
            );
        }
    }

    #[test]
    fn every_tool_description_names_the_board_object() {
        let server = Board::new(scratch_board("descriptions"), None);
        for tool in server.tool_router.list_all() {
            let description = tool.description.clone().unwrap_or_default();
            assert!(!description.is_empty(), "{} has no description", tool.name);
            assert!(
                description.len() < 120,
                "{} description is too long for a tool list",
                tool.name
            );
        }
    }
}
