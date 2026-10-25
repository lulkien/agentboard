//! CLI entry: parse, call the store, render text, set the exit code.

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use rmcp::ServiceExt;

use agentboard::domain::{self, EventKind, NewTask, Status, TaskPatch};
use agentboard::store::{
    Board, BoardOptions, ClaimOutcome, FeedFilter, ListFilter, UpdateInput, WaitSpec,
    parse_status_csv,
};
use agentboard::{cli, render};

fn main() -> ExitCode {
    let args = Cli::try_parse().unwrap_or_else(|error| error.exit());
    let Cli { db, actor, cmd } = args;
    match run(db, actor, cmd) {
        Ok((text, code)) => {
            if !text.is_empty() {
                println!("{text}");
            }
            code
        }
        Err(error) => {
            eprintln!("agentboard: {error}");
            ExitCode::FAILURE
        }
    }
}

use cli::{Cli, Cmd};

fn board_path(db: Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(path) = db {
        return Ok(path);
    }
    if let Ok(path) = std::env::var("AGENTBOARD_DB") {
        return Ok(PathBuf::from(path));
    }
    let base = match std::env::var("XDG_STATE_HOME") {
        Ok(base) => PathBuf::from(base),
        Err(_) => {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".local/state")
        }
    };
    Ok(base.join("agentboard/lmdb"))
}

fn actor_name(actor: Option<String>) -> Option<String> {
    actor
        .or_else(|| std::env::var("AGENTBOARD_ACTOR").ok())
        .filter(|name| !name.trim().is_empty())
}

fn need_actor(actor: Option<String>) -> Result<String, String> {
    actor.ok_or_else(|| {
        "actor required: pass --actor or set AGENTBOARD_ACTOR (names are board addresses)"
            .to_owned()
    })
}

