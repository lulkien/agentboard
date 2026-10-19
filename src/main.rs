//! `agentboard` — one binary, two front doors:
//!   * MCP over stdio (default, or `serve`) for Claude/Hermes agents
//!   * a plain CLI for shell hooks, cron, and humans
//!
//! Both call the same `store` functions, so claim/lease/status semantics exist
//! in exactly one place.

mod server;
mod store;

use rmcp::ServiceExt;
use rmcp::transport::stdio;

use server::Board;
use std::process::ExitCode;

const USAGE: &str = "\
agentboard — task board + tmux bridge for a team of CLI agents

  agentboard                     MCP server on stdio (default)
  agentboard add <title> [--description D | --spec D] [--goal G] [--status S] [--assignee A] [--priority N] [--parent T1]
  agentboard ls [--status S] [--assignee A] [--limit N]
  agentboard get <T1>
  agentboard claim [<T1>] [--ttl MIN]
  agentboard note <T1> <text>
  agentboard report <T1> <text>
  agentboard ask <T1> <question>
  agentboard answer <T1> <text>
  agentboard done <T1> <summary> [--commit REF]
  agentboard review <T1> <summary>
  agentboard status <T1> <status> [note]
  agentboard block <T1> <reason>
  agentboard wait <T1> [--timeout SEC] [--after SEQ]
  agentboard brief [--since MIN]
  agentboard sessions
  agentboard register <tmux-name> [--agent claude|hermes] [--task T1]
  agentboard tail <tmux-name> [--lines N]
  agentboard send <tmux-name> <text>
  agentboard path

Global: --actor NAME (default $AGENTBOARD_ACTOR, else 'agent'), --db PATH
";

const VALUE_FLAGS: [&str; 18] = [
    "--actor", "--db", "--spec", "--description", "--goal", "--assignee", "--priority", "--parent", "--status", "--limit",
    "--after", "--ttl", "--commit", "--timeout", "--since", "--agent", "--task", "--lines",
];

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn has(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

/// Positional args: everything that is not a flag or a flag's value.
fn positional(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
            continue;
        }
        if VALUE_FLAGS.contains(&a.as_str()) {
            skip = true;
            continue;
        }
        if a.starts_with("--") {
            continue;
        }
        out.push(a.clone());
    }
    out
}

fn req<'a>(v: Option<&'a String>, what: &str) -> Result<&'a str, String> {
    v.map(String::as_str).ok_or_else(|| format!("missing {what}"))
}

fn tid(raw: Option<&String>) -> Result<i64, String> {
    store::parse_id(req(raw, "task id")?)
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

async fn dispatch(cmd: &str, pos: &[String], argv: &[String], actor: &str) -> Result<String, String> {
    match cmd {
        "path" => Ok(store::db_path().display().to_string()),
        "add" => {
            let title = req(pos.get(1), "title")?;
            let parent = flag(argv, "--parent").map(|p| store::parse_id(&p)).transpose()?;
            let prio = flag(argv, "--priority").and_then(|p| p.parse().ok()).unwrap_or(100);
            let description = flag(argv, "--description").or_else(|| flag(argv, "--spec")).unwrap_or_default();
            let goal = flag(argv, "--goal").unwrap_or_default();
            let status = match flag(argv, "--status") {
                Some(raw) => store::norm(&raw).ok_or_else(|| format!("bad status {raw:?}"))?,
                None => "ready",
            };
            store::add(actor, title, &description, &goal, status, flag(argv, "--assignee").as_deref(), prio, parent)
        }
        "ls" | "list" => {
            let limit = flag(argv, "--limit").and_then(|p| p.parse().ok()).unwrap_or(20);
            store::list(flag(argv, "--status").as_deref(), flag(argv, "--assignee").as_deref(), limit)
        }
        "get" => store::get(tid(pos.get(1))?),
        "claim" => {
            let id = pos.get(1).map(|p| store::parse_id(p)).transpose()?;
            let ttl = flag(argv, "--ttl").and_then(|p| p.parse().ok()).unwrap_or(30);
            store::claim(actor, id, ttl * 60)
        }
        "note" => store::note(actor, tid(pos.get(1))?, req(pos.get(2), "text")?),
        "report" => store::report(actor, tid(pos.get(1))?, req(pos.get(2), "text")?),
        "ask" => store::ask(actor, tid(pos.get(1))?, req(pos.get(2), "question")?),
        "answer" => store::answer(actor, tid(pos.get(1))?, req(pos.get(2), "text")?),
        "done" => store::finish(actor, tid(pos.get(1))?, "done", req(pos.get(2), "summary")?, flag(argv, "--commit").as_deref()),
        "review" => store::finish(actor, tid(pos.get(1))?, "review", req(pos.get(2), "summary")?, flag(argv, "--commit").as_deref()),
        "block" => store::block(actor, tid(pos.get(1))?, req(pos.get(2), "reason")?),
        "status" => {
            let want = req(pos.get(2), "status")?;
            let patch = store::Patch {
                status: Some(want),
                report: None,
                note: pos.get(3).map(String::as_str),
                commit: None,
                needs: None,
                ttl_s: 0,
            };
            store::apply(actor, tid(pos.get(1))?, &patch)
        }
        "wait" => {
            let secs = flag(argv, "--timeout").and_then(|p| p.parse().ok()).unwrap_or(60);
            let after = flag(argv, "--after").and_then(|p| p.parse().ok());
            store::wait(tid(pos.get(1))?, after, secs * 1000).await
        }
        "brief" => store::brief(flag(argv, "--since").and_then(|p| p.parse().ok()).unwrap_or(120)),
        "sessions" => store::sessions(),
        "register" => {
            let name = req(pos.get(1), "tmux session name")?;
            let task = flag(argv, "--task").map(|p| store::parse_id(&p)).transpose()?;
            let cwd = std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default();
            let agent = flag(argv, "--agent").unwrap_or_else(|| "agent".to_owned());
            store::register_session(name, &agent, &cwd, task)
        }
        "tail" => {
            let name = req(pos.get(1), "tmux session name")?;
            let lines = flag(argv, "--lines").and_then(|p| p.parse().ok()).unwrap_or(40);
            store::tail(name, lines)
        }
        "send" => store::send(req(pos.get(1), "tmux session name")?, req(pos.get(2), "text")?),
        other => Err(format!("unknown command {other:?} (try --help)")),
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if has(&argv, "--help") || has(&argv, "-h") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    store::set_overrides(flag(&argv, "--db"), flag(&argv, "--actor"));
    let actor = store::default_actor();

    let pos = positional(&argv);
    let cmd = pos.first().map_or("serve", String::as_str);
    if matches!(cmd, "serve" | "mcp") {
        return serve().await;
    }

    match dispatch(cmd, &pos, &argv, &actor).await {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("agentboard: {e}");
            ExitCode::FAILURE
        }
    }
}
