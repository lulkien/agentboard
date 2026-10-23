//! CLI types. Validation and rules live in the store; this file is argv only.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "agentboard",
    version,
    about = "Shared task board for a team of CLI agents",
    after_help = "Exit codes: 0 ok, 1 refused or failed, 3 no ready tasks. Mutations need an actor: --actor or $AGENTBOARD_ACTOR."
)]
pub struct Cli {
    /// Board directory (default $AGENTBOARD_DB, else the state dir's lmdb)
    #[arg(long, global = true, value_name = "DIR")]
    pub db: Option<PathBuf>,
    /// Actor name for this invocation (default $AGENTBOARD_ACTOR)
    #[arg(long, global = true, value_name = "NAME")]
    pub actor: Option<String>,
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// MCP server on stdio (the default with no command)
    Serve,
    /// Create a task
    Add {
        #[arg(allow_hyphen_values = true)]
        title: String,
        #[arg(long, default_value = "", allow_hyphen_values = true)]
        description: String,
        #[arg(long, default_value = "", allow_hyphen_values = true)]
        goal: String,
        #[arg(long)]
        status: Option<String>,
        #[arg(long, default_value_t = 100)]
        priority: i32,
        #[arg(long)]
        assignee: Option<String>,
        #[arg(long, value_name = "ID")]
        parent: Option<u64>,
        #[arg(long)]
        path: Option<String>,
        /// Register an unknown target assignee
        #[arg(long)]
        new_actor: bool,
    },
    /// List tasks, lowest priority first
    Ls {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        assignee: Option<String>,
        #[arg(long, value_name = "ID")]
        parent: Option<u64>,
        #[arg(long)]
        path: Option<String>,
        /// Include done and canceled rows
        #[arg(long)]
        all: bool,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long, default_value_t = 0)]
        offset: usize,
    },
    /// Show one task
    Get {
        #[arg(value_name = "ID")]
        id: u64,
        /// Print the full description and goal
        #[arg(long)]
        full: bool,
    },
    /// Change fields, status or owner
    Update {
        #[arg(value_name = "ID")]
        id: u64,
        #[arg(long = "if-revision", value_name = "REV")]
        if_revision: u64,
        #[arg(long, allow_hyphen_values = true)]
        title: Option<String>,
        #[arg(long, allow_hyphen_values = true)]
        description: Option<String>,
        #[arg(long, allow_hyphen_values = true)]
        goal: Option<String>,
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        priority: Option<i32>,
        #[arg(long)]
        assignee: Option<String>,
        #[arg(long)]
        clear_assignee: bool,
        #[arg(long, value_name = "ID")]
        parent: Option<u64>,
        #[arg(long)]
        clear_parent: bool,
        #[arg(long)]
        path: Option<String>,
        #[arg(long)]
        clear_path: bool,
        /// Override another actor's protected task
        #[arg(long)]
        force: bool,
        #[arg(long)]
        reason: Option<String>,
        #[arg(long, allow_hyphen_values = true)]
        comment: Option<String>,
        /// Register an unknown target assignee
        #[arg(long)]
        new_actor: bool,
    },
    /// Claim the next ready task, or one by id
    Claim {
        #[arg(value_name = "ID")]
        id: Option<u64>,
    },
    /// Append a comment
    Comment {
        #[arg(value_name = "ID")]
        id: u64,
        #[arg(long, allow_hyphen_values = true)]
        body: Option<String>,
        /// Read the body from a file, or - for stdin
        #[arg(long, value_name = "PATH")]
        body_file: Option<String>,
    },
    /// Show the event feed
    Feed {
        #[arg(long, value_name = "CURSOR")]
        after: Option<String>,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        recipient: Option<String>,
        #[arg(long, value_name = "ID")]
        task: Option<u64>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Block up to this many seconds for a matching event
        #[arg(long, default_value_t = 0)]
        wait: u64,
    },
    /// Counts by status plus active work
    Brief,
    /// List registered actors
    Agents,
    /// Print the resolved board directory
    Path,
    /// Discard this board and initialize an empty one
    Reset {
        #[arg(long)]
        yes: bool,
    },
}
