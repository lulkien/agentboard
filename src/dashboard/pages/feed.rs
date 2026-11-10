use std::fmt::Write;

use agentboard::domain::{EventKind, Project};
use agentboard::store::{Board, Cursor, FeedFilter};

use super::time::format_timestamp;
use super::{PAGE_SIZE, PageError};
use crate::dashboard::html;
use crate::dashboard::routes::{Filters, Page};

pub(super) fn feed(
    board: &Board,
    filters: Filters,
    projects: &[Project],
) -> Result<String, PageError> {
    let filter = FeedFilter {
        task: filters.feed_task,
        kind: filters.feed_kind,
        recipient: None,
        after: None,
        limit: PAGE_SIZE,
    };
    if let Some(project) = filters.project {
        let page = board
            .feed_in_project(&filter, project, filters.before.as_deref(), PAGE_SIZE)
            .map_err(feed_error)?;
        return Ok(feed_page(
            &filter,
            &page.events,
            page.older,
            Some(page.before),
            filters.project,
            projects,
        ));
    }
    if let Some(before) = filters.before.as_deref() {
        let page = board.feed_before(&filter, before).map_err(feed_error)?;
        Ok(feed_page(
            &filter,
            &page.events,
            page.older,
            Some(page.before),
            None,
            projects,
        ))
    } else {
        let page = board
            .feed(&filter)
            .map_err(|error| PageError::Store(error.to_string()))?;
        let before = page.events.first().map(|event| Cursor {
            instance_id: page.next.instance_id.clone(),
            seq: event.seq,
        });
        Ok(feed_page(
            &filter,
            &page.events,
            page.older,
            before,
            None,
            projects,
        ))
    }
}

fn feed_error(error: agentboard::store::Error) -> PageError {
    match error {
        agentboard::store::Error::CursorInstanceMismatch
        | agentboard::store::Error::InvalidCursor(_) => PageError::BadRequest,
        other => PageError::Store(other.to_string()),
    }
}

fn feed_page(
    filter: &FeedFilter<'_>,
    events: &[agentboard::domain::Event],
    older: bool,
    before: Option<Cursor>,
    project: Option<u64>,
    projects: &[Project],
) -> String {
    let task_id = filter.task;
    let kind = filter.kind;
    let mut body = String::from(
        "<section class=\"panel feed-panel\"><form id=\"feed-filter\" method=\"get\" action=\"/feed\"><label>Task ID<input name=\"task\" inputmode=\"numeric\" value=\"",
    );
    if let Some(task) = task_id {
        let _ = write!(body, "{task}");
    }
    body.push_str(
        "\"></label><label>Event kind<select name=\"kind\"><option value=\"\">All events</option>",
    );
    for event_kind in EventKind::ALL {
        let selected = kind == Some(event_kind);
        let _ = write!(
            body,
            "<option value=\"{}\"{}>{}</option>",
            event_kind.as_str(),
            if selected { " selected" } else { "" },
            event_kind.as_str()
        );
    }
    let _ = write!(
        body,
        "</select></label><button type=\"submit\">Apply filters</button></form><p class=\"result-count\">{} matching events shown</p>",
        events.len()
    );
    if let Some(project) = project {
        let _ = write!(
            body,
            "<input form=\"feed-filter\" type=\"hidden\" name=\"project\" value=\"{project}\">"
        );
    }
    if events.is_empty() {
        body.push_str("<p class=\"empty\">No matching events</p>");
    } else {
        body.push_str("<ol class=\"timeline feed-timeline\">");
        for event in events {
            render_feed_event(&mut body, event, project);
        }
        body.push_str("</ol>");
    }
    body.push_str("<nav class=\"pagination\" aria-label=\"Feed pages\">");
    if let Some(before) = before.filter(|_| older) {
        let href = html::escape(&html::feed_url(
            task_id,
            kind,
            Some(&before.encode()),
            project,
        ));
        let _ = write!(body, "<a href=\"{href}\">Older events</a>");
    }
    let newest = html::escape(&html::feed_url(task_id, kind, None, project));
    let _ = write!(body, "<a href=\"{newest}\">Newest</a></nav></section>");
    html::shell("Feed", Page::Feed, &body, project, projects)
}

fn render_feed_event(body: &mut String, event: &agentboard::domain::Event, project: Option<u64>) {
    let actor = html::escape(&event.actor);
    let kind = event.kind.as_str();
    let timestamp = format_timestamp(event.timestamp_ms);
    let href = html::escape(&html::url(&format!("/tasks/{}", event.task_id), project));
    let _ = write!(
        body,
        "<li><article class=\"feed-event\"><header class=\"timeline-heading\"><span class=\"badge event-kind\">{kind}</span><a href=\"{href}\">T{}</a><span class=\"meta\">by {actor}</span><time datetime=\"{timestamp}\">{timestamp}</time></header>",
        event.task_id
    );
    if let Some(text) = event
        .payload
        .get("text")
        .and_then(serde_json::Value::as_str)
    {
        let _ = write!(
            body,
            "<div class=\"full-text event-text\">{}</div>",
            html::escape(text)
        );
    } else {
        let mut has_text = false;
        for (label, key) in [("Comment", "comment"), ("Reason", "reason")] {
            if let Some(text) = event.payload.get(key).and_then(serde_json::Value::as_str) {
                let _ = write!(
                    body,
                    "<p class=\"full-text event-text\"><strong>{label}:</strong> {}</p>",
                    html::escape(text)
                );
                has_text = true;
            }
        }
        if let Some(changes) = event.payload.get("changes")
            && let Ok(text) = serde_json::to_string_pretty(changes)
        {
            let _ = write!(
                body,
                "<pre class=\"full-text event-text\">{}</pre>",
                html::escape(&text)
            );
            has_text = true;
        }
        if !has_text {
            let description = match event.kind {
                EventKind::Created => "Task created",
                EventKind::Claimed => "Task claimed",
                EventKind::Updated => "Task updated",
                EventKind::Commented => "Comment added",
            };
            let _ = write!(body, "<p class=\"meta\">{description}</p>");
        }
    }
    body.push_str("</article></li>");
}
