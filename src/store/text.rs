//! Backend-neutral rows and the pure text that renders them. Both the SQLite and the
//! SurrealDB store load plain values (the row structs below), then call these
//! functions, so `ls`, `get`, `children`, `epic ...` and `brief` print the same text
//! whatever the backend. Nothing here touches a database or a driver type; a field
//! whose type differs between backends is normalised where the row is loaded.

use crate::clip::{
    CAP_DESCRIPTION, CAP_DIFF, CAP_EVENT, CAP_GOAL, CAP_NOTE, CAP_REPORT, clip, clip_marked, field,
};
use crate::store::{now_ms, short, short_diff, short_epic};
use std::path::{Component, Path, PathBuf};

/// One durable event line for a move that leaves immediate relatives elsewhere.
pub fn epic_gap(members: &[(i64, Option<i64>)]) -> Option<String> {
    if members.is_empty() {
        return None;
    }
    Some(format!(
        "family spans epics: {}",
        members
            .iter()
            .map(|(id, epic)| format!(
                "{} remains in {}",
                short(*id),
                epic.map_or("none".to_owned(), short_epic)
            ))
            .collect::<Vec<_>>()
            .join("; ")
    ))
}

/// One task as stored.
#[derive(Clone, Debug)]
pub struct TaskRow {
    pub reporter: String,
    pub id: i64,
    pub status: String,
    pub assignee: Option<String>,
    pub needs: Option<String>,
    pub title: String,
    pub description: String,
    pub goal: String,
    /// Absolute project root the work lives in; '' = not repo-bound.
    pub path: String,
    /// Git ref the worker starts on; HEAD = leave the pane's checkout alone.
    pub branch: String,
    pub lease_until: Option<i64>,
    pub updated_at: i64,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub report: Option<String>,
    pub note: Option<String>,
    pub commit_ref: Option<String>,
    pub priority: i64,
    pub epic_id: Option<i64>,
    pub parent_id: Option<i64>,
    /// Moves into `needs_fix` so far; above `MAX_REOPENS` the cap stopped the row.
    pub reopen_count: i64,
}

/// One registry row as `sessions` lists it and `manager status` joins it.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct SessionRow {
    pub name: String,
    pub agent: String,
    pub cwd: Option<String>,
    pub task_id: Option<i64>,
    pub seen: i64,
    pub status: String,
}

/// One diff as stored: what a coder submitted for review, verbatim.
#[derive(Clone, Debug)]
pub struct DiffRow {
    pub id: i64,
    pub task_id: i64,
    /// `reopen_count + 1` of the task when it was recorded.
    pub round: i64,
    pub body: String,
    pub created_at: i64,
    /// `Some(diff)` marks an evidence-only submission for a re-run round: the body
    /// is what was run, never a patch. `None` is a normal diff.
    pub reviews: Option<i64>,
}

/// One round's submission: the row the coder recorded, and the patch a reviewer
/// must read. For a normal diff the two are the same row; for an evidence-only
/// submission `record` is the evidence and `diff` is the earlier patch it names.
#[derive(Clone, Debug)]
pub struct Submission {
    pub record: DiffRow,
    pub diff: DiffRow,
}

impl Submission {
    /// A normal diff is its own submission.
    pub fn patch(d: DiffRow) -> Self {
        Self {
            record: d.clone(),
            diff: d,
        }
    }

    /// Whether this round recorded evidence instead of a new patch.
    pub fn evidence_only(&self) -> bool {
        self.record.reviews.is_some()
    }
}

/// One epic as stored.
#[derive(Clone, Debug)]
pub struct EpicRow {
    pub id: i64,
    pub title: String,
    pub description: String,
    pub goal: String,
    pub status: String,
    pub path: String,
    pub created_at: i64,
    pub completed_at: Option<i64>,
}

/// One event of a task or an epic.
#[derive(Clone, Debug)]
pub struct EventRow {
    pub seq: i64,
    pub actor: String,
    pub kind: String,
    pub body: String,
    pub ts: i64,
}

/// What `brief` shows for one open epic.
pub struct BriefEpic {
    pub id: i64,
    pub title: String,
    pub path: String,
    pub done: i64,
    pub total: i64,
    /// `["2 blocking", "1 needs_fix"]`: only what a manager must act on.
    pub urgent: Vec<String>,
}

