//! M3 proofs: claims, comments, lists, brief, feed paging and the bounded wait.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::domain::{Error as DomainError, EventKind, NewTask, Status, TaskPatch};

use super::{
    Board, BoardOptions, ClaimOutcome, Error, FeedFilter, ListFilter, UpdateInput, WAIT_MAX_MS,
};

static NEXT_SCRATCH: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "agentboard-lmdb-m3-{}-{nanos}-{}",
            std::process::id(),
            NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn options() -> BoardOptions {
    BoardOptions { map_size_mb: 8 }
}

fn draft(title: &str, status: Status, priority: i32, assignee: Option<&str>) -> NewTask {
    NewTask {
        title: title.into(),
        description: String::new(),
        goal: String::new(),
        status,
        priority,
        assignee: assignee.map(str::to_owned),
        parent_id: None,
        path: None,
    }
}

fn status_patch(status: Status) -> TaskPatch {
    TaskPatch {
        status: Some(status),
        ..TaskPatch::default()
    }
}

fn list_all() -> ListFilter<'static> {
    ListFilter {
        status: None,
        assignee: None,
        parent: None,
        path: None,
        include_terminal: true,
        limit: 50,
        offset: 0,
    }
}

#[test]
fn claim_walks_the_index_and_skips_foreign_rows() {
    let scratch = Scratch::new();
    let board = Board::open_with_options(scratch.path(), options()).unwrap();
    let held = board
        .add(
            "manager",
            draft("held by alice", Status::Todo, -10, Some("alice")),
            true,
            0,
        )
        .unwrap()
        .task;
    let free = board
        .add("manager", draft("free", Status::Todo, 0, None), false, 1)
        .unwrap()
        .task;
    let claimed = board.claim("bob", None, 2).unwrap();
    let ClaimOutcome::Claimed(updated) = claimed else {
        panic!("expected a claim, got {claimed:?}");
    };
    assert_eq!(updated.task.id, free.id);
    assert_eq!(updated.task.status, Status::InProgress);
    assert_eq!(updated.task.assignee.as_deref(), Some("bob"));
    assert_eq!(updated.task.revision, 2);
    assert_eq!(updated.event.kind, EventKind::Claimed);
    assert_eq!(updated.event.payload["old_status"], "todo");
    assert_eq!(updated.event.payload["new_status"], "in_progress");
    assert_eq!(board.claim("bob", None, 3).unwrap(), ClaimOutcome::NoWork);
    let alice = board.claim("alice", None, 4).unwrap();
    let ClaimOutcome::Claimed(updated) = alice else {
        panic!("alice must still be able to claim her own row, got {alice:?}");
    };
    assert_eq!(updated.task.id, held.id);
    let mut filter = list_all();
    filter.status = Some(Status::Todo);
    assert!(board.list(&filter).unwrap().tasks.is_empty());
}

#[test]
fn explicit_claim_refuses_held_completed_and_missing_rows() {
    let scratch = Scratch::new();
    let board = Board::open_with_options(scratch.path(), options()).unwrap();
    let task = board
        .add(
            "manager",
            draft("work", Status::Todo, 0, Some("alice")),
            true,
            0,
        )
        .unwrap()
        .task;
    assert!(matches!(
        board.claim("bob", Some(task.id), 1),
        Err(Error::Domain(DomainError::Held { holder })) if holder == "alice"
    ));
    assert!(matches!(
        board.claim("alice", Some(task.id), 1).unwrap(),
        ClaimOutcome::Claimed(_)
    ));
    let again = board.claim("alice", Some(task.id), 2).unwrap();
    let ClaimOutcome::AlreadyHeld(current) = again else {
        panic!("a held claim must be an idempotent read, got {again:?}");
    };
    assert!(matches!(current.status, Status::InProgress));
    let done = board
        .add("manager", draft("done", Status::Done, 0, None), false, 3)
        .unwrap()
        .task;
    assert!(matches!(
        board.claim("alice", Some(done.id), 4),
        Err(Error::Domain(DomainError::NotClaimable(Status::Done)))
    ));
    assert!(matches!(
        board.claim("alice", Some(99), 5),
        Err(Error::MissingTask(99))
    ));
}

