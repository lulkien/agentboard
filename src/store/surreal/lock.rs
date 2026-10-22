//! Cross-process write lock. The SurrealDB server is not atomic under contention
//! (docs/surrealdb-migration.md, Decision 6), so every board write holds an
//! exclusive file lock for the whole store operation. A holder that dies loses the
//! lock with its process; a holder that is alive but stuck makes waiters give up
//! at a deadline with one line naming the lock file and the holder.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Seek, Write};
use std::path::Path;
use std::time::{Duration, Instant};

pub const LOCK_FILE: &str = "board.write.lock";
/// How long a writer waits for the lock before giving up.
pub const DEFAULT_WAIT: Duration = Duration::from_secs(5);
const RETRY_EVERY: Duration = Duration::from_millis(10);

/// Held for the life of the value; dropping it releases the lock.
pub struct WriteLock {
    file: Option<File>,
}

impl Drop for WriteLock {
    fn drop(&mut self) {
        if let Some(f) = &self.file {
            let _ = f.unlock();
        }
    }
}

/// Lifetime exclusion for agentboard-managed v2 daemons and owner upgrades.
/// Unlike WriteLock, drop only closes the descriptor: a spawned daemon inherits
/// the same flock open-file description and retains it after the caller exits.
/// Legacy/manual raw surreal servers do NOT take this lock. The v1 cutover
/// therefore still requires the port/positive-fd checks and --owner confirmation.
pub struct StoreLock {
    file: File,
}

impl StoreLock {
    pub fn acquire(dir: &Path) -> Result<Self, String> {
        let path = dir.join("board.lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| format!("board store lock {}: {e}", path.display()))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                let holder = std::fs::read_to_string(&path).unwrap_or_default();
                return Err(format!(
                    "board store lock busy: {} held by pid {}; stop that managed server before starting another server or upgrading",
                    path.display(),
                    holder.trim()
                ));
            }
            Err(TryLockError::Error(error)) => {
                return Err(format!("board store lock {}: {error}", path.display()));
            }
        }
        let lock = Self { file };
        lock.stamp(std::process::id())?;
        Ok(lock)
    }

    fn stamp(&self, pid: u32) -> Result<(), String> {
        let mut file = &self.file;
        file.set_len(0)
            .and_then(|_| file.rewind())
            .and_then(|_| write!(file, "{pid}"))
            .and_then(|_| file.flush())
            .map_err(|e| format!("board store lock holder stamp: {e}"))
    }

    /// Clear CLOEXEC only in the forked child. Parent close must not explicitly
    /// unlock the shared flock, which would also release the child's lock.
    pub fn spawn(
        &self,
        command: &mut std::process::Command,
    ) -> Result<std::process::Child, String> {
        use std::os::fd::AsRawFd;
        use std::os::unix::process::CommandExt;
        unsafe extern "C" {
            fn fcntl(fd: i32, command: i32, ...) -> i32;
            fn kill(pid: i32, signal: i32) -> i32;
        }
        let fd = self.file.as_raw_fd();
        // SAFETY: fcntl is async-signal-safe; F_SETFD=2 and flags=0 retain the
        // existing descriptor across exec. No allocation or Rust IO in pre_exec.
        unsafe {
            command.pre_exec(move || {
                if fcntl(fd, 2, 0i32) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("board server spawn: {e}"))?;
        if let Err(error) = self.stamp(child.id()) {
            // SAFETY: server callers create a process group/session led by child.
            unsafe {
                kill(-(child.id() as i32), 9);
            }
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        Ok(child)
    }
}

#[cfg(test)]
pub fn lock_path(dir: &Path) -> std::path::PathBuf {
    dir.join(LOCK_FILE)
}

/// Takes the exclusive lock on `<dir>/board.write.lock`, retrying until `wait` elapses.
// std's file locks need Rust 1.89, the `rust-version` in Cargo.toml.
pub fn acquire(dir: &Path, wait: Duration) -> Result<WriteLock, String> {
    if unsafe_disabled() {
        return Ok(WriteLock { file: None });
    }
    acquire_named(dir, LOCK_FILE, wait)
}

/// `AGENTBOARD_UNSAFE_NO_WRITE_LOCK=1` turns the write lock into a no-op. It exists
/// only so `tests/surreal.sh` can show its contention checks fail without the lock
/// (a mutation check), and it is compiled into debug builds only: a release binary
/// has no such switch and ignores the variable, so a stray environment variable can
/// never switch off the one cross-process guard.
#[cfg(debug_assertions)]
fn unsafe_disabled() -> bool {
    std::env::var("AGENTBOARD_UNSAFE_NO_WRITE_LOCK").is_ok_and(|v| !v.is_empty() && v != "0")
}

#[cfg(not(debug_assertions))]
fn unsafe_disabled() -> bool {
    false
}

/// Same lock on `<dir>/<name>`; the server start-up uses its own file.
pub fn acquire_named(dir: &Path, name: &str, wait: Duration) -> Result<WriteLock, String> {
    let path = dir.join(name);
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| format!("board write lock {}: {e}", path.display()))?;
    let start = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => {
                // Best effort: lets a blocked writer name the holder.
                let _ = file.set_len(0);
                let _ = file.rewind();
                let _ = write!(file, "{}", std::process::id());
                let _ = file.flush();
                return Ok(WriteLock { file: Some(file) });
            }
            Err(TryLockError::WouldBlock) => {
                if start.elapsed() >= wait {
                    return Err(busy_line(&path, wait));
                }
                std::thread::sleep(RETRY_EVERY);
            }
            Err(TryLockError::Error(e)) => {
                return Err(format!("board write lock {}: {e}", path.display()));
            }
        }
    }
}

