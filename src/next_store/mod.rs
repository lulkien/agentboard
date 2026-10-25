//! LMDB store under construction. The current binary still uses its legacy
//! `src/store` module until the CLI and MCP adapters cut over together.
//! Heed permits one open environment per path in a process; share a `Board`
//! handle when a process has multiple callers.

mod feed;
pub mod keys;
mod list;
mod open;
mod tasks;
mod wait;

pub use feed::{Cursor, FeedFilter, FeedPage, WAIT_MAX_MS};
pub use list::{Brief, DEFAULT_LIST_LIMIT, Epic, ListFilter, MAX_LIST_LIMIT, TaskPage};
pub use open::{Board, BoardOptions, Error, Metadata, ResetOutcome, Result};
pub use tasks::{Added, Agent, ClaimOutcome, ReleaseInput, UpdateInput, Updated};
pub use wait::{AssigneeMatch, MAX_WAIT_EVENTS, WaitOutcome, WaitRow, WaitSpec, parse_status_csv};

#[cfg(test)]
mod m3_tests;
#[cfg(test)]
mod tests;
