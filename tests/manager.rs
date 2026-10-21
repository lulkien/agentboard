//! `agentboard manager` against a stub herdr: a UnixListener on a temp path
//! answering scripted NDJSON. Each case runs the real binary with
//! `AGENTBOARD_HERDR_SOCK` pointing at the stub, and HOME/XDG_CONFIG_HOME/
//! HERDR_SOCKET pointed away, so no test can reach the live herdr session.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Maps a request to the `result` to send back; `None` accepts and never answers.
type Script =
    Box<dyn Fn(&str, &Value) -> Option<Result<Value, (&'static str, &'static str)>> + Send + Sync>;

struct Stub {
    dir: PathBuf,
    sock: PathBuf,
    log: Arc<Mutex<Vec<String>>>,
    /// The methods each connection carried, in order.
    conns: Arc<Mutex<Vec<Vec<String>>>>,
}

impl Drop for Stub {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn stub(name: &str, script: Script) -> Stub {
    let dir =
        std::env::temp_dir().join(format!("agentboard-manager-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("herdr.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let conns: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(Vec::new()));
    let (seen, by_conn) = (Arc::clone(&log), Arc::clone(&conns));
    let script = Arc::new(script);
    thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let (seen, by_conn, script) = (seen.clone(), by_conn.clone(), script.clone());
            let n = {
                let mut c = by_conn.lock().unwrap();
                c.push(Vec::new());
                c.len() - 1
            };
            thread::spawn(move || {
                let mut reader = BufReader::new(conn.try_clone().unwrap());
                let mut conn = conn;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let Ok(req) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    let method = req["method"].as_str().unwrap_or("").to_owned();
                    seen.lock().unwrap().push(method.clone());
                    by_conn.lock().unwrap()[n].push(method.clone());
                    let id = req["id"].clone();
                    let reply = match script(&method, &req["params"]) {
                        None => continue,
                        Some(Ok(result)) => json!({"id": id, "result": result}),
                        Some(Err((code, message))) => {
                            json!({"id": id, "error": {"code": code, "message": message}})
                        }
                    };
                    let _ = writeln!(conn, "{reply}");
                    return; // herdr answers one request and closes
                }
            });
        }
    });
    Stub {
        dir,
        sock,
        log,
        conns,
    }
}

fn pong(protocol: i64) -> Value {
    json!({"type": "pong", "version": "0.9.0", "protocol": protocol})
}

fn agent(name: &str, pane: &str, status: &str) -> Value {
    json!({"name": name, "agent": "claude", "agent_status": status, "workspace_id": "w1",
           "tab_id": "w1:t1", "pane_id": pane, "cwd": "/repo", "foreground_cwd": "/repo",
           "focused": false, "terminal_id": "t", "revision": 1})
}

fn ok_script() -> Script {
    Box::new(|method, _| {
        Some(Ok(match method {
            "ping" => pong(22),
            "agent.list" => {
                json!({"type": "agent_list", "agents": [agent("coder", "w1:p2", "working")]})
            }
            "tab.list" => json!({"type": "tab_list", "tabs": [
                {"tab_id": "w1:t1", "workspace_id": "w1", "number": 1, "label": "main",
                 "focused": true, "pane_count": 2, "agent_status": "working"}]}),
            "session.snapshot" => json!({"type": "session_snapshot", "snapshot": {
                "version": "0.9.0", "protocol": 22, "workspaces": [{}], "tabs": [{}], "panes": [{}, {}],
                "layouts": [], "agents": [{}], "focused_workspace_id": "w1",
                "focused_tab_id": "w1:t1", "focused_pane_id": "w1:p2"}}),
            "agent.read" => json!({"type": "pane_read", "read": {"text": "screen text\n"}}),
            _ => return Some(Err(("method_not_found", "no such method"))),
        }))
    })
}

fn manager(sock: &Path, home: &Path, args: &[&str], extra: &[(&str, &str)]) -> (Output, Duration) {
    manager_env(Some(sock), home, args, extra)
}

