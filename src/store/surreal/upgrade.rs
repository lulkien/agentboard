//! Offline additive upgrades to the current persisted schema version. The startup lock excludes old and new auto-starters;
//! the private server is never advertised at the board's configured endpoint.
use super::*;
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::process::Child;
use std::time::Instant;

struct OfflineServer(Child);
impl Drop for OfflineServer {
    fn drop(&mut self) {
        unsafe extern "C" {
            fn kill(pid: i32, sig: i32) -> i32;
        }
        // SAFETY: spawn_offline makes the child's pid its process group id.
        unsafe {
            kill(-(self.0.id() as i32), 9);
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn owner_uid() -> u32 {
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    // SAFETY: geteuid has no arguments or side effects.
    unsafe { geteuid() }
}

fn scope(uid: u32) -> String {
    format!(
        "checking positive holder evidence among uid {uid} processes; unreadable processes warn only; board.lock excludes managed v2 servers; legacy/manual servers rely on --owner confirming every seat and server is stopped"
    )
}

fn process_uid(process: &Path) -> std::io::Result<u32> {
    // /proc/<pid> ownership changes for non-dumpable processes. status keeps
    // the real/effective/saved/fs UIDs; compare effective UID with geteuid().
    let status = std::fs::read_to_string(process.join("status"))?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|uid| uid.parse().ok())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "missing or invalid effective Uid in process status",
            )
        })
}

// IO seams let tests simulate a different UID with unreadable descriptors,
// without root privileges or depending on which processes the host runs.
fn inspect_process(
    root: &Path,
    process: &Path,
    pid: u32,
    uid: u32,
    read_uid: impl FnOnce(&Path) -> std::io::Result<u32>,
    read_fds: impl FnOnce(&Path) -> std::io::Result<std::fs::ReadDir>,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    let mut warning = |step: &str, error: &std::io::Error| {
        let name =
            std::fs::read_to_string(process.join("comm")).unwrap_or_else(|_| "unknown".into());
        let message = format!(
            "warning: cannot inspect pid {pid} ({}): {step}: {error}; treating as not a holder; {}",
            name.trim(),
            scope(uid)
        );
        if !warnings
            .iter()
            .any(|existing| existing.starts_with(&format!("warning: cannot inspect pid {pid} (")))
        {
            warnings.push(message);
        }
    };
    let process_uid = match read_uid(process) {
        Ok(uid) => uid,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            warning("read process uid", &error);
            return Ok(());
        }
    };
    if process_uid != uid {
        // This scope relies on owner-private board storage: other unprivileged
        // UIDs cannot access that directory. Root can bypass those permissions;
        // the owner's explicit stopped-all-servers confirmation covers root.
        return Ok(());
    }
    let fds = match read_fds(&process.join("fd")) {
        Ok(fds) => fds,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            warning("open fd directory", &error);
            return Ok(());
        }
    };
    for fd in fds {
        let fd = match fd {
            Ok(fd) => fd,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                warning("list fd entry", &error);
                continue;
            }
        };
        let path = match std::fs::read_link(fd.path()) {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                warning("read fd link", &error);
                continue;
            }
        };
        if path.starts_with(root) {
            return Err(format!(
                "board upgrade refused: board held by pid {pid} ({}); {}; stop every seat and server first",
                root.display(),
                scope(uid)
            ));
        }
    }
    Ok(())
}

/// Check open descriptors, not a pid file: this includes a server bound to a
/// different port and paths accessed through symlinks. Old servers predate our
/// lifetime lock. For them this scan proves only positive evidence; --owner is
/// the transition guard for uninspectable legacy/manual servers.
fn refuse_holders(dir: &Path, warnings: &mut Vec<String>) -> Result<(), String> {
    let uid = owner_uid();
    let root = std::fs::canonicalize(dir).map_err(|e| e.to_string())?;
    let procs = match std::fs::read_dir("/proc") {
        Ok(procs) => procs,
        Err(error) => {
            warnings.push(format!("warning: cannot open /proc (pid unavailable): {error}; treating as not a holder; {}", scope(uid)));
            return Ok(());
        }
    };
    for entry in procs {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                warnings.push(format!("warning: cannot list /proc process entry (pid unavailable): {error}; treating as not a holder; {}", scope(uid)));
                continue;
            }
        };
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|n| n.parse::<u32>().ok()) else {
            continue;
        };
        if pid == std::process::id() {
            continue;
        }
        inspect_process(
            &root,
            &entry.path(),
            pid,
            uid,
            process_uid,
            |path| std::fs::read_dir(path),
            warnings,
        )?;
    }
    Ok(())
}

