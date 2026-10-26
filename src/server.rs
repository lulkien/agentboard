//! MCP surface: 10 tools, text-only results, short acknowledgements.
//!
//! Result shape is deliberate: reads are one line per task, writes answer with the
//! changed record plus its event cursor. Agents pay tokens per byte.

use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::schemars::JsonSchema;
use rmcp::{ErrorData, ServerHandler, tool, tool_handler, tool_router};
use serde::Deserialize;

use crate::domain::{EventKind, NewTask, Status, TaskPatch};
use crate::render;
use crate::store::{
    Board as BoardHandle, ClaimOutcome, FeedFilter, ListFilter, ReleaseInput, UpdateInput,
};

const NAME: &str = "agentboard";
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn text(body: String) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(body)])
}

fn err(message: String) -> ErrorData {
    ErrorData::invalid_params(message, None)
}

/// Store work is synchronous and LMDB serialises writers, so every call runs off
/// the async runtime threads.
async fn blocking<T: Send + 'static>(
    board: &Arc<BoardHandle>,
    f: impl FnOnce(&BoardHandle) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let board = Arc::clone(board);
    tokio::task::spawn_blocking(move || f(&board))
        .await
        .map_err(|error| format!("task join: {error}"))?
}

fn actor_from_env() -> Option<String> {
    std::env::var("AGENTBOARD_ACTOR")
        .ok()
        .filter(|name| !name.trim().is_empty())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
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
    /// todo | in_progress | blocked | review | done | canceled
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub parent: Option<u64>,
    #[serde(default)]
    pub path: Option<String>,
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
    pub path: Option<String>,
    /// Include done and canceled rows
    #[serde(default)]
    pub all: bool,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
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
    #[serde(default)]
    pub status: Option<String>,
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
    pub path: Option<String>,
    #[serde(default)]
    pub clear_path: bool,
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
    #[serde(default)]
    pub after: Option<String>,
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
        Self {
            tool_router: Self::tool_router(),
            board: Arc::new(board),
            actor,
        }
    }

    /// The actor this invocation writes as: `--actor` wins over the environment,
    /// so a wrapper that passes a flag is not silently overridden by an exported
    /// variable.
    fn actor_name(&self) -> Result<String, String> {
        self.actor
            .clone()
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
    ) -> Result<CallToolResult, ErrorData> {
        let actor = self.actor_name().map_err(err)?;
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
            path: args.path,
        };
        let added = blocking(&self.board, move |board| {
            board
                .add(&actor, input, args.new_actor, now_ms())
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(err)?;
        Ok(text(format!(
            "ok\n{}\ncursor {}",
            render::task_record(&added.task, false),
            added.event.seq
        )))
    }

    #[tool(
        name = "task_list",
        description = "List tasks by status, assignee, parent or path"
    )]
    async fn task_list(
        &self,
        Parameters(args): Parameters<ListArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let status = match args.status.as_deref() {
            Some(raw) => Some(Status::parse(raw).map_err(|e| err(e.to_string()))?),
            None => None,
        };
        let ListArgs {
            assignee,
            parent,
            path,
            all,
            limit,
            offset,
            ..
        } = args;
        let page = blocking(&self.board, move |board| {
            board
                .list(&ListFilter {
                    status,
                    assignee: assignee.as_deref(),
                    parent,
                    path: path.as_deref(),
                    include_terminal: all,
                    limit: limit.unwrap_or(20),
                    offset: offset.unwrap_or(0),
                })
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(err)?;
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
        description = "One task with its revision; full text on request"
    )]
    async fn task_get(
        &self,
        Parameters(args): Parameters<GetArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let task = blocking(&self.board, move |board| {
            board.get(args.id).map_err(|e| e.to_string())
        })
        .await
        .map_err(err)?
        .ok_or_else(|| err(format!("task T{} does not exist", args.id)))?;
        Ok(text(render::task_record(&task, args.full)))
    }

    #[tool(
        name = "task_epic",
        description = "One epic with direct-child status counts and progress"
    )]
    async fn task_epic(
        &self,
        Parameters(args): Parameters<EpicArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let epic = blocking(&self.board, move |board| {
            board.epic(args.id).map_err(|e| e.to_string())
        })
        .await
        .map_err(err)?;
        Ok(text(render::epic_text(&epic)))
    }

    #[tool(
        name = "task_update",
        description = "Change fields, status or owner with an expected revision"
    )]
    async fn task_update(
        &self,
        Parameters(mut args): Parameters<UpdateArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let actor = self.actor_name().map_err(err)?;
        let status = match args.status.as_deref() {
            Some(raw) => Some(Status::parse(raw).map_err(|e| err(e.to_string()))?),
            None => None,
        };
        if args.assignee.is_some() && args.clear_assignee {
            return Err(err("choose assignee or clear_assignee, not both".into()));
        }
        if args.parent.is_some() && args.clear_parent {
            return Err(err("choose parent or clear_parent, not both".into()));
        }
        if args.path.is_some() && args.clear_path {
            return Err(err("choose path or clear_path, not both".into()));
        }
        if args.release && (status.is_some() || args.assignee.is_some() || args.clear_assignee) {
            return Err(err(
                "choose release or status/assignee/clear_assignee, not both".into(),
            ));
        }
        // A release writes the row itself; a field flag beside it would be read
        // and discarded, so name the conflict instead of dropping the argument.
        if args.release
            && (args.title.is_some()
                || args.description.is_some()
                || args.goal.is_some()
                || args.priority.is_some()
                || args.parent.is_some()
                || args.clear_parent
                || args.path.is_some()
                || args.clear_path)
        {
            return Err(err("choose release or a field change, not both".into()));
        }
        if args.release {
            // Taken, not borrowed: the closure below owns what it sends, and the
            // field path that follows is unreachable once `release` is set.
            let reason = args.reason.take();
            let comment = args.comment.take();
            let released = blocking(&self.board, move |board| {
                board
                    .release(ReleaseInput {
                        id: args.id,
                        actor: &actor,
                        expected_revision: args.if_revision,
                        force: args.force,
                        force_reason: reason.as_deref(),
                        comment: comment.as_deref(),
                        now_ms: now_ms(),
                    })
                    .map_err(|e| e.to_string())
            })
            .await
            .map_err(err)?;
            return Ok(text(format!(
                "ok\n{}\ncursor {}",
                render::task_record(&released.task, false),
                released.event.seq
            )));
        }
        let patch = TaskPatch {
            title: args.title,
            description: args.description,
            goal: args.goal,
            status,
            priority: args.priority,
            assignee: if args.clear_assignee {
                Some(None)
            } else {
                args.assignee.map(Some)
            },
            parent_id: if args.clear_parent {
                Some(None)
            } else {
                args.parent.map(Some)
            },
            path: if args.clear_path {
                Some(None)
            } else {
                args.path.map(Some)
            },
        };
        let updated = blocking(&self.board, move |board| {
            board
                .update(UpdateInput {
                    id: args.id,
                    actor: &actor,
                    expected_revision: args.if_revision,
                    patch: &patch,
                    force: args.force,
                    force_reason: args.reason.as_deref(),
                    new_actor: args.new_actor,
                    comment: args.comment.as_deref(),
                    now_ms: now_ms(),
                })
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(err)?;
        Ok(text(format!(
            "ok\n{}\ncursor {}",
            render::task_record(&updated.task, false),
            updated.event.seq
        )))
    }

    #[tool(
        name = "task_claim",
        description = "Claim the next ready task or one by id; returns its work order"
    )]
    async fn task_claim(
        &self,
        Parameters(args): Parameters<ClaimArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let actor = self.actor_name().map_err(err)?;
        let outcome = blocking(&self.board, move |board| {
            board
                .claim(&actor, args.id, now_ms())
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(err)?;
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
    ) -> Result<CallToolResult, ErrorData> {
        let actor = self.actor_name().map_err(err)?;
        let event = blocking(&self.board, move |board| {
            board
                .comment(&actor, args.id, &args.body, now_ms())
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(err)?;
        Ok(text(format!("ok\ncursor {}", event.seq)))
    }

    #[tool(
        name = "board_feed",
        description = "Events after a cursor, filtered by task, kind or recipient; optional wait"
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
            let page = match wait {
                Some(seconds) if seconds > 0 => board.wait(&filter, seconds.saturating_mul(1000)),
                _ => board.feed(&filter),
            };
            page.map_err(|e| e.to_string())
        })
        .await
        .map_err(err)?;
        Ok(text(render::feed_page(&page, full)))
    }

    #[tool(
        name = "board_brief",
        description = "Epic progress, unparented active tasks, then counts by status"
    )]
    async fn board_brief(&self) -> Result<CallToolResult, ErrorData> {
        let brief = blocking(&self.board, move |board| {
            board.brief(10).map_err(|e| e.to_string())
        })
        .await
        .map_err(err)?;
        Ok(text(render::brief_text(&brief)))
    }

    #[tool(
        name = "board_agents",
        description = "Registered actor names, newest activity first"
    )]
    async fn board_agents(&self) -> Result<CallToolResult, ErrorData> {
        let agents = blocking(&self.board, move |board| {
            board.agents().map_err(|e| e.to_string())
        })
        .await
        .map_err(err)?;
        if agents.is_empty() {
            return Ok(text("no actors".into()));
        }
        let lines: Vec<String> = agents.iter().map(render::agent_line).collect();
        Ok(text(lines.join("\n")))
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
        BoardHandle::open_with_options(&path, crate::store::BoardOptions { map_size_mb: 8 })
            .expect("scratch board")
    }

    #[test]
    fn the_tool_surface_is_ten_tools_with_valid_schemas() {
        let server = Board::new(scratch_board("surface"), None);
        let tools = server.tool_router.list_all();
        let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
        assert_eq!(
            names,
            vec![
                "board_agents",
                "board_brief",
                "board_feed",
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
