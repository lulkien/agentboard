use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use heed::types::Bytes;
use heed::{CommitOrAbort, Database, Env, EnvOpenOptions, MdbError, WithoutTls};
use serde::{Deserialize, Serialize};

use crate::domain;

pub(super) type Db = Database<'static, Bytes, Bytes>;
pub(super) type StoreEnv = Env<WithoutTls>;
pub(super) type ReadTxn<'env> = heed::RoTxn<'env, WithoutTls>;
pub(super) const FORMAT_VERSION: u32 = 1;
const APPLICATION: &str = "agentboard-lmdb";
const ENVELOPE_KEY: &[u8] = b"envelope";
const DEFAULT_MAP_SIZE_MB: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BoardOptions {
    pub map_size_mb: usize,
}

impl Default for BoardOptions {
    fn default() -> Self {
        Self {
            map_size_mb: DEFAULT_MAP_SIZE_MB,
        }
    }
}

impl BoardOptions {
    pub fn from_env() -> Result<Self> {
        match std::env::var("AGENTBOARD_MAP_SIZE_MB") {
            Ok(raw) => {
                let map_size_mb = raw
                    .parse()
                    .map_err(|_| Error::InvalidMapSize(raw.clone()))?;
                let options = Self { map_size_mb };
                options.map_size_bytes()?;
                Ok(options)
            }
            Err(std::env::VarError::NotPresent) => Ok(Self::default()),
            Err(error) => Err(Error::InvalidMapSize(error.to_string())),
        }
    }