/// Everything `brief` prints, already loaded.
pub struct BriefData {
    pub since_min: i64,
    /// Open tasks per status, ordered by status name.
    pub counts: Vec<(String, i64)>,
    pub done_recent: i64,
    pub epics: Vec<BriefEpic>,
    /// Open tasks outside any epic, in display order, at most 25.
    pub rows: Vec<TaskRow>,
    /// Recent completions, newest first, at most 8: (id, report).
    pub dones: Vec<(i64, Option<String>)>,
}

// ---- the review chain ----------------------------------------------------------------

/// Moves into `needs_fix` a row may take; the next one is refused and lands `not_done`.
pub const MAX_REOPENS: i64 = 3;

/// Levels of a parent chain: implement -> review -> fix. A child of a level-3 row is refused.
pub const MAX_CHAIN: usize = 3;

/// A move into `needs_fix` from a row already reopened `count` times: the count it
/// would reach, and whether that is over the cap.
pub fn reopen_after(count: i64) -> (i64, bool) {
    (count + 1, count + 1 > MAX_REOPENS)
}

/// The refusal for a subtree `height` levels deep placed under `parent`, which is at
/// level `parent_level`: the deepest row would reach `parent_level + height`.
pub fn chain_refusal(parent: i64, parent_level: usize, height: usize) -> String {
    format!(
        "chain too deep: {} is level {parent_level} and the new subtree spans {height}, reaching level {} (implement -> review -> fix); no fourth level",
        short(parent),
        parent_level + height
    )
}

/// Whether a capped child moves its parent to `not_done`: only an open parent. Closed
/// work (`done`, `dropped`) and a parent already `not_done` are left alone.
pub fn cascades_to(parent_status: &str) -> bool {
    matches!(
        parent_status,
        "ready" | "working" | "blocking" | "needs_fix" | "review"
    )
}

/// The note on a capped child whose parent was not moved.
pub fn cap_skipped_note(parent: i64, parent_status: &str) -> String {
    format!(
        "reopen cap {MAX_REOPENS} hit; parent {} is {parent_status}, left as it is",
        short(parent)
    )
}

/// The event body of a row the cap stopped: the caller's body is kept after it.
pub fn cap_note(body: &str) -> String {
    format!("reopen cap {MAX_REOPENS} hit: not needs_fix, now not_done\n{body}")
}

/// The event body of a parent whose child the cap stopped.
pub fn cap_cascade_note(child: i64) -> String {
    format!(
        "{} hit the reopen cap {MAX_REOPENS}; this row is now not_done",
        short(child)
    )
}

/// The one-line `Err` a capped move returns, after the row has landed `not_done`.
pub fn cap_refusal(id: i64) -> String {
    format!(
        "{} reopen cap {MAX_REOPENS} hit: the row is now not_done, not needs_fix",
        short(id)
    )
}

// ---- the review gate and the auto-filed review row ---------------------------------

/// The refusal for a move into `review` with no submission since the last claim.
pub fn gate_refusal(id: i64) -> String {
    let t = short(id);
    format!(
        "{t} cannot move to review: no diff or evidence-only submission recorded since it was last claimed; record one first: agentboard diff add {t} --body-file - (diff_add id={t}); a round whose tree did not change records evidence for the newest patch of this row: agentboard diff add {t} --evidence-only --reviews D<n> --body-file - (diff_add id={t} reviews=D<n>)"
    )
}

/// The refusal when `--evidence-only` names no diff.
pub fn evidence_needs_reviews(id: i64) -> String {
    let t = short(id);
    format!(
        "diff add --evidence-only {t}: name the newest patch of this row with --reviews D<n> (MCP: reviews=\"D<n>\")"
    )
}

/// The refusal when `--reviews` arrives without `--evidence-only`.
pub fn reviews_needs_evidence(id: i64) -> String {
    let t = short(id);
    format!(
        "diff add {t} --reviews: --reviews belongs to an evidence-only submission; pass --evidence-only too (MCP: the reviews argument already means evidence-only), or omit it for a normal diff"
    )
}

/// The refusal when an evidence-only submission names another evidence-only row.
pub fn evidence_not_a_patch(task: i64, diff: i64) -> String {
    format!(
        "diff add --evidence-only {}: {} is evidence-only, not a patch; name the newest patch of this row with --reviews D<n>",
        short(task),
        short_diff(diff)
    )
}

