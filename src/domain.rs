//! Board records and rules that do not depend on LMDB or an adapter.
//!
//! Store operations will enforce these rules inside a write transaction and
//! append events in that same transaction. This module never writes a board.

use serde::{Deserialize, Serialize};

/// Milliseconds since the Unix epoch, the board's only clock.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

pub const MAX_ACTOR_CHARS: usize = 64;
pub const MAX_TITLE_CHARS: usize = 200;
pub const MAX_TEXT_CHARS: usize = 32_768;
pub const MAX_REPORT_CHARS: usize = 262_144;

pub fn check_len(field: &'static str, text: &str, limit: usize) -> Result<(), Error> {
    let size = text.chars().count();
    if size > limit {
        Err(Error::TooLong { field, size, limit })
    } else {
        Ok(())
    }
}

/// Joins two report sections. The seat writes its own heading; the board only
/// separates sections, so a report stays readable without relying on blank lines.
pub const REPORT_SEPARATOR: &str = "\n\n---\n\n";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub project_id: u64,
    pub project_name: String,
    pub project_path: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewProject {
    pub project_name: String,
    pub project_path: String,
}

pub fn validate_project(name: &str, path: &str) -> Result<(), Error> {
    if name.trim().is_empty() || name.trim() != name || name.chars().any(char::is_control) {
        return Err(Error::InvalidProjectName);
    }
    if path.trim().is_empty() || path.trim() != path || path.chars().any(char::is_control) {
        return Err(Error::InvalidProjectPath);
    }
    Ok(())
}

/// The stored names `in_progress` and `review` are board format and stay frozen;
/// the variants are what clients see as `implementing` and `reviewing`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Todo,
    Planning,
    #[serde(rename = "in_progress")]
    Implementing,
    Blocked,
    #[serde(rename = "review")]
    Reviewing,
    Done,
    Canceled,
}

