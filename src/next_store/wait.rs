//! Per-row blocking wait: re-query the watched rows until every one of them is in
//! a wanted status, or the deadline expires.

use std::time::{Duration, Instant};

use crate::domain::{Event, Status, Task};

use super::feed::Cursor;
use super::keys;
use super::open::{Board, Error, Result};

/// How often a parked waiter re-queries the rows it watches.
const POLL_INTERVAL_MS: u64 = 25;

/// The most event lines one watched row may contribute to an answer. A manager
/// parked on an epic for half an hour would otherwise get every event of every
/// row; the oldest events are kept, so the story from the watermark forward stays.
pub const MAX_WAIT_EVENTS: usize = 20;

pub struct WaitSpec<'a> {
    /// Task ids named by the caller; with `epic` these are the epics to expand.
    pub ids: &'a [u64],
    /// Watch the direct children of every named id instead of the ids themselves.
    pub epic: bool,
    /// Wanted statuses; empty is a refusal, never a vacuous match.
    pub statuses: &'a [Status],
    /// Optional watermark: events after this cursor explain a match.
    pub after: Option<&'a str>,
    /// 0 blocks until a match; there is no cap here.
    pub timeout_ms: u64,
}

pub struct WaitRow {
    pub task: Task,
    /// Events after the watermark; empty means the row was already wanted when the
    /// wait started.
    pub events: Vec<Event>,
    /// Newest event at or below the watermark, filled only when `events` is empty.
    pub last: Option<Event>,
    /// Events after the watermark that `MAX_WAIT_EVENTS` left out; 0 for a row
    /// that was already wanted.
    pub omitted: usize,
}

pub struct WaitOutcome {
    pub matched: bool,
    /// The statuses asked for, in board order.
    pub wanted: Vec<Status>,
    /// Watched rows, sorted by id.
    pub rows: Vec<WaitRow>,
    /// The watermark actually used.
    pub since: Cursor,
    /// The board's high sequence at the moment the wait returned.
    pub cursor: Cursor,
    pub waited_ms: u64,
}

/// Parse a CSV of wanted statuses: trimmed, deduplicated, in board order. Both
/// adapters use this so `--status` and `status` read the same way.
pub fn parse_status_csv(raw: &str) -> Result<Vec<Status>> {
    let mut wanted = Vec::new();
    for item in raw.split(',') {
        let name = item.trim();
        if name.is_empty() {
            return Err(Error::WaitWithoutStatus);
        }
        let status = Status::parse(name)?;
        if !wanted.contains(&status) {
            wanted.push(status);
        }
    }
    if wanted.is_empty() {
        return Err(Error::WaitWithoutStatus);
    }
    wanted.sort_by_key(|status| keys::status_code(*status));
    Ok(wanted)
}

struct Poll {
    matched: bool,
    rows: Vec<WaitRow>,
    cursor: Cursor,
}

impl Board {
    /// Block until every watched row is in one of `spec.statuses`, or the deadline
    /// expires. The watched set is re-resolved on every poll, so a child added
    /// mid-epic joins the set. Each poll opens one read transaction and drops it
    /// before sleeping, so a parked waiter holds nothing and blocks no writer.
    pub fn wait_tasks(&self, spec: &WaitSpec<'_>) -> Result<WaitOutcome> {
        if spec.statuses.is_empty() {
            return Err(Error::WaitWithoutStatus);
        }
        if spec.ids.is_empty() {
            return Err(Error::WaitWithoutTask);
        }
        let wanted = ordered(spec.statuses);
        let since = self.wait_watermark(spec.after)?;
        let deadline = (spec.timeout_ms != 0).then(|| Duration::from_millis(spec.timeout_ms));
        let started = Instant::now();
        loop {
            let poll = self.wait_poll(&since, spec, &wanted)?;
            let elapsed = started.elapsed();
            if poll.matched {
                return Ok(WaitOutcome {
                    matched: true,
                    wanted,
                    rows: poll.rows,
                    since,
                    cursor: poll.cursor,
                    waited_ms: elapsed.as_millis() as u64,
                });
            }
            let Some(deadline) = deadline else {
                std::thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
                continue;
            };
            if elapsed >= deadline {
                return Ok(WaitOutcome {
                    matched: false,
                    wanted,
                    rows: poll.rows,
                    since,
                    cursor: poll.cursor,
                    waited_ms: elapsed.as_millis() as u64,
                });
            }
            std::thread::sleep((deadline - elapsed).min(Duration::from_millis(POLL_INTERVAL_MS)));
        }
    }