pub fn run() -> Result<String, String> {
    let mut warnings = Vec::new();
    let result = run_with_warnings(&mut warnings);
    let notices = if warnings.is_empty() {
        String::new()
    } else {
        format!("{}\n", warnings.join("\n"))
    };
    result
        .map(|body| format!("{notices}{body}"))
        .map_err(|error| format!("{notices}{error}"))
}

fn run_with_warnings(warnings: &mut Vec<String>) -> Result<String, String> {
    let s = Settings::from_env()?;
    let timing = serve::Timing::default();
    if !s.dir.is_dir() {
        return Err(format!(
            "board upgrade requires an existing board directory: {}",
            s.dir.display()
        ));
    }
    // Both generations' auto-start paths use this lock, even before opening DB.
    let parent = s.dir.parent().unwrap_or(Path::new("."));
    let _starting = lock::acquire_named(parent, serve::START_LOCK, Duration::ZERO)?;
    let _writing = lock::acquire(&s.dir, Duration::ZERO)?;
    for address in s.url.to_socket_addrs().map_err(|e| e.to_string())? {
        if TcpStream::connect_timeout(&address, timing.probe).is_ok() {
            return Err(format!(
                "board upgrade refused: server running at {}; {}; stop every seat and server first",
                s.url,
                scope(owner_uid())
            ));
        }
    }
    // V2 managed servers hold this flock for their entire lifetime. V1/raw
    // servers do not: their one-time cutover still needs the port check and
    // --owner confirmation; SurrealKV itself permits concurrent processes.
    let lifetime = lock::StoreLock::acquire(&s.dir)?;
    refuse_holders(&s.dir, warnings)?;
    let socket = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let mut private = s.clone();
    private.url = socket.local_addr().map_err(|e| e.to_string())?.to_string();
    drop(socket);
    let bin = serve::find_binary(
        std::env::var("PATH").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
    .ok_or_else(|| "board upgrade: surreal 2.7.x binary missing".to_owned())?;
    let mut server = OfflineServer(spawn_offline(&bin, &private, &lifetime)?);
    let start = Instant::now();
    loop {
        if let Some(status) = server.0.try_wait().map_err(|e| e.to_string())? {
            return Err(format!(
                "board upgrade refused: exclusive offline server exited {status}; check {} for a board holder or storage error",
                serve::log_path(&private).display()
            ));
        }
        if serve::healthy(&private.url, timing.probe) {
            break;
        }
        if start.elapsed() >= timing.deadline {
            return Err("board upgrade: offline server start timed out".into());
        }
        std::thread::sleep(timing.poll_every);
    }
    let (_, version) = serve::http_get(&private.url, "/version", timing.probe)?;
    check_version(&private.url, &version)?;
    let db = bounded(&private, &timing, open_db(&private))?;
    block_on(apply_with_budget(apply(&db), Duration::from_secs(300)))
}

// The temporary server must also die if the upgrading process is SIGKILLed.
// Unlike normal auto-serve it is owned maintenance infrastructure, never detached.
#[cfg(target_os = "linux")]
fn spawn_offline(bin: &Path, s: &Settings, lifetime: &lock::StoreLock) -> Result<Child, String> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    unsafe extern "C" {
        fn prctl(option: i32, ...) -> i32;
        fn getppid() -> i32;
    }
    let parent = std::process::id() as i32;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(serve::log_path(s))
        .map_err(|e| e.to_string())?;
    let err = log.try_clone().map_err(|e| e.to_string())?;
    let mut command = Command::new(bin);
    command
        .args(serve::server_args(s))
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err))
        .process_group(0);
    // SAFETY: only async-signal-safe Linux syscalls run between fork and exec.
    // Check the parent after registering the signal to close the death race.
    unsafe {
        command.pre_exec(move || {
            if prctl(1, 9i32, 0i32, 0i32, 0i32) != 0 || getppid() != parent {
                return Err(std::io::Error::other("upgrade owner exited"));
            }
            Ok(())
        });
    }
    lifetime.spawn(&mut command)
}

