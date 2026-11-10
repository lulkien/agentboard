use std::fmt::Write;

use agentboard::domain::{EventKind, Task};
use agentboard::store::{Board, FeedFilter, ListFilter, TaskPage};

use super::PageError;
use super::time::format_timestamp;
use crate::dashboard::html;
use crate::dashboard::routes::Page;

pub(super) fn detail(board: &Board, id: u64) -> Result<String, PageError> {
    let task = board
        .get(id)
        .map_err(|e| PageError::Store(e.to_string()))?
        .ok_or(PageError::Missing)?;
    let child_filter = ListFilter {
        status: None,
        assignee: None,
        parent: Some(id),
        project: None,
        include_terminal: true,
        limit: 20,
        offset: 0,
    };
    let children = board
        .list(&child_filter)
        .map_err(|e| PageError::Store(e.to_string()))?;
    let epic = if children.total > 0 {
        Some(
            board
                .epic(id)
                .map_err(|e| PageError::Store(e.to_string()))?,
        )
    } else {
        None
    };
    let feed = board
        .feed(&FeedFilter {
            task: Some(id),
            kind: None,
            recipient: None,
            after: None,
            limit: 20,
        })
        .map_err(|e| PageError::Store(e.to_string()))?;
    Ok(task_detail(&task, &children, epic.as_ref(), &feed.events))
}

fn task_detail(
    task: &Task,
    children: &TaskPage,
    epic: Option<&agentboard::store::Epic>,
    events: &[agentboard::domain::Event],
) -> String {
    let mut body = String::new();
    let title = html::escape(&task.title);
    let _ = write!(
        body,
        "<nav class=\"breadcrumbs\" aria-label=\"Breadcrumb\"><a href=\"/tasks\">Tasks</a> <span aria-hidden=\"true\">/</span> <span>T{}</span></nav><header class=\"issue-header\"><p class=\"meta\">Task T{}</p><div class=\"card-heading\"><h2>{title}</h2>{}</div></header><div class=\"detail-layout\"><div class=\"detail-main\"><section class=\"panel text-section\"><h2>Description</h2><div class=\"full-text\">{}</div></section><section class=\"panel text-section\"><h2>Goal</h2><div class=\"full-text\">{}</div></section>",
        task.id,
        task.id,
        html::status_badge(task.status),
        html::escape(&task.description),
        html::escape(&task.goal)
    );
    for (heading, report) in [
        ("Analyst report", task.analyst_report.as_str()),
        ("Coder report", task.coder_report.as_str()),
        ("Reviewer report", task.reviewer_report.as_str()),
    ] {
        if !report.is_empty() {
            let _ = write!(
                body,
                "<section class=\"panel text-section\"><h2>{heading}</h2><div class=\"full-text\">{}</div></section>",
                html::escape(report)
            );
        }
    }
    if let Some(epic) = epic {
        let maximum = epic.total.max(1);
        let progress = epic.done.min(maximum);
        let _ = write!(
            body,
            "<section class=\"panel\"><div class=\"card-heading\"><h2>Child tasks</h2><span class=\"meta\">{} / {} done</span></div><progress max=\"{maximum}\" value=\"{progress}\" aria-label=\"{}: {} of {} direct children done\"></progress><div class=\"table-scroll\"><table><thead><tr><th scope=\"col\">ID</th><th scope=\"col\">Status</th><th scope=\"col\">Title</th><th scope=\"col\">Assignee</th></tr></thead><tbody>",
            epic.done, epic.total, title, epic.done, epic.total
        );
        for child in &children.tasks {
            let child_title = html::escape(&child.title);
            let assignee = html::escape(child.assignee.as_deref().unwrap_or("—"));
            let _ = write!(
                body,
                "<tr><td><a href=\"/tasks/{}\">T{}</a></td><td>{}</td><td>{child_title}</td><td>{assignee}</td></tr>",
                child.id,
                child.id,
                html::status_badge(child.status)
            );
        }
        body.push_str("</tbody></table></div>");
        if children.total > children.tasks.len() {
            let _ = write!(
                body,
                "<p><a href=\"/tasks?parent={}\">View all {} child tasks</a></p>",
                task.id, children.total
            );
        }
        body.push_str("</section>");
    }
    body.push_str("<section class=\"panel activity\"><div class=\"card-heading\"><h2>Activity</h2><a href=\"/feed?task=");
    let _ = write!(body, "{}\">Older history</a></div>", task.id);
    if events.is_empty() {
        body.push_str("<p class=\"empty\">No activity yet</p>");
    } else {
        body.push_str("<ol class=\"timeline\">");
        for event in events {
            let actor = html::escape(&event.actor);
            let kind = event.kind.as_str();
            let timestamp = format_timestamp(event.timestamp_ms);
            let _ = write!(
                body,
                "<li><div class=\"timeline-heading\"><strong>{}</strong><span class=\"meta\">{}</span><time datetime=\"{timestamp}\">{timestamp}</time></div>",
                kind, actor
            );
            let payload_text = event
                .payload
                .get("text")
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    event
                        .payload
                        .get("reason")
                        .and_then(serde_json::Value::as_str)
                });
            if let Some(text) = payload_text {
                let _ = write!(
                    body,
                    "<div class=\"full-text event-text\">{}</div>",
                    html::escape(text)
                );
            } else if event.kind == EventKind::Updated {
                body.push_str("<p class=\"meta\">Task fields updated</p>");
            }
            body.push_str("</li>");
        }
        body.push_str("</ol>");
    }
    body.push_str("</section></div><aside class=\"panel metadata\" aria-label=\"Task metadata\"><h2>Details</h2><dl>");
    metadata_item(&mut body, "Status", &html::status_badge(task.status));
    metadata_item(
        &mut body,
        "Priority",
        &format!(
            "<span class=\"priority priority-value\">{}</span>",
            task.priority
        ),
    );
    metadata_item(
        &mut body,
        "Assignee",
        &html::escape(task.assignee.as_deref().unwrap_or("Unassigned")),
    );
    metadata_item(&mut body, "Reporter", &html::escape(&task.reporter));
    if let Some(parent) = task.parent_id {
        metadata_item(
            &mut body,
            "Parent",
            &format!("<a href=\"/tasks/{parent}\">T{parent}</a>"),
        );
    }
    metadata_item(
        &mut body,
        "Project",
        &html::escape(
            task.project_id
                .map_or_else(|| "—".to_owned(), |id| id.to_string())
                .as_str(),
        ),
    );
    metadata_item(&mut body, "Revision", &task.revision.to_string());
    let created = format_timestamp(task.created_at_ms);
    metadata_item(
        &mut body,
        "Created",
        &format!("<time datetime=\"{created}\">{created}</time>"),
    );
    let updated = format_timestamp(task.updated_at_ms);
    metadata_item(
        &mut body,
        "Updated",
        &format!("<time datetime=\"{updated}\">{updated}</time>"),
    );
    body.push_str("</dl></aside></div>");
    html::shell(
        &format!("T{} | {}", task.id, task.title),
        Page::Detail(task.id),
        &body,
    )
}

fn metadata_item(body: &mut String, label: &str, value: &str) {
    let _ = write!(
        body,
        "<div class=\"metadata-row\"><dt>{label}</dt><dd>{value}</dd></div>"
    );
}
