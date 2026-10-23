//! Board records and rules that do not depend on LMDB or an adapter.
//!
//! Store operations will enforce these rules inside a write transaction and
//! append events in that same transaction. This module never writes a board.

use serde::{Deserialize, Serialize};

pub const MAX_ACTOR_CHARS: usize = 64;
pub const MAX_TITLE_CHARS: usize = 200;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Todo,
    InProgress,
    Blocked,
    Review,
    Done,
    Canceled,
}

impl Status {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::InProgress => "in_progress",
            Self::Blocked => "blocked",
            Self::Review => "review",
            Self::Done => "done",
            Self::Canceled => "canceled",
        }
    }

    pub fn parse(value: &str) -> Result<Self, Error> {
        match value {
            "todo" => Ok(Self::Todo),
            "in_progress" => Ok(Self::InProgress),
            "blocked" => Ok(Self::Blocked),
            "review" => Ok(Self::Review),
            "done" => Ok(Self::Done),
            "canceled" => Ok(Self::Canceled),
            _ => Err(Error::UnknownStatus(value.to_owned())),
        }
    }

    pub const fn requires_assignee(self) -> bool {
        matches!(self, Self::InProgress | Self::Blocked)
    }

    pub const fn protects_assignee(self) -> bool {
        matches!(self, Self::InProgress | Self::Blocked | Self::Review)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    EmptyActor,
    InvalidActor,
    EmptyAssignee,
    InvalidAssignee,
    EmptyTitle,
    InvalidTitle,
    UnknownStatus(String),
    SelfParent,
    MissingAssignee(Status),
    MissingForceReason,
    ForceReasonWithoutForce,
    Held { holder: String },
    StaleRevision { expected: u64, actual: u64 },
    NotClaimable(Status),
    NoChange,
    RevisionOverflow,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyActor => f.write_str("actor must be nonempty"),
            Self::InvalidActor => write!(
                f,
                "actor must be trimmed, have no control characters, and be at most {MAX_ACTOR_CHARS} characters"
            ),
            Self::EmptyAssignee => f.write_str("assignee must be nonempty when set"),
            Self::InvalidAssignee => f.write_str("assignee must be a valid actor name"),
            Self::EmptyTitle => f.write_str("task title must be nonempty"),
            Self::InvalidTitle => write!(
                f,
                "task title must be trimmed, have no control characters, and be at most {MAX_TITLE_CHARS} characters"
            ),
            Self::UnknownStatus(value) => write!(f, "unknown task status: {value:?}"),
            Self::SelfParent => f.write_str("task cannot be its own parent"),
            Self::MissingAssignee(status) => {
                write!(f, "{} requires an assignee", status.as_str())
            }
            Self::MissingForceReason => f.write_str("forced update requires a reason"),
            Self::ForceReasonWithoutForce => f.write_str("force reason requires force=true"),
            Self::Held { holder } => write!(f, "task is held by {holder}"),
            Self::StaleRevision { expected, actual } => {
                write!(
                    f,
                    "stale task revision {expected}; current revision is {actual}"
                )
            }
            Self::NotClaimable(status) => write!(f, "{} cannot be claimed", status.as_str()),
            Self::NoChange => f.write_str("task update has no field changes"),
            Self::RevisionOverflow => f.write_str("task revision overflow"),
        }
    }
}

impl std::error::Error for Error {}

