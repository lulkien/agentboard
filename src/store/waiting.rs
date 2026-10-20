//! The backend-neutral half of `wait`: argument validation, which polled tasks match,
//! and the `matched:` / `timeout:` text. Each store loads the watched set and polls
//! its own database, then hands plain values to these functions, so the reply is the
//! same text whatever the backend.

use crate::clip::{CAP_LAST, clip_marked};
use crate::store::board::Wait;
use crate::store::{STATUS_LIST, norm, parse_epic_id, parse_id, short};

/// First-line markers: MCP has no exit codes, so the verdict is in the text.
pub const MATCHED: &str = "matched:";
pub const TIMEOUT: &str = "timeout:";

/// Every `kind` the store writes. `ask` is not one: it logs a `block` event.
pub const KINDS: [&str; 11] = [
    "add",
    "claim",
    "report",
    "note",
    "block",
    "answer",
    "review",
    "done",
    "needs_fix",
    "edit",
    "dispatch",
];

pub fn norm_kind(raw: &str) -> Option<&'static str> {
    let k = raw.trim().to_ascii_lowercase();
    let k = if k == "ask" { "block" } else { k.as_str() };
    KINDS.into_iter().find(|x| *x == k)
}

/// `a,b,c` -> canonical names. An empty list or an unknown name is an error,
/// never a filter that silently matches nothing.
pub fn csv(
    raw: &str,
    what: &str,
    canon: fn(&str) -> Option<&'static str>,
    list: &str,
) -> Result<Vec<&'static str>, String> {
    let items: Vec<&str> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if items.is_empty() {
        return Err(format!("wait: empty --{what} list ({list})"));
    }
    items
        .into_iter()
        .map(|i| canon(i).ok_or_else(|| format!("wait: bad {what} {i:?} ({list})")))
        .collect()
}

/// The validated arguments of a `wait` call.
pub struct Parsed {
    pub statuses: Option<Vec<&'static str>>,
    pub kinds: Option<Vec<&'static str>>,
    pub ids: Vec<i64>,
    pub epic: Option<i64>,
}

/// Validates before anything touches the database: a typo is an error, never a
/// filter that matches nothing.
pub fn parse(w: &Wait<'_>) -> Result<Parsed, String> {
    let statuses = w
        .status
        .map(|s| csv(s, "status", norm, STATUS_LIST))
        .transpose()?;
    let kinds = w
        .kind
        .map(|k| csv(k, "kind", norm_kind, &KINDS.join("|")))
        .transpose()?;
    let ids = w
        .ids
        .iter()
        .map(|s| parse_id(s))
        .collect::<Result<Vec<_>, _>>()?;
    let epic = w.epic.map(parse_epic_id).transpose()?;
    if ids.is_empty() && epic.is_none() && !w.open {
        return Err("wait: name tasks (wait T1 T2), --epic E<n> or --open".to_owned());
    }
    Ok(Parsed {
        statuses,
        kinds,
        ids,
        epic,
    })
}

/// The watched set once the selectors are expanded: sorted, deduplicated; an empty
/// set without `--open` is an error (`--open` is a predicate: tasks may appear
/// while waiting).
pub fn finish_set(mut ids: Vec<i64>, open: bool) -> Result<Vec<i64>, String> {
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() && !open {
        return Err("wait: selectors match no tasks".to_owned());
    }
    Ok(ids)
}

pub struct Ev {
    pub seq: i64,
    pub actor: String,
    pub kind: String,
    pub body: String,
}

impl Ev {
    pub fn line(&self) -> String {
        // The delta is what the caller asked for (answers live here): never clipped.
        format!("#{} {} {}: {}", self.seq, self.actor, self.kind, self.body)
    }
}

pub struct Seen {
    pub id: i64,
    pub status: String,
    pub assignee: Option<String>,
    pub updated_at: i64,
    /// Events with seq > the start watermark.
    pub events: Vec<Ev>,
}

impl Seen {
    pub fn header(&self) -> String {
        format!(
            "{} {} {}",
            short(self.id),
            self.status,
            self.assignee.as_deref().unwrap_or("-")
        )
    }
}

