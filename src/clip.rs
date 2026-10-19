//! Compact rendering of free text. Everything shown to a reader is first folded
//! to one line (`\n` -> ` ⏎ `, 3 chars); every cap, count and marker below is in
//! chars of that folded view, so "N of M shown" always compares like with like.

pub const CAP_DESCRIPTION: usize = 4000;
pub const CAP_GOAL: usize = 4000;
pub const CAP_REPORT: usize = 6000;
pub const CAP_NOTE: usize = 800;
pub const CAP_EVENT: usize = 220;
pub const CAP_LAST: usize = 400;

fn fold(s: &str) -> String {
    s.replace('\n', " ⏎ ")
}

/// Folded text, cut to `max` chars with a bare `…` when it does not fit. For
/// titles and other lines where loss is expected and the full value is one
/// `get` away.
pub fn clip(s: &str, max: usize) -> String {
    let one_line = fold(s);
    if one_line.chars().count() <= max {
        return one_line;
    }
    let cut: String = one_line.chars().take(max).collect();
    format!("{cut}…")
}

/// Folded text cut to `max` chars, or the whole thing. Second value is the
/// folded length, `Some` only when something was cut.
fn cut(s: &str, max: usize) -> (String, Option<usize>) {
    let one_line = fold(s);
    let total = one_line.chars().count();
    if total <= max {
        return (one_line, None);
    }
    (one_line.chars().take(max).collect(), Some(total))
}

/// Like `clip`, but a cut ends in `[truncated: N of M chars; <hint>]` instead of
/// a bare `…`, so loss is never silent.
pub fn clip_marked(s: &str, max: usize, hint: &str) -> String {
    match cut(s, max) {
        (shown, None) => shown,
        (shown, Some(total)) => format!("{shown} [truncated: {max} of {total} chars; {hint}]"),
    }
}

/// A long free-text field in `get` / `claim`. Plain mode is compact and, when
/// the value does not fit in `max`, ends in a marker naming the field, the
/// shown and total length (folded chars, raw count added when it differs) and
/// the way to the rest. `full` returns the value byte-for-byte, no marker.
pub fn field(label: &str, value: &str, max: usize, full: bool, tag: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    if full {
        return format!("{label}:\n{value}\n");
    }
    match cut(value, max) {
        (shown, None) => format!("{label}:\n{shown}\n"),
        (shown, Some(total)) => {
            let raw = value.chars().count();
            let unit = if raw == total {
                String::new()
            } else {
                format!(" (newline = 3 chars; {raw} raw)")
            };
            format!(
                "{label}:\n{shown} [truncated {label}: {max} of {total} chars shown{unit}; full text: `agentboard get {tag} --full` or task_get id={tag} full=true]\n"
            )
        }
    }
}
