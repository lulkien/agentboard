use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::{NewTask, Status, TaskPatch};

use super::open::{FORMAT_VERSION, Metadata, write_metadata};
use super::{Board, BoardOptions, Error, UpdateInput};

static NEXT_SCRATCH: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "agentboard-lmdb-m2-{}-{nanos}-{}",
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
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn options() -> BoardOptions {
    BoardOptions { map_size_mb: 8 }
}

#[cfg(target_os = "linux")]
fn lock_fds() -> Vec<String> {
    let mut held = Vec::new();
    let Ok(entries) = fs::read_dir("/proc/self/fd") else {
        return held;
    };
    for entry in entries.flatten() {
        if let Ok(target) = fs::read_link(entry.path())
            && target
                .file_name()
                .is_some_and(|name| name == "lifetime.lock")
        {
            held.push(format!(
                "{} -> {}",
                entry.file_name().to_string_lossy(),
                target.display()
            ));
        }
    }
    held
}

#[cfg(not(target_os = "linux"))]
fn lock_fds() -> Vec<String> {
    Vec::new()
}

fn draft(assignee: Option<&str>) -> NewTask {
    NewTask {
        title: "Prepare a patch".into(),
        description: "Issue context".into(),
        goal: "One reviewed result".into(),
        status: Status::Todo,
        priority: 10,
        assignee: assignee.map(str::to_owned),
        parent_id: None,
        path: None,
    }
}

#[test]
fn fresh_boards_are_independent_and_add_is_atomic() {
    let a = Scratch::new();
    let b = Scratch::new();
    let first = Board::open_with_options(a.path(), options()).unwrap();
    let second = Board::open_with_options(b.path(), options()).unwrap();
    assert!(matches!(
        Board::open_with_options(a.path(), options()),
        Err(Error::EnvironmentAlreadyOpen)
    ));
    assert_ne!(
        first.metadata().unwrap().instance_id,
        second.metadata().unwrap().instance_id
    );
    let added = first
        .add("manager", draft(Some("coder")), true, 10)
        .unwrap();
    assert_eq!(added.task.id, 1);
    assert_eq!(added.event.seq, 1);
    assert_eq!(added.event.recipient.as_deref(), Some("coder"));
    assert_eq!(first.get(1).unwrap(), Some(added.task));
    assert_eq!(first.event(1).unwrap(), Some(added.event));
    assert_eq!(second.get(1).unwrap(), None);
    assert_eq!(first.metadata().unwrap().next_task_id, 2);
    assert_eq!(first.metadata().unwrap().next_event_seq, 2);
    assert_eq!(
        first.agent("manager").unwrap().unwrap().last_seen_ms,
        Some(10)
    );
    assert_eq!(first.agent("coder").unwrap().unwrap().last_seen_ms, None);
    let txn = first.env.read_txn().unwrap();
    assert_eq!(first.tasks.len(&txn).unwrap(), 1);
    assert_eq!(first.events.len(&txn).unwrap(), 1);
    assert_eq!(first.events_by_task.len(&txn).unwrap(), 1);
    assert_eq!(first.agents.len(&txn).unwrap(), 2);
    assert_eq!(first.tasks_by_status.len(&txn).unwrap(), 1);
    assert_eq!(first.tasks_by_assignee.len(&txn).unwrap(), 1);
}

#[test]
fn failed_add_leaves_counters_roster_events_and_indexes_untouched() {
    let scratch = Scratch::new();
    let board = Board::open_with_options(scratch.path(), options()).unwrap();
    assert!(matches!(
        board.add("manager", draft(Some("misspelled")), false, 10),
        Err(Error::UnknownAssignee(name)) if name == "misspelled"
    ));
    assert_eq!(board.get(1).unwrap(), None);
    assert_eq!(board.event(1).unwrap(), None);
    assert_eq!(board.agent("manager").unwrap(), None);
    assert_eq!(board.agent("misspelled").unwrap(), None);
    assert_eq!(board.metadata().unwrap().next_task_id, 1);
    let txn = board.env.read_txn().unwrap();
    assert_eq!(board.tasks_by_status.len(&txn).unwrap(), 0);
    assert_eq!(board.tasks_by_assignee.len(&txn).unwrap(), 0);
    drop(txn);
    let mut child = draft(None);
    child.parent_id = Some(99);
    assert!(matches!(
        board.add("manager", child, false, 11),
        Err(Error::MissingParent(99))
    ));
    assert_eq!(board.metadata().unwrap().next_task_id, 1);
}

