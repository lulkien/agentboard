//! `agentboard manager`: herdr driven from the binary. `client` is the socket
//! (discovery, protocol pin, deadline), `api` the typed read methods, `write` the
//! methods that change the session plus the working guard, `team` the composite
//! verbs (`up`, `down`, `renew`). This file joins the board's session registry
//! against live herdr truth and renders the verbs' one-line results.

mod api;
mod client;
mod team;
mod write;

pub use team::{Up, down, renew, up};
use write::guard;

pub use api::Agent;
pub use client::Client;

use crate::store::text::{SessionRow, ago};
use crate::store::{self, short};
use serde::Serialize;
use serde_json::{Value, json};

/// Why a verb stopped. A refusal (a working agent, no `--force`) exits 3 so a
/// script can tell "not now" from "broken"; everything else exits 1.
#[derive(Debug)]
pub enum Failure {
    Refused(String),
    Other(String),
}

impl From<String> for Failure {
    fn from(s: String) -> Self {
        Failure::Other(s)
    }
}

impl From<&str> for Failure {
    fn from(s: &str) -> Self {
        Failure::Other(s.to_owned())
    }
}

/// One seat of a team: the board role it takes its MCP config from, the agent
/// kind to start, and the herdr name (role plus any prefix).
pub struct Seat<'a> {
    pub role: &'a str,
    pub kind: &'a str,
    pub name: String,
}

/// One line of `manager status`: a registry row, a live agent, or both.
#[derive(Debug, Serialize, PartialEq)]
pub struct StatusRow {
    pub name: String,
    pub kind: Option<String>,
    /// `registered` (row and live agent), `gone` (row, no live agent) or
    /// `unregistered` (live agent, no row).
    pub board: &'static str,
    /// herdr's agent status; none for a gone row.
    pub status: Option<String>,
    pub tab_id: Option<String>,
    pub pane_id: Option<String>,
    pub cwd: Option<String>,
    pub foreground_cwd: Option<String>,
    pub task: Option<String>,
    /// The registry row's own status (`up`); none for an unregistered agent.
    pub session_status: Option<String>,
    pub seen_ms: Option<i64>,
}

/// Registry rows keep their order; live agents no row names follow, unregistered.
pub fn join(rows: Vec<SessionRow>, agents: Vec<Agent>) -> Vec<StatusRow> {
    let mut out = Vec::new();
    let mut taken = vec![false; agents.len()];
    for r in rows {
        let live = agents
            .iter()
            .position(|a| a.name.as_deref() == Some(r.name.as_str()));
        let mut row = StatusRow {
            name: r.name,
            kind: Some(r.agent),
            board: "gone",
            status: None,
            tab_id: None,
            pane_id: None,
            cwd: r.cwd,
            foreground_cwd: None,
            task: r.task_id.map(short),
            session_status: Some(r.status),
            seen_ms: Some(r.seen),
        };
        if let Some(i) = live {
            taken[i] = true;
            let a = &agents[i];
            row.board = "registered";
            row.kind = a.agent.clone().or(row.kind);
            row.status = Some(a.agent_status.clone());
            row.tab_id = Some(a.tab_id.clone());
            row.pane_id = Some(a.pane_id.clone());
            row.cwd = a.cwd.clone().or(row.cwd);
            row.foreground_cwd = a.foreground_cwd.clone();
        }
        out.push(row);
    }
    for (a, _) in agents.into_iter().zip(taken).filter(|(_, t)| !t) {
        out.push(StatusRow {
            name: a.name.unwrap_or_else(|| a.pane_id.clone()),
            kind: a.agent,
            board: "unregistered",
            status: Some(a.agent_status),
            tab_id: Some(a.tab_id),
            pane_id: Some(a.pane_id),
            cwd: a.cwd,
            foreground_cwd: a.foreground_cwd,
            task: None,
            session_status: None,
            seen_ms: None,
        });
    }
    out
}

fn dash(v: &Option<String>) -> String {
    v.clone().unwrap_or_else(|| "-".to_owned())
}

