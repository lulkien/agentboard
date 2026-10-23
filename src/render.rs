//! Compact text rendering shared by the CLI and the MCP tools.

use crate::domain::{Event, Task};
use crate::store::{Agent, Brief, FeedPage};

/// Width of a task title in a one-line list.
const TITLE_WIDTH: usize = 72;
/// Width of a free-text field in a compact record.
const FIELD_WIDTH: usize = 160;

/// Collapse whitespace and cut to `width`, marking the cut so a reader knows the
/// text continues. Full text is available from `get --full` / `task_get(full=true)`.
pub fn truncate(text: &str, width: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= width {
        return collapsed;
    }
    let mut out: String = collapsed.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub fn task_line(task: &Task) -> String {
    format!(
        "T{} {:<12} prio {:>4} {:<10} {}",
        task.id,
        task.status.as_str(),
        task.priority,
        task.assignee.as_deref().unwrap_or("-"),
        truncate(&task.title, TITLE_WIDTH)
    )
}

/// A compact record, or the stored text itself when `full` is set: no title clip,
/// no whitespace collapsing, no marker. A reader that asks for full text must get
/// exactly what was written.
pub fn task_record(task: &Task, full: bool) -> String {
    let mut lines = Vec::new();
    if full {
        lines.push(format!(
            "T{} {} prio {} {}",
            task.id,
            task.status.as_str(),
            task.priority,
            task.assignee.as_deref().unwrap_or("-")
        ));
        lines.push(format!("title {}", task.title));
    } else {
        lines.push(task_line(task));
    }
    lines.push(format!("revision {}", task.revision));
    if let Some(parent) = task.parent_id {
        lines.push(format!("parent T{parent}"));
    }
    if let Some(path) = task.path.as_deref() {
        lines.push(format!(
            "path {}",
            if full {
                path.to_owned()
            } else {
                truncate(path, FIELD_WIDTH)
            }
        ));
    }
    for (name, value) in [
        ("description", task.description.as_str()),
        ("goal", task.goal.as_str()),
    ] {
        if !value.is_empty() {
            lines.push(format!(
                "{name} {}",
                if full {
                    value.to_owned()
                } else {
                    truncate(value, FIELD_WIDTH)
                }
            ));
        }
    }
    lines.join("\n")
}

pub fn event_line(event: &Event, full: bool) -> String {
    let target = format!("T{}", event.task_id);
    let mut line = format!(
        "{} {} {:<10} {:<8} {}",
        event.seq,
        event.timestamp_ms,
        event.actor,
        event.kind_str(),
        target
    );
    if let Some(recipient) = event.recipient.as_deref() {
        line.push_str(&format!(" -> {recipient}"));
    }
    let body = event
        .payload
        .get("text")
        .or_else(|| event.payload.get("comment"))
        .or_else(|| event.payload.get("reason"))
        .and_then(|value| value.as_str());
    if let Some(body) = body {
        let shown = if full {
            body.to_owned()
        } else {
            truncate(body, FIELD_WIDTH)
        };
        line.push_str(&format!(" | {shown}"));
    }
    line
}

pub fn feed_page(page: &FeedPage, full: bool) -> String {
    let mut lines: Vec<String> = page
        .events
        .iter()
        .map(|event| event_line(event, full))
        .collect();
    lines.push(format!("cursor {}", page.next.encode()));
    if page.more {
        lines.push("more true".into());
    }
    if page.older {
        lines.push("older true".into());
    }
    lines.join("\n")
}

pub fn brief_text(brief: &Brief) -> String {
    let counts: Vec<String> = brief
        .counts
        .iter()
        .map(|(status, count)| format!("{} {count}", status.as_str()))
        .collect();
    let mut lines = vec![counts.join("  ")];
    for task in &brief.active {
        lines.push(task_line(task));
    }
    lines.join("\n")
}

pub fn agent_line(agent: &Agent) -> String {
    let seen = agent
        .last_seen_ms
        .map_or_else(|| "never".to_owned(), |ms| ms.to_string());
    let task = agent
        .last_task_id
        .map_or_else(|| "-".to_owned(), |id| format!("T{id}"));
    format!(
        "{:<20} last_seen {:<14} last_task {}",
        agent.name, seen, task
    )
}

pub fn claim_line(outcome: &crate::store::ClaimOutcome) -> String {
    match outcome {
        crate::store::ClaimOutcome::Claimed(updated) => task_record(&updated.task, false),
        crate::store::ClaimOutcome::AlreadyHeld(task) => task_record(task, false),
        crate::store::ClaimOutcome::NoWork => "no ready tasks".to_owned(),
    }
}