/// The refusal when an evidence-only submission names a diff recorded on another row.
pub fn evidence_not_on_row(task: i64, diff: i64, owner: i64) -> String {
    format!(
        "diff add --evidence-only {}: {} is recorded on {}, not on {}; evidence names the newest patch of the submitting row (same row, no re-parent)",
        short(task),
        short_diff(diff),
        short(owner),
        short(task)
    )
}

/// The refusal when an evidence-only submission names a superseded patch.
pub fn evidence_stale(task: i64, diff: i64, newest: i64) -> String {
    format!(
        "diff add --evidence-only {}: {} is not the newest patch of {}; the newest patch is {} (name it with --reviews D<n>)",
        short(task),
        short_diff(diff),
        short(task),
        short_diff(newest)
    )
}

/// The title of the review row filed for an implement row.
pub fn review_title(implement_title: &str) -> String {
    format!("Review {}", clip(implement_title, 80))
}

/// The description of a new review row: the implement row, the submission and the
/// report verbatim. The first line names which case this round is, so the reviewer
/// never has to guess which row to read.
pub fn review_description(implement: &TaskRow, report: Option<&str>, s: &Submission) -> String {
    let d = short_diff(s.diff.id);
    let report = report.filter(|r| !r.trim().is_empty()).unwrap_or("(none)");
    if s.evidence_only() {
        return format!(
            "Review of {} \"{}\" (round {}).\nNo new diff this round: {} is evidence-only for {d}; the diff under review is {d}; read only {d}: `agentboard diff get {d} --full` or diff_get id={d} full=true.\n\nEvidence (verbatim):\n{}\n\nImplement report (verbatim):\n{report}",
            short(implement.id),
            implement.title,
            s.record.round,
            short_diff(s.record.id),
            s.record.body
        );
    }
    format!(
        "Review of {} \"{}\" (round {}).\nNewest diff: {d}; read only this one: `agentboard diff get {d} --full` or diff_get id={d} full=true.\n\nImplement report (verbatim):\n{report}",
        short(implement.id),
        implement.title,
        s.diff.round
    )
}

/// What the reviewer does with a review row.
pub fn review_goal(implement: i64, s: &Submission) -> String {
    let d = short_diff(s.diff.id);
    let which = if s.evidence_only() {
        format!(
            "Read only {d}, the diff under review ({} is evidence-only this round; it is not a patch).",
            short_diff(s.record.id)
        )
    } else {
        format!(
            "Read only the newest patch of {} (now {d}; a resubmission adds a newer one).",
            short(implement)
        )
    };
    format!(
        "{which} Give the verdict in this row's report. Findings: file a fix row under this row (parent = this row). Clean: close this row done. Close the fix row and this review row yourself; never close {}, the manager does.",
        short(implement)
    )
}

/// The note on an existing review row when its implement row is submitted again.
pub fn review_resubmitted_note(implement: i64, s: &Submission) -> String {
    if s.evidence_only() {
        return format!(
            "{} submitted again: no new diff; {} is evidence-only for {} (round {}); the diff under review is {}",
            short(implement),
            short_diff(s.record.id),
            short_diff(s.diff.id),
            s.diff.round,
            short_diff(s.diff.id)
        );
    }
    format!(
        "{} submitted again: newest diff {} (round {})",
        short(implement),
        short_diff(s.diff.id),
        s.diff.round
    )
}

/// The note on the implement row naming the review row filed for it.
pub fn review_filed_note(review: i64) -> String {
    format!("review row {} filed", short(review))
}

// ---- time and status --------------------------------------------------------------

pub fn code(status: &str) -> &str {
    match status {
        "ready" | "triage" => "rdy",
        "working" | "running" => "wrk",
        "blocking" | "blocked" => "blk",
        "not_done" => "nd",
        "needs_fix" => "fix",
        "review" => "rev",
        "done" => "don",
        "dropped" => "drp",
        other => other,
    }
}

pub fn ago(ts_ms: i64) -> String {
    let d = (now_ms() - ts_ms).max(0) / 1000;
    match d {
        0..=59 => format!("{d}s"),
        60..=3599 => format!("{}m", d / 60),
        3600..=86_399 => format!("{}h", d / 3600),
        _ => format!("{}d", d / 86_400),
    }
}

