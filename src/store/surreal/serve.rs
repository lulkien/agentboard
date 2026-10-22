//! Auto-serve: make sure a local `surreal` server answers on the configured
//! address, starting one if none does. HTTP is a hand-written GET over
//! `TcpStream`, so no HTTP crate is needed for a status line and a short body.

use super::Settings;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Serialises server start-up, in the state dir beside the data dir.
pub(super) const START_LOCK: &str = "surreal.start.lock";

pub const INSTALL_HINT: &str = "curl -sSf https://install.surrealdb.com | sh -s -- --version 2.7.0";

/// Budgets for the probe and the start-up wait.
#[derive(Clone, Copy)]
pub struct Timing {
    pub probe: Duration,
    pub poll_every: Duration,
    pub deadline: Duration,
    /// Budget for the WebSocket handshake, sign-in and bootstrap after `/health`
    /// answers: a server that answers HTTP but wedges there must not hang a verb.
    pub connect: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            probe: Duration::from_millis(300),
            poll_every: Duration::from_millis(100),
            deadline: Duration::from_secs(10),
            connect: Duration::from_secs(10),
        }
    }
}

/// What `ensure_up` did: the pid it spawned, or `None` if a server was already up.
pub struct Served {
    pub spawned_pid: Option<u32>,
}

/// `GET <path>` with one overall `budget`; returns the status code and the body.
pub fn http_get(addr: &str, path: &str, budget: Duration) -> Result<(u16, String), String> {
    let start = Instant::now();
    let sock = addr
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .next()
        .ok_or_else(|| "no address".to_owned())?;
    let mut s = TcpStream::connect_timeout(&sock, budget).map_err(|e| e.to_string())?;
    let left = |start: Instant| {
        budget
            .saturating_sub(start.elapsed())
            .max(Duration::from_millis(1))
    };
    s.set_write_timeout(Some(left(start)))
        .map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(left(start)))
        .map_err(|e| e.to_string())?;
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    )
    .map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    // Bounded: a status line and a short body are all that is read.
    s.take(64 * 1024)
        .read_to_end(&mut raw)
        .map_err(|e| e.to_string())?;
    parse_response(&String::from_utf8_lossy(&raw))
}

/// Status line and body of an HTTP/1.1 response. The body is de-chunked when the
/// server sends it chunked (`/version` does).
pub fn parse_response(raw: &str) -> Result<(u16, String), String> {
    let status = raw
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse::<u16>().ok())
        .ok_or_else(|| {
            format!(
                "not an HTTP response: {:?}",
                raw.lines().next().unwrap_or("")
            )
        })?;
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw, ""));
    let chunked = head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked");
    let body = if chunked {
        dechunk(body)
    } else {
        body.to_owned()
    };
    Ok((status, body.trim().to_owned()))
}

/// `<hex size>\r\n<data>\r\n ... 0\r\n`. Stops at the first malformed size.
fn dechunk(mut rest: &str) -> String {
    let mut out = String::new();
    while let Some((size, tail)) = rest.split_once("\r\n") {
        let Ok(n) = usize::from_str_radix(size.split(';').next().unwrap_or("").trim(), 16) else {
            break;
        };
        if n == 0 || tail.len() < n || !tail.is_char_boundary(n) {
            break;
        }
        out.push_str(&tail[..n]);
        rest = tail[n..].strip_prefix("\r\n").unwrap_or(&tail[n..]);
    }
    out
}

/// True when `/health` answers 200 within the probe budget.
pub fn healthy(url: &str, budget: Duration) -> bool {
    matches!(http_get(url, "/health", budget), Ok((200, _)))
}

/// The `surreal` binary: `$PATH` first, then `~/.surrealdb/surreal`.
pub fn find_binary(path_var: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    let on_path = path_var
        .into_iter()
        .flat_map(|p| p.split(':'))
        .filter(|d| !d.is_empty())
        .map(|d| Path::new(d).join("surreal"));
    let installer = home.map(|h| Path::new(h).join(".surrealdb/surreal"));
    on_path.chain(installer).find(|p| p.is_file())
}

pub fn log_path(s: &Settings) -> PathBuf {
    s.dir
        .parent()
        .map_or_else(|| PathBuf::from("surreal.log"), |p| p.join("surreal.log"))
}