/// Header plus rows, every column but the last padded to its widest cell.
fn table(head: &[&str], rows: Vec<Vec<String>>) -> String {
    if rows.is_empty() {
        return "(none)".to_owned();
    }
    let head: Vec<String> = head.iter().map(|h| (*h).to_owned()).collect();
    let all: Vec<&Vec<String>> = std::iter::once(&head).chain(rows.iter()).collect();
    let last = head.len() - 1;
    let widths: Vec<usize> = (0..last)
        .map(|c| all.iter().map(|r| r[c].chars().count()).max().unwrap_or(0))
        .collect();
    let lines: Vec<String> = all
        .iter()
        .map(|r| {
            let mut line = String::new();
            for (c, w) in widths.iter().enumerate() {
                line.push_str(&format!("{:<w$} ", r[c]));
            }
            line.push_str(&r[last]);
            line
        })
        .collect();
    lines.join("\n")
}

/// `manager status`: one line per agent, or the same data as one JSON object.
pub fn status(c: &Client, json: bool) -> Result<String, String> {
    let live = c.agents()?;
    let rows = join(store::session_rows()?, live);
    if json {
        let doc = json!({
            "herdr": {"version": c.version, "protocol": client::PROTOCOL},
            "agents": rows,
        });
        return serde_json::to_string_pretty(&doc).map_err(|e| e.to_string());
    }
    let cells = rows
        .iter()
        .map(|r| {
            let cwd = r.foreground_cwd.as_ref().or(r.cwd.as_ref()).cloned();
            vec![
                r.name.clone(),
                dash(&r.kind),
                r.board.to_owned(),
                dash(&r.status),
                dash(&r.tab_id),
                dash(&r.pane_id),
                dash(&r.task),
                r.seen_ms.map_or("-".to_owned(), ago),
                dash(&cwd),
            ]
        })
        .collect();
    Ok(table(
        &[
            "NAME", "KIND", "BOARD", "STATUS", "TAB", "PANE", "TASK", "SEEN", "CWD",
        ],
        cells,
    ))
}

/// `manager snapshot`: herdr's `session.snapshot`, raw with `--json`.
pub fn snapshot(c: &Client, json: bool) -> Result<String, String> {
    let snap = c.snapshot()?;
    if json {
        return serde_json::to_string_pretty(&snap).map_err(|e| e.to_string());
    }
    let count = |k: &str| snap.get(k).and_then(Value::as_array).map_or(0, Vec::len);
    let focus = |k: &str| {
        snap.get(k)
            .and_then(Value::as_str)
            .unwrap_or("-")
            .to_owned()
    };
    Ok(format!(
        "herdr {} protocol {}\nworkspaces {} tabs {} panes {} agents {}\nfocused workspace {} tab {} pane {}",
        c.version,
        client::PROTOCOL,
        count("workspaces"),
        count("tabs"),
        count("panes"),
        count("agents"),
        focus("focused_workspace_id"),
        focus("focused_tab_id"),
        focus("focused_pane_id"),
    ))
}

fn cwd_of(cwd: &Option<String>, fg: &Option<String>) -> String {
    dash(&fg.clone().or_else(|| cwd.clone()))
}

pub fn agent_list(c: &Client) -> Result<String, String> {
    let cells = c
        .agents()?
        .iter()
        .map(|a| {
            vec![
                a.name.clone().unwrap_or_else(|| a.pane_id.clone()),
                dash(&a.agent),
                a.agent_status.clone(),
                a.tab_id.clone(),
                a.pane_id.clone(),
                cwd_of(&a.cwd, &a.foreground_cwd),
            ]
        })
        .collect();
    Ok(table(
        &["NAME", "KIND", "STATUS", "TAB", "PANE", "CWD"],
        cells,
    ))
}

