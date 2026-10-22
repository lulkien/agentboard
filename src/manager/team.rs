//! The composite verbs: `up` opens a team (tab, two panes, two agents, board
//! rows), `down` closes it, `renew` clears both panes between epics. Each is a
//! fixed sequence of the methods in `write`, and every stop goes through `guard`.

use super::client::Client;
use super::write::guard;
use super::{Failure, Seat};
use crate::store;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::{Duration, Instant};

const START_TIMEOUT_MS: u64 = 120_000;
const PROMPT_TIMEOUT_MS: u64 = 10_000;
/// The one budget for a fresh pane's shell to become available. A shell starts in
/// well under a second (the old helpers slept 1s); 10s is a wide margin that still
/// fails fast on a pane that never comes up. Independent of the start and prompt
/// budgets because `agent_pane_busy` is refused immediately, so no retry spends
/// `START_TIMEOUT_MS`.
const SHELL_READY_BUDGET: Duration = Duration::from_secs(10);
const SHELL_RETRY_WAIT: Duration = Duration::from_millis(500);
/// Screen text that shows an agent's input box. The layouts are the agents' own;
/// these are the lines `renew` accepts as evidence of a prompt, and a screen with
/// none of them is refused with its text, which is how a new layout gets noticed.
const PROMPT_MARKERS: [&str; 2] = ["Ask anything", "Try \""];

fn home_file(rel: &str) -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty());
    Ok(PathBuf::from(home.ok_or("HOME is not set")?).join(rel))
}

/// The argv a seat's agent starts with. claude needs its MCP config, which is what
/// gives it its board tools and actor; opencode needs `--standalone` or its TUI
/// prints a banner and exits. Codex selects its role profile with `-p`.
/// The config is checked here so a missing file refuses
/// before any pane exists.
fn seat_args(role: &str, kind: &str) -> Result<Vec<String>, String> {
    match kind {
        "claude" => {
            let conf = home_file(&format!(".claude/{role}-mcp.json"))?;
            if !conf.is_file() {
                return Err(format!("up: {} does not exist", conf.display()));
            }
            let mut args = vec![
                "--strict-mcp-config".to_owned(),
                format!("--mcp-config={}", conf.display()),
            ];
            // Passed only when present, or claude refuses to start without it.
            let voice = home_file(".claude/caveman-full.md")?;
            if voice.is_file() {
                args.push(format!("--append-system-prompt-file={}", voice.display()));
            }
            Ok(args)
        }
        "opencode" => Ok(vec!["--standalone".to_owned()]),
        "codex" => Ok(vec!["-p".to_owned(), actor_role(role).to_owned()]),
        _ => Err(format!("up: no seat argv for kind {kind:?}")),
    }
}

// Claude's existing MCP config calls the coder role "coding".
fn actor_role(role: &str) -> &str {
    if role == "coding" { "coder" } else { role }
}

fn seat_env(env: &[(String, String)], role: &str) -> Vec<(String, String)> {
    let mut env: Vec<_> = env
        .iter()
        .filter(|(k, _)| k != "AGENTBOARD_ACTOR")
        .cloned()
        .collect();
    env.push(("AGENTBOARD_ACTOR".to_owned(), actor_role(role).to_owned()));
    env
}

pub struct Up<'a> {
    pub repo: &'a str,
    pub coder: &'a str,
    pub reviewer: &'a str,
    pub label: Option<&'a str>,
    pub prefix: &'a str,
    /// `K=V`, shared by both panes; AGENTBOARD_ACTOR is always the seat role.
    pub env: &'a [String],
}

