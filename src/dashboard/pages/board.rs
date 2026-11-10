use std::fmt::Write;

use agentboard::domain::{Project, Status, Task};
use agentboard::store::{Agent, Overview};

use super::time::format_timestamp;
use crate::dashboard::{html, routes::Page};

const SEAT_CAP: usize = 5;

pub(super) fn board_page(
    overview: &Overview,
    agents: &[Agent],
    project: Option<u64>,
    projects: &[Project],
) -> String {
    let mut body = String::new();
    let _ = write!(
        body,
        "<section class=\"attention panel\" aria-labelledby=\"attention-title\"><h2 id=\"attention-title\">Needs attention <span>{}</span></h2><ol class=\"strip-list\">",
        overview.attention_total
    );
    for task in &overview.attention {
        strip_item(&mut body, task, project);
    }
    if overview.attention.is_empty() {
        body.push_str("<li class=\"empty\">No rows need attention</li>");
    }
    body.push_str("</ol>");
    if overview.attention_total > overview.attention.len() {
        let _ = write!(
            body,
            "<a href=\"{}\">+{} more tasks</a>",
            html::escape(&html::url("/tasks", project)),
            overview.attention_total - overview.attention.len()
        );
    }
    body.push_str("</section><div class=\"board-layout\"><section class=\"seat-rack panel\" aria-labelledby=\"seats-title\"><h2 id=\"seats-title\">Seats</h2>");
    let mut names: Vec<&str> = agents.iter().map(|agent| agent.name.as_str()).collect();
    for held in &overview.held {
        if !names.contains(&held.assignee.as_str()) {
            names.push(&held.assignee);
        }
    }
    names.sort_unstable();
    for name in names {
        let held = overview.held.iter().find(|held| held.assignee == name);
        let _ = write!(
            body,
            "<section class=\"seat\"><h3>{}</h3>",
            html::escape(name)
        );
        if let Some(held) = held {
            body.push_str("<ol class=\"strip-list\">");
            for task in held.tasks.iter().take(SEAT_CAP) {
                strip_item(&mut body, task, project);
            }
            body.push_str("</ol>");
            if held.total > SEAT_CAP {
                let href = html::url(&format!("/tasks?assignee={}", html::encode(name)), project);
                let _ = write!(
                    body,
                    "<a href=\"{}\">+{} more</a>",
                    html::escape(&href),
                    held.total - SEAT_CAP
                );
            }
        } else {
            body.push_str("<p class=\"empty\">idle");
            if let Some(last_seen) = agents
                .iter()
                .find(|agent| agent.name == name)
                .and_then(|agent| agent.last_seen_ms)
            {
                let _ = write!(
                    body,
                    " · last seen <time datetime=\"{}\">{}</time>",
                    format_timestamp(last_seen),
                    format_timestamp(last_seen)
                );
            }
            body.push_str("</p>");
        }
        body.push_str("</section>");
    }
    body.push_str("</section><div class=\"status-bays\">");
    for lane in &overview.lanes {
        let status = lane.status.as_str();
        let href = html::url(&format!("/tasks?status={status}"), project);
        let _ = write!(
            body,
            "<section class=\"status-bay panel\" aria-labelledby=\"bay-{status}\"><details{}><summary><span id=\"bay-{status}\">{} {}</span></summary><ol class=\"strip-list\">",
            if lane.total <= 8 { " open" } else { "" },
            status,
            lane.total
        );
        for task in &lane.tasks {
            strip_item(&mut body, task, project);
        }
        body.push_str("</ol>");
        if lane.total > lane.tasks.len() {
            let _ = write!(
                body,
                "<a href=\"{}\">+{} more</a>",
                html::escape(&href),
                lane.total - lane.tasks.len()
            );
        }
        body.push_str("</details></section>");
    }
    body.push_str("<p class=\"closed-counts\">Closed: ");
    for status in [Status::Done, Status::Canceled] {
        let count = overview
            .counts
            .iter()
            .find(|(item, _)| *item == status)
            .map_or(0, |(_, count)| *count);
        let href = html::url(&format!("/tasks?status={}", status.as_str()), project);
        let _ = write!(
            body,
            "<a href=\"{}\">{} {count}</a> ",
            html::escape(&href),
            status.as_str()
        );
    }
    body.push_str("</p></div></div>");
    html::shell("Board", Page::Board, &body, project, projects)
}

fn strip_item(out: &mut String, task: &Task, project: Option<u64>) {
    let href = html::url(&format!("/tasks/{}", task.id), project);
    let _ = write!(out, "<li>{}</li>", html::strip(task, &href));
}