    /// `None` waits from now; a cursor from another board refuses like the feed.
    fn wait_watermark(&self, after: Option<&str>) -> Result<Cursor> {
        let Some(raw) = after else {
            return self.cursor_now();
        };
        let cursor = Cursor::parse(raw)?;
        let txn = self.env.read_txn()?;
        let metadata = self.checked_metadata(&txn)?;
        if !cursor.instance_id.is_empty() && cursor.instance_id != metadata.instance_id {
            return Err(Error::CursorInstanceMismatch);
        }
        Ok(Cursor {
            instance_id: metadata.instance_id,
            seq: cursor.seq,
        })
    }

    /// One snapshot: the watched set, the verdict, and — on a match only — the
    /// events that explain it.
    fn wait_poll(&self, since: &Cursor, spec: &WaitSpec<'_>, wanted: &[Status]) -> Result<Poll> {
        let txn = self.env.read_txn()?;
        let metadata = self.checked_metadata(&txn)?;
        let mut rows = Vec::new();
        let mut matched = true;
        for id in self.watched_ids(&txn, spec)? {
            let task = self
                .read_task_in_txn(&txn, id)?
                .ok_or(Error::Corrupt("watched task disappeared"))?;
            if !wanted.contains(&task.status) {
                matched = false;
            }
            rows.push(WaitRow {
                task,
                events: Vec::new(),
                last: None,
                omitted: 0,
            });
        }
        let cursor = Cursor {
            instance_id: metadata.instance_id,
            seq: metadata.next_event_seq.saturating_sub(1),
        };
        if matched {
            for row in &mut rows {
                self.wait_events(&txn, row, since)?;
            }
        }
        // The transaction is dropped here, before the caller sleeps.
        Ok(Poll {
            matched,
            rows,
            cursor,
        })
    }

    /// The rows this poll watches: the ids themselves, or their direct children.
    fn watched_ids(&self, txn: &heed::RoTxn<'_>, spec: &WaitSpec<'_>) -> Result<Vec<u64>> {
        let mut ids = Vec::new();
        for &id in spec.ids {
            if self.read_task_in_txn(txn, id)?.is_none() {
                return Err(Error::MissingTask(id));
            }
            if !spec.epic {
                ids.push(id);
                continue;
            }
            let mut children = 0;
            for entry in self.tasks.iter(txn)? {
                let (_, raw) = entry?;
                let task: Task = serde_json::from_slice(raw)?;
                if task.parent_id == Some(id) {
                    ids.push(task.id);
                    children += 1;
                }
            }
            if children == 0 {
                return Err(Error::NotEpic(id));
            }
        }
        ids.sort_unstable();
        ids.dedup();
        Ok(ids)
    }

    /// Fill a row's events after the watermark, or its newest event at or below it.
    fn wait_events(&self, txn: &heed::RoTxn<'_>, row: &mut WaitRow, since: &Cursor) -> Result<()> {
        let mut fresh = Vec::new();
        let mut newest_below = None;
        for entry in self
            .events_by_task
            .prefix_iter(txn, &keys::task(row.task.id))?
        {
            let (key, _) = entry?;
            let bytes: [u8; 8] = key
                .get(8..16)
                .ok_or(Error::Corrupt("invalid event index key"))?
                .try_into()
                .map_err(|_| Error::Corrupt("invalid event index key"))?;
            let seq = u64::from_be_bytes(bytes);
            if seq > since.seq {
                fresh.push(seq);
            } else {
                newest_below = Some(seq);
            }
        }
        if fresh.is_empty() {
            if let Some(seq) = newest_below {
                row.last = Some(self.wait_event(txn, seq)?);
            }
            return Ok(());
        }
        // `fresh` is in sequence order, so keeping the first window keeps the
        // causal story from the watermark forward; the newest events are the ones
        // counted into `omitted`.
        row.omitted = fresh.len().saturating_sub(MAX_WAIT_EVENTS);
        fresh.truncate(MAX_WAIT_EVENTS);
        for seq in fresh {
            row.events.push(self.wait_event(txn, seq)?);
        }
        Ok(())
    }

    fn wait_event(&self, txn: &heed::RoTxn<'_>, sequence: u64) -> Result<Event> {
        self.read_event_in_txn(txn, sequence)?
            .ok_or(Error::Corrupt("event index points to a missing event"))
    }
}

fn ordered(statuses: &[Status]) -> Vec<Status> {
    let mut wanted: Vec<Status> = statuses.to_vec();
    wanted.sort_by_key(|status| keys::status_code(*status));
    wanted.dedup();
    wanted
}