pub fn up(c: &Client, o: &Up) -> Result<String, Failure> {
    let env = super::parse_env(o.env)?;
    let repo = std::fs::canonicalize(o.repo)
        .ok()
        .filter(|p| p.is_dir())
        .ok_or_else(|| format!("up: --repo {}: not a directory", o.repo))?;
    let repo_s = repo.display().to_string();
    let label = o.label.map(str::to_owned).unwrap_or_else(|| {
        repo.file_name()
            .map_or("team".to_owned(), |n| n.to_string_lossy().into_owned())
    });
    let seats = [
        Seat {
            role: "coding",
            kind: o.coder,
            name: format!("{}coder", o.prefix),
        },
        Seat {
            role: "reviewer",
            kind: o.reviewer,
            name: format!("{}reviewer", o.prefix),
        },
    ];
    let argv = [
        seat_args(seats[0].role, seats[0].kind)?,
        seat_args(seats[1].role, seats[1].kind)?,
    ];
    let live = c.agents()?;
    for s in &seats {
        if let Some(a) = live.iter().find(|a| a.name.as_deref() == Some(&s.name)) {
            return Err(format!(
                "up: {} is already live in {}; run `manager down` first or pick another --role-name-prefix",
                s.name, a.pane_id
            )
            .into());
        }
    }

    let tab = c.tab_create(&repo_s, Some(&label), &seat_env(&env, seats[0].role))?;
    let mut panes = vec![tab.root_pane.clone()];
    let mut registered: Vec<&str> = Vec::new();
    let made = (|| -> Result<Vec<String>, String> {
        panes.push(c.pane_split(
            &tab.root_pane,
            "right",
            Some(&repo_s),
            &seat_env(&env, seats[1].role),
        )?);
        let mut notes = Vec::new();
        for (i, s) in seats.iter().enumerate() {
            let pane = panes[i].clone();
            start_when_shell_ready(c, s, &pane, &argv[i])?;
            notes.extend(settle(c, &s.name, &pane, &repo_s)?);
        }
        // Rows only once both seats are at a prompt, so a failure on the second
        // seat leaves nothing behind but what the cleanup below removes.
        for s in &seats {
            store::register_session(&s.name, s.kind, &repo_s, None)?;
            registered.push(&s.name);
        }
        Ok(notes)
    })();
    let notes = match made {
        Ok(n) => n,
        Err(e) => {
            let closed: Vec<&str> = panes
                .iter()
                .filter(|p| c.pane_close(p).is_ok())
                .map(String::as_str)
                .collect();
            for name in &registered {
                let _ = store::unregister(name);
            }
            return Err(format!("{e} (closed {})", closed.join(", ")).into());
        }
    };
    let mut line = format!(
        "up tab {}: {} pane {} ({}), {} pane {} ({})",
        tab.tab_id, seats[0].name, panes[0], seats[0].kind, seats[1].name, panes[1], seats[1].kind
    );
    // A trust answer is a decision made for the operator: never silent.
    for n in notes {
        line.push_str("; ");
        line.push_str(&n);
    }
    Ok(line)
}

/// herdr answers `agent.start` with `agent_pane_busy: ... is not an available
/// shell` in the window between `tab.create`/`pane.split` and the new pane's shell
/// being ready. Observed live on herdr 0.9.0: the first start after a create was
/// refused this way, and a split tab root DID accept `agent.start` once its shell was
/// up, so this is a startup window, not a rule about roots. The old shell helpers hid
/// it behind `sleep 1`; here that one error is retried within `SHELL_READY_BUDGET`,
/// and any other error, or the same one at the deadline, ends the start.
fn start_when_shell_ready(c: &Client, s: &Seat, pane: &str, args: &[String]) -> Result<(), String> {
    let end = Instant::now() + SHELL_READY_BUDGET;
    loop {
        match c.agent_start(&s.name, s.kind, pane, Some(START_TIMEOUT_MS), args) {
            Err(e) if e.contains("agent_pane_busy") && Instant::now() < end => {
                sleep(SHELL_RETRY_WAIT)
            }
            done => return done,
        }
    }
}

/// How long a started seat gets to reach its prompt. `AGENTBOARD_UP_PROMPT_TIMEOUT_MS`
/// overrides it.
fn prompt_budget() -> Duration {
    let ms = std::env::var("AGENTBOARD_UP_PROMPT_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&ms| ms > 0)
        .unwrap_or(60_000);
    Duration::from_millis(ms)
}

fn screen_tail(c: &Client, pane: &str) -> String {
    let text = c
        .pane_read(pane, "recent", Some(12))
        .map(|r| r.text)
        .unwrap_or_default();
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let tail = &lines[lines.len().saturating_sub(4)..];
    if tail.is_empty() {
        "(blank)".to_owned()
    } else {
        tail.join(" | ")
    }
}

/// True when `screen` names `repo` as a whole path token: the character before the
/// match and the one after it must both be non-path characters (start or end of
/// text, whitespace, a quote, a box border). So `/tmp/repo` is found in
/// `for /tmp/repo ?` but not in `/tmp/repo2`, `/tmp/repo-x`, `/home/u/tmp/repo` or
/// `/x/tmp/repo`.
fn names_repo(screen: &str, repo: &str) -> bool {
    let path_char = |c: char| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '~');
    screen.match_indices(repo).any(|(i, _)| {
        let before = screen[..i].chars().next_back();
        let after = screen[i + repo.len()..].chars().next();
        !before.is_some_and(path_char) && !after.is_some_and(path_char)
    })
}

