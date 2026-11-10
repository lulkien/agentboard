//! Read-only JSON API: one GET endpoint per dashboard page. Each endpoint reuses
//! `routes::parse` for its query, so unknown, duplicate and malformed keys are
//! refused the way `routes::parse` refuses them, and answers from the same
//! store read models. Task rows are summaries (no description, goal or reports)
//! except in the detail endpoint; statuses use the client-facing names.

use std::sync::Arc;

use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::domain::{Event, Status, Task, now_ms};
use crate::routes::{self, Filters, Page, Route};
use crate::store::{
    Board, Cursor, Epic, Error as StoreError, FeedFilter, ListFilter, MAX_LIST_LIMIT, TaskPage,
};

use super::App;

const PAGE_SIZE: usize = 20;
const LANE_CAP: usize = 8;
const CHILD_STRIP_CAP: usize = 4;
const DETAIL_CHILDREN: usize = 20;
const DETAIL_EVENTS: usize = 20;
const MAX_EPIC_DEPTH: usize = 6;

enum ApiError {
    BadRequest,
    Missing,
    Store(String),
}

impl From<StoreError> for ApiError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::CursorInstanceMismatch | StoreError::InvalidCursor(_) => Self::BadRequest,
            other => Self::Store(other.to_string()),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            Self::BadRequest => (StatusCode::BAD_REQUEST, "invalid api query").into_response(),
            Self::Missing => (StatusCode::NOT_FOUND, "not found").into_response(),
            Self::Store(message) => (StatusCode::INTERNAL_SERVER_ERROR, message).into_response(),
        }
    }
}

pub(super) async fn board(State(app): State<App>, RawQuery(query): RawQuery) -> Response {
    run(app.board, "/".into(), query, board_json).await
}

pub(super) async fn epics(State(app): State<App>, RawQuery(query): RawQuery) -> Response {
    run(app.board, "/epics".into(), query, epics_json).await
}

pub(super) async fn tasks(State(app): State<App>, RawQuery(query): RawQuery) -> Response {
    run(app.board, "/tasks".into(), query, tasks_json).await
}