pub fn agent_get(c: &Client, target: &str) -> Result<String, String> {
    let a = c.agent(target)?;
    Ok(format!(
        "name: {}\nkind: {}\nstatus: {}\nworkspace: {}\ntab: {}\npane: {}\nfocused: {}\ncwd: {}\nforeground_cwd: {}",
        dash(&a.name),
        dash(&a.agent),
        a.agent_status,
        a.workspace_id,
        a.tab_id,
        a.pane_id,
        a.focused,
        dash(&a.cwd),
        dash(&a.foreground_cwd),
    ))
}

pub fn agent_read(
    c: &Client,
    target: &str,
    source: &str,
    lines: Option<u32>,
) -> Result<String, String> {
    Ok(c.agent_read(target, source, lines)?
        .text
        .trim_end()
        .to_owned())
}

pub fn tab_list(c: &Client) -> Result<String, String> {
    let cells = c
        .tabs()?
        .iter()
        .map(|t| {
            vec![
                t.tab_id.clone(),
                t.workspace_id.clone(),
                t.number.to_string(),
                t.agent_status.clone(),
                t.pane_count.to_string(),
                if t.focused { "focused" } else { "-" }.to_owned(),
                t.label.clone(),
            ]
        })
        .collect();
    Ok(table(
        &[
            "TAB",
            "WORKSPACE",
            "NUM",
            "STATUS",
            "PANES",
            "FOCUS",
            "LABEL",
        ],
        cells,
    ))
}

pub fn pane_list(c: &Client) -> Result<String, String> {
    let cells = c
        .panes()?
        .iter()
        .map(|p| {
            vec![
                p.pane_id.clone(),
                p.tab_id.clone(),
                dash(&p.agent),
                p.agent_status.clone(),
                if p.focused { "focused" } else { "-" }.to_owned(),
                cwd_of(&p.cwd, &p.foreground_cwd),
            ]
        })
        .collect();
    Ok(table(
        &["PANE", "TAB", "KIND", "STATUS", "FOCUS", "CWD"],
        cells,
    ))
}

pub fn pane_get(c: &Client, pane_id: &str) -> Result<String, String> {
    let p = c.pane(pane_id)?;
    Ok(format!(
        "pane: {}\nworkspace: {}\ntab: {}\nlabel: {}\nkind: {}\nstatus: {}\nfocused: {}\ncwd: {}\nforeground_cwd: {}",
        p.pane_id,
        p.workspace_id,
        p.tab_id,
        dash(&p.label),
        dash(&p.agent),
        p.agent_status,
        p.focused,
        dash(&p.cwd),
        dash(&p.foreground_cwd),
    ))
}

pub fn pane_read(
    c: &Client,
    pane_id: &str,
    source: &str,
    lines: Option<u32>,
) -> Result<String, String> {
    Ok(c.pane_read(pane_id, source, lines)?
        .text
        .trim_end()
        .to_owned())
}

pub fn tab_new(c: &Client, cwd: &str, label: Option<&str>) -> Result<String, String> {
    let t = c.tab_create(cwd, label, &[])?;
    Ok(format!("tab {} root pane {}", t.tab_id, t.root_pane))
}

/// `K=V` strings into pairs; the value may itself contain `=`.
fn parse_env(env: &[String]) -> Result<Vec<(String, String)>, String> {
    env.iter()
        .map(|kv| {
            kv.split_once('=')
                .filter(|(k, _)| !k.is_empty())
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .ok_or_else(|| format!("--env {kv:?}: want K=V"))
        })
        .collect()
}

pub fn pane_split(
    c: &Client,
    target: &str,
    direction: &str,
    cwd: Option<&str>,
    env: &[String],
) -> Result<String, String> {
    let env = parse_env(env)?;
    let pane = c.pane_split(target, direction, cwd, &env)?;
    Ok(format!("pane {pane} split {direction} of {target}"))
}

pub fn pane_close(c: &Client, pane_id: &str, force: bool) -> Result<String, Failure> {
    let forced = guard(&c.agents()?, "stop", &[pane_id], force)?;
    c.pane_close(pane_id)?;
    Ok(forced_note(format!("closed pane {pane_id}"), &forced))
}