/// herdr names an agent after its kind when a startup dialog delayed readiness, so
/// the name is forced. Then the seat must END AT A PROMPT, or `up` refuses with what
/// is on its screen.
///
/// A fresh folder raises claude's folder-trust dialog. Answering it (down, enter; the
/// cursor starts on "No, exit") is a decision made for the operator, so it is made
/// only when the screen shows the dialog is for `repo` (a full-path match on the
/// unwrapped screen), and the returned note says so. A dialog whose folder is another
/// path, or cannot be read off the screen, gets no keys: the start fails at once
/// naming the screen. The dialog's wording and layout are claude's; the stub fixture
/// only pins how this code handles them.
fn settle(c: &Client, name: &str, pane: &str, repo: &str) -> Result<Option<String>, String> {
    let end = Instant::now() + prompt_budget();
    let mut renamed = false;
    let mut note = None;
    loop {
        let live = c.agents()?;
        let seat = live.iter().find(|a| a.pane_id == pane);
        if let Some(a) = seat {
            if !renamed && a.name.as_deref() != Some(name) {
                c.agent_rename(pane, name)?;
            }
            renamed = true;
            if matches!(a.agent_status.as_str(), "idle" | "done") && !a.launch_pending {
                return Ok(note);
            }
        }
        let screen = c.pane_read(pane, "recent_unwrapped", Some(20))?.text;
        if note.is_none() && screen.to_lowercase().contains("trust this folder") {
            if !names_repo(&screen, repo) {
                return Err(format!(
                    "up: {name}: a folder-trust dialog is on screen but it does not show {repo}; sent no keys; screen: {}; answer it in pane {pane} yourself",
                    screen_tail(c, pane)
                ));
            }
            c.agent_send_keys(pane, &["down".to_owned(), "enter".to_owned()])?;
            note = Some(format!(
                "{name}: answered the folder-trust dialog for {repo}: down, enter"
            ));
        }
        if Instant::now() >= end {
            let (st, lp) = seat.map_or(("absent", false), |a| {
                (a.agent_status.as_str(), a.launch_pending)
            });
            return Err(format!(
                "up: {name} is not at a prompt after {}s: status={st} launch_pending={lp}; screen: {}; answer it in pane {pane} or start the seat by hand",
                prompt_budget().as_secs(),
                screen_tail(c, pane)
            ));
        }
        sleep(Duration::from_secs(1));
    }
}

pub fn down(c: &Client, names: &[String], force: bool) -> Result<String, Failure> {
    let live = c.agents()?;
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let forced = guard(&live, "stop", &refs, force)?;
    let mut stopped = Vec::new();
    for n in names {
        if let Some(a) = live.iter().find(|a| a.name.as_deref() == Some(n)) {
            c.pane_close(&a.pane_id)?;
            stopped.push(format!("{n} pane {} (was {})", a.pane_id, a.agent_status));
        }
    }
    let mut removed = Vec::new();
    for n in names {
        match store::unregister(n) {
            Ok(_) => removed.push(n.as_str()),
            Err(e) if e.starts_with("no registered session") => {}
            Err(e) => return Err(e.into()),
        }
    }
    if stopped.is_empty() && removed.is_empty() {
        return Err(format!(
            "down: no live agent or board row named {}",
            names.join(", ")
        )
        .into());
    }
    let mut out = format!(
        "stopped {}; unregistered {}",
        or_none(&stopped.join(", ")),
        or_none(&removed.join(", "))
    );
    if !forced.is_empty() {
        out.push_str(&format!("; forced: {}", forced.join(", ")));
    }
    Ok(out)
}

fn or_none(s: &str) -> &str {
    if s.is_empty() { "nothing" } else { s }
}

/// The first evidence of an input box on `screen`: a line that, once box-drawing
/// borders are stripped, starts with a `>`-style cursor, or a `PROMPT_MARKERS` text.
fn prompt_marker(screen: &str) -> Option<String> {
    for line in screen.lines() {
        let bare = line.trim_matches(|c: char| c.is_whitespace() || "│┃╭╮╰╯─┌┐└┘".contains(c));
        if let Some(c) = bare.chars().next().filter(|c| matches!(c, '>' | '❯' | '›')) {
            return Some(c.to_string());
        }
        if let Some(m) = PROMPT_MARKERS.iter().find(|m| line.contains(**m)) {
            return Some((*m).to_owned());
        }
    }
    None
}

