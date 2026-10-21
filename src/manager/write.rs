//! The methods that change the live herdr session, and the one guard every verb
//! that stops or renames an agent goes through. Keeping both here means a new
//! verb cannot reach `pane.close` without passing the working check.

use super::Failure;
use super::api::Agent;
use super::client::Client;
use serde_json::{Map, Value, json};
use std::time::Duration;

/// herdr statuses that mean an agent is mid-turn; stopping one loses work.
const BUSY: [&str; 2] = ["working", "blocked"];

/// Looks the targets up in a fresh `agent.list` and refuses while any is busy.
/// A target is a live name or a pane id; one that matches no agent (a plain shell
/// pane) is not guarded. `force` overrides the refusal and the returned notes
/// (`coder was working`) say what it overrode, so the caller can print "forced".
pub fn guard(
    live: &[Agent],
    verb: &str,
    targets: &[&str],
    force: bool,
) -> Result<Vec<String>, Failure> {
    let mut forced = Vec::new();
    for t in targets {
        let Some(a) = live
            .iter()
            .find(|a| a.name.as_deref() == Some(t) || a.pane_id == *t)
        else {
            continue;
        };
        if !BUSY.contains(&a.agent_status.as_str()) {
            continue;
        }
        let who = a.name.as_deref().unwrap_or(&a.pane_id);
        if !force {
            return Err(Failure::Refused(format!(
                "refusing to {verb} {who}: status={}; wait for it to settle or pass --force",
                a.agent_status
            )));
        }
        forced.push(format!("{who} was {}", a.agent_status));
    }
    Ok(forced)
}

fn str_at(v: &Value, method: &str, path: &[&str]) -> Result<String, String> {
    path.iter()
        .try_fold(v, |v, k| v.get(k))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            format!(
                "herdr: {method}: unexpected reply shape: no {}",
                path.join(".")
            )
        })
}

fn env_map(env: &[(String, String)]) -> Value {
    Value::Object(
        env.iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect::<Map<_, _>>(),
    )
}

/// The ids a new tab hands back.
pub struct NewTab {
    pub tab_id: String,
    pub root_pane: String,
}

impl Client {
    pub fn tab_create(
        &self,
        cwd: &str,
        label: Option<&str>,
        env: &[(String, String)],
    ) -> Result<NewTab, String> {
        let mut p = json!({"cwd": cwd});
        if let Some(l) = label {
            p["label"] = json!(l);
        }
        if !env.is_empty() {
            p["env"] = env_map(env);
        }
        let r = self.call("tab.create", p)?;
        Ok(NewTab {
            tab_id: str_at(&r, "tab.create", &["tab", "tab_id"])?,
            root_pane: str_at(&r, "tab.create", &["root_pane", "pane_id"])?,
        })
    }

    /// Returns the new pane id.
    pub fn pane_split(
        &self,
        target: &str,
        direction: &str,
        cwd: Option<&str>,
        env: &[(String, String)],
    ) -> Result<String, String> {
        let mut p = json!({"direction": direction, "target_pane_id": target});
        if let Some(c) = cwd {
            p["cwd"] = json!(c);
        }
        if !env.is_empty() {
            p["env"] = env_map(env);
        }
        let r = self.call("pane.split", p)?;
        str_at(&r, "pane.split", &["pane", "pane_id"])
    }

    pub fn pane_close(&self, pane_id: &str) -> Result<(), String> {
        self.call("pane.close", json!({"pane_id": pane_id}))
            .map(drop)
    }

    /// `args` reach herdr verbatim; an empty list is left out of the request.
    pub fn agent_start(
        &self,
        name: &str,
        kind: &str,
        pane_id: &str,
        timeout_ms: Option<u64>,
        args: &[String],
    ) -> Result<(), String> {
        let mut p = json!({"name": name, "kind": kind, "pane_id": pane_id});
        if !args.is_empty() {
            p["args"] = json!(args);
        }
        if let Some(t) = timeout_ms {
            p["timeout_ms"] = json!(t);
        }
        let wait = Duration::from_millis(timeout_ms.unwrap_or(0));
        self.call_for("agent.start", p, wait).map(drop)
    }

    pub fn agent_prompt(&self, target: &str, text: &str) -> Result<(), String> {
        self.call("agent.prompt", json!({"target": target, "text": text}))
            .map(drop)
    }

    pub fn agent_send_keys(&self, target: &str, keys: &[String]) -> Result<(), String> {
        self.call("agent.send_keys", json!({"target": target, "keys": keys}))
            .map(drop)
    }

    pub fn agent_rename(&self, target: &str, name: &str) -> Result<(), String> {
        self.call("agent.rename", json!({"target": target, "name": name}))
            .map(drop)
    }

    pub fn agent_focus(&self, target: &str) -> Result<(), String> {
        self.call("agent.focus", json!({"target": target}))
            .map(drop)
    }

    /// Blocks until the agent reaches one of `until` (any status when empty) or
    /// `timeout_ms` passes; the reply is the agent's record at that moment.
    pub fn agent_wait(
        &self,
        target: &str,
        until: &[String],
        timeout_ms: Option<u64>,
    ) -> Result<Value, String> {
        let mut p = json!({"target": target});
        if !until.is_empty() {
            p["until"] = json!(until);
        }
        if let Some(t) = timeout_ms {
            p["timeout_ms"] = json!(t);
        }
        let wait = Duration::from_millis(timeout_ms.unwrap_or(30_000));
        self.call_for("agent.wait", p, wait)
    }

