//! Typed requests and replies for the herdr methods the read-only verbs use.
//! Only the fields a verb prints are modelled; the rest of a reply is ignored.

use super::client::Client;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

#[derive(Debug, Clone, Deserialize)]
pub struct Agent {
    pub name: Option<String>,
    pub agent: Option<String>,
    pub agent_status: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub cwd: Option<String>,
    pub foreground_cwd: Option<String>,
    pub focused: bool,
    /// herdr sets this while a freshly started agent has not yet reached its prompt.
    #[serde(default)]
    pub launch_pending: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Tab {
    pub tab_id: String,
    pub workspace_id: String,
    pub number: i64,
    pub label: String,
    pub focused: bool,
    pub pane_count: i64,
    pub agent_status: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Pane {
    pub pane_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub label: Option<String>,
    pub agent: Option<String>,
    pub agent_status: String,
    pub cwd: Option<String>,
    pub foreground_cwd: Option<String>,
    pub focused: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Read {
    pub text: String,
}

/// Pulls `result[key]` out of a reply as `T`.
fn take<T: DeserializeOwned>(method: &str, mut result: Value, key: &str) -> Result<T, String> {
    let part = result.get_mut(key).map(Value::take).unwrap_or(Value::Null);
    serde_json::from_value(part).map_err(|e| {
        let why = e
            .to_string()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        format!("herdr: {method}: unexpected reply shape: {why}")
    })
}

impl Client {
    /// The whole session as herdr reports it, left raw on purpose.
    pub fn snapshot(&self) -> Result<Value, String> {
        take(
            "session.snapshot",
            self.call("session.snapshot", json!({}))?,
            "snapshot",
        )
    }

    pub fn agents(&self) -> Result<Vec<Agent>, String> {
        take("agent.list", self.call("agent.list", json!({}))?, "agents")
    }

    pub fn agent(&self, target: &str) -> Result<Agent, String> {
        take(
            "agent.get",
            self.call("agent.get", json!({"target": target}))?,
            "agent",
        )
    }

    pub fn tabs(&self) -> Result<Vec<Tab>, String> {
        take("tab.list", self.call("tab.list", json!({}))?, "tabs")
    }

    pub fn panes(&self) -> Result<Vec<Pane>, String> {
        take("pane.list", self.call("pane.list", json!({}))?, "panes")
    }

    pub fn pane(&self, pane_id: &str) -> Result<Pane, String> {
        take(
            "pane.get",
            self.call("pane.get", json!({"pane_id": pane_id}))?,
            "pane",
        )
    }

    pub fn agent_read(
        &self,
        target: &str,
        source: &str,
        lines: Option<u32>,
    ) -> Result<Read, String> {
        let params = json!({"target": target, "source": source, "lines": lines});
        take("agent.read", self.call("agent.read", params)?, "read")
    }

    pub fn pane_read(
        &self,
        pane_id: &str,
        source: &str,
        lines: Option<u32>,
    ) -> Result<Read, String> {
        let params = json!({"pane_id": pane_id, "source": source, "lines": lines});
        take("pane.read", self.call("pane.read", params)?, "read")
    }
}
