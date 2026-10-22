//! MCP surface: 16 tools, text-only results, short acks.
//!
//! Result shape is deliberate: reads are one line per task, writes answer with
//! `ok T7 …` instead of echoing the record. Agents pay tokens per byte.

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::schemars::JsonSchema;
use rmcp::{ErrorData, ServerHandler, tool, tool_handler, tool_router};
use serde::Deserialize;

use crate::ident;
use crate::store;

/// Verify the base count from the actual router. `scripts/sync-tool-count.py`
/// reads tools/list to publish the header and docs; the test below binds the
/// router, source markers and generated header together.
#[cfg(test)]
pub fn tool_count() -> usize {
    Board::new().tool_router.list_all().len()
}

fn actor() -> String {
    store::default_actor()
}

fn text(s: String) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(s)])
}

fn err(e: String) -> ErrorData {
    if e.starts_with("board schema mismatch:") {
        ErrorData::internal_error(e, None)
    } else {
        ErrorData::invalid_params(e, None)
    }
}

/// A claim refused because someone holds the task is not a malformed request;
/// every other claim failure keeps the code the rest of the tools use.
fn claim_err(e: store::ClaimError) -> ErrorData {
    match e {
        store::ClaimError::Held(m) => ErrorData::internal_error(m, None),
        store::ClaimError::Other(m) => err(m),
    }
}

