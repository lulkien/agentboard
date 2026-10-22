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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Maps a request to the `result` to send back; `None` accepts and never answers.
type Script =
    Box<dyn Fn(&str, &Value) -> Option<Result<Value, (&'static str, &'static str)>> + Send + Sync>;

struct Stub {
    raw: Arc<Mutex<Vec<String>>>,
    dir: PathBuf,
    sock: PathBuf,
    log: Arc<Mutex<Vec<String>>>,
    /// The methods each connection carried, in order.
    conns: Arc<Mutex<Vec<Vec<String>>>>,
    /// Every non-ping request as `{method, params}`, in arrival order.
    reqs: Arc<Mutex<Vec<Value>>>,
}

impl Drop for Stub {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// What a stub does with an `events.subscribe` connection after its ack.
type Stream = Arc<dyn Fn(&mut UnixStream) + Send + Sync>;

fn stub(name: &str, script: Script) -> Stub {
    stub_streaming(name, script, None)
}

fn stub_streaming(name: &str, script: Script, stream: Option<Stream>) -> Stub {
    let dir =
        std::env::temp_dir().join(format!("agentboard-manager-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("herdr.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let raw = Arc::new(Mutex::new(Vec::new()));
    let raw_thread = raw.clone();
    let log = Arc::new(Mutex::new(Vec::new()));
    let conns: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(Vec::new()));
    let reqs: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let (seen, by_conn, by_req) = (Arc::clone(&log), Arc::clone(&conns), Arc::clone(&reqs));
    let script = Arc::new(script);
    thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let raw = raw_thread.clone();
            let (seen, by_conn, by_req, script, stream) = (
                seen.clone(),
                by_conn.clone(),
                by_req.clone(),
                script.clone(),
                stream.clone(),
            );
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
                    raw.lock().unwrap().push(line.clone());
                    let method = req["method"].as_str().unwrap_or("").to_owned();
                    seen.lock().unwrap().push(method.clone());
                    by_conn.lock().unwrap()[n].push(method.clone());
                    if method != "ping" {
                        by_req
                            .lock()
                            .unwrap()
                            .push(json!({"method": method, "params": req["params"]}));
                    }
                    let id = req["id"].clone();
                    let reply = match script(&method, &req["params"]) {
                        None => continue,
                        Some(Ok(result)) => json!({"id": id, "result": result}),
                        Some(Err((code, message))) => {
                            json!({"id": id, "error": {"code": code, "message": message}})
                        }
                    };
                    let _ = writeln!(conn, "{reply}");
                    if method == "events.subscribe"
                        && let Some(stream) = &stream
                    {
                        stream(&mut conn);
                    }
                    return; // herdr answers one request and closes
                }
            });
        }
    });
    Stub {
        raw,
        dir,
        sock,
        log,
        conns,
        reqs,
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
    // Manager-only commands now check the board schema too. Supply an actual
    // throwaway server, while keeping HOME isolated from the user's herdr config.
    #[cfg(feature = "surreal")]
    let board_server = ScratchBoard::start(home);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_agentboard"));
    cmd.arg("manager")
        .args(args)
        .env_remove("AGENTBOARD_HERDR_SOCK")
        .env("HERDR_SOCKET", home.join("none"))
        .env("XDG_CONFIG_HOME", home.join("xdg"))
        .env("HOME", home)
        .env_remove("AGENTBOARD_SURREAL_URL")
        .env("AGENTBOARD_DB", home.join("board"));
    #[cfg(feature = "surreal")]
    cmd.env("AGENTBOARD_SURREAL_URL", &board_server.url);
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

#[cfg(feature = "surreal")]
struct ScratchBoard {
    child: std::process::Child,
    url: String,
}
#[cfg(feature = "surreal")]
impl ScratchBoard {
    fn start(home: &Path) -> Self {
        use std::process::Stdio;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = listener.local_addr().unwrap().to_string();
        drop(listener);
        std::fs::create_dir_all(home).unwrap();
        let binary = std::env::var("HOME")
            .map(|h| PathBuf::from(h).join(".surrealdb/surreal"))
            .unwrap();
        let binary = if binary.is_file() {
            binary
        } else {
            PathBuf::from("surreal")
        };
        let child = Command::new(binary)
            .args([
                "db",
                "start",
                "--no-banner",
                "--log",
                "error",
                "--bind",
                &url,
                "--username",
                "root",
                "--password",
                "root",
                "memory",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let board = Self { child, url };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !Command::new("curl")
            .args(["-sf", &format!("http://{}/health", board.url)])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
        {
            assert!(Instant::now() < deadline, "scratch board never started");
            thread::sleep(Duration::from_millis(20));
        }
        board
    }
}
#[cfg(feature = "surreal")]
impl Drop for ScratchBoard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
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

/// Replies shaped like herdr's for every mutating method, with `coder` in pane
/// `w1:p2` at `status`.
fn write_script(status: &'static str) -> Script {
    Box::new(move |method, _| {
        Some(Ok(match method {
            "ping" => pong(22),
            "agent.list" => {
                json!({"type": "agent_list", "agents": [agent("coder", "w1:p2", status)]})
            }
            "tab.create" => json!({"type": "tab_created",
                "tab": {"tab_id": "w1:t5"}, "root_pane": {"pane_id": "w1:p6"}}),
            "pane.split" => json!({"type": "pane_info", "pane": {"pane_id": "w1:p7"}}),
            _ => json!({"type": "ok"}),
        }))
    })
}

/// Runs one verb against a fresh stub; returns (exit code, stdout, stderr, requests).
fn run_write(status: &'static str, args: &[&str]) -> (Option<i32>, String, String, Vec<Value>) {
    static N: AtomicUsize = AtomicUsize::new(0);
    let s = stub(
        &format!("write{}", N.fetch_add(1, Ordering::Relaxed)),
        write_script(status),
    );
    let (o, _) = manager(&s.sock, &s.dir, args, &[]);
    let reqs = s.reqs.lock().unwrap().clone();
    (o.status.code(), text(&o.stdout), text(&o.stderr), reqs)
}

fn show(args: &[&str], out: &str, reqs: &[Value]) {
    println!("$ agentboard manager {}", args.join(" "));
    for r in reqs {
        println!("  -> {r}");
    }
    println!("  out: {}", out.trim_end());
}

#[test]
fn each_mutating_verb_sends_its_method_and_params() {
    let cases: [(Vec<&str>, Value, &str); 8] = [
        (
            vec!["tab", "new", "--cwd", "/repo", "--label", "team"],
            json!({"method": "tab.create", "params": {"cwd": "/repo", "label": "team"}}),
            "tab w1:t5 root pane w1:p6",
        ),
        (
            vec![
                "pane", "split", "w1:p6", "--right", "--cwd", "/repo", "--env", "A=1", "--env",
                "B=x=y",
            ],
            json!({"method": "pane.split", "params": {"direction": "right",
                "target_pane_id": "w1:p6", "cwd": "/repo", "env": {"A": "1", "B": "x=y"}}}),
            "pane w1:p7 split right of w1:p6",
        ),
        (
            vec!["pane", "split", "w1:p6", "--down"],
            json!({"method": "pane.split", "params": {"direction": "down", "target_pane_id": "w1:p6"}}),
            "pane w1:p7 split down of w1:p6",
        ),
        (
            vec![
                "agent",
                "start",
                "coder",
                "--kind",
                "claude",
                "--pane",
                "w1:p6",
                "--timeout-ms",
                "120000",
            ],
            json!({"method": "agent.start", "params": {"name": "coder", "kind": "claude",
                "pane_id": "w1:p6", "timeout_ms": 120000}}),
            "started coder (claude) in w1:p6",
        ),
        (
            vec!["agent", "prompt", "coder", "- claim T1"],
            json!({"method": "agent.prompt", "params": {"target": "coder", "text": "- claim T1"}}),
            "prompted coder",
        ),
        (
            vec!["agent", "keys", "coder", "down", "enter"],
            json!({"method": "agent.send_keys", "params": {"target": "coder", "keys": ["down", "enter"]}}),
            "sent down enter to coder",
        ),
        (
            vec!["agent", "focus", "coder"],
            json!({"method": "agent.focus", "params": {"target": "coder"}}),
            "focused coder",
        ),
        (
            vec!["pane", "close", "w1:p9"],
            json!({"method": "pane.close", "params": {"pane_id": "w1:p9"}}),
            "closed pane w1:p9",
        ),
    ];
    for (args, want, out_line) in cases {
        let (code, out, err, reqs) = run_write("idle", &args);
        show(&args, &out, &reqs);
        assert_eq!(code, Some(0), "{args:?}: {err}");
        assert_eq!(out.trim_end(), out_line, "{args:?}");
        let sent: Vec<&Value> = reqs
            .iter()
            .filter(|r| r["method"] != "agent.list")
            .collect();
        assert_eq!(sent, [&want], "{args:?}");
    }
}

#[test]
fn argv_after_dashdash_reaches_agent_start_verbatim() {
    let claude = [
        "--strict-mcp-config",
        "--mcp-config=/home/u/.claude/coding-mcp.json",
        "--append-system-prompt-file=/home/u/.claude/caveman-full.md",
        "--model",
        "sonnet",
    ];
    for (name, kind, argv) in [
        ("coder", "claude", claude.to_vec()),
        ("reviewer", "opencode", vec!["--standalone"]),
    ] {
        let mut args = vec![
            "agent", "start", name, "--kind", kind, "--pane", "w1:p6", "--",
        ];
        args.extend(&argv);
        let (code, out, err, reqs) = run_write("idle", &args);
        show(&args, &out, &reqs);
        assert_eq!(code, Some(0), "{err}");
        assert_eq!(reqs.len(), 1, "{reqs:?}");
        assert_eq!(reqs[0]["params"]["args"], json!(argv), "{reqs:?}");
        assert!(
            reqs[0]["params"].get("timeout_ms").is_none(),
            "no timeout asked, none sent: {reqs:?}"
        );
    }
}

#[test]
fn a_working_agent_is_refused_exit_3_and_nothing_is_closed_until_force() {
    for (verb, closes) in [
        (vec!["pane", "close", "w1:p2"], "pane.close"),
        (vec!["pane", "close", "w1:p2", "--force"], "pane.close"),
        (vec!["agent", "rename", "coder", "other"], "agent.rename"),
        (
            vec!["agent", "rename", "coder", "other", "--force"],
            "agent.rename",
        ),
        (vec!["down"], "pane.close"),
    ] {
        let forced = verb.contains(&"--force");
        let (code, out, err, reqs) = run_write("working", &verb);
        show(&verb, &out, &reqs);
        let sent: Vec<&str> = reqs.iter().map(|r| r["method"].as_str().unwrap()).collect();
        if forced {
            assert!(sent.contains(&closes), "{verb:?}: {sent:?}");
            assert_eq!(code, Some(0), "{verb:?}: {err}");
            assert!(out.contains("forced: coder was working"), "{out}");
        } else {
            assert_eq!(code, Some(3), "{verb:?}: {err}");
            assert!(
                err.contains("refusing to ") && err.contains("status=working"),
                "{err}"
            );
            assert!(one_line(&err), "{err}");
            assert!(!sent.contains(&closes), "{verb:?} sent {closes}: {sent:?}");
            assert!(out.is_empty(), "{out}");
        }
    }
}

#[test]
fn down_names_the_agent_in_its_refusal() {
    let (code, _, err, _) = run_write("working", &["down"]);
    assert_eq!(code, Some(3));
    assert!(
        err.contains("refusing to stop coder: status=working"),
        "{err}"
    );
}

#[test]
fn renew_refuses_a_working_pane_before_typing_anything() {
    let (code, _, err, reqs) = run_write("working", &["renew", "--names", "coder"]);
    assert_eq!(code, Some(3), "{err}");
    assert!(
        err.contains("refusing to clear coder: status=working"),
        "{err}"
    );
    let sent: Vec<&str> = reqs.iter().map(|r| r["method"].as_str().unwrap()).collect();
    assert_eq!(
        sent,
        ["agent.list"],
        "nothing but the status read: {sent:?}"
    );
}

#[test]
fn renew_clears_through_send_text_then_enter_waits_and_cites_the_marker() {
    let s = stub(
        "renew",
        Box::new(|method, _| {
            Some(Ok(match method {
                "ping" => pong(22),
                "agent.list" => {
                    json!({"type": "agent_list", "agents": [agent("coder", "w1:p2", "idle")]})
                }
                "agent.wait" => json!({"type": "agent_info", "agent": {"agent_status": "idle"}}),
                "pane.read" => json!({"type": "pane_read", "read": {"text": "banner\n❯ \n\n"}}),
                _ => json!({"type": "ok"}),
            }))
        }),
    );
    let (o, _) = manager(&s.sock, &s.dir, &["renew", "--names", "coder"], &[]);
    assert!(o.status.success(), "{}", text(&o.stderr));
    let reqs = s.reqs.lock().unwrap().clone();
    show(&["renew", "--names", "coder"], &text(&o.stdout), &reqs);
    let sent: Vec<&str> = reqs.iter().map(|r| r["method"].as_str().unwrap()).collect();
    assert_eq!(
        sent,
        [
            "agent.list",
            "pane.send_text",
            "pane.send_keys",
            "agent.wait",
            "pane.read"
        ]
    );
    assert_eq!(
        reqs[1]["params"],
        json!({"pane_id": "w1:p2", "text": "/clear"})
    );
    assert_eq!(
        reqs[2]["params"],
        json!({"pane_id": "w1:p2", "keys": ["enter"]})
    );
    assert_eq!(reqs[3]["params"]["until"], json!(["idle", "done"]));
    assert!(
        text(&o.stdout).contains("idle, prompt marker \"❯\" on screen"),
        "{}",
        text(&o.stdout)
    );
}

#[test]
fn renew_says_still_launching_when_the_seat_never_reaches_a_prompt() {
    let s = stub(
        "launching",
        Box::new(|method, _| {
            let mut a = agent("coder", "w1:p2", "idle");
            if method == "agent.wait" {
                return Some(Err(("timeout", "timed out waiting for agent status")));
            }
            Some(Ok(match method {
                "ping" => pong(22),
                "agent.list" => {
                    a["launch_pending"] = json!(true);
                    json!({"type": "agent_list", "agents": [a]})
                }
                "pane.read" => json!({"type": "pane_read", "read": {"text": "Pick a theme\n"}}),
                _ => json!({"type": "ok"}),
            }))
        }),
    );
    let (o, _) = manager(&s.sock, &s.dir, &["renew", "--names", "coder"], &[]);
    assert_eq!(o.status.code(), Some(1));
    let err = text(&o.stderr);
    println!("{err}");
    assert!(
        err.contains("still launching") && err.contains("Pick a theme") && one_line(&err),
        "{err}"
    );
}

#[test]
fn renew_refuses_an_idle_pane_that_shows_no_prompt_marker() {
    let s = stub(
        "nomarker",
        Box::new(|method, _| {
            Some(Ok(match method {
                "ping" => pong(22),
                "agent.list" => {
                    json!({"type": "agent_list", "agents": [agent("coder", "w1:p2", "idle")]})
                }
                "agent.wait" => json!({"type": "agent_info", "agent": {"agent_status": "idle"}}),
                "pane.read" => {
                    json!({"type": "pane_read", "read": {"text": "banner\nauto mode on\n"}})
                }
                _ => json!({"type": "ok"}),
            }))
        }),
    );
    let (o, _) = manager(&s.sock, &s.dir, &["renew", "--names", "coder"], &[]);
    let err = text(&o.stderr);
    println!("{err}");
    assert_eq!(o.status.code(), Some(1));
    assert!(
        err.contains("no prompt marker") && err.contains("auto mode on") && one_line(&err),
        "{err}"
    );
    assert!(text(&o.stdout).is_empty(), "{}", text(&o.stdout));
}

/// A stub herdr with coder (w1:p2) and reviewer (w1:p3) that answers
/// `events.subscribe` with `plan`: `(ms since the ack, pane, status)` events, then
/// hangs up when `close`, else holds the stream until the client lets go (`released`).
fn events_stub(
    name: &str,
    plan: Vec<(u64, &'static str, &'static str)>,
    close: bool,
) -> (Stub, Arc<AtomicBool>) {
    let released = Arc::new(AtomicBool::new(false));
    let flag = released.clone();
    let stream: Stream = Arc::new(move |conn| {
        let t0 = Instant::now();
        for (ms, pane, status) in &plan {
            thread::sleep(Duration::from_millis(*ms).saturating_sub(t0.elapsed()));
            let ev = json!({"event": "pane.agent_status_changed", "data": {
                "type": "pane_agent_status_changed", "pane_id": pane,
                "workspace_id": "w1", "agent_status": status}});
            let _ = writeln!(conn, "{ev}");
        }
        if close {
            return;
        }
        let _ = conn.set_read_timeout(Some(Duration::from_secs(10)));
        let mut buf = [0u8; 64];
        while std::io::Read::read(conn, &mut buf).unwrap_or(0) > 0 {}
        flag.store(true, Ordering::SeqCst);
    });
    let script: Script = Box::new(|method, _| {
        Some(Ok(match method {
            "ping" => pong(22),
            "agent.list" => json!({"type": "agent_list", "agents": [
                agent("coder", "w1:p2", "idle"), agent("reviewer", "w1:p3", "idle")]}),
            "events.subscribe" => json!({"type": "subscription_started"}),
            _ => return Some(Err(("method_not_found", "no such method"))),
        }))
    });
    (stub_streaming(name, script, Some(stream)), released)
}

fn wait_released(released: &AtomicBool) {
    let end = Instant::now() + Duration::from_secs(2);
    while !released.load(Ordering::SeqCst) && Instant::now() < end {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        released.load(Ordering::SeqCst),
        "the client never closed the stream"
    );
}

#[test]
fn events_wait_wakes_on_the_named_agents_transition_and_says_so() {
    let (s, released) = events_stub("ev-match", vec![(300, "w1:p2", "working")], false);
    let (o, took) = manager(
        &s.sock,
        &s.dir,
        &[
            "events",
            "wait",
            "--until",
            "working",
            "--agent",
            "coder",
            "--timeout",
            "5",
        ],
        &[],
    );
    let out = text(&o.stdout);
    println!("{out}");
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    assert!(
        out.starts_with("matched: agent coder -> working after 0.")
            && out.trim_end().lines().count() == 1,
        "{out}"
    );
    assert!(took < Duration::from_secs(3), "took {took:?}");
    let subs = s.reqs.lock().unwrap().clone();
    let sub = subs
        .iter()
        .find(|r| r["method"] == "events.subscribe")
        .unwrap();
    assert_eq!(
        sub["params"],
        json!({"subscriptions": [{"type": "pane.agent_status_changed", "pane_id": "w1:p2"}]})
    );
    wait_released(&released);
}

#[test]
fn events_wait_json_is_one_object() {
    let (s, _) = events_stub("ev-json", vec![(100, "w1:p3", "idle")], false);
    let (o, _) = manager(
        &s.sock,
        &s.dir,
        &[
            "events",
            "wait",
            "--until",
            "idle,done",
            "--timeout",
            "5",
            "--json",
        ],
        &[],
    );
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(
        (
            v["matched"].clone(),
            v["agent"].clone(),
            v["status"].clone()
        ),
        (json!(true), json!("reviewer"), json!("idle"))
    );
    assert!(v["waited_ms"].as_u64().unwrap() >= 90, "{v}");
    // No --agent: every live agent is watched, one subscription each.
    let subs = s.reqs.lock().unwrap().clone();
    let n = subs
        .iter()
        .find(|r| r["method"] == "events.subscribe")
        .unwrap()["params"]["subscriptions"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(n, 2);
}

#[test]
fn events_wait_times_out_exit_3_in_wait_shape_and_lets_go_of_the_socket() {
    let (s, released) = events_stub("ev-timeout", vec![], false);
    let (o, took) = manager(
        &s.sock,
        &s.dir,
        &[
            "events",
            "wait",
            "--until",
            "working",
            "--agent",
            "coder",
            "--timeout",
            "1",
        ],
        &[],
    );
    let out = text(&o.stdout);
    println!("{out}");
    assert_eq!(o.status.code(), Some(3), "{}", text(&o.stderr));
    assert_eq!(
        out.trim_end(),
        "timeout: no agent transition after 1s (watching coder)"
    );
    assert!(
        took >= Duration::from_secs(1) && took < Duration::from_secs(3),
        "took {took:?}"
    );
    assert!(text(&o.stderr).is_empty());
    wait_released(&released);
}

#[test]
fn events_wait_ignores_other_agents_and_unwanted_statuses() {
    // reviewer works (wrong agent), coder goes idle (wrong status): neither wakes it.
    let plan = vec![(100, "w1:p3", "working"), (200, "w1:p2", "idle")];
    let (s, _) = events_stub("ev-ignore", plan.clone(), false);
    let (o, _) = manager(
        &s.sock,
        &s.dir,
        &[
            "events",
            "wait",
            "--until",
            "working",
            "--agent",
            "coder",
            "--timeout",
            "1",
        ],
        &[],
    );
    assert_eq!(o.status.code(), Some(3), "{}", text(&o.stdout));
    // The same stream does wake it once coder reaches the wanted status.
    let mut plan = plan;
    plan.push((400, "w1:p2", "working"));
    let (s, _) = events_stub("ev-ignore2", plan, false);
    let (o, _) = manager(
        &s.sock,
        &s.dir,
        &[
            "events",
            "wait",
            "--until",
            "working",
            "--agent",
            "coder",
            "--timeout",
            "5",
        ],
        &[],
    );
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    assert!(
        text(&o.stdout).starts_with("matched: agent coder -> working after 0.4"),
        "{}",
        text(&o.stdout)
    );
}

#[test]
fn events_wait_on_a_server_that_hangs_up_is_one_error_line_not_a_hang() {
    let (s, _) = events_stub("ev-close", vec![], true);
    let (o, took) = manager(
        &s.sock,
        &s.dir,
        &[
            "events",
            "wait",
            "--until",
            "working",
            "--agent",
            "coder",
            "--timeout",
            "30",
        ],
        &[],
    );
    let err = text(&o.stderr);
    assert_eq!(o.status.code(), Some(1), "{err}");
    assert_eq!(
        err.trim_end(),
        "agentboard: herdr: events.subscribe: connection closed mid-wait"
    );
    assert!(text(&o.stdout).is_empty());
    assert!(took < Duration::from_secs(5), "took {took:?}");
}

#[test]
fn events_wait_refuses_a_typo_and_an_unknown_agent_before_blocking() {
    let (s, _) = events_stub("ev-refuse", vec![], false);
    let (o, took) = manager(&s.sock, &s.dir, &["events", "wait", "--until", "idel"], &[]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        text(&o.stderr).contains("bad agent status \"idel\""),
        "{}",
        text(&o.stderr)
    );
    let (o, _) = manager(
        &s.sock,
        &s.dir,
        &["events", "wait", "--until", "idle", "--agent", "ghost"],
        &[],
    );
    assert_eq!(o.status.code(), Some(1));
    assert!(
        text(&o.stderr).contains("no live agent \"ghost\""),
        "{}",
        text(&o.stderr)
    );
    assert!(took < Duration::from_secs(2));
    assert!(
        !s.reqs
            .lock()
            .unwrap()
            .iter()
            .any(|r| r["method"] == "events.subscribe")
    );
}

fn layout_stub(name: &str, status: &'static str, reject: bool) -> Stub {
    stub(
        name,
        Box::new(move |method, _| {
            Some(match method {
                "ping" => Ok(pong(22)),
                "layout.export" => {
                    Ok(json!({"type":"layout_export", "layout":{"root":layout_tree()}}))
                }
                "agent.list" => Ok(json!({"agents":[agent("coder", "w1:p2", status)]})),
                "tab.list" => Ok(
                    json!({"tabs":[{"tab_id":"w1:t1","workspace_id":"w1","number":1,"label":"test","focused":true,"pane_count":2,"agent_status":status}]}),
                ),
                "layout.apply" if reject => Err(("invalid_layout", "cannot restore\nthis layout")),
                _ => Ok(json!({"type":"ok"})),
            })
        }),
    )
}
fn layout_tree() -> Value {
    json!({"type":"split","direction":"right","ratio":0.37251,
        "first":{"type":"pane","pane_id":"w1:p2"},"second":{"type":"pane","pane_id":"w1:p3"}})
}
#[test]
fn layout_export_round_trip_and_apply_exact_bytes() {
    let s = layout_stub("layout-roundtrip", "idle", false);
    let (o, _) = manager(
        &s.sock,
        &s.dir,
        &["layout", "export", "--tab", "w1:t1", "--json"],
        &[],
    );
    assert!(o.status.success(), "{}", text(&o.stderr));
    let exported = text(&o.stdout);
    assert_eq!(exported.trim_end().lines().count(), 1, "{exported}");
    assert_eq!(
        serde_json::from_str::<Value>(&exported).unwrap(),
        layout_tree()
    );
    let (o, _) = manager(
        &s.sock,
        &s.dir,
        &["layout", "export", "--tab", "w1:t1"],
        &[],
    );
    assert!(o.status.success(), "{}", text(&o.stderr));
    let pretty = text(&o.stdout);
    assert!(pretty.lines().count() > 1, "{pretty}");
    assert_eq!(
        serde_json::from_str::<Value>(&pretty).unwrap(),
        layout_tree()
    );
    let file = s.dir.join("root.json");
    std::fs::write(&file, &exported).unwrap();
    let (o, _) = manager(
        &s.sock,
        &s.dir,
        &[
            "layout",
            "apply",
            "--tab",
            "w1:t1",
            "--root",
            file.to_str().unwrap(),
        ],
        &[],
    );
    assert!(o.status.success(), "{}", text(&o.stderr));
    let mut child = Command::new(env!("CARGO_BIN_EXE_agentboard"))
        .args([
            "manager", "layout", "apply", "--tab", "w1:t1", "--root", "-",
        ])
        .env("AGENTBOARD_HERDR_SOCK", &s.sock)
        .env("AGENTBOARD_DB", s.dir.join("stdin-board"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(exported.as_bytes())
        .unwrap();
    let stdin_out = child.wait_with_output().unwrap();
    assert!(stdin_out.status.success(), "{}", text(&stdin_out.stderr));
    let raw = s.raw.lock().unwrap();
    let sent = raw
        .iter()
        .rev()
        .find(|l| l.contains("layout.apply"))
        .unwrap();
    assert!(
        sent.contains(&format!("\"root\":{}", exported.trim_end())),
        "{sent}"
    );
    println!("exported: {exported}apply root bytes equal exported root bytes: true");
    let reqs = s.reqs.lock().unwrap();
    assert_eq!(reqs[0]["params"], json!({"tab_id":"w1:t1"}));
    assert_eq!(
        reqs.last().unwrap()["params"],
        json!({"tab_id":"w1:t1","root":layout_tree()})
    );
    // Every method uses its own connection preceded by ping.
    let log = s.log.lock().unwrap();
    for (i, m) in log.iter().enumerate() {
        if m != "ping" {
            assert_eq!(log[i - 1], "ping");
        }
    }
}
#[test]
fn layout_apply_empty_malformed_missing_root_and_missing_tab_send_nothing() {
    let s = layout_stub("layout-invalid", "idle", false);
    let missing = s.dir.join("missing-root.json");
    let (o, _) = manager(
        &s.sock,
        &s.dir,
        &[
            "layout",
            "apply",
            "--tab",
            "w1:t1",
            "--root",
            missing.to_str().unwrap(),
        ],
        &[],
    );
    let err = text(&o.stderr);
    println!("missing root: {}", err.trim_end());
    assert_eq!(o.status.code(), Some(1));
    assert!(
        one_line(&err) && err.contains(missing.to_str().unwrap()),
        "{err}"
    );
    assert!(
        err.contains("read root file") && !err.contains("invalid JSON"),
        "{err}"
    );
    for root in [
        "",
        "{",
        "null",
        "{}",
        "[]",
        "{\"type\":\"split\"}",
        "{\"type\":\"pane\",\"env\":[]}",
    ] {
        let (o, _) = manager(
            &s.sock,
            &s.dir,
            &["layout", "apply", "--tab", "w1:t1", "--root", root],
            &[],
        );
        assert_eq!(o.status.code(), Some(1));
        assert!(one_line(&text(&o.stderr)), "{}", text(&o.stderr));
    }
    for args in [
        vec!["layout", "apply", "--tab", "w1:t1"],
        vec!["layout", "apply", "--root", "{\"type\":\"pane\"}"],
    ] {
        let (o, _) = manager(&s.sock, &s.dir, &args, &[]);
        assert_eq!(o.status.code(), Some(1));
        assert!(one_line(&text(&o.stderr)));
    }
    assert!(s.reqs.lock().unwrap().is_empty());
}

#[test]
fn layout_apply_trims_tab_and_accepts_null_pane_options() {
    let s = layout_stub("layout-null-options", "idle", false);
    let root = r#"{"type":"pane","pane_id":null,"cwd":null,"label":null,"env":null}"#;
    let (o, _) = manager(
        &s.sock,
        &s.dir,
        &["layout", "apply", "--tab", " \tw1:t1 \n", "--root", root],
        &[],
    );
    assert!(o.status.success(), "{}", text(&o.stderr));
    assert_eq!(text(&o.stdout).trim_end(), "applied layout to w1:t1");
    let reqs = s.reqs.lock().unwrap();
    let sent = reqs.iter().find(|r| r["method"] == "layout.apply").unwrap();
    assert_eq!(sent["params"]["tab_id"], "w1:t1");
    assert_eq!(
        sent["params"]["root"],
        serde_json::from_str::<Value>(root).unwrap()
    );
    println!("padded tab: sent w1:t1; null pane options: accepted");
}
#[test]
fn layout_apply_busy_guard_unknown_tab_and_herdr_error() {
    let root = layout_tree().to_string();
    for (name, status, tab, reject, code, message) in [
        (
            "layout-busy",
            "working",
            "w1:t1",
            false,
            3,
            "status=working",
        ),
        (
            "layout-blocked",
            "blocked",
            "w1:t1",
            false,
            3,
            "status=blocked",
        ),
        (
            "layout-padded-busy",
            "working",
            " \tw1:t1 \n",
            false,
            3,
            "status=working",
        ),
        ("layout-unknown", "idle", "w1:t404", false, 1, "unknown tab"),
        (
            "layout-error",
            "idle",
            "w1:t1",
            true,
            1,
            "herdr: layout.apply: invalid_layout: cannot restore this layout",
        ),
    ] {
        let s = layout_stub(name, status, reject);
        let (o, _) = manager(
            &s.sock,
            &s.dir,
            &["layout", "apply", "--tab", tab, "--root", &root],
            &[],
        );
        assert_eq!(o.status.code(), Some(code));
        let err = text(&o.stderr);
        assert!(one_line(&err) && err.contains(message), "{err}");
        assert_eq!(
            s.reqs
                .lock()
                .unwrap()
                .iter()
                .any(|r| r["method"] == "layout.apply"),
            reject
        );
    }
}

#[test]
fn layout_apply_force_reports_guard_override_and_preserves_bytes() {
    let s = layout_stub("layout-force", "working", false);
    let (o, _) = manager(
        &s.sock,
        &s.dir,
        &[
            "layout",
            "apply",
            "--tab",
            "w1:t1",
            "--root",
            r#"{ "type" : "pane", "pane_id" : "w1:p2" }"#,
            "--force",
        ],
        &[],
    );
    assert!(o.status.success(), "{}", text(&o.stderr));
    assert!(text(&o.stdout).contains("forced: coder was working"));
    assert!(
        s.raw
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.contains(r#""root":{ "type" : "pane", "pane_id" : "w1:p2" }"#))
    );
}

/// The pre-E11 client can claim and submit, but holds no diff verbs. Exercise
/// the MCP adapter and apply rule together, on the harness's throwaway board.
#[test]
fn pre_gate_mcp_client_cannot_submit_without_a_diff() {
    let output = Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/schema-version.py"
        ))
        .arg("--review-only")
        .arg(env!("CARGO_BIN_EXE_agentboard"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("no-diff review refused by name"));
}
