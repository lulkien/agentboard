//! Task listing and the compact board summary.

use std::collections::HashMap;

use crate::domain::{Status, Task};

use super::feed::Cursor;
use super::keys;
use super::open::{Board, Error, Result};

pub const DEFAULT_LIST_LIMIT: usize = 20;
pub const MAX_LIST_LIMIT: usize = 200;

pub struct ListFilter<'a> {
    pub status: Option<Status>,
    pub assignee: Option<&'a str>,
    pub parent: Option<u64>,
    pub path: Option<&'a str>,
    /// Terminal rows (`done`, `canceled`) stay out unless this is set.
    pub include_terminal: bool,
    pub limit: usize,
    pub offset: usize,
}

pub struct TaskPage {
    pub tasks: Vec<Task>,
    /// Rows matching the filter before paging.
    pub total: usize,
    pub more: bool,
    /// Snapshot cursor for a following `feed` call.
    pub cursor: Cursor,
}

pub struct Brief {
    pub counts: Vec<(Status, u64)>,
    pub epics: Vec<Epic>,
    pub active: Vec<Task>,
}

pub struct Epic {
    pub task: Task,
    pub counts: Vec<(Status, u64)>,
    pub done: u64,
    pub total: u64,
    pub cursor: Cursor,
}

fn status_counts(counts: &[u64; Status::ALL.len()]) -> Vec<(Status, u64)> {
    Status::ALL
        .iter()
        .map(|s| (*s, counts[keys::status_code(*s) as usize]))
        .collect()
}

fn rollup(task: Task, counts: &[u64; Status::ALL.len()], cursor: Cursor) -> Epic {
    Epic {
        task,
        counts: status_counts(counts),
        done: counts[keys::status_code(Status::Done) as usize],
        total: counts.iter().sum(),
        cursor,
    }
}

impl Board {
    /// Direct-child counts in the caller's snapshot, including write transactions.
    pub(super) fn child_counts(
        &self,
        txn: &heed::RoTxn<'_>,
    ) -> Result<HashMap<u64, [u64; Status::ALL.len()]>> {
        let mut counts = HashMap::new();
        for entry in self.tasks.iter(txn)? {
            let (_, raw) = entry?;
            let task: Task = serde_json::from_slice(raw)?;
            if let Some(parent) = task.parent_id {
                counts.entry(parent).or_insert([0; Status::ALL.len()])
                    [keys::status_code(task.status) as usize] += 1;
            }
        }
        Ok(counts)
    }

    pub fn epic(&self, id: u64) -> Result<Epic> {
        let txn = self.env.read_txn()?;
        let metadata = self.checked_metadata(&txn)?;
        let task = self
            .read_task_in_txn(&txn, id)?
            .ok_or(Error::MissingTask(id))?;
        let counts = self.child_counts(&txn)?;
        let counts = counts.get(&id).ok_or(Error::NotEpic(id))?;
        Ok(rollup(
            task,
            counts,
            Cursor {
                instance_id: metadata.instance_id,
                seq: metadata.next_event_seq.saturating_sub(1),
            },
        ))
    }
    /// Filtered page ordered by `(priority, id)`. The status or assignee index picks
    /// the candidates; the remaining filters run on the decoded task.
    pub fn list(&self, filter: &ListFilter<'_>) -> Result<TaskPage> {
        let txn = self.env.read_txn()?;
        let metadata = self.checked_metadata(&txn)?;
        let mut tasks = Vec::new();
        for id in self.candidate_ids(&txn, filter)? {
            let task = self
                .read_task_in_txn(&txn, id)?
                .ok_or(Error::Corrupt("index points to missing task"))?;
            if !keep(filter, &task) {
                continue;
            }
            tasks.push(task);
        }
        tasks.sort_by_key(|task| (task.priority, task.id));
        let total = tasks.len();
        let limit = filter.limit.clamp(1, MAX_LIST_LIMIT);
        let offset = filter.offset.min(total);
        let tasks: Vec<Task> = tasks.into_iter().skip(offset).take(limit).collect();
        let more = offset + tasks.len() < total;
        Ok(TaskPage {
            tasks,
            total,
            more,
            cursor: Cursor {
                instance_id: metadata.instance_id,
                seq: metadata.next_event_seq.saturating_sub(1),
            },
        })
    }