/// Runs a store call off the async runtime threads. The store is synchronous: a
/// sqlite busy timeout, the surreal write-lock retry and a tmux subprocess all park
/// the calling thread, and parked reactor workers would stall every other tool.
async fn offload<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("task join: {e}"))
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let result = offload(f).await?;
    for line in store::take_migrations() {
        eprintln!("{line}");
    }
    result
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
    /// Epic the work belongs to, e.g. E3. A subtask inherits its parent's epic.
    #[serde(default)]
    pub epic: Option<String>,
    /// Parent task id, e.g. T3 (a subtask link, such as a fix task under the review that found it).
    #[serde(default)]
    pub parent: Option<String>,
    /// Project root the work lives in. `~` expands, a relative path resolves against the server's cwd, it need not exist yet.
    #[serde(default)]
    pub path: Option<String>,
    /// Git ref the worker starts on. Default HEAD leaves the pane's checkout alone; empty string means HEAD.
    #[serde(default)]
    pub branch: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EpicAddArgs {
    /// One-line epic title.
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// What done looks like for the whole epic.
    #[serde(default)]
    pub goal: String,
    /// Project root the work belongs to. `~` expands, a relative path resolves against the server's cwd, it need not exist yet.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EpicListArgs {
    /// open|done|dropped. Default open.
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DiffAddArgs {
    /// Task id the diff belongs to, T7 or 7.
    pub id: String,
    /// The diff text, stored verbatim. Whitespace only is refused. With reviews, the
    /// evidence for a re-run round: what you ran and its output, never a patch.
    pub body: String,
    /// Evidence-only submission: the newest patch of this row, D26. Omit for a normal
    /// diff. The named diff must be a patch recorded on this row and must be its newest
    /// patch (no re-parent); the gate then accepts this round without a new patch.
    #[serde(default)]
    pub reviews: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DiffListArgs {
    /// Task id T7, or epic id E3 for every diff of the epic's tasks.
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DiffGetArgs {
    /// Diff id, D12.
    pub id: String,
    /// true returns the body unclipped and verbatim. Default false: a long body is cut with a `[truncated ...]` marker.
    #[serde(default)]
    pub full: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EpicGetArgs {
    /// Epic id, E3.
    pub id: String,
    /// true returns description, goal and event bodies unclipped and verbatim.
    #[serde(default)]
    pub full: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EpicUpdateArgs {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub goal: Option<String>,
    #[serde(default)]
    pub path: Option<String>,

    /// Epic id, E3.
    pub id: String,
    /// open|done|dropped. Closing an epic is the manager's call, never automatic.
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListArgs {
    /// ready|working|blocking|not_done|needs_fix|review|done|dropped. Omit for all open work.
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
pub struct GetArgs {
    /// Task id, T7 or 7.
    pub id: String,
    /// true returns description, goal, report, note and event bodies unclipped and verbatim. Default false: long fields are cut with a `[truncated ...]` marker.
    #[serde(default)]
    pub full: bool,
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
#[serde(deny_unknown_fields)]
pub struct UpdateArgs {
    /// Target open epic, E3; cannot split a parent/child chain.
    #[serde(default)]
    pub epic: Option<String>,
    #[serde(default)]
    pub priority: Option<i64>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub goal: Option<String>,
    #[serde(default)]
    pub path: Option<String>,

    pub id: String,
    /// ready|working|blocking|not_done|needs_fix|review|done|dropped. Omit to keep the current one.
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
pub struct WaitArgs {
    /// One task id, T7 or 7. Same as ids=[id].
    #[serde(default)]
    pub id: Option<String>,
    /// Task ids to watch.
    #[serde(default)]
    pub ids: Vec<String>,
    /// Watch every task in this epic (E3).
    #[serde(default)]
    pub epic: Option<String>,
    /// Watch every task whose status is not done/dropped.
    #[serde(default)]
    pub open: bool,
    /// CSV of statuses (ready|working|blocking|not_done|needs_fix|review|done|dropped): match only while the task's CURRENT status is in it. Already true => returns at once.
    #[serde(default)]
    pub status: Option<String>,
    /// CSV of event kinds (add|claim|report|note|block|answer|review|done|needs_fix|edit|dispatch|diff; ask = block): match only new events of these kinds.
    #[serde(default)]
    pub kind: Option<String>,
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
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "task_add",
        description = "Create a task. description holds everything the worker needs (repo path, files, situation); goal holds how to do it and the definition of done. epic (E3) says which epic it belongs to; parent (T7) makes it a subtask and inherits the parent's epic. path is the project root and branch the git ref the worker starts on (default HEAD leaves the pane's checkout alone). Returns its short id (T7).",
        annotations(title = "Add task", read_only_hint = false, destructive_hint = false)
    )]
    async fn task_add(
        &self,
        Parameters(a): Parameters<AddArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        blocking(store::check_entry).await.map_err(err)?;
        let parent = match a.parent.as_deref() {
            Some(p) => Some(store::parse_id(p).map_err(err)?),
            None => None,
        };
        let epic = match a.epic.as_deref() {
            Some(e) => Some(store::parse_epic_id(e).map_err(err)?),
            None => None,
        };
        let status = match a.status.as_deref() {
            Some(raw) => store::norm(raw).ok_or_else(|| err(format!("bad status {raw:?}")))?,
            None => "ready",
        };
        run!(
            blocking(move || {
                let task = store::NewTask {
                    title: &a.title,
                    description: &a.description,
                    goal: &a.goal,
                    status,
                    assignee: a.assignee.as_deref(),
                    priority: a.priority.unwrap_or(100),
                    epic,
                    parent,
                    path: a.path.as_deref(),
                    branch: a.branch.as_deref(),
                };
                store::add(&actor(), &task)
            })
            .await
        )
    }

    #[tool(
        name = "task_list",
        description = "Open tasks, one compact line each: id, status, assignee, title.",
        annotations(title = "List tasks", read_only_hint = true)
    )]
    async fn task_list(
        &self,
        Parameters(a): Parameters<ListArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        blocking(store::check_entry).await.map_err(err)?;
        run!(
            blocking(move || store::list(
                a.status.as_deref(),
                a.assignee.as_deref(),
                a.limit.unwrap_or(20).clamp(1, 200)
            ))
            .await
        )
    }

    #[tool(
        name = "task_get",
        description = "Task record: status, timestamps, description, goal, report, note, recent events. Long fields end in a `[truncated <field>: N of M chars ...]` marker; pass full=true to get them whole and verbatim. Manager reads this to collect a worker's result.",
        annotations(title = "Get task", read_only_hint = true)
    )]
    async fn task_get(
        &self,
        Parameters(a): Parameters<GetArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        blocking(store::check_entry).await.map_err(err)?;
        let id = store::parse_id(&a.id).map_err(err)?;
        run!(blocking(move || store::get(id, a.full)).await)
    }

    #[tool(
        name = "task_children",
        description = "Tasks under an id, one compact line each: id, status, assignee, age, title, ordered by id. A task id (T7) lists its subtasks; an epic id (E3) lists every task in the epic.",
        annotations(title = "List children", read_only_hint = true)
    )]
    async fn task_children(
        &self,
        Parameters(a): Parameters<IdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        blocking(store::check_entry).await.map_err(err)?;
        let of = store::parse_ref(&a.id).map_err(err)?;
        run!(blocking(move || store::children(of)).await)
    }

    #[tool(
        name = "epic_add",
        description = "Create an epic: a project-scoped group of tasks (E1, E2...). It is never claimed, leased or dispatched. path is the project root the work belongs to; dispatch warns when a pane's cwd is outside it. Returns its id (E3); adds a warning line if the path does not exist yet.",
        annotations(title = "Add epic", read_only_hint = false, destructive_hint = false)
    )]
    async fn epic_add(
        &self,
        Parameters(a): Parameters<EpicAddArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        blocking(store::check_entry).await.map_err(err)?;
        run!(
            blocking(move || store::epic_add(
                &actor(),
                &a.title,
                &a.description,
                &a.goal,
                a.path.as_deref()
            ))
            .await
        )
    }

    #[tool(
        name = "epic_list",
        description = "Epics, one line each: id, status, done/total tasks, path, title. Default open.",
        annotations(title = "List epics", read_only_hint = true)
    )]
    async fn epic_list(
        &self,
        Parameters(a): Parameters<EpicListArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        blocking(store::check_entry).await.map_err(err)?;
        run!(blocking(move || store::epic_list(a.status.as_deref())).await)
    }

    #[tool(
        name = "diff_add",
        description = "Record what you changed for a task, verbatim: a diff row, append-only, round = the task's reopen count + 1. A reviewer reads only the newest patch, so record the complete change each time. Whitespace-only is refused. A re-run round whose tree did not change records evidence instead: pass reviews=\"D<n>\" naming the newest patch of this row (the body is then evidence, never a patch; a superseded patch, a diff on another row and an evidence-only row are refused). Returns D<n> (T<n> round <r>).",
        annotations(title = "Add diff", read_only_hint = false, destructive_hint = false)
    )]
    async fn diff_add(
        &self,
        Parameters(a): Parameters<DiffAddArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        blocking(store::check_entry).await.map_err(err)?;
        let id = store::parse_id(&a.id).map_err(err)?;
        let reviews = a
            .reviews
            .as_deref()
            .map(store::parse_diff_id)
            .transpose()
            .map_err(err)?;
        let actor = actor();
        run!(blocking(move || store::diff_add(&actor, id, &a.body, reviews)).await)
    }

    #[tool(
        name = "diff_get",
        description = "One diff: task, round, age and body. Only the newest patch of a task is reviewed (`diff_list` shows which). An evidence-only row is labelled as such and points at the diff it re-runs; it is not a patch. A long body ends in a `[truncated ...]` marker; full=true returns it whole and verbatim.",
        annotations(title = "Get diff", read_only_hint = true)
    )]
    async fn diff_get(
        &self,
        Parameters(a): Parameters<DiffGetArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        blocking(store::check_entry).await.map_err(err)?;
        let id = store::parse_diff_id(&a.id).map_err(err)?;
        run!(blocking(move || store::diff_get(id, a.full)).await)
    }

    #[tool(
        name = "diff_list",
        description = "A task's diffs (T7), or every diff of an epic's tasks (E3), newest first, one line each: D12 round 2 3m 184 lines. An evidence-only row reads D13 evidence-only -> D12: it is not a patch. Only the newest patch of a task is reviewed. `(no diffs)` when there are none.",
        annotations(title = "List diffs", read_only_hint = true)
    )]
    async fn diff_list(
        &self,
        Parameters(a): Parameters<DiffListArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        blocking(store::check_entry).await.map_err(err)?;
        let of = store::parse_ref(&a.id).map_err(err)?;
        run!(blocking(move || store::diff_list(of)).await)
    }

    #[tool(
        name = "epic_get",
        description = "Epic record: status, path, task counts, description, goal, recent events. Long fields end in a `[truncated ...]` marker; full=true returns them whole.",
        annotations(title = "Get epic", read_only_hint = true)
    )]
    async fn epic_get(
        &self,
        Parameters(a): Parameters<EpicGetArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        blocking(store::check_entry).await.map_err(err)?;
        let id = store::parse_epic_id(&a.id).map_err(err)?;
        run!(blocking(move || store::epic_get(id, a.full)).await)
    }

    #[tool(
        name = "epic_update",
        description = "Update an epic: status (open|done|dropped), note, title, description, goal, path. Only passed fields change; combined spec/status changes are atomic. Unknown fields are refused by name. Closing is the manager's call.",
        annotations(
            title = "Update epic",
            read_only_hint = false,
            destructive_hint = false
        )
    )]
    async fn epic_update(
        &self,
        Parameters(a): Parameters<EpicUpdateArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        blocking(store::check_entry).await.map_err(err)?;
        let id = store::parse_epic_id(&a.id).map_err(err)?;
        run!(
            blocking(move || {
                if a.title.is_some()
                    || a.description.is_some()
                    || a.goal.is_some()
                    || a.path.is_some()
                {
                    store::epic_edit(
                        &actor(),
                        id,
                        &store::EpicEdit {
                            title: a.title.as_deref(),
                            description: a.description.as_deref(),
                            goal: a.goal.as_deref(),
                            path: a.path.as_deref(),
                            status: a.status.as_deref(),
                            note: a.note.as_deref(),
                        },
                    )
                } else if let Some(status) = a.status {
                    store::epic_status(&actor(), id, &status, a.note.as_deref())
                } else {
                    Err("nothing to edit: pass status, title, description, goal or path".into())
                }
            })
            .await
        )
    }

    #[tool(
        name = "task_claim",
        description = "Atomically claim a task (or the next ready one) and return its description + goal. Claim expires after ttl_min; heartbeat to extend.",
        annotations(title = "Claim task", read_only_hint = false, destructive_hint = false)
    )]
    async fn task_claim(
        &self,
        Parameters(a): Parameters<ClaimArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        blocking(store::check_entry).await.map_err(err)?;
        let id = match a.id.as_deref() {
            Some(s) => Some(store::parse_id(s).map_err(err)?),
            None => None,
        };
        let ttl = a.ttl_min.unwrap_or(30).clamp(1, 720);
        offload(move || store::claim(&actor(), id, ttl * 60))
            .await
            .map_err(err)?
            .map(text)
            .map_err(claim_err)
    }

    #[tool(
        name = "task_update",
        description = "Update a task: status, report, note, commit, heartbeat, or spec fields title, description, goal, priority, assignee, path, branch, epic. Spec edits refuse a live worker lease; epic moves require an open target and preserve parent/child chains. Combined spec/state changes are atomic. Unknown fields are refused by name. For a human decision set status=blocking with the question in note.",
        annotations(
            title = "Update task",
            read_only_hint = false,
            destructive_hint = false
        )
    )]
    async fn task_update(
        &self,
        Parameters(a): Parameters<UpdateArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        blocking(store::check_entry).await.map_err(err)?;
        let id = store::parse_id(&a.id).map_err(err)?;
        let who = actor();
        let ttl_s = a.ttl_min.unwrap_or(30).clamp(1, 720) * 60;
        let has_edit = a.title.is_some()
            || a.description.is_some()
            || a.goal.is_some()
            || a.path.is_some()
            || a.branch.is_some()
            || a.priority.is_some()
            || a.assignee.is_some()
            || a.epic.is_some();
        if has_edit {
            let epic = a
                .epic
                .as_deref()
                .map(store::parse_epic_id)
                .transpose()
                .map_err(err)?;
            return offload(move || {
                let edit = store::TaskEdit {
                    title: a.title.as_deref(),
                    description: a.description.as_deref(),
                    goal: a.goal.as_deref(),
                    path: a.path.as_deref(),
                    branch: a.branch.as_deref(),
                    priority: a.priority,
                    assignee: a.assignee.as_deref(),
                    epic,
                };
                if a.heartbeat {
                    return Err(store::ClaimError::Other(
                        "spec edits cannot heartbeat a held task; send a separate execution update"
                            .into(),
                    ));
                }
                if a.status.is_none()
                    && a.report.is_none()
                    && a.note.is_none()
                    && a.commit.is_none()
                {
                    store::edit(&who, id, &edit)
                } else {
                    store::apply(
                        &who,
                        id,
                        &store::Patch {
                            edit: Some(&edit),
                            status: a.status.as_deref(),
                            report: a.report.as_deref(),
                            note: a.note.as_deref(),
                            commit: a.commit.as_deref(),
                            needs: None,
                            ttl_s,
                        },
                    )
                    .map_err(|message| {
                        if message.starts_with(&format!("T{id} working by ")) {
                            store::ClaimError::Held(message)
                        } else {
                            store::ClaimError::Other(message)
                        }
                    })
                }
            })
            .await
            .map_err(err)?
            .map(text)
            .map_err(claim_err);
        }
        let beat = if a.heartbeat {
            let who = who.clone();
            Some(
                blocking(move || store::heartbeat(&who, id, ttl_s))
                    .await
                    .map_err(err)?,
            )
        } else {
            None
        };
        if a.status.is_none() && a.report.is_none() && a.note.is_none() && a.commit.is_none() {
            return match beat {
                Some(ack) => Ok(text(ack)),
                None => Err(err(
                    "nothing to do: pass status, report, note, commit or heartbeat".to_owned(),
                )),
            };
        }
        run!(
            blocking(move || {
                let patch = store::Patch {
                    edit: None,
                    status: a.status.as_deref(),
                    report: a.report.as_deref(),
                    note: a.note.as_deref(),
                    commit: a.commit.as_deref(),
                    needs: None,
                    ttl_s,
                };
                store::apply(&who, id, &patch)
            })
            .await
        )
    }

    #[tool(
        name = "task_wait",
        description = "Server-side long-poll over a set of tasks. Selectors (union, at least one): id, ids, epic (direct children), open (not done/dropped). Filters: status (CSV, current status; already true returns at once), kind (CSV, new event kinds); both given = both hold; none = any new event. after_seq: only events past it. FIRST LINE is `matched:` (then per task `T7 review claude1` and its new event lines) or `timeout:` (then every watched status and the tail of the freshest task). Unknown task, status or kind is an error. Prefer this over repeated task_get.",
        annotations(title = "Wait for change", read_only_hint = true)
    )]
    async fn task_wait(
        &self,
        Parameters(a): Parameters<WaitArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        blocking(store::check_entry).await.map_err(err)?;
        let ms = a.timeout_s.unwrap_or(60).clamp(1, 300) * 1000;
        let ids: Vec<String> = a.id.into_iter().chain(a.ids).collect();
        // The wait polls asynchronously, but its setup (sqlite open, surreal
        // connect) parks the thread; drive the whole future on a blocking thread.
        let rt = tokio::runtime::Handle::current();
        run!(
            blocking(move || rt
                .block_on(store::wait(store::Wait {
                    ids: &ids,
                    epic: a.epic.as_deref(),
                    open: a.open,
                    status: a.status.as_deref(),
                    kind: a.kind.as_deref(),
                    after: a.after_seq,
                    timeout_ms: ms,
                }))
                .map(|o| o.text))
            .await
        )
    }

    #[tool(
        name = "board_brief",
        description = "Team status in one screen: counts, open tasks ordered by urgency, recent completions. Use for standup/reporting.",
        annotations(title = "Board brief", read_only_hint = true, idempotent_hint = true)
    )]
    async fn board_brief(
        &self,
        Parameters(a): Parameters<BriefArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        blocking(store::check_entry).await.map_err(err)?;
        let mins = a.since_min.unwrap_or(120).clamp(1, 20_160);
        run!(blocking(move || store::brief(mins)).await)
    }

    #[tool(
        name = "session_register",
        description = "Register this session's tmux name so the board can map task -> pane.",
        annotations(
            title = "Register session",
            read_only_hint = false,
            destructive_hint = false
        )
    )]
    async fn session_register(
        &self,
        Parameters(a): Parameters<RegisterArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        blocking(store::check_entry).await.map_err(err)?;
        let task = match a.task_id.as_deref() {
            Some(t) => Some(store::parse_id(t).map_err(err)?),
            None => None,
        };
        run!(
            blocking(move || store::register_session(
                &a.name,
                a.agent.as_deref().unwrap_or("agent"),
                a.cwd.as_deref().unwrap_or(""),
                task
            ))
            .await
        )
    }
}