/// One-line timestamp for listings: "13:32Z" style is locale-safe enough here.
pub fn stamp(ts_ms: i64) -> String {
    let secs = ts_ms / 1000;
    let (h, m) = ((secs / 3600) % 24, (secs / 60) % 60);
    format!("{h:02}:{m:02}")
}

pub fn when(ts: i64) -> String {
    format!("{} ({}Z)", ago(ts), stamp(ts))
}

/// Canonical status order for summaries: most urgent first.
const STATUS_ORDER: [&str; 8] = [
    "blocking",
    "not_done",
    "needs_fix",
    "working",
    "review",
    "ready",
    "done",
    "dropped",
];

/// Display rank for `ls`: most urgent first. (SQL orders in the sqlite store; the
/// surreal store sorts with this.)
#[cfg(feature = "surreal")]
pub fn list_rank(status: &str) -> u8 {
    match status {
        "blocking" => 0,
        "not_done" => 1,
        "needs_fix" => 2,
        "working" => 3,
        "review" => 4,
        "ready" => 5,
        _ => 6,
    }
}

/// Display rank for `brief` (no epic): ready and everything else share the last rank.
#[cfg(feature = "surreal")]
pub fn brief_rank(status: &str) -> u8 {
    match status {
        "blocking" => 0,
        "not_done" => 1,
        "needs_fix" => 2,
        "working" => 3,
        "review" => 4,
        _ => 5,
    }
}

// ---- change diffs for edit events --------------------------------------------------

/// The value to store for a text field an edit may set, noting `name old->new`
/// (character lengths) in `diffs` when it moves.
pub fn text_change<'a>(
    diffs: &mut Vec<String>,
    name: &str,
    old: &'a str,
    new: Option<&'a str>,
) -> &'a str {
    let value = new.unwrap_or(old);
    if value != old {
        diffs.push(format!(
            "{name} {}->{}",
            old.chars().count(),
            value.chars().count()
        ));
    }
    value
}

/// Like `text_change` for the short fields (title, assignee, path): the event
/// records `name "old"->"new"`, each clipped to 80 chars, so a same-length
/// change is visible and the old text is recoverable.
pub fn value_change<'a>(
    diffs: &mut Vec<String>,
    name: &str,
    old: &'a str,
    new: Option<&'a str>,
) -> &'a str {
    let value = new.unwrap_or(old);
    if value != old {
        diffs.push(format!(
            "{name} \"{}\"->\"{}\"",
            clip(old, 80),
            clip(value, 80)
        ));
    }
    value
}

/// `title, goal` out of `["title 12->15", "goal 3->9"]`.
pub fn field_names(diffs: &[String]) -> String {
    let names: Vec<&str> = diffs
        .iter()
        .filter_map(|d| d.split_whitespace().next())
        .collect();
    names.join(", ")
}

// ---- paths -------------------------------------------------------------------------

/// Absolute, lexically clean project path: `~` and `~/x` expand against `home`,
/// a relative path resolves against `cwd`, `.`/`..` collapse, no trailing slash.
/// The path need not exist (an epic may be written before the repo).
pub fn normalize_path(raw: &str, cwd: &Path, home: Option<&str>) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    let expanded: PathBuf = match (raw, home) {
        ("~", Some(h)) => PathBuf::from(h),
        (r, Some(h)) if r.starts_with("~/") => Path::new(h).join(&r[2..]),
        (r, _) => PathBuf::from(r),
    };
    let abs = if expanded.is_absolute() {
        expanded
    } else {
        cwd.join(expanded)
    };
    let mut out = PathBuf::new();
    for c in abs.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out.to_string_lossy().into_owned()
}

/// The marker `epic get` and `brief` show for a path that is not on disk.
pub fn path_note(path: &str) -> &'static str {
    if !path.is_empty() && !Path::new(path).exists() {
        " (does not exist)"
    } else {
        ""
    }
}

/// The branch a task starts on: trimmed, and `''` means `HEAD` (start on
/// whatever the pane has checked out). A name git could not check out is refused
/// here, so the CLI and the MCP server share one rule and one message.
pub fn norm_branch(raw: &str) -> Result<String, String> {
    let t = raw.trim();
    if t.is_empty() {
        return Ok("HEAD".to_owned());
    }
    let bad = t.starts_with('-')
        || t.chars()
            .any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c));
    if bad {
        return Err(format!(
            "bad branch {raw:?}: a git ref cannot start with '-' or hold whitespace, control characters or any of ~^:?*[\\"
        ));
    }
    Ok(t.to_owned())
}

