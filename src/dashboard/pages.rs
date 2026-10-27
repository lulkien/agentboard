use std::fmt::Write;

use agentboard::domain::{EventKind, Status, Task};
use agentboard::store::{Board, Brief, Cursor, FeedFilter, ListFilter, MAX_LIST_LIMIT, TaskPage};

use super::html;
use super::routes::{Filters, Page, Route};

const PAGE_SIZE: usize = 20;

pub enum PageError {
    Missing,
    BadRequest,
    Store(String),
}

pub fn render(board: &Board, route: Route) -> Result<String, PageError> {
    match route.page {
        Page::Epics => {
            let brief = board
                .brief(0)
                .map_err(|e| PageError::Store(e.to_string()))?;
            Ok(epics(&brief))
        }
        Page::Tasks => {
            if let Some(parent) = route.filters.parent {
                board
                    .get(parent)
                    .map_err(|e| PageError::Store(e.to_string()))?
                    .ok_or(PageError::Missing)?;
            }
            let filters = route.filters;
            let page = board
                .list_newest(&ListFilter {
                    status: filters.status,
                    assignee: filters.assignee.as_deref(),
                    parent: filters.parent,
                    path: None,
                    include_terminal: true,
                    limit: PAGE_SIZE.min(MAX_LIST_LIMIT),
                    offset: filters.offset,
                })
                .map_err(|e| PageError::Store(e.to_string()))?;
            Ok(tasks(&page, &filters))
        }
        Page::Detail(id) => detail(board, id),
        Page::Feed => feed(board, route.filters),
    }
}

