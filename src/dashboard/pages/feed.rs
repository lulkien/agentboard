use agentboard::domain::{EventKind, Project};
use agentboard::store::{Board, Cursor, FeedFilter};

use super::time::format_timestamp;
use super::{PAGE_SIZE, PageError};
use crate::dashboard::html;
use crate::dashboard::routes::{Filters, Page};
use crate::dashboard::template::{Fragment, Template, html as html_slot, text};

const FEED: Template = Template::new("feed", include_str!("../templates/feed.html"));
const OPTION: Template =
    Template::new("feed-option", include_str!("../templates/feed-option.html"));
const PROJECT_INPUT: Template = Template::new(
    "feed-project-input",
    include_str!("../templates/feed-project-input.html"),
);
const EMPTY: Template = Template::new("feed-empty", include_str!("../templates/feed-empty.html"));
const EVENT_LIST: Template = Template::new(
    "feed-event-list",
    include_str!("../templates/feed-event-list.html"),
);
const OLDER: Template = Template::new("feed-older", include_str!("../templates/feed-older.html"));
const NEWEST: Template =
    Template::new("feed-newest", include_str!("../templates/feed-newest.html"));
const EVENT: Template = Template::new("feed-event", include_str!("../templates/feed-event.html"));
const EVENT_TEXT: Template = Template::new(
    "feed-event-text",
    include_str!("../templates/feed-event-text.html"),
);
const LABELED_TEXT: Template = Template::new(
    "feed-event-labeled-text",
    include_str!("../templates/feed-event-labeled-text.html"),
);
const CHANGES: Template = Template::new(
    "feed-event-changes",
    include_str!("../templates/feed-event-changes.html"),
);
const DESCRIPTION: Template = Template::new(
    "feed-event-description",
    include_str!("../templates/feed-event-description.html"),
);

pub(super) fn feed(
    board: &Board,
    filters: Filters,
    projects: &[Project],
) -> Result<Fragment, PageError> {
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
            &filters,
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
            &filters,
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
            &filters,
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
    filters: &Filters,
    projects: &[Project],
) -> Fragment {
    let project = filters.project;
    let kind_options: Fragment = EventKind::ALL
        .into_iter()
        .map(|event_kind| {
            OPTION.fill(&[
                ("kind", text(event_kind.as_str())),
                (
                    "selected",
                    html_slot(if filter.kind == Some(event_kind) {
                        Fragment::lit(" selected")
                    } else {
                        Fragment::empty()
                    }),
                ),
            ])
        })
        .collect();
    let project_input = project.map_or_else(Fragment::empty, |project| {
        PROJECT_INPUT.fill(&[("project", text(project))])
    });
    let event_list = if events.is_empty() {
        EMPTY.fill(&[])
    } else {
        let rendered_events: Fragment = events
            .iter()
            .map(|event| render_feed_event(event, filters))
            .collect();
        EVENT_LIST.fill(&[("events", html_slot(rendered_events))])
    };
    let older_link = before
        .filter(|_| older)
        .map_or_else(Fragment::empty, |before| {
            let href = html::feed_url(filter.task, filter.kind, Some(&before.encode()), project);
            OLDER.fill(&[("href", text(href))])
        });
    let newest = html::feed_url(filter.task, filter.kind, None, project);
    let newest_link = NEWEST.fill(&[("href", text(newest))]);
    let body = FEED.fill(&[
        (
            "task_id",
            text(filter.task.map(|task| task.to_string()).unwrap_or_default()),
        ),
        ("kind_options", html_slot(kind_options)),
        ("project_input", html_slot(project_input)),
        ("event_count", text(events.len())),
        ("event_list", html_slot(event_list)),
        ("older", html_slot(older_link)),
        ("newest", html_slot(newest_link)),
    ]);
    html::shell("Feed", Page::Feed, body, project, projects)
}

fn render_feed_event(event: &agentboard::domain::Event, filters: &Filters) -> Fragment {
    let mut content = Vec::new();
    if let Some(value) = event
        .payload
        .get("text")
        .and_then(serde_json::Value::as_str)
    {
        content.push(EVENT_TEXT.fill(&[("text", text(value))]));
    } else {
        let mut has_text = false;
        for (label, key) in [("Comment", "comment"), ("Reason", "reason")] {
            if let Some(value) = event.payload.get(key).and_then(serde_json::Value::as_str) {
                content.push(LABELED_TEXT.fill(&[("label", text(label)), ("text", text(value))]));
                has_text = true;
            }
        }
        if let Some(changes) = event.payload.get("changes")
            && let Ok(json) = serde_json::to_string_pretty(changes)
        {
            content.push(CHANGES.fill(&[("json", text(json))]));
            has_text = true;
        }
        if !has_text {
            let description = match event.kind {
                EventKind::Created => "Task created",
                EventKind::Claimed => "Task claimed",
                EventKind::Updated => "Task updated",
                EventKind::Commented => "Comment added",
            };
            content.push(DESCRIPTION.fill(&[("description", text(description))]));
        }
    }
    let content: Fragment = content.into_iter().collect();
    let timestamp = format_timestamp(event.timestamp_ms);
    EVENT.fill(&[
        ("kind", text(event.kind.as_str())),
        (
            "href",
            text(html::url(
                &format!("/tasks/{}", event.task_id),
                filters.project,
            )),
        ),
        ("task_id", text(event.task_id)),
        ("actor", text(&event.actor)),
        ("timestamp", text(timestamp)),
        ("content", html_slot(content)),
    ])
}
