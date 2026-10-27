//! CLI types. Validation and rules live in the store; this file is argv only.

#[cfg(feature = "dashboard")]
use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::store::WAIT_MAX_MS;

fn feed_wait_seconds(value: &str) -> Result<u64, String> {
    let seconds = value.parse::<u64>().map_err(|error| error.to_string())?;
    let maximum = WAIT_MAX_MS / 1000;
    if seconds == 0 || seconds > maximum {
        return Err(format!(
            "requested wait {seconds}s is outside the allowed range 1..={maximum} seconds; retry with 1 to {maximum} seconds"
        ));
    }
    Ok(seconds)
}

#[derive(Parser, Debug)]
#[command(
    name = "agentboard",
    version,
    about = "Shared task board for a team of CLI agents",
    after_help = "Exit codes: 0 ok, 1 refused or failed, 2 usage, 3 no ready tasks or wait timeout. Mutations need an actor: --actor or $AGENTBOARD_ACTOR."
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
    /// Read-only dashboard over HTTP
    #[cfg(feature = "dashboard")]
    Dashboard {
        /// Address to listen on
        #[arg(value_name = "IP:PORT", default_value = "127.0.0.1:8080")]
        address: SocketAddr,
    },
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
        #[arg(long, default_value_t = 100, allow_hyphen_values = true)]
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
    /// Show direct-child progress for an epic
    Epic {
        #[arg(value_name = "ID")]
        id: u64,
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
        #[arg(long, allow_hyphen_values = true)]
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
        /// Return the row to todo and clear its owner, in one write
        #[arg(long)]
        release: bool,
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
    /// Show the event feed (blocking waits capped at 300 seconds)
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
        /// Print whole comment and update text, untruncated
        #[arg(long)]
        full: bool,
        /// Seconds for a matching event; accepted range 1..=300; omit for a nonblocking read
        #[arg(long, value_parser = feed_wait_seconds)]
        wait: Option<u64>,
    },
    /// Block until watched rows reach a status (300 seconds maximum)
    Wait {
        /// Task ids to watch
        #[arg(value_name = "ID", required = true, num_args = 1..)]
        ids: Vec<u64>,
        /// The ids name epics; watch their direct children instead
        #[arg(long)]
        epic: bool,
        /// CSV of wanted statuses (default done,canceled)
        #[arg(long, value_name = "CSV")]
        status: Option<String>,
        /// Also match when a watched row is handed to a new owner (optionally NAME)
        // No allow_hyphen_values: with an optional value it would swallow the next
        // flag, so `--assignee --epic` would name "--epic". Use `--assignee=-x`.
        #[arg(long, value_name = "NAME", num_args = 0..=1)]
        assignee: Option<Option<String>>,
        /// Seconds; accepted range 1..=300 to bound manager waits; values outside it are rejected
        #[arg(long, default_value_t = WAIT_MAX_MS / 1000, value_parser = feed_wait_seconds)]
        timeout: u64,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocking_wait_ceiling() {
        for (input, expected) in [("1", 1), ("300", 300)] {
            let cli = Cli::try_parse_from(["agentboard", "wait", "1", "--timeout", input]).unwrap();
            let Some(Cmd::Wait { timeout, .. }) = cli.cmd else {
                panic!("expected wait")
            };
            assert_eq!(timeout, expected, "wait --timeout {input}");
        }
        for (input, expected) in [("1", 1), ("300", 300)] {
            let cli = Cli::try_parse_from(["agentboard", "feed", "--wait", input]).unwrap();
            let Some(Cmd::Feed { wait, .. }) = cli.cmd else {
                panic!("expected feed")
            };
            assert_eq!(wait, Some(expected), "feed --wait {input}");
        }
        for input in ["0", "301", "1800", "18446744073709551615"] {
            for args in [
                vec!["agentboard", "wait", "1", "--timeout", input],
                vec!["agentboard", "feed", "--wait", input],
            ] {
                let error = Cli::try_parse_from(args).unwrap_err();
                assert_eq!(error.kind(), clap::error::ErrorKind::ValueValidation);
                assert_ne!(error.exit_code(), 0);
                let diagnostic = error.to_string();
                assert!(diagnostic.contains(input), "{diagnostic}");
                assert!(diagnostic.contains("300"), "{diagnostic}");
                assert!(diagnostic.contains("1..=300"), "{diagnostic}");
            }
        }
        let cli = Cli::try_parse_from(["agentboard", "wait", "1"]).unwrap();
        let Some(Cmd::Wait { timeout, .. }) = cli.cmd else {
            panic!("expected wait")
        };
        assert!((1..=300).contains(&timeout));
        let cli = Cli::try_parse_from(["agentboard", "feed"]).unwrap();
        assert!(matches!(cli.cmd, Some(Cmd::Feed { wait: None, .. })));
    }

    #[test]
    fn wait_assignee_is_an_optional_value() {
        let parse = |extra: &[&str]| {
            let mut args = vec!["agentboard", "wait", "1"];
            args.extend_from_slice(extra);
            let Some(Cmd::Wait { assignee, epic, .. }) = Cli::try_parse_from(args).unwrap().cmd
            else {
                panic!("expected wait")
            };
            (assignee, epic)
        };
        assert_eq!(parse(&[]), (None, false));
        assert_eq!(parse(&["--assignee"]), (Some(None), false));
        assert_eq!(
            parse(&["--assignee", "coder"]),
            (Some(Some("coder".into())), false)
        );
        assert_eq!(parse(&["--assignee", "--epic"]), (Some(None), true));
        assert_eq!(parse(&["--assignee=-x"]), (Some(Some("-x".into())), false));
    }
}
