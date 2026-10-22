//! `agentboard` — one binary, two front doors:
//!   * MCP over stdio (default, or `serve`) for Claude/Hermes agents
//!   * a plain CLI for shell hooks, cron, and humans
//!
//! Both call the same `store` functions, so claim/lease/status semantics exist
//! in exactly one place.

// One backend per build. Both or neither cannot work: fail here, naming the fix.
#[cfg(all(feature = "sqlite", feature = "surreal"))]
compile_error!(
    "features `sqlite` and `surreal` are exclusive. Build with `cargo build --release` (surreal) or `cargo build --release --no-default-features --features sqlite`."
);
#[cfg(not(any(feature = "sqlite", feature = "surreal")))]
compile_error!(
    "enable exactly one backend. Build with `cargo build --release` (surreal) or `cargo build --release --no-default-features --features sqlite`."
);

mod cli;
mod clip;
mod ident;
mod manager;
mod server;
mod store;

use clap::Parser;
use clap::error::ErrorKind;
use cli::{
    Cli, Cmd, DiffCmd, EpicCmd, ManagerAgentCmd, ManagerCmd, ManagerEventsCmd, ManagerLayoutCmd,
    ManagerPaneCmd, ManagerTabCmd, SessionsCmd,
};
use rmcp::ServiceExt;
use rmcp::transport::stdio;

use server::Board;
use std::path::Path;
use std::process::ExitCode;

/// A board path of the wrong kind for this build is refused before anything else
/// runs: a SQLite file where the surreal build wants a data directory, or a
/// directory where the sqlite build wants a file.
#[cfg(all(feature = "surreal", not(feature = "sqlite")))]
fn path_refusal(p: &Path) -> Option<String> {
    p.is_file().then(|| {
        let p = p.display();
        format!(
            "path '{p}' is a SQLite file, not a SurrealDB directory. Move it aside (mv '{p}' '{p}.old') and retry."
        )
    })
}

#[cfg(not(all(feature = "surreal", not(feature = "sqlite"))))]
fn path_refusal(p: &Path) -> Option<String> {
    p.is_dir().then(|| {
        let p = p.display();
        format!(
            "path '{p}' is a directory, not a SQLite board file. Point --db or AGENTBOARD_DB at a file, or move it aside (mv '{p}' '{p}.old') and retry."
        )
    })
}

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
        Cmd::Serve | Cmd::Wait { .. } | Cmd::Manager { .. } => unreachable!("handled in main"),
        Cmd::Path => store::board_path().map(|p| p.display().to_string()),
        Cmd::Board {
            verb: cli::BoardCmd::Upgrade { owner },
        } => store::board_upgrade(owner),
        Cmd::BoardBuild => Ok(ident::line(server::tool_count())),
        Cmd::Add {
            title,
            description,
            goal,
            status,
            assignee,
            priority,
            epic,
            parent,
            path,
            branch,
        } => {
            let epic = epic.as_deref().map(store::parse_epic_id).transpose()?;
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
                epic,
                parent,
                path: path.as_deref(),
                branch: branch.as_deref(),
            };
            store::add(actor, &task)
        }
        Cmd::Edit {
            id,
            title,
            description,
            goal,
            priority,
            assignee,
            path,
            branch,
        } => match store::parse_ref(&id)? {
            store::Ref::Task(n) => {
                let edit = store::TaskEdit {
                    title: title.as_deref(),
                    description: description.as_deref(),
                    goal: goal.as_deref(),
                    priority,
                    assignee: assignee.as_deref(),
                    path: path.as_deref(),
                    branch: branch.as_deref(),
                };
                store::edit(actor, n, &edit).map_err(|e| e.to_string())
            }
            store::Ref::Epic(n) => {
                if priority.is_some() || assignee.is_some() || branch.is_some() {
                    return Err("--priority, --assignee and --branch are for tasks".to_owned());
                }
                let edit = store::EpicEdit {
                    title: title.as_deref(),
                    description: description.as_deref(),
                    goal: goal.as_deref(),
                    path: path.as_deref(),
                };
                store::epic_edit(actor, n, &edit)
            }
        },
        Cmd::Ls {
            status,
            assignee,
            limit,
        } => store::list(status.as_deref(), assignee.as_deref(), limit),
        Cmd::Get { id, full } => store::get(tid(&id)?, full),
        Cmd::Children { id } => store::children(store::parse_ref(&id)?),
        Cmd::Parent { id, parent } => {
            let parent = if parent.trim().eq_ignore_ascii_case("none") {
                None
            } else {
                Some(tid(&parent)?)
            };
            store::set_parent(actor, tid(&id)?, parent)
        }
        Cmd::Diff { action } => match action {
            DiffCmd::Add {
                id,
                body,
                body_file,
                evidence_only,
                reviews,
            } => {
                let task = tid(&id)?;
                let body = match (body, body_file.as_deref()) {
                    (Some(b), _) => b,
                    (None, Some("-")) => std::io::read_to_string(std::io::stdin())
                        .map_err(|e| format!("diff add: reading stdin: {e}"))?,
                    (None, Some(path)) => std::fs::read_to_string(path)
                        .map_err(|e| format!("diff add: reading {path}: {e}"))?,
                    (None, None) => {
                        return Err(String::from(
                            "diff add: pass --body TEXT or --body-file PATH (`-` reads stdin)",
                        ));
                    }
                };
                // The flags are a pair: evidence without a named diff has nothing to
                // re-run, and a named diff without the flag is a plain diff.
                let reviews = match (evidence_only, reviews) {
                    (true, None) => return Err(store::text::evidence_needs_reviews(task)),
                    (false, Some(_)) => return Err(store::text::reviews_needs_evidence(task)),
                    (true, Some(d)) => Some(store::parse_diff_id(&d)?),
                    (false, None) => None,
                };
                store::diff_add(actor, task, &body, reviews)
            }
            DiffCmd::Ls { id } => store::diff_list(store::parse_ref(&id)?),
            DiffCmd::Get { id, full } => store::diff_get(store::parse_diff_id(&id)?, full),
        },
        Cmd::Epic { action } => match action {
            EpicCmd::Add {
                title,
                description,
                goal,
                path,
            } => store::epic_add(
                actor,
                &title,
                &description.unwrap_or_default(),
                &goal.unwrap_or_default(),
                path.as_deref(),
            ),
            EpicCmd::Ls { status } => store::epic_list(status.as_deref()),
            EpicCmd::Get { id, full } => store::epic_get(store::parse_epic_id(&id)?, full),
            EpicCmd::Status { id, status, note } => {
                store::epic_status(actor, store::parse_epic_id(&id)?, &status, note.as_deref())
            }
        },
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
        Cmd::Sessions { action: None } => store::sessions(),
        Cmd::Sessions {
            action: Some(SessionsCmd::Prune { dry_run }),
        } => store::prune(dry_run),
        Cmd::Unregister { name } => store::unregister(&name),
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

