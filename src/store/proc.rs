//! One bounded subprocess runner for the pane bridges (herdr, tmux). A wedged
//! client must not hang `dispatch`, `send` or `tail`: it gets a deadline, and on
//! expiry the whole process group is killed, not just the direct child.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const MAX_BYTES: u64 = 1 << 20;

/// Deadline for one herdr/tmux call. `AGENTBOARD_DELIVER_TIMEOUT_MS` overrides
/// the 2s default so a test can prove the kill path in milliseconds.
fn deadline() -> Duration {
    let ms = std::env::var("AGENTBOARD_DELIVER_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000);
    Duration::from_millis(ms)
}

/// Reads a pipe to the end (capped) off-thread, so a chatty child cannot fill it
/// and stall.
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = pipe {
            let _ = p.take(MAX_BYTES).read_to_end(&mut buf);
        }
        let _ = tx.send(buf);
    });
    rx
}

/// Runs `cmd` with null stdin and captured output. `Err` is a one-line reason:
/// spawn failure or `timed out after Nms`. A nonzero exit is `Ok` (see `status`).
///
/// The child leads its own process group and the group is killed at the
/// deadline, so descendants (a shell stub's `sleep`) die with it. A request the
/// remote end already accepted may still complete after the client dies: work
/// already delivered cannot be cancelled from here.
pub fn run(cmd: &mut Command) -> Result<Output, String> {
    let limit = deadline();
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|e| e.to_string())?;
    let (out_rx, err_rx) = (drain(child.stdout.take()), drain(child.stderr.take()));
    let end = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if Instant::now() < end => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                // `kill -KILL -- -PGID`: the group id is the child's pid (process_group(0)).
                let _ = Command::new("kill")
                    .args(["-KILL", "--", &format!("-{}", child.id())])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("timed out after {}ms", limit.as_millis()));
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    // A descendant that outlives the child may hold the pipes: do not wait on it long.
    let grace = Duration::from_millis(200);
    Ok(Output {
        status,
        stdout: out_rx.recv_timeout(grace).unwrap_or_default(),
        stderr: err_rx.recv_timeout(grace).unwrap_or_default(),
    })
}