pub fn validate_actor(actor: &str) -> Result<(), Error> {
    if actor.trim().is_empty() {
        Err(Error::EmptyActor)
    } else if actor.trim() != actor
        || actor.chars().count() > MAX_ACTOR_CHARS
        || actor.chars().any(char::is_control)
    {
        Err(Error::InvalidActor)
    } else {
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: u64,
    pub title: String,
    pub description: String,
    pub goal: String,
    pub status: Status,
    pub priority: i32,
    pub reporter: String,
    pub assignee: Option<String>,
    pub parent_id: Option<u64>,
    pub path: Option<String>,
    pub revision: u64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewTask {
    pub title: String,
    pub description: String,
    pub goal: String,
    pub status: Status,
    pub priority: i32,
    pub assignee: Option<String>,
    pub parent_id: Option<u64>,
    pub path: Option<String>,
}

impl Task {
    pub fn new(id: u64, actor: &str, input: NewTask, now_ms: i64) -> Result<Self, Error> {
        validate_actor(actor)?;
        let task = Self {
            id,
            title: input.title,
            description: input.description,
            goal: input.goal,
            status: input.status,
            priority: input.priority,
            reporter: actor.to_owned(),
            assignee: input.assignee,
            parent_id: input.parent_id,
            path: input.path,
            revision: 1,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        };
        task.validate()?;
        Ok(task)
    }

    pub fn validate(&self) -> Result<(), Error> {
        if self.title.trim().is_empty() {
            return Err(Error::EmptyTitle);
        }
        if self.title.trim() != self.title
            || self.title.chars().count() > MAX_TITLE_CHARS
            || self.title.chars().any(char::is_control)
        {
            return Err(Error::InvalidTitle);
        }
        validate_actor(&self.reporter)?;
        if let Some(assignee) = self.assignee.as_deref() {
            validate_actor(assignee).map_err(|error| match error {
                Error::EmptyActor => Error::EmptyAssignee,
                _ => Error::InvalidAssignee,
            })?;
        }
        if self.status.requires_assignee() && self.assignee.is_none() {
            return Err(Error::MissingAssignee(self.status));
        }
        if self.parent_id == Some(self.id) {
            return Err(Error::SelfParent);
        }
        Ok(())
    }

    /// Prepares a claim using the same revision and ownership rules as updates.
    /// The store persists the returned task and event in one write transaction.
    /// An id-less store scan skips `Held` and `NotClaimable` candidates; an
    /// explicit task ID returns those refusals to the caller.
    /// `AlreadyHeld` is a read: the store must not append an event for it.
    pub fn claim(&self, actor: &str, explicit: bool, now_ms: i64) -> Result<Claim, Error> {
        validate_actor(actor)?;
        if let Some(holder) = self.assignee.as_deref()
            && holder != actor
        {
            return Err(Error::Held {
                holder: holder.to_owned(),
            });
        }
        match self.status {
            Status::Todo => {
                let patch = TaskPatch {
                    status: Some(Status::InProgress),
                    assignee: Some(Some(actor.to_owned())),
                    ..TaskPatch::default()
                };
                Ok(Claim::Started(
                    self.apply(actor, self.revision, &patch, false, None, now_ms)?
                        .task,
                ))
            }
            Status::Review if explicit && self.assignee.is_none() => {
                let patch = TaskPatch {
                    assignee: Some(Some(actor.to_owned())),
                    ..TaskPatch::default()
                };
                Ok(Claim::ReviewTaken(
                    self.apply(actor, self.revision, &patch, false, None, now_ms)?
                        .task,
                ))
            }
            Status::Review | Status::InProgress
                if explicit && self.assignee.as_deref() == Some(actor) =>
            {
                Ok(Claim::AlreadyHeld(self.clone()))
            }
            status => Err(Error::NotClaimable(status)),
        }
    }

    /// Prepare one task-field update. The store commits the returned task and
    /// audit reason with its event after checking references and the roster.
    /// A foreign holder's protected fields require both `force` and a reason.
    pub fn apply(
        &self,
        actor: &str,
        expected_revision: u64,
        patch: &TaskPatch,
        force: bool,
        force_reason: Option<&str>,
        now_ms: i64,
    ) -> Result<Applied, Error> {
        validate_actor(actor)?;
        if expected_revision != self.revision {
            return Err(Error::StaleRevision {
                expected: expected_revision,
                actual: self.revision,
            });
        }
        if force && !force_reason.is_some_and(|reason| !reason.trim().is_empty()) {
            return Err(Error::MissingForceReason);
        }
        if !force && force_reason.is_some() {
            return Err(Error::ForceReasonWithoutForce);
        }
        if !force
            && let Some(holder) = self
                .assignee
                .as_deref()
                .filter(|holder| self.status.protects_assignee() && *holder != actor)
        {
            return Err(Error::Held {
                holder: holder.to_owned(),
            });
        }
        let mut next = self.clone();
        if let Some(title) = &patch.title {
            next.title.clone_from(title);
        }
        if let Some(description) = &patch.description {
            next.description.clone_from(description);
        }
        if let Some(goal) = &patch.goal {
            next.goal.clone_from(goal);
        }
        if let Some(status) = patch.status {
            next.status = status;
        }
        if let Some(priority) = patch.priority {
            next.priority = priority;
        }
        if let Some(assignee) = &patch.assignee {
            next.assignee.clone_from(assignee);
        }
        if let Some(parent_id) = patch.parent_id {
            next.parent_id = parent_id;
        }
        if let Some(path) = &patch.path {
            next.path.clone_from(path);
        }
        next.validate()?;
        if next == *self {
            return Err(Error::NoChange);
        }
        next.revision = self
            .revision
            .checked_add(1)
            .ok_or(Error::RevisionOverflow)?;
        next.updated_at_ms = now_ms;
        Ok(Applied {
            task: next,
            force_reason: force_reason.map(str::to_owned),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Claim {
    Started(Task),
    ReviewTaken(Task),
    AlreadyHeld(Task),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Applied {
    pub task: Task,
    /// Present for any successful field change made with `force=true`.
    pub force_reason: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub goal: Option<String>,
    pub status: Option<Status>,
    pub priority: Option<i32>,
    /// `None` leaves the field alone, `Some(None)` clears it.
    pub assignee: Option<Option<String>>,
    pub parent_id: Option<Option<u64>>,
    pub path: Option<Option<String>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Created,
    Updated,
    Claimed,
    Commented,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub task_id: u64,
    pub actor: String,
    pub kind: EventKind,
    pub timestamp_ms: i64,
    pub recipient: Option<String>,
    pub payload: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> NewTask {
        NewTask {
            title: "Fix claim race".into(),
            description: "".into(),
            goal: "exactly one winner".into(),
            status: Status::Todo,
            priority: 100,
            assignee: None,
            parent_id: None,
            path: None,
        }
    }

    #[test]
    fn creation_checks_actor_title_and_active_owner() {
        assert_eq!(Task::new(1, "  ", draft(), 0), Err(Error::EmptyActor));
        let mut input = draft();
        input.title = " \n ".into();
        assert_eq!(Task::new(1, "alice", input, 0), Err(Error::EmptyTitle));
        let mut input = draft();
        input.status = Status::InProgress;
        assert_eq!(
            Task::new(1, "alice", input, 0),
            Err(Error::MissingAssignee(Status::InProgress))
        );
        let mut input = draft();
        input.assignee = Some("  ".into());
        assert_eq!(Task::new(1, "alice", input, 0), Err(Error::EmptyAssignee));
        let task = Task::new(1, "alice", draft(), 12).unwrap();
        assert_eq!(task.revision, 1);
        assert_eq!(task.reporter, "alice");
    }

    #[test]
    fn claim_does_not_take_other_actors_work() {
        let task = Task::new(1, "manager", draft(), 0).unwrap();
        let Claim::Started(started) = task.claim("alice", false, 1).unwrap() else {
            panic!("todo claim must start work");
        };
        assert_eq!(started.status, Status::InProgress);
        assert_eq!(started.assignee.as_deref(), Some("alice"));
        assert_eq!(started.revision, 2);
        assert_eq!(started.updated_at_ms, 1);
        let own = task
            .apply(
                "manager",
                1,
                &TaskPatch {
                    assignee: Some(Some("alice".into())),
                    status: Some(Status::InProgress),
                    ..TaskPatch::default()
                },
                false,
                None,
                1,
            )
            .unwrap()
            .task;
        assert_eq!(
            own.claim("bob", true, 2),
            Err(Error::Held {
                holder: "alice".into()
            })
        );
        assert_eq!(
            own.claim("alice", true, 2),
            Ok(Claim::AlreadyHeld(own.clone()))
        );
        assert_eq!(
            own.claim("alice", false, 2),
            Err(Error::NotClaimable(Status::InProgress))
        );
    }

    #[test]
    fn stale_owner_cannot_overwrite_reassignment() {
        let task = Task::new(1, "manager", draft(), 0).unwrap();
        let held = task
            .apply(
                "manager",
                1,
                &TaskPatch {
                    assignee: Some(Some("alice".into())),
                    status: Some(Status::InProgress),
                    ..TaskPatch::default()
                },
                false,
                None,
                1,
            )
            .unwrap()
            .task;
        let take_over = TaskPatch {
            assignee: Some(Some("bob".into())),
            ..TaskPatch::default()
        };
        assert_eq!(
            held.apply("manager", 2, &take_over, false, None, 2),
            Err(Error::Held {
                holder: "alice".into()
            })
        );
        assert_eq!(
            held.apply("manager", 2, &take_over, true, Some(" "), 2),
            Err(Error::MissingForceReason)
        );
        assert_eq!(
            held.apply("manager", 2, &take_over, true, None, 2),
            Err(Error::MissingForceReason)
        );
        assert_eq!(
            held.apply("manager", 2, &take_over, false, Some("quota"), 2),
            Err(Error::ForceReasonWithoutForce)
        );
        let replaced = held
            .apply("manager", 2, &take_over, true, Some("quota"), 2)
            .unwrap();
        assert_eq!(replaced.force_reason.as_deref(), Some("quota"));
        let replaced = replaced.task;
        assert_eq!(replaced.assignee.as_deref(), Some("bob"));
        assert_eq!(replaced.revision, 3);
        assert_eq!(
            replaced.apply("alice", 2, &TaskPatch::default(), false, None, 3),
            Err(Error::StaleRevision {
                expected: 2,
                actual: 3
            })
        );
        assert_eq!(
            replaced.apply("alice", 3, &TaskPatch::default(), false, None, 3),
            Err(Error::Held {
                holder: "bob".into()
            })
        );
    }

    #[test]
    fn clearing_owner_requires_non_active_status() {
        let task = Task::new(1, "manager", draft(), 0).unwrap();
        let held = task
            .apply(
                "manager",
                1,
                &TaskPatch {
                    assignee: Some(Some("alice".into())),
                    status: Some(Status::InProgress),
                    ..TaskPatch::default()
                },
                false,
                None,
                1,
            )
            .unwrap()
            .task;
        assert_eq!(
            held.apply(
                "alice",
                2,
                &TaskPatch {
                    assignee: Some(None),
                    ..TaskPatch::default()
                },
                false,
                None,
                2
            ),
            Err(Error::MissingAssignee(Status::InProgress))
        );
        let released = held
            .apply(
                "alice",
                2,
                &TaskPatch {
                    assignee: Some(None),
                    status: Some(Status::Todo),
                    ..TaskPatch::default()
                },
                false,
                None,
                2,
            )
            .unwrap()
            .task;
        assert_eq!(released.status, Status::Todo);
        assert_eq!(released.assignee, None);
    }

    #[test]
    fn coder_hands_one_task_to_reviewer() {
        let mut input = draft();
        input.assignee = Some("coder".into());
        let task = Task::new(7, "manager", input, 10).unwrap();
        let working = task
            .apply(
                "coder",
                1,
                &TaskPatch {
                    status: Some(Status::InProgress),
                    ..TaskPatch::default()
                },
                false,
                None,
                11,
            )
            .unwrap()
            .task;
        let review = working
            .apply(
                "coder",
                2,
                &TaskPatch {
                    status: Some(Status::Review),
                    assignee: Some(Some("reviewer".into())),
                    ..TaskPatch::default()
                },
                false,
                None,
                12,
            )
            .unwrap()
            .task;
        assert_eq!(review.id, 7);
        assert_eq!(review.reporter, "manager");
        assert_eq!(review.revision, 3);
        assert_eq!(
            review.claim("reviewer", true, 13),
            Ok(Claim::AlreadyHeld(review.clone()))
        );
        assert_eq!(
            review.claim("coder", true, 13),
            Err(Error::Held {
                holder: "reviewer".into()
            })
        );
    }

    #[test]
    fn identical_update_does_not_advance_revision() {
        let task = Task::new(1, "manager", draft(), 5).unwrap();
        assert_eq!(
            task.apply("manager", 1, &TaskPatch::default(), false, None, 6),
            Err(Error::NoChange)
        );
        assert_eq!(task.revision, 1);
        assert_eq!(task.updated_at_ms, 5);
    }

    #[test]
    fn explicit_review_claim_assigns_without_changing_status() {
        let mut input = draft();
        input.status = Status::Review;
        let task = Task::new(4, "manager", input, 10).unwrap();
        assert_eq!(
            task.claim("reviewer", false, 11),
            Err(Error::NotClaimable(Status::Review))
        );
        let Claim::ReviewTaken(taken) = task.claim("reviewer", true, 11).unwrap() else {
            panic!("explicit review claim must take work");
        };
        assert_eq!(taken.status, Status::Review);
        assert_eq!(taken.assignee.as_deref(), Some("reviewer"));
        assert_eq!(taken.revision, 2);
        assert_eq!(task.revision, 1);
    }

    #[test]
    fn assigned_todo_refuses_other_actor() {
        let mut input = draft();
        input.assignee = Some("alice".into());
        let task = Task::new(1, "manager", input, 0).unwrap();
        assert_eq!(
            task.claim("bob", true, 1),
            Err(Error::Held {
                holder: "alice".into()
            })
        );
    }

    #[test]
    fn blocked_owner_is_protected_and_force_reason_survives() {
        let mut input = draft();
        input.status = Status::Blocked;
        assert_eq!(
            Task::new(1, "manager", input.clone(), 0),
            Err(Error::MissingAssignee(Status::Blocked))
        );
        input.assignee = Some("alice".into());
        let task = Task::new(1, "manager", input, 0).unwrap();
        let patch = TaskPatch {
            assignee: Some(Some("bob".into())),
            ..TaskPatch::default()
        };
        assert_eq!(
            task.apply("manager", 1, &patch, false, None, 1),
            Err(Error::Held {
                holder: "alice".into()
            })
        );
        let applied = task
            .apply("manager", 1, &patch, true, Some("quota exhausted"), 1)
            .unwrap();
        assert_eq!(applied.force_reason.as_deref(), Some("quota exhausted"));
        assert_eq!(applied.task.status, Status::Blocked);
        assert_eq!(applied.task.assignee.as_deref(), Some("bob"));
    }

    #[test]
    fn forced_foreign_title_edit_keeps_override_reason_without_reassignment() {
        let mut input = draft();
        input.status = Status::InProgress;
        input.assignee = Some("alice".into());
        let task = Task::new(1, "manager", input, 0).unwrap();
        let patch = TaskPatch {
            title: Some("Clarify acceptance".into()),
            ..TaskPatch::default()
        };
        let applied = task
            .apply("manager", 1, &patch, true, Some("manager correction"), 1)
            .unwrap();
        assert_eq!(applied.force_reason.as_deref(), Some("manager correction"));
        assert_eq!(applied.task.assignee, task.assignee);
        assert_eq!(applied.task.revision, 2);
    }

    #[test]
    fn force_reason_on_own_task_is_audited() {
        let mut input = draft();
        input.status = Status::InProgress;
        input.assignee = Some("alice".into());
        let task = Task::new(1, "manager", input, 0).unwrap();
        let patch = TaskPatch {
            title: Some("Clarify acceptance".into()),
            ..TaskPatch::default()
        };
        assert_eq!(
            task.apply("alice", 1, &patch, true, None, 1),
            Err(Error::MissingForceReason)
        );
        let applied = task
            .apply("alice", 1, &patch, true, Some("clarification"), 1)
            .unwrap();
        assert_eq!(applied.force_reason.as_deref(), Some("clarification"));
        assert_eq!(applied.task.assignee.as_deref(), Some("alice"));
    }

    #[test]
    fn status_names_parse_through_one_shared_rule() {
        for status in [
            Status::Todo,
            Status::InProgress,
            Status::Blocked,
            Status::Review,
            Status::Done,
            Status::Canceled,
        ] {
            assert_eq!(Status::parse(status.as_str()), Ok(status));
        }
        let error = Status::parse("queued").unwrap_err();
        assert_eq!(error, Error::UnknownStatus("queued".into()));
        assert_eq!(error.to_string(), "unknown task status: \"queued\"");
    }

    #[test]
    fn self_parent_and_unbounded_display_fields_refuse() {
        let mut input = draft();
        input.parent_id = Some(1);
        assert_eq!(Task::new(1, "manager", input, 0), Err(Error::SelfParent));
        assert_eq!(
            Task::new(1, "alice\nT99 done", draft(), 0),
            Err(Error::InvalidActor)
        );
        assert_eq!(
            Task::new(1, "a".repeat(MAX_ACTOR_CHARS + 1).as_str(), draft(), 0),
            Err(Error::InvalidActor)
        );
        let mut input = draft();
        input.title = "one\ntwo".into();
        assert_eq!(Task::new(1, "manager", input, 0), Err(Error::InvalidTitle));
        let mut input = draft();
        input.title = "x".repeat(MAX_TITLE_CHARS + 1);
        assert_eq!(Task::new(1, "manager", input, 0), Err(Error::InvalidTitle));
    }

    #[test]
    fn display_limits_and_priority_extremes_are_accepted() {
        let mut input = draft();
        input.title = "界".repeat(MAX_TITLE_CHARS);
        input.priority = i32::MIN;
        let task = Task::new(1, &"界".repeat(MAX_ACTOR_CHARS), input, 0).unwrap();
        assert_eq!(task.title.chars().count(), MAX_TITLE_CHARS);
        assert_eq!(task.reporter.chars().count(), MAX_ACTOR_CHARS);
        assert_eq!(task.priority, i32::MIN);
        let mut input = draft();
        input.priority = i32::MAX;
        assert_eq!(
            Task::new(2, "manager", input, 0).unwrap().priority,
            i32::MAX
        );
    }

    #[test]
    fn completed_tasks_cannot_be_claimed() {
        for status in [Status::Done, Status::Canceled] {
            let mut input = draft();
            input.status = status;
            let task = Task::new(1, "manager", input, 0).unwrap();
            assert_eq!(
                task.claim("alice", true, 1),
                Err(Error::NotClaimable(status))
            );
        }
    }

    #[test]
    fn revision_overflow_refuses_without_mutation() {
        let mut task = Task::new(1, "manager", draft(), 0).unwrap();
        task.revision = u64::MAX;
        let patch = TaskPatch {
            title: Some("changed".into()),
            ..TaskPatch::default()
        };
        assert_eq!(
            task.apply("manager", u64::MAX, &patch, false, None, 1),
            Err(Error::RevisionOverflow)
        );
        assert_eq!(task.title, "Fix claim race");
    }

    #[test]
    fn persisted_types_round_trip() {
        let task = Task::new(1, "manager", draft(), 4).unwrap();
        let task_bytes = serde_json::to_vec(&task).unwrap();
        assert_eq!(serde_json::from_slice::<Task>(&task_bytes).unwrap(), task);
        let event = Event {
            seq: 1,
            task_id: 1,
            actor: "manager".into(),
            kind: EventKind::Created,
            timestamp_ms: 4,
            recipient: None,
            payload: serde_json::json!({"task": task}),
        };
        let event_bytes = serde_json::to_vec(&event).unwrap();
        assert_eq!(
            serde_json::from_slice::<Event>(&event_bytes).unwrap(),
            event
        );
        assert_eq!(
            serde_json::to_string(&Status::InProgress).unwrap(),
            "\"in_progress\""
        );
    }
}
