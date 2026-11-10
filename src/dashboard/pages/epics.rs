use agentboard::domain::{Project, Status};
use agentboard::store::{Board, Brief, Epic, ListFilter};

use super::PageError;
use crate::dashboard::html;
use crate::dashboard::routes::Page;
use crate::dashboard::template::{Fragment, Template, html as html_slot, text};

const CHILD_STRIP_CAP: usize = 4;

const EPICS: Template = Template::new("epics", include_str!("../templates/epics.html"));
const EMPTY: Template = Template::new("epics-empty", "<p class=\"empty panel\">No epics yet</p>");
const EPIC_ROW: Template = Template::new("epic-row", include_str!("../templates/epic-row.html"));
const PARENT_LINK: Template = Template::new(
    "epic-parent-link",
    "<a class=\"meta\" href=\"{{href}}\">in T{{parent}}</a>",
);
const SVG_RECT: Template = Template::new(
    "epic-svg-rect",
    "<rect class=\"status-{{status}}\" x=\"{{x}}\" y=\"0\" width=\"{{width}}\" height=\"10\"></rect>",
);
const COUNT_ITEM: Template = Template::new(
    "epic-count-item",
    "<li class=\"status-{{status}}\">{{status}}: <strong>{{count}}</strong></li>",
);
const CHILD_ITEM: Template = Template::new("epic-child-item", "<li>{{strip}}</li>");
const MORE_CHILDREN: Template = Template::new(
    "epic-more-children",
    "<p class=\"epic-more\"><a href=\"{{href}}\">+{{count}} more</a></p>",
);

pub(super) fn epics(
    board: &Board,
    brief: &Brief,
    project: Option<u64>,
    projects: &[Project],
) -> Result<Fragment, PageError> {
    let content = if brief.epics.is_empty() {
        EMPTY.fill(&[])
    } else {
        let mut rows = Vec::with_capacity(brief.epics.len());
        for epic in &brief.epics {
            let depth = epic_depth(epic, &brief.epics).min(6);
            let children = board
                .list(&ListFilter {
                    status: None,
                    assignee: None,
                    parent: Some(epic.task.id),
                    project,
                    include_terminal: true,
                    limit: CHILD_STRIP_CAP,
                    offset: 0,
                })
                .map_err(|e| PageError::Store(e.to_string()))?;
            rows.push(render_epic(
                epic,
                &children.tasks,
                children.total,
                depth,
                project,
            ));
        }
        rows.into_iter().collect()
    };
    let body = EPICS.fill(&[("content", html_slot(content))]);
    Ok(html::shell("Epics", Page::Epics, body, project, projects))
}

fn epic_depth(epic: &Epic, epics: &[Epic]) -> usize {
    let mut depth = 0;
    let mut parent = epic.task.parent_id;
    while let Some(id) = parent {
        let Some(ancestor) = epics.iter().find(|candidate| candidate.task.id == id) else {
            break;
        };
        depth += 1;
        parent = ancestor.task.parent_id;
        if depth >= epics.len() {
            break;
        }
    }
    depth
}

fn render_epic(
    epic: &Epic,
    children: &[agentboard::domain::Task],
    child_total: usize,
    depth: usize,
    project: Option<u64>,
) -> Fragment {
    let task = &epic.task;
    let href = html::url(&format!("/tasks?parent={}", task.id), project);
    let strip = html::strip(task, &href);
    let parent_link = task.parent_id.map_or_else(Fragment::empty, |parent| {
        let href = html::url(&format!("/tasks/{parent}"), project);
        PARENT_LINK.fill(&[("href", text(href)), ("parent", text(parent))])
    });
    let total = epic.total.max(1);
    let mut x = 0.0_f64;
    let mut rects = Vec::new();
    for status in Status::ALL {
        let count = epic
            .counts
            .iter()
            .find_map(|(found, count)| (*found == status).then_some(*count))
            .unwrap_or(0);
        if count == 0 {
            continue;
        }
        let width = (count as f64 / total as f64) * 100.0;
        rects.push(SVG_RECT.fill(&[
            ("status", text(status.as_str())),
            ("x", text(format!("{x:.3}"))),
            ("width", text(format!("{width:.3}"))),
        ]));
        x += width;
    }
    let rects: Fragment = rects.into_iter().collect();
    let counts: Fragment = Status::ALL
        .into_iter()
        .filter_map(|status| {
            let count = epic
                .counts
                .iter()
                .find_map(|(found, count)| (*found == status).then_some(*count))
                .unwrap_or(0);
            (count > 0).then(|| {
                COUNT_ITEM.fill(&[("status", text(status.as_str())), ("count", text(count))])
            })
        })
        .collect();
    let children_strips: Fragment = children
        .iter()
        .map(|child| {
            let href = html::url(&format!("/tasks/{}", child.id), project);
            CHILD_ITEM.fill(&[("strip", html_slot(html::strip(child, &href)))])
        })
        .collect();
    let more = if child_total > children.len() {
        let href = html::url(&format!("/tasks?parent={}", task.id), project);
        MORE_CHILDREN.fill(&[
            ("href", text(href)),
            ("count", text(child_total - children.len())),
        ])
    } else {
        Fragment::empty()
    };
    let children_href = html::url(&format!("/tasks?parent={}", task.id), project);
    EPIC_ROW.fill(&[
        ("depth", text(depth)),
        ("strip", html_slot(strip)),
        ("parent_link", html_slot(parent_link)),
        ("priority", text(task.priority)),
        ("done", text(epic.done)),
        ("total", text(epic.total)),
        ("rects", html_slot(rects)),
        ("progress", text(epic.done)),
        ("counts", html_slot(counts)),
        ("children_href", text(children_href)),
        ("children", html_slot(children_strips)),
        ("more", html_slot(more)),
    ])
}