/// `manager`: the socket is found and the protocol pinned before any verb runs;
/// `--tmux` is refused before the socket is touched.
fn manager(tmux: bool, verb: ManagerCmd) -> Result<String, manager::Failure> {
    if tmux {
        return Err("--tmux: no tmux backend yet; only --herdr is implemented".into());
    }
    if let ManagerCmd::Layout {
        action: ManagerLayoutCmd::Apply { tab, root, .. },
    } = &verb
    {
        if tab.as_deref().is_none_or(|s| s.trim().is_empty()) {
            return Err("layout apply requires --tab <id>".into());
        }
        if root.is_none() {
            return Err("layout apply requires --root <json|file|->".into());
        }
    }
    let c = manager::Client::connect()?;
    Ok(match verb {
        ManagerCmd::Layout { action } => match action {
            ManagerLayoutCmd::Export { tab, json: compact } => {
                manager::layout_export(&c, tab.as_deref(), compact)?
            }
            ManagerLayoutCmd::Apply { tab, root, force } => {
                return manager::layout_apply(
                    &c,
                    tab.as_deref().unwrap_or_default(),
                    root.as_deref().unwrap_or_default(),
                    force,
                );
            }
        },
        ManagerCmd::Status { json } => manager::status(&c, json)?,
        ManagerCmd::Snapshot { json } => manager::snapshot(&c, json)?,
        ManagerCmd::Up {
            repo,
            coder,
            reviewer,
            label,
            role_name_prefix,
            env,
        } => {
            return manager::up(
                &c,
                &manager::Up {
                    repo: &repo,
                    coder: &coder,
                    reviewer: &reviewer,
                    label: label.as_deref(),
                    prefix: &role_name_prefix,
                    env: &env,
                },
            );
        }
        ManagerCmd::Down { force, names } => return manager::down(&c, &names, force),
        ManagerCmd::Renew { names } => return manager::renew(&c, &names),
        ManagerCmd::Agent { action } => match action {
            ManagerAgentCmd::List => manager::agent_list(&c)?,
            ManagerAgentCmd::Get { target } => manager::agent_get(&c, &target)?,
            ManagerAgentCmd::Read {
                target,
                source,
                lines,
            } => manager::agent_read(&c, &target, &source, lines)?,
            ManagerAgentCmd::Start {
                name,
                kind,
                pane,
                timeout_ms,
                argv,
            } => manager::agent_start(&c, &name, &kind, &pane, timeout_ms, &argv)?,
            ManagerAgentCmd::Prompt { target, text } => manager::agent_prompt(&c, &target, &text)?,
            ManagerAgentCmd::Keys { target, keys } => manager::agent_keys(&c, &target, &keys)?,
            ManagerAgentCmd::Rename {
                target,
                name,
                force,
            } => return manager::agent_rename(&c, &target, &name, force),
            ManagerAgentCmd::Focus { target } => manager::agent_focus(&c, &target)?,
            ManagerAgentCmd::Wait {
                target,
                until,
                timeout_ms,
            } => manager::agent_wait(&c, &target, &until, timeout_ms)?,
        },
        ManagerCmd::Events { action } => match action {
            ManagerEventsCmd::Wait {
                until,
                agent,
                timeout,
                json,
            } => manager::events_wait(&c, &agent, &until, timeout, json)?,
        },
        ManagerCmd::Tab { action } => match action {
            ManagerTabCmd::List => manager::tab_list(&c)?,
            ManagerTabCmd::New { cwd, label } => manager::tab_new(&c, &cwd, label.as_deref())?,
        },
        ManagerCmd::Pane { action } => match action {
            ManagerPaneCmd::List => manager::pane_list(&c)?,
            ManagerPaneCmd::Get { pane_id } => manager::pane_get(&c, &pane_id)?,
            ManagerPaneCmd::Read {
                pane_id,
                source,
                lines,
            } => manager::pane_read(&c, &pane_id, &source, lines)?,
            ManagerPaneCmd::Split {
                pane_id,
                right,
                down: _,
                cwd,
                env,
            } => {
                let dir = if right { "right" } else { "down" };
                manager::pane_split(&c, &pane_id, dir, cwd.as_deref(), &env)?
            }
            ManagerPaneCmd::Close { pane_id, force } => {
                return manager::pane_close(&c, &pane_id, force);
            }
        },
    })
}

