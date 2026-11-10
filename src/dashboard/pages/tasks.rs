use agentboard::domain::{Project, Status, Task};
use agentboard::store::TaskPage;

use super::PAGE_SIZE;
use crate::dashboard::html;
use crate::dashboard::routes::{Filters, Page};
use crate::dashboard::template::{Fragment, Template, html as html_slot, text};

const TASKS: Template = Template::new("tasks", include_str!("../templates/tasks.html"));
const STATUS_ALL: Template = Template::new(
    "task-status-all",
    include_str!("../templates/task-status-all.html"),
);
const STATUS: Template =
    Template::new("task-status", include_str!("../templates/task-status.html"));
const OPTION: Template =
    Template::new("task-option", include_str!("../templates/task-option.html"));
const TASK_ROW: Template = Template::new("task-row", include_str!("../templates/task-row.html"));
const PARENT_NOTE: Template = Template::new(
    "task-parent-note",
    include_str!("../templates/task-parent-note.html"),
);
const PROJECT_INPUT: Template = Template::new(
    "task-project-input",
    include_str!("../templates/task-project-input.html"),
);
const EMPTY_ROW: Template = Template::new(
    "task-empty-row",
    include_str!("../templates/task-empty-row.html"),
);
const PAGE_LINK: Template = Template::new(
    "task-page-link",
    include_str!("../templates/task-page-link.html"),
);
const PAGE_DISABLED: Template = Template::new(
    "task-page-disabled",
    include_str!("../templates/task-page-disabled.html"),
);

pub(super) fn tasks(
    page: &TaskPage,
    counts: &[(Status, u64)],
    filters: &Filters,
    projects: &[Project],
) -> Fragment {
    let total: u64 = counts.iter().map(|(_, count)| count).sum();
    let status_all = STATUS_ALL.fill(&[
        (
            "href",
            text(html::query_url(
                &Filters {
                    status: None,
                    ..filters.clone()
                },
                None,
            )),
        ),
        (
            "current",
            html_slot(if filters.status.is_none() {
                Fragment::lit(" aria-current=\"true\"")
            } else {
                Fragment::empty()
            }),
        ),
        ("total", text(total)),
    ]);
    let status_links: Fragment = counts
        .iter()
        .map(|(status, count)| {
            let href = html::query_url(
                &Filters {
                    status: Some(*status),
                    ..filters.clone()
                },
                None,
            );
            STATUS.fill(&[
                ("status", text(status.as_str())),
                ("href", text(href)),
                (
                    "current",
                    html_slot(if filters.status == Some(*status) {
                        Fragment::lit(" aria-current=\"true\"")
                    } else {
                        Fragment::empty()
                    }),
                ),
                ("count", text(count)),
            ])
        })
        .collect();
    let status_chips: Fragment = [status_all]
        .into_iter()
        .chain(std::iter::once(status_links))
        .collect();

    let status_options: Fragment = Status::ALL
        .into_iter()
        .map(|status| {
            OPTION.fill(&[
                ("status", text(status.as_str())),
                (
                    "selected",
                    html_slot(if filters.status == Some(status) {
                        Fragment::lit(" selected")
                    } else {
                        Fragment::empty()
                    }),
                ),
            ])
        })
        .collect();
    let project_input = filters.project.map_or_else(Fragment::empty, |project| {
        PROJECT_INPUT.fill(&[("project", text(project))])
    });
    let parent_note = filters.parent.map_or_else(Fragment::empty, |id| {
        PARENT_NOTE.fill(&[
            (
                "href",
                text(html::url(&format!("/tasks?parent={id}"), filters.project)),
            ),
            ("parent", text(id)),
        ])
    });
    let offset = filters.offset.min(page.total);
    let start = if page.tasks.is_empty() { 0 } else { offset + 1 };
    let empty_row = if page.tasks.is_empty() {
        EMPTY_ROW.fill(&[])
    } else {
        Fragment::empty()
    };
    let rows: Fragment = page
        .tasks
        .iter()
        .map(|task| render_task_row(task, filters, projects))
        .collect();
    let page_count = page.total.div_ceil(PAGE_SIZE).max(1);
    let page_number = (offset / PAGE_SIZE + 1) as u64;
    let previous = if offset > 0 {
        PAGE_LINK.fill(&[
            ("label", text("Previous")),
            ("rel", text("prev")),
            (
                "href",
                text(html::query_url(
                    filters,
                    Some(offset.saturating_sub(PAGE_SIZE)),
                )),
            ),
            ("arrow", text("←")),
        ])
    } else {
        PAGE_DISABLED.fill(&[("label", text("Previous")), ("arrow", text("←"))])
    };
    let next = if page.more {
        PAGE_LINK.fill(&[
            ("label", text("Next")),
            ("rel", text("next")),
            (
                "href",
                text(html::query_url(
                    filters,
                    Some(offset.saturating_add(page.tasks.len())),
                )),
            ),
            ("arrow", text("→")),
        ])
    } else {
        PAGE_DISABLED.fill(&[("label", text("Next")), ("arrow", text("→"))])
    };
    let body = TASKS.fill(&[
        ("status_chips", html_slot(status_chips)),
        ("status_options", html_slot(status_options)),
        (
            "assignee",
            text(filters.assignee.as_deref().unwrap_or_default()),
        ),
        (
            "parent",
            text(filters.parent.map(|id| id.to_string()).unwrap_or_default()),
        ),
        ("project_input", html_slot(project_input)),
        ("parent_note", html_slot(parent_note)),
        ("start", text(start)),
        ("end", text(offset + page.tasks.len())),
        ("total", text(page.total)),
        ("empty_row", html_slot(empty_row)),
        ("rows", html_slot(rows)),
        ("previous", html_slot(previous)),
        ("page_number", text(page_number)),
        ("page_count", text(page_count)),
        ("next", html_slot(next)),
    ]);
    html::shell("Tasks", Page::Tasks, body, filters.project, projects)
}

fn render_task_row(task: &Task, filters: &Filters, projects: &[Project]) -> Fragment {
    let project = task.project_id.map_or_else(
        || "—".to_owned(),
        |id| {
            projects
                .iter()
                .find(|item| item.project_id == id)
                .map_or_else(|| id.to_string(), |item| item.project_name.clone())
        },
    );
    TASK_ROW.fill(&[
        (
            "href",
            text(html::url(&format!("/tasks/{}", task.id), filters.project)),
        ),
        ("id", text(task.id)),
        ("badge", html_slot(html::status_badge(task.status))),
        ("title", text(&task.title)),
        ("assignee", text(task.assignee.as_deref().unwrap_or("—"))),
        ("project", text(project)),
        ("priority", text(task.priority)),
    ])
}