/// The one failure line for a server that is missing or never came up. The
/// password is never printed.
pub fn unavailable(s: &Settings, why: &str) -> String {
    format!(
        "board server unavailable: {why} `surreal db start --no-banner --bind {} ... surrealkv://{}`; see {}. Install: {INSTALL_HINT}",
        s.url,
        s.dir.display(),
        log_path(s).display()
    )
}

/// The arguments the server is started with. The schema-refusal hint
/// (`pkill -f 'bind <url>'`) must match this command line; a test holds them together.
pub fn server_args(s: &Settings) -> Vec<String> {
    vec![
        "db".to_owned(),
        "start".into(),
        "--no-banner".into(),
        "--log".into(),
        "warn".into(),
        "--bind".into(),
        s.url.clone(),
        "--username".into(),
        s.user.clone(),
        "--password".into(),
        s.pass.clone(),
        format!("surrealkv://{}", s.dir.display()),
    ]
}

fn spawn(bin: &Path, s: &Settings) -> Result<std::process::Child, String> {
    if !bin.is_file() {
        return Err(format!("spawn {}: no such file", bin.display()));
    }
    std::fs::create_dir_all(&s.dir).map_err(|e| format!("data dir {}: {e}", s.dir.display()))?;
    let log = log_path(s);
    let out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .map_err(|e| format!("log {}: {e}", log.display()))?;
    let err = out.try_clone().map_err(|e| e.to_string())?;
    let args = server_args(s);
    // Own session, so the server outlives the call (and its terminal) that started
    // it. `setsid` calls setsid(2) on itself and execs when it is not a group
    // leader, which a spawned child is not, so the pid stays the server's. Without
    // the utility, fall back to a process group of its own.
    let mut cmd = match find_tool("setsid") {
        Some(setsid) => {
            let mut c = Command::new(setsid);
            c.arg(bin).args(&args);
            c
        }
        None => {
            let mut c = Command::new(bin);
            c.args(&args).process_group(0);
            c
        }
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .spawn()
        .map_err(|e| format!("spawn {}: {e}", bin.display()))
}

fn find_tool(name: &str) -> Option<PathBuf> {
    std::env::var("PATH")
        .ok()?
        .split(':')
        .filter(|d| !d.is_empty())
        .map(|d| Path::new(d).join(name))
        .find(|p| p.is_file())
}

/// Waits on the child from a thread so it never lingers as a zombie in a
/// long-lived process (the MCP server). The server outlives us by design, so the
/// thread simply ends with the process.
fn reap(mut child: std::process::Child) {
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}

/// Makes sure a server answers. If one does, nothing is spawned. Otherwise starts
/// `bin` (default: the resolved `surreal`) and polls `/health` until `t.deadline`.
/// A spawn that loses the port to another starter is fine: the winner answers
/// `/health` and this connects to it.
pub fn ensure_up(s: &Settings, t: &Timing, bin: Option<&Path>) -> Result<Served, String> {
    if healthy(&s.url, t.probe) {
        return Ok(Served { spawned_pid: None });
    }
    let found;
    let bin = match bin {
        Some(b) => b,
        None => {
            found = find_binary(
                std::env::var("PATH").ok().as_deref(),
                std::env::var("HOME").ok().as_deref(),
            )
            .ok_or_else(|| {
                unavailable(
                    s,
                    "binary not found in PATH or ~/.surrealdb/surreal; could not run",
                )
            })?;
            &found
        }
    };
    // Two starters must not both open the data directory: the one that loses the
    // port has by then already touched the store, and a second server on the same
    // directory corrupted it in testing. Starting is serialised by a lock beside
    // the data dir; whoever waits re-probes and finds the winner's server up.
    let state_dir = s.dir.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(state_dir).map_err(|e| {
        unavailable(
            s,
            &format!("state dir {}: {e}; could not run", state_dir.display()),
        )
    })?;
    let _starting = super::lock::acquire_named(state_dir, START_LOCK, t.deadline + t.deadline)
        .map_err(|e| unavailable(s, &format!("{e}; could not run")))?;
    if healthy(&s.url, t.probe) {
        return Ok(Served { spawned_pid: None });
    }
    let mut child = spawn(bin, s).map_err(|e| unavailable(s, &format!("{e}; ran")))?;
    let pid = child.id();
    let start = Instant::now();
    loop {
        if healthy(&s.url, t.probe) {
            reap(child);
            return Ok(Served {
                spawned_pid: Some(pid),
            });
        }
        // Reap it if it already exited (e.g. it lost the port to another starter);
        // keep polling, since the winner is about to answer.
        let _ = child.try_wait();
        if start.elapsed() >= t.deadline {
            reap(child);
            return Err(unavailable(
                s,
                &format!(
                    "no /health answer within {} ms from",
                    t.deadline.as_millis()
                ),
            ));
        }
        std::thread::sleep(t.poll_every);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_line_and_body() {
        let (c, b) = parse_response("HTTP/1.1 200 OK\r\ncontent-length: 4\r\n\r\nok\r\n").unwrap();
        assert_eq!((c, b.as_str()), (200, "ok"));
        let (c, b) = parse_response("HTTP/1.1 503 Service Unavailable\r\n\r\n").unwrap();
        assert_eq!((c, b.as_str()), (503, ""));
        let chunked = "HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\nF\r\nsurrealdb-2.7.0\r\n0\r\n\r\n";
        assert_eq!(parse_response(chunked).unwrap().1, "surrealdb-2.7.0");
        assert!(parse_response("garbage").is_err());
        assert!(parse_response("").is_err());
    }

    #[test]
    fn finds_the_binary_on_path_then_in_the_installer_dir() {
        let root = std::env::temp_dir().join(format!("ab-bin-{}", std::process::id()));
        let (a, b) = (root.join("a"), root.join("home/.surrealdb"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let home = root.join("home");
        let home = home.to_str().unwrap();
        assert!(find_binary(Some(a.to_str().unwrap()), Some(home)).is_none());
        std::fs::write(b.join("surreal"), "").unwrap();
        assert_eq!(
            find_binary(Some(a.to_str().unwrap()), Some(home)).unwrap(),
            b.join("surreal")
        );
        std::fs::write(a.join("surreal"), "").unwrap();
        assert_eq!(
            find_binary(Some(a.to_str().unwrap()), Some(home)).unwrap(),
            a.join("surreal"),
            "PATH wins"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_failure_line_has_command_log_and_install_hint_and_no_password() {
        let s = Settings::defaults_for_test("/tmp/ab-x/data", "127.0.0.1:18720");
        let line = unavailable(&s, "could not run");
        assert!(!line.contains("secret"), "{line}");
        assert_eq!(
            line,
            "board server unavailable: could not run `surreal db start --no-banner --bind 127.0.0.1:18720 ... surrealkv:///tmp/ab-x/data`; see /tmp/ab-x/surreal.log. Install: curl -sSf https://install.surrealdb.com | sh -s -- --version 2.7.0"
        );
        assert!(!line.contains('\n'));
    }

    #[test]
    fn a_missing_binary_and_a_server_that_never_answers_are_one_line_each() {
        let dir = std::env::temp_dir().join(format!("ab-down-{}", std::process::id()));
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let s = Settings::defaults_for_test(
            dir.join("data").to_str().unwrap(),
            &format!("127.0.0.1:{port}"),
        );
        let quick = Timing {
            probe: Duration::from_millis(100),
            poll_every: Duration::from_millis(20),
            deadline: Duration::from_millis(400),
            connect: Duration::from_millis(400),
        };
        // spawn fails: no such binary
        let err = ensure_up(&s, &quick, Some(Path::new("/nonexistent/surreal")))
            .err()
            .unwrap();
        println!("server-absent line: {err}");
        assert!(
            err.starts_with("board server unavailable: spawn /nonexistent/surreal"),
            "{err}"
        );
        assert!(err.contains("Install: curl"), "{err}");
        assert!(!err.contains('\n'));
        // spawn works but nothing ever answers /health: bounded, not a hang
        let t = Instant::now();
        let err = ensure_up(&s, &quick, Some(Path::new("/bin/true")))
            .err()
            .unwrap();
        assert!(t.elapsed() < Duration::from_secs(3), "must not hang");
        println!("never-answers line: {err}");
        assert!(err.contains("no /health answer within 400 ms"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
