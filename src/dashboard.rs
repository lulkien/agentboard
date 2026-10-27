//! Read-only HTTP dashboard for an existing agentboard.

use std::fmt::Write as FmtWrite;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use agentboard::domain::Status;
use agentboard::store::{Board, BoardOptions, ListFilter, MAX_LIST_LIMIT};

const MAX_REQUEST_BYTES: usize = 8 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(2);
const PAGE_SIZE: usize = 20;

pub fn serve(path: PathBuf, address: SocketAddr) -> Result<(), String> {
    if !path.join("data.mdb").is_file() {
        return Err(format!(
            "dashboard requires an existing board at {}",
            path.display()
        ));
    }
    let board = Board::open_with_options(
        &path,
        BoardOptions::from_env().map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let listener =
        TcpListener::bind(address).map_err(|error| format!("dashboard bind: {error}"))?;
    for incoming in listener.incoming() {
        match incoming {
            Ok(mut stream) => {
                if let Err(error) = respond(&mut stream, &board)
                    && error.kind() != io::ErrorKind::TimedOut
                    && error.kind() != io::ErrorKind::WouldBlock
                {
                    eprintln!("dashboard request: {error}");
                }
            }
            Err(error) => eprintln!("dashboard accept: {error}"),
        }
    }
    Ok(())
}

fn respond(stream: &mut TcpStream, board: &Board) -> io::Result<()> {
    let deadline = Instant::now() + READ_TIMEOUT;
    let mut request = Vec::with_capacity(1024);
    let mut chunk = [0_u8; 1024];
    let header_end = loop {
        if let Some(end) = find_header_end(&request) {
            break HeaderRead::Complete(end);
        }
        if request.len() == MAX_REQUEST_BYTES {
            break HeaderRead::TooLarge;
        }
        let remaining_time = deadline.saturating_duration_since(Instant::now());
        if remaining_time.is_zero() {
            break HeaderRead::TimedOut;
        }
        stream.set_read_timeout(Some(remaining_time))?;
        let read_limit = (MAX_REQUEST_BYTES - request.len()).min(chunk.len());
        match stream.read(&mut chunk[..read_limit]) {
            Ok(0) => break HeaderRead::Incomplete,
            Ok(read) => request.extend_from_slice(&chunk[..read]),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                break HeaderRead::TimedOut;
            }
            Err(error) => return Err(error),
        }
    };

    let header_end = match header_end {
        HeaderRead::Complete(end) => end,
        HeaderRead::TooLarge => {
            return write_response(
                stream,
                "431 Request Header Fields Too Large",
                "request headers too large",
            );
        }
        HeaderRead::TimedOut => {
            return write_response(stream, "408 Request Timeout", "request header timed out");
        }
        HeaderRead::Incomplete => {
            return write_response(stream, "400 Bad Request", "incomplete request header");
        }
    };

    let request_line = request[..header_end]
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    let request_line = match std::str::from_utf8(request_line) {
        Ok(line) => line.trim_end_matches('\r'),
        Err(_) => return write_response(stream, "400 Bad Request", "invalid request line"),
    };
    let mut parts = request_line.split_ascii_whitespace();
    let (Some(method), Some(target), Some(version)) = (parts.next(), parts.next(), parts.next())
    else {
        return write_response(stream, "400 Bad Request", "invalid request line");
    };
    if parts.next().is_some() || !version.starts_with("HTTP/") {
        return write_response(stream, "400 Bad Request", "invalid request line");
    }
    if method != "GET" {
        return write_response(stream, "405 Method Not Allowed", "method not allowed");
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/" {
        return write_response(stream, "404 Not Found", "not found");
    }
    let filters = match parse_query(query) {
        Ok(filters) => filters,
        Err(()) => return write_response(stream, "400 Bad Request", "invalid dashboard query"),
    };

    let page = match board.list(&ListFilter {
        status: filters.status,
        assignee: filters.assignee.as_deref(),
        parent: None,
        path: None,
        include_terminal: true,
        limit: PAGE_SIZE.min(MAX_LIST_LIMIT),
        offset: filters.offset,
    }) {
        Ok(page) => page,
        Err(error) => {
            let body = format!(
                "<h1>Board read failed</h1><p>{}</p>",
                html_escape(&error.to_string())
            );
            return write_response(stream, "500 Internal Server Error", &body);
        }
    };
    let brief = match board.brief(0) {
        Ok(brief) => brief,
        Err(error) => {
            let body = format!(
                "<h1>Board read failed</h1><p>{}</p>",
                html_escape(&error.to_string())
            );
            return write_response(stream, "500 Internal Server Error", &body);
        }
    };
    let body = render_dashboard(&brief, &page, &filters);
    write_response(stream, "200 OK", &body)
}