#[test]
fn stale_update_and_quota_takeover() {
    let scratch = Scratch::new();
    let board = Board::open_with_options(scratch.path(), options()).unwrap();
    let task = board
        .add(
            "manager",
            draft("work", Status::Todo, 0, Some("alice")),
            true,
            0,
        )
        .unwrap()
        .task;
    let ClaimOutcome::Claimed(claimed) = board.claim("alice", Some(task.id), 1).unwrap() else {
        panic!("alice must claim her row");
    };
    assert_eq!(claimed.task.revision, 2);
    let review = status_patch(Status::Review);
    assert!(matches!(
        board.update(UpdateInput {
            id: task.id,
            actor: "alice",
            expected_revision: 1,
            patch: &review,
            force: false,
            force_reason: None,
            new_actor: false,
            comment: None,
            now_ms: 2,
        }),
        Err(Error::Domain(DomainError::StaleRevision {
            expected: 1,
            actual: 2
        }))
    ));
    let takeover = TaskPatch {
        assignee: Some(Some("bob".into())),
        ..TaskPatch::default()
    };
    let moved = board
        .update(UpdateInput {
            id: task.id,
            actor: "manager",
            expected_revision: 2,
            patch: &takeover,
            force: true,
            force_reason: Some("quota"),
            new_actor: true,
            comment: Some("quota handover; patch ref abc123"),
            now_ms: 3,
        })
        .unwrap();
    assert_eq!(moved.task.revision, 3);
    assert_eq!(moved.task.assignee.as_deref(), Some("bob"));
    assert_eq!(moved.event.kind, EventKind::Updated);
    assert_eq!(moved.event.recipient.as_deref(), Some("bob"));
    assert_eq!(moved.event.payload["reason"], "quota");
    assert_eq!(
        moved.event.payload["comment"],
        "quota handover; patch ref abc123"
    );
    assert_eq!(moved.event.payload["changes"]["assignee"]["new"], "bob");
    let title = TaskPatch {
        title: Some("half done".into()),
        ..TaskPatch::default()
    };
    assert!(matches!(
        board.update(UpdateInput {
            id: task.id,
            actor: "alice",
            expected_revision: 2,
            patch: &title,
            force: false,
            force_reason: None,
            new_actor: false,
            comment: None,
            now_ms: 4,
        }),
        Err(Error::Domain(DomainError::StaleRevision {
            expected: 2,
            actual: 3
        }))
    ));
    assert!(matches!(
        board.update(UpdateInput {
            id: task.id,
            actor: "alice",
            expected_revision: 3,
            patch: &title,
            force: false,
            force_reason: None,
            new_actor: false,
            comment: None,
            now_ms: 5,
        }),
        Err(Error::Domain(DomainError::Held { holder })) if holder == "bob"
    ));
}

#[test]
fn forced_own_task_records_the_reason_without_a_handoff() {
    let scratch = Scratch::new();
    let board = Board::open_with_options(scratch.path(), options()).unwrap();
    let task = board
        .add(
            "manager",
            draft("work", Status::InProgress, 0, Some("alice")),
            true,
            0,
        )
        .unwrap()
        .task;
    let title = TaskPatch {
        title: Some("clarify acceptance".into()),
        ..TaskPatch::default()
    };
    let updated = board
        .update(UpdateInput {
            id: task.id,
            actor: "alice",
            expected_revision: 1,
            patch: &title,
            force: true,
            force_reason: Some("manager correction"),
            new_actor: false,
            comment: None,
            now_ms: 1,
        })
        .unwrap();
    assert_eq!(updated.event.payload["reason"], "manager correction");
    assert_eq!(updated.event.recipient, None);
    assert_eq!(updated.task.assignee.as_deref(), Some("alice"));
}

#[test]
fn comments_do_not_bump_revision_and_appear_in_the_feed() {
    let scratch = Scratch::new();
    let board = Board::open_with_options(scratch.path(), options()).unwrap();
    let task = board
        .add("manager", draft("work", Status::Todo, 0, None), false, 10)
        .unwrap()
        .task;
    let event = board
        .comment("bob", task.id, "seen; starting now", 11)
        .unwrap();
    assert_eq!(event.kind, EventKind::Commented);
    assert_eq!(event.seq, 2);
    assert_eq!(event.payload["text"], "seen; starting now");
    let current = board.get(task.id).unwrap().unwrap();
    assert_eq!(current.revision, 1);
    assert_eq!(current.updated_at_ms, 10);
    assert!(matches!(
        board.comment("bob", task.id, "   ", 12),
        Err(Error::EmptyComment)
    ));
    assert!(matches!(
        board.comment("bob", 99, "orphan", 12),
        Err(Error::MissingTask(99))
    ));
    let page = board
        .feed(&FeedFilter {
            task: Some(task.id),
            kind: Some(EventKind::Commented),
            recipient: None,
            after: None,
            limit: 10,
        })
        .unwrap();
    assert_eq!(page.events.len(), 1);
    assert_eq!(page.events[0].actor, "bob");
}

