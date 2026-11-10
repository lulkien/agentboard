use agentboard::domain::{Event, EventKind, Project, Task};
use agentboard::store::{Board, Epic, FeedFilter, ListFilter, TaskPage};

use super::PageError;
use super::time::format_timestamp;
use crate::dashboard::html;
use crate::dashboard::routes::Page;
use crate::dashboard::template::{Fragment, Slot, Template, html as html_slot, text};

const START: Template = Template::new(
    "detail-start",
    include_str!("../templates/detail-start.html"),
);
const REPORT: Template = Template::new(
    "detail-report",
    include_str!("../templates/detail-report.html"),
);
const CHILDREN: Template = Template::new(
    "detail-children",
    include_str!("../templates/detail-children.html"),
);
const CHILD: Template = Template::new(
    "detail-child",
    include_str!("../templates/detail-child.html"),
);
const CHILD_MORE: Template = Template::new(
    "detail-child-more",
    include_str!("../templates/detail-child-more.html"),
);
const ACTIVITY: Template = Template::new(
    "detail-activity",
    include_str!("../templates/detail-activity.html"),
);
const NO_ACTIVITY: Template = Template::new(
    "detail-no-activity",
    include_str!("../templates/detail-no-activity.html"),
);
const EVENT: Template = Template::new(
    "detail-event",
    include_str!("../templates/detail-event.html"),
);
const EVENTS: Template = Template::new(
    "detail-events",
    include_str!("../templates/detail-events.html"),
);
const EVENT_TEXT: Template = Template::new(
    "detail-event-text",
    include_str!("../templates/detail-event-text.html"),
);
const CHANGE_EMPTY: Template = Template::new(
    "detail-change-empty",
    include_str!("../templates/detail-change-empty.html"),
);
const CHANGES: Template = Template::new(
    "detail-changes",
    include_str!("../templates/detail-changes.html"),
);
const CHANGE: Template = Template::new(
    "detail-change",
    include_str!("../templates/detail-change.html"),
);
const ON_PAGE: Template = Template::new(
    "detail-on-page",
    include_str!("../templates/detail-on-page.html"),
);
const ON_PAGE_ITEM: Template = Template::new(
    "detail-on-page-item",
    include_str!("../templates/detail-on-page-item.html"),
);
const METADATA: Template = Template::new(
    "metadata-row",
    include_str!("../templates/metadata-row.html"),
);
const PROJECT_LINK: Template = Template::new(
    "detail-project-link",
    include_str!("../templates/detail-project-link.html"),
);
const PRIORITY: Template = Template::new(
    "detail-priority",
    include_str!("../templates/detail-priority.html"),
);
const PARENT: Template = Template::new(
    "detail-parent",
    include_str!("../templates/detail-parent.html"),
);
const TIME: Template = Template::new("detail-time", include_str!("../templates/detail-time.html"));

