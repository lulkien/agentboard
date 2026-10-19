//! `agentboard` — one binary, two front doors:
//!   * MCP over stdio (default, or `serve`) for Claude/Hermes agents
//!   * a plain CLI for shell hooks, cron, and humans
//!
//! Both call the same `store` functions, so claim/lease/status semantics exist
//! in exactly one place.

mod cli;
mod clip;
mod server;
mod store;

use clap::Parser;
use clap::error::ErrorKind;
use cli::{Cli, Cmd};
use rmcp::ServiceExt;
use rmcp::transport::stdio;

use server::Board;
use std::process::ExitCode;

fn tid(raw: &str) -> Result<i64, String> {
    store::parse_id(raw)
}

async fn serve() -> ExitCode {
    let service = match Board::new().serve(stdio()).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("agentboard: serve failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    match service.waiting().await {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("agentboard: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Every command but `wait`: text out, exit 0 or 1.
fn dispatch(cmd: Cmd, actor: &str) -> Result<String, String> {
    match cmd {
        Cmd::Serve | Cmd::Wait { .. } => unreachable!("handled in main"),
        Cmd::Path => Ok(store::db_path().display().to_string()),
        Cmd::Add {
            title,
            description,
            goal,
            status,
            assignee,
            priority,
            parent,
        } => {
            let parent = parent.as_deref().map(tid).transpose()?;
            let status = match status {
                Some(raw) => store::norm(&raw).ok_or_else(|| format!("bad status {raw:?}"))?,
                None => "ready",
            };
            let task = store::NewTask {
                title: &title,
                description: &description.unwrap_or_default(),
                goal: &goal.unwrap_or_default(),
                status,
                assignee: assignee.as_deref(),
                priority,
                parent,
            };
            store::add(actor, &task)
        }
        Cmd::Ls {
            status,
            assignee,
            limit,
        } => store::list(status.as_deref(), assignee.as_deref(), limit),
        Cmd::Get { id, full } => store::get(tid(&id)?, full),
        Cmd::Children { id } => store::children(tid(&id)?),
        Cmd::Claim { id, ttl } => store::claim(
            actor,
            id.as_deref().map(tid).transpose()?,
            ttl.clamp(1, 720) * 60,
        )
        .map_err(|e| e.to_string()),
        Cmd::Heartbeat { id, ttl } => store::heartbeat(actor, tid(&id)?, ttl.clamp(1, 720) * 60),
        Cmd::Note { id, text } => store::note(actor, tid(&id)?, &text),
        Cmd::Report { id, text } => store::report(actor, tid(&id)?, &text),
        Cmd::Ask { id, question } => store::ask(actor, tid(&id)?, &question),
        Cmd::Answer { id, text } => store::answer(actor, tid(&id)?, &text),
        Cmd::Done {
            id,
            summary,
            commit,
        } => store::finish(actor, tid(&id)?, "done", &summary, commit.as_deref()),
        Cmd::Review {
            id,
            summary,
            commit,
        } => store::finish(actor, tid(&id)?, "review", &summary, commit.as_deref()),
        Cmd::Block { id, reason } => store::block(actor, tid(&id)?, &reason),
        Cmd::Status { id, status, note } => {
            let patch = store::Patch {
                status: Some(&status),
                report: None,
                note: note.as_deref(),
                commit: None,
                needs: None,
                ttl_s: 0,
            };
            store::apply(actor, tid(&id)?, &patch)
        }
        Cmd::Brief { since } => store::brief(since),
        Cmd::Sessions => store::sessions(),
        Cmd::Register { name, agent, task } => {
            let task = task.as_deref().map(tid).transpose()?;
            let cwd = std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            store::register_session(&name, &agent, &cwd, task)
        }
        Cmd::Tail { name, lines } => store::tail(&name, lines),
        Cmd::Dispatch {
            id,
            to,
            force,
            print,
        } => store::dispatch(actor, tid(&id)?, to.as_deref(), force, print),
        Cmd::Send { name, text } => store::send(&name, &text),
    }
}

/// `wait` is the one command whose exit code carries data: 0 matched, 3 timeout.
async fn wait(cmd: Cmd) -> Result<(String, ExitCode), String> {
    let Cmd::Wait {
        ids,
        epic,
        open,
        status,
        kind,
        after,
        timeout,
    } = cmd
    else {
        unreachable!("caller matched Wait")
    };
    let out = store::wait(store::Wait {
        ids: &ids,
        epic: epic.as_deref(),
        open,
        status: status.as_deref(),
        kind: kind.as_deref(),
        after,
        timeout_ms: timeout.saturating_mul(1000),
    })
    .await?;
    Ok((
        out.text,
        if out.timed_out {
            ExitCode::from(3)
        } else {
            ExitCode::SUCCESS
        },
    ))
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        // --help / --version are not errors: stdout, exit 0.
        Err(e) if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) => {
            print!("{e}");
            return ExitCode::SUCCESS;
        }
        // clap exits 2 on misuse; the contract is 1, and agents read the prefix.
        Err(e) => {
            eprintln!("agentboard: {}", e.to_string().trim_end());
            return ExitCode::FAILURE;
        }
    };
    store::set_overrides(cli.db, cli.actor);
    let actor = store::default_actor();

    let result = match cli.cmd {
        None | Some(Cmd::Serve) => return serve().await,
        Some(c @ Cmd::Wait { .. }) => wait(c).await,
        Some(c) => dispatch(c, &actor).map(|text| (text, ExitCode::SUCCESS)),
    };
    match result {
        Ok((text, code)) => {
            println!("{text}");
            code
        }
        Err(e) => {
            eprintln!("agentboard: {e}");
            ExitCode::FAILURE
        }
    }
}
