//! CLI entry: parse, call the store, render text, set the exit code.

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use rmcp::ServiceExt;

use agentboard::domain::{self, EventKind, NewProject, NewTask, Status, TaskPatch};
use agentboard::store::{
    AssigneeMatch, Board, BoardOptions, ClaimOutcome, FeedFilter, ListFilter, UpdateInput,
    WaitSpec, parse_status_csv,
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

use cli::{AgentsCmd, Cli, Cmd, ProjectCmd};

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

fn open(path: &PathBuf) -> agentboard::store::Result<Board> {
    Board::open_with_options(path, BoardOptions::from_env()?)
}

fn run(
    db: Option<PathBuf>,
    actor: Option<String>,
    cmd: Option<Cmd>,
) -> Result<(String, ExitCode), Box<dyn std::error::Error>> {
    let path = board_path(db)?;
    let actor = actor_name(actor);
    match cmd {
        None | Some(Cmd::Serve) => serve(path, actor),
        Some(Cmd::Daemon {
            address,
            allow_non_loopback,
        }) => daemon(path, address, allow_non_loopback),
        Some(Cmd::Path) => Ok((path.display().to_string(), ExitCode::SUCCESS)),
        Some(Cmd::Project { cmd }) => {
            let board = open(&path)?;
            match cmd {
                ProjectCmd::Add { name, path } => {
                    let project = board.add_project(NewProject {
                        project_name: name,
                        project_path: path,
                    })?;
                    Ok((project.project_id.to_string(), ExitCode::SUCCESS))
                }
                ProjectCmd::Ls => {
                    let projects = board.list_projects()?;
                    let output = if projects.is_empty() {
                        "no projects".to_owned()
                    } else {
                        projects
                            .iter()
                            .map(render::project_line)
                            .collect::<Vec<_>>()
                            .join("\n")
                    };
                    Ok((output, ExitCode::SUCCESS))
                }
                ProjectCmd::Delete { id } => {
                    let project = board.delete_project(id)?;
                    Ok((render::deleted_project(&project), ExitCode::SUCCESS))
                }
                ProjectCmd::Update { id, name, path } => {
                    let project = board.update_project(id, name, path)?;
                    Ok((render::project_line(&project), ExitCode::SUCCESS))
                }
            }
        }
        Some(Cmd::Reset { yes }) => {
            if !yes {
                return Err("reset discards the board; pass --yes to confirm".into());
            }
            let options = BoardOptions::from_env()?;
            let outcome = Board::reset(&path, options)?;
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
            project,
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
                project_id: project,
            };
            let added = board.add(&actor, input, new_actor, now_ms())?;
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
            project,
            all,
            limit,
            offset,
        }) => {
            let status = parse_status(status.as_deref())?;
            let board = open(&path)?;
            let project = agentboard::server::resolve_project(&board, project)?;
            let page = board.list(&ListFilter {
                status,
                assignee: assignee.as_deref(),
                parent,
                project,
                include_terminal: all,
                limit,
                offset,
            })?;
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
            let epic = board.epic(id)?;
            Ok((render::epic_text(&epic), ExitCode::SUCCESS))
        }
        Some(Cmd::Get { id, full }) => {
            let board = open(&path)?;
            let task = board
                .get(id)?
                .ok_or_else(|| format!("task T{id} does not exist"))?;
            Ok((render::task_record(&task, full), ExitCode::SUCCESS))
        }
        Some(Cmd::Update {
            id,
            if_revision,
            title,
            description,
            goal,
            analyst_report,
            analyst_report_file,
            coder_report,
            coder_report_file,
            reviewer_report,
            reviewer_report_file,
            replace_report,
            status,
            priority,
            assignee,
            clear_assignee,
            parent,
            clear_parent,
            project,
            clear_project,
            release,
            force,
            reason,
            comment,
            new_actor,
        }) => {
            let actor = need_actor(actor)?;
            let status = parse_status(status.as_deref())?;
            let assignee = domain::set_or_clear("assignee", assignee, clear_assignee)?;
            let parent = domain::set_or_clear("parent", parent, clear_parent)?;
            let project = domain::set_or_clear("project", project, clear_project)?;
            let board = open(&path)?;
            let analyst_report =
                merge_report("analyst-report", analyst_report, analyst_report_file)?;
            let coder_report = merge_report("coder-report", coder_report, coder_report_file)?;
            let reviewer_report =
                merge_report("reviewer-report", reviewer_report, reviewer_report_file)?;
            let patch = TaskPatch {
                title,
                description,
                goal,
                analyst_report,
                coder_report,
                reviewer_report,
                replace_reports: replace_report,
                status,
                priority,
                assignee,
                parent_id: parent,
                project_id: project,
            };
            let updated = board.update(UpdateInput {
                id,
                actor: &actor,
                expected_revision: if_revision,
                patch: &patch,
                release,
                force,
                force_reason: reason.as_deref(),
                new_actor,
                comment: comment.as_deref(),
                now_ms: now_ms(),
            })?;
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
            match board.claim(&actor, id, now_ms())? {
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
            let event = board.comment(&actor, id, &text, now_ms())?;
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
            let page = if let Some(wait) = wait {
                board.wait(&filter, wait.saturating_mul(1000))?
            } else {
                board.feed(&filter)?
            };
            Ok((render::feed_page(&page, full), ExitCode::SUCCESS))
        }
        Some(Cmd::Wait {
            ids,
            epic,
            status,
            assignee,
            after,
            timeout,
        }) => {
            let statuses = wait_statuses(status.as_deref())?;
            let board = open(&path)?;
            let outcome = board.wait_tasks(&WaitSpec {
                ids: &ids,
                epic,
                statuses: &statuses,
                after: after.as_deref(),
                timeout_ms: timeout.saturating_mul(1000),
                assignee: assignee.as_ref().map(|name| {
                    name.as_deref()
                        .map_or(AssigneeMatch::Any, AssigneeMatch::To)
                }),
            })?;
            let code = if outcome.matched {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(3)
            };
            Ok((render::wait_text(&outcome), code))
        }
        Some(Cmd::Brief { project }) => {
            let board = open(&path)?;
            let project = agentboard::server::resolve_project(&board, project)?;
            let brief = board.brief_for_project(10, project)?;
            Ok((render::brief_text(&brief), ExitCode::SUCCESS))
        }
        Some(Cmd::Agents { cmd }) => {
            let board = open(&path)?;
            if let Some(AgentsCmd::Delete { name }) = cmd {
                let agent = board.delete_agent(&name)?;
                return Ok((render::deleted_agent(&agent), ExitCode::SUCCESS));
            }
            let agents = board.agents()?;
            if agents.is_empty() {
                return Ok(("no actors".into(), ExitCode::SUCCESS));
            }
            let lines: Vec<String> = agents.iter().map(render::agent_line).collect();
            Ok((lines.join("\n"), ExitCode::SUCCESS))
        }
    }
}

