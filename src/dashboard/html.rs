use std::fmt::Write;

use super::template::{Fragment, Template, html as html_slot, text as text_slot};
use agentboard::domain::Project;
use agentboard::domain::{EventKind, Status};
use agentboard::routes::{Filters, Page};

const SHELL: Template = Template::new("shell", include_str!("templates/shell.html"));
const PROJECT_OPTION: Template = Template::new(
    "project-option",
    include_str!("templates/project-option.html"),
);
const BADGE: Template = Template::new("badge", include_str!("templates/badge.html"));
const STRIP: Template = Template::new("strip", include_str!("templates/strip.html"));

pub fn shell(
    title: &str,
    active: Page,
    content: Fragment,
    project: Option<u64>,
    projects: &[Project],
) -> Fragment {
    shell_with_aside(title, active, Fragment::empty(), content, project, projects)
}

pub fn shell_with_aside(
    title: &str,
    active: Page,
    aside: Fragment,
    content: Fragment,
    project: Option<u64>,
    projects: &[Project],
) -> Fragment {
    let board_url = url("/", project);
    let epics_url = url("/epics", project);
    let tasks_url = url("/tasks", project);
    let feed_url = url("/feed", project);
    let action = match active {
        Page::Board => "/",
        Page::Epics => "/epics",
        Page::Tasks => "/tasks",
        Page::Detail(_) => "/tasks",
        Page::Feed => "/feed",
    };
    let project_options: Fragment = projects
        .iter()
        .map(|item| {
            PROJECT_OPTION.fill(&[
                ("id", text_slot(item.project_id)),
                (
                    "selected",
                    text_slot(if project == Some(item.project_id) {
                        " selected"
                    } else {
                        ""
                    }),
                ),
                ("name", text_slot(&item.project_name)),
            ])
        })
        .collect();
    let board_active = active == Page::Board;
    let epics_active = active == Page::Epics;
    let tasks_active = matches!(active, Page::Tasks | Page::Detail(_));
    let feed_active = active == Page::Feed;
    SHELL.fill(&[
        ("title", text_slot(title)),
        ("board_url", text_slot(board_url)),
        ("epics_url", text_slot(epics_url)),
        ("tasks_url", text_slot(tasks_url)),
        ("feed_url", text_slot(feed_url)),
        (
            "board_class",
            text_slot(if board_active { "active" } else { "" }),
        ),
        ("board_current", html_slot(current_page(board_active))),
        (
            "epics_class",
            text_slot(if epics_active { "active" } else { "" }),
        ),
        ("epics_current", html_slot(current_page(epics_active))),
        (
            "tasks_class",
            text_slot(if tasks_active { "active" } else { "" }),
        ),
        ("tasks_current", html_slot(current_page(tasks_active))),
        (
            "feed_class",
            text_slot(if feed_active { "active" } else { "" }),
        ),
        ("feed_current", html_slot(current_page(feed_active))),
        ("action", text_slot(action)),
        (
            "all_projects_selected",
            html_slot(if project.is_none() {
                Fragment::lit(" selected")
            } else {
                Fragment::empty()
            }),
        ),
        ("project_options", html_slot(project_options)),
        ("head_aside", html_slot(aside)),
        ("content", html_slot(content)),
    ])
}

fn current_page(active: bool) -> Fragment {
    if active {
        Fragment::lit(" aria-current=\"page\"")
    } else {
        Fragment::empty()
    }
}

pub fn url(path: &str, project: Option<u64>) -> String {
    if let Some(project) = project {
        format!(
            "{path}{}project={project}",
            if path.contains('?') { '&' } else { '?' }
        )
    } else {
        path.to_owned()
    }
}

pub fn status_badge(status: Status) -> Fragment {
    BADGE.fill(&[
        ("status", text_slot(status.as_str())),
        ("glyph", text_slot(status_glyph(status))),
    ])
}

pub fn status_glyph(status: Status) -> char {
    match status {
        Status::Todo => '○',
        Status::Planning => '◇',
        Status::Implementing => '▶',
        Status::Blocked => '■',
        Status::Reviewing => '◎',
        Status::Done => '✓',
        Status::Canceled => '✕',
    }
}

pub fn strip(task: &agentboard::domain::Task, href: &str) -> Fragment {
    let status = task.status.as_str();
    STRIP.fill(&[
        ("status", text_slot(status)),
        ("href", text_slot(href)),
        ("title", text_slot(&task.title)),
        ("glyph", text_slot(status_glyph(task.status))),
        ("id", text_slot(task.id)),
    ])
}

pub fn query_url(filters: &Filters, offset: Option<usize>) -> String {
    let mut parts = Vec::new();
    if let Some(status) = filters.status {
        parts.push(format!("status={}", status.as_str()));
    }
    if let Some(assignee) = &filters.assignee {
        parts.push(format!("assignee={}", encode(assignee)));
    }
    if let Some(parent) = filters.parent {
        parts.push(format!("parent={parent}"));
    }
    if let Some(offset) = offset {
        parts.push(format!("offset={offset}"));
    }
    if let Some(project) = filters.project {
        parts.push(format!("project={project}"));
    }
    if parts.is_empty() {
        "/tasks".to_owned()
    } else {
        format!("/tasks?{}", parts.join("&"))
    }
}

pub fn encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

pub fn feed_url(
    task: Option<u64>,
    kind: Option<EventKind>,
    before: Option<&str>,
    project: Option<u64>,
) -> String {
    let mut parts = Vec::new();
    if let Some(task) = task {
        parts.push(format!("task={task}"));
    }
    if let Some(kind) = kind {
        parts.push(format!("kind={}", kind.as_str()));
    }
    if let Some(before) = before {
        parts.push(format!("before={}", encode(before)));
    }
    if let Some(project) = project {
        parts.push(format!("project={project}"));
    }
    if parts.is_empty() {
        "/feed".to_owned()
    } else {
        format!("/feed?{}", parts.join("&"))
    }
}
