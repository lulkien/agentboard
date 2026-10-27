use agentboard::domain::{EventKind, Status};
use agentboard::store::Cursor;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Page {
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

#[derive(Debug, Default)]
pub struct Filters {
    pub status: Option<Status>,
    pub assignee: Option<String>,
    pub parent: Option<u64>,
    pub offset: usize,
    pub feed_task: Option<u64>,
    pub feed_kind: Option<EventKind>,
    pub before: Option<String>,
}

pub fn parse(path: &str, query: &str) -> Result<Route, ()> {
    let page = if matches!(path, "/" | "/epics") {
        Page::Epics
    } else if path == "/tasks" {
        Page::Tasks
    } else if path == "/feed" {
        Page::Feed
    } else if let Some(raw_id) = path.strip_prefix("/tasks/") {
        if raw_id.is_empty() || raw_id.contains('/') || !query.is_empty() {
            return Err(());
        }
        Page::Detail(positive(raw_id)?)
    } else {
        return Err(());
    };
    let mut filters = Filters::default();
    let mut seen = [false; 7];
    if !query.is_empty() {
        for pair in query.split('&') {
            if pair.is_empty() {
                return Err(());
            }
            let (key, value) = pair.split_once('=').ok_or(())?;
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
                _ => return Err(()),
            };
            if seen[slot] {
                return Err(());
            }
            seen[slot] = true;
            match key.as_str() {
                "status" if !value.is_empty() => {
                    filters.status = Some(Status::parse(&value).map_err(|_| ())?)
                }
                "status" => {}
                "assignee" => {
                    if !value.is_empty() {
                        filters.assignee = Some(value);
                    }
                }
                "parent" if !value.is_empty() => filters.parent = Some(positive(&value)?),
                "parent" => {}
                "offset" => filters.offset = value.parse().map_err(|_| ())?,
                "task" if !value.is_empty() => filters.feed_task = Some(positive(&value)?),
                "task" => {}
                "kind" if !value.is_empty() => {
                    filters.feed_kind = Some(EventKind::parse(&value).map_err(|_| ())?)
                }
                "kind" => {}
                "before" => {
                    Cursor::parse(&value).map_err(|_| ())?;
                    filters.before = Some(value);
                }
                _ => unreachable!(),
            }
        }
    }
    if page == Page::Epics
        && (filters.status.is_some()
            || filters.assignee.is_some()
            || filters.parent.is_some()
            || filters.offset != 0)
    {
        return Err(());
    }
    if matches!(page, Page::Detail(_))
        && (filters.status.is_some()
            || filters.assignee.is_some()
            || filters.parent.is_some()
            || filters.offset != 0)
    {
        return Err(());
    }
    if page == Page::Feed
        && (filters.status.is_some()
            || filters.assignee.is_some()
            || filters.parent.is_some()
            || filters.offset != 0)
    {
        return Err(());
    }
    if page != Page::Feed
        && (filters.feed_task.is_some() || filters.feed_kind.is_some() || filters.before.is_some())
    {
        return Err(());
    }
    Ok(Route { page, filters })
}

fn positive(value: &str) -> Result<u64, ()> {
    let id: u64 = value.parse().map_err(|_| ())?;
    if id == 0 { Err(()) } else { Ok(id) }
}

fn decode(value: &str) -> Result<String, ()> {
    let mut bytes = Vec::with_capacity(value.len());
    let raw = value.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        match raw[i] {
            b'+' => bytes.push(b' '),
            b'%' => {
                let high = *raw.get(i + 1).ok_or(())?;
                let low = *raw.get(i + 2).ok_or(())?;
                bytes.push((hex(high)? << 4) | hex(low)?);
                i += 2;
            }
            byte => bytes.push(byte),
        }
        i += 1;
    }
    String::from_utf8(bytes).map_err(|_| ())
}

fn hex(byte: u8) -> Result<u8, ()> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(()),
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
    }
}