/// `wait` is the one command whose exit code carries data: 0 matched, 3 timeout.
/// With `--agent` it also wakes on a herdr agent's transition, whichever comes first.
async fn wait(cmd: Cmd) -> Result<(String, ExitCode), String> {
    let Cmd::Wait {
        ids,
        epic,
        open,
        status,
        kind,
        after,
        agent,
        until,
        timeout,
    } = cmd
    else {
        unreachable!("caller matched Wait")
    };
    let board = store::Wait {
        ids: &ids,
        epic: epic.as_deref(),
        open,
        status: status.as_deref(),
        kind: kind.as_deref(),
        after,
        timeout_ms: timeout.saturating_mul(1000),
    };
    let code = |timed_out| {
        if timed_out {
            ExitCode::from(3)
        } else {
            ExitCode::SUCCESS
        }
    };
    if agent.is_empty() {
        let out = store::wait(board).await?;
        return Ok((out.text, code(out.timed_out)));
    }

    let named = !ids.is_empty() || epic.is_some() || open;
    if !named && (status.is_some() || kind.is_some() || after.is_some()) {
        return Err(
            "wait: --status, --kind and --after need task ids, --epic or --open".to_owned(),
        );
    }
    let until = manager::parse_until(&if until.is_empty() {
        vec!["idle,blocked,done".to_owned()]
    } else {
        until
    })?;
    let limit = std::time::Duration::from_secs(timeout);
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = cancel.clone();
    // Connecting is part of the watch, so every herdr failure reaches one place.
    let mut watch = tokio::task::spawn_blocking(move || {
        let c = manager::Client::connect()?;
        manager::watch(&c, &agent, &until, limit, &flag)
    });

    if !named {
        return Ok(match flatten((&mut watch).await)? {
            (_, Some(h)) => (manager::hit_line(&h), ExitCode::SUCCESS),
            (seen, None) => (manager::timeout_line(limit, &seen), ExitCode::from(3)),
        });
    }
    let board = store::wait(board);
    tokio::pin!(board);
    tokio::select! {
        biased;
        b = &mut board => {
            // Stop the stream and join it, so its connection is closed before we return.
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            settle(b?, flatten(watch.await), limit)
        }
        w = &mut watch => match flatten(w) {
            Ok((_, Some(h))) => Ok((manager::hit_line(&h), ExitCode::SUCCESS)),
            Ok(seen) => settle(board.await?, Ok(seen), limit),
            // herdr failed while the board is still waiting. A board that is already
            // ready (herdr failing fast) still wins; otherwise the herdr error decides.
            Err(e) => match tokio::time::timeout(AGENT_ERROR_GRACE, &mut board).await {
                Ok(b) => settle(b?, Err(e), limit),
                Err(_) => Err(e),
            },
        },
    }
}

/// How long a board that is ready or about to be is given once the agent side failed.
const AGENT_ERROR_GRACE: std::time::Duration = std::time::Duration::from_millis(250);