impl Default for Board {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Board {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::new(ServerCapabilities::builder().enable_tools().build());
        info.server_info = Implementation::new(ident::NAME, ident::VERSION);
        info
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_setter_fields_are_refused_by_name() {
        let task = serde_json::json!({"id": "T1", "unknown_probe": true});
        let epic = serde_json::json!({"id": "E1", "unknown_probe": true});
        assert!(
            serde_json::from_value::<UpdateArgs>(task)
                .unwrap_err()
                .to_string()
                .contains("unknown_probe")
        );
        assert!(
            serde_json::from_value::<EpicUpdateArgs>(epic)
                .unwrap_err()
                .to_string()
                .contains("unknown_probe")
        );
    }

    #[test]
    fn the_reported_tool_count_is_the_number_of_tool_markers() {
        // Both sides derived: the registry the client is served, and the markers the
        // surface is written with. Counted at line start, so prose that names the
        // marker (this file's comments do) is not a marker, and the literal is
        // assembled here so this test does not count itself.
        let marker = concat!("#[to", "ol(");
        let markers = include_str!("server.rs")
            .lines()
            .filter(|l| l.trim_start().starts_with(marker))
            .count();
        assert_eq!(tool_count(), markers);
        let header = include_str!("server.rs").lines().next().unwrap();
        assert!(header.contains(&format!("MCP surface: {} tools", tool_count())));
    }
}