/// `sock` is `$AGENTBOARD_HERDR_SOCK`; `None` leaves it unset.
fn manager_env(
    sock: Option<&Path>,
    home: &Path,
    args: &[&str],
    extra: &[(&str, &str)],
) -> (Output, Duration) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_agentboard"));
    cmd.arg("manager")
        .args(args)
        .env_remove("AGENTBOARD_HERDR_SOCK")
        .env("HERDR_SOCKET", home.join("none"))
        .env("XDG_CONFIG_HOME", home.join("xdg"))
        .env("HOME", home);
    if let Some(sock) = sock {
        cmd.env("AGENTBOARD_HERDR_SOCK", sock);
    }
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let start = Instant::now();
    let out = cmd.output().unwrap();
    (out, start.elapsed())
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn one_line(err: &str) -> bool {
    err.trim_end().lines().count() == 1
}

#[test]
fn happy_path_reads_and_only_read_methods_are_sent() {
    let s = stub("happy", ok_script());
    for (args, want) in [
        (vec!["agent", "list"], "coder"),
        (vec!["tab", "list"], "main"),
        (vec!["snapshot"], "workspaces 1 tabs 1 panes 2 agents 1"),
        (vec!["agent", "read", "coder"], "screen text"),
    ] {
        let (o, _) = manager(&s.sock, &s.dir, &args, &[]);
        assert!(o.status.success(), "{args:?}: {}", text(&o.stderr));
        assert!(
            text(&o.stdout).contains(want),
            "{args:?}: {}",
            text(&o.stdout)
        );
    }
    let log = s.log.lock().unwrap();
    let read_only = [
        "ping",
        "agent.list",
        "tab.list",
        "session.snapshot",
        "agent.read",
    ];
    assert!(
        log.iter().all(|m| read_only.contains(&m.as_str())),
        "{log:?}"
    );
    let conns = s.conns.lock().unwrap();
    assert!(
        conns.iter().all(|c| c.len() == 1),
        "one request per connection: {conns:?}"
    );
    for (i, c) in conns.iter().enumerate().filter(|(_, c)| c[0] != "ping") {
        assert_eq!(
            conns[i - 1],
            ["ping"],
            "{} was not pinned on the connection before it: {conns:?}",
            c[0]
        );
    }
    let methods = conns.iter().filter(|c| c[0] != "ping").count();
    assert_eq!(methods, 4, "{conns:?}");
    assert_eq!(
        conns.len(),
        2 * methods + methods,
        "connect probe + ping + method per verb: {conns:?}"
    );
}

#[test]
fn protocol_21_is_refused_naming_both_numbers() {
    let s = stub("p21", Box::new(|_, _| Some(Ok(pong(21)))));
    let (o, _) = manager(&s.sock, &s.dir, &["agent", "list"], &[]);
    assert_eq!(o.status.code(), Some(1));
    let err = text(&o.stderr);
    assert!(
        err.contains("protocol 21") && err.contains("want 22"),
        "{err}"
    );
    assert!(one_line(&err), "{err}");
    assert_eq!(
        *s.log.lock().unwrap(),
        ["ping"],
        "nothing runs past the pin"
    );
}