#[test]
fn format_and_map_size_refuse_before_any_task_decode() {
    let scratch = Scratch::new();
    let board = Board::open_with_options(scratch.path(), options()).unwrap();
    let original = board.metadata().unwrap().instance_id;
    board.add("manager", draft(None), false, 0).unwrap();
    drop(board);
    assert!(matches!(
        Board::open_with_options(scratch.path(), BoardOptions { map_size_mb: 16 }),
        Err(Error::MapSizeMismatch {
            configured_mb: 16,
            stored_mb: 8
        })
    ));
    let board = Board::open_with_options(scratch.path(), options()).unwrap();
    let mut metadata = board.metadata().unwrap();
    metadata.format_version = FORMAT_VERSION + 1;
    let mut txn = board.env.write_txn().unwrap();
    write_metadata(board.meta, &mut txn, &metadata).unwrap();
    txn.commit().unwrap();
    assert!(matches!(
        board.get(1),
        Err(Error::FormatMismatch { expected: FORMAT_VERSION, found }) if found == FORMAT_VERSION + 1
    ));
    drop(board);
    assert!(matches!(
        Board::open_with_options(scratch.path(), options()),
        Err(Error::FormatMismatch { expected: FORMAT_VERSION, found }) if found == FORMAT_VERSION + 1
    ));
    let reset = Board::reset(scratch.path(), options());
    assert!(
        reset.is_ok(),
        "reset failed: {reset:?}; lock fds: {:?}",
        lock_fds()
    );
    let reset = reset.unwrap();
    assert_eq!(reset.old_instance_id.as_deref(), Some(original.as_str()));
    assert_ne!(reset.new_instance_id, original);
    let board = Board::open_with_options(scratch.path(), options()).unwrap();
    assert_eq!(board.get(1).unwrap(), None);
    assert_eq!(board.metadata().unwrap().next_task_id, 1);
}

#[test]
fn reset_refuses_attached_board_and_retries_interrupted_disposal() {
    let scratch = Scratch::new();
    let board = Board::open_with_options(scratch.path(), options()).unwrap();
    board.add("manager", draft(None), false, 0).unwrap();
    assert!(matches!(
        Board::reset(scratch.path(), options()),
        Err(Error::ResetBusy)
    ));
    assert!(board.get(1).unwrap().is_some());
    drop(board);
    fs::remove_file(scratch.path().join("data.mdb")).unwrap();
    let reset = Board::reset(scratch.path(), options()).unwrap();
    assert_eq!(reset.old_instance_id, None);
    let board = Board::open_with_options(scratch.path(), options()).unwrap();
    assert_eq!(board.get(1).unwrap(), None);
}

#[test]
fn simultaneous_first_use_allocates_unique_tasks() {
    let scratch = Scratch::new();
    let go = scratch.path().join("go");
    let board_path = scratch.path().join("board");
    let mut workers = Vec::new();
    for actor in 0..8 {
        workers.push(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "store::tests::first_use_worker", "--nocapture"])
                .env("AGENTBOARD_M2_TEST_PATH", &board_path)
                .env("AGENTBOARD_M2_TEST_GO", &go)
                .env("AGENTBOARD_M2_TEST_ACTOR", format!("worker-{actor}"))
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    fs::write(&go, b"start").unwrap();
    let mut ids = Vec::new();
    for worker in workers {
        let output = worker.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "stdout: {} stderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let id = stdout
            .lines()
            .find_map(|line| line.split_once("M2_TASK_ID=").map(|(_, id)| id))
            .unwrap()
            .parse::<u64>()
            .unwrap();
        ids.push(id);
    }
    ids.sort_unstable();
    assert_eq!(ids, (1..=8).collect::<Vec<_>>());
    let board = Board::open_with_options(&board_path, options()).unwrap();
    assert_eq!(board.metadata().unwrap().next_event_seq, 9);
    let txn = board.env.read_txn().unwrap();
    assert_eq!(board.events.len(&txn).unwrap(), 8);
    assert_eq!(board.tasks_by_status.len(&txn).unwrap(), 8);
}