enum HeaderRead {
    Complete(usize),
    TooLarge,
    TimedOut,
    Incomplete,
}

fn find_header_end(request: &[u8]) -> Option<usize> {
    request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|offset| offset + 4)
}

fn write_response(stream: &mut TcpStream, status: &str, body: &str) -> io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

#[derive(Default)]
struct DashboardFilters {
    status: Option<Status>,
    assignee: Option<String>,
    offset: usize,
}

fn parse_query(query: &str) -> Result<DashboardFilters, ()> {
    let mut filters = DashboardFilters::default();
    let mut seen_status = false;
    let mut seen_assignee = false;
    let mut seen_offset = false;
    if query.is_empty() {
        return Ok(filters);
    }
    for pair in query.split('&') {
        if pair.is_empty() {
            return Err(());
        }
        let (raw_key, raw_value) = pair.split_once('=').ok_or(())?;
        let key = url_decode(raw_key)?;
        let value = url_decode(raw_value)?;
        match key.as_str() {
            "status" if !seen_status => {
                seen_status = true;
                filters.status = Some(Status::parse(&value).map_err(|_| ())?);
            }
            "assignee" if !seen_assignee => {
                seen_assignee = true;
                if !value.is_empty() {
                    filters.assignee = Some(value);
                }
            }
            "offset" if !seen_offset => {
                seen_offset = true;
                filters.offset = value.parse().map_err(|_| ())?;
            }
            _ => return Err(()),
        }
    }
    Ok(filters)
}

fn url_decode(value: &str) -> Result<String, ()> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => decoded.push(b' '),
            b'%' => {
                let high = *bytes.get(index + 1).ok_or(())?;
                let low = *bytes.get(index + 2).ok_or(())?;
                decoded.push((hex_value(high)? << 4) | hex_value(low)?);
                index += 2;
            }
            byte => decoded.push(byte),
        }
        index += 1;
    }
    String::from_utf8(decoded).map_err(|_| ())
}

fn hex_value(byte: u8) -> Result<u8, ()> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(()),
    }
}

