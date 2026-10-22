//! New workflow rules. Statuses opt a chain into this protocol; legacy status
//! paths remain unchanged. No persisted field or schema generation is added.
use super::short;
use super::text::{MAX_REOPENS, TaskRow};

pub fn transition(level: usize, current: &str, next: &str) -> Result<(), String> {
    if current == next {
        return Ok(());
    }
    let allowed = match next {
        "needs_impl" => level == 1 && matches!(current, "ready" | "working" | "needs_impl"),
        "needs_review" => level == 2 && matches!(current, "ready" | "working" | "needs_review"),
        _ => true,
    };
    if allowed {
        Ok(())
    } else {
        Err(format!(
            "level {level} cannot move from {current} to {next}: analyze -> implement -> review"
        ))
    }
}

/// Publishing a plan or a submitted implementation requires a real transition,
/// never a creation-time status shortcut (even with a plan in the description).
pub fn initial_status(status: &str) -> Result<(), String> {
    if matches!(status, "needs_impl" | "needs_review") {
        Err(format!(
            "cannot add at {status}; create ready or working, then publish through a status transition"
        ))
    } else {
        Ok(())
    }
}

/// None validates an existing subtree; Some identifies the attaching row. -1 is
/// a not-yet-allocated child, so every existing child is a conflicting sibling.
pub fn single_child(parent: i64, children: &[i64], moving: Option<i64>) -> Result<(), String> {
    let conflict = match moving {
        Some(id) => children.iter().find(|child| **child != id),
        None => children.get(1),
    };
    if let Some(child) = conflict {
        Err(format!(
            "new chain {} already has child {}; only one child is allowed",
            short(parent),
            short(*child)
        ))
    } else {
        Ok(())
    }
}

pub fn depth_refusal(parent: i64, level: usize, height: usize) -> String {
    format!(
        "chain too deep: {} is level {level}, reaching level {}; a review row takes no child; reopen the implement row",
        short(parent),
        level + height
    )
}

/// Validate the coder-filed review brief against the persisted parent and submission.
pub fn review_brief(
    reporter: &str,
    implement: &TaskRow,
    description: &str,
    submission: Option<&super::text::Submission>,
) -> Result<(), String> {
    if reporter.trim().is_empty() || implement.assignee.as_deref() != Some(reporter) {
        return Err(format!(
            "review creation refused: reporter {reporter:?} must be the assignee or holder of {}",
            short(implement.id)
        ));
    }
    let submission = submission.ok_or_else(|| super::text::gate_refusal(implement.id))?;
    let names: Vec<&str> = description
        .split(|c: char| !c.is_ascii_alphanumeric())
        .collect();
    let plan = short(
        implement
            .parent_id
            .ok_or("review parent has no analyze plan")?,
    );
    let diff = super::short_diff(submission.diff.id);
    if !names.contains(&plan.as_str()) || !names.contains(&diff.as_str()) {
        return Err(format!(
            "review description must name diff {diff} and analyze plan {plan}"
        ));
    }
    if !matches!(
        implement.status.as_str(),
        "working" | "needs_review" | "review"
    ) {
        return Err(format!(
            "{} is not ready to hand over for review",
            short(implement.id)
        ));
    }
    Ok(())
}

/// Refresh only an admissible persisted review: its reporter is immutable and
/// its plan reference must already be present. Only the round's diff may change.
pub fn refresh_review(
    review: &TaskRow,
    implement: &TaskRow,
    report: Option<&str>,
    submission: &super::text::Submission,
) -> Result<String, String> {
    let plan = short(
        implement
            .parent_id
            .ok_or("review parent has no analyze plan")?,
    );
    let description = format!(
        "Analyze plan {plan}.\n{}",
        super::text::review_description(implement, report, submission)
    );
    review_brief(&review.reporter, implement, &description, Some(submission))?;
    if !review
        .description
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|name| name == plan)
    {
        return Err(format!(
            "review description must name diff {} and analyze plan {plan}",
            super::short_diff(submission.diff.id)
        ));
    }
    Ok(description)
}