pub(super) async fn task(
    State(app): State<App>,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> Response {
    run(app.board, format!("/tasks/{id}"), query, detail_json).await
}

pub(super) async fn feed(State(app): State<App>, RawQuery(query): RawQuery) -> Response {
    run(app.board, "/feed".into(), query, feed_json).await
}

pub(super) async fn projects(State(app): State<App>, RawQuery(query): RawQuery) -> Response {
    // `project` is the only key every page accepts, so the board page's grammar
    // is this endpoint's grammar too.
    run(app.board, "/".into(), query, projects_json).await
}

/// Parse the query, check a named project exists, build the body on a blocking
/// thread. Nothing here writes.
async fn run(
    board: Arc<Board>,
    page_path: String,
    query: Option<String>,
    build: fn(&Board, Route) -> Result<Value, ApiError>,
) -> Response {
    let work = move || {
        let route = routes::parse(&page_path, query.as_deref().unwrap_or(""))
            .map_err(|_| ApiError::BadRequest)?;
        if let Some(id) = route.filters.project {
            board.get_project(id)?.ok_or(ApiError::Missing)?;
        }
        build(&board, route)
    };
    match tokio::task::spawn_blocking(work).await {
        Ok(Ok(body)) => axum::Json(body).into_response(),
        Ok(Err(error)) => error.into_response(),
        Err(error) => ApiError::Store(error.to_string()).into_response(),
    }
}

fn board_json(board: &Board, route: Route) -> Result<Value, ApiError> {
    let project = route.filters.project;
    let overview = board.overview(project, LANE_CAP)?;
    let agents = board.agents()?;
    Ok(json!({
        "project": project,
        "now_ms": now_ms(),
        "cursor": overview.cursor.encode(),
        "counts": counts(&overview.counts),
        "lanes": overview.lanes.iter().map(|lane| json!({
            "status": lane.status.as_str(),
            "total": lane.total,
            "tasks": summaries(&lane.tasks),
        })).collect::<Vec<_>>(),
        "attention": { "total": overview.attention_total, "tasks": summaries(&overview.attention) },
        "held": overview.held.iter().map(|held| json!({
            "assignee": held.assignee,
            "total": held.total,
            "tasks": summaries(&held.tasks),
        })).collect::<Vec<_>>(),
        "agents": agents,
    }))
}

fn epics_json(board: &Board, route: Route) -> Result<Value, ApiError> {
    let project = route.filters.project;
    let brief = board.brief_for_project(0, project)?;
    let mut rows = Vec::with_capacity(brief.epics.len());
    for epic in &brief.epics {
        let children = board.list(&ListFilter {
            status: None,
            assignee: None,
            parent: Some(epic.task.id),
            project,
            include_terminal: true,
            limit: CHILD_STRIP_CAP,
            offset: 0,
        })?;
        rows.push(json!({
            "task": summary(&epic.task),
            "depth": epic_depth(epic, &brief.epics).min(MAX_EPIC_DEPTH),
            "counts": counts(&epic.counts),
            "done": epic.done,
            "total": epic.total,
            "children": { "total": children.total, "tasks": summaries(&children.tasks) },
        }));
    }
    Ok(json!({ "project": project, "epics": rows }))
}

fn tasks_json(board: &Board, route: Route) -> Result<Value, ApiError> {
    let Filters {
        project,
        status,
        assignee,
        parent,
        offset,
        ..
    } = route.filters;
    if let Some(parent) = parent {
        board.get(parent)?.ok_or(ApiError::Missing)?;
    }
    let filter = ListFilter {
        status,
        assignee: assignee.as_deref(),
        parent,
        project,
        include_terminal: true,
        limit: PAGE_SIZE.min(MAX_LIST_LIMIT),
        offset,
    };
    let page = board.list_newest(&filter)?;
    let status_counts = board.status_counts(&filter)?;
    Ok(json!({
        "project": project,
        "status": status.map(Status::as_str),
        "assignee": assignee,
        "parent": parent,
        "offset": offset,
        "page_size": PAGE_SIZE,
        "total": page.total,
        "more": page.more,
        "counts": counts(&status_counts),
        "tasks": summaries(&page.tasks),
    }))
}

fn detail_json(board: &Board, route: Route) -> Result<Value, ApiError> {
    let Page::Detail(id) = route.page else {
        return Err(ApiError::BadRequest);
    };
    let task = board.get(id)?.ok_or(ApiError::Missing)?;
    let children: TaskPage = board.list(&ListFilter {
        status: None,
        assignee: None,
        parent: Some(id),
        project: None,
        include_terminal: true,
        limit: DETAIL_CHILDREN,
        offset: 0,
    })?;
    let epic = if children.total > 0 {
        let epic = board.epic(id)?;
        json!({ "counts": counts(&epic.counts), "done": epic.done, "total": epic.total })
    } else {
        Value::Null
    };
    let feed = board.feed(&FeedFilter {
        task: Some(id),
        kind: None,
        recipient: None,
        after: None,
        limit: DETAIL_EVENTS,
    })?;
    let mut full = summary(&task);
    for (key, text) in [
        ("description", &task.description),
        ("goal", &task.goal),
        ("analyst_report", &task.analyst_report),
        ("coder_report", &task.coder_report),
        ("reviewer_report", &task.reviewer_report),
    ] {
        full[key] = json!(text);
    }
    full["reporter"] = json!(task.reporter);
    Ok(json!({
        "project": route.filters.project,
        "now_ms": now_ms(),
        "task": full,
        "children": { "total": children.total, "tasks": summaries(&children.tasks) },
        "epic": epic,
        "events": events(&feed.events),
    }))
}

fn feed_json(board: &Board, route: Route) -> Result<Value, ApiError> {
    let filters = route.filters;
    let filter = FeedFilter {
        task: filters.feed_task,
        kind: filters.feed_kind,
        recipient: None,
        after: None,
        limit: PAGE_SIZE,
    };
    let (page_events, older, before) = if let Some(project) = filters.project {
        let page = board.feed_in_project(&filter, project, filters.before.as_deref(), PAGE_SIZE)?;
        (page.events, page.older, Some(page.before))
    } else if let Some(before) = filters.before.as_deref() {
        let page = board.feed_before(&filter, before)?;
        (page.events, page.older, Some(page.before))
    } else {
        let page = board.feed(&filter)?;
        let before = page.events.first().map(|event| Cursor {
            instance_id: page.next.instance_id.clone(),
            seq: event.seq,
        });
        (page.events, page.older, before)
    };
    Ok(json!({
        "project": filters.project,
        "task": filters.feed_task,
        "kind": filters.feed_kind.map(|kind| kind.as_str()),
        "page_size": PAGE_SIZE,
        "events": events(&page_events),
        "older": older,
        // Pass back as `before` for the next older page; null when there is none.
        "before": before.filter(|_| older).map(|cursor| cursor.encode()),
    }))
}

fn projects_json(board: &Board, route: Route) -> Result<Value, ApiError> {
    Ok(json!({ "project": route.filters.project, "projects": board.list_projects()? }))
}

fn summary(task: &Task) -> Value {
    json!({
        "id": task.id,
        "title": task.title,
        "status": task.status.as_str(),
        "priority": task.priority,
        "assignee": task.assignee,
        "parent_id": task.parent_id,
        "project_id": task.project_id,
        "revision": task.revision,
        "created_at_ms": task.created_at_ms,
        "updated_at_ms": task.updated_at_ms,
    })
}

fn summaries(tasks: &[Task]) -> Vec<Value> {
    tasks.iter().map(summary).collect()
}

/// Every status in board order, so a client never has to guess missing keys.
fn counts(counts: &[(Status, u64)]) -> Vec<Value> {
    counts
        .iter()
        .map(|(status, count)| json!({ "status": status.as_str(), "count": count }))
        .collect()
}

fn events(events: &[Event]) -> Vec<Value> {
    events
        .iter()
        .map(|event| serde_json::to_value(event).unwrap_or(Value::Null))
        .collect()
}

fn epic_depth(epic: &Epic, epics: &[Epic]) -> usize {
    let mut depth = 0;
    let mut parent = epic.task.parent_id;
    while let Some(id) = parent {
        let Some(ancestor) = epics.iter().find(|candidate| candidate.task.id == id) else {
            break;
        };
        depth += 1;
        parent = ancestor.task.parent_id;
        if depth >= epics.len() {
            break;
        }
    }
    depth
}