#[test]
fn list_filters_pages_and_brief_counts() {
    let scratch = Scratch::new();
    let board = Board::open_with_options(scratch.path(), options()).unwrap();
    let low = board
        .add(
            "manager",
            draft("low", Status::Todo, -5, Some("alice")),
            true,
            0,
        )
        .unwrap()
        .task;
    let middle = board
        .add("manager", draft("middle", Status::Todo, 0, None), false, 1)
        .unwrap()
        .task;
    let closed = board
        .add("manager", draft("closed", Status::Done, -9, None), false, 2)
        .unwrap()
        .task;
    let review = board
        .add(
            "manager",
            draft("review", Status::Review, 0, Some("bob")),
            true,
            3,
        )
        .unwrap()
        .task;
    let child = board
        .add("manager", draft("child", Status::Todo, 1, None), false, 4)
        .unwrap()
        .task;
    let parent = TaskPatch {
        parent_id: Some(Some(review.id)),
        ..TaskPatch::default()
    };
    board
        .update(UpdateInput {
            id: child.id,
            actor: "manager",
            expected_revision: 1,
            patch: &parent,
            force: false,
            force_reason: None,
            new_actor: false,
            comment: None,
            now_ms: 5,
        })
        .unwrap();
    let mut filter = list_all();
    filter.include_terminal = false;
    let open = board.list(&filter).unwrap();
    assert_eq!(open.total, 4);
    assert_eq!(
        open.tasks.iter().map(|task| task.id).collect::<Vec<_>>(),
        vec![low.id, middle.id, review.id, child.id]
    );
    filter.include_terminal = true;
    filter.limit = 2;
    filter.offset = 1;
    let page = board.list(&filter).unwrap();
    assert_eq!(page.total, 5);
    assert_eq!(
        page.tasks.iter().map(|task| task.id).collect::<Vec<_>>(),
        vec![low.id, middle.id]
    );
    assert!(page.more);
    filter.offset = 0;
    filter.limit = 50;
    filter.assignee = Some("alice");
    assert_eq!(
        board
            .list(&filter)
            .unwrap()
            .tasks
            .iter()
            .map(|task| task.id)
            .collect::<Vec<_>>(),
        vec![low.id]
    );
    filter.assignee = None;
    filter.parent = Some(review.id);
    assert_eq!(
        board
            .list(&filter)
            .unwrap()
            .tasks
            .iter()
            .map(|task| task.id)
            .collect::<Vec<_>>(),
        vec![child.id]
    );
    filter.parent = None;
    filter.status = Some(Status::Done);
    assert_eq!(
        board
            .list(&filter)
            .unwrap()
            .tasks
            .iter()
            .map(|task| task.id)
            .collect::<Vec<_>>(),
        vec![closed.id]
    );
    let brief = board.brief(10).unwrap();
    let count = |status: Status| {
        brief
            .counts
            .iter()
            .find(|(row, _)| *row == status)
            .map(|(_, count)| *count)
            .unwrap()
    };
    assert_eq!(count(Status::Todo), 3);
    assert_eq!(count(Status::Done), 1);
    assert_eq!(count(Status::Review), 1);
    let active: Vec<u64> = brief.active.iter().map(|task| task.id).collect();
    assert_eq!(active, vec![review.id]);
}

