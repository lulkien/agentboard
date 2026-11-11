//! Heed permits one open environment per path in a process; share a `Board`
//! handle when a process has multiple callers.

mod epics;
mod feed;
pub mod keys;
mod list;
mod open;
mod overview;
mod projects;
mod tasks;
mod wait;

pub(super) const POLL_INTERVAL_MS: u64 = 25;

pub use epics::{EpicRow, Epics};
pub use feed::{Cursor, FeedFilter, FeedPage, OlderFeedPage, WAIT_MAX_MS};
pub use list::{Brief, DEFAULT_LIST_LIMIT, Epic, ListFilter, MAX_LIST_LIMIT, TaskPage};
pub use open::{Board, BoardOptions, Error, Metadata, ResetOutcome, Result};
pub use overview::{Held, Lane, Overview};
pub use tasks::{Added, Agent, ClaimOutcome, UpdateInput, Updated};
pub use wait::{AssigneeMatch, MAX_WAIT_EVENTS, WaitOutcome, WaitRow, WaitSpec, parse_status_csv};

#[cfg(test)]
mod m3_tests;
#[cfg(test)]
mod overview_tests;
#[cfg(test)]
mod tests;