#[test]
fn first_use_worker() {
    let Ok(path) = std::env::var("AGENTBOARD_M2_TEST_PATH") else {
        return;
    };
    let go = std::env::var("AGENTBOARD_M2_TEST_GO").unwrap();
    let actor = std::env::var("AGENTBOARD_M2_TEST_ACTOR").unwrap();
    for _ in 0..1000 {
        if Path::new(&go).exists() {
            let board = Board::open_with_options(path, options()).unwrap();
            let id = board.add(&actor, draft(None), false, 0).unwrap().task.id;
            println!("M2_TASK_ID={id}");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("first-use worker did not receive start signal");
}

#[test]
fn reset_is_excluded_by_another_process() {
    let scratch = Scratch::new();
    let board_path = scratch.path().join("board");
    let ready = scratch.path().join("ready");
    let release = scratch.path().join("release");
    let board = Board::open_with_options(&board_path, options()).unwrap();
    board.add("manager", draft(None), false, 0).unwrap();
    drop(board);
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "store::tests::hold_board_worker", "--nocapture"])
        .env("AGENTBOARD_M2_HOLD_PATH", &board_path)
        .env("AGENTBOARD_M2_HOLD_READY", &ready)
        .env("AGENTBOARD_M2_HOLD_RELEASE", &release)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut appeared = false;
    for _ in 0..1000 {
        if ready.exists() {
            appeared = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let refused = if appeared {
        Some(Board::reset(&board_path, options()))
    } else {
        None
    };
    fs::write(&release, b"done").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        appeared,
        "child never opened board: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(matches!(refused, Some(Err(Error::ResetBusy))));
    let board = Board::open_with_options(&board_path, options()).unwrap();
    assert!(board.get(1).unwrap().is_some());
    drop(board);
    Board::reset(&board_path, options()).unwrap();
}

#[test]
fn hold_board_worker() {
    let Ok(path) = std::env::var("AGENTBOARD_M2_HOLD_PATH") else {
        return;
    };
    let ready = std::env::var("AGENTBOARD_M2_HOLD_READY").unwrap();
    let release = std::env::var("AGENTBOARD_M2_HOLD_RELEASE").unwrap();
    let board = Board::open_with_options(path, options()).unwrap();
    fs::write(ready, b"open").unwrap();
    for _ in 0..1000 {
        if Path::new(&release).exists() {
            drop(board);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("hold worker did not receive release signal");
}

#[test]
fn full_map_aborts_write_and_preserves_prior_rows() {
    let scratch = Scratch::new();
    let options = BoardOptions { map_size_mb: 1 };
    let board = Board::open_with_options(scratch.path(), options).unwrap();
    board.add("manager", draft(None), false, 0).unwrap();
    let mut huge = draft(None);
    huge.description = "x".repeat(2 * 1024 * 1024);
    let error = board.add("manager", huge, false, 1).unwrap_err();
    assert!(matches!(error, Error::MapFull));
    assert!(error.to_string().contains("AGENTBOARD_MAP_SIZE_MB"));
    assert!(board.get(1).unwrap().is_some());
    assert_eq!(board.get(2).unwrap(), None);
    assert_eq!(board.metadata().unwrap().next_task_id, 2);
    assert_eq!(board.event(2).unwrap(), None);
}

#[test]
fn unrelated_directory_refuses_initialization() {
    let scratch = Scratch::new();
    fs::write(scratch.path().join("unrelated.txt"), b"keep").unwrap();
    assert!(matches!(
        Board::open_with_options(scratch.path(), options()),
        Err(Error::UnknownEntry(name)) if name == "unrelated.txt"
    ));
    assert_eq!(
        fs::read(scratch.path().join("unrelated.txt")).unwrap(),
        b"keep"
    );
    // A refused open must not leave a lock file behind in a foreign directory.
    assert!(!scratch.path().join("lifetime.lock").exists());
}

#[cfg(unix)]
#[test]
fn reset_refuses_symlinked_lmdb_file() {
    let scratch = Scratch::new();
    let board_dir = scratch.path().join("board");
    fs::create_dir(&board_dir).unwrap();
    let external = scratch.path().join("external-data");
    fs::write(&external, b"keep").unwrap();
    std::os::unix::fs::symlink(&external, board_dir.join("data.mdb")).unwrap();
    assert!(matches!(
        Board::reset(&board_dir, options()),
        Err(Error::UnsafePath(_))
    ));
    assert_eq!(fs::read(external).unwrap(), b"keep");
}

#[test]
fn updates_replace_both_index_keys_and_record_only_real_handoffs() {
    let scratch = Scratch::new();
    let board = Board::open_with_options(scratch.path(), options()).unwrap();
    let original = board
        .add("manager", draft(Some("alice")), true, 0)
        .unwrap()
        .task;
    let working_patch = TaskPatch {
        status: Some(Status::InProgress),
        ..TaskPatch::default()
    };
    let working = board
        .update(UpdateInput {
            id: 1,
            actor: "alice",
            expected_revision: 1,
            patch: &working_patch,
            force: false,
            force_reason: None,
            new_actor: false,
            comment: None,
            now_ms: 1,
        })
        .unwrap();
    assert_eq!(working.task.revision, 2);
    assert_eq!(working.event.recipient, None);
    let review_patch = TaskPatch {
        status: Some(Status::Review),
        assignee: Some(Some("reviewer".into())),
        ..TaskPatch::default()
    };
    let review = board
        .update(UpdateInput {
            id: 1,
            actor: "alice",
            expected_revision: 2,
            patch: &review_patch,
            force: false,
            force_reason: None,
            new_actor: true,
            comment: Some("patch abc123"),
            now_ms: 2,
        })
        .unwrap();
    assert_eq!(review.task.revision, 3);
    assert_eq!(review.event.recipient.as_deref(), Some("reviewer"));
    assert_eq!(review.event.payload["changes"]["assignee"]["old"], "alice");
    assert_eq!(
        review.event.payload["changes"]["assignee"]["new"],
        "reviewer"
    );
    assert_eq!(board.agent("reviewer").unwrap().unwrap().last_seen_ms, None);
    let txn = board.env.read_txn().unwrap();
    assert_eq!(board.tasks_by_status.len(&txn).unwrap(), 1);
    assert_eq!(board.tasks_by_assignee.len(&txn).unwrap(), 1);
    assert_eq!(
        board
            .tasks_by_status
            .get(&txn, &super::keys::status(&original))
            .unwrap(),
        None
    );
    assert_eq!(
        board
            .tasks_by_assignee
            .get(&txn, &super::keys::assignee(&original).unwrap())
            .unwrap(),
        None
    );
    assert!(
        board
            .tasks_by_status
            .get(&txn, &super::keys::status(&review.task))
            .unwrap()
            .is_some()
    );
    assert!(
        board
            .tasks_by_assignee
            .get(&txn, &super::keys::assignee(&review.task).unwrap())
            .unwrap()
            .is_some()
    );
    drop(txn);
    let title_patch = TaskPatch {
        title: Some("Clarified".into()),
        ..TaskPatch::default()
    };
    let forced = board
        .update(UpdateInput {
            id: 1,
            actor: "manager",
            expected_revision: 3,
            patch: &title_patch,
            force: true,
            force_reason: Some("scope correction"),
            new_actor: false,
            comment: None,
            now_ms: 3,
        })
        .unwrap();
    assert_eq!(forced.event.recipient, None);
    assert_eq!(forced.event.payload["reason"], "scope correction");
    assert_eq!(board.metadata().unwrap().next_event_seq, 5);
}

#[test]
fn failed_update_preserves_indexes_and_parent_acyclicity() {
    let scratch = Scratch::new();
    let board = Board::open_with_options(scratch.path(), options()).unwrap();
    board.add("manager", draft(None), false, 0).unwrap();
    let mut child = draft(None);
    child.parent_id = Some(1);
    board.add("manager", child, false, 1).unwrap();
    let cycle = TaskPatch {
        parent_id: Some(Some(2)),
        ..TaskPatch::default()
    };
    assert!(matches!(
        board.update(UpdateInput {
            id: 1,
            actor: "manager",
            expected_revision: 1,
            patch: &cycle,
            force: false,
            force_reason: None,
            new_actor: false,
            comment: None,
            now_ms: 2,
        }),
        Err(Error::ParentCycle(1))
    ));
    let unknown = TaskPatch {
        assignee: Some(Some("not-registered".into())),
        ..TaskPatch::default()
    };
    assert!(matches!(
        board.update(UpdateInput {
            id: 1, actor: "manager", expected_revision: 1, patch: &unknown,
            force: false, force_reason: None, new_actor: false, comment: None, now_ms: 2,
        }),
        Err(Error::UnknownAssignee(name)) if name == "not-registered"
    ));
    assert_eq!(board.get(1).unwrap().unwrap().revision, 1);
    assert_eq!(board.metadata().unwrap().next_event_seq, 3);
    assert_eq!(board.agent("not-registered").unwrap(), None);
    let txn = board.env.read_txn().unwrap();
    assert_eq!(board.tasks_by_status.len(&txn).unwrap(), 2);
    assert_eq!(board.events.len(&txn).unwrap(), 2);
}

#[test]
fn existing_board_refuses_a_missing_named_database() {
    let scratch = Scratch::new();
    {
        let mut builder = heed::EnvOpenOptions::new();
        builder.max_dbs(7).map_size(8 * 1024 * 1024);
        // SAFETY: the scratch environment is opened and dropped inside this test.
        let env = unsafe { builder.open(scratch.path()) }.unwrap();
        let txn = env.write_txn().unwrap();
        let metadata = Metadata {
            application: "agentboard-lmdb".into(),
            format_version: FORMAT_VERSION,
            instance_id: "half-built".into(),
            next_task_id: 1,
            next_event_seq: 1,
            map_size_mb: 8,
        };
        env.create_databases_and_commit(txn, |opener, txn| -> super::Result<_> {
            let meta = opener
                .create_database::<heed::types::Bytes, heed::types::Bytes>(txn, Some("meta"))?;
            meta.put(txn, b"envelope", &serde_json::to_vec(&metadata).unwrap())?;
            Ok(heed::CommitOrAbort::<_, ()>::Commit(()))
        })
        .unwrap();
    }
    assert!(matches!(
        Board::open_with_options(scratch.path(), options()),
        Err(Error::Corrupt("missing tasks database"))
    ));
}

#[test]
fn reset_refuses_unknown_directory_entry_and_preserves_the_board() {
    let scratch = Scratch::new();
    let board = Board::open_with_options(scratch.path(), options()).unwrap();
    board.add("manager", draft(None), false, 0).unwrap();
    drop(board);
    fs::write(scratch.path().join("notes.txt"), b"keep me").unwrap();
    let outcome = Board::reset(scratch.path(), options());
    assert!(
        matches!(outcome, Err(Error::UnknownEntry(ref name)) if name == "notes.txt"),
        "expected UnknownEntry(notes.txt), got {outcome:?}; lock fds: {:?}",
        lock_fds()
    );
    assert!(scratch.path().join("data.mdb").exists());
    assert_eq!(
        fs::read(scratch.path().join("notes.txt")).unwrap(),
        b"keep me"
    );
    // With the board attached the attached-client refusal is reported first.
    let board = Board::open_with_options(scratch.path(), options()).unwrap();
    let outcome = Board::reset(scratch.path(), options());
    assert!(
        matches!(outcome, Err(Error::ResetBusy)),
        "expected ResetBusy, got {outcome:?}"
    );
    assert_eq!(board.get(1).unwrap().unwrap().title, "Prepare a patch");
    drop(board);
    // Once the stray entry is gone the board opens and its rows are intact.
    fs::remove_file(scratch.path().join("notes.txt")).unwrap();
    let board = Board::open_with_options(scratch.path(), options()).unwrap();
    assert_eq!(board.get(1).unwrap().unwrap().title, "Prepare a patch");
}

#[test]
fn indexed_claim_probe_skips_foreign_assignee_and_preserves_order() {
    let scratch = Scratch::new();
    let board = Board::open_with_options(scratch.path(), options()).unwrap();
    let mut alice = draft(Some("alice"));
    alice.priority = -10;
    board.add("manager", alice, true, 0).unwrap();
    assert_eq!(board.next_claimable_id("bob").unwrap(), None);
    let mut bob = draft(Some("bob"));
    bob.priority = -5;
    board.add("manager", bob, true, 1).unwrap();
    board.add("manager", draft(None), false, 2).unwrap();
    assert_eq!(board.next_claimable_id("alice").unwrap(), Some(1));
    assert_eq!(board.next_claimable_id("bob").unwrap(), Some(2));
    assert_eq!(board.next_claimable_id("charlie").unwrap(), Some(3));
}
