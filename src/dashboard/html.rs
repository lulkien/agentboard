use std::fmt::Write;

use super::routes::{Filters, Page};
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

pub fn shell(title: &str, active: Page, content: &str) -> String {
    let mut out = String::with_capacity(content.len() + 1200);
    let safe_title = escape(title);
    let _ = write!(
        out,
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"color-scheme\" content=\"light dark\"><title>{safe_title} · Agentboard</title><link rel=\"stylesheet\" href=\"/assets/dashboard.css\"><script defer src=\"/assets/theme.js\"></script></head><body><header class=\"topbar\"><a class=\"brand\" href=\"/epics\">Agentboard</a><nav aria-label=\"Main navigation\"><a class=\"{}\" href=\"/epics\">Epics</a><a class=\"{}\" href=\"/tasks\">Tasks</a><a class=\"{}\" href=\"/feed\">Feed</a></nav><button class=\"theme-toggle\" type=\"button\" data-theme-toggle aria-pressed=\"false\" aria-label=\"Change color theme\">Theme: system</button></header><main><h1>{safe_title}</h1>{content}</main></body></html>",
        if active == Page::Epics { "active" } else { "" },
        if matches!(active, Page::Tasks | Page::Detail(_)) {
            "active"
        } else {
            ""
        },
        if active == Page::Feed { "active" } else { "" }
    );
    out
}

pub fn status_badge(status: Status) -> String {
    format!(
        "<span class=\"badge status-{}\">{}</span>",
        status.as_str(),
        status.as_str()
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

pub fn feed_url(task: Option<u64>, kind: Option<EventKind>, before: Option<&str>) -> String {
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
    if parts.is_empty() {
        "/feed".to_owned()
    } else {
        format!("/feed?{}", parts.join("&"))
    }
}