fn serve(
    path: PathBuf,
    actor: Option<String>,
) -> Result<(String, ExitCode), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime: {e}"))?;
    runtime.block_on(async move {
        let board = Board::open_with_options(&path, BoardOptions::from_env()?)?;
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

fn daemon(
    path: PathBuf,
    address: std::net::SocketAddr,
    allow_non_loopback: bool,
) -> Result<(String, ExitCode), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime: {e}"))?;
    runtime.block_on(async move {
        let board = open(&path)?;
        agentboard::daemon::serve(board, address, allow_non_loopback).await?;
        Ok((String::new(), ExitCode::SUCCESS))
    })
}

fn read_source(path: &str) -> Result<String, String> {
    if path == "-" {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|e| e.to_string())?;
        Ok(text)
    } else {
        std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
    }
}

/// One report body from `--<flag>`, `--<flag>-file`, or neither.
fn merge_report(
    flag: &str,
    body: Option<String>,
    file: Option<String>,
) -> Result<Option<String>, String> {
    match (body, file) {
        (Some(_), Some(_)) => Err(format!("choose --{flag} or --{flag}-file, not both")),
        (Some(text), None) => Ok(Some(text)),
        (None, Some(path)) => Ok(Some(read_source(&path)?)),
        (None, None) => Ok(None),
    }
}

fn read_body(body: Option<String>, file: Option<String>) -> Result<String, String> {
    match (body, file) {
        (Some(text), None) => Ok(text),
        (None, Some(path)) => read_source(&path),
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
            let names: Vec<_> = Status::ALL.iter().map(|s| s.as_str()).collect();
            format!("wait: --status needs at least one of {}", names.join(", "))
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
