//! Read-only HTTP dashboard for an existing agentboard.

use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use agentboard::store::{Board, BoardOptions};

mod html;
mod pages;
mod routes;
mod template;

const ERROR_TEMPLATE: template::Template =
    template::Template::new("error", include_str!("dashboard/templates/error.html"));

const MAX_REQUEST_BYTES: usize = 8 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(2);

pub fn serve(path: PathBuf, address: SocketAddr, allow_non_loopback: bool) -> Result<(), String> {
    if !address.ip().is_loopback() && !allow_non_loopback {
        return Err(
            "dashboard binding to a non-loopback address requires --allow-non-loopback".into(),
        );
    }
    if !path.join("data.mdb").is_file() {
        return Err(format!(
            "dashboard requires an existing board at {}",
            path.display()
        ));
    }
    let board =
        Board::open_with_options(&path, BoardOptions::from_env().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let listener = TcpListener::bind(address).map_err(|e| format!("dashboard bind: {e}"))?;
    let bound_address = listener
        .local_addr()
        .map_err(|e| format!("dashboard bound address: {e}"))?;
    for incoming in listener.incoming() {
        match incoming {
            Ok(mut stream) => {
                if let Err(error) = respond(&mut stream, &board, bound_address)
                    && !matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    )
                {
                    eprintln!("dashboard request: {error}");
                }
            }
            Err(error) => eprintln!("dashboard accept: {error}"),
        }
    }
    Ok(())
}