/// A stored branch for display: an empty value (a row written before branches
/// existed) reads as `HEAD`.
pub fn shown_branch(branch: &str) -> &str {
    if branch.is_empty() { "HEAD" } else { branch }
}

// ---- tasks -------------------------------------------------------------------------

/// The refusal for a task somebody holds: who, the lease, and the manager's way out.
pub fn held_text(r: &TaskRow) -> String {
    format!(
        "{} {} by {}{} — manager: agentboard status {} ready",
        short(r.id),
        r.status,
        r.assignee.as_deref().unwrap_or("nobody"),
        r.lease_until.map_or(String::new(), |l| format!(
            " (lease {}m left, until {}Z)",
            ((l - now_ms()).max(0) + 59_999) / 60_000,
            stamp(l)
        )),
        short(r.id)
    )
}

/// The `ls` listing: one line per task, or `(no tasks)`.
pub fn list_text(rows: &[TaskRow]) -> String {
    if rows.is_empty() {
        return "(no tasks)".to_owned();
    }
    let mut out = String::new();
    for r in rows {
        let who = r.assignee.as_deref().unwrap_or("-");
        let flag = if r.needs.as_deref() == Some("input") {
            " [input]"
        } else {
            ""
        };
        out.push_str(&format!(
            "{} {} {:<9} {:>4} {}{}\n",
            short(r.id),
            code(&r.status),
            who,
            ago(r.updated_at),
            clip(&r.title, 64),
            flag
        ));
    }
    out.trim_end().to_owned()
}

/// One compact line per child, ordered by id: id, status, assignee, age, title.
pub fn children_text(label: &str, rows: &[TaskRow]) -> String {
    if rows.is_empty() {
        return format!("{label} has no children");
    }
    let mut out = String::new();
    for r in rows {
        out.push_str(&format!(
            "{} {} {:<9} {:>4} {}\n",
            short(r.id),
            code(&r.status),
            r.assignee.as_deref().unwrap_or("-"),
            ago(r.updated_at),
            clip(&r.title, 64)
        ));
    }
    out.trim_end().to_owned()
}

/// `children: 4 (2 done, 1 working, 1 needs_fix)`, or None when there are none.
pub fn summary_text(label: &str, counts: &[(String, i64)]) -> Option<String> {
    let total: i64 = counts.iter().map(|(_, n)| n).sum();
    if total == 0 {
        return None;
    }
    let mut parts = Vec::new();
    for st in STATUS_ORDER {
        if let Some((_, n)) = counts.iter().find(|(s, _)| s == st) {
            parts.push(format!("{n} {st}"));
        }
    }
    Some(format!("{label}: {total} ({})", parts.join(", ")))
}

/// The last events as an `events:` block, oldest first. `newest_first` is as
/// loaded (at most 8, newest first); bodies are clipped with a marker unless `full`.
pub fn events_text(newest_first: &[EventRow], full: bool, hint: &str) -> String {
    if newest_first.is_empty() {
        return String::new();
    }
    let mut out = String::from("events:\n");
    for e in newest_first.iter().rev() {
        let text = if full {
            e.body.clone()
        } else {
            clip_marked(&e.body, CAP_EVENT, hint)
        };
        out.push_str(&format!(
            "  #{} {} {} {}: {text}\n",
            e.seq,
            e.actor,
            e.kind,
            ago(e.ts)
        ));
    }
    out
}

// ---- diffs -------------------------------------------------------------------------

/// The refusal for a diff with nothing in it.
pub const EMPTY_DIFF: &str = "diff add: the body is empty (whitespace only); pass the diff text with --body, --body-file PATH or --body-file -";

/// The refusal for evidence-only with nothing in it.
pub const EMPTY_EVIDENCE: &str = "diff add --evidence-only: the evidence is empty (whitespace only); pass what you ran and its output with --body, --body-file PATH or --body-file -, and name the diff under review with --reviews D<n>";