#[test]
fn feed_pages_without_gaps_and_keeps_recipient_history() {
    let scratch = Scratch::new();
    let board = Board::open_with_options(scratch.path(), options()).unwrap();
    let instance = board.metadata().unwrap().instance_id;
    for index in 0..3 {
        board
            .add(
                "manager",
                draft(&format!("task {index}"), Status::Todo, index, None),
                false,
                index as i64,
            )
            .unwrap();
    }
    let task = board.get(2).unwrap().unwrap();
    let handoff = TaskPatch {
        assignee: Some(Some("bob".into())),
        ..TaskPatch::default()
    };
    board
        .update(UpdateInput {
            id: task.id,
            actor: "manager",
            expected_revision: task.revision,
            patch: &handoff,
            force: true,
            force_reason: Some("assign"),
            new_actor: true,
            comment: Some("over to bob"),
            now_ms: 10,
        })
        .unwrap();
    let later = TaskPatch {
        assignee: Some(Some("carol".into())),
        ..TaskPatch::default()
    };
    let current = board.get(task.id).unwrap().unwrap();
    board
        .update(UpdateInput {
            id: task.id,
            actor: "manager",
            expected_revision: current.revision,
            patch: &later,
            force: true,
            force_reason: Some("reassign"),
            new_actor: true,
            comment: None,
            now_ms: 11,
        })
        .unwrap();
    let mut cursor = format!("{instance}:0");
    let mut seen = Vec::new();
    for _ in 0..20 {
        let page = board
            .feed(&FeedFilter {
                task: None,
                kind: None,
                recipient: None,
                after: Some(cursor.as_str()),
                limit: 2,
            })
            .unwrap();
        for event in &page.events {
            seen.push(event.seq);
        }
        cursor = page.next.encode();
        if !page.more {
            break;
        }
    }
    assert_eq!(seen, (1..=seen.len() as u64).collect::<Vec<_>>());
    assert_eq!(seen.len(), 5);
    let bob = board
        .feed(&FeedFilter {
            task: None,
            kind: Some(EventKind::Updated),
            recipient: Some("bob"),
            after: Some(format!("{instance}:0").as_str()),
            limit: 10,
        })
        .unwrap();
    assert_eq!(bob.events.len(), 1);
    assert_eq!(bob.events[0].task_id, task.id);
    assert!(matches!(
        board.feed(&FeedFilter {
            task: None,
            kind: None,
            recipient: None,
            after: Some("other-board:1"),
            limit: 5,
        }),
        Err(Error::CursorInstanceMismatch)
    ));
    assert!(matches!(
        board.feed(&FeedFilter {
            task: None,
            kind: None,
            recipient: None,
            after: Some("garbage"),
            limit: 5,
        }),
        Err(Error::InvalidCursor(_))
    ));
}

#[test]
fn wait_times_out_then_returns_a_late_event_from_another_process() {
    assert_eq!(WAIT_MAX_MS, 300_000);
    let scratch = Scratch::new();
    let board = Board::open_with_options(scratch.path(), options()).unwrap();
    board
        .add("manager", draft("first", Status::Todo, 0, None), false, 0)
        .unwrap();
    let started = Instant::now();
    let empty = board
        .wait(
            &FeedFilter {
                task: None,
                kind: None,
                recipient: None,
                after: None,
                limit: 10,
            },
            60,
        )
        .unwrap();
    assert!(empty.events.is_empty());
    assert!(started.elapsed() >= Duration::from_millis(40));
    let cursor = empty.next.encode();
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "store::m3_tests::late_event_worker",
            "--nocapture",
        ])
        .env("AGENTBOARD_M3_LATE_PATH", scratch.path())
        .env("AGENTBOARD_M3_LATE_DELAY_MS", "80")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let page = board
        .wait(
            &FeedFilter {
                task: None,
                kind: Some(EventKind::Created),
                recipient: None,
                after: Some(cursor.as_str()),
                limit: 10,
            },
            5_000,
        )
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "late worker failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(page.events.len(), 1);
    assert_eq!(page.events[0].payload["task"]["title"], "late");
}

#[test]
fn late_event_worker() {
    let Ok(path) = std::env::var("AGENTBOARD_M3_LATE_PATH") else {
        return;
    };
    let delay: u64 = std::env::var("AGENTBOARD_M3_LATE_DELAY_MS")
        .unwrap()
        .parse()
        .unwrap();
    std::thread::sleep(Duration::from_millis(delay));
    let board = Board::open_with_options(path, options()).unwrap();
    board
        .add("manager", draft("late", Status::Todo, 0, None), false, 1)
        .unwrap();
}

