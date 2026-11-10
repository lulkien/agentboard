use agentboard::store::{Board, ListFilter, MAX_LIST_LIMIT};

use super::routes::{Page, Route};

mod detail;
mod epics;
mod feed;
mod tasks;
mod time;

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
                    project: None,
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
