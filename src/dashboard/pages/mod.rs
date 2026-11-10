use agentboard::store::{Board, ListFilter, MAX_LIST_LIMIT};

use super::routes::{Page, Route};
use super::template::Fragment;

mod board;
mod detail;
mod epics;
mod feed;
mod tasks;
mod time;

use board::board_page;
use detail::detail;
use epics::epics;
use feed::feed;
use tasks::tasks;

const PAGE_SIZE: usize = 20;

pub enum PageError {
    Missing,
    BadRequest,
    Store(String),
}

pub fn render(board: &Board, route: Route) -> Result<Fragment, PageError> {
    let project = route.filters.project;
    if let Some(id) = project {
        board
            .get_project(id)
            .map_err(|e| PageError::Store(e.to_string()))?
            .ok_or(PageError::Missing)?;
    }
    let projects = board
        .list_projects()
        .map_err(|e| PageError::Store(e.to_string()))?;
    match route.page {
        Page::Board => {
            let overview = board
                .overview(project, 8)
                .map_err(|e| PageError::Store(e.to_string()))?;
            let agents = board
                .agents()
                .map_err(|e| PageError::Store(e.to_string()))?;
            Ok(board_page(&overview, &agents, project, &projects))
        }
        Page::Epics => {
            let brief = board
                .brief_for_project(0, project)
                .map_err(|e| PageError::Store(e.to_string()))?;
            epics(board, &brief, project, &projects)
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
                    project,
                    include_terminal: true,
                    limit: PAGE_SIZE.min(MAX_LIST_LIMIT),
                    offset: filters.offset,
                })
                .map_err(|e| PageError::Store(e.to_string()))?;
            let counts = board
                .status_counts(&ListFilter {
                    status: filters.status,
                    assignee: filters.assignee.as_deref(),
                    parent: filters.parent,
                    project,
                    include_terminal: true,
                    limit: PAGE_SIZE,
                    offset: 0,
                })
                .map_err(|e| PageError::Store(e.to_string()))?;
            Ok(tasks(&page, &counts, &filters, &projects))
        }
        Page::Detail(id) => detail(board, id, project, &projects),
        Page::Feed => feed(board, route.filters, &projects),
    }
}