#[test]
fn roster_registers_the_caller_and_needs_opt_in_for_a_target() {
    let scratch = Scratch::new();
    let board = Board::open_with_options(scratch.path(), options()).unwrap();
    assert!(matches!(
        board.add(
            "manager",
            draft("work", Status::Todo, 0, Some("typoed")),
            false,
            0
        ),
        Err(Error::UnknownAssignee(name)) if name == "typoed"
    ));
    assert!(board.agent("typoed").unwrap().is_none());
    assert!(board.agent("manager").unwrap().is_none());
    let task = board
        .add("manager", draft("work", Status::Todo, 0, None), false, 1)
        .unwrap()
        .task;
    let manager = board.agent("manager").unwrap().unwrap();
    assert_eq!(manager.first_seen_ms, 1);
    assert_eq!(manager.last_seen_ms, Some(1));
    assert_eq!(manager.last_task_id, Some(task.id));
    board
        .add(
            "manager",
            draft("for bob", Status::Todo, 0, Some("bob")),
            true,
            2,
        )
        .unwrap();
    let bob = board.agent("bob").unwrap().unwrap();
    assert_eq!(bob.first_seen_ms, 2);
    assert_eq!(bob.last_seen_ms, None);
    board
        .add(
            "bob",
            draft("own work", Status::Todo, 0, Some("bob")),
            false,
            3,
        )
        .unwrap();
    assert_eq!(board.agent("bob").unwrap().unwrap().last_seen_ms, Some(3));
}

#[test]
fn idempotent_claim_writes_nothing() {
    let scratch = Scratch::new();
    let board = Board::open_with_options(scratch.path(), options()).unwrap();
    let task = board
        .add(
            "manager",
            draft("work", Status::Todo, 0, Some("alice")),
            true,
            0,
        )
        .unwrap()
        .task;
    let ClaimOutcome::Claimed(claimed) = board.claim("alice", Some(task.id), 5).unwrap() else {
        panic!("alice must claim her row");
    };
    let cursor_before = board.cursor_now().unwrap();
    let again = board.claim("alice", Some(task.id), 99).unwrap();
    let ClaimOutcome::AlreadyHeld(current) = again else {
        panic!("a held claim must be an idempotent read, got {again:?}");
    };
    assert_eq!(current, claimed.task);
    assert_eq!(current.revision, claimed.task.revision);
    assert_eq!(current.updated_at_ms, 5);
    assert_eq!(board.cursor_now().unwrap(), cursor_before);
    let page = board
        .feed(&FeedFilter {
            task: Some(task.id),
            kind: None,
            recipient: None,
            after: None,
            limit: 10,
        })
        .unwrap();
    assert_eq!(page.events.len(), 2);
}

#[test]
fn review_handoff_stays_on_one_task_with_generic_statuses() {
    let scratch = Scratch::new();
    let board = Board::open_with_options(scratch.path(), options()).unwrap();
    let task = board
        .add(
            "manager",
            draft("implement", Status::Todo, 0, Some("coder")),
            true,
            0,
        )
        .unwrap()
        .task;
    let ClaimOutcome::Claimed(claimed) = board.claim("coder", Some(task.id), 1).unwrap() else {
        panic!("coder must claim the task");
    };
    assert_eq!(claimed.task.status, Status::InProgress);
    let to_review = TaskPatch {
        status: Some(Status::Review),
        assignee: Some(Some("reviewer".into())),
        ..TaskPatch::default()
    };
    let handed = board
        .update(UpdateInput {
            id: task.id,
            actor: "coder",
            expected_revision: claimed.task.revision,
            patch: &to_review,
            force: false,
            force_reason: None,
            new_actor: true,
            comment: Some("commit abc123; cargo test 48 passed"),
            now_ms: 2,
        })
        .unwrap();
    assert_eq!(handed.event.recipient.as_deref(), Some("reviewer"));
    assert_eq!(handed.task.revision, claimed.task.revision + 1);
    let review_claim = board.claim("reviewer", Some(task.id), 3).unwrap();
    let ClaimOutcome::AlreadyHeld(current) = review_claim else {
        panic!("a reviewer claim must be an idempotent read, got {review_claim:?}");
    };
    assert_eq!(current.revision, handed.task.revision);
    let back_to_coder = TaskPatch {
        status: Some(Status::Todo),
        assignee: Some(Some("coder".into())),
        ..TaskPatch::default()
    };
    let fix = board
        .update(UpdateInput {
            id: task.id,
            actor: "reviewer",
            expected_revision: current.revision,
            patch: &back_to_coder,
            force: false,
            force_reason: None,
            new_actor: false,
            comment: Some("fix: claim path needs a skip test"),
            now_ms: 4,
        })
        .unwrap();
    assert_eq!(fix.event.recipient.as_deref(), Some("coder"));
    assert_eq!(fix.task.id, task.id);
    assert_eq!(board.list(&list_all()).unwrap().total, 1);
}

