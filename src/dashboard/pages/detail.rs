use std::fmt::Write;

use agentboard::domain::{EventKind, Project, Task};
use agentboard::store::{Board, FeedFilter, ListFilter, TaskPage};

use super::PageError;
use super::time::format_timestamp;
use crate::dashboard::html;
use crate::dashboard::routes::Page;

pub(super) fn detail(
    board: &Board,
    id: u64,
    project: Option<u64>,
    projects: &[Project],
) -> Result<String, PageError> {
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
    Ok(task_detail(
        &task,
        &children,
        epic.as_ref(),
        &feed.events,
        project,
        projects,
    ))
}

fn task_detail(
    task: &Task,
    children: &TaskPage,
    epic: Option<&agentboard::store::Epic>,
    events: &[agentboard::domain::Event],
    project: Option<u64>,
    projects: &[Project],
) -> String {
    let mut body = String::new();
    let title = html::escape(&task.title);
    let tasks_href = html::escape(&html::url("/tasks", project));
    let _ = write!(
        body,
        "<nav class=\"breadcrumbs\" aria-label=\"Breadcrumb\"><a href=\"{tasks_href}\">Tasks</a> <span aria-hidden=\"true\">/</span> <span>T{}</span></nav><header class=\"issue-header\"><p class=\"meta\">Task T{}</p><div class=\"card-heading\"><h2>{title}</h2>{}</div></header><div class=\"detail-layout\"><div class=\"detail-main\"><section class=\"panel text-section\" id=\"goal\"><h2>Goal</h2><div class=\"full-text\">{}</div></section><section class=\"panel text-section\" id=\"description\"><h2>Description</h2><div class=\"full-text\">{}</div></section>",
        task.id,
        task.id,
        html::status_badge(task.status),
        html::escape(&task.goal),
        html::escape(&task.description)
    );
    for (id, heading, report) in [
        (
            "analyst-report",
            "Analyst report",
            task.analyst_report.as_str(),
        ),
        ("coder-report", "Coder report", task.coder_report.as_str()),
        (
            "reviewer-report",
            "Reviewer report",
            task.reviewer_report.as_str(),
        ),
    ] {
        if !report.is_empty() {
            let _ = write!(
                body,
                "<section class=\"panel text-section\" id=\"{id}\"><h2>{heading}</h2><div class=\"full-text\">{}</div></section>",
                html::escape(report)
            );
        }
    }
    let mut on_page = vec![("Goal", "goal"), ("Description", "description")];
    for (id, heading, report) in [
        (
            "analyst-report",
            "Analyst report",
            task.analyst_report.as_str(),
        ),
        ("coder-report", "Coder report", task.coder_report.as_str()),
        (
            "reviewer-report",
            "Reviewer report",
            task.reviewer_report.as_str(),
        ),
    ] {
        if !report.is_empty() {
            on_page.push((heading, id));
        }
    }
    on_page.push(("Activity", "activity"));
    let mut children_section = String::new();
    if let Some(epic) = epic {
        let maximum = epic.total.max(1);
        let progress = epic.done.min(maximum);
        let _ = write!(
            &mut children_section,
            "<section class=\"panel children-panel\"><div class=\"card-heading\"><h2>Child tasks</h2><span class=\"meta\">{} / {} done</span></div><progress max=\"{maximum}\" value=\"{progress}\" aria-label=\"{}: {} of {} direct children done\"></progress><ul class=\"strip-list\">",
            epic.done, epic.total, title, epic.done, epic.total
        );
        for child in &children.tasks {
            let href = html::escape(&html::url(&format!("/tasks/{}", child.id), project));
            let _ = write!(
                &mut children_section,
                "<li>{}</li>",
                html::strip(child, &href)
            );
        }
        children_section.push_str("</ul>");
        if children.total > children.tasks.len() {
            let href = html::escape(&html::url(&format!("/tasks?parent={}", task.id), project));
            let _ = write!(
                &mut children_section,
                "<p><a href=\"{href}\">View all {} child tasks</a></p>",
                children.total
            );
        }
        children_section.push_str("</section>");
    }
    let history_href = html::escape(&html::url(&format!("/feed?task={}", task.id), project));
    let _ = write!(
        body,
        "<section class=\"panel activity\" id=\"activity\"><div class=\"card-heading\"><h2>Activity</h2><a href=\"{history_href}\">Older history</a></div>"
    );
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
                let mut changes = Vec::new();
                if let Some(values) = event
                    .payload
                    .get("changes")
                    .and_then(serde_json::Value::as_object)
                {
                    for (field, pair) in values {
                        let old = pair.get("old").map(display_value).unwrap_or_default();
                        let new = pair.get("new").map(display_value).unwrap_or_default();
                        changes.push((field.as_str(), old, new));
                    }
                }
                if changes.is_empty() {
                    body.push_str("<p class=\"meta\">Task fields updated</p>");
                } else {
                    body.push_str("<ul class=\"change-list\">");
                    for (field, old, new) in changes {
                        let _ = write!(
                            body,
                            "<li><strong>{}</strong>: {} → {}</li>",
                            html::escape(field),
                            html::escape(&old),
                            html::escape(&new)
                        );
                    }
                    body.push_str("</ul>");
                }
            }
            body.push_str("</li>");
        }
        body.push_str("</ol>");
    }
    body.push_str("</section></div><aside class=\"detail-aside\"><nav class=\"panel on-this-page\" aria-label=\"On this page\"><h2>On this page</h2><ul>");
    for (label, id) in on_page {
        let _ = write!(body, "<li><a href=\"#{id}\">{label}</a></li>");
    }
    body.push_str("</ul></nav>");
    body.push_str(&children_section);
    body.push_str(
        "<section class=\"panel metadata\" aria-label=\"Task metadata\"><h2>Details</h2><dl>",
    );
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
        let href = html::escape(&html::url(&format!("/tasks/{parent}"), project));
        metadata_item(
            &mut body,
            "Parent",
            &format!("<a href=\"{href}\">T{parent}</a>"),
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
    body.push_str("</dl></section></aside></div>");
    html::shell(
        &format!("T{} | {}", task.id, task.title),
        Page::Detail(task.id),
        &body,
        project,
        projects,
    )
}

fn display_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(value) => value.clone(),
        serde_json::Value::Null => "—".to_owned(),
        other => other.to_string(),
    }
}

fn metadata_item(body: &mut String, label: &str, value: &str) {
    let _ = write!(
        body,
        "<div class=\"metadata-row\"><dt>{label}</dt><dd>{value}</dd></div>"
    );
}