/// Clears each named agent's context and confirms it: the agent reaches idle AND its
/// screen shows a prompt marker, else renew refuses with the screen. No `--force`:
/// a working pane is never cleared.
pub fn renew(c: &Client, names: &[String]) -> Result<String, Failure> {
    let live = c.agents()?;
    let mut panes = Vec::new();
    for n in names {
        let a = live
            .iter()
            .find(|a| a.name.as_deref() == Some(n))
            .ok_or_else(|| format!("renew: no live agent named {n}"))?;
        panes.push(a.pane_id.clone());
    }
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    guard(&live, "clear", &refs, false)?;
    for p in &panes {
        c.pane_send_text(p, "/clear")?;
        c.pane_send_keys(p, &["enter"])?;
    }
    sleep(Duration::from_secs(1));
    let mut lines = Vec::new();
    for (n, p) in names.iter().zip(&panes) {
        let until = ["idle".to_owned(), "done".to_owned()];
        if let Err(e) = c.agent_wait(n, &until, Some(PROMPT_TIMEOUT_MS)) {
            let seat = c.agents()?.into_iter().find(|a| a.pane_id == *p);
            let why = match seat {
                Some(a) if a.launch_pending => {
                    format!(
                        "still launching (status={}, launch_pending)",
                        a.agent_status
                    )
                }
                Some(a) if a.agent_status == "blocked" => {
                    "blocked on a prompt of its own".to_owned()
                }
                Some(a) => format!("wedged (status={})", a.agent_status),
                None => "gone".to_owned(),
            };
            return Err(format!(
                "renew: {n} did not come back to a prompt: {why}; screen: {}; {e}",
                screen_tail(c, p)
            )
            .into());
        }
        let screen = c.pane_read(p, "visible", Some(40))?.text;
        let Some(marker) = prompt_marker(&screen) else {
            return Err(format!(
                "renew: {n} is idle after /clear but no prompt marker is on its screen, so the cleared context is not confirmed; screen: {}",
                screen_tail(c, p)
            )
            .into());
        };
        lines.push(format!(
            "renewed {n} pane {p}: idle, prompt marker {marker:?} on screen"
        ));
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seat_argv_selects_codex_role_and_refuses_unknown_kinds() {
        for (role, profile) in [
            ("coding", "coder"),
            ("coder", "coder"),
            ("reviewer", "reviewer"),
            ("analyzer", "analyzer"),
            ("manager", "manager"),
        ] {
            assert_eq!(seat_args(role, "codex").unwrap(), ["-p", profile]);
            assert_eq!(seat_args(role, "opencode").unwrap(), ["--standalone"]);
        }
        for kind in ["gemini", "unknown-kind", ""] {
            let error = seat_args("reviewer", kind).unwrap_err();
            assert!(error.contains(&format!("{kind:?}")), "{error}");
        }
    }

    #[test]
    fn a_path_is_found_only_as_a_whole_component() {
        assert!(names_repo("trust /tmp/repo ?", "/tmp/repo"));
        assert!(names_repo("/tmp/repo", "/tmp/repo"));
        assert!(!names_repo("trust /tmp/repo2 ?", "/tmp/repo"));
        assert!(!names_repo("trust /tmp/repo-x", "/tmp/repo"));
        assert!(!names_repo("trust /other", "/tmp/repo"));
        assert!(!names_repo("trust /home/u/tmp/repo ?", "/tmp/repo"));
        assert!(!names_repo("/x/tmp/repo", "/tmp/repo"));
        // the live round-4 shape: the path follows "for " and sits at a word start
        let live = "/tmp/agentboard-live.nrnQOu/repo";
        assert!(names_repo(&format!("answered for {live}: down"), live));
        assert!(names_repo(&format!("│ {live} │"), live));
        assert!(names_repo(&format!("\"{live}\""), live));
    }

    #[test]
    fn prompt_markers_ignore_borders_and_plain_text() {
        assert_eq!(prompt_marker("hi\n│ > \n").as_deref(), Some(">"));
        assert_eq!(prompt_marker("❯ ").as_deref(), Some("❯"));
        assert_eq!(
            prompt_marker("Ask anything...").as_deref(),
            Some("Ask anything")
        );
        assert_eq!(prompt_marker("done\nauto mode on\n"), None);
    }
}
