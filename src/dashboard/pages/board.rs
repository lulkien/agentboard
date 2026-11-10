use agentboard::domain::{Project, Status, Task};
use agentboard::store::{Agent, Overview};

use super::time::format_age;
use crate::dashboard::{
    html,
    routes::Page,
    template::{Fragment, Template, html as html_slot, text},
};

const BOARD: Template = Template::new("board", include_str!("../templates/board.html"));
const ATTENTION: Template = Template::new(
    "board-attention",
    include_str!("../templates/board-attention.html"),
);
const EMPTY_ATTENTION: Template = Template::new(
    "board-empty-attention",
    include_str!("../templates/board-empty-attention.html"),
);
const LIST_ITEM: Template = Template::new(
    "board-list-item",
    include_str!("../templates/board-list-item.html"),
);
const MORE_TASKS: Template = Template::new(
    "board-more-tasks",
    include_str!("../templates/board-more-tasks.html"),
);
const SEATS: Template = Template::new("board-seats", include_str!("../templates/board-seats.html"));
const SEAT_HELD: Template = Template::new(
    "board-seat-chip-held",
    include_str!("../templates/board-seat-chip-held.html"),
);
const SEAT_IDLE: Template = Template::new(
    "board-seat-chip-idle",
    include_str!("../templates/board-seat-chip-idle.html"),
);
const EMPTY_SEATS: Template = Template::new(
    "board-seats-empty",
    include_str!("../templates/board-seats-empty.html"),
);
const KANBAN: Template = Template::new(
    "board-kanban",
    include_str!("../templates/board-kanban.html"),
);
const COLUMN: Template = Template::new(
    "board-column",
    include_str!("../templates/board-column.html"),
);
const CARD: Template = Template::new("board-card", include_str!("../templates/board-card.html"));
const CLOSED_ROW: Template = Template::new(
    "board-closed-row",
    include_str!("../templates/board-closed-row.html"),
);
const CLOSED_ITEM: Template = Template::new(
    "board-closed-item",
    include_str!("../templates/board-closed-item.html"),
);
const EMPTY_COLUMN: Template = Template::new(
    "board-empty-column",
    include_str!("../templates/board-empty-column.html"),
);
const MORE_CARDS: Template = Template::new(
    "board-more-cards",
    include_str!("../templates/board-more-cards.html"),
);
const CARD_PARENT: Template = Template::new(
    "board-card-parent",
    include_str!("../templates/board-card-parent.html"),
);
const CARD_PROJECT: Template = Template::new(
    "board-card-project",
    include_str!("../templates/board-card-project.html"),
);
const CARD_TIME: Template = Template::new(
    "board-card-time",
    include_str!("../templates/board-card-time.html"),
);

