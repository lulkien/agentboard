use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::domain::{Event, EventKind, NewTask, Status, Task, TaskPatch, validate_actor};

use super::keys;
use super::open::{Board, Error, Result, write_metadata};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Agent {
    pub name: String,
    pub first_seen_ms: i64,
    pub last_seen_ms: Option<i64>,
    pub last_task_id: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Added {
    pub task: Task,
    pub event: Event,
}

pub struct UpdateInput<'a> {
    pub id: u64,
    pub actor: &'a str,
    pub expected_revision: u64,
    pub patch: &'a TaskPatch,
    pub force: bool,
    pub force_reason: Option<&'a str>,
    pub new_actor: bool,
    pub comment: Option<&'a str>,
    pub now_ms: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Updated {
    pub task: Task,
    pub event: Event,
}

impl Board {
    /// Create the task, roster changes, indexes, and event in one write txn.
    /// `new_actor` registers an unknown target assignee, never the caller.
    pub fn add(&self, actor: &str, input: NewTask, new_actor: bool, now_ms: i64) -> Result<Added> {
        let mut txn = self.env.write_txn()?;
        let mut metadata = self.checked_metadata(&txn)?;
        let task_id = metadata.next_task_id;
        let event_seq = metadata.next_event_seq;
        let task = Task::new(task_id, actor, input, now_ms)?;
        if let Some(parent_id) = task.parent_id
            && self.tasks.get(&txn, &keys::task(parent_id))?.is_none()
        {
            return Err(Error::MissingParent(parent_id));
        }
        self.ensure_target(&mut txn, task.assignee.as_deref(), actor, new_actor, now_ms)?;
        self.touch_actor(&mut txn, actor, task_id, now_ms)?;
        let event = Event {
            seq: event_seq,
            task_id,
            actor: actor.to_owned(),
            kind: EventKind::Created,
            timestamp_ms: now_ms,
            recipient: task.assignee.clone(),
            payload: serde_json::json!({"task": task}),
        };
        self.tasks
            .put(&mut txn, &keys::task(task_id), &serde_json::to_vec(&task)?)?;
        self.events.put(
            &mut txn,
            &keys::task(event_seq),
            &serde_json::to_vec(&event)?,
        )?;
        self.events_by_task
            .put(&mut txn, &keys::event_by_task(task_id, event_seq), b"")?;
        self.tasks_by_status
            .put(&mut txn, &keys::status(&task), b"")?;
        if let Some(index_key) = keys::assignee(&task) {
            self.tasks_by_assignee.put(&mut txn, &index_key, b"")?;
        }
        metadata.next_task_id = task_id.checked_add(1).ok_or(Error::CounterOverflow)?;
        metadata.next_event_seq = event_seq.checked_add(1).ok_or(Error::CounterOverflow)?;
        write_metadata(self.meta, &mut txn, &metadata)?;
        txn.commit()?;
        Ok(Added { task, event })
    }

    pub fn update(&self, input: UpdateInput<'_>) -> Result<Updated> {
        let mut txn = self.env.write_txn()?;
        let mut metadata = self.checked_metadata(&txn)?;
        let old = self
            .read_task_in_txn(&txn, input.id)?
            .ok_or(Error::MissingTask(input.id))?;
        let applied = old.apply(
            input.actor,
            input.expected_revision,
            input.patch,
            input.force,
            input.force_reason,
            input.now_ms,
        )?;
        let next = applied.task;
        self.check_parent_chain(&txn, &next)?;
        self.ensure_target(
            &mut txn,
            next.assignee.as_deref(),
            input.actor,
            input.new_actor,
            input.now_ms,
        )?;
        self.touch_actor(&mut txn, input.actor, input.id, input.now_ms)?;
        let old_status_key = keys::status(&old);
        if !self.tasks_by_status.delete(&mut txn, &old_status_key)? {
            return Err(Error::Corrupt("missing old status index key"));
        }
        if let Some(old_assignee_key) = keys::assignee(&old)
            && !self.tasks_by_assignee.delete(&mut txn, &old_assignee_key)?
        {
            return Err(Error::Corrupt("missing old assignee index key"));
        }
        self.tasks_by_status
            .put(&mut txn, &keys::status(&next), b"")?;
        if let Some(next_assignee_key) = keys::assignee(&next) {
            self.tasks_by_assignee
                .put(&mut txn, &next_assignee_key, b"")?;
        }
        let event_seq = metadata.next_event_seq;
        let recipient = if next.assignee != old.assignee {
            next.assignee.clone()
        } else {
            None
        };
        let mut payload = serde_json::Map::new();
        payload.insert("changes".into(), changes(&old, &next)?);
        if let Some(reason) = applied.force_reason {
            payload.insert("reason".into(), reason.into());
        }
        if let Some(comment) = input.comment {
            payload.insert("comment".into(), comment.into());
        }
        let event = Event {
            seq: event_seq,
            task_id: input.id,
            actor: input.actor.to_owned(),
            kind: EventKind::Updated,
            timestamp_ms: input.now_ms,
            recipient,
            payload: payload.into(),
        };
        self.tasks
            .put(&mut txn, &keys::task(input.id), &serde_json::to_vec(&next)?)?;
        self.events.put(
            &mut txn,
            &keys::task(event_seq),
            &serde_json::to_vec(&event)?,
        )?;
        self.events_by_task
            .put(&mut txn, &keys::event_by_task(input.id, event_seq), b"")?;
        metadata.next_event_seq = event_seq.checked_add(1).ok_or(Error::CounterOverflow)?;
        write_metadata(self.meta, &mut txn, &metadata)?;
        txn.commit()?;
        Ok(Updated { task: next, event })
    }

    pub fn get(&self, id: u64) -> Result<Option<Task>> {
        let txn = self.env.read_txn()?;
        self.checked_metadata(&txn)?;
        let Some(raw) = self.tasks.get(&txn, &keys::task(id))? else {
            return Ok(None);
        };
        let task: Task = serde_json::from_slice(raw)?;
        task.validate()?;
        if task.id != id {
            return Err(Error::Corrupt("task key/ID mismatch"));
        }
        Ok(Some(task))
    }

    /// Read-only candidate probe. The later claim write rechecks this same
    /// index under its write transaction before changing task ownership.
    pub fn next_claimable_id(&self, actor: &str) -> Result<Option<u64>> {
        validate_actor(actor)?;
        let txn = self.env.read_txn()?;
        self.checked_metadata(&txn)?;
        Ok(self.next_claimable_in_txn(&txn, actor)?.map(|task| task.id))
    }

    pub fn event(&self, sequence: u64) -> Result<Option<Event>> {
        let txn = self.env.read_txn()?;
        self.checked_metadata(&txn)?;
        let Some(raw) = self.events.get(&txn, &keys::task(sequence))? else {
            return Ok(None);
        };
        let event: Event = serde_json::from_slice(raw)?;
        if event.seq != sequence {
            return Err(Error::Corrupt("event key/sequence mismatch"));
        }
        Ok(Some(event))
    }

    pub fn agent(&self, name: &str) -> Result<Option<Agent>> {
        let txn = self.env.read_txn()?;
        self.checked_metadata(&txn)?;
        let Some(raw) = self.agents.get(&txn, name.as_bytes())? else {
            return Ok(None);
        };
        let agent: Agent = serde_json::from_slice(raw)?;
        if agent.name != name {
            return Err(Error::Corrupt("actor roster key/name mismatch"));
        }
        Ok(Some(agent))
    }

    fn read_task_in_txn(&self, txn: &heed::RoTxn<'_>, id: u64) -> Result<Option<Task>> {
        let Some(raw) = self.tasks.get(txn, &keys::task(id))? else {
            return Ok(None);
        };
        let task: Task = serde_json::from_slice(raw)?;
        task.validate()?;
        if task.id != id {
            return Err(Error::Corrupt("task key/ID mismatch"));
        }
        Ok(Some(task))
    }

    fn next_claimable_in_txn(&self, txn: &heed::RoTxn<'_>, actor: &str) -> Result<Option<Task>> {
        let prefix = [keys::status_code(Status::Todo)];
        for entry in self.tasks_by_status.prefix_iter(txn, &prefix)? {
            let (key, _) = entry?;
            let id_bytes: [u8; 8] = key
                .get(5..13)
                .ok_or(Error::Corrupt("invalid status index key"))?
                .try_into()
                .map_err(|_| Error::Corrupt("invalid status index key"))?;
            if key.len() != 13 {
                return Err(Error::Corrupt("invalid status index key"));
            }
            let id = u64::from_be_bytes(id_bytes);
            let task = self
                .read_task_in_txn(txn, id)?
                .ok_or(Error::Corrupt("status index points to missing task"))?;
            if task.status != Status::Todo || keys::status(&task) != key {
                return Err(Error::Corrupt("status index disagrees with task"));
            }
            if task
                .assignee
                .as_deref()
                .is_none_or(|holder| holder == actor)
            {
                return Ok(Some(task));
            }
        }
        Ok(None)
    }

    fn check_parent_chain(&self, txn: &heed::RoTxn<'_>, task: &Task) -> Result<()> {
        let mut seen = HashSet::new();
        let mut parent_id = task.parent_id;
        while let Some(id) = parent_id {
            if id == task.id {
                return Err(Error::ParentCycle(task.id));
            }
            if !seen.insert(id) {
                return Err(Error::Corrupt("existing parent cycle"));
            }
            let parent = self
                .read_task_in_txn(txn, id)?
                .ok_or(Error::MissingParent(id))?;
            parent_id = parent.parent_id;
        }
        Ok(())
    }

    fn ensure_target(
        &self,
        txn: &mut heed::RwTxn<'_>,
        target: Option<&str>,
        actor: &str,
        new_actor: bool,
        now_ms: i64,
    ) -> Result<()> {
        if let Some(target) = target
            && target != actor
            && self.agents.get(txn, target.as_bytes())?.is_none()
        {
            if !new_actor {
                return Err(Error::UnknownAssignee(target.to_owned()));
            }
            let record = Agent {
                name: target.to_owned(),
                first_seen_ms: now_ms,
                last_seen_ms: None,
                last_task_id: None,
            };
            self.agents
                .put(txn, target.as_bytes(), &serde_json::to_vec(&record)?)?;
        }
        Ok(())
    }

    fn touch_actor(
        &self,
        txn: &mut heed::RwTxn<'_>,
        actor: &str,
        task_id: u64,
        now_ms: i64,
    ) -> Result<()> {
        let record = match self.agents.get(txn, actor.as_bytes())? {
            Some(raw) => {
                let mut record: Agent = serde_json::from_slice(raw)?;
                if record.name != actor {
                    return Err(Error::Corrupt("actor roster key/name mismatch"));
                }
                record.last_seen_ms = Some(now_ms);
                record.last_task_id = Some(task_id);
                record
            }
            None => Agent {
                name: actor.to_owned(),
                first_seen_ms: now_ms,
                last_seen_ms: Some(now_ms),
                last_task_id: Some(task_id),
            },
        };
        self.agents
            .put(txn, actor.as_bytes(), &serde_json::to_vec(&record)?)?;
        Ok(())
    }
}

fn changes(old: &Task, next: &Task) -> Result<serde_json::Value> {
    let old_value = serde_json::to_value(old)?;
    let next_value = serde_json::to_value(next)?;
    let old_fields = old_value
        .as_object()
        .ok_or(Error::Corrupt("task is not a JSON object"))?;
    let next_fields = next_value
        .as_object()
        .ok_or(Error::Corrupt("task is not a JSON object"))?;
    let mut changed = serde_json::Map::new();
    for field in [
        "title",
        "description",
        "goal",
        "status",
        "priority",
        "assignee",
        "parent_id",
        "path",
    ] {
        let before = old_fields
            .get(field)
            .ok_or(Error::Corrupt("task field missing"))?;
        let after = next_fields
            .get(field)
            .ok_or(Error::Corrupt("task field missing"))?;
        if before != after {
            changed.insert(
                field.into(),
                serde_json::json!({"old": before, "new": after}),
            );
        }
    }
    Ok(changed.into())
}
