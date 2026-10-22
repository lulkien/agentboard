//! Offline v1 -> v2 only. The startup lock excludes old and new auto-starters;
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

/// Check open descriptors, not a pid file: this includes a server bound to a
/// different port and paths accessed through symlinks. Old servers predate our
/// maintenance lock, so merely taking board.lock would not establish exclusion.
fn refuse_holders(dir: &Path) -> Result<(), String> {
    let root = std::fs::canonicalize(dir).map_err(|e| e.to_string())?;
    let procs = std::fs::read_dir("/proc").map_err(|error| {
        format!("board upgrade refused: cannot open /proc (pid unavailable): {error}; Linux /proc is required to verify the board is offline")
    })?;
    for entry in procs {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "board upgrade refused: cannot list /proc process entry (pid unavailable): {error}; cannot verify board is offline"
                ));
            }
        };
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|n| n.parse::<u32>().ok()) else {
            continue;
        };
        if pid == std::process::id() {
            continue;
        }
        let refused = |step: &str, error: &std::io::Error| {
            format!(
                "board upgrade refused: cannot inspect pid {pid}: {step}: {error}; cannot verify board is offline"
            )
        };
        let fds = match std::fs::read_dir(entry.path().join("fd")) {
            Ok(fds) => fds,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(refused("open fd directory", &error)),
        };
        for fd in fds {
            let fd = match fd {
                Ok(fd) => fd,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(refused("list fd entry", &error)),
            };
            let path = match std::fs::read_link(fd.path()) {
                Ok(path) => path,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(refused("read fd link", &error)),
            };
            if path.starts_with(&root) {
                return Err(format!(
                    "board upgrade refused: board held by pid {pid} ({}); stop every seat and server first",
                    root.display()
                ));
            }
        }
    }
    Ok(())
}

pub fn run() -> Result<String, String> {
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
                "board upgrade refused: server running at {}; stop every seat and server first",
                s.url
            ));
        }
    }
    refuse_holders(&s.dir)?;
    let socket = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let mut private = s.clone();
    private.url = socket.local_addr().map_err(|e| e.to_string())?.to_string();
    drop(socket);
    let bin = serve::find_binary(
        std::env::var("PATH").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
    .ok_or_else(|| "board upgrade: surreal 2.7.x binary missing".to_owned())?;
    let mut server = OfflineServer(spawn_offline(&bin, &private)?);
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
fn spawn_offline(bin: &Path, s: &Settings) -> Result<Child, String> {
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
    command
        .spawn()
        .map_err(|e| format!("board upgrade server: {e}"))
}

#[cfg(not(target_os = "linux"))]
fn spawn_offline(_: &Path, _: &Settings) -> Result<Child, String> {
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
    if !matches!(found, Some(1 | SCHEMA_VERSION)) {
        return Err(format!(
            "board upgrade supports only version 1 -> {SCHEMA_VERSION} (or an already-current board); found {found:?}; no changes made"
        ));
    }
    let before = counts(db).await?;
    // BEGIN covers definitions, backfill and stamps. A crash before COMMIT leaves
    // version 1 and its rows unchanged; IF NOT EXISTS also tolerates earlier DDL.
    let query = format!(
        "BEGIN TRANSACTION;\n{SCHEMA}\n{}\nUPSERT meta:schema SET version=$v, build=$build;\nUPSERT meta:opened SET version=$v, build=$build;\nUPSERT meta:writer SET version=$v, build=$build;\nCOMMIT TRANSACTION;",
        backfill()
    );
    db.query(query)
        .bind(("v", SCHEMA_VERSION))
        .bind(("build", crate::ident::stamp()))
        .await
        .and_then(surrealdb::Response::check)
        .map_err(|e| db_err(&e))?;
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
    out.push_str(&format!("board_stamp (SurrealDB meta:schema, meta:opened, meta:writer): generation={SCHEMA_VERSION} {}\ncommitted atomically", crate::ident::stamp()));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

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
