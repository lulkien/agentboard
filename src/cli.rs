//! The command line as types. Parsing only: validation of statuses, kinds and
//! agent names stays in `store`, so its aliases and error text exist once.
//! Task ids are strings here and go through `store::parse_id` (`T3` or `3`).

use clap::{Parser, Subcommand};

const EXIT_CODES: &str = "Exit codes: 0 success or wait matched, 3 wait timed out, 1 any error (bad flag, unknown task, ...).\n\
With no command, agentboard serves MCP on stdio.\n\
Free-text values that begin with '-' are taken literally (`note T1 --bogus` stores \"--bogus\"); prefer --flag=value when unsure.";

const WAIT_HELP: &str = concat!("Block until a watched task matches.

Selectors (union, at least one; ids need no flag):
  T1 T2...   those tasks
  --epic E1  every task in that epic
  --open     every task not done/dropped, re-evaluated while waiting (new tasks count)

--status CSV  current status in CSV (", crate::task_statuses!(), "); already true => returns at once
--kind CSV    new event kind in CSV (add|claim|report|note|block|answer|review|done|needs_fix|edit|dispatch|diff; `ask` is an alias of `block`)
Both filters => both must hold; neither => any new event. --after: only events with seq > SEQ.

Exit 0 and first line `matched:`; exit 3 and first line `timeout:`.");

/// `--db` means a different thing in each build, so each build says its own.
#[cfg(not(feature = "surreal"))]
const DB_HELP: &str = "Database path (default $AGENTBOARD_DB, else the state dir)";
#[cfg(feature = "surreal")]
const DB_HELP: &str = "Data directory of the local board server (default $AGENTBOARD_DB, else the state dir's `surreal`)";

#[derive(Parser)]
#[command(name = "agentboard", version, about = "Task board + tmux bridge for a team of CLI agents", after_help = EXIT_CODES)]
pub struct Cli {
    /// Actor name for events (default $AGENTBOARD_ACTOR, else 'agent')
    #[arg(long, global = true, value_name = "NAME")]
    pub actor: Option<String>,
    #[arg(long, global = true, value_name = "PATH", help = DB_HELP)]
    pub db: Option<String>,
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// MCP server on stdio (the default with no command)
    Serve,
    /// Create a task
    Add {
        #[arg(allow_hyphen_values = true)]
        title: String,
        #[arg(long, alias = "spec", allow_hyphen_values = true)]
        description: Option<String>,
        #[arg(long, allow_hyphen_values = true)]
        goal: Option<String>,
        /// Initial status (default ready)
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        assignee: Option<String>,
        #[arg(long, default_value_t = 100, allow_negative_numbers = true)]
        priority: i64,
        /// Epic the work belongs to, e.g. E1 (a subtask inherits its parent's)
        #[arg(long)]
        epic: Option<String>,
        /// Parent task (a subtask link), e.g. T1
        #[arg(long)]
        parent: Option<String>,
        /// Project root the work lives in; ~ expands, a relative path resolves against the cwd
        #[arg(long, allow_hyphen_values = true)]
        path: Option<String>,
        /// Git branch the worker starts on (default HEAD: leave the checkout alone)
        #[arg(long, allow_hyphen_values = true)]
        branch: Option<String>,
    },
    /// Diffs: what a coder submitted for review, append-only; only the newest is reviewed
    Diff {
        #[command(subcommand)]
        action: DiffCmd,
    },
    /// Epics: project-scoped groups of tasks (E1, E2...)
    Epic {
        #[command(subcommand)]
        action: EpicCmd,
    },
    /// Amend a task's or epic's spec; refused while a worker holds the task
    Edit {
        /// T3 or E3
        id: String,
        /// Tasks only: move to an open epic, preserving the parent/child chain
        #[arg(long)]
        epic: Option<String>,
        #[arg(long, allow_hyphen_values = true)]
        title: Option<String>,
        #[arg(long, alias = "spec", allow_hyphen_values = true)]
        description: Option<String>,
        #[arg(long, allow_hyphen_values = true)]
        goal: Option<String>,
        /// Tasks only
        #[arg(long, allow_negative_numbers = true)]
        priority: Option<i64>,
        /// Tasks only; empty clears it
        #[arg(long)]
        assignee: Option<String>,
        /// Project root of a task or an epic, normalised like `add --path`; empty clears it
        #[arg(long, allow_hyphen_values = true)]
        path: Option<String>,
        /// Tasks only: branch the worker starts on (default HEAD); empty means HEAD
        #[arg(long, allow_hyphen_values = true)]
        branch: Option<String>,
    },
    /// Set (or `none` to clear) a task's parent task
    Parent {
        id: String,
        /// A task id, or `none`
        parent: String,
    },
    /// List open tasks
    Ls {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        assignee: Option<String>,
        #[arg(long, default_value_t = 20, allow_negative_numbers = true)]
        limit: usize,
    },
    /// Show a task
    Get {
        id: String,
        /// Description/goal/report/note/events unclipped, verbatim
        #[arg(long)]
        full: bool,
    },
    /// Subtasks of a task (T7) or every task in an epic (E3)
    Children { id: String },
    /// Claim a task, or the next ready one
    Claim {
        id: Option<String>,
        /// Lease minutes (1..720)
        #[arg(long, default_value_t = 30, allow_negative_numbers = true)]
        ttl: i64,
    },
    /// Extend the lease on a task you hold
    Heartbeat {
        id: String,
        #[arg(long, default_value_t = 30, allow_negative_numbers = true)]
        ttl: i64,
    },
    /// Record a progress note
    Note {
        id: String,
        #[arg(allow_hyphen_values = true)]
        text: String,
    },
    /// Write the task report without moving it
    Report {
        id: String,
        #[arg(allow_hyphen_values = true)]
        text: String,
    },
    /// Block on a question
    Ask {
        id: String,
        #[arg(allow_hyphen_values = true)]
        question: String,
    },
    /// Answer a blocked task and resume it
    Answer {
        id: String,
        #[arg(allow_hyphen_values = true)]
        text: String,
    },
    /// Finish a task
    Done {
        id: String,
        #[arg(allow_hyphen_values = true)]
        summary: String,
        #[arg(long)]
        commit: Option<String>,
    },
    /// Move a task to review
    Review {
        id: String,
        #[arg(allow_hyphen_values = true)]
        summary: String,
        #[arg(long)]
        commit: Option<String>,
    },
    /// Set a task status.
    #[command(about = concat!("Set a status (", crate::task_statuses!(), ")"))]
    Status {
        id: String,
        status: String,
        #[arg(allow_hyphen_values = true)]
        note: Option<String>,
    },
    /// Block a task with a reason
    Block {
        id: String,
        #[arg(allow_hyphen_values = true)]
        reason: String,
    },
    /// Block until watched tasks match; exit 0 matched, 3 timeout
    #[command(long_about = WAIT_HELP)]
    Wait {
        /// Task ids to watch
        ids: Vec<String>,
        #[arg(long)]
        epic: Option<String>,
        #[arg(long)]
        open: bool,
        #[arg(long, value_name = "CSV")]
        status: Option<String>,
        #[arg(long, value_name = "CSV")]
        kind: Option<String>,
        #[arg(long, value_name = "SEQ", allow_negative_numbers = true)]
        after: Option<i64>,
        /// Also wake on this herdr agent's transition (name or pane id), repeatable
        #[arg(long)]
        agent: Vec<String>,
        /// With --agent: the agent statuses that wake it (default idle,blocked,done)
        #[arg(long, requires = "agent")]
        until: Vec<String>,
        /// Seconds
        #[arg(long, default_value_t = 60, allow_negative_numbers = true)]
        timeout: u64,
    },
    /// Team status in one screen
    Brief {
        /// Window for the done list, minutes
        #[arg(long, default_value_t = 120, allow_negative_numbers = true)]
        since: i64,
    },
    /// Registered agent sessions (`sessions prune` removes dead rows)
    Sessions {
        #[command(subcommand)]
        action: Option<SessionsCmd>,
    },
    /// Remove one registered session row
    Unregister { name: String },
    /// Map a tmux/herdr session to a task
    Register {
        name: String,
        #[arg(long, default_value = "agent")]
        agent: String,
        #[arg(long)]
        task: Option<String>,
    },
    /// Last lines of a pane
    Tail {
        name: String,
        #[arg(long, default_value_t = 40, allow_negative_numbers = true)]
        lines: usize,
    },
    /// Type a line into a pane
    Send {
        name: String,
        #[arg(allow_hyphen_values = true)]
        text: String,
    },
    /// Tell a registered pane to claim a task
    Dispatch {
        id: String,
        /// Session to send to (default: the one registered for the task)
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        force: bool,
        /// Show the prompt without sending it
        #[arg(long)]
        print: bool,
    },
    /// Where the database lives
    Path,
    /// Owner maintenance (never exposed over MCP)
    Board {
        #[command(subcommand)]
        verb: BoardCmd,
    },
    /// Drive the herdr session: `manager [--herdr|--tmux] <verb>`
    Manager {
        /// herdr backend (the default)
        #[arg(long, global = true, conflicts_with = "tmux")]
        herdr: bool,
        /// Refused: there is no tmux backend yet
        #[arg(long, global = true)]
        tmux: bool,
        #[command(subcommand)]
        verb: ManagerCmd,
    },
}

#[derive(Subcommand)]
pub enum ManagerCmd {
    /// Capture or restore a tab layout
    Layout {
        #[command(subcommand)]
        action: ManagerLayoutCmd,
    },
    /// Board sessions joined with live herdr agents (registered, gone, unregistered)
    Status {
        #[arg(long)]
        json: bool,
    },
    /// herdr's session.snapshot summary, or the raw snapshot with --json
    Snapshot {
        #[arg(long)]
        json: bool,
    },
    /// Live herdr agents
    Agent {
        #[command(subcommand)]
        action: ManagerAgentCmd,
    },
    /// herdr events: block until something happens instead of polling
    Events {
        #[command(subcommand)]
        action: ManagerEventsCmd,
    },
    /// herdr tabs
    Tab {
        #[command(subcommand)]
        action: ManagerTabCmd,
    },
    /// herdr panes
    Pane {
        #[command(subcommand)]
        action: ManagerPaneCmd,
    },
    /// Open a team: tab, two panes, coder and reviewer started and registered by name
    Up {
        /// Repo the tab and both agents are rooted in
        #[arg(long)]
        repo: String,
        #[arg(long, default_value = "claude")]
        coder: String,
        #[arg(long, default_value = "opencode")]
        reviewer: String,
        /// Tab label (default: the repo directory name)
        #[arg(long, allow_hyphen_values = true)]
        label: Option<String>,
        /// Prepended to `coder` and `reviewer` to make the herdr and board names
        #[arg(long, default_value = "")]
        role_name_prefix: String,
        /// K=V set in both panes' environment, repeatable
        #[arg(long, allow_hyphen_values = true)]
        env: Vec<String>,
    },
    /// Close a team's panes and unregister it; refuses while an agent is working or blocked
    Down {
        /// Stop agents that are working or blocked anyway
        #[arg(long)]
        force: bool,
        #[arg(long, value_delimiter = ',', default_value = "coder,reviewer")]
        names: Vec<String>,
    },
    /// Epic boundary: /clear each agent's context and confirm it is back at a prompt
    Renew {
        #[arg(long, value_delimiter = ',', default_value = "coder,reviewer")]
        names: Vec<String>,
    },
}

#[derive(Subcommand)]
pub enum ManagerEventsCmd {
    /// Block until an agent moves to a status; exit 0 matched, 3 timeout
    Wait {
        /// idle, working, blocked, done or unknown; comma list or repeat
        #[arg(long, required = true)]
        until: Vec<String>,
        /// Agent name or pane id, repeatable (default: every live agent)
        #[arg(long)]
        agent: Vec<String>,
        /// Seconds
        #[arg(long, default_value_t = 60, allow_negative_numbers = true)]
        timeout: u64,
        #[arg(long)]
        json: bool,
    },
}

const READ_SOURCES: [&str; 4] = ["visible", "recent", "recent_unwrapped", "detection"];

#[derive(Subcommand)]
pub enum ManagerAgentCmd {
    /// One line per live agent
    List,
    /// One agent, by name or pane id
    Get { target: String },
    /// Screen text of an agent's pane
    Read {
        target: String,
        #[arg(long, default_value = "recent", value_parser = READ_SOURCES)]
        source: String,
        #[arg(long)]
        lines: Option<u32>,
    },
    /// Start an agent in an existing shell pane; argv after `--` reaches herdr verbatim
    Start {
        name: String,
        #[arg(long)]
        kind: String,
        #[arg(long)]
        pane: String,
        #[arg(long)]
        timeout_ms: Option<u64>,
        #[arg(last = true, allow_hyphen_values = true)]
        argv: Vec<String>,
    },
    /// Submit one turn to an agent
    Prompt {
        target: String,
        #[arg(allow_hyphen_values = true)]
        text: String,
    },
    /// Send keystrokes (`down`, `enter`, ...) to an agent
    Keys {
        target: String,
        #[arg(required = true, allow_hyphen_values = true)]
        keys: Vec<String>,
    },
    /// Rename an agent; refuses while it is working or blocked
    Rename {
        target: String,
        name: String,
        #[arg(long)]
        force: bool,
    },
    /// Focus an agent's pane
    Focus { target: String },
    /// Block until an agent reaches a status
    Wait {
        target: String,
        /// idle, working, blocked, done or unknown; repeat for several
        #[arg(long)]
        until: Vec<String>,
        #[arg(long)]
        timeout_ms: Option<u64>,
    },
}

#[derive(Subcommand)]
pub enum ManagerTabCmd {
    /// One line per tab
    List,
    /// Create a tab rooted in a directory
    New {
        #[arg(long)]
        cwd: String,
        #[arg(long, allow_hyphen_values = true)]
        label: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum ManagerPaneCmd {
    /// One line per pane
    List,
    /// One pane
    Get { pane_id: String },
    /// Split a pane
    Split {
        pane_id: String,
        #[arg(long, conflicts_with = "down", required_unless_present = "down")]
        right: bool,
        #[arg(long)]
        down: bool,
        #[arg(long)]
        cwd: Option<String>,
        /// K=V, repeatable
        #[arg(long, allow_hyphen_values = true)]
        env: Vec<String>,
    },
    /// Close a pane; refuses while an agent in it is working or blocked
    Close {
        pane_id: String,
        #[arg(long)]
        force: bool,
    },
    /// Screen text of a pane
    Read {
        pane_id: String,
        #[arg(long, default_value = "recent", value_parser = READ_SOURCES)]
        source: String,
        #[arg(long)]
        lines: Option<u32>,
    },
}

#[derive(Subcommand)]
pub enum SessionsCmd {
    /// Delete rows whose tmux session / herdr agent is gone
    Prune {
        /// List what would be removed; delete nothing
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
pub enum DiffCmd {
    /// Record a diff for a task (round = its reopen count + 1); stored verbatim, never edited
    Add {
        /// Task id, e.g. T7
        id: String,
        /// The diff text
        #[arg(long, allow_hyphen_values = true, conflicts_with = "body_file")]
        body: Option<String>,
        /// Read the diff from a file, or from stdin with `-`
        #[arg(long, allow_hyphen_values = true)]
        body_file: Option<String>,
        /// Record evidence for a re-run round instead of a diff; needs --reviews
        #[arg(long)]
        evidence_only: bool,
        /// The newest patch of this row, e.g. D26 (evidence-only submissions only)
        #[arg(long, allow_hyphen_values = true)]
        reviews: Option<String>,
    },
    /// A task's diffs (or an epic's), newest first, one line each
    Ls {
        /// Task or epic id, e.g. T7 or E2
        id: String,
    },
    /// Show one diff: task, round, age and body
    Get {
        /// Diff id, e.g. D12
        id: String,
        /// The body unclipped, verbatim
        #[arg(long)]
        full: bool,
    },
}

#[derive(Subcommand)]
pub enum EpicCmd {
    /// Create an epic
    Add {
        #[arg(allow_hyphen_values = true)]
        title: String,
        #[arg(long, alias = "spec", allow_hyphen_values = true)]
        description: Option<String>,
        #[arg(long, allow_hyphen_values = true)]
        goal: Option<String>,
        /// Project root; ~ expands, relative resolves against the cwd, need not exist yet
        #[arg(long, allow_hyphen_values = true)]
        path: Option<String>,
    },
    /// List epics (default: open)
    Ls {
        /// open|done|dropped
        #[arg(long)]
        status: Option<String>,
    },
    /// Show an epic
    Get {
        id: String,
        /// Description/goal/events unclipped, verbatim
        #[arg(long)]
        full: bool,
    },
    /// Set an epic open|done|dropped (closing is never automatic)
    Status {
        id: String,
        status: String,
        #[arg(allow_hyphen_values = true)]
        note: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum ManagerLayoutCmd {
    /// Export the tree exactly as herdr reports it
    Export {
        #[arg(long, allow_hyphen_values = true)]
        tab: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Replace an explicitly named tab's layout (JSON, file, or - for stdin)
    Apply {
        #[arg(long, allow_hyphen_values = true)]
        tab: Option<String>,
        #[arg(long, allow_hyphen_values = true)]
        root: Option<String>,
        #[arg(long)]
        force: bool,
    },
}

#[derive(clap::Subcommand, Debug)]
pub enum BoardCmd {
    /// Upgrade a stopped v1 SurrealDB board additively, preserving its rows
    Upgrade {
        /// Confirm you own this board and have stopped every seat and server
        #[arg(long)]
        owner: bool,
    },
}