    pub fn pane_send_text(&self, pane_id: &str, text: &str) -> Result<(), String> {
        self.call("pane.send_text", json!({"pane_id": pane_id, "text": text}))
            .map(drop)
    }

    pub fn pane_send_keys(&self, pane_id: &str, keys: &[&str]) -> Result<(), String> {
        self.call("pane.send_keys", json!({"pane_id": pane_id, "keys": keys}))
            .map(drop)
    }
}

/// Replace only the named tab, through the same busy-agent guard as down/renew.
pub fn layout_apply(c: &Client, tab: &str, source: &str, force: bool) -> Result<String, Failure> {
    if tab.trim().is_empty() {
        return Err("layout apply requires a nonempty --tab".into());
    }
    let text = if source == "-" {
        std::io::read_to_string(std::io::stdin()).map_err(|e| e.to_string())?
    } else if std::path::Path::new(source).is_file() {
        std::fs::read_to_string(source).map_err(|e| e.to_string())?
    } else {
        source.to_owned()
    };
    let root: Box<serde_json::value::RawValue> =
        serde_json::from_str(&text).map_err(|e| format!("layout apply: invalid JSON: {e}"))?;
    let value: Value = serde_json::from_str(root.get()).map_err(|e| e.to_string())?;
    if !valid_tree(&value) {
        return Err("layout apply: malformed or empty tree".into());
    }
    // The transport is NDJSON: literal line breaks in pretty input must be
    // compacted. Single-line JSON (including export --json) stays byte-for-byte.
    let root = if root.get().contains(['\n', '\r']) {
        serde_json::value::RawValue::from_string(value.to_string()).map_err(|e| e.to_string())?
    } else {
        root
    };
    let live = c.agents()?;
    let targets: Vec<&str> = live
        .iter()
        .filter(|a| a.tab_id == tab)
        .map(|a| a.pane_id.as_str())
        .collect();
    let forced = guard(&live, "replace layout of", &targets, force)?;
    // Requiring an existing tab prevents herdr's apply fallback from creating one.
    if !c.tabs()?.iter().any(|t| t.tab_id == tab) {
        return Err(format!("layout apply: unknown tab {tab:?}").into());
    }
    #[derive(serde::Serialize)]
    struct Params<'a> {
        tab_id: &'a str,
        root: &'a serde_json::value::RawValue,
    }
    c.call(
        "layout.apply",
        Params {
            tab_id: tab,
            root: &root,
        },
    )?;
    Ok(super::forced_note(
        format!("applied layout to {tab}"),
        &forced,
    ))
}

fn valid_tree(v: &Value) -> bool {
    match v.get("type").and_then(Value::as_str) {
        Some("pane") => {
            ["pane_id", "cwd", "label"]
                .iter()
                .all(|k| v.get(k).is_none_or(|x| x.is_null() || x.is_string()))
                && v.get("command").is_none_or(|x| {
                    x.is_null() || x.as_array().is_some_and(|a| a.iter().all(Value::is_string))
                })
                && v.get("env").is_none_or(|x| {
                    x.as_object()
                        .is_some_and(|o| o.values().all(Value::is_string))
                })
        }
        Some("split") => {
            matches!(v["direction"].as_str(), Some("right" | "down"))
                && v["ratio"].as_f64().is_some_and(|r| r > 0.0 && r < 1.0)
                && valid_tree(&v["first"])
                && valid_tree(&v["second"])
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live(name: Option<&str>, pane: &str, status: &str) -> Agent {
        Agent {
            name: name.map(str::to_owned),
            agent: Some("claude".to_owned()),
            agent_status: status.to_owned(),
            workspace_id: "w1".to_owned(),
            tab_id: "w1:t1".to_owned(),
            pane_id: pane.to_owned(),
            cwd: None,
            foreground_cwd: None,
            focused: false,
            launch_pending: false,
        }
    }

    fn refusal(r: Result<Vec<String>, Failure>) -> String {
        match r {
            Err(Failure::Refused(m)) => m,
            other => panic!("wanted a refusal, got {other:?}"),
        }
    }

    #[test]
    fn working_and_blocked_are_refused_by_name_or_pane() {
        let agents = [
            live(Some("coder"), "w1:p1", "working"),
            live(Some("reviewer"), "w1:p2", "blocked"),
        ];
        let m = refusal(guard(&agents, "stop", &["coder"], false));
        assert!(
            m.starts_with("refusing to stop coder: status=working"),
            "{m}"
        );
        let m = refusal(guard(&agents, "clear", &["w1:p2"], false));
        assert!(
            m.starts_with("refusing to clear reviewer: status=blocked"),
            "{m}"
        );
    }

    #[test]
    fn idle_done_and_unknown_targets_pass_and_force_reports_what_it_overrode() {
        let agents = [
            live(Some("coder"), "w1:p1", "idle"),
            live(Some("reviewer"), "w1:p2", "working"),
        ];
        assert!(
            guard(&agents, "stop", &["coder", "shell-pane"], false)
                .unwrap()
                .is_empty()
        );
        let notes = guard(&agents, "stop", &["coder", "reviewer"], true).unwrap();
        assert_eq!(notes, ["reviewer was working"]);
    }
}