pub struct Snapshot {
    pub rows: Vec<TaskRow>,
    pub diff_actor: Option<String>,
    pub close_actor: Option<String>,
}

pub enum Action<'a> {
    Close { status: &'a str, verdict: &'a str },
    Reopen { reason: &'a str },
    Reject { reason: &'a str },
}

pub struct Change {
    pub id: i64,
    pub status: String,
    pub count: i64,
    pub body: String,
    pub kind: &'static str,
}

pub fn plan(actor: &str, snapshot: &Snapshot, action: Action<'_>) -> Result<Vec<Change>, String> {
    if actor.trim().is_empty() {
        return Err("chain operation requires an acting identity".into());
    }
    let rows = &snapshot.rows;
    if rows.len() != 3
        || rows[0].parent_id.is_some()
        || rows[1].parent_id != Some(rows[0].id)
        || rows[2].parent_id != Some(rows[1].id)
    {
        return Err(
            "chain requires exactly analyze -> implement -> review, with no review children".into(),
        );
    }
    let (kind, statuses, count, body) = match action {
        Action::Close { status, verdict } => {
            if !matches!(status, "done" | "not_done") {
                return Err("chain close requires done or not_done".into());
            }
            for (row, allowed) in [
                (&rows[0], &["needs_impl"][..]),
                (&rows[1], &["needs_review", "review"][..]),
                (&rows[2], &["needs_review", "review"][..]),
            ] {
                if !allowed.contains(&row.status.as_str()) {
                    return Err(format!(
                        "cannot close {}: status {} (expected {})",
                        short(row.id),
                        row.status,
                        allowed.join(" or ")
                    ));
                }
            }
            let verdict = if verdict.trim().is_empty() {
                rows[2].report.as_deref().unwrap_or("")
            } else {
                verdict
            };
            if verdict.trim().is_empty() {
                return Err(format!("{} has no verdict", short(rows[2].id)));
            }
            let coder = snapshot
                .diff_actor
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .ok_or("chain has no recorded diff identity")?;
            if actor == coder {
                return Err("chain close refused: acting identity recorded the newest diff".into());
            }
            (
                "chain_close",
                vec![status; 3],
                rows[1].reopen_count,
                verdict.to_owned(),
            )
        }
        Action::Reopen { reason } => {
            if reason.trim().is_empty() {
                return Err("chain reopen requires a reason".into());
            }
            for row in rows {
                if !matches!(row.status.as_str(), "done" | "not_done")
                    || row.status != rows[0].status
                {
                    return Err(format!(
                        "cannot reopen {}: status {} (expected a closed chain)",
                        short(row.id),
                        row.status
                    ));
                }
                if row
                    .commit_ref
                    .as_deref()
                    .is_some_and(|c| !c.trim().is_empty())
                {
                    return Err(format!(
                        "cannot reopen {}: already committed",
                        short(row.id)
                    ));
                }
            }
            let closer = snapshot
                .close_actor
                .as_deref()
                .ok_or("chain has no recorded close identity")?;
            if actor == closer {
                return Err("chain reopen refused: acting identity recorded the close".into());
            }
            let count = rows[1].reopen_count + 1;
            if count > MAX_REOPENS {
                return Err(format!(
                    "reopen cap {MAX_REOPENS} hit; reopen the implement row is no longer allowed"
                ));
            }
            (
                "chain_reopen",
                vec!["needs_impl", "needs_impl", "needs_review"],
                count,
                reason.to_owned(),
            )
        }
        Action::Reject { reason } => {
            if reason.trim().is_empty() {
                return Err("review rejection requires a reason".into());
            }
            if !matches!(
                rows[2].status.as_str(),
                "ready" | "working" | "needs_review" | "review"
            ) {
                return Err(format!(
                    "cannot reject {} from {}",
                    short(rows[2].id),
                    rows[2].status
                ));
            }
            if !matches!(rows[1].status.as_str(), "needs_review" | "review")
                || rows[0].status != "needs_impl"
            {
                return Err("review rejection requires the implement row awaiting review".into());
            }
            let count = rows[1].reopen_count + 1;
            if count > MAX_REOPENS {
                (
                    "chain_reject",
                    vec!["not_done"; 3],
                    count,
                    format!("reopen cap {MAX_REOPENS} hit; {reason}"),
                )
            } else {
                (
                    "chain_reject",
                    vec!["needs_impl", "needs_fix", "needs_fix"],
                    count,
                    reason.to_owned(),
                )
            }
        }
    };
    Ok(rows
        .iter()
        .zip(statuses)
        .enumerate()
        .map(|(index, (row, status))| Change {
            id: row.id,
            status: status.to_owned(),
            count: if index == 1 { count } else { row.reopen_count },
            body: body.clone(),
            kind,
        })
        .collect())
}

pub fn summary(changes: &[Change]) -> String {
    format!(
        "{}: {}",
        changes[0].kind,
        changes
            .iter()
            .map(|c| format!("{} {}", short(c.id), c.status))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// A capped rejection commits the terminal state, then reports refusal, just as
/// the legacy reopen cap does. Other errors never commit a partial chain.
pub fn finish((text, capped): (String, bool)) -> Result<String, String> {
    if capped {
        Err(format!(
            "reopen cap {MAX_REOPENS} hit; {text}; reopen the implement row is no longer allowed"
        ))
    } else {
        Ok(text)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::store::{self, NewTask, Patch};
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    #[cfg(feature = "sqlite")]
    pub fn pause_close() {
        if let Ok(path) = std::env::var("AGENTBOARD_CHAIN_PAUSE") {
            std::fs::write(path, "first row updated inside transaction").unwrap();
            std::thread::sleep(Duration::from_secs(30));
        }
    }

    fn add(title: &str, parent: Option<i64>, status: &str) -> Result<i64, String> {
        let review = parent.is_some_and(|p| store::get(p, true).unwrap().contains("parent: T1"));
        add_as(
            if review { "coder" } else { "manager" },
            title,
            parent,
            status,
            if review {
                "Read D1 against analyze plan T1"
            } else {
                ""
            },
        )
    }
    fn add_as(
        actor: &str,
        title: &str,
        parent: Option<i64>,
        status: &str,
        description: &str,
    ) -> Result<i64, String> {
        store::add(
            actor,
            &NewTask {
                title,
                description,
                goal: "",
                status,
                assignee: None,
                priority: 100,
                epic: None,
                parent,
                path: None,
                branch: None,
            },
        )
        .map(|reply| {
            reply.split_whitespace().next().unwrap()[1..]
                .parse()
                .unwrap()
        })
    }
    fn edit_description(id: i64, description: &str) {
        store::edit(
            "manager",
            id,
            &crate::store::TaskEdit {
                description: Some(description),
                epic: None,
                title: None,
                goal: None,
                priority: None,
                assignee: None,
                path: None,
                branch: None,
            },
        )
        .unwrap_or_else(|e| panic!("{e}"));
    }
    fn change(actor: &str, id: i64, status: &str, report: Option<&str>) -> Result<String, String> {
        store::apply(
            actor,
            id,
            &Patch {
                edit: None,
                status: Some(status),
                report,
                note: None,
                commit: None,
                needs: None,
                ttl_s: 60,
            },
        )
    }
    fn stable_record(id: i64) -> String {
        store::get(id, true)
            .unwrap()
            .lines()
            .filter(|line| {
                !line.starts_with("created:")
                    && !line.starts_with("started:")
                    && !line.starts_with("completed:")
                    && !line.starts_with("lease:")
                    && !line.starts_with("diff:")
                    && !line.trim_start().starts_with('#')
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn resubmit(actor: &str, id: i64, body: &str) {
        assert!(
            change(actor, id, "needs_review", None)
                .unwrap_err()
                .contains("cannot move from")
        );
        let claim = store::claim(actor, None, 60).unwrap_or_else(|e| panic!("{e}"));
        assert!(claim.starts_with(&format!("T{id} working")), "{claim}");
        assert!(
            change(actor, id, "needs_review", None)
                .unwrap_err()
                .contains("no diff")
        );
        store::diff_add(actor, id, body, None).unwrap();
        change(actor, id, "needs_review", None).unwrap();
    }
    fn statuses(ids: &[i64], expected: &[&str]) {
        for (id, status) in ids.iter().zip(expected) {
            let text = store::get(*id, true).unwrap();
            assert!(
                text.lines()
                    .next()
                    .unwrap()
                    .starts_with(&format!("T{id} {status} ")),
                "{text}"
            );
        }
    }
    fn seed() {
        assert_eq!(add("analyze", None, "ready").unwrap(), 1);
        assert!(
            change("analyzer", 1, "needs_review", None)
                .unwrap_err()
                .contains("level 1")
        );
        change("analyzer", 1, "needs_impl", Some("plan")).unwrap();
        assert_eq!(add("implement", Some(1), "ready").unwrap(), 2);
        assert!(
            change("coder", 2, "needs_review", None)
                .unwrap_err()
                .contains("no diff")
        );
        store::claim("coder", Some(2), 60).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            add_as("coder", "review", Some(2), "ready", "D1 T1")
                .unwrap_err()
                .contains("no diff")
        );
        store::diff_add("coder", 2, "patch one", None).unwrap();
        store::report("coder", 2, "implementation").unwrap();
        for actor in ["manager", "reviewer", "wanderer"] {
            assert!(
                add_as(actor, "review", Some(2), "ready", "D1 T1")
                    .unwrap_err()
                    .contains("reporter")
            );
        }
        for description in ["", "Read D1", "Read T1", "Read D10 T10"] {
            assert!(
                add_as("coder", "review", Some(2), "ready", description)
                    .unwrap_err()
                    .contains("analyze plan")
            );
        }
        assert_eq!(add("review", Some(2), "ready").unwrap(), 3);
        statuses(&[1, 2, 3], &["needs_impl", "needs_review", "ready"]);
        let close_error = store::chain_close("reviewer", 1, "done", "pass").unwrap_err();
        assert!(close_error.contains("T3"), "{close_error}");
        change("reviewer", 3, "review", Some("pass")).unwrap();
    }

    #[test]
    fn transition_table_and_published_statuses() {
        use super::transition;
        for current in ["ready", "working"] {
            assert!(transition(1, current, "needs_impl").is_ok());
            assert!(transition(2, current, "needs_review").is_ok());
            assert!(transition(1, current, "needs_review").is_err());
            assert!(transition(3, current, "needs_impl").is_err());
        }
        for status in ["needs_impl", "needs_review"] {
            assert_eq!(store::norm(status), Some(status));
        }
        use clap::CommandFactory;
        let mut cli = crate::cli::Cli::command();
        assert!(
            cli.render_long_help()
                .to_string()
                .contains(store::STATUS_LIST)
        );
        let mut wait = cli.find_subcommand("wait").unwrap().clone();
        assert!(
            wait.render_long_help()
                .to_string()
                .contains(store::STATUS_LIST)
        );
        for (schema, field) in [
            (schemars::schema_for!(crate::server::UpdateArgs), "status"),
            (schemars::schema_for!(crate::server::ListArgs), "status"),
            (schemars::schema_for!(crate::server::WaitArgs), "status"),
        ] {
            assert!(
                schema.as_value()["properties"][field]["description"]
                    .as_str()
                    .unwrap()
                    .contains(store::STATUS_LIST)
            );
        }
    }

    // A child test process owns its environment and board. No unsafe set_var or
    // default-board access in cargo test's multithreaded parent.
    #[test]
    fn probe() {
        let Ok(mode) = std::env::var("AGENTBOARD_CHAIN_PROBE") else {
            return;
        };
        match mode.as_str() {
            "close" => {
                seed();
                store::chain_close("reviewer", 1, "done", "pass").unwrap();
            }
            "inspect" => {
                statuses(&[1, 2, 3], &["needs_impl", "needs_review", "review"]);
                for id in 1..=3 {
                    assert!(!store::get(id, true).unwrap().contains("chain_close"));
                }
                store::report("reviewer", 3, "").unwrap();
                assert!(
                    store::chain_close("reviewer", 1, "done", "")
                        .unwrap_err()
                        .contains("no verdict")
                );
                store::chain_close("reviewer", 1, "done", "pass").unwrap();
                statuses(&[1, 2, 3], &["done"; 3]);
                assert!(
                    change("reviewer", 2, "ready", None)
                        .unwrap_err()
                        .contains("closed chain is final")
                );
            }
            "publish" => {
                for status in ["needs_review", "review"] {
                    for reporter in ["manager", "reviewer", "coder"] {
                        let root = add_as("manager", "analyze", None, "ready", "").unwrap();
                        change("analyzer", root, "needs_impl", Some("plan")).unwrap();
                        let implementation =
                            add_as("manager", "implement", None, "ready", "").unwrap();
                        store::claim("coder", Some(implementation), 60)
                            .unwrap_or_else(|e| panic!("{e}"));
                        let brief =
                            format!("Read the forthcoming diff against analyze plan T{root}");
                        let review = add_as(
                            reporter,
                            "prebuilt review",
                            Some(implementation),
                            "ready",
                            &brief,
                        )
                        .unwrap();
                        store::set_parent("manager", implementation, Some(root)).unwrap();
                        store::diff_add("coder", implementation, "patch", None).unwrap();
                        let before = stable_record(review);
                        let result = change("coder", implementation, status, Some("publish"));
                        if reporter != "coder" {
                            let error = result.unwrap_err();
                            assert!(
                                error.contains("review creation refused: reporter")
                                    && error.contains(reporter),
                                "{error}"
                            );
                            assert_eq!(stable_record(review), before);
                            statuses(&[implementation, review], &["working", "ready"]);
                        } else {
                            result.unwrap();
                            let record = store::get(review, true).unwrap();
                            assert!(record.contains("reporter: coder"));
                            // A missing plan cannot be silently repaired by publication.
                            edit_description(review, "no plan");
                            change("coder", implementation, "working", None).unwrap();
                            store::diff_add("coder", implementation, "next patch", None).unwrap();
                            assert!(
                                change("coder", implementation, status, None)
                                    .unwrap_err()
                                    .contains("analyze plan")
                            );
                            edit_description(review, &brief);
                            change("coder", implementation, status, None).unwrap();
                            let record = store::get(review, true).unwrap();
                            assert!(
                                record.contains(&format!("Analyze plan T{root}"))
                                    && record.contains("reporter: coder")
                            );
                            assert_eq!(
                                store::children(crate::store::Ref::Task(implementation))
                                    .unwrap()
                                    .lines()
                                    .count(),
                                1
                            );
                            assert!(
                                add_as("coder", "duplicate", Some(implementation), "ready", &brief)
                                    .unwrap_err()
                                    .contains("only one child")
                            );
                            // Reviewer use must also reject a subsequently damaged brief.
                            edit_description(review, "no plan");
                            assert!(
                                change("reviewer", review, "review", None)
                                    .unwrap_err()
                                    .contains("analyze plan")
                            );
                        }
                    }
                }
            }
            "guards" => {
                seed();
                let loose = add("loose", None, "ready").unwrap();
                for parent in [1, 2] {
                    let before = stable_record(parent);
                    assert!(
                        add("duplicate", Some(parent), "ready")
                            .unwrap_err()
                            .contains("only one child")
                    );
                    assert!(
                        store::set_parent("manager", loose, Some(parent))
                            .unwrap_err()
                            .contains("only one child")
                    );
                    assert_eq!(stable_record(parent), before);
                    let before_count = stable_record(parent);
                    assert!(
                        change("reviewer", parent, "needs_fix", Some("invalid rejection"))
                            .unwrap_err()
                            .contains("only the review row")
                    );
                    assert_eq!(stable_record(parent), before_count);
                }
                store::set_parent("manager", 2, Some(1)).unwrap(); // same child is allowed
                store::set_parent("manager", 3, Some(2)).unwrap();
                store::chain_close("reviewer", 1, "done", "pass").unwrap();
                for parent in [1, 2] {
                    assert!(
                        add("closed duplicate", Some(parent), "ready")
                            .unwrap_err()
                            .contains("only one child")
                    );
                    assert!(
                        store::set_parent("manager", loose, Some(parent))
                            .unwrap_err()
                            .contains("only one child")
                    );
                }
                let legacy = add("legacy root", None, "ready").unwrap();
                assert!(
                    add("bad legacy status", Some(legacy), "needs_review")
                        .unwrap_err()
                        .contains("cannot add at needs_review")
                );
                assert!(
                    add("plan shortcut", None, "needs_impl")
                        .unwrap_err()
                        .contains("cannot add at needs_impl")
                );
                // Legacy branching stays legal; opting that fork into the new protocol does not.
                add("legacy child one", Some(legacy), "ready").unwrap();
                add("legacy child two", Some(legacy), "ready").unwrap();
                assert!(
                    change("analyzer", legacy, "needs_impl", Some("plan"))
                        .unwrap_err()
                        .contains("only one child")
                );
                let root = add("new root", None, "ready").unwrap();
                change("analyzer", root, "needs_impl", Some("plan")).unwrap();
                assert!(
                    store::set_parent("manager", legacy, Some(root))
                        .unwrap_err()
                        .contains("only one child")
                );
                assert!(
                    add("bad new status", Some(root), "needs_review")
                        .unwrap_err()
                        .contains("cannot add at needs_review")
                );
                let implementation = add("new implementation", Some(root), "ready").unwrap();
                assert!(
                    change("coder", implementation, "needs_review", None)
                        .unwrap_err()
                        .contains("no diff")
                );
                store::diff_add("coder", implementation, "valid patch", None).unwrap();
                change("coder", implementation, "needs_review", None).unwrap();
                statuses(&[root, implementation], &["needs_impl", "needs_review"]);
            }
            "rejection" => {
                seed();
                for (index, current) in ["ready", "working", "ready", "working"].iter().enumerate()
                {
                    change("reviewer", 3, current, None).unwrap();
                    let result = change("reviewer", 3, "needs_fix", Some("rework required"));
                    if index == 3 {
                        assert!(result.unwrap_err().contains("reopen cap 3"));
                        statuses(&[1, 2, 3], &["not_done"; 3]);
                        assert!(
                            change("manager", 2, "ready", None)
                                .unwrap_err()
                                .contains("closed chain is final")
                        );
                    } else {
                        result.unwrap();
                        statuses(&[1, 2, 3], &["needs_impl", "needs_fix", "needs_fix"]);
                        assert!(
                            store::get(2, true)
                                .unwrap()
                                .contains(&format!("reopens: {}/3", index + 1))
                        );
                        resubmit("coder", 2, "rework patch");
                        let diffs = store::diff_list(crate::store::Ref::Task(2)).unwrap();
                        assert!(
                            diffs.contains("round 1") && diffs.contains("round 2"),
                            "{diffs}"
                        );
                        let children = store::children(crate::store::Ref::Task(2)).unwrap();
                        assert_eq!(children.lines().count(), 1, "{children}");
                        assert!(
                            add("second review", Some(2), "ready")
                                .unwrap_err()
                                .contains("only one child")
                        );
                        let review = store::get(3, true).unwrap();
                        assert!(review.contains("Analyze plan T1"));
                    }
                }
            }
            "workflow" => {
                seed();
                assert!(
                    store::chain_close("coder", 1, "done", "pass")
                        .unwrap_err()
                        .contains("newest diff")
                );
                assert!(
                    store::chain_close("", 1, "done", "pass")
                        .unwrap_err()
                        .contains("identity")
                );
                assert!(store::chain_close("reviewer", 1, "ready", "pass").is_err());
                let error = add("forbidden fix", Some(3), "ready").unwrap_err();
                assert!(
                    error.contains("a review row takes no child; reopen the implement row"),
                    "{error}"
                );
                let loose = add("loose", None, "ready").unwrap();
                assert!(
                    store::set_parent("manager", loose, Some(3))
                        .unwrap_err()
                        .contains("reopen the implement row")
                );
                store::chain_close("reviewer", 1, "done", "pass").unwrap();
                statuses(&[1, 2, 3], &["done"; 3]);
                assert!(
                    store::chain_reopen("reviewer", 1, "reconsider")
                        .unwrap_err()
                        .contains("recorded the close")
                );
                assert!(
                    store::chain_reopen("manager", 1, " ")
                        .unwrap_err()
                        .contains("reason")
                );
                store::chain_reopen("manager", 1, "new evidence").unwrap();
                statuses(&[1, 2, 3], &["needs_impl", "needs_impl", "needs_review"]);
                assert!(
                    store::get(2, true)
                        .unwrap()
                        .contains("manager chain_reopen")
                );
                assert!(store::get(2, true).unwrap().contains("new evidence"));
                store::note("second-coder", 2, "pending implementation").unwrap();
                assert!(
                    change("second-coder", 2, "needs_review", None)
                        .unwrap_err()
                        .contains("cannot move from")
                );
                store::claim("coder", Some(2), 60).unwrap_or_else(|e| panic!("{e}"));
                assert!(
                    change("second-coder", 2, "needs_review", None)
                        .unwrap_err()
                        .contains("no diff")
                );
                store::note("second-coder", 2, "working on the reopen").unwrap();
                store::note("reviewer", 3, "awaiting next diff").unwrap();
                // An independent newest author, not a named role, determines close authority.
                store::diff_add("second-coder", 2, "patch two", None).unwrap();
                change("second-coder", 2, "needs_review", None).unwrap();
                change("reviewer", 3, "review", None).unwrap();
                assert!(
                    store::chain_close("manager", 1, "done", "")
                        .unwrap_err()
                        .contains("no verdict")
                );
                assert!(store::chain_close("second-coder", 1, "not_done", "failed").is_err());
                store::chain_close("coder", 1, "not_done", "failed").unwrap();
                statuses(&[1, 2, 3], &["not_done"; 3]);
                store::chain_reopen("manager", 1, "retry").unwrap();
                resubmit("coder", 2, "patch three");
                change("reviewer", 3, "review", None).unwrap();
                assert!(
                    store::chain_close("manager", 1, "done", "")
                        .unwrap_err()
                        .contains("no verdict")
                );
                change("reviewer", 3, "needs_fix", Some("bug")).unwrap();
                statuses(&[1, 2, 3], &["needs_impl", "needs_fix", "needs_fix"]);
                assert!(store::get(2, true).unwrap().contains("reopens: 3/3"));
                resubmit("coder", 2, "patch four");
                change("reviewer", 3, "review", Some("pass")).unwrap();
                store::chain_close("reviewer", 1, "done", "pass").unwrap();
                assert!(
                    store::chain_reopen("manager", 1, "fourth")
                        .unwrap_err()
                        .contains("reopen cap 3")
                );
                statuses(&[1, 2, 3], &["done"; 3]);
                // Legacy implement -> review -> fix remains usable without new statuses.
                let old = add("legacy", None, "review").unwrap();
                let review = add("legacy review", Some(old), "ready").unwrap();
                let fix = add("legacy fix", Some(review), "ready").unwrap();
                change("reviewer", review, "needs_fix", Some("fix it")).unwrap();
                change("coder", fix, "done", Some("fixed")).unwrap();
                change("reviewer", review, "done", Some("pass")).unwrap();
                change("manager", old, "done", Some("verified")).unwrap();
                statuses(&[old, review, fix], &["done"; 3]);
            }
            _ => panic!("unknown probe"),
        }
    }

    struct Fixture {
        dir: PathBuf,
        server: Option<Child>,
        port: u16,
        surreal: PathBuf,
    }
    impl Fixture {
        fn new(label: &str) -> Option<Self> {
            let dir = std::env::temp_dir()
                .join(format!("agentboard-chain-{}-{label}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let surreal = if Command::new("surreal").arg("version").output().is_ok() {
                PathBuf::from("surreal")
            } else {
                PathBuf::from(std::env::var("HOME").unwrap()).join(".surrealdb/surreal")
            };
            let mut result = Self {
                dir,
                server: None,
                port: 0,
                surreal,
            };
            if cfg!(feature = "surreal") {
                if !result.surreal.exists() && result.surreal != Path::new("surreal") {
                    assert!(
                        std::env::var_os("AGENTBOARD_REQUIRE_SURREAL").is_none(),
                        "surreal binary required"
                    );
                    eprintln!("SKIP: chain tests need surrealdb 2.7.x");
                    return None;
                }
                result.port = std::net::TcpListener::bind("127.0.0.1:0")
                    .unwrap()
                    .local_addr()
                    .unwrap()
                    .port();
                result.start();
            }
            Some(result)
        }
        fn start(&mut self) {
            self.server = Some(
                Command::new(&self.surreal)
                    .args([
                        "db",
                        "start",
                        "--no-banner",
                        "--log",
                        "warn",
                        "--bind",
                        &format!("127.0.0.1:{}", self.port),
                        "--username",
                        "root",
                        "--password",
                        "root",
                        &format!("surrealkv://{}", self.dir.join("server").display()),
                    ])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap(),
            );
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if std::net::TcpStream::connect(("127.0.0.1", self.port)).is_ok() {
                    break;
                }
                assert!(Instant::now() < deadline, "scratch server did not start");
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        fn child(&self, mode: &str) -> Command {
            let mut cmd = Command::new(std::env::current_exe().unwrap());
            cmd.args(["--exact", "store::chain::tests::probe", "--nocapture"])
                .env("AGENTBOARD_CHAIN_PROBE", mode)
                .env("AGENTBOARD_DB", self.dir.join("board"))
                .env("AGENTBOARD_SURREAL_URL", format!("127.0.0.1:{}", self.port))
                .env("AGENTBOARD_SURREAL_USER", "root")
                .env("AGENTBOARD_SURREAL_PASS", "root");
            cmd
        }
        fn run(&self, mode: &str) {
            let out = self.child(mode).output().unwrap();
            assert!(
                out.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Some(mut server) = self.server.take() {
                let _ = server.kill();
                let _ = server.wait();
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
    #[test]
    fn new_workflow_and_legacy_workflow() {
        let Some(fixture) = Fixture::new("workflow") else {
            return;
        };
        fixture.run("workflow");
    }
    #[test]
    fn publishing_prebuilt_review_validates_persisted_reporter_and_plan() {
        let Some(fixture) = Fixture::new("publish") else {
            return;
        };
        fixture.run("publish");
    }
    #[test]
    fn new_chain_shape_and_status_guards() {
        let Some(fixture) = Fixture::new("guards") else {
            return;
        };
        fixture.run("guards");
    }
    #[test]
    fn reviewer_rejection_reopens_implement_and_caps() {
        let Some(fixture) = Fixture::new("rejection") else {
            return;
        };
        fixture.run("rejection");
    }
    #[test]
    fn killed_chain_close_is_atomic() {
        let Some(mut fixture) = Fixture::new("crash") else {
            return;
        };
        let marker = fixture.dir.join("paused");
        let mut child = fixture
            .child("close")
            .env("AGENTBOARD_CHAIN_PAUSE", &marker)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !marker.exists() {
            assert!(
                child.try_wait().unwrap().is_none(),
                "close child exited before interruption point"
            );
            assert!(
                Instant::now() < deadline,
                "close never reached interruption point"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        if let Some(mut server) = fixture.server.take() {
            // The server transaction pauses after its first row update. Crash
            // the DB too: killing only the client need not cancel server work.
            std::thread::sleep(Duration::from_millis(400));
            server.kill().unwrap();
            server.wait().unwrap();
        }
        child.kill().unwrap();
        assert!(!child.wait().unwrap().success());
        if cfg!(feature = "surreal") {
            fixture.start();
        }
        fixture.run("inspect");
    }
}
