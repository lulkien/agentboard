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
const MORE: Template = Template::new("board-more", include_str!("../templates/board-more.html"));
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
const BAYS: Template = Template::new("board-bays", include_str!("../templates/board-bays.html"));
const BAY: Template = Template::new("board-bay", include_str!("../templates/board-bay.html"));
const CLOSED_COUNTS: Template = Template::new(
    "board-closed-counts",
    include_str!("../templates/board-closed-counts.html"),
);
const CLOSED_COUNT: Template = Template::new(
    "board-closed-count",
    include_str!("../templates/board-closed-count.html"),
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

    let bays: Fragment = overview
        .lanes
        .iter()
        .map(|lane| {
            let status = lane.status.as_str();
            let href = html::url(&format!("/tasks?status={status}"), project);
            let items: Fragment = lane
                .tasks
                .iter()
                .map(|task| strip_item(task, project))
                .collect();
            let more = if lane.total > lane.tasks.len() {
                MORE.fill(&[
                    ("href", text(href)),
                    ("count", text(lane.total - lane.tasks.len())),
                ])
            } else {
                Fragment::empty()
            };
            BAY.fill(&[
                ("status", text(status)),
                ("total", text(lane.total)),
                ("open", text(if lane.total <= 8 { " open" } else { "" })),
                ("items", html_slot(items)),
                ("more", html_slot(more)),
            ])
        })
        .collect();
    let counts: Fragment = [Status::Done, Status::Canceled]
        .into_iter()
        .map(|status| {
            let count = overview
                .counts
                .iter()
                .find(|(item, _)| *item == status)
                .map_or(0, |(_, count)| *count);
            let href = html::url(&format!("/tasks?status={}", status.as_str()), project);
            CLOSED_COUNT.fill(&[
                ("href", text(href)),
                ("status", text(status.as_str())),
                ("count", text(count)),
            ])
        })
        .collect();
    let closed_counts = CLOSED_COUNTS.fill(&[("counts", html_slot(counts))]);
    let bays = BAYS.fill(&[
        ("bays", html_slot(bays)),
        ("closed_counts", html_slot(closed_counts)),
    ]);
    let body = BOARD.fill(&[
        ("attention", html_slot(attention)),
        ("seats", html_slot(seats)),
        ("bays", html_slot(bays)),
    ]);
    html::shell("Board", Page::Board, body, project, projects)
}

fn strip_item(task: &Task, project: Option<u64>) -> Fragment {
    let href = html::url(&format!("/tasks/{}", task.id), project);
    LIST_ITEM.fill(&[("content", html_slot(html::strip(task, &href)))])
}
