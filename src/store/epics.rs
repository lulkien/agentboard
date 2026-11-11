//! Epic dashboard rows from one task-table snapshot.

use std::collections::HashMap;

use crate::domain::{Status, Task};

use super::feed::Cursor;
use super::keys;
use super::list::{Epic, TaskHead, rollup};
use super::open::{Board, Result, read_txn};

pub struct EpicRow {
    pub epic: Epic,
    pub children: Vec<Task>,
    pub children_total: usize,
}

pub struct Epics {
    pub rows: Vec<EpicRow>,
    #[cfg(test)]
    pub tasks_visited: usize,
}

impl Board {
    /// Reads every epic and its child strip from one task-table walk.
    pub fn epics(&self, project: Option<u64>, child_cap: usize) -> Result<Epics> {
        let txn = read_txn(&self.env)?;
        let metadata = self.checked_metadata(&txn)?;
        let cursor = Cursor {
            instance_id: metadata.instance_id,
            seq: metadata.next_event_seq.saturating_sub(1),
        };
        let mut groups: HashMap<u64, ([u64; Status::ALL.len()], Vec<Task>)> = HashMap::new();
        #[cfg(test)]
        let mut tasks_visited = 0;
        for entry in self.tasks.iter(&txn)? {
            let (_, raw) = entry?;
            #[cfg(test)]
            super::note_task_read();
            #[cfg(test)]
            {
                tasks_visited += 1;
            }
            let head: TaskHead<'_> = serde_json::from_slice(raw)?;
            if let Some(parent) = head.parent_id {
                let group = groups
                    .entry(parent)
                    .or_insert(([0; Status::ALL.len()], Vec::new()));
                if project.is_none() || Some(head.project_id) == project {
                    group.0[keys::status_code(head.status) as usize] += 1;
                    group.1.push(serde_json::from_slice(raw)?);
                }
            }
        }
        let mut rows = Vec::with_capacity(groups.len());
        for (parent, (counts, mut children)) in groups {
            let Some(task) = self.read_task_in_txn(&txn, parent)? else {
                continue;
            };
            if project.is_some() && Some(task.project_id) != project {
                continue;
            }
            children.sort_by_key(|task| (task.priority, task.id));
            let children_total = children.len();
            children.truncate(child_cap);
            rows.push(EpicRow {
                epic: rollup(task, &counts, cursor.clone()),
                children,
                children_total,
            });
        }
        rows.sort_by_key(|row| (row.epic.task.priority, row.epic.task.id));
        Ok(Epics {
            rows,
            #[cfg(test)]
            tasks_visited,
        })
    }
}