fn render_dashboard(
    brief: &agentboard::store::Brief,
    page: &agentboard::store::TaskPage,
    filters: &DashboardFilters,
) -> String {
    let mut html = String::with_capacity(8192);
    html.push_str("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Agentboard</title><style>body{font:16px system-ui,sans-serif;max-width:1100px;margin:2rem auto;padding:0 1rem;color:#18212b}table{border-collapse:collapse;width:100%;margin:1rem 0}th,td{text-align:left;border-bottom:1px solid #ccd3da;padding:.5rem}ul.counts{display:flex;gap:1rem;flex-wrap:wrap;padding:0;list-style:none}ul.counts li{background:#f1f4f7;padding:.5rem .75rem;border-radius:.4rem}progress{width:10rem}form{display:flex;gap:.75rem;align-items:end;flex-wrap:wrap}label{display:grid;gap:.25rem}nav{display:flex;gap:1rem}</style></head><body><h1>Agentboard</h1>");

    html.push_str("<section aria-labelledby=\"counts-heading\"><h2 id=\"counts-heading\">Task counts</h2><ul class=\"counts\">");
    for status in Status::ALL {
        let count = brief
            .counts
            .iter()
            .find_map(|(candidate, count)| (*candidate == status).then_some(count))
            .copied()
            .unwrap_or(0);
        let _ = write!(
            html,
            "<li><span>{}</span>: <strong>{count}</strong></li>",
            status.as_str()
        );
    }
    html.push_str("</ul></section><section aria-labelledby=\"epics-heading\"><h2 id=\"epics-heading\">Epics</h2>");
    if brief.epics.is_empty() {
        html.push_str("<p>No epics</p>");
    } else {
        html.push_str("<ul>");
        for epic in &brief.epics {
            let maximum = epic.total.max(1);
            let progress = epic.done.min(maximum);
            let _ = write!(
                html,
                "<li>T{} {} — {}/{} done <progress max=\"{maximum}\" value=\"{progress}\" aria-label=\"epic progress\"></progress></li>",
                epic.task.id,
                html_escape(&epic.task.title),
                epic.done,
                epic.total
            );
        }
        html.push_str("</ul>");
    }

    html.push_str("</section><section aria-labelledby=\"tasks-heading\"><h2 id=\"tasks-heading\">Tasks</h2><form method=\"get\" action=\"/\"><label>Status<select name=\"status\"><option value=\"\">All statuses</option>");
    for status in Status::ALL {
        let selected = filters.status == Some(status);
        let _ = write!(
            html,
            "<option value=\"{}\"{}>{}</option>",
            status.as_str(),
            if selected { " selected" } else { "" },
            status.as_str()
        );
    }
    let shown_offset = filters.offset.min(page.total);
    let _ = write!(
        html,
        "</select></label><label>Assignee<input name=\"assignee\" value=\"{}\"></label><button type=\"submit\">Filter</button></form><p>Showing {}–{} of {}</p><table><thead><tr><th>ID</th><th>Status</th><th>Title</th><th>Assignee</th><th>Path</th><th>Priority</th></tr></thead><tbody>",
        html_escape(filters.assignee.as_deref().unwrap_or_default()),
        if page.tasks.is_empty() {
            0
        } else {
            shown_offset + 1
        },
        shown_offset + page.tasks.len(),
        page.total
    );
    if page.tasks.is_empty() {
        html.push_str("<tr><td colspan=\"6\">No tasks match these filters</td></tr>");
    } else {
        for task in &page.tasks {
            let assignee = task.assignee.as_deref().unwrap_or("—");
            let path = task.path.as_deref().unwrap_or("—");
            let _ = write!(
                html,
                "<tr><td>T{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                task.id,
                html_escape(task.status.as_str()),
                html_escape(&task.title),
                html_escape(assignee),
                html_escape(path),
                task.priority
            );
        }
    }
    html.push_str("</tbody></table><nav>");
    if shown_offset > 0 {
        let previous_offset = shown_offset.saturating_sub(PAGE_SIZE);
        let _ = write!(
            html,
            "<a href=\"{}\">Previous</a>",
            page_url(filters, previous_offset)
        );
    }
    if page.more {
        let next_offset = filters.offset.saturating_add(page.tasks.len());
        let _ = write!(
            html,
            "<a href=\"{}\">Next</a>",
            page_url(filters, next_offset)
        );
    }
    html.push_str("</nav></section></body></html>");
    html
}

fn page_url(filters: &DashboardFilters, offset: usize) -> String {
    let mut params = Vec::new();
    if let Some(status) = filters.status {
        params.push(format!("status={}", status.as_str()));
    }
    if let Some(assignee) = &filters.assignee {
        params.push(format!("assignee={}", url_encode(assignee)));
    }
    params.push(format!("offset={offset}"));
    format!("/?{}", params.join("&"))
}

fn url_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

fn html_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        escaped.push_str(match character {
            '&' => "&amp;",
            '<' => "&lt;",
            '>' => "&gt;",
            '"' => "&quot;",
            '\'' => "&#39;",
            _ => {
                escaped.push(character);
                continue;
            }
        });
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_rejects_invalid_and_duplicate_values() {
        assert!(parse_query("status=review").unwrap().status == Some(Status::Reviewing));
        assert!(parse_query("status=bogus").is_err());
        assert!(parse_query("offset=-1").is_err());
        assert!(parse_query("status=todo&status=done").is_err());
        assert!(parse_query("unexpected=x").is_err());
    }

    #[test]
    fn escapes_html_text_and_attributes() {
        assert_eq!(
            html_escape("<script a=\"x\">&'"),
            "&lt;script a=&quot;x&quot;&gt;&amp;&#39;"
        );
    }
}