pub(super) fn board_page(
    overview: &Overview,
    agents: &[Agent],
    project: Option<u64>,
    projects: &[Project],
    now_ms: i64,
) -> Fragment {
    let attention_items: Fragment = overview
        .attention
        .iter()
        .map(|task| strip_item(task, project))
        .collect();
    let empty_attention = if overview.attention.is_empty() {
        EMPTY_ATTENTION.fill(&[])
    } else {
        Fragment::empty()
    };
    let more_attention = if overview.attention_total > overview.attention.len() {
        MORE_TASKS.fill(&[
            ("href", text(html::url("/tasks", project))),
            (
                "count",
                text(overview.attention_total - overview.attention.len()),
            ),
        ])
    } else {
        Fragment::empty()
    };
    let attention = ATTENTION.fill(&[
        ("total", text(overview.attention_total)),
        ("items", html_slot(attention_items)),
        ("empty", html_slot(empty_attention)),
        ("more", html_slot(more_attention)),
    ]);

    let mut names: Vec<&str> = agents.iter().map(|agent| agent.name.as_str()).collect();
    for held in &overview.held {
        if !names.contains(&held.assignee.as_str()) {
            names.push(&held.assignee);
        }
    }
    names.sort_unstable();
    let empty_seats = if names.is_empty() {
        EMPTY_SEATS.fill(&[])
    } else {
        Fragment::empty()
    };
    let seats: Fragment = names
        .into_iter()
        .map(|name| {
            let held = overview.held.iter().find(|held| held.assignee == name);
            if let Some(held) = held {
                let href = html::url(&format!("/tasks?assignee={}", html::encode(name)), project);
                SEAT_HELD.fill(&[
                    ("href", text(href)),
                    ("name", text(name)),
                    ("count", text(held.total)),
                ])
            } else {
                let age = agents
                    .iter()
                    .find(|agent| agent.name == name)
                    .and_then(|agent| agent.last_seen_ms)
                    .map_or_else(String::new, |ts| format!(" {}", format_age(now_ms, ts)));
                SEAT_IDLE.fill(&[("name", text(name)), ("age", text(age))])
            }
        })
        .collect();
    let seats = SEATS.fill(&[
        ("seats", html_slot(seats)),
        ("empty", html_slot(empty_seats)),
    ]);

    let columns: Fragment = overview
        .lanes
        .iter()
        .map(|lane| {
            let status = lane.status.as_str();
            let cards: Fragment = lane
                .tasks
                .iter()
                .map(|task| render_card(task, project, projects, now_ms))
                .collect();
            let items = if lane.tasks.is_empty() {
                EMPTY_COLUMN.fill(&[])
            } else {
                cards
            };
            let href = html::url(&format!("/tasks?status={status}"), project);
            let more = if lane.total > lane.tasks.len() {
                MORE_CARDS.fill(&[
                    ("href", text(href)),
                    ("count", text(lane.total - lane.tasks.len())),
                ])
            } else {
                Fragment::empty()
            };
            COLUMN.fill(&[
                ("status", text(status)),
                ("total", text(lane.total)),
                ("glyph", text(html::status_glyph(lane.status))),
                ("items", html_slot(items)),
                ("more", html_slot(more)),
            ])
        })
        .collect();
    let closed_items: Fragment = [Status::Done, Status::Canceled]
        .into_iter()
        .map(|status| {
            let count = overview
                .counts
                .iter()
                .find(|(item, _)| *item == status)
                .map_or(0, |(_, count)| *count);
            let href = html::url(&format!("/tasks?status={}", status.as_str()), project);
            CLOSED_ITEM.fill(&[
                ("href", text(href)),
                ("glyph", text(html::status_glyph(status))),
                ("status", text(status.as_str())),
                ("count", text(count)),
            ])
        })
        .collect();
    let closed_summary = CLOSED_ROW.fill(&[("items", html_slot(closed_items))]);
    let kanban = KANBAN.fill(&[("columns", html_slot(columns))]);
    let body = BOARD.fill(&[
        ("attention", html_slot(attention)),
        ("seats", html_slot(seats)),
        ("bays", html_slot(kanban)),
    ]);
    html::shell_with_aside(
        "Board",
        Page::Board,
        closed_summary,
        body,
        project,
        projects,
    )
}

fn render_card(task: &Task, project: Option<u64>, projects: &[Project], now_ms: i64) -> Fragment {
    let status = task.status.as_str();
    let href = html::url(&format!("/tasks/{}", task.id), project);
    let flag_class = if matches!(task.status, Status::Blocked | Status::Reviewing) {
        "card-flag"
    } else {
        "visually-hidden"
    };
    let parent = task
        .parent_id
        .map_or_else(Fragment::empty, |id| CARD_PARENT.fill(&[("id", text(id))]));
    let project_name = if project.is_none() {
        task.project_id
            .and_then(|id| projects.iter().find(|item| item.project_id == id))
            .map_or_else(Fragment::empty, |item| {
                CARD_PROJECT.fill(&[("name", text(&item.project_name))])
            })
    } else {
        Fragment::empty()
    };
    let timestamp = super::time::format_timestamp(task.updated_at_ms);
    let age = format_age(now_ms, task.updated_at_ms);
    let time = CARD_TIME.fill(&[("timestamp", text(&timestamp)), ("age", text(age))]);
    CARD.fill(&[
        ("class", text(status)),
        ("href", text(href)),
        ("title", text(&task.title)),
        ("glyph", text(html::status_glyph(task.status))),
        ("id", text(task.id)),
        ("flag_class", text(flag_class)),
        ("status", text(status)),
        ("title_text", text(&task.title)),
        (
            "assignee",
            text(task.assignee.as_deref().unwrap_or("Unassigned")),
        ),
        ("parent", html_slot(parent)),
        ("project", html_slot(project_name)),
        ("time", html_slot(time)),
    ])
}

fn strip_item(task: &Task, project: Option<u64>) -> Fragment {
    let href = html::url(&format!("/tasks/{}", task.id), project);
    LIST_ITEM.fill(&[("content", html_slot(html::strip(task, &href)))])
}
