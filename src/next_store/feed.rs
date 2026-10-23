//! Global event feed, opaque resume cursors and a bounded long-poll.

use std::time::{Duration, Instant};

use crate::domain::{Event, EventKind};

use super::keys;
use super::open::{Board, Error, Result};

/// The longest a single `wait` call blocks, per the spec.
pub const WAIT_MAX_MS: u64 = 300_000;
const POLL_INTERVAL_MS: u64 = 25;
const DEFAULT_PAGE: usize = 50;
const MAX_PAGE: usize = 500;

/// A resume position: the board instance it belongs to plus a global event sequence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cursor {
    pub instance_id: String,
    pub seq: u64,
}

impl Cursor {
    pub fn encode(&self) -> String {
        format!("{}:{}", self.instance_id, self.seq)
    }

    pub fn parse(raw: &str) -> Result<Self> {
        let (instance_id, seq) = raw
            .rsplit_once(':')
            .ok_or_else(|| Error::InvalidCursor(raw.to_owned()))?;
        let seq = seq
            .parse()
            .map_err(|_| Error::InvalidCursor(raw.to_owned()))?;
        Ok(Self {
            instance_id: instance_id.to_owned(),
            seq,
        })
    }
}

pub struct FeedFilter<'a> {
    pub task: Option<u64>,
    pub kind: Option<EventKind>,
    pub recipient: Option<&'a str>,
    /// Encoded cursor from a previous page; `None` asks for the newest page.
    pub after: Option<&'a str>,
    pub limit: usize,
}

pub struct FeedPage {
    pub events: Vec<Event>,
    pub next: Cursor,
    pub more: bool,
}

impl Board {
    /// One page of events in ascending sequence order. The cursor is taken from the
    /// same read snapshot the rows came from, so a caller can list and then wait
    /// without missing a handoff committed in between.
    pub fn feed(&self, filter: &FeedFilter<'_>) -> Result<FeedPage> {
        let txn = self.env.read_txn()?;
        let metadata = self.checked_metadata(&txn)?;
        let high = metadata.next_event_seq.saturating_sub(1);
        let limit = if filter.limit == 0 {
            DEFAULT_PAGE
        } else {
            filter.limit
        }
        .clamp(1, MAX_PAGE);
        let start = match filter.after {
            Some(raw) => {
                let cursor = Cursor::parse(raw)?;
                if cursor.instance_id != metadata.instance_id {
                    return Err(Error::CursorInstanceMismatch);
                }
                // A cursor is the last sequence the caller saw, so resume after it.
                cursor.seq.saturating_add(1)
            }
            None => high.saturating_sub(limit as u64 - 1),
        };
        let mut events = Vec::new();
        let mut more = false;
        let mut scanned = start;
        while scanned <= high {
            let Some(raw) = self.events.get(&txn, &keys::task(scanned))? else {
                scanned += 1;
                continue;
            };
            let event: Event = serde_json::from_slice(raw)?;
            if event.seq != scanned {
                return Err(Error::Corrupt("event key/sequence mismatch"));
            }
            scanned += 1;
            if !matches(filter, &event) {
                continue;
            }
            events.push(event);
            if events.len() == limit {
                more = scanned <= high;
                break;
            }
        }
        let next_seq = match events.last() {
            Some(event) if more => event.seq,
            _ => high,
        };
        Ok(FeedPage {
            events,
            next: Cursor {
                instance_id: metadata.instance_id,
                seq: next_seq,
            },
            more,
        })
    }

    /// The current snapshot position, for a caller that wants to wait from "now".
    pub fn cursor_now(&self) -> Result<Cursor> {
        let txn = self.env.read_txn()?;
        let metadata = self.checked_metadata(&txn)?;
        Ok(Cursor {
            instance_id: metadata.instance_id,
            seq: metadata.next_event_seq.saturating_sub(1),
        })
    }

    /// Poll until a matching event exists or the timeout expires. Every poll opens
    /// and drops its own read transaction, so a parked waiter blocks no writer.
    /// With no cursor, the wait starts now instead of replaying history.
    pub fn wait(&self, filter: &FeedFilter<'_>, timeout_ms: u64) -> Result<FeedPage> {
        let timeout = Duration::from_millis(timeout_ms.min(WAIT_MAX_MS));
        let started = Instant::now();
        let from_now = if filter.after.is_none() {
            Some(self.cursor_now()?.encode())
        } else {
            None
        };
        let after = filter.after.or(from_now.as_deref());
        let filter = FeedFilter {
            task: filter.task,
            kind: filter.kind,
            recipient: filter.recipient,
            after,
            limit: filter.limit,
        };
        loop {
            let page = self.feed(&filter)?;
            if !page.events.is_empty() {
                return Ok(page);
            }
            let elapsed = started.elapsed();
            if elapsed >= timeout {
                return Ok(page);
            }
            std::thread::sleep((timeout - elapsed).min(Duration::from_millis(POLL_INTERVAL_MS)));
        }
    }
}

fn matches(filter: &FeedFilter<'_>, event: &Event) -> bool {
    if let Some(task) = filter.task
        && event.task_id != task
    {
        return false;
    }
    if let Some(kind) = filter.kind
        && event.kind != kind
    {
        return false;
    }
    if let Some(recipient) = filter.recipient
        && event.recipient.as_deref() != Some(recipient)
    {
        return false;
    }
    true
}