fn open(path: &PathBuf) -> Result<Board, String> {
    Board::open_with_options(path, BoardOptions::from_env().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

fn run(
    db: Option<PathBuf>,
    actor: Option<String>,
    cmd: Option<Cmd>,
) -> Result<(String, ExitCode), String> {
    let path = board_path(db)?;
    let actor = actor_name(actor);
    match cmd {
        None | Some(Cmd::Serve) => serve(path, actor),
        Some(Cmd::Path) => Ok((path.display().to_string(), ExitCode::SUCCESS)),
        Some(Cmd::Reset { yes }) => {
            if !yes {
                return Err("reset discards the board; pass --yes to confirm".into());
            }
            let options = BoardOptions::from_env().map_err(|e| e.to_string())?;
            let outcome = Board::reset(&path, options).map_err(|e| e.to_string())?;
            Ok((
                format!(
                    "reset {}\nnew board instance {}",
                    outcome.path.display(),
                    outcome.new_instance_id
                ),
                ExitCode::SUCCESS,
            ))
        }
        Some(Cmd::Add {
            title,
            description,
            goal,
            status,
            priority,
            assignee,
            parent,
            path: project,
            new_actor,
        }) => {
            let actor = need_actor(actor)?;
            let status = parse_status(status.as_deref())?.unwrap_or(Status::Todo);
            let board = open(&path)?;
            let input = NewTask {
                title,
                description,
                goal,
                status,
                priority,
                assignee,
                parent_id: parent,
                path: project,
            };
            let added = board
                .add(&actor, input, new_actor, now_ms())
                .map_err(|e| e.to_string())?;
            Ok((
                format!(
                    "ok\n{}\ncursor {}",
                    render::task_record(&added.task, false),
                    added.event.seq
                ),
                ExitCode::SUCCESS,
            ))
        }
        Some(Cmd::Ls {
            status,
            assignee,
            parent,
            path: project,
            all,
            limit,
            offset,
        }) => {
            let status = parse_status(status.as_deref())?;
            let board = open(&path)?;
            let page = board
                .list(&ListFilter {
                    status,
                    assignee: assignee.as_deref(),
                    parent,
                    path: project.as_deref(),
                    include_terminal: all,
                    limit,
                    offset,
                })
                .map_err(|e| e.to_string())?;
            let mut lines: Vec<String> = page.tasks.iter().map(render::task_line).collect();
            if lines.is_empty() {
                lines.push("no tasks".into());
            }
            lines.push(format!(
                "total {} cursor {}",
                page.total,
                page.cursor.encode()
            ));
            if page.more {
                lines.push("more true".into());
            }
            Ok((lines.join("\n"), ExitCode::SUCCESS))
        }
        Some(Cmd::Epic { id }) => {
            let board = open(&path)?;
            let epic = board.epic(id).map_err(|e| e.to_string())?;
            Ok((render::epic_text(&epic), ExitCode::SUCCESS))
        }
        Some(Cmd::Get { id, full }) => {
            let board = open(&path)?;
            let task = board
                .get(id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("task T{id} does not exist"))?;
            Ok((render::task_record(&task, full), ExitCode::SUCCESS))
        }
        Some(Cmd::Update {
            id,
            if_revision,
            title,
            description,
            goal,
            status,
            priority,
            assignee,
            clear_assignee,
            parent,
            clear_parent,
            path: project,
            clear_path,
            force,
            reason,
            comment,
            new_actor,
        }) => {
            let actor = need_actor(actor)?;
            let status = parse_status(status.as_deref())?;
            if clear_assignee && assignee.is_some() {
                return Err("choose --assignee or --clear-assignee, not both".into());
            }
            if clear_parent && parent.is_some() {
                return Err("choose --parent or --clear-parent, not both".into());
            }
            if clear_path && project.is_some() {
                return Err("choose --path or --clear-path, not both".into());
            }
            let patch = TaskPatch {
                title,
                description,
                goal,
                status,
                priority,
                assignee: if clear_assignee {
                    Some(None)
                } else {
                    assignee.map(Some)
                },
                parent_id: if clear_parent {
                    Some(None)
                } else {
                    parent.map(Some)
                },
                path: if clear_path {
                    Some(None)
                } else {
                    project.map(Some)
                },
            };
            let board = open(&path)?;
            let updated = board
                .update(UpdateInput {
                    id,
                    actor: &actor,
                    expected_revision: if_revision,
                    patch: &patch,
                    force,
                    force_reason: reason.as_deref(),
                    new_actor,
                    comment: comment.as_deref(),
                    now_ms: now_ms(),
                })
                .map_err(|e| e.to_string())?;
            Ok((
                format!(
                    "ok\n{}\ncursor {}",
                    render::task_record(&updated.task, false),
                    updated.event.seq
                ),
                ExitCode::SUCCESS,
            ))
        }
        Some(Cmd::Claim { id }) => {
            let actor = need_actor(actor)?;
            let board = open(&path)?;
            match board
                .claim(&actor, id, now_ms())
                .map_err(|e| e.to_string())?
            {
                ClaimOutcome::NoWork => Ok(("no ready tasks".into(), ExitCode::from(3))),
                outcome => Ok((render::claim_line(&outcome), ExitCode::SUCCESS)),
            }
        }
        Some(Cmd::Comment {
            id,
            body,
            body_file,
        }) => {
            let actor = need_actor(actor)?;
            let text = read_body(body, body_file)?;
            let board = open(&path)?;
            let event = board
                .comment(&actor, id, &text, now_ms())
                .map_err(|e| e.to_string())?;
            Ok((format!("ok\ncursor {}", event.seq), ExitCode::SUCCESS))
        }
        Some(Cmd::Feed {
            after,
            kind,
            recipient,
            task,
            limit,
            full,
            wait,
        }) => {
            let kind = parse_kind(kind.as_deref())?;
            let board = open(&path)?;
            let filter = FeedFilter {
                task,
                kind,
                recipient: recipient.as_deref(),
                after: after.as_deref(),
                limit,
            };
            let page = if wait == 0 {
                board.feed(&filter).map_err(|e| e.to_string())?
            } else {
                board
                    .wait(&filter, wait.saturating_mul(1000))
                    .map_err(|e| e.to_string())?
            };
            Ok((render::feed_page(&page, full), ExitCode::SUCCESS))
        }
        Some(Cmd::Wait {
            ids,
            epic,
            status,
            timeout,
        }) => {
            let statuses = wait_statuses(status.as_deref())?;
            let board = open(&path)?;
            let outcome = board
                .wait_tasks(&WaitSpec {
                    ids: &ids,
                    epic,
                    statuses: &statuses,
                    after: None,
                    timeout_ms: timeout.saturating_mul(1000),
                })
                .map_err(|e| e.to_string())?;
            let code = if outcome.matched {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(3)
            };
            Ok((render::wait_text(&outcome), code))
        }
        Some(Cmd::Brief) => {
            let board = open(&path)?;
            let brief = board.brief(10).map_err(|e| e.to_string())?;
            Ok((render::brief_text(&brief), ExitCode::SUCCESS))
        }
        Some(Cmd::Agents) => {
            let board = open(&path)?;
            let agents = board.agents().map_err(|e| e.to_string())?;
            if agents.is_empty() {
                return Ok(("no actors".into(), ExitCode::SUCCESS));
            }
            let lines: Vec<String> = agents.iter().map(render::agent_line).collect();
            Ok((lines.join("\n"), ExitCode::SUCCESS))
        }
    }
}

fn serve(path: PathBuf, actor: Option<String>) -> Result<(String, ExitCode), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime: {e}"))?;
    runtime.block_on(async move {
        let board =
            Board::open_with_options(&path, BoardOptions::from_env().map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        let server = agentboard::server::Board::new(board, actor);
        let running = server
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|error| format!("mcp server: {error}"))?;
        running
            .waiting()
            .await
            .map_err(|error| format!("mcp server: {error}"))?;
        Ok((String::new(), ExitCode::SUCCESS))
    })
}

fn read_body(body: Option<String>, file: Option<String>) -> Result<String, String> {
    match (body, file) {
        (Some(text), None) => Ok(text),
        (None, Some(path)) => {
            let mut text = String::new();
            if path == "-" {
                std::io::stdin()
                    .read_to_string(&mut text)
                    .map_err(|e| e.to_string())?;
            } else {
                text = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
            }
            Ok(text)
        }
        (Some(_), Some(_)) => Err("choose --body or --body-file, not both".into()),
        (None, None) => Err("comment needs --body or --body-file".into()),
    }
}

fn parse_status(raw: Option<&str>) -> Result<Option<Status>, String> {
    raw.map(|value| Status::parse(value).map_err(|e| e.to_string()))
        .transpose()
}

/// Wanted statuses for a wait: an absent flag means `done,canceled`. An empty list
/// is refused by name here, because a wait with nothing wanted would match at once.
fn wait_statuses(raw: Option<&str>) -> Result<Vec<Status>, String> {
    let Some(raw) = raw else {
        return Ok(vec![Status::Done, Status::Canceled]);
    };
    parse_status_csv(raw).map_err(|error| match error {
        agentboard::store::Error::WaitWithoutStatus => {
            "wait: --status needs at least one of todo, in_progress, blocked, review, done, canceled"
                .to_owned()
        }
        other => other.to_string(),
    })
}

fn parse_kind(raw: Option<&str>) -> Result<Option<EventKind>, String> {
    raw.map(|value| EventKind::parse(value).map_err(|e| e.to_string()))
        .transpose()
}

fn now_ms() -> i64 {
    domain::now_ms()
}