#[test]
fn no_socket_anywhere_names_all_four_paths() {
    let home =
        std::env::temp_dir().join(format!("agentboard-manager-{}-nosock", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    // `manager_env` points HERDR_SOCKET at home/none, which does not exist, so the
    // variable is removed here to reach the all-paths message.
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_agentboard"));
    let o = cmd
        .args(["manager", "agent", "list"])
        .env_remove("AGENTBOARD_HERDR_SOCK")
        .env_remove("HERDR_SOCKET")
        .env("XDG_CONFIG_HOME", home.join("xdg"))
        .env("HOME", &home)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    let err = text(&o.stderr);
    for path in [
        "$AGENTBOARD_HERDR_SOCK",
        "$HERDR_SOCKET",
        &home.join("xdg/herdr/herdr.sock").display().to_string(),
        &home.join(".config/herdr/herdr.sock").display().to_string(),
    ] {
        assert!(err.contains(path), "{path} missing from: {err}");
    }
    assert!(one_line(&err), "{err}");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_named_socket_that_is_missing_is_refused_never_replaced_by_another() {
    let other = stub("fallthrough", ok_script());
    let missing = other.dir.join("staging.sock");
    let (o, _) = manager_env(
        Some(&missing),
        &other.dir,
        &["agent", "list"],
        &[("HERDR_SOCKET", other.sock.to_str().unwrap())],
    );
    assert_eq!(o.status.code(), Some(1));
    let err = text(&o.stderr);
    assert!(
        err.contains("$AGENTBOARD_HERDR_SOCK") && err.contains(&missing.display().to_string()),
        "{err}"
    );
    assert!(one_line(&err), "{err}");
    assert!(
        other.log.lock().unwrap().is_empty(),
        "a request reached the other stub"
    );
}

#[test]
fn a_stub_that_never_answers_hits_the_deadline() {
    let s = stub("silent", Box::new(|_, _| None));
    let (o, took) = manager(
        &s.sock,
        &s.dir,
        &["agent", "list"],
        &[("AGENTBOARD_HERDR_TIMEOUT_MS", "300")],
    );
    assert_eq!(o.status.code(), Some(1));
    let err = text(&o.stderr);
    assert!(err.contains("timed out after 300ms"), "{err}");
    assert!(one_line(&err), "{err}");
    assert!(took < Duration::from_millis(1500), "took {took:?}");
}

unsafe extern "C" {
    fn listen(fd: i32, backlog: i32) -> i32;
}

#[test]
fn a_socket_that_stops_accepting_hits_the_deadline_on_connect() {
    let dir = std::env::temp_dir().join(format!("agentboard-manager-{}-full", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("herdr.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    // A backlog of 1 and a client that is never accepted: the next connect blocks.
    assert_eq!(unsafe { listen(listener.as_raw_fd(), 1) }, 0);
    let mut parked = Vec::new();
    for _ in 0..4 {
        let sock = sock.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(UnixStream::connect(sock));
        });
        if let Ok(Ok(c)) = rx.recv_timeout(Duration::from_millis(200)) {
            parked.push(c);
        }
    }
    let (o, took) = manager(
        &sock,
        &dir,
        &["agent", "list"],
        &[("AGENTBOARD_HERDR_TIMEOUT_MS", "300")],
    );
    assert_eq!(o.status.code(), Some(1));
    let err = text(&o.stderr);
    assert!(parked.len() < 4, "the accept queue never filled");
    assert!(
        err.contains("connect") && err.contains("timed out after 300ms"),
        "{err}"
    );
    assert!(one_line(&err), "{err}");
    assert!(took < Duration::from_millis(1500), "took {took:?}");
    drop((listener, parked));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_malformed_reply_is_one_error_line_not_a_panic() {
    let dir = std::env::temp_dir().join(format!("agentboard-manager-{}-junk", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("herdr.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    thread::spawn(move || {
        for mut conn in listener.incoming().flatten() {
            let mut line = String::new();
            let _ = BufReader::new(&conn).read_line(&mut line);
            let _ = conn.write_all(b"{not json\n");
        }
    });
    let (o, _) = manager(&sock, &dir, &["agent", "list"], &[]);
    assert_eq!(o.status.code(), Some(1));
    let err = text(&o.stderr);
    assert!(err.contains("malformed reply"), "{err}");
    assert!(!err.contains("panicked"), "{err}");
    assert!(one_line(&err), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tmux_is_refused_by_name_before_any_socket_is_touched() {
    let s = stub("tmux", ok_script());
    let (o, _) = manager(&s.sock, &s.dir, &["--tmux", "agent", "list"], &[]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        text(&o.stderr).contains("--tmux: no tmux backend yet; only --herdr is implemented"),
        "{}",
        text(&o.stderr)
    );
    assert!(s.log.lock().unwrap().is_empty());
}
