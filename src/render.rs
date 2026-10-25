//! Compact text rendering shared by the CLI and the MCP tools.

use crate::domain::{Event, Status, Task};
use crate::store::{Agent, Brief, Epic, FeedPage, WaitOutcome};

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
    let mut lines: Vec<String> = brief.epics.iter().map(epic_line).collect();
    for task in &brief.active {
        lines.push(task_line(task));
    }
    lines.push(counts_text(&brief.counts));
    lines.join("\n")
}

fn counts_text(counts: &[(Status, u64)]) -> String {
    counts
        .iter()
        .map(|(status, count)| format!("{} {count}", status.as_str()))
        .collect::<Vec<_>>()
        .join("  ")
}

pub fn epic_line(epic: &Epic) -> String {
    format!(
        "{}  done {}/{}",
        task_line(&epic.task),
        epic.done,
        epic.total
    )
}

pub fn epic_text(epic: &Epic) -> String {
    format!(
        "{}\n{}\ndone {}/{}\ntotal {} cursor {}",
        task_line(&epic.task),
        counts_text(&epic.counts),
        epic.done,
        epic.total,
        epic.total,
        epic.cursor.encode()
    )
}

/// The verdict, one compact row per watched row, then the cursor to follow on with.
/// A row that was already wanted when the wait started shows its newest earlier
/// event as `last:` — context, never the cause of the return.
pub fn wait_text(outcome: &WaitOutcome) -> String {
    let in_wanted = outcome
        .rows
        .iter()
        .filter(|row| outcome.wanted.contains(&row.task.status))
        .count();
    let watched = outcome.rows.len();
    let wanted = outcome
        .wanted
        .iter()
        .map(|status| status.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let mut lines = vec![if outcome.matched {
        format!("matched: {in_wanted} of {watched} watched in {wanted}")
    } else {
        format!(
            "timeout: {in_wanted} of {watched} watched in {wanted} after {}ms",
            outcome.waited_ms
        )
    }];
    for row in &outcome.rows {
        lines.push(task_line(&row.task));
        if !row.events.is_empty() {
            lines.extend(
                row.events
                    .iter()
                    .map(|event| format!("  {}", event_line(event, false))),
            );
            if row.omitted > 0 {
                lines.push(format!("  … {} more events", row.omitted));
            }
        } else if outcome.matched
            && let Some(last) = row.last.as_ref()
        {
            lines.push(format!("  last: {}", event_line(last, false)));
        }
    }
    lines.push(format!("cursor {}", outcome.cursor.encode()));
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
