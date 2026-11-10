use std::fmt::Write;

use super::routes::{Filters, Page};
use agentboard::domain::Project;
use agentboard::domain::{EventKind, Status};

pub fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

pub fn shell(
    title: &str,
    active: Page,
    content: &str,
    project: Option<u64>,
    projects: &[Project],
) -> String {
    let mut out = String::with_capacity(content.len() + 1200);
    let safe_title = escape(title);
    let board_url = escape(&url("/", project));
    let epics_url = escape(&url("/epics", project));
    let tasks_url = escape(&url("/tasks", project));
    let feed_url = escape(&url("/feed", project));
    let _ = write!(
        out,
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"color-scheme\" content=\"light dark\"><title>{safe_title} | Agentboard</title><link rel=\"stylesheet\" href=\"/assets/dashboard.css\"><script defer src=\"/assets/theme.js\"></script></head><body><a class=\"skip-link\" href=\"#main\">Skip to content</a><header class=\"topbar\"><a class=\"brand\" href=\"{board_url}\">Agentboard</a><nav aria-label=\"Main navigation\"><a class=\"{}\" href=\"{board_url}\"{}>Board</a><a class=\"{}\" href=\"{epics_url}\"{}>Epics</a><a class=\"{}\" href=\"{tasks_url}\"{}>Tasks</a><a class=\"{}\" href=\"{feed_url}\"{}>Feed</a></nav>",
        if active == Page::Board { "active" } else { "" },
        if active == Page::Board {
            " aria-current=\"page\""
        } else {
            ""
        },
        if active == Page::Epics { "active" } else { "" },
        if active == Page::Epics {
            " aria-current=\"page\""
        } else {
            ""
        },
        if matches!(active, Page::Tasks | Page::Detail(_)) {
            "active"
        } else {
            ""
        },
        if matches!(active, Page::Tasks | Page::Detail(_)) {
            " aria-current=\"page\""
        } else {
            ""
        },
        if active == Page::Feed { "active" } else { "" },
        if active == Page::Feed {
            " aria-current=\"page\""
        } else {
            ""
        }
    );
    let action = match active {
        Page::Board => "/",
        Page::Epics => "/epics",
        Page::Tasks => "/tasks",
        Page::Detail(_) => "/tasks",
        Page::Feed => "/feed",
    };
    let _ = write!(
        out,
        "<form class=\"project-switcher\" method=\"get\" action=\"{action}\"><select class=\"header-control\" name=\"project\" aria-label=\"Project\"><option value=\"\"{}>All projects</option>",
        if project.is_none() { " selected" } else { "" }
    );
    for item in projects {
        let _ = write!(
            out,
            "<option value=\"{}\"{}>{}</option>",
            item.project_id,
            if project == Some(item.project_id) {
                " selected"
            } else {
                ""
            },
            escape(&item.project_name)
        );
    }
    out.push_str("</select><button class=\"header-control\" type=\"submit\">Show</button></form></header><main id=\"main\"><h1>");
    out.push_str(&safe_title);
    out.push_str("</h1>");
    out.push_str(content);
    out.push_str("</main><footer><button class=\"header-control theme-toggle\" type=\"button\" data-theme-toggle aria-pressed=\"false\" aria-label=\"Change theme\" title=\"Change theme\">Change theme</button></footer></body></html>");
    out
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

pub fn status_badge(status: Status) -> String {
    format!(
        "<span class=\"badge status-{}\"><span aria-hidden=\"true\">{}</span> {}</span>",
        status.as_str(),
        status_glyph(status),
        status.as_str()
    )
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

pub fn strip(task: &agentboard::domain::Task, href: &str) -> String {
    let status = task.status.as_str();
    let title = escape(&task.title);
    let href = escape(href);
    format!(
        "<a class=\"task-strip status-{status}\" href=\"{href}\" title=\"{title}\"><span class=\"strip-glyph\" aria-hidden=\"true\">{}</span><span class=\"strip-id\">T{}</span><span class=\"strip-title\">{title}</span><span class=\"visually-hidden\">{status}</span></a>",
        status_glyph(task.status),
        task.id
    )
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
