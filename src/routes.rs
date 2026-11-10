use crate::domain::{EventKind, Status};
use crate::store::Cursor;

/// A path or query the dashboard grammar refuses.
#[derive(Debug)]
pub struct BadQuery;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Page {
    Board,
    Epics,
    Tasks,
    Detail(u64),
    Feed,
}

#[derive(Debug)]
pub struct Route {
    pub page: Page,
    pub filters: Filters,
}

#[derive(Clone, Debug, Default)]
pub struct Filters {
    pub project: Option<u64>,
    pub status: Option<Status>,
    pub assignee: Option<String>,
    pub parent: Option<u64>,
    pub offset: usize,
    pub feed_task: Option<u64>,
    pub feed_kind: Option<EventKind>,
    pub before: Option<String>,
}

pub fn parse(path: &str, query: &str) -> Result<Route, BadQuery> {
    let page = if path == "/" {
        Page::Board
    } else if path == "/epics" {
        Page::Epics
    } else if path == "/tasks" {
        Page::Tasks
    } else if path == "/feed" {
        Page::Feed
    } else if let Some(raw_id) = path.strip_prefix("/tasks/") {
        if raw_id.is_empty() || raw_id.contains('/') {
            return Err(BadQuery);
        }
        Page::Detail(positive(raw_id)?)
    } else {
        return Err(BadQuery);
    };
    let mut filters = Filters::default();
    let mut seen = [false; 8];
    if !query.is_empty() {
        for pair in query.split('&') {
            if pair.is_empty() {
                return Err(BadQuery);
            }
            let (key, value) = pair.split_once('=').ok_or(BadQuery)?;
            let key = decode(key)?;
            let value = decode(value)?;
            let slot = match key.as_str() {
                "status" => 0,
                "assignee" => 1,
                "parent" => 2,
                "offset" => 3,
                "task" => 4,
                "kind" => 5,
                "before" => 6,
                "project" => 7,
                _ => return Err(BadQuery),
            };
            if seen[slot] {
                return Err(BadQuery);
            }
            seen[slot] = true;
            match key.as_str() {
                "status" if !value.is_empty() => {
                    filters.status = Some(Status::parse(&value).map_err(|_| BadQuery)?)
                }
                "status" => {}
                "assignee" => {
                    if !value.is_empty() {
                        filters.assignee = Some(value);
                    }
                }
                "parent" if !value.is_empty() => filters.parent = Some(positive(&value)?),
                "parent" => {}
                "offset" if !value.is_empty() => {
                    filters.offset = value.parse().map_err(|_| BadQuery)?
                }
                "offset" => {}
                "task" if !value.is_empty() => filters.feed_task = Some(positive(&value)?),
                "task" => {}
                "kind" if !value.is_empty() => {
                    filters.feed_kind = Some(EventKind::parse(&value).map_err(|_| BadQuery)?)
                }
                "kind" => {}
                "before" if !value.is_empty() => {
                    Cursor::parse(&value).map_err(|_| BadQuery)?;
                    filters.before = Some(value);
                }
                "before" => {}
                "project" if !value.is_empty() => filters.project = Some(positive(&value)?),
                "project" => {}
                _ => unreachable!(),
            }
        }
    }
    let allowed = match page {
        Page::Board | Page::Epics | Page::Detail(_) => [7].as_slice(),
        Page::Tasks => [0, 1, 2, 3, 7].as_slice(),
        Page::Feed => [4, 5, 6, 7].as_slice(),
    };
    if seen
        .iter()
        .enumerate()
        .any(|(slot, present)| *present && !allowed.contains(&slot))
    {
        return Err(BadQuery);
    }
    Ok(Route { page, filters })
}

fn positive(value: &str) -> Result<u64, BadQuery> {
    let id: u64 = value.parse().map_err(|_| BadQuery)?;
    if id == 0 { Err(BadQuery) } else { Ok(id) }
}

fn decode(value: &str) -> Result<String, BadQuery> {
    let mut bytes = Vec::with_capacity(value.len());
    let raw = value.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        match raw[i] {
            b'+' => bytes.push(b' '),
            b'%' => {
                let high = *raw.get(i + 1).ok_or(BadQuery)?;
                let low = *raw.get(i + 2).ok_or(BadQuery)?;
                bytes.push((hex(high)? << 4) | hex(low)?);
                i += 2;
            }
            byte => bytes.push(byte),
        }
        i += 1;
    }
    String::from_utf8(bytes).map_err(|_| BadQuery)
}

fn hex(byte: u8) -> Result<u8, BadQuery> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(BadQuery),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unknown_duplicate_and_malformed_queries() {
        for query in [
            "status=bogus",
            "status=todo&status=done",
            "unknown=x",
            "offset=-1",
            "parent=0",
            "assignee=%FF",
            "status=%",
        ] {
            assert!(parse("/tasks", query).is_err(), "accepted {query}");
        }
        assert!(parse("/tasks", "status=").is_ok());
        assert!(parse("/tasks", "parent=4&offset=20").is_ok());
        assert!(parse("/epics", "offset=1").is_err());
        assert!(parse("/feed", "task=4&kind=commented&before=board-id:12").is_ok());
        for query in [
            "task=0",
            "kind=invalid",
            "before=bad",
            "task=1&task=2",
            "kind=created&kind=updated",
            "task=1&unknown=x",
        ] {
            assert!(
                parse("/feed", query).is_err(),
                "accepted feed query {query}"
            );
        }
        assert!(parse("/tasks", "kind=created").is_err());
        assert!(parse("/tasks/3", "project=1").is_ok());
        assert!(parse("/tasks/3", "status=").is_err());
        assert!(parse("/tasks/3", "offset=0").is_err());
        assert!(parse("/epics", "status=").is_err());
        assert!(parse("/feed", "offset=0").is_err());
        assert!(parse("/tasks", "project=0").is_err());
        assert!(parse("/tasks", "project=1&project=2").is_err());
    }
}