fn respond(stream: &mut TcpStream, board: &Board, bound_address: SocketAddr) -> io::Result<()> {
    let deadline = Instant::now() + READ_TIMEOUT;
    let mut request = Vec::with_capacity(1024);
    let mut chunk = [0_u8; 1024];
    let result = loop {
        if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
            break ReadResult::Complete(end + 4);
        }
        if request.len() == MAX_REQUEST_BYTES {
            break ReadResult::TooLarge;
        }
        let time = deadline.saturating_duration_since(Instant::now());
        if time.is_zero() {
            break ReadResult::Timeout;
        }
        stream.set_read_timeout(Some(time))?;
        let read_limit = (MAX_REQUEST_BYTES - request.len()).min(chunk.len());
        match stream.read(&mut chunk[..read_limit]) {
            Ok(0) => break ReadResult::Incomplete,
            Ok(n) => request.extend_from_slice(&chunk[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                break ReadResult::Timeout;
            }
            Err(e) => return Err(e),
        }
    };
    let end = match result {
        ReadResult::Complete(end) => end,
        ReadResult::TooLarge => {
            return response(
                stream,
                "431 Request Header Fields Too Large",
                "text/plain; charset=utf-8",
                "request headers too large",
            );
        }
        ReadResult::Timeout => {
            return response(
                stream,
                "408 Request Timeout",
                "text/plain; charset=utf-8",
                "request header timed out",
            );
        }
        ReadResult::Incomplete => {
            return response(
                stream,
                "400 Bad Request",
                "text/plain; charset=utf-8",
                "incomplete request header",
            );
        }
    };
    let request_text = match std::str::from_utf8(&request[..end]) {
        Ok(request) => request,
        Err(_) => {
            return response(
                stream,
                "400 Bad Request",
                "text/plain; charset=utf-8",
                "invalid request line",
            );
        }
    };
    let mut lines = request_text.split("\r\n");
    let line = match lines.next() {
        Some(line) => line,
        None => {
            return response(
                stream,
                "400 Bad Request",
                "text/plain; charset=utf-8",
                "invalid request line",
            );
        }
    };
    let mut host = None;
    for header in lines {
        if header.is_empty() {
            break;
        }
        let Some((name, value)) = header.split_once(':') else {
            return response(
                stream,
                "400 Bad Request",
                "text/plain; charset=utf-8",
                "invalid request header",
            );
        };
        if name.eq_ignore_ascii_case("host") && host.replace(value.trim()).is_some() {
            return response(
                stream,
                "400 Bad Request",
                "text/plain; charset=utf-8",
                "duplicate host header",
            );
        }
    }
    let Some(host) = host else {
        return response(
            stream,
            "400 Bad Request",
            "text/plain; charset=utf-8",
            "host header required",
        );
    };
    if !host_allowed(host, bound_address) {
        return response(
            stream,
            "421 Misdirected Request",
            "text/plain; charset=utf-8",
            "misdirected request",
        );
    }
    let mut fields = line.split_ascii_whitespace();
    let (Some(method), Some(target), Some(version)) = (fields.next(), fields.next(), fields.next())
    else {
        return response(
            stream,
            "400 Bad Request",
            "text/plain; charset=utf-8",
            "invalid request line",
        );
    };
    if fields.next().is_some() || !version.starts_with("HTTP/") {
        return response(
            stream,
            "400 Bad Request",
            "text/plain; charset=utf-8",
            "invalid request line",
        );
    }
    if method != "GET" {
        return response(
            stream,
            "405 Method Not Allowed",
            "text/plain; charset=utf-8",
            "method not allowed",
        );
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path == "/assets/dashboard.css" {
        return response(
            stream,
            "200 OK",
            "text/css; charset=utf-8",
            include_str!("dashboard/style.css"),
        );
    }
    if path == "/assets/theme.js" {
        return response(
            stream,
            "200 OK",
            "text/javascript; charset=utf-8",
            include_str!("dashboard/theme.js"),
        );
    }
    if path
        .strip_prefix("/tasks/")
        .is_some_and(|tail| tail.contains('/'))
    {
        return response(
            stream,
            "404 Not Found",
            "text/plain; charset=utf-8",
            "not found",
        );
    }
    if !matches!(path, "/" | "/epics" | "/tasks" | "/feed") && !path.starts_with("/tasks/") {
        return response(
            stream,
            "404 Not Found",
            "text/plain; charset=utf-8",
            "not found",
        );
    }
    let route = match routes::parse(path, query) {
        Ok(route) => route,
        Err(()) => {
            return response(
                stream,
                "400 Bad Request",
                "text/plain; charset=utf-8",
                "invalid dashboard query",
            );
        }
    };
    let body = match pages::render(board, route) {
        Ok(body) => body,
        Err(pages::PageError::BadRequest) => {
            return response(
                stream,
                "400 Bad Request",
                "text/plain; charset=utf-8",
                "invalid dashboard query",
            );
        }
        Err(pages::PageError::Missing) => {
            return response(
                stream,
                "404 Not Found",
                "text/plain; charset=utf-8",
                "not found",
            );
        }
        Err(pages::PageError::Store(message)) => {
            ERROR_TEMPLATE.fill(&[("message", template::text(message))])
        }
    };
    let body = body.into_string();
    response(stream, "200 OK", "text/html; charset=utf-8", &body)
}

fn host_allowed(host: &str, bound_address: SocketAddr) -> bool {
    let host = host.trim();
    if host
        .parse::<SocketAddr>()
        .is_ok_and(|candidate| candidate == bound_address)
    {
        return true;
    }
    let bare_ip = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);
    if bare_ip
        .parse::<IpAddr>()
        .is_ok_and(|candidate| candidate == bound_address.ip())
    {
        return true;
    }
    let (name, port) = match host.rsplit_once(':') {
        Some((name, raw_port)) if !name.contains(':') => {
            let Ok(port) = raw_port.parse::<u16>() else {
                return false;
            };
            (name, Some(port))
        }
        _ => (host, None),
    };
    if port.is_some_and(|port| port != bound_address.port()) {
        return false;
    }
    name.eq_ignore_ascii_case("localhost") || name == "127.0.0.1"
}

enum ReadResult {
    Complete(usize),
    TooLarge,
    Timeout,
    Incomplete,
}

fn response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &str,
) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\n",
        body.len()
    )?;
    if content_type.starts_with("text/html") {
        stream.write_all(b"Cache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; style-src 'self'; script-src 'self'; connect-src 'self'; img-src 'self'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'\r\n")?;
    }
    if content_type.starts_with("text/plain") {
        stream.write_all(b"Cache-Control: no-store\r\n")?;
    }
    if status.starts_with("405") {
        stream.write_all(b"Allow: GET\r\n")?;
    }
    stream.write_all(b"\r\n")?;
    stream.write_all(body.as_bytes())?;
    stream.flush()
}