impl Status {
    pub const ALL: [Status; 7] = [
        Status::Todo,
        Status::Planning,
        Status::Implementing,
        Status::Blocked,
        Status::Reviewing,
        Status::Done,
        Status::Canceled,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::Planning => "planning",
            Self::Implementing => "implementing",
            Self::Blocked => "blocked",
            Self::Reviewing => "reviewing",
            Self::Done => "done",
            Self::Canceled => "canceled",
        }
    }

    /// The seven names, then the legacy inputs `in_progress` and `review`, which
    /// are accepted so old scripts and live seats keep working and never printed.
    pub fn parse(value: &str) -> Result<Self, Error> {
        match value {
            "todo" => Ok(Self::Todo),
            "planning" => Ok(Self::Planning),
            "implementing" | "in_progress" => Ok(Self::Implementing),
            "blocked" => Ok(Self::Blocked),
            "reviewing" | "review" => Ok(Self::Reviewing),
            "done" => Ok(Self::Done),
            "canceled" => Ok(Self::Canceled),
            _ => Err(Error::UnknownStatus(value.to_owned())),
        }
    }

    /// Board order, the order counts and wait verdicts print in.
    pub fn position(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    pub const fn requires_assignee(self) -> bool {
        matches!(self, Self::Planning | Self::Implementing | Self::Blocked)
    }

    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Canceled)
    }

    pub const fn protects_assignee(self) -> bool {
        matches!(
            self,
            Self::Planning | Self::Implementing | Self::Blocked | Self::Reviewing
        )
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
    TooLong {
        field: &'static str,
        size: usize,
        limit: usize,
    },
    UnknownStatus(String),
    EmptyReport,
    SelfParent,
    MissingAssignee(Status),
    UnknownProject(u64),
    MissingForceReason,
    ForceReasonWithoutForce,
    Held {
        holder: String,
    },
    StaleRevision {
        expected: u64,
        actual: u64,
    },
    NotClaimable(Status),
    ClosedAssignee(Status),
    UnknownEventKind(String),
    NoChange,
    RevisionOverflow,
    InvalidProjectName,
    InvalidProjectPath,
    EmptyProjectPatch,
    SetAndClear(&'static str),
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
            Self::TooLong { field, size, limit } => {
                write!(f, "{field} is {size} characters; the limit is {limit}")
            }
            Self::UnknownStatus(value) => write!(f, "unknown task status: {value:?}"),
            Self::EmptyReport => f.write_str("report body must be nonempty when appending"),
            Self::UnknownEventKind(value) => write!(f, "unknown event kind: {value:?}"),
            Self::SelfParent => f.write_str("task cannot be its own parent"),
            Self::MissingAssignee(status) => {
                write!(f, "{} requires an assignee", status.as_str())
            }
            Self::UnknownProject(id) => write!(f, "unknown project {id}"),
            Self::MissingForceReason => f.write_str("forced update requires a reason"),
            Self::ForceReasonWithoutForce => f.write_str("force reason requires force=true"),
            Self::Held { holder } => write!(
                f,
                "task is held by {holder}; force with a reason takes it, or release with force and a reason returns it to todo"
            ),
            Self::StaleRevision { expected, actual } => {
                write!(
                    f,
                    "stale task revision {expected}; current revision is {actual}"
                )
            }
            Self::NotClaimable(status) => write!(f, "{} cannot be claimed", status.as_str()),
            Self::ClosedAssignee(status) => write!(
                f,
                "{} task cannot change assignee; reopen it with an open status in the same update",
                status.as_str()
            ),
            Self::NoChange => f.write_str("task update has no field changes"),
            Self::RevisionOverflow => f.write_str("task revision overflow"),
            Self::InvalidProjectName => f.write_str(
                "project name must be nonempty, trimmed, and contain no control characters",
            ),
            Self::InvalidProjectPath => f.write_str(
                "project path must be nonempty, trimmed, and contain no control characters",
            ),
            Self::EmptyProjectPatch => f.write_str("project update has no field changes"),
            Self::SetAndClear(field) => write!(f, "choose {field} or clear_{field}, not both"),
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
    /// Per-seat long-form notes. Old boards have no such field, so each defaults
    /// to empty rather than refusing to load.
    #[serde(default)]
    pub analyst_report: String,
    #[serde(default)]
    pub coder_report: String,
    #[serde(default)]
    pub reviewer_report: String,
    pub status: Status,
    pub priority: i32,
    pub reporter: String,
    pub assignee: Option<String>,
    pub parent_id: Option<u64>,
    pub project_id: Option<u64>,
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
    pub project_id: Option<u64>,
}

impl Task {
    pub fn new(id: u64, actor: &str, input: NewTask, now_ms: i64) -> Result<Self, Error> {
        validate_actor(actor)?;
        check_len("description", &input.description, MAX_TEXT_CHARS)?;
        check_len("goal", &input.goal, MAX_TEXT_CHARS)?;
        let task = Self {
            id,
            title: input.title,
            description: input.description,
            goal: input.goal,
            analyst_report: String::new(),
            coder_report: String::new(),
            reviewer_report: String::new(),
            status: input.status,
            priority: input.priority,
            reporter: actor.to_owned(),
            assignee: input.assignee,
            parent_id: input.parent_id,
            project_id: input.project_id,
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
                    status: Some(Status::Implementing),
                    assignee: Some(Some(actor.to_owned())),
                    ..TaskPatch::default()
                };
                Ok(Claim::Started(
                    self.apply(actor, self.revision, &patch, false, None, now_ms)?
                        .task,
                ))
            }
            Status::Reviewing if explicit && self.assignee.is_none() => {
                let patch = TaskPatch {
                    assignee: Some(Some(actor.to_owned())),
                    ..TaskPatch::default()
                };
                Ok(Claim::ReviewTaken(
                    self.apply(actor, self.revision, &patch, false, None, now_ms)?
                        .task,
                ))
            }
            Status::Reviewing | Status::Planning | Status::Implementing
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
        if force && force_reason.is_none_or(|reason| reason.trim().is_empty()) {
            return Err(Error::MissingForceReason);
        }
        if !force && force_reason.is_some() {
            return Err(Error::ForceReasonWithoutForce);
        }
        if let Some(reason) = force_reason {
            check_len("reason", reason, MAX_TEXT_CHARS)?;
        }
        if let Some(description) = &patch.description {
            check_len("description", description, MAX_TEXT_CHARS)?;
        }
        if let Some(goal) = &patch.goal {
            check_len("goal", goal, MAX_TEXT_CHARS)?;
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
        if let Some(project_id) = patch.project_id {
            next.project_id = project_id;
        }
        let mut report_mode = None;
        for (field, report, body) in [
            (
                "analyst_report",
                &mut next.analyst_report,
                patch.analyst_report.as_deref(),
            ),
            (
                "coder_report",
                &mut next.coder_report,
                patch.coder_report.as_deref(),
            ),
            (
                "reviewer_report",
                &mut next.reviewer_report,
                patch.reviewer_report.as_deref(),
            ),
        ] {
            let Some(body) = body else { continue };
            report_mode = Some(if patch.replace_reports {
                "replace"
            } else {
                "append"
            });
            if patch.replace_reports {
                report.clear();
                report.push_str(body);
            } else if body.trim().is_empty() {
                return Err(Error::EmptyReport);
            } else if report.is_empty() {
                report.push_str(body);
            } else {
                report.push_str(REPORT_SEPARATOR);
                report.push_str(body);
            }
            check_len(field, report, MAX_REPORT_CHARS)?;
        }
        next.validate()?;
        if next.status.is_terminal() && next.assignee != self.assignee {
            return Err(Error::ClosedAssignee(next.status));
        }
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
            report_mode,
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
    /// `append` or `replace` when this patch wrote a report, else `None`.
    pub report_mode: Option<&'static str>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub goal: Option<String>,
    pub analyst_report: Option<String>,
    pub coder_report: Option<String>,
    pub reviewer_report: Option<String>,
    /// Rewrite the named reports instead of appending a section.
    pub replace_reports: bool,
    pub status: Option<Status>,
    pub priority: Option<i32>,
    /// `None` leaves the field alone, `Some(None)` clears it.
    pub assignee: Option<Option<String>>,
    pub parent_id: Option<Option<u64>>,
    pub project_id: Option<Option<u64>>,
}

impl TaskPatch {
    pub fn is_empty(&self) -> bool {
        *self
            == Self {
                replace_reports: self.replace_reports,
                ..Self::default()
            }
    }
}

pub fn set_or_clear<T>(
    field: &'static str,
    set: Option<T>,
    clear: bool,
) -> Result<Option<Option<T>>, Error> {
    match (set, clear) {
        (Some(_), true) => Err(Error::SetAndClear(field)),
        (_, true) => Ok(Some(None)),
        (Some(value), false) => Ok(Some(Some(value))),
        (None, false) => Ok(None),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Created,
    Updated,
    Claimed,
    Commented,
}

impl EventKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
            Self::Claimed => "claimed",
            Self::Commented => "commented",
        }
    }

    pub const ALL: [EventKind; 4] = [
        EventKind::Created,
        EventKind::Updated,
        EventKind::Claimed,
        EventKind::Commented,
    ];

    pub fn parse(raw: &str) -> Result<Self, Error> {
        match raw {
            "created" => Ok(Self::Created),
            "updated" => Ok(Self::Updated),
            "claimed" => Ok(Self::Claimed),
            "commented" => Ok(Self::Commented),
            other => Err(Error::UnknownEventKind(other.to_owned())),
        }
    }
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

impl Event {
    pub fn kind_str(&self) -> &'static str {
        self.kind.as_str()
    }
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
            project_id: None,
        }
    }

    #[test]
    fn patch_emptiness_and_set_clear_rules_are_shared() {
        assert!(TaskPatch::default().is_empty());
        assert!(
            TaskPatch {
                replace_reports: true,
                ..TaskPatch::default()
            }
            .is_empty()
        );
        let one_field = vec![
            TaskPatch {
                title: Some("t".into()),
                ..TaskPatch::default()
            },
            TaskPatch {
                description: Some("d".into()),
                ..TaskPatch::default()
            },
            TaskPatch {
                goal: Some("g".into()),
                ..TaskPatch::default()
            },
            TaskPatch {
                analyst_report: Some("a".into()),
                ..TaskPatch::default()
            },
            TaskPatch {
                coder_report: Some("c".into()),
                ..TaskPatch::default()
            },
            TaskPatch {
                reviewer_report: Some("r".into()),
                ..TaskPatch::default()
            },
            TaskPatch {
                status: Some(Status::Done),
                ..TaskPatch::default()
            },
            TaskPatch {
                priority: Some(1),
                ..TaskPatch::default()
            },
            TaskPatch {
                assignee: Some(None),
                ..TaskPatch::default()
            },
            TaskPatch {
                parent_id: Some(None),
                ..TaskPatch::default()
            },
            TaskPatch {
                project_id: Some(None),
                ..TaskPatch::default()
            },
        ];
        assert!(one_field.iter().all(|patch| !patch.is_empty()));
        assert_eq!(
            set_or_clear("assignee", Some("a"), false),
            Ok(Some(Some("a")))
        );
        assert_eq!(set_or_clear::<&str>("assignee", None, true), Ok(Some(None)));
        assert_eq!(set_or_clear::<&str>("assignee", None, false), Ok(None));
        assert_eq!(
            set_or_clear("assignee", Some("a"), true),
            Err(Error::SetAndClear("assignee"))
        );
        assert_eq!(
            Error::SetAndClear("assignee").to_string(),
            "choose assignee or clear_assignee, not both"
        );
    }

    #[test]
    fn creation_checks_actor_title_and_active_owner() {
        assert_eq!(Task::new(1, "  ", draft(), 0), Err(Error::EmptyActor));
        let mut input = draft();
        input.title = " \n ".into();
        assert_eq!(Task::new(1, "alice", input, 0), Err(Error::EmptyTitle));
        let mut input = draft();
        input.status = Status::Implementing;
        assert_eq!(
            Task::new(1, "alice", input, 0),
            Err(Error::MissingAssignee(Status::Implementing))
        );
        let mut input = draft();
        input.assignee = Some("  ".into());
        assert_eq!(Task::new(1, "alice", input, 0), Err(Error::EmptyAssignee));
        let task = Task::new(1, "alice", draft(), 12).unwrap();
        assert_eq!(task.revision, 1);
        assert_eq!(task.reporter, "alice");
    }

    #[test]
    fn closed_rows_refuse_owner_changes() {
        let patch = |status: Option<Status>, assignee: Option<Option<&str>>| TaskPatch {
            status,
            assignee: assignee.map(|a| a.map(str::to_owned)),
            ..TaskPatch::default()
        };
        for closed in [Status::Done, Status::Canceled] {
            let mut input = draft();
            input.status = closed;
            input.assignee = Some("alice".into());
            let task = Task::new(1, "manager", input, 0).unwrap();
            for (force, reason) in [(false, None), (true, Some("quota"))] {
                for change in [Some("bob"), None] {
                    assert_eq!(
                        task.apply("manager", 1, &patch(None, Some(change)), force, reason, 1),
                        Err(Error::ClosedAssignee(closed))
                    );
                }
            }
            for open in [Status::Todo, Status::Implementing] {
                let reopened = task
                    .apply(
                        "manager",
                        1,
                        &patch(Some(open), Some(Some("bob"))),
                        false,
                        None,
                        1,
                    )
                    .unwrap()
                    .task;
                assert_eq!(reopened.status, open);
                assert_eq!(reopened.assignee.as_deref(), Some("bob"));
                assert_eq!(reopened.revision, 2);
            }
            let edit = TaskPatch {
                title: Some("renamed".into()),
                ..TaskPatch::default()
            };
            assert!(task.apply("manager", 1, &edit, false, None, 1).is_ok());
        }
        let mut input = draft();
        input.status = Status::Implementing;
        input.assignee = Some("alice".into());
        let task = Task::new(1, "manager", input, 0).unwrap();
        assert_eq!(
            task.apply(
                "alice",
                1,
                &patch(Some(Status::Done), Some(Some("bob"))),
                false,
                None,
                1
            ),
            Err(Error::ClosedAssignee(Status::Done))
        );
        assert!(
            task.apply("alice", 1, &patch(Some(Status::Done), None), false, None, 1)
                .is_ok()
        );
    }

    #[test]
    fn claim_does_not_take_other_actors_work() {
        let task = Task::new(1, "manager", draft(), 0).unwrap();
        let Claim::Started(started) = task.claim("alice", false, 1).unwrap() else {
            panic!("todo claim must start work");
        };
        assert_eq!(started.status, Status::Implementing);
        assert_eq!(started.assignee.as_deref(), Some("alice"));
        assert_eq!(started.revision, 2);
        assert_eq!(started.updated_at_ms, 1);
        let own = task
            .apply(
                "manager",
                1,
                &TaskPatch {
                    assignee: Some(Some("alice".into())),
                    status: Some(Status::Implementing),
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
            Err(Error::NotClaimable(Status::Implementing))
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
                    status: Some(Status::Implementing),
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
                    status: Some(Status::Implementing),
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
            Err(Error::MissingAssignee(Status::Implementing))
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
                    status: Some(Status::Implementing),
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
                    status: Some(Status::Reviewing),
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
        input.status = Status::Reviewing;
        let task = Task::new(4, "manager", input, 10).unwrap();
        assert_eq!(
            task.claim("reviewer", false, 11),
            Err(Error::NotClaimable(Status::Reviewing))
        );
        let Claim::ReviewTaken(taken) = task.claim("reviewer", true, 11).unwrap() else {
            panic!("explicit review claim must take work");
        };
        assert_eq!(taken.status, Status::Reviewing);
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
        input.status = Status::Implementing;
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
        input.status = Status::Implementing;
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
        for status in Status::ALL {
            assert_eq!(Status::parse(status.as_str()), Ok(status));
        }
        assert_eq!(Status::parse("in_progress"), Ok(Status::Implementing));
        assert_eq!(Status::parse("review"), Ok(Status::Reviewing));
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
    fn text_limits_apply_to_new_and_changed_values_only() {
        let at_limit = "界".repeat(MAX_TEXT_CHARS);
        let mut input = draft();
        input.description.clone_from(&at_limit);
        input.goal.clone_from(&at_limit);
        let task = Task::new(1, "manager", input, 0).unwrap();
        for (field, value) in [
            ("description", at_limit.as_str()),
            ("goal", at_limit.as_str()),
        ] {
            assert_eq!(value.chars().count(), MAX_TEXT_CHARS);
            let patch = if field == "description" {
                TaskPatch {
                    description: Some(format!("{value}界")),
                    ..TaskPatch::default()
                }
            } else {
                TaskPatch {
                    goal: Some(format!("{value}界")),
                    ..TaskPatch::default()
                }
            };
            assert_eq!(
                task.apply("manager", 1, &patch, false, None, 1),
                Err(Error::TooLong {
                    field,
                    size: MAX_TEXT_CHARS + 1,
                    limit: MAX_TEXT_CHARS
                })
            );
        }
        let mut oversized = draft();
        oversized.description = "x".repeat(MAX_TEXT_CHARS + 1);
        assert_eq!(
            Task::new(1, "manager", oversized, 0),
            Err(Error::TooLong {
                field: "description",
                size: MAX_TEXT_CHARS + 1,
                limit: MAX_TEXT_CHARS
            })
        );
        let mut oversized = draft();
        oversized.goal = "x".repeat(MAX_TEXT_CHARS + 1);
        assert_eq!(
            Task::new(1, "manager", oversized, 0),
            Err(Error::TooLong {
                field: "goal",
                size: MAX_TEXT_CHARS + 1,
                limit: MAX_TEXT_CHARS
            })
        );
        let reason = "r".repeat(MAX_TEXT_CHARS + 1);
        assert_eq!(
            task.apply(
                "manager",
                1,
                &TaskPatch {
                    title: Some("renamed".into()),
                    ..TaskPatch::default()
                },
                true,
                Some(&reason),
                1
            ),
            Err(Error::TooLong {
                field: "reason",
                size: MAX_TEXT_CHARS + 1,
                limit: MAX_TEXT_CHARS
            })
        );
    }

    #[test]
    fn report_limit_bounds_resulting_total_for_append_and_replace() {
        let task = Task::new(1, "manager", draft(), 0).unwrap();
        let body = "x".repeat(MAX_REPORT_CHARS - REPORT_SEPARATOR.chars().count() - 1);
        let seeded = task
            .apply(
                "manager",
                1,
                &TaskPatch {
                    coder_report: Some(body),
                    ..TaskPatch::default()
                },
                false,
                None,
                1,
            )
            .unwrap()
            .task;
        let final_body = "y".to_owned();
        let accepted = seeded
            .apply(
                "manager",
                2,
                &TaskPatch {
                    coder_report: Some(final_body.clone()),
                    ..TaskPatch::default()
                },
                false,
                None,
                2,
            )
            .unwrap()
            .task;
        assert_eq!(accepted.coder_report.chars().count(), MAX_REPORT_CHARS);
        assert_eq!(
            accepted.apply(
                "manager",
                3,
                &TaskPatch {
                    coder_report: Some("z".into()),
                    ..TaskPatch::default()
                },
                false,
                None,
                3
            ),
            Err(Error::TooLong {
                field: "coder_report",
                size: MAX_REPORT_CHARS + REPORT_SEPARATOR.chars().count() + 1,
                limit: MAX_REPORT_CHARS
            })
        );
        assert_eq!(
            task.apply(
                "manager",
                1,
                &TaskPatch {
                    coder_report: Some("x".repeat(MAX_REPORT_CHARS + 1)),
                    replace_reports: true,
                    ..TaskPatch::default()
                },
                false,
                None,
                1
            ),
            Err(Error::TooLong {
                field: "coder_report",
                size: MAX_REPORT_CHARS + 1,
                limit: MAX_REPORT_CHARS
            })
        );
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
            serde_json::to_string(&Status::Implementing).unwrap(),
            "\"in_progress\""
        );
    }

    #[test]
    fn stored_status_names_stay_frozen_and_planning_is_held_work() {
        for (status, stored) in [
            (Status::Planning, "\"planning\""),
            (Status::Implementing, "\"in_progress\""),
            (Status::Reviewing, "\"review\""),
        ] {
            assert_eq!(serde_json::to_string(&status).unwrap(), stored);
            assert_eq!(serde_json::from_str::<Status>(stored).unwrap(), status);
        }
        assert!(Status::Planning.requires_assignee());
        assert!(Status::Planning.protects_assignee());
        let mut input = draft();
        input.status = Status::Planning;
        assert_eq!(
            Task::new(1, "manager", input.clone(), 0),
            Err(Error::MissingAssignee(Status::Planning))
        );
        input.assignee = Some("analyst".into());
        let task = Task::new(1, "manager", input, 0).unwrap();
        assert_eq!(
            task.claim("analyst", false, 1),
            Err(Error::NotClaimable(Status::Planning))
        );
        assert_eq!(
            task.claim("analyst", true, 1),
            Ok(Claim::AlreadyHeld(task.clone()))
        );
    }

    #[test]
    fn reports_append_by_default_and_replace_on_request() {
        let task = Task::new(1, "manager", draft(), 0).unwrap();
        let append = |body: &str| TaskPatch {
            coder_report: Some(body.into()),
            ..TaskPatch::default()
        };
        let first = task
            .apply(
                "coder",
                1,
                &append("## Coder report 01\nbuilt"),
                false,
                None,
                1,
            )
            .unwrap();
        assert_eq!(first.report_mode, Some("append"));
        assert_eq!(first.task.coder_report, "## Coder report 01\nbuilt");

        let second = first
            .task
            .apply(
                "coder",
                2,
                &append("## Coder report 02\nfixed"),
                false,
                None,
                2,
            )
            .unwrap();
        assert_eq!(
            second.task.coder_report,
            "## Coder report 01\nbuilt\n\n---\n\n## Coder report 02\nfixed"
        );
        assert_eq!(second.report_mode, Some("append"));

        let replace = TaskPatch {
            coder_report: Some("## Coder report 02\nrewritten".into()),
            replace_reports: true,
            ..TaskPatch::default()
        };
        let replaced = second
            .task
            .apply("coder", 3, &replace, false, None, 3)
            .unwrap();
        assert_eq!(replaced.report_mode, Some("replace"));
        assert_eq!(replaced.task.coder_report, "## Coder report 02\nrewritten");

        let clear = TaskPatch {
            coder_report: Some(String::new()),
            replace_reports: true,
            ..TaskPatch::default()
        };
        let cleared = replaced
            .task
            .apply("coder", 4, &clear, false, None, 4)
            .unwrap();
        assert_eq!(cleared.task.coder_report, "");
    }

    #[test]
    fn empty_report_append_refuses_and_old_json_still_loads() {
        let task = Task::new(1, "manager", draft(), 0).unwrap();
        let empty = TaskPatch {
            analyst_report: Some("  \n".into()),
            ..TaskPatch::default()
        };
        assert_eq!(
            task.apply("analyst", 1, &empty, false, None, 1),
            Err(Error::EmptyReport)
        );
        let mut value = serde_json::to_value(&task).unwrap();
        let fields = value.as_object_mut().unwrap();
        for name in ["analyst_report", "coder_report", "reviewer_report"] {
            fields.remove(name);
        }
        let loaded: Task = serde_json::from_value(value).unwrap();
        assert_eq!(loaded.analyst_report, "");
        assert_eq!(loaded.coder_report, "");
        assert_eq!(loaded.reviewer_report, "");
    }
}