type Watched = (Vec<String>, Option<manager::Hit>);

fn flatten(r: Result<Result<Watched, String>, tokio::task::JoinError>) -> Result<Watched, String> {
    r.map_err(|e| e.to_string())?
        .map_err(|e| format!("wait: agent watch failed: {e}"))
}

/// The board finished and the agent watch is stopped. One rule: a board match is
/// returned whatever the agent side did (its failure is noted on stderr, so a degraded
/// `--agent` is visible); only when the board timed out does the agent side decide.
fn settle(
    b: store::Outcome,
    w: Result<Watched, String>,
    limit: std::time::Duration,
) -> Result<(String, ExitCode), String> {
    if !b.timed_out {
        if let Err(e) = w {
            eprintln!("agentboard: board matched, --agent watch degraded: {e}");
        }
        return Ok((b.text, ExitCode::SUCCESS));
    }
    match w? {
        (_, Some(h)) => Ok((manager::hit_line(&h), ExitCode::SUCCESS)),
        (seen, None) => Ok((
            format!("{}\n{}", b.text, manager::timeout_line(limit, &seen)),
            ExitCode::from(3),
        )),
    }
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
    // `path` only prints the path; every other verb, `serve` included, reaches the store,
    // so a board path of the wrong kind stops it first. For `serve` that is before the
    // transport starts: one line on stderr, nothing on stdout, no tool call needed.
    if !matches!(cli.cmd, Some(Cmd::Path)) {
        let refused = store::board_path().and_then(|p| path_refusal(&p).map_or(Ok(()), Err));
        if let Err(e) = refused {
            eprintln!("agentboard: {e}");
            return ExitCode::FAILURE;
        }
    }

    let result = match cli.cmd {
        None | Some(Cmd::Serve) => return serve().await,
        Some(c @ Cmd::Wait { .. }) => wait(c).await,
        Some(Cmd::Manager { tmux, verb, .. }) => match manager(tmux, verb) {
            Ok(text) => Ok((text, ExitCode::SUCCESS)),
            // A refusal is "not now", not "broken": exit 3, as `wait` does for a timeout.
            Err(manager::Failure::Refused(why)) => {
                eprintln!("agentboard: {why}");
                return ExitCode::from(3);
            }
            Err(manager::Failure::TimedOut(text)) => {
                println!("{text}");
                return ExitCode::from(3);
            }
            Err(manager::Failure::Other(why)) => Err(why),
        },
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn each_build_says_only_its_own_db_help() {
        let help = Cli::command().render_long_help().to_string();
        let sqlite = "Database path (default $AGENTBOARD_DB, else the state dir)";
        let surreal = "Data directory of the local board server";
        if cfg!(feature = "surreal") {
            assert!(help.contains(surreal) && !help.contains(sqlite), "{help}");
        } else {
            assert!(help.contains(sqlite) && !help.contains(surreal), "{help}");
        }
    }

    #[test]
    fn the_path_verb_and_the_store_resolve_the_same_board_path() {
        // `--db` wins, in both builds, and `path` is the store's own resolution.
        store::set_overrides(Some("/tmp/agentboard-path-test/board".to_owned()), None);
        let want = std::path::PathBuf::from("/tmp/agentboard-path-test/board");
        assert_eq!(store::board_path().unwrap(), want);
        assert_eq!(
            dispatch(Cmd::Path, "a").unwrap(),
            want.display().to_string()
        );
    }

    #[test]
    fn a_board_path_of_the_wrong_kind_is_refused_with_one_line() {
        let root = std::env::temp_dir().join(format!("ab-pathkind-{}", std::process::id()));
        let (file, dir) = (root.join("a-file"), root.join("a-dir"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&file, b"x").unwrap();
        let (file_s, dir_s) = (file.display(), dir.display());
        if cfg!(feature = "surreal") {
            assert_eq!(
                path_refusal(&file).unwrap(),
                format!(
                    "path '{file_s}' is a SQLite file, not a SurrealDB directory. Move it aside (mv '{file_s}' '{file_s}.old') and retry."
                )
            );
            assert!(path_refusal(&dir).is_none());
        } else {
            assert_eq!(
                path_refusal(&dir).unwrap(),
                format!(
                    "path '{dir_s}' is a directory, not a SQLite board file. Point --db or AGENTBOARD_DB at a file, or move it aside (mv '{dir_s}' '{dir_s}.old') and retry."
                )
            );
            assert!(path_refusal(&file).is_none());
        }
        assert!(path_refusal(&root.join("missing")).is_none());
        assert!(!path_refusal(&file).unwrap_or_default().contains('\n'));
        let _ = std::fs::remove_dir_all(&root);
    }
}
