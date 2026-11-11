//! Task listing and the compact board summary.

use std::borrow::Cow;
use std::collections::HashMap;

use serde::Deserialize;

use crate::domain::{Status, Task};

use super::feed::Cursor;
use super::keys;
use super::open::{Board, Error, ReadTxn, Result, read_txn};

pub const DEFAULT_LIST_LIMIT: usize = 20;
pub const MAX_LIST_LIMIT: usize = 200;

#[derive(Deserialize)]
pub(super) struct TaskHead<'a> {
    pub id: u64,
    pub status: Status,
    #[serde(borrow)]
    pub assignee: Option<Cow<'a, str>>,
    pub parent_id: Option<u64>,
    pub project_id: Option<u64>,
}

impl<'a> From<&'a Task> for TaskHead<'a> {
    fn from(task: &'a Task) -> Self {
        Self {
            id: task.id,
            status: task.status,
            assignee: task.assignee.as_deref().map(Cow::Borrowed),
            parent_id: task.parent_id,
            project_id: task.project_id,
        }
    }
}

pub struct ListFilter<'a> {
    pub status: Option<Status>,
    pub assignee: Option<&'a str>,
    pub parent: Option<u64>,
    pub project: Option<u64>,
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

pub(super) fn rollup(task: Task, counts: &[u64; Status::ALL.len()], cursor: Cursor) -> Epic {
    Epic {
        task,
        counts: status_counts(counts),
        done: counts[keys::status_code(Status::Done) as usize],
        total: counts.iter().sum(),
        cursor,
    }
}

impl Board {
    /// Counts for every status after all other filters, including terminal rows.
    pub fn status_counts(&self, filter: &ListFilter<'_>) -> Result<Vec<(Status, u64)>> {
        let txn = read_txn(&self.env)?;
        self.checked_metadata(&txn)?;
        let without_status = ListFilter {
            status: None,
            assignee: filter.assignee,
            parent: filter.parent,
            project: filter.project,
            include_terminal: true,
            limit: filter.limit,
            offset: filter.offset,
        };
        let mut counts = [0_u64; Status::ALL.len()];
        for entry in self.tasks.iter(&txn)? {
            let (_, raw) = entry?;
            #[cfg(test)]
            super::note_task_read();
            let task: TaskHead<'_> = serde_json::from_slice(raw)?;
            if keep(&without_status, &task) {
                counts[keys::status_code(task.status) as usize] += 1;
            }
        }
        Ok(status_counts(&counts))
    }

    /// Direct-child counts in the caller's snapshot, including write transactions.
    pub(super) fn child_counts(
        &self,
        txn: &ReadTxn<'_>,
    ) -> Result<HashMap<u64, [u64; Status::ALL.len()]>> {
        self.child_counts_for_project(txn, None)
    }

    fn child_counts_for_project(
        &self,
        txn: &ReadTxn<'_>,
        project: Option<u64>,
    ) -> Result<HashMap<u64, [u64; Status::ALL.len()]>> {
        let mut counts = HashMap::new();
        for entry in self.tasks.iter(txn)? {
            let (_, raw) = entry?;
            #[cfg(test)]
            super::note_task_read();
            let task: TaskHead<'_> = serde_json::from_slice(raw)?;
            if let Some(parent) = task.parent_id {
                let child_counts = counts.entry(parent).or_insert([0; Status::ALL.len()]);
                if project.is_none() || task.project_id == project {
                    child_counts[keys::status_code(task.status) as usize] += 1;
                }
            }
        }
        Ok(counts)
    }

    pub fn epic(&self, id: u64) -> Result<Epic> {
        let txn = read_txn(&self.env)?;
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
        let txn = read_txn(&self.env)?;
        let metadata = self.checked_metadata(&txn)?;
        let mut tasks = Vec::new();
        for id in self.candidate_ids(&txn, filter)? {
            let task = self
                .read_task_in_txn(&txn, id)?
                .ok_or(Error::Corrupt("index points to missing task"))?;
            if !keep(filter, &TaskHead::from(&task)) {
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
        let txn = read_txn(&self.env)?;
        let metadata = self.checked_metadata(&txn)?;
        let limit = filter.limit.clamp(1, MAX_LIST_LIMIT);
        let mut total = 0usize;
        let mut tasks = Vec::with_capacity(limit);
        for entry in self.tasks.rev_iter(&txn)? {
            let (_, raw) = entry?;
            #[cfg(test)]
            super::note_task_read();
            let task: TaskHead<'_> = serde_json::from_slice(raw)?;
            if !keep(filter, &task) {
                continue;
            }
            if total >= filter.offset && tasks.len() < limit {
                tasks.push(serde_json::from_slice(raw)?);
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
        self.brief_for_project(active_limit, None)
    }

    /// Derived epic progress and active work for one project, or the whole board.
    pub fn brief_for_project(&self, active_limit: usize, project: Option<u64>) -> Result<Brief> {
        let txn = read_txn(&self.env)?;
        let metadata = self.checked_metadata(&txn)?;
        let children = self.child_counts_for_project(&txn, project)?;
        let cursor = Cursor {
            instance_id: metadata.instance_id,
            seq: metadata.next_event_seq.saturating_sub(1),
        };
        let mut counts = [0_u64; Status::ALL.len()];
        let mut active: Vec<Task> = Vec::new();
        let mut epics = Vec::new();
        for entry in self.tasks.iter(&txn)? {
            let (_, raw) = entry?;
            #[cfg(test)]
            super::note_task_read();
            let task: TaskHead<'_> = serde_json::from_slice(raw)?;
            if project.is_some() && task.project_id != project {
                continue;
            }
            counts[keys::status_code(task.status) as usize] += 1;
            if let Some(child_counts) = children.get(&task.id) {
                epics.push(rollup(
                    serde_json::from_slice(raw)?,
                    child_counts,
                    cursor.clone(),
                ));
            } else if task.parent_id.is_none()
                && !task.status.is_terminal()
                && task.status != Status::Todo
            {
                active.push(serde_json::from_slice(raw)?);
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

    fn candidate_ids(&self, txn: &ReadTxn<'_>, filter: &ListFilter<'_>) -> Result<Vec<u64>> {
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
            #[cfg(test)]
            super::note_task_read();
            ids.push(id_from_index_key(key, 0)?);
        }
        Ok(ids)
    }
}

fn keep(filter: &ListFilter<'_>, task: &TaskHead<'_>) -> bool {
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
    if let Some(project) = filter.project
        && task.project_id != Some(project)
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