    /// Dashboard page ordered by descending task id, without retaining the full
    /// filtered result in memory. The task database key is the task id in
    /// big-endian order, so its reverse iterator is newest-first.
    pub fn list_newest(&self, filter: &ListFilter<'_>) -> Result<TaskPage> {
        let txn = self.env.read_txn()?;
        let metadata = self.checked_metadata(&txn)?;
        let limit = filter.limit.clamp(1, MAX_LIST_LIMIT);
        let mut total = 0usize;
        let mut tasks = Vec::with_capacity(limit);
        for entry in self.tasks.rev_iter(&txn)? {
            let (_, raw) = entry?;
            let task: Task = serde_json::from_slice(raw)?;
            if !keep(filter, &task) {
                continue;
            }
            if total >= filter.offset && tasks.len() < limit {
                tasks.push(task);
            }
            total += 1;
        }
        let offset = filter.offset.min(total);
        let more = offset.saturating_add(tasks.len()) < total;
        Ok(TaskPage {
            tasks,
            total,
            more,
            cursor: Cursor {
                instance_id: metadata.instance_id,
                seq: metadata.next_event_seq.saturating_sub(1),
            },
        })
    }

    /// Derived epic progress and unparented active work, with global counts.
    pub fn brief(&self, active_limit: usize) -> Result<Brief> {
        let txn = self.env.read_txn()?;
        let metadata = self.checked_metadata(&txn)?;
        let children = self.child_counts(&txn)?;
        let cursor = Cursor {
            instance_id: metadata.instance_id,
            seq: metadata.next_event_seq.saturating_sub(1),
        };
        let mut counts = [0_u64; Status::ALL.len()];
        let mut active = Vec::new();
        let mut epics = Vec::new();
        for entry in self.tasks.iter(&txn)? {
            let (_, raw) = entry?;
            let task: Task = serde_json::from_slice(raw)?;
            counts[keys::status_code(task.status) as usize] += 1;
            if let Some(child_counts) = children.get(&task.id) {
                epics.push(rollup(task, child_counts, cursor.clone()));
            } else if task.parent_id.is_none()
                && !task.status.is_terminal()
                && task.status != Status::Todo
            {
                active.push(task);
            }
        }
        active.sort_by_key(|task| (task.priority, task.id));
        active.truncate(active_limit);
        epics.sort_by_key(|epic| (epic.task.priority, epic.task.id));
        Ok(Brief {
            counts: status_counts(&counts),
            epics,
            active,
        })
    }

    fn candidate_ids(&self, txn: &heed::RoTxn<'_>, filter: &ListFilter<'_>) -> Result<Vec<u64>> {
        let mut ids = Vec::new();
        if let Some(status) = filter.status {
            let prefix = [keys::status_code(status)];
            for entry in self.tasks_by_status.prefix_iter(txn, &prefix)? {
                let (key, _) = entry?;
                ids.push(id_from_index_key(key, 5)?);
            }
            return Ok(ids);
        }
        if let Some(assignee) = filter.assignee {
            let mut prefix = assignee.as_bytes().to_vec();
            prefix.push(0);
            for entry in self.tasks_by_assignee.prefix_iter(txn, &prefix)? {
                let (key, _) = entry?;
                let id_at = key
                    .len()
                    .checked_sub(8)
                    .ok_or(Error::Corrupt("invalid assignee index key"))?;
                ids.push(id_from_index_key(key, id_at)?);
            }
            return Ok(ids);
        }
        for entry in self.tasks.iter(txn)? {
            let (key, _) = entry?;
            ids.push(id_from_index_key(key, 0)?);
        }
        Ok(ids)
    }
}

fn keep(filter: &ListFilter<'_>, task: &Task) -> bool {
    // An explicit status filter is honoured as asked; the terminal exclusion only
    // applies to an unfiltered list.
    if filter.status.is_none()
        && !filter.include_terminal
        && matches!(task.status, Status::Done | Status::Canceled)
    {
        return false;
    }
    if let Some(status) = filter.status
        && task.status != status
    {
        return false;
    }
    if let Some(assignee) = filter.assignee
        && task.assignee.as_deref() != Some(assignee)
    {
        return false;
    }
    if let Some(parent) = filter.parent
        && task.parent_id != Some(parent)
    {
        return false;
    }
    if let Some(path) = filter.path
        && task.path.as_deref() != Some(path)
    {
        return false;
    }
    true
}

fn id_from_index_key(key: &[u8], offset: usize) -> Result<u64> {
    let bytes: [u8; 8] = key
        .get(offset..offset + 8)
        .ok_or(Error::Corrupt("invalid index key"))?
        .try_into()
        .map_err(|_| Error::Corrupt("invalid index key"))?;
    Ok(u64::from_be_bytes(bytes))
}
