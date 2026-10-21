//! The command line as types. Parsing only: validation of statuses, kinds and
//! agent names stays in `store`, so its aliases and error text exist once.
//! Task ids are strings here and go through `store::parse_id` (`T3` or `3`).

use clap::{Parser, Subcommand};

const EXIT_CODES: &str = "Exit codes: 0 success or wait matched, 3 wait timed out, 1 any error (bad flag, unknown task, ...).\n\
With no command, agentboard serves MCP on stdio.\n\
Free-text values that begin with '-' are taken literally (`note T1 --bogus` stores \"--bogus\"); prefer --flag=value when unsure.";

const WAIT_HELP: &str = "Block until a watched task matches.

Selectors (union, at least one; ids need no flag):
  T1 T2...   those tasks
  --epic E1  every task in that epic
  --open     every task not done/dropped, re-evaluated while waiting (new tasks count)

--status CSV  current status in CSV (ready|working|blocking|not_done|needs_fix|review|done|dropped); already true => returns at once
--kind CSV    new event kind in CSV (add|claim|report|note|block|answer|review|done|needs_fix|edit|dispatch; `ask` is an alias of `block`)
Both filters => both must hold; neither => any new event. --after: only events with seq > SEQ.

Exit 0 and first line `matched:`; exit 3 and first line `timeout:`.";

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
    #[command(alias = "mcp")]
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
    /// Epics: project-scoped groups of tasks (E1, E2...)
    Epic {
        #[command(subcommand)]
        action: EpicCmd,
    },
    /// Amend a task's or epic's spec; refused while a worker holds the task
    Edit {
        /// T3 or E3
        id: String,
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
    #[command(alias = "list")]
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
    /// Set a status (ready|working|blocking|not_done|needs_fix|review|done|dropped)
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