pub(super) fn detail(
    board: &Board,
    id: u64,
    project: Option<u64>,
    projects: &[Project],
) -> Result<Fragment, PageError> {
    let task = board
        .get(id)
        .map_err(|e| PageError::Store(e.to_string()))?
        .ok_or(PageError::Missing)?;
    let child_filter = ListFilter {
        status: None,
        assignee: None,
        parent: Some(id),
        project: None,
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
    Ok(task_detail(
        &task,
        &children,
        epic.as_ref(),
        &feed.events,
        project,
        projects,
    ))
}

fn task_detail(
    task: &Task,
    children: &TaskPage,
    epic: Option<&Epic>,
    events: &[Event],
    project: Option<u64>,
    projects: &[Project],
) -> Fragment {
    let reports = [
        (
            "analyst-report",
            "Analyst report",
            task.analyst_report.as_str(),
        ),
        ("coder-report", "Coder report", task.coder_report.as_str()),
        (
            "reviewer-report",
            "Reviewer report",
            task.reviewer_report.as_str(),
        ),
    ];
    let report_sections: Fragment = reports
        .iter()
        .filter(|(_, _, report)| !report.is_empty())
        .map(|(id, heading, report)| {
            REPORT.fill(&[
                ("id", text(id)),
                ("heading", text(heading)),
                ("report", text(report)),
            ])
        })
        .collect();

    let mut page_items = vec![("Goal", "goal"), ("Description", "description")];
    page_items.extend(
        reports
            .iter()
            .filter(|(_, _, report)| !report.is_empty())
            .map(|(id, heading, _)| (*heading, *id)),
    );
    page_items.push(("Activity", "activity"));
    let page_links: Fragment = page_items
        .iter()
        .map(|(label, id)| ON_PAGE_ITEM.fill(&[("label", text(label)), ("id", text(id))]))
        .collect();
    let on_page = ON_PAGE.fill(&[("items", html_slot(page_links))]);

    let child_section = epic.map_or_else(Fragment::empty, |epic| {
        let maximum = epic.total.max(1);
        let progress = epic.done.min(maximum);
        let child_items: Fragment = children
            .tasks
            .iter()
            .map(|child| {
                let href = html::url(&format!("/tasks/{}", child.id), project);
                CHILD.fill(&[("strip", html_slot(html::strip(child, &href)))])
            })
            .collect();
        let more = if children.total > children.tasks.len() {
            let href = html::url(&format!("/tasks?parent={}", task.id), project);
            CHILD_MORE.fill(&[("href", text(href)), ("total", text(children.total))])
        } else {
            Fragment::empty()
        };
        CHILDREN.fill(&[
            ("done", text(epic.done)),
            ("total", text(epic.total)),
            ("title", text(&task.title)),
            ("maximum", text(maximum)),
            ("progress", text(progress)),
            ("children", html_slot(child_items)),
            ("more", html_slot(more)),
        ])
    });

    let history_href = html::url(&format!("/feed?task={}", task.id), project);
    let activity_content = activity_content(events, projects);
    let activity = ACTIVITY.fill(&[
        ("href", text(history_href)),
        ("content", html_slot(activity_content)),
    ]);

    let mut metadata = vec![
        metadata_row("Status", html_slot(html::status_badge(task.status))),
        metadata_row(
            "Priority",
            html_slot(PRIORITY.fill(&[("priority", text(task.priority))])),
        ),
        metadata_row(
            "Assignee",
            text(task.assignee.as_deref().unwrap_or("Unassigned")),
        ),
        metadata_row("Reporter", text(&task.reporter)),
    ];
    if let Some(parent) = task.parent_id {
        let href = html::url(&format!("/tasks/{parent}"), project);
        metadata.push(metadata_row(
            "Parent",
            html_slot(PARENT.fill(&[("href", text(href)), ("parent", text(parent))])),
        ));
    }
    let project_value = task.project_id.map_or_else(
        || text("—"),
        |id| match projects.iter().find(|item| item.project_id == id) {
            Some(item) => html_slot(PROJECT_LINK.fill(&[
                ("href", text(html::url("/tasks", Some(id)))),
                ("name", text(&item.project_name)),
                ("path", text(&item.project_path)),
            ])),
            None => text(id),
        },
    );
    metadata.push(metadata_row("Project", project_value));
    metadata.push(metadata_row("Revision", text(task.revision)));
    let created = format_timestamp(task.created_at_ms);
    metadata.push(metadata_row(
        "Created",
        html_slot(TIME.fill(&[("time", text(&created))])),
    ));
    let updated = format_timestamp(task.updated_at_ms);
    metadata.push(metadata_row(
        "Updated",
        html_slot(TIME.fill(&[("time", text(&updated))])),
    ));
    let metadata: Fragment = metadata.into_iter().collect();

    let body = START.fill(&[
        ("tasks_href", text(html::url("/tasks", project))),
        ("id", text(task.id)),
        ("title", text(&task.title)),
        ("badge", html_slot(html::status_badge(task.status))),
        ("goal", text(&task.goal)),
        ("description", text(&task.description)),
        ("reports", html_slot(report_sections)),
        ("on_page", html_slot(on_page)),
        ("children", html_slot(child_section)),
        ("activity", html_slot(activity)),
        ("metadata", html_slot(metadata)),
    ]);
    html::shell(
        &format!("T{} | {}", task.id, task.title),
        Page::Detail(task.id),
        body,
        project,
        projects,
    )
}

fn activity_content(events: &[Event], projects: &[Project]) -> Fragment {
    if events.is_empty() {
        return NO_ACTIVITY.fill(&[]);
    }
    let items: Fragment = events
        .iter()
        .map(|event| {
            let timestamp = format_timestamp(event.timestamp_ms);
            let body = if let Some(payload_text) = event
                .payload
                .get("text")
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    event
                        .payload
                        .get("reason")
                        .and_then(serde_json::Value::as_str)
                }) {
                EVENT_TEXT.fill(&[("text", text(payload_text))])
            } else if event.kind == EventKind::Updated {
                updated_changes(event, projects)
            } else {
                Fragment::empty()
            };
            EVENT.fill(&[
                ("kind", text(event.kind.as_str())),
                ("actor", text(&event.actor)),
                ("timestamp", text(&timestamp)),
                ("body", html_slot(body)),
            ])
        })
        .collect();
    EVENTS.fill(&[("items", html_slot(items))])
}

fn updated_changes(event: &Event, projects: &[Project]) -> Fragment {
    let changes: Vec<_> = event
        .payload
        .get("changes")
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flat_map(|values| values.iter())
        .map(|(field, pair)| {
            let old = pair
                .get("old")
                .map(|v| display_change_value(field, v, projects))
                .unwrap_or_default();
            let new = pair
                .get("new")
                .map(|v| display_change_value(field, v, projects))
                .unwrap_or_default();
            (field.clone(), old, new)
        })
        .collect();
    if changes.is_empty() {
        CHANGE_EMPTY.fill(&[])
    } else {
        let rows: Fragment = changes
            .iter()
            .map(|(field, old, new)| {
                CHANGE.fill(&[
                    ("field", text(field)),
                    ("old", text(old)),
                    ("new", text(new)),
                ])
            })
            .collect();
        CHANGES.fill(&[("rows", html_slot(rows))])
    }
}

fn display_change_value(field: &str, value: &serde_json::Value, projects: &[Project]) -> String {
    if field == "project_id"
        && let Some(id) = value.as_u64()
    {
        return projects
            .iter()
            .find(|project| project.project_id == id)
            .map_or_else(|| id.to_string(), |project| project.project_name.clone());
    }
    display_value(value)
}

fn metadata_row(label: &str, value: Slot) -> Fragment {
    METADATA.fill(&[("label", text(label)), ("value", value)])
}

fn display_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(value) => value.clone(),
        serde_json::Value::Null => "—".to_owned(),
        other => other.to_string(),
    }
}
