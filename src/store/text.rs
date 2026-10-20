//! Backend-neutral rows and the pure text that renders them. Both the SQLite and the
//! SurrealDB store load plain values (the row structs below), then call these
//! functions, so `ls`, `get`, `children`, `epic ...` and `brief` print the same text
//! whatever the backend. Nothing here touches a database or a driver type; a field
//! whose type differs between backends is normalised where the row is loaded.

use crate::clip::{
    CAP_DESCRIPTION, CAP_EVENT, CAP_GOAL, CAP_NOTE, CAP_REPORT, clip, clip_marked, field,
};
use crate::store::{now_ms, short, short_epic};
use std::path::{Component, Path, PathBuf};

/// One task as stored.
#[derive(Clone, Debug)]
pub struct TaskRow {
    pub id: i64,
    pub status: String,
    pub assignee: Option<String>,
    pub needs: Option<String>,
    pub title: String,
    pub description: String,
    pub goal: String,
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

// ---- time and status --------------------------------------------------------------

pub fn code(status: &str) -> &str {
    match status {
        "ready" | "triage" => "rdy",
        "working" | "running" => "wrk",
        "blocking" | "blocked" => "blk",
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
const STATUS_ORDER: [&str; 7] = [
    "blocking",
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
        "needs_fix" => 1,
        "working" => 2,
        "review" => 3,
        "ready" => 4,
        _ => 5,
    }
}

/// Display rank for `brief` (no epic): ready and everything else share the last rank.
#[cfg(feature = "surreal")]
pub fn brief_rank(status: &str) -> u8 {
    match status {
        "blocking" => 0,
        "needs_fix" => 1,
        "working" => 2,
        "review" => 3,
        _ => 4,
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

/// `get`: the task, its epic line, the subtask summary and the event block.
/// `full` returns description, goal, report and note unclipped and verbatim.
pub fn get_text(
    r: &TaskRow,
    epic: Option<&EpicRow>,
    summary: Option<&str>,
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
    out.push_str(&format!("title: {}\n", r.title));
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
