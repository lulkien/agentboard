use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::{EventKind, NewProject, NewTask, Status};

use super::{Board, BoardOptions, Error, FeedFilter, ListFilter};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "agentboard-overview-{}-{nanos}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn board(scratch: &Scratch) -> Board {
    Board::open_with_options(&scratch.0, BoardOptions { map_size_mb: 8 }).unwrap()
}

fn project(board: &Board, name: &str) -> u64 {
    board
        .add_project(NewProject {
            project_name: name.into(),
            project_path: format!("/{name}"),
        })
        .unwrap()
        .project_id
}

#[allow(clippy::too_many_arguments)]
fn add(
    board: &Board,
    title: &str,
    status: Status,
    priority: i32,
    assignee: Option<&str>,
    parent: Option<u64>,
    project: u64,
    at: i64,
) -> u64 {
    board
        .add(
            "manager",
            NewTask {
                title: title.into(),
                description: String::new(),
                goal: String::new(),
                status,
                priority,
                assignee: assignee.map(str::to_owned),
                parent_id: parent,
                project_id: Some(project),
            },
            true,
            at,
        )
        .unwrap()
        .task
        .id
}

#[test]
fn overview_groups_lanes_attention_and_seats_with_caps() {
    let scratch = Scratch::new();
    let board = board(&scratch);
    let a = project(&board, "a");
    let b = project(&board, "b");
    let late = add(
        &board,
        "late",
        Status::Blocked,
        10,
        Some("zoe"),
        None,
        a,
        30,
    );
    let early = add(
        &board,
        "early",
        Status::Blocked,
        -1,
        Some("amy"),
        None,
        a,
        10,
    );
    let todo = add(&board, "todo", Status::Todo, 5, Some("amy"), None, a, 5);
    let review = add(
        &board,
        "review",
        Status::Reviewing,
        0,
        Some("zoe"),
        None,
        a,
        1,
    );
    add(&board, "unassigned", Status::Todo, 0, None, None, a, 2);
    add(
        &board,
        "foreign",
        Status::Blocked,
        -10,
        Some("amy"),
        None,
        b,
        0,
    );
    add(&board, "done", Status::Done, 0, None, None, a, 0);
    let view = board.overview(Some(a), 3).unwrap();
    assert_eq!(view.cursor.seq, board.cursor_now().unwrap().seq);
    assert_eq!(view.counts.iter().map(|(_, n)| n).sum::<u64>(), 6);
    assert_eq!(view.attention_total, 4);
    assert_eq!(
        view.attention.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![early, late, todo]
    );
    assert_eq!(
        view.lanes.iter().map(|l| l.status).collect::<Vec<_>>(),
        Status::ALL[..5]
    );
    let blocked = view
        .lanes
        .iter()
        .find(|l| l.status == Status::Blocked)
        .unwrap();
    assert_eq!(blocked.total, 2);
    assert_eq!(
        blocked.tasks.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![early, late]
    );
    assert_eq!(
        view.held
            .iter()
            .map(|h| h.assignee.as_str())
            .collect::<Vec<_>>(),
        vec!["amy", "zoe"]
    );
    assert_eq!(view.held[0].total, 2);
    assert_eq!(view.held[1].tasks[0].id, review);
    let capped = board.overview(Some(a), 1).unwrap();
    assert_eq!(capped.attention.len(), 1);
    assert_eq!(capped.attention_total, 4);
    assert_eq!(
        capped
            .lanes
            .iter()
            .find(|l| l.status == Status::Blocked)
            .unwrap()
            .total,
        2
    );
    assert_eq!(capped.held[0].total, 2);
    assert_eq!(capped.held[0].tasks.len(), 1);
}

#[test]
fn status_counts_ignore_status_but_honour_project_parent_assignee() {
    let scratch = Scratch::new();
    let board = board(&scratch);
    let a = project(&board, "a");
    let b = project(&board, "b");
    let parent = add(&board, "parent", Status::Todo, 0, None, None, a, 1);
    add(
        &board,
        "todo",
        Status::Todo,
        0,
        Some("amy"),
        Some(parent),
        a,
        2,
    );
    add(
        &board,
        "done",
        Status::Done,
        0,
        Some("amy"),
        Some(parent),
        a,
        3,
    );
    add(
        &board,
        "other",
        Status::Blocked,
        0,
        Some("zoe"),
        Some(parent),
        a,
        4,
    );
    add(
        &board,
        "foreign",
        Status::Done,
        0,
        Some("amy"),
        Some(parent),
        b,
        5,
    );
    let filter = ListFilter {
        status: Some(Status::Todo),
        assignee: Some("amy"),
        parent: Some(parent),
        project: Some(a),
        include_terminal: false,
        limit: 1,
        offset: 10,
    };
    let counts = board.status_counts(&filter).unwrap();
    assert_eq!(counts.iter().map(|(_, n)| n).sum::<u64>(), 2);
    assert_eq!(
        counts.iter().find(|(s, _)| *s == Status::Todo).unwrap().1,
        1
    );
    assert_eq!(
        counts.iter().find(|(s, _)| *s == Status::Done).unwrap().1,
        1
    );
    assert_eq!(
        counts.iter().map(|(s, _)| *s).collect::<Vec<_>>(),
        Status::ALL
    );
}

#[test]
fn feed_in_project_pages_back_without_gaps_or_foreign_rows() {
    let scratch = Scratch::new();
    let board = board(&scratch);
    let a = project(&board, "a");
    let b = project(&board, "b");
    let task_a = add(&board, "a", Status::Todo, 0, None, None, a, 1);
    let task_b = add(&board, "b", Status::Todo, 0, None, None, b, 2);
    board.comment("manager", task_a, "a1", 3).unwrap();
    board.comment("manager", task_b, "b1", 4).unwrap();
    board.comment("manager", task_a, "a2", 5).unwrap();
    let filter = FeedFilter {
        task: None,
        kind: None,
        recipient: None,
        after: None,
        limit: 1,
    };
    let newest = board.feed_in_project(&filter, a, None, 2).unwrap();
    assert_eq!(
        newest.events.iter().map(|e| e.seq).collect::<Vec<_>>(),
        vec![3, 5]
    );
    assert!(newest.older);
    assert_eq!(newest.before.seq, 3);
    let older = board
        .feed_in_project(&filter, a, Some(&newest.before.encode()), 2)
        .unwrap();
    assert_eq!(
        older.events.iter().map(|e| e.seq).collect::<Vec<_>>(),
        vec![1]
    );
    assert!(!older.older);
    let comments = board
        .feed_in_project(
            &FeedFilter {
                task: Some(task_a),
                kind: Some(EventKind::Commented),
                ..filter
            },
            a,
            None,
            10,
        )
        .unwrap();
    assert_eq!(
        comments.events.iter().map(|e| e.seq).collect::<Vec<_>>(),
        vec![3, 5]
    );
    assert!(
        board
            .feed_in_project(&filter, a, Some("0"), 2)
            .unwrap()
            .events
            .is_empty()
    );
    assert!(matches!(
        board.feed_in_project(&filter, a, Some("foreign:2"), 2),
        Err(Error::CursorInstanceMismatch)
    ));
}