#[test]
fn eight_processes_claim_each_task_exactly_once() {
    let scratch = Scratch::new();
    let board_path = scratch.path().join("board");
    let board = Board::open_with_options(&board_path, options()).unwrap();
    for index in 0..24 {
        board
            .add(
                "manager",
                draft(&format!("task {index}"), Status::Todo, index, None),
                false,
                index as i64,
            )
            .unwrap();
    }
    drop(board);
    let mut workers = Vec::new();
    for actor in 0..8 {
        workers.push(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "store::m3_tests::claim_worker", "--nocapture"])
                .env("AGENTBOARD_M3_CLAIM_PATH", &board_path)
                .env("AGENTBOARD_M3_CLAIM_ACTOR", format!("worker-{actor}"))
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    let mut claimed = Vec::new();
    for worker in workers {
        let output = worker.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "worker failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if let Some(id) = line.strip_prefix("M3_CLAIMED=") {
                claimed.push(id.parse::<u64>().unwrap());
            }
        }
    }
    claimed.sort_unstable();
    claimed.dedup();
    assert_eq!(claimed.len(), 24);
    let board = Board::open_with_options(&board_path, options()).unwrap();
    let mut filter = list_all();
    filter.status = Some(Status::InProgress);
    assert_eq!(board.list(&filter).unwrap().total, 24);
    assert!(board.claim("worker-0", None, 99).unwrap() == ClaimOutcome::NoWork);
}

#[test]
fn claim_worker() {
    let Ok(path) = std::env::var("AGENTBOARD_M3_CLAIM_PATH") else {
        return;
    };
    let actor = std::env::var("AGENTBOARD_M3_CLAIM_ACTOR").unwrap();
    let board = Board::open_with_options(path, options()).unwrap();
    for _ in 0..200 {
        match board.claim(&actor, None, 1).unwrap() {
            ClaimOutcome::Claimed(updated) => println!("M3_CLAIMED={}", updated.task.id),
            ClaimOutcome::NoWork => return,
            ClaimOutcome::AlreadyHeld(task) => {
                panic!("id-less claim answered AlreadyHeld for T{}", task.id)
            }
        }
    }
    panic!("worker exhausted its attempt budget");
}

#[test]
#[ignore = "records the M3 claim latency budget; run with --ignored"]
fn claim_latency_over_one_thousand_tasks() {
    let scratch = Scratch::new();
    let board = Board::open_with_options(scratch.path(), options()).unwrap();
    for index in 0..1000 {
        board
            .add(
                "manager",
                draft(&format!("task {index}"), Status::Todo, index, None),
                false,
                index as i64,
            )
            .unwrap();
    }
    let latencies = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for worker in 0..8 {
            let board = &board;
            let latencies = &latencies;
            scope.spawn(move || {
                let actor = format!("worker-{worker}");
                loop {
                    let started = Instant::now();
                    match board.claim(&actor, None, 0).unwrap() {
                        ClaimOutcome::Claimed(_) => {
                            latencies
                                .lock()
                                .unwrap()
                                .push(started.elapsed().as_micros() as u64);
                        }
                        ClaimOutcome::NoWork => break,
                        ClaimOutcome::AlreadyHeld(task) => {
                            panic!("id-less claim answered AlreadyHeld for T{}", task.id)
                        }
                    }
                }
            });
        }
    });
    let mut latencies = latencies.into_inner().unwrap();
    assert_eq!(latencies.len(), 1000);
    latencies.sort_unstable();
    let p50 = latencies[latencies.len() / 2] as f64 / 1000.0;
    let p95 = latencies[latencies.len() * 95 / 100] as f64 / 1000.0;
    println!("claim latency: p50={p50:.3}ms p95={p95:.3}ms over 1000 tasks");
    // Debug builds carry the target. The release budget is 5 ms.
    assert!(p95 <= 25.0, "p95 {p95}ms exceeds the debug budget");
}