fn busy_line(path: &Path, wait: Duration) -> String {
    let holder = File::open(path).ok().and_then(|mut f| {
        let mut s = String::new();
        f.read_to_string(&mut s).ok()?;
        s.trim().parse::<u32>().ok()
    });
    let who = holder.map_or(String::new(), |p| format!(" held by pid {p}"));
    format!(
        "board write lock busy: {}{who}; gave up after {} ms. If that process is stuck, kill it; the lock frees when it exits.",
        path.display(),
        wait.as_millis()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};

    fn scratch(tag: &str) -> std::path::PathBuf {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let d = std::env::temp_dir().join(format!("ab-lock-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn serialises_two_threads() {
        let dir = scratch("serial");
        let inside = Arc::new(AtomicUsize::new(0));
        let overlap = Arc::new(AtomicUsize::new(0));
        let go = Arc::new(Barrier::new(2));
        let hs: Vec<_> = (0..2)
            .map(|_| {
                let (dir, inside, overlap, go) =
                    (dir.clone(), inside.clone(), overlap.clone(), go.clone());
                std::thread::spawn(move || {
                    go.wait();
                    for _ in 0..20 {
                        let _g = acquire(&dir, Duration::from_secs(5)).unwrap();
                        if inside.fetch_add(1, Ordering::SeqCst) != 0 {
                            overlap.fetch_add(1, Ordering::SeqCst);
                        }
                        std::thread::sleep(Duration::from_millis(2));
                        inside.fetch_sub(1, Ordering::SeqCst);
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        assert_eq!(overlap.load(Ordering::SeqCst), 0, "two holders at once");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_blocked_try_expires_with_the_holder_named() {
        let dir = scratch("busy");
        let held = acquire(&dir, Duration::from_secs(1)).unwrap();
        let t = Instant::now();
        let err = acquire(&dir, Duration::from_millis(200)).err().unwrap();
        assert!(t.elapsed() >= Duration::from_millis(200));
        assert!(t.elapsed() < Duration::from_secs(2), "must not hang");
        let want = format!(
            "board write lock busy: {} held by pid {}; gave up after 200 ms.",
            lock_path(&dir).display(),
            std::process::id()
        );
        assert!(err.starts_with(&want), "{err}");
        println!("lock timeout line: {err}");
        drop(held);
        // released on drop: the next writer gets it at once
        acquire(&dir, Duration::from_millis(200)).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
