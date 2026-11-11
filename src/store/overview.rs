//! Dashboard snapshot: status lanes, attention rows, and held work.

use std::collections::HashMap;

use crate::domain::{Status, Task};

use super::feed::Cursor;
use super::keys;
use super::list::TaskHead;
use super::open::{Board, Result, read_txn};

pub struct Overview {
    pub cursor: Cursor,
    pub counts: Vec<(Status, u64)>,
    pub lanes: Vec<Lane>,
    pub attention: Vec<Task>,
    pub attention_total: usize,
    pub held: Vec<Held>,
}

pub struct Lane {
    pub status: Status,
    pub tasks: Vec<Task>,
    pub total: usize,
}

pub struct Held {
    pub assignee: String,
    pub tasks: Vec<Task>,
    pub total: usize,
}

impl Board {
    /// All dashboard groups come from one read snapshot and one task walk.
    pub fn overview(&self, project: Option<u64>, cap: usize) -> Result<Overview> {
        let txn = read_txn(&self.env)?;
        let metadata = self.checked_metadata(&txn)?;
        let mut counts = [0_u64; Status::ALL.len()];
        let mut lanes = vec![Vec::new(); Status::ALL.len()];
        let mut held: HashMap<String, Vec<Task>> = HashMap::new();
        let mut attention = Vec::new();
        for entry in self.tasks.iter(&txn)? {
            let (_, raw) = entry?;
            #[cfg(test)]
            super::note_task_read();
            let head: TaskHead<'_> = serde_json::from_slice(raw)?;
            if project.is_some() && Some(head.project_id) != project {
                continue;
            }
            counts[keys::status_code(head.status) as usize] += 1;
            if head.status.is_terminal() {
                continue;
            }
            let task: Task = serde_json::from_slice(raw)?;
            if !task.status.is_terminal() {
                lanes[keys::status_code(task.status) as usize].push(task.clone());
                if let Some(assignee) = &task.assignee {
                    held.entry(assignee.clone()).or_default().push(task.clone());
                }
            }
            if matches!(task.status, Status::Blocked | Status::Reviewing)
                || (task.status == Status::Todo && task.assignee.is_some())
            {
                attention.push(task);
            }
        }
        let attention_total = attention.len();
        attention.sort_by_key(|task| {
            let group = match task.status {
                Status::Blocked => 0,
                Status::Todo => 1,
                _ => 2,
            };
            (group, task.updated_at_ms, task.id)
        });
        attention.truncate(cap);
        let lanes = Status::ALL
            .iter()
            .copied()
            .filter(|status| !status.is_terminal())
            .map(|status| {
                let mut tasks = std::mem::take(&mut lanes[keys::status_code(status) as usize]);
                tasks.sort_by_key(|task| (task.priority, task.id));
                let total = tasks.len();
                tasks.truncate(cap);
                Lane {
                    status,
                    tasks,
                    total,
                }
            })
            .collect();
        let mut held: Vec<Held> = held
            .into_iter()
            .map(|(assignee, mut tasks)| {
                tasks.sort_by_key(|task| (task.priority, task.id));
                let total = tasks.len();
                tasks.truncate(cap);
                Held {
                    assignee,
                    tasks,
                    total,
                }
            })
            .collect();
        held.sort_by(|a, b| a.assignee.cmp(&b.assignee));
        Ok(Overview {
            cursor: Cursor {
                instance_id: metadata.instance_id,
                seq: metadata.next_event_seq.saturating_sub(1),
            },
            counts: Status::ALL
                .iter()
                .map(|status| (*status, counts[keys::status_code(*status) as usize]))
                .collect(),
            lanes,
            attention,
            attention_total,
            held,
        })
    }
}