    fn map_size_bytes(self) -> Result<usize> {
        if self.map_size_mb == 0 {
            return Err(Error::InvalidMapSize(self.map_size_mb.to_string()));
        }
        self.map_size_mb
            .checked_mul(1024 * 1024)
            .ok_or_else(|| Error::InvalidMapSize(self.map_size_mb.to_string()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Metadata {
    pub application: String,
    pub format_version: u32,
    pub instance_id: String,
    pub next_task_id: u64,
    pub next_event_seq: u64,
    pub map_size_mb: usize,
}

impl Metadata {
    fn fresh(map_size_mb: usize) -> Result<Self> {
        let mut random = [0; 16];
        File::open("/dev/urandom")?.read_exact(&mut random)?;
        let instance_id = random.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(Self {
            application: APPLICATION.into(),
            format_version: FORMAT_VERSION,
            instance_id,
            next_task_id: 1,
            next_event_seq: 1,
            map_size_mb,
        })
    }
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Lmdb(heed::Error),
    Json(serde_json::Error),
    Domain(domain::Error),
    InvalidMapSize(String),
    MapSizeMismatch {
        configured_mb: usize,
        stored_mb: usize,
    },
    EnvironmentAlreadyOpen,
    MapFull,
    ReadersFull,
    FormatMismatch {
        expected: u32,
        found: u32,
    },
    Unstamped,
    ForeignBoard,
    UnknownEntry(String),
    Corrupt(&'static str),
    EmptyComment,
    InvalidCursor(String),
    CursorInstanceMismatch,
    WaitWithoutStatus,
    WaitWithoutTask,
    UnknownAssignee(String),
    MissingTask(u64),
    NotReleasable {
        id: u64,
        status: domain::Status,
    },
    NoHolder(u64),
    NotEpic(u64),
    EpicOpenChildren {
        id: u64,
        count: u64,
    },
    MissingParent(u64),
    ParentTerminal {
        id: u64,
        status: domain::Status,
    },
    ReopenUnderTerminalParent {
        id: u64,
        parent: u64,
        status: domain::Status,
    },
    ParentCycle(u64),
    CounterOverflow,
    ResetBusy,
    UnsafePath(PathBuf),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "board I/O: {error}"),
            Self::Lmdb(error) => write!(f, "LMDB: {error}"),
            Self::Json(error) => write!(f, "invalid board JSON: {error}"),
            Self::Domain(error) => write!(f, "{error}"),
            Self::InvalidMapSize(raw) => write!(f, "invalid AGENTBOARD_MAP_SIZE_MB: {raw}"),
            Self::MapSizeMismatch { configured_mb, stored_mb } => write!(f, "AGENTBOARD_MAP_SIZE_MB mismatch: configured {configured_mb} MiB, board uses {stored_mb} MiB"),
            Self::EnvironmentAlreadyOpen => f.write_str("LMDB environment already open in this process; reuse its Board handle"),
            Self::MapFull => f.write_str("LMDB map full; set AGENTBOARD_MAP_SIZE_MB for a fresh board or reset this disposable board with a larger size"),
            Self::ReadersFull => f.write_str("LMDB reader slots exhausted (limit 256)"),
            Self::FormatMismatch { expected, found } => write!(f, "board format {found} is incompatible with expected {expected}; run board reset --yes after stopping clients"),
            Self::Unstamped => f.write_str("unstamped board; inspect it before board reset --yes"),
            Self::ForeignBoard => f.write_str("directory is not an agentboard LMDB board"),
            Self::UnknownEntry(name) => write!(
                f,
                "unexpected entry {name:?} in the board directory; a board holds only data.mdb, lock.mdb and lifetime.lock, so move it aside"
            ),
            Self::Corrupt(reason) => write!(f, "corrupt board: {reason}"),
            Self::EmptyComment => f.write_str("comment must be nonempty"),
            Self::InvalidCursor(raw) => write!(f, "invalid feed cursor {raw:?}"),
            Self::CursorInstanceMismatch => {
                f.write_str("feed cursor belongs to a different board; reset a cursor or reset the board")
            }
            Self::WaitWithoutStatus => f.write_str("wait needs at least one wanted status"),
            Self::WaitWithoutTask => f.write_str("wait needs at least one task id"),
            Self::UnknownAssignee(name) => write!(f, "unknown assignee {name:?}; use new_actor=true to register the target"),
            Self::MissingTask(id) => write!(f, "task T{id} does not exist"),
            Self::NotReleasable { id, status } => {
                write!(f, "task T{id} is {}; nothing to release", status.as_str())
            }
            Self::NoHolder(id) => write!(f, "task T{id} has no holder to release"),
            Self::NotEpic(id) => write!(f, "task T{id} is not an epic; an epic is a task with children"),
            Self::EpicOpenChildren { id, count } => write!(f, "epic T{id} has {count} open children; close or drop them first"),
            Self::MissingParent(id) => write!(f, "parent T{id} does not exist"),
            Self::ParentTerminal { id, status } => write!(f, "parent T{id} is {}; reopen it before adding children", status.as_str()),
            Self::ReopenUnderTerminalParent { id, parent, status } => write!(f, "parent T{parent} is {}; reopen it before reopening T{id}", status.as_str()),
            Self::ParentCycle(id) => write!(f, "task T{id} parent link would create a cycle"),
            Self::CounterOverflow => f.write_str("board ID counter overflow"),
            Self::ResetBusy => f.write_str("board reset refused: another client is attached"),
            Self::UnsafePath(path) => write!(f, "board reset refused to follow symlink {}", path.display()),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<heed::Error> for Error {
    fn from(error: heed::Error) -> Self {
        match error {
            heed::Error::Mdb(MdbError::MapFull) => Self::MapFull,
            heed::Error::Mdb(MdbError::ReadersFull) => Self::ReadersFull,
            _ => Self::Lmdb(error),
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<domain::Error> for Error {
    fn from(error: domain::Error) -> Self {
        Self::Domain(error)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub(super) fn read_txn(env: &StoreEnv) -> Result<ReadTxn<'_>> {
    match env.read_txn() {
        Ok(txn) => Ok(txn),
        Err(heed::Error::Mdb(MdbError::ReadersFull)) => {
            env.clear_stale_readers()?;
            env.read_txn().map_err(Error::from)
        }
        Err(error) => Err(error.into()),
    }
}

pub struct Board {
    pub(super) env: StoreEnv,
    pub(super) meta: Db,
    pub(super) tasks: Db,
    pub(super) events: Db,
    pub(super) events_by_task: Db,
    pub(super) agents: Db,
    pub(super) tasks_by_status: Db,
    pub(super) tasks_by_assignee: Db,
    pub(super) path: PathBuf,
    _lifetime_lock: File,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResetOutcome {
    pub path: PathBuf,
    pub old_instance_id: Option<String>,
    pub new_instance_id: String,
}

impl Board {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_options(path, BoardOptions::from_env()?)
    }

    pub fn open_with_options(path: impl AsRef<Path>, options: BoardOptions) -> Result<Self> {
        options.map_size_bytes()?;
        let path = prepare_path(path.as_ref())?;
        // A fresh directory must be ours before anything is created in it; an
        // existing board may carry incidental files and is opened by its metadata.
        if !path.join("data.mdb").exists() {
            refuse_unknown_entries(&path)?;
        }
        // An exclusive-to-shared lock downgrade is a release, so a reset can unlink
        // the files in that gap. Retry when the mapped data file is not the one we
        // opened.
        let mut replaced = false;
        for _ in 0..3 {
            let before = data_file_id(&path);
            let board = Self::open_once(&path, options)?;
            let after = data_file_id(&path);
            let stable = match (before, after) {
                (Some(before), Some(after)) => before == after,
                (None, Some(_)) => true,
                (_, None) => false,
            };
            if stable {
                return Ok(board);
            }
            replaced = true;
            drop(board);
        }
        debug_assert!(replaced);
        Err(Error::Corrupt(
            "board files were replaced while opening; retry",
        ))
    }

    fn open_once(path: &Path, options: BoardOptions) -> Result<Self> {
        let lock = open_lifetime_lock(path)?;
        // Ask for the exclusive lock first: it is what a first opener needs, and
        // trying it before any shared lock removes the release-then-retry window.
        match lock.try_lock() {
            Ok(()) => {
                let board = Self::open_locked(path.to_owned(), options, lock)?;
                board._lifetime_lock.lock_shared()?;
                Ok(board)
            }
            Err(fs::TryLockError::WouldBlock) => {
                // Somebody else owns or holds the board; wait as a reader.
                lock.lock_shared()?;
                Self::open_locked(path.to_owned(), options, lock)
            }
            Err(fs::TryLockError::Error(error)) => Err(error.into()),
        }
    }

    fn open_locked(path: PathBuf, options: BoardOptions, lock: File) -> Result<Self> {
        let fresh = !path.join("data.mdb").exists();
        if fresh {
            for entry in fs::read_dir(&path)? {
                let entry = entry?;
                match entry.file_name().to_str() {
                    Some("lifetime.lock") => {}
                    Some("lock.mdb") => return Err(Error::Unstamped),
                    _ => return Err(Error::ForeignBoard),
                }
            }
        }
        let mut builder = EnvOpenOptions::new().read_txn_without_tls();
        builder.max_dbs(7).max_readers(256);
        if fresh {
            builder.map_size(options.map_size_bytes()?);
        }
        // The lifetime lock prevents reset from unlinking files while mapped.
        // No other code path modifies LMDB files directly.
        let env = unsafe { builder.open(&path) }.map_err(|error| match error {
            heed::Error::EnvAlreadyOpened => Error::EnvironmentAlreadyOpen,
            other => Error::from(other),
        })?;
        let (meta, tasks, events, events_by_task, agents, tasks_by_status, tasks_by_assignee) =
            if fresh {
                let wtxn = env.write_txn()?;
                env.create_databases_and_commit(wtxn, |opener, txn| -> Result<_> {
                    let meta = opener.create_database::<Bytes, Bytes>(txn, Some("meta"))?;
                    let envelope = Metadata::fresh(options.map_size_mb)?;
                    meta.put(txn, ENVELOPE_KEY, &serde_json::to_vec(&envelope)?)?;
                    let tasks = opener.create_database::<Bytes, Bytes>(txn, Some("tasks"))?;
                    let events = opener.create_database::<Bytes, Bytes>(txn, Some("events"))?;
                    let events_by_task =
                        opener.create_database::<Bytes, Bytes>(txn, Some("events_by_task"))?;
                    let agents = opener.create_database::<Bytes, Bytes>(txn, Some("agents"))?;
                    let tasks_by_status =
                        opener.create_database::<Bytes, Bytes>(txn, Some("tasks_by_status"))?;
                    let tasks_by_assignee =
                        opener.create_database::<Bytes, Bytes>(txn, Some("tasks_by_assignee"))?;
                    Ok(CommitOrAbort::<_, ()>::Commit((
                        meta,
                        tasks,
                        events,
                        events_by_task,
                        agents,
                        tasks_by_status,
                        tasks_by_assignee,
                    )))
                })?
                .unwrap_commit()
            } else {
                let rtxn = read_txn(&env)?;
                env.open_databases_and_commit(rtxn, |opener, txn| -> Result<_> {
                    let meta = opener
                        .open_database::<Bytes, Bytes>(txn, Some("meta"))?
                        .ok_or(Error::Unstamped)?;
                    let raw = meta.get(txn, ENVELOPE_KEY)?.ok_or(Error::Unstamped)?;
                    let envelope: Metadata = serde_json::from_slice(raw)?;
                    check_metadata(&envelope, options)?;
                    let tasks = opener
                        .open_database::<Bytes, Bytes>(txn, Some("tasks"))?
                        .ok_or(Error::Corrupt("missing tasks database"))?;
                    let events = opener
                        .open_database::<Bytes, Bytes>(txn, Some("events"))?
                        .ok_or(Error::Corrupt("missing events database"))?;
                    let events_by_task = opener
                        .open_database::<Bytes, Bytes>(txn, Some("events_by_task"))?
                        .ok_or(Error::Corrupt("missing events_by_task database"))?;
                    let agents = opener
                        .open_database::<Bytes, Bytes>(txn, Some("agents"))?
                        .ok_or(Error::Corrupt("missing agents database"))?;
                    let tasks_by_status = opener
                        .open_database::<Bytes, Bytes>(txn, Some("tasks_by_status"))?
                        .ok_or(Error::Corrupt("missing tasks_by_status database"))?;
                    let tasks_by_assignee = opener
                        .open_database::<Bytes, Bytes>(txn, Some("tasks_by_assignee"))?
                        .ok_or(Error::Corrupt("missing tasks_by_assignee database"))?;
                    Ok(CommitOrAbort::<_, ()>::Commit((
                        meta,
                        tasks,
                        events,
                        events_by_task,
                        agents,
                        tasks_by_status,
                        tasks_by_assignee,
                    )))
                })?
                .unwrap_commit()
            };
        if env.info().map_size != options.map_size_bytes()? {
            return Err(Error::Corrupt(
                "LMDB map size disagrees with board metadata",
            ));
        }
        Ok(Self {
            env,
            meta,
            tasks,
            events,
            events_by_task,
            agents,
            tasks_by_status,
            tasks_by_assignee,
            path,
            _lifetime_lock: lock,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn metadata(&self) -> Result<Metadata> {
        let txn = read_txn(&self.env)?;
        self.checked_metadata(&txn)
    }

    pub(super) fn checked_metadata(&self, txn: &ReadTxn<'_>) -> Result<Metadata> {
        let metadata = read_metadata(self.meta, txn)?;
        let actual_map_size = self.env.info().map_size;
        let actual_map_mb = actual_map_size / (1024 * 1024);
        if actual_map_mb * 1024 * 1024 != actual_map_size {
            return Err(Error::Corrupt("LMDB map size is not a whole MiB"));
        }
        check_metadata(
            &metadata,
            BoardOptions {
                map_size_mb: actual_map_mb,
            },
        )?;
        Ok(metadata)
    }

    pub fn reset(path: impl AsRef<Path>, options: BoardOptions) -> Result<ResetOutcome> {
        options.map_size_bytes()?;
        let path = prepare_path(path.as_ref())?;
        let lock = open_lifetime_lock(&path)?;
        // The attached-client refusal wins over a directory complaint, and the
        // directory is re-checked under the exclusive lock so a stray file cannot
        // turn the disposal into a half-deleted board.
        // A client that is closing can hold the lock for a moment; give the
        // exclusive request a short, bounded grace before calling it attached.
        let mut waited = 0;
        loop {
            match lock.try_lock() {
                Ok(()) => break,
                Err(fs::TryLockError::WouldBlock) if waited < 20 => {
                    waited += 1;
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(fs::TryLockError::WouldBlock) => return Err(Error::ResetBusy),
                Err(fs::TryLockError::Error(error)) => return Err(Error::Io(error)),
            }
        }
        refuse_unknown_entries(&path)?;
        let old_instance_id = inspect_for_reset(&path)?;
        for name in ["data.mdb", "lock.mdb"] {
            let file = path.join(name);
            refuse_symlink(&file)?;
            match fs::remove_file(file) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        let board = Self::open_locked(path.clone(), options, lock)?;
        let new_instance_id = board.metadata()?.instance_id;
        drop(board);
        Ok(ResetOutcome {
            path,
            old_instance_id,
            new_instance_id,
        })
    }
}

pub(super) fn read_metadata(db: Db, txn: &ReadTxn<'_>) -> Result<Metadata> {
    let raw = db
        .get(txn, ENVELOPE_KEY)?
        .ok_or(Error::Corrupt("missing metadata envelope"))?;
    Ok(serde_json::from_slice(raw)?)
}

pub(super) fn write_metadata(db: Db, txn: &mut heed::RwTxn<'_>, metadata: &Metadata) -> Result<()> {
    db.put(txn, ENVELOPE_KEY, &serde_json::to_vec(metadata)?)?;
    Ok(())
}

fn check_metadata(metadata: &Metadata, options: BoardOptions) -> Result<()> {
    if metadata.application != APPLICATION {
        return Err(Error::ForeignBoard);
    }
    if metadata.format_version != FORMAT_VERSION {
        return Err(Error::FormatMismatch {
            expected: FORMAT_VERSION,
            found: metadata.format_version,
        });
    }
    if metadata.map_size_mb != options.map_size_mb {
        return Err(Error::MapSizeMismatch {
            configured_mb: options.map_size_mb,
            stored_mb: metadata.map_size_mb,
        });
    }
    Ok(())
}

fn prepare_path(path: &Path) -> Result<PathBuf> {
    fs::create_dir_all(path)?;
    let path = fs::canonicalize(path)?;
    for name in ["lifetime.lock", "data.mdb", "lock.mdb"] {
        refuse_symlink(&path.join(name))?;
    }
    Ok(path)
}

#[cfg(unix)]
fn data_file_id(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path.join("data.mdb"))
        .ok()
        .map(|meta| (meta.dev(), meta.ino()))
}

#[cfg(not(unix))]
fn data_file_id(_path: &Path) -> Option<(u64, u64)> {
    None
}

fn refuse_unknown_entries(path: &Path) -> Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name();
        if !matches!(
            name.to_str(),
            Some("lifetime.lock" | "data.mdb" | "lock.mdb")
        ) {
            return Err(Error::UnknownEntry(name.to_string_lossy().into_owned()));
        }
    }
    Ok(())
}

fn refuse_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(Error::UnsafePath(path.to_owned()))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn open_lifetime_lock(path: &Path) -> Result<File> {
    let file = path.join("lifetime.lock");
    refuse_symlink(&file)?;
    Ok(OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(file)?)
}

fn inspect_for_reset(path: &Path) -> Result<Option<String>> {
    if !path.join("data.mdb").exists() {
        refuse_unknown_entries(path)?;
        return Ok(None);
    }
    let mut builder = EnvOpenOptions::new().read_txn_without_tls();
    builder.max_dbs(7).max_readers(256);
    // This inspection environment is dropped before any LMDB file is removed.
    let env = unsafe { builder.open(path) }?;
    let txn = read_txn(&env)?;
    let meta = env
        .open_databases_and_commit(txn, |opener, txn| {
            opener
                .open_database::<Bytes, Bytes>(txn, Some("meta"))
                .map(CommitOrAbort::<_, ()>::Commit)
        })?
        .unwrap_commit()
        .ok_or(Error::Unstamped)?;
    let txn = read_txn(&env)?;
    let raw = meta.get(&txn, ENVELOPE_KEY)?.ok_or(Error::Unstamped)?;
    let metadata: serde_json::Value = serde_json::from_slice(raw)?;
    if metadata
        .get("application")
        .and_then(serde_json::Value::as_str)
        != Some(APPLICATION)
    {
        return Err(Error::ForeignBoard);
    }
    let instance_id = metadata
        .get("instance_id")
        .and_then(serde_json::Value::as_str)
        .ok_or(Error::Corrupt("missing board instance ID"))?;
    Ok(Some(instance_id.to_owned()))
}