/// A task matches when its current status passes `statuses` (if given) and, unless
/// `statuses` is the only filter, it has a new event passing `kinds` (if given).
pub fn hits<'a>(
    seen: &'a [Seen],
    statuses: Option<&[&'static str]>,
    kinds: Option<&[&'static str]>,
) -> Vec<(&'a Seen, Vec<&'a Ev>)> {
    seen.iter()
        .filter_map(|t| {
            if !statuses.is_none_or(|s| s.contains(&t.status.as_str())) {
                return None;
            }
            let evs: Vec<&Ev> = t
                .events
                .iter()
                .filter(|e| kinds.is_none_or(|k| k.contains(&e.kind.as_str())))
                .collect();
            let needs_event = kinds.is_some() || statuses.is_none();
            (!needs_event || !evs.is_empty()).then_some((t, evs))
        })
        .collect()
}

/// The `matched:` reply. `last` looks up a task's latest event, shown as context
/// for a task already in the wanted status with nothing new.
pub fn matched_text(
    watched: usize,
    hits: &[(&Seen, Vec<&Ev>)],
    last: &dyn Fn(i64) -> Option<Ev>,
) -> String {
    let mut out = format!("{MATCHED} {} of {watched} watched\n", hits.len());
    for (t, evs) in hits {
        out.push_str(&t.header());
        out.push('\n');
        if evs.is_empty() {
            // Already in the wanted status with nothing new. The latest event
            // is context only: it may postdate the move that got it here.
            if let Some(e) = last(t.id) {
                out.push_str(&format!("last: {}\n", e.line()));
            }
        }
        for e in evs {
            out.push_str(&e.line());
            out.push('\n');
        }
    }
    out.trim_end().to_owned()
}

/// The freshest watched task: the one whose tail a timeout shows.
pub fn freshest(seen: &[Seen]) -> Option<i64> {
    seen.iter()
        .max_by_key(|t| (t.updated_at, t.id))
        .map(|t| t.id)
}

/// The `timeout:` reply: never blind, every status plus the tail of the freshest
/// task. `tail` is that task's plain `get` text, clipped here.
pub fn timeout_text(timeout_ms: u64, seen: &[Seen], tail: Option<&str>) -> String {
    let mut out = format!(
        "{TIMEOUT} nothing matched after {}s ({} watched)\n",
        timeout_ms / 1000,
        seen.len()
    );
    for t in seen {
        out.push_str(&t.header());
        out.push('\n');
    }
    if let (Some(id), Some(tail)) = (freshest(seen), tail) {
        let tag = short(id);
        out.push_str(&format!(
            "last ({tag}): {}",
            clip_marked(tail, CAP_LAST, &format!("task_get id={tag} for the record"))
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_accepts_known_names_and_aliases() {
        assert_eq!(
            csv("review, done", "status", norm, STATUS_LIST).unwrap(),
            vec!["review", "done"]
        );
        assert_eq!(
            csv("running", "status", norm, STATUS_LIST).unwrap(),
            vec!["working"]
        );
        assert_eq!(
            csv("ask,note", "kind", norm_kind, "k").unwrap(),
            vec!["block", "note"]
        );
    }

    #[test]
    fn csv_rejects_unknown_and_empty() {
        assert!(
            csv("rview", "status", norm, STATUS_LIST)
                .unwrap_err()
                .contains("bad status \"rview\"")
        );
        assert!(
            csv("note,nope", "kind", norm_kind, "k")
                .unwrap_err()
                .contains("bad kind \"nope\"")
        );
        assert!(
            csv("", "status", norm, STATUS_LIST)
                .unwrap_err()
                .contains("empty --status")
        );
        assert!(
            csv(" , ", "kind", norm_kind, "k")
                .unwrap_err()
                .contains("empty --kind")
        );
    }

    #[test]
    fn reclaim_is_not_a_kind() {
        assert!(norm_kind("reclaim").is_none());
        assert_eq!(norm_kind("ask"), Some("block"));
    }

    #[test]
    fn the_set_is_sorted_deduplicated_and_never_silently_empty() {
        assert_eq!(finish_set(vec![4, 2, 4], false).unwrap(), vec![2, 4]);
        assert!(
            finish_set(vec![], false)
                .unwrap_err()
                .contains("match no tasks")
        );
        assert!(finish_set(vec![], true).unwrap().is_empty());
    }
}
