use std::fmt::Write;

use agentboard::domain::{Project, Status};
use agentboard::store::{Board, Brief, Epic, ListFilter};

use super::PageError;
use crate::dashboard::html;
use crate::dashboard::routes::Page;

const CHILD_STRIP_CAP: usize = 4;

pub(super) fn epics(
    board: &Board,
    brief: &Brief,
    project: Option<u64>,
    projects: &[Project],
) -> Result<String, PageError> {
    let mut body = String::from("<section class=\"epic-list\" aria-label=\"Epics\">");
    if brief.epics.is_empty() {
        body.push_str("<p class=\"empty panel\">No epics yet</p>");
    } else {
        for epic in &brief.epics {
            let depth = epic_depth(epic, &brief.epics).min(6);
            let children = board
                .list(&ListFilter {
                    status: None,
                    assignee: None,
                    parent: Some(epic.task.id),
                    project,
                    include_terminal: true,
                    limit: CHILD_STRIP_CAP,
                    offset: 0,
                })
                .map_err(|e| PageError::Store(e.to_string()))?;
            render_epic(
                &mut body,
                epic,
                &children.tasks,
                children.total,
                depth,
                project,
            );
        }
    }
    body.push_str("</section>");
    Ok(html::shell("Epics", Page::Epics, &body, project, projects))
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

fn render_epic(
    body: &mut String,
    epic: &Epic,
    children: &[agentboard::domain::Task],
    child_total: usize,
    depth: usize,
    project: Option<u64>,
) {
    let task = &epic.task;
    let href = html::url(&format!("/tasks?parent={}", task.id), project);
    let strip = html::strip(task, &href);
    let parent_link = task
        .parent_id
        .map(|parent| {
            let href = html::escape(&html::url(&format!("/tasks/{parent}"), project));
            format!("<a class=\"meta\" href=\"{href}\">in T{parent}</a>")
        })
        .unwrap_or_default();
    let total = epic.total.max(1);
    let _ = write!(
        body,
        "<article class=\"epic-row depth-{depth}\"><div class=\"epic-heading\">{strip}{parent_link}<span class=\"meta\">Priority {}</span></div><div class=\"epic-rollup\"><svg class=\"rollup-bar\" viewBox=\"0 0 100 10\" role=\"img\" aria-label=\"{} of {} direct children done\" preserveAspectRatio=\"none\">",
        task.priority, epic.done, epic.total
    );
    let mut x = 0.0_f64;
    for status in Status::ALL {
        let count = epic
            .counts
            .iter()
            .find_map(|(found, count)| (*found == status).then_some(*count))
            .unwrap_or(0);
        if count == 0 {
            continue;
        }
        let width = (count as f64 / total as f64) * 100.0;
        let _ = write!(
            body,
            "<rect class=\"status-{}\" x=\"{x:.3}\" y=\"0\" width=\"{width:.3}\" height=\"10\"></rect>",
            status.as_str()
        );
        x += width;
    }
    let _ = write!(
        body,
        "</svg><span class=\"progress-copy\">{} / {} done</span><ul class=\"counts\">",
        epic.done, epic.total
    );
    for status in Status::ALL {
        let count = epic
            .counts
            .iter()
            .find_map(|(found, count)| (*found == status).then_some(*count))
            .unwrap_or(0);
        if count > 0 {
            let _ = write!(
                body,
                "<li class=\"status-{}\">{}: <strong>{count}</strong></li>",
                status.as_str(),
                status.as_str()
            );
        }
    }
    let children_href = html::escape(&html::url(&format!("/tasks?parent={}", task.id), project));
    let _ = write!(
        body,
        "</ul><a href=\"{children_href}\">All children</a></div><ul class=\"strip-list epic-children\">"
    );
    for child in children {
        let href = html::url(&format!("/tasks/{}", child.id), project);
        let _ = write!(body, "<li>{}</li>", html::strip(child, &href));
    }
    body.push_str("</ul>");
    if child_total > children.len() {
        let more = child_total - children.len();
        let href = html::escape(&html::url(&format!("/tasks?parent={}", task.id), project));
        let _ = write!(
            body,
            "<p class=\"epic-more\"><a href=\"{href}\">+{more} more</a></p>"
        );
    }
    body.push_str("</article>");
}