/// The event body of a recorded diff: the row and its size, never the diff itself.
pub fn diff_event(id: i64, round: i64, body: &str, reviews: Option<i64>) -> String {
    let what = match reviews {
        Some(d) => format!("round {round} evidence-only -> {}", short_diff(d)),
        None => format!("round {round}"),
    };
    format!("{} {what} {} lines", short_diff(id), body.lines().count())
}

/// `D12 round 2 3m 184 lines`; with `task`, `D12 T5 round 2 ...` (an epic's listing).
/// An evidence-only row says so and names the diff it re-runs.
pub fn diff_line(d: &DiffRow, task: bool) -> String {
    let of = if task {
        format!(" {}", short(d.task_id))
    } else {
        String::new()
    };
    let what = match d.reviews {
        Some(r) => format!("evidence-only -> {}", short_diff(r)),
        None => format!("round {}", d.round),
    };
    format!(
        "{}{of} {what} {} {} lines",
        short_diff(d.id),
        ago(d.created_at),
        d.body.lines().count()
    )
}

/// The `diff ls` listing, newest first: one line per row, or `(no diffs)`.
pub fn diff_list_text(rows: &[DiffRow], task: bool) -> String {
    if rows.is_empty() {
        return "(no diffs)".to_owned();
    }
    rows.iter()
        .map(|d| diff_line(d, task))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `diff get`: a header line, then the body clipped with a marker unless `full`.
/// An evidence-only row is labelled as such and points at the diff it re-runs, so it
/// can never be read as a patch.
pub fn diff_text(d: &DiffRow, full: bool) -> String {
    let tag = short_diff(d.id);
    let head = match d.reviews {
        Some(r) => format!(
            "{tag} {} round {} evidence-only, reviews {} {}; not a diff: the patch under review is {} (`agentboard diff get {} --full`)",
            short(d.task_id),
            d.round,
            short_diff(r),
            when(d.created_at),
            short_diff(r),
            short_diff(r)
        ),
        None => format!(
            "{tag} {} round {} {}",
            short(d.task_id),
            d.round,
            when(d.created_at)
        ),
    };
    let body = if full {
        d.body.clone()
    } else {
        clip_marked(
            &d.body,
            CAP_DIFF,
            &format!(
                "full text: `agentboard diff get {tag} --full` or diff_get id={tag} full=true"
            ),
        )
    };
    format!("{head}\n{body}")
}

/// `get`: the task, its epic line, the subtask summary and the event block.
/// `full` returns description, goal, report and note unclipped and verbatim.
pub fn get_text(
    r: &TaskRow,
    epic: Option<&EpicRow>,
    summary: Option<&str>,
    newest_diff: Option<&DiffRow>,
    events: &str,
    full: bool,
) -> String {
    let mut out = format!(
        "{} {} {} priority {}\n",
        short(r.id),
        r.status,
        r.assignee.as_deref().unwrap_or("-"),
        r.priority
    );
    out.push_str(&format!(
        "reporter: {}\n",
        if r.reporter.trim().is_empty() {
            "unknown (filed before this field existed)"
        } else {
            &r.reporter
        }
    ));
    out.push_str(&format!("title: {}\n", r.title));
    if r.path.is_empty() {
        out.push_str("path: -\n");
    } else {
        out.push_str(&format!("path: {}{}\n", r.path, path_note(&r.path)));
    }
    out.push_str(&format!("branch: {}\n", shown_branch(&r.branch)));
    if let (Some(e), Some(epic)) = (r.epic_id, epic) {
        out.push_str(&format!(
            "epic: {} {}{}\n",
            short_epic(e),
            clip(&epic.title, 60),
            if epic.path.is_empty() {
                String::new()
            } else {
                format!(" [{}]", epic.path)
            }
        ));
    }
    if let Some(p) = r.parent_id {
        out.push_str(&format!("parent: {}\n", short(p)));
    }
    if r.reopen_count > MAX_REOPENS {
        out.push_str(&format!(
            "reopens: {}/{MAX_REOPENS} (cap hit, stopped)\n",
            r.reopen_count
        ));
    } else if r.reopen_count > 0 {
        out.push_str(&format!("reopens: {}/{MAX_REOPENS}\n", r.reopen_count));
    }
    out.push_str(&format!("created: {}\n", when(r.created_at)));
    if let Some(s) = r.started_at {
        out.push_str(&format!("started: {}\n", when(s)));
    }
    if let Some(c) = r.completed_at {
        out.push_str(&format!("completed: {}\n", when(c)));
    }
    if let Some(u) = r.lease_until {
        let left = u - now_ms();
        if left > 0 {
            out.push_str(&format!("lease: {}m left\n", left / 60_000));
        }
    }
    if let Some(line) = summary {
        out.push_str(line);
        out.push('\n');
    }
    if let Some(d) = newest_diff {
        out.push_str(&format!("diff: {}\n", diff_line(d, false)));
    }
    let t = short(r.id);
    out.push_str(&field(
        "description",
        &r.description,
        CAP_DESCRIPTION,
        full,
        &t,
    ));
    out.push_str(&field("goal", &r.goal, CAP_GOAL, full, &t));
    out.push_str(&field(
        "report",
        r.report.as_deref().unwrap_or(""),
        CAP_REPORT,
        full,
        &t,
    ));
    out.push_str(&field(
        "note",
        r.note.as_deref().unwrap_or(""),
        CAP_NOTE,
        full,
        &t,
    ));
    if let Some(c) = &r.commit_ref {
        out.push_str(&format!("commit: {c}\n"));
    }
    out.push_str(events);
    out.trim_end().to_owned()
}

/// The one line a claim hands the worker about where the work lives, or "" when
/// the task is not repo-bound: there is nothing for it to check out.
pub fn workdir_text(r: &TaskRow) -> String {
    if r.path.is_empty() {
        String::new()
    } else {
        format!("path: {} [branch {}]\n", r.path, shown_branch(&r.branch))
    }
}

/// The hint a clipped task event names.
pub fn task_hint(id: i64) -> String {
    let t = short(id);
    format!("`agentboard get {t} --full` or task_get id={t} full=true")
}

// ---- epics -------------------------------------------------------------------------

/// One `epic ls` line.
pub fn epic_list_line(e: &EpicRow, want: &str, done: i64, total: i64) -> String {
    format!(
        "{} {want} {done}/{total} done {}{} {}\n",
        short_epic(e.id),
        if e.path.is_empty() { "-" } else { &e.path },
        path_note(&e.path),
        clip(&e.title, 60)
    )
}

/// `epic get`: `full` returns description and goal unclipped, verbatim.
pub fn epic_text(e: &EpicRow, summary: Option<&str>, events: &str, full: bool) -> String {
    let tag = short_epic(e.id);
    let mut out = format!("{tag} {}\ntitle: {}\n", e.status, e.title);
    if e.path.is_empty() {
        out.push_str("path: -\n");
    } else {
        out.push_str(&format!("path: {}{}\n", e.path, path_note(&e.path)));
    }
    out.push_str(&format!("created: {}\n", when(e.created_at)));
    if let Some(c) = e.completed_at {
        out.push_str(&format!("completed: {}\n", when(c)));
    }
    if let Some(line) = summary {
        out.push_str(line);
        out.push('\n');
    }
    let hint = epic_hint(e.id);
    for (label, value, cap) in [
        ("description", &e.description, CAP_DESCRIPTION),
        ("goal", &e.goal, CAP_GOAL),
    ] {
        if value.is_empty() {
            continue;
        }
        let shown = if full {
            value.clone()
        } else {
            clip_marked(value, cap, &hint)
        };
        out.push_str(&format!("{label}:\n{shown}\n"));
    }
    out.push_str(events);
    out.trim_end().to_owned()
}

/// The hint a clipped epic event names.
pub fn epic_hint(id: i64) -> String {
    let tag = short_epic(id);
    format!("`agentboard epic get {tag} --full` or epic_get id={tag} full=true")
}

// ---- brief -------------------------------------------------------------------------

/// One-screen status: counts + one line per open epic and open task + recent completions.
pub fn brief_text(b: &BriefData) -> String {
    let mut out = String::new();
    let mut head: Vec<String> = b
        .counts
        .iter()
        .map(|(s, n)| format!("{n} {}", code(s)))
        .collect();
    head.push(format!("{} done/{}m", b.done_recent, b.since_min));
    out.push_str(&head.join(", "));
    out.push('\n');
    // One line per open epic: progress, the project path, and only the counts a
    // manager must act on. Its tasks are one `children E<n>` away. Dropped tasks
    // leave the denominator; closing an epic is the manager's call, never automatic.
    for e in &b.epics {
        out.push_str(&format!(
            "EPIC {} {}/{} done{} {}{} {}\n",
            short_epic(e.id),
            e.done,
            e.total,
            if e.urgent.is_empty() {
                String::new()
            } else {
                format!(" [{}]", e.urgent.join(", "))
            },
            if e.path.is_empty() { "-" } else { &e.path },
            path_note(&e.path),
            clip(&e.title, 60)
        ));
    }
    for r in &b.rows {
        let wait = if r.needs.as_deref() == Some("input") {
            "ON INPUT"
        } else {
            ""
        };
        out.push_str(&format!(
            "{} {} {} {} {}{}\n",
            short(r.id),
            code(&r.status),
            r.assignee.as_deref().unwrap_or("-"),
            ago(r.updated_at),
            clip(&r.title, 60),
            if wait.is_empty() {
                String::new()
            } else {
                format!(" [{wait}]")
            }
        ));
    }
    if !b.dones.is_empty() {
        out.push_str("done:\n");
        for (id, res) in &b.dones {
            out.push_str(&format!(
                "  {} {}\n",
                short(*id),
                clip(res.as_deref().unwrap_or(""), 80)
            ));
        }
    }
    out.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CWD: &str = "/work/here";

    fn n(raw: &str) -> String {
        normalize_path(raw, Path::new(CWD), Some("/home/u"))
    }

    #[test]
    fn the_fourth_reopen_is_over_the_cap() {
        assert_eq!(reopen_after(0), (1, false));
        assert_eq!(reopen_after(2), (3, false));
        assert_eq!(reopen_after(3), (4, true));
        assert_eq!(reopen_after(4), (5, true));
        assert_eq!((MAX_REOPENS, MAX_CHAIN), (3, 3));
        let c = chain_refusal(7, 2, 2);
        assert!(
            c.contains("T7 is level 2")
                && c.contains("reaching level 4")
                && c.contains("no fourth level")
        );
        assert!(cascades_to("review") && cascades_to("ready"));
        assert!(!cascades_to("done") && !cascades_to("dropped") && !cascades_to("not_done"));
        let note = cap_note("findings: x");
        assert!(note.contains("not_done") && note.ends_with("findings: x"));
    }

    #[test]
    fn paths_are_absolute_clean_and_slashless() {
        assert_eq!(n("/srv/app/"), "/srv/app");
        assert_eq!(n("/srv//app/./x/../y"), "/srv/app/y");
        assert_eq!(n("~"), "/home/u");
        assert_eq!(n("~/code/app/"), "/home/u/code/app");
        assert_eq!(n("proj"), "/work/here/proj");
        assert_eq!(n("../other/"), "/work/other");
        assert_eq!(n("."), "/work/here");
        assert_eq!(n("/"), "/");
        assert_eq!(n("  "), "");
        assert_eq!(n("/../.."), "/", "cannot climb above the root");
    }

    #[test]
    fn tilde_without_home_is_just_a_relative_name() {
        assert_eq!(
            normalize_path("~/x", Path::new(CWD), None),
            "/work/here/~/x"
        );
    }

    #[test]
    fn a_branch_defaults_to_head_and_refuses_what_git_cannot_check_out() {
        assert_eq!(norm_branch("").unwrap(), "HEAD");
        assert_eq!(norm_branch("   ").unwrap(), "HEAD");
        assert_eq!(norm_branch(" feat/x ").unwrap(), "feat/x");
        assert_eq!(norm_branch("HEAD").unwrap(), "HEAD");
        for bad in [
            "-x", "feat x", "a~b", "a^b", "a:b", "a?b", "a*b", "a[b", "a\\b", "a\tb",
        ] {
            assert!(norm_branch(bad).is_err(), "{bad} must be refused");
        }
        assert_eq!(shown_branch(""), "HEAD");
        assert_eq!(shown_branch("feat/x"), "feat/x");
    }

    #[test]
    fn summary_counts_follow_the_urgency_order_and_skip_empty() {
        let counts = vec![
            ("done".to_owned(), 2),
            ("working".to_owned(), 1),
            ("needs_fix".to_owned(), 1),
        ];
        assert_eq!(
            summary_text("children", &counts).as_deref(),
            Some("children: 4 (1 needs_fix, 1 working, 2 done)")
        );
        assert_eq!(summary_text("children", &[]), None);
    }
}