#[cfg(not(target_os = "linux"))]
fn spawn_offline(_: &Path, _: &Settings, _: &lock::StoreLock) -> Result<Child, String> {
    Err("board upgrade requires Linux /proc and parent-death signaling".into())
}

// Applying/backfilling the board is a separate phase from connecting. A large
// board gets five minutes, and an expiry names the operation that timed out.
async fn apply_with_budget<T>(
    work: impl std::future::Future<Output = Result<T, String>>,
    budget: Duration,
) -> Result<T, String> {
    tokio::time::timeout(budget, work).await.map_err(|_| {
        format!("board upgrade: apply timed out after {} ms; transaction not confirmed; keep seats stopped and retry board upgrade --owner", budget.as_millis())
    })?
}

async fn counts(db: &Db) -> Result<Vec<(String, usize)>, String> {
    let mut counts = Vec::new();
    for table in ["tasks", "epics", "events", "diffs", "sessions", "counters"] {
        let mut r = db
            .query(format!("SELECT count() AS n FROM {table} GROUP ALL"))
            .await
            .map_err(|e| db_err(&e))?;
        let rows: Vec<serde_json::Value> = r.take(0).map_err(|e| db_err(&e))?;
        counts.push((
            table.to_owned(),
            rows.first().and_then(|v| v["n"].as_u64()).unwrap_or(0) as usize,
        ));
    }
    Ok(counts)
}

async fn apply(db: &Db) -> Result<String, String> {
    let found = marker_version(db).await?;
    if !found.is_some_and(|v| (1..=SCHEMA_VERSION).contains(&v)) {
        return Err(format!(
            "board upgrade supports versions 1..={SCHEMA_VERSION} (or an already-current board); found {found:?}; no changes made"
        ));
    }
    let before = counts(db).await?;
    bootstrap(db, &Settings::from_env()?).await?;
    let after = counts(db).await?;
    let mut out = format!(
        "board upgrade: version {} -> {SCHEMA_VERSION}\n",
        found.unwrap()
    );
    out.push_str("additive definitions checked (IF NOT EXISTS):\n");
    out.push_str(SCHEMA.trim());
    out.push_str("\nbackfill missing defaults only:\n");
    out.push_str(&backfill());
    for ((table, n), (_, m)) in before.iter().zip(&after) {
        out.push_str(&format!("rows {table}: {n} -> {m}\n"));
    }
    out.push_str(&format!(
        "schema version={SCHEMA_VERSION}\ncommitted atomically"
    ));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn different_uid_unreadable_process_does_not_block() {
        let mut inspected = false;
        inspect_process(
            Path::new("/board"),
            Path::new("/injected/pid"),
            7,
            1000,
            |_| Ok(2000),
            |_| {
                inspected = true;
                Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
            },
            &mut Vec::new(),
        )
        .unwrap();
        assert!(
            !inspected,
            "another uid's unreadable descriptors must not be inspected"
        );
    }

    #[test]
    fn same_uid_unreadable_process_warns_and_continues() {
        let mut warnings = Vec::new();
        inspect_process(
            Path::new("/board"),
            Path::new("/injected/pid"),
            7,
            1000,
            |_| Ok(1000),
            |_| Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
            &mut warnings,
        )
        .unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("pid 7 (unknown): open fd directory: permission denied"));
        assert!(warnings[0].contains("treating as not a holder"));
        assert!(warnings[0].contains("uid 1000 processes"));
    }

    #[tokio::test]
    async fn apply_timeout_names_the_upgrade_and_preserves_database_errors() {
        let error = apply_with_budget(
            std::future::pending::<Result<(), String>>(),
            Duration::from_millis(1),
        )
        .await
        .unwrap_err();
        assert_eq!(
            error,
            "board upgrade: apply timed out after 1 ms; transaction not confirmed; keep seats stopped and retry board upgrade --owner"
        );
        let error = apply_with_budget(
            async { Err::<(), _>("database refused".to_owned()) },
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
        assert_eq!(error, "database refused");
    }
}
