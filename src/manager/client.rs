//! The herdr socket, one request per connection: NDJSON out, one NDJSON line back.
//! Socket discovery, the protocol pin and the deadline live here so no verb can
//! skip them. Every failure is one line starting `herdr:`; nothing here shells the
//! herdr binary.

use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// The herdr wire protocol this build speaks; a server on any other is refused.
pub const PROTOCOL: i64 = 22;
const MAX_REPLY: usize = 16 << 20;

pub struct Client {
    sock: PathBuf,
    deadline: Duration,
    pub version: String,
}

#[derive(Deserialize)]
struct Pong {
    protocol: i64,
    version: String,
}

/// Per-call deadline. `AGENTBOARD_HERDR_TIMEOUT_MS` overrides the 2s default.
fn deadline() -> Duration {
    let ms = std::env::var("AGENTBOARD_HERDR_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&ms| ms > 0)
        .unwrap_or(2000);
    Duration::from_millis(ms)
}

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// The socket to talk to. A path the caller named (`$AGENTBOARD_HERDR_SOCK`, else
/// `$HERDR_SOCKET`) is used or refused, never skipped: a missing one must not
/// quietly send the manager to a different herdr. Only when neither is set does
/// the first existing `$XDG_CONFIG_HOME/herdr/herdr.sock`, `~/.config/herdr/herdr.sock`
/// win, and when none exists the error names all four.
fn discover() -> Result<PathBuf, String> {
    for var in ["AGENTBOARD_HERDR_SOCK", "HERDR_SOCKET"] {
        if let Some(p) = env_path(var) {
            return if p.exists() {
                Ok(p)
            } else {
                Err(format!(
                    "herdr: ${var} is set but {} does not exist",
                    p.display()
                ))
            };
        }
    }
    let defaults = [
        env_path("XDG_CONFIG_HOME").map(|d| d.join("herdr/herdr.sock")),
        env_path("HOME").map(|d| d.join(".config/herdr/herdr.sock")),
    ];
    if let Some(p) = defaults.iter().flatten().find(|p| p.exists()) {
        return Ok(p.clone());
    }
    let shown = |p: &Option<PathBuf>, label: &str| match p {
        Some(p) => p.display().to_string(),
        None => format!("{label} (unset)"),
    };
    Err(format!(
        "herdr: no socket found; tried $AGENTBOARD_HERDR_SOCK (unset), $HERDR_SOCKET (unset), {}, {}",
        shown(&defaults[0], "$XDG_CONFIG_HOME/herdr/herdr.sock"),
        shown(&defaults[1], "~/.config/herdr/herdr.sock"),
    ))
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn next_id() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    format!(
        "agentboard-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

impl Client {
    /// Finds the socket and pins it once, so a wrong protocol or a dead socket
    /// fails before any verb runs. `call` pins again before every method.
    pub fn connect() -> Result<Client, String> {
        Client::at(discover()?, deadline())
    }

    fn at(sock: PathBuf, deadline: Duration) -> Result<Client, String> {
        let mut c = Client {
            sock,
            deadline,
            version: String::new(),
        };
        c.version = c.pin(Instant::now() + deadline)?.version;
        Ok(c)
    }

    fn timed_out(&self, what: &str) -> String {
        format!(
            "herdr: {what} timed out after {}ms (AGENTBOARD_HERDR_TIMEOUT_MS)",
            self.deadline.as_millis()
        )
    }

    /// A bounded connect: `UnixStream::connect` blocks forever on a socket whose
    /// accept queue is full, so it runs on a thread the deadline can abandon.
    fn dial(&self, end: Instant) -> Result<UnixStream, String> {
        let (tx, rx) = mpsc::channel();
        let sock = self.sock.clone();
        thread::spawn(move || {
            let _ = tx.send(UnixStream::connect(sock));
        });
        let left = end.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(r) => r.map_err(|e| format!("herdr: connect {}: {e}", self.sock.display())),
            Err(_) => Err(self.timed_out(&format!("connect {}", self.sock.display()))),
        }
    }

    /// The protocol pin: `ping` on a connection of its own, anything but protocol
    /// 22 refused naming both numbers. herdr answers one request and closes, so a
    /// method cannot ride the pinged connection; `call` pins right before every
    /// method instead, and nothing is sent past a refusal.
    fn pin(&self, end: Instant) -> Result<Pong, String> {
        let mut s = self.dial(end)?;
        let pong: Pong = serde_json::from_value(self.exchange(&mut s, end, "ping", json!({}))?)
            .map_err(|e| {
                format!(
                    "herdr: ping: unexpected reply shape: {}",
                    one_line(&e.to_string())
                )
            })?;
        if pong.protocol != PROTOCOL {
            return Err(format!(
                "herdr: speaks protocol {}, want {PROTOCOL}",
                pong.protocol
            ));
        }
        Ok(pong)
    }

    /// One method, pinned first, the whole of it (both connects, ping, request)
    /// bounded by the deadline: a server that dribbles bytes cannot stretch it.
    /// `Ok` is the `result` value.
    pub(super) fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let end = Instant::now() + self.deadline;
        self.pin(end)?;
        let mut s = self.dial(end)?;
        self.exchange(&mut s, end, method, params)
    }

    /// A long-poll method (`agent.start`, `agent.wait`): pinned under the normal
    /// deadline, then the method itself gets `wait` plus the normal deadline, so the
    /// caller's own timeout governs and a timeout message names the real budget.
    pub(super) fn call_for(
        &self,
        method: &str,
        params: Value,
        wait: Duration,
    ) -> Result<Value, String> {
        self.pin(Instant::now() + self.deadline)?;
        let long = Client {
            sock: self.sock.clone(),
            deadline: self.deadline + wait,
            version: self.version.clone(),
        };
        let end = Instant::now() + long.deadline;
        let mut s = long.dial(end)?;
        long.exchange(&mut s, end, method, params)
    }

    /// Writes one request and reads its one reply line.
    fn exchange(
        &self,
        s: &mut UnixStream,
        end: Instant,
        method: &str,
        params: Value,
    ) -> Result<Value, String> {
        let id = next_id();
        let mut line = json!({"id": id, "method": method, "params": params}).to_string();
        line.push('\n');

        let left = end
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| self.timed_out(method))?;
        s.set_write_timeout(Some(left))
            .map_err(|e| format!("herdr: {method}: {e}"))?;
        s.write_all(line.as_bytes()).map_err(|e| match e.kind() {
            ErrorKind::WouldBlock | ErrorKind::TimedOut => self.timed_out(method),
            _ => format!("herdr: {method}: write: {e}"),
        })?;

        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        let nl = loop {
            if let Some(i) = buf.iter().position(|&b| b == b'\n') {
                break i;
            }
            if buf.len() > MAX_REPLY {
                return Err(format!("herdr: {method}: reply over {MAX_REPLY} bytes"));
            }
            let left = end
                .checked_duration_since(Instant::now())
                .filter(|d| !d.is_zero())
                .ok_or_else(|| self.timed_out(method))?;
            s.set_read_timeout(Some(left))
                .map_err(|e| format!("herdr: {method}: {e}"))?;
            match s.read(&mut chunk) {
                Ok(0) => return Err(format!("herdr: {method}: connection closed before a reply")),
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(e)
                    if matches!(
                        e.kind(),
                        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                    ) => {}
                Err(e) => return Err(format!("herdr: {method}: read: {e}")),
            }
        };
        let first = &buf[..=nl];
        let reply: Value = serde_json::from_slice(first).map_err(|e| {
            format!(
                "herdr: {method}: malformed reply: {}",
                one_line(&e.to_string())
            )
        })?;
        if reply.get("id").and_then(Value::as_str) != Some(id.as_str()) {
            return Err(format!(
                "herdr: {method}: reply id does not match the request"
            ));
        }
        if let Some(err) = reply.get("error") {
            let field = |k: &str| err.get(k).and_then(Value::as_str).unwrap_or("?");
            return Err(format!(
                "herdr: {method}: {}: {}",
                one_line(field("code")),
                one_line(field("message"))
            ));
        }
        reply
            .get("result")
            .cloned()
            .ok_or_else(|| format!("herdr: {method}: reply has neither result nor error"))
    }
}