fn detail(board: &Board, id: u64) -> Result<String, PageError> {
    let task = board
        .get(id)
        .map_err(|e| PageError::Store(e.to_string()))?
        .ok_or(PageError::Missing)?;
    let child_filter = ListFilter {
        status: None,
        assignee: None,
        parent: Some(id),
        path: None,
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

fn feed(board: &Board, filters: Filters) -> Result<String, PageError> {
    let filter = FeedFilter {
        task: filters.feed_task,
        kind: filters.feed_kind,
        recipient: None,
        after: None,
        limit: PAGE_SIZE,
    };
    if let Some(before) = filters.before.as_deref() {
        let page = board.feed_before(&filter, before).map_err(feed_error)?;
        Ok(feed_page(
            &filter,
            &page.events,
            page.older,
            Some(page.before),
        ))
    } else {
        let page = board
            .feed(&filter)
            .map_err(|error| PageError::Store(error.to_string()))?;
        let before = page.events.first().map(|event| Cursor {
            instance_id: page.next.instance_id.clone(),
            seq: event.seq,
        });
        Ok(feed_page(&filter, &page.events, page.older, before))
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
) -> String {
    let task_id = filter.task;
    let kind = filter.kind;
    let mut body = String::from(
        "<section class=\"panel feed-panel\"><form method=\"get\" action=\"/feed\"><label>Task ID<input name=\"task\" inputmode=\"numeric\" value=\"",
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
    if events.is_empty() {
        body.push_str("<p class=\"empty\">No matching events</p>");
    } else {
        body.push_str("<ol class=\"timeline feed-timeline\">");
        for event in events {
            render_feed_event(&mut body, event);
        }
        body.push_str("</ol>");
    }
    body.push_str("<nav class=\"pagination\" aria-label=\"Feed pages\">");
    if let Some(before) = before.filter(|_| older) {
        let href = html::escape(&html::feed_url(task_id, kind, Some(&before.encode())));
        let _ = write!(body, "<a href=\"{href}\">Older events</a>");
    }
    let newest = html::escape(&html::feed_url(task_id, kind, None));
    let _ = write!(body, "<a href=\"{newest}\">Newest</a></nav></section>");
    html::shell("Feed", Page::Feed, &body)
}

fn render_feed_event(body: &mut String, event: &agentboard::domain::Event) {
    let actor = html::escape(&event.actor);
    let kind = event.kind.as_str();
    let timestamp = format_timestamp(event.timestamp_ms);
    let _ = write!(
        body,
        "<li><article class=\"feed-event\"><header class=\"timeline-heading\"><span class=\"badge event-kind\">{kind}</span><a href=\"/tasks/{}\">T{}</a><span class=\"meta\">by {actor} · <time datetime=\"{timestamp}\">{timestamp}</time></span></header>",
        event.task_id, event.task_id
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
                "<li><div class=\"timeline-heading\"><strong>{}</strong><span class=\"meta\">{} · <time datetime=\"{timestamp}\">{timestamp}</time></span></div>",
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
        "Path",
        &html::escape(task.path.as_deref().unwrap_or("—")),
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
        &format!("T{} · {}", task.id, task.title),
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

fn format_timestamp(timestamp_ms: i64) -> String {
    let days = timestamp_ms.div_euclid(86_400_000);
    let millis = timestamp_ms.rem_euclid(86_400_000);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let hour = millis / 3_600_000;
    let minute = millis / 60_000 % 60;
    let second = millis / 1_000 % 60;
    let subsecond = millis % 1_000;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{subsecond:03}Z")
}

fn epics(brief: &Brief) -> String {
    let mut body = String::from("<section class=\"card-grid\" aria-label=\"Epics\">");
    if brief.epics.is_empty() {
        body.push_str("<p class=\"empty card\">No epics yet</p>");
    }
    for epic in &brief.epics {
        let max = epic.total.max(1);
        let value = epic.done.min(max);
        let title = html::escape(&epic.task.title);
        let _ = write!(
            body,
            "<article class=\"card epic-card\"><div class=\"card-heading\"><h2><a href=\"/tasks?parent={}\">{}</a></h2>{}</div><p class=\"meta\">T{} · priority {}</p><p class=\"progress-copy\">{} / {} done</p><progress max=\"{max}\" value=\"{value}\" aria-label=\"{}: {} of {} direct children done\"></progress><ul class=\"counts\">",
            epic.task.id,
            title,
            html::status_badge(epic.task.status),
            epic.task.id,
            epic.task.priority,
            epic.done,
            epic.total,
            title,
            epic.done,
            epic.total
        );
        for status in Status::ALL {
            let count = epic
                .counts
                .iter()
                .find_map(|(s, n)| (*s == status).then_some(n))
                .copied()
                .unwrap_or(0);
            if count > 0 {
                let _ = write!(
                    body,
                    "<li>{}: <strong>{count}</strong></li>",
                    status.as_str()
                );
            }
        }
        body.push_str("</ul><a class=\"button-link\" href=\"/tasks?parent=");
        let _ = write!(body, "{}\">View child tasks</a></article>", epic.task.id);
    }
    body.push_str("</section>");
    html::shell("Epics", Page::Epics, &body)
}

fn tasks(page: &TaskPage, filters: &Filters) -> String {
    let mut body = String::from(
        "<section class=\"panel\"><form method=\"get\" action=\"/tasks\"><label>Status<select name=\"status\"><option value=\"\">All statuses</option>",
    );
    for status in Status::ALL {
        let selected = filters.status == Some(status);
        let _ = write!(
            body,
            "<option value=\"{}\"{}>{}</option>",
            status.as_str(),
            if selected { " selected" } else { "" },
            status.as_str()
        );
    }
    let assignee = html::escape(filters.assignee.as_deref().unwrap_or_default());
    let parent = filters.parent.map(|id| id.to_string()).unwrap_or_default();
    let _ = write!(
        body,
        "</select></label><label>Assignee<input name=\"assignee\" value=\"{assignee}\" autocomplete=\"off\"></label><label>Parent ID<input name=\"parent\" inputmode=\"numeric\" value=\"{parent}\"></label><button type=\"submit\">Apply filters</button></form>"
    );
    if let Some(id) = filters.parent {
        let _ = write!(
            body,
            "<p class=\"filter-note\">Showing direct children of epic <a href=\"/tasks?parent={id}\">T{id}</a></p>"
        );
    }
    let offset = filters.offset.min(page.total);
    let start = if page.tasks.is_empty() { 0 } else { offset + 1 };
    let _ = write!(
        body,
        "<p class=\"result-count\">Showing {start}–{} of {} tasks</p><div class=\"table-scroll\"><table><thead><tr><th scope=\"col\">ID</th><th scope=\"col\">Status</th><th scope=\"col\">Title</th><th scope=\"col\">Assignee</th><th scope=\"col\">Path</th><th scope=\"col\">Priority</th></tr></thead><tbody>",
        offset + page.tasks.len(),
        page.total
    );
    if page.tasks.is_empty() {
        body.push_str(
            "<tr><td colspan=\"6\" class=\"empty\">No tasks match these filters</td></tr>",
        );
    }
    for task in &page.tasks {
        render_task_row(&mut body, task);
    }
    body.push_str("</tbody></table></div><nav class=\"pagination\" aria-label=\"Task pages\">");
    if offset > 0 {
        let _ = write!(
            body,
            "<a href=\"{}\">Previous</a>",
            html::query_url(filters, Some(offset.saturating_sub(PAGE_SIZE)))
        );
    }
    if page.more {
        let _ = write!(
            body,
            "<a href=\"{}\">Next</a>",
            html::query_url(filters, Some(offset.saturating_add(page.tasks.len())))
        );
    }
    body.push_str("</nav></section>");
    html::shell("Tasks", Page::Tasks, &body)
}

fn render_task_row(out: &mut String, task: &Task) {
    let assignee = html::escape(task.assignee.as_deref().unwrap_or("—"));
    let path = html::escape(task.path.as_deref().unwrap_or("—"));
    let title = html::escape(&task.title);
    let _ = write!(
        out,
        "<tr><td><a href=\"/tasks/{}\">T{}</a></td><td>{}</td><td>{title}</td><td>{assignee}</td><td class=\"path\">{path}</td><td class=\"priority\">{}</td></tr>",
        task.id,
        task.id,
        html::status_badge(task.status),
        task.priority
    );
}