pub fn agent_start(
    c: &Client,
    name: &str,
    kind: &str,
    pane: &str,
    timeout_ms: Option<u64>,
    args: &[String],
) -> Result<String, String> {
    c.agent_start(name, kind, pane, timeout_ms, args)?;
    Ok(format!("started {name} ({kind}) in {pane}"))
}

pub fn agent_prompt(c: &Client, target: &str, text: &str) -> Result<String, String> {
    c.agent_prompt(target, text)?;
    Ok(format!("prompted {target}"))
}

pub fn agent_keys(c: &Client, target: &str, keys: &[String]) -> Result<String, String> {
    c.agent_send_keys(target, keys)?;
    Ok(format!("sent {} to {target}", keys.join(" ")))
}

pub fn agent_rename(c: &Client, target: &str, name: &str, force: bool) -> Result<String, Failure> {
    let forced = guard(&c.agents()?, "rename", &[target], force)?;
    c.agent_rename(target, name)?;
    Ok(forced_note(format!("renamed {target} to {name}"), &forced))
}

pub fn agent_focus(c: &Client, target: &str) -> Result<String, String> {
    c.agent_focus(target)?;
    Ok(format!("focused {target}"))
}

pub fn agent_wait(
    c: &Client,
    target: &str,
    until: &[String],
    timeout_ms: Option<u64>,
) -> Result<String, String> {
    let r = c.agent_wait(target, until, timeout_ms)?;
    let status = r
        .pointer("/agent/agent_status")
        .and_then(Value::as_str)
        .unwrap_or("reached");
    Ok(format!("{target}: {status}"))
}

fn forced_note(done: String, forced: &[String]) -> String {
    if forced.is_empty() {
        done
    } else {
        format!("{done}; forced: {}", forced.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(name: &str, task: Option<i64>) -> SessionRow {
        SessionRow {
            name: name.to_owned(),
            agent: "claude".to_owned(),
            cwd: Some("/repo".to_owned()),
            task_id: task,
            seen: 1000,
            status: "up".to_owned(),
        }
    }

    fn live(name: Option<&str>, pane: &str, status: &str) -> Agent {
        Agent {
            name: name.map(str::to_owned),
            agent: Some("opencode".to_owned()),
            agent_status: status.to_owned(),
            workspace_id: "w1".to_owned(),
            tab_id: "w1:t1".to_owned(),
            pane_id: pane.to_owned(),
            cwd: Some("/live".to_owned()),
            foreground_cwd: None,
            focused: false,
            launch_pending: false,
        }
    }

    #[test]
    fn join_marks_registered_gone_and_unregistered() {
        let rows = join(
            vec![session("coder", Some(7)), session("old", None)],
            vec![
                live(Some("coder"), "w1:p1", "working"),
                live(Some("stray"), "w1:p2", "idle"),
            ],
        );
        let got: Vec<(&str, &str, Option<&str>)> = rows
            .iter()
            .map(|r| (r.name.as_str(), r.board, r.status.as_deref()))
            .collect();
        assert_eq!(
            got,
            [
                ("coder", "registered", Some("working")),
                ("old", "gone", None),
                ("stray", "unregistered", Some("idle")),
            ]
        );
        assert_eq!(rows[0].task.as_deref(), Some("T7"));
        assert_eq!(rows[0].pane_id.as_deref(), Some("w1:p1"));
        assert_eq!(rows[1].pane_id, None);
        assert_eq!(rows[2].task, None);
    }

    #[test]
    fn an_unnamed_live_agent_is_listed_by_pane_id() {
        let rows = join(vec![], vec![live(None, "w1:p9", "idle")]);
        assert_eq!(rows[0].name, "w1:p9");
        assert_eq!(rows[0].board, "unregistered");
    }

    #[test]
    fn table_pads_every_column_but_the_last() {
        let t = table(
            &["A", "B"],
            vec![vec!["long-cell".to_owned(), "x".to_owned()]],
        );
        assert_eq!(t, "A         B\nlong-cell x");
        assert_eq!(table(&["A"], vec![]), "(none)");
    }
}
