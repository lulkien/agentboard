//! Ordered byte keys for the planned LMDB databases.

use crate::domain::{Status, Task};

pub fn task(id: u64) -> [u8; 8] {
    id.to_be_bytes()
}

pub fn event_by_task(task_id: u64, sequence: u64) -> [u8; 16] {
    let mut key = [0; 16];
    key[..8].copy_from_slice(&task_id.to_be_bytes());
    key[8..].copy_from_slice(&sequence.to_be_bytes());
    key
}

/// Persisted index bytes: never renumber, only append.
pub const fn status_code(status: Status) -> u8 {
    match status {
        Status::Todo => 0,
        Status::Implementing => 1,
        Status::Blocked => 2,
        Status::Reviewing => 3,
        Status::Done => 4,
        Status::Canceled => 5,
        Status::Planning => 6,
    }
}

/// Lexical byte order is `(status, signed priority, task ID)`.
pub fn status(task: &Task) -> [u8; 13] {
    let mut key = [0; 13];
    key[0] = status_code(task.status);
    key[1..5].copy_from_slice(&((task.priority as u32) ^ 0x8000_0000).to_be_bytes());
    key[5..].copy_from_slice(&task.id.to_be_bytes());
    key
}

/// NUL ends the validated actor name, then status and task ID follow.
pub fn assignee(task: &Task) -> Option<Vec<u8>> {
    let actor = task.assignee.as_deref()?;
    let mut key = Vec::with_capacity(actor.len() + 10);
    key.extend_from_slice(actor.as_bytes());
    key.push(0);
    key.push(status_code(task.status));
    key.extend_from_slice(&task.id.to_be_bytes());
    Some(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{NewTask, Task};

    fn task(id: u64, status: Status, priority: i32, assignee: Option<&str>) -> Task {
        Task::new(
            id,
            "manager",
            NewTask {
                title: "work".into(),
                description: String::new(),
                goal: String::new(),
                status,
                priority,
                assignee: assignee.map(str::to_owned),
                parent_id: None,
                path: None,
            },
            0,
        )
        .unwrap()
    }

    #[test]
    fn status_codes_are_persisted_and_never_renumbered() {
        let codes: Vec<u8> = [
            Status::Todo,
            Status::Implementing,
            Status::Blocked,
            Status::Reviewing,
            Status::Done,
            Status::Canceled,
            Status::Planning,
        ]
        .map(status_code)
        .to_vec();
        assert_eq!(codes, [0, 1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn integer_keys_sort_numerically() {
        assert!(super::task(2) < super::task(10));
        assert!(event_by_task(1, 10) < event_by_task(2, 1));
        assert!(event_by_task(1, 2) < event_by_task(1, 10));
    }

    #[test]
    fn status_index_sorts_signed_priority_then_id() {
        let keys = [
            status(&task(4, Status::Todo, i32::MIN, None)),
            status(&task(2, Status::Todo, -1, None)),
            status(&task(1, Status::Todo, 0, None)),
            status(&task(3, Status::Todo, 0, None)),
            status(&task(5, Status::Todo, i32::MAX, None)),
            status(&task(1, Status::Implementing, i32::MIN, Some("alice"))),
        ];
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(keys[0][0], status_code(Status::Todo));
        assert_eq!(keys[5][0], status_code(Status::Implementing));
    }

    #[test]
    fn assignee_index_groups_actor_then_status_and_id() {
        let alice_1 = task(1, Status::Todo, 100, Some("alice"));
        let alice_2 = task(2, Status::Todo, 100, Some("alice"));
        let alice_review = task(3, Status::Reviewing, 100, Some("alice"));
        let bob = task(1, Status::Todo, 100, Some("bob"));
        let keys = [
            assignee(&alice_1).unwrap(),
            assignee(&alice_2).unwrap(),
            assignee(&alice_review).unwrap(),
            assignee(&bob).unwrap(),
        ];
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(assignee(&task(1, Status::Todo, 100, None)), None);
    }
}
