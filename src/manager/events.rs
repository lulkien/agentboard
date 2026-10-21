//! Waking on an agent transition. herdr's `events.wait` takes one pane and one
//! status; `events.subscribe` takes many panes and leaves the status to the caller,
//! so this uses `subscribe`, filters client-side, and drops the connection on a
//! match, a timeout and a cancel. Only transitions after the subscription count: a
//! pane already in a wanted status does not wake it (`manager agent wait` is the
//! "has it reached" check).

use super::client::Client;
use serde_json::{Value, json};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

const STATUSES: &str = "idle|working|blocked|done|unknown";

pub struct Hit {
    pub agent: String,
    pub status: String,
    pub waited: Duration,
}

/// `idle,working` (or repeated flags) into canonical statuses; a typo is an error,
/// never a wait that cannot match.
pub fn parse_until(raw: &[String]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for s in raw.iter().flat_map(|r| r.split(',')).map(str::trim) {
        if s.is_empty() {
            continue;
        }
        if !STATUSES.split('|').any(|k| k == s) {
            return Err(format!("wait: bad agent status {s:?} ({STATUSES})"));
        }
        out.push(s.to_owned());
    }
    if out.is_empty() {
        return Err(format!("wait: empty --until list ({STATUSES})"));
    }
    Ok(out)
}

/// Blocks until one of `names` (every live agent when empty) moves to a status in
/// `until`. `Ok(None)`: `wait` ran out, or `cancel` was set. Returns the agents
/// watched alongside, so a timeout can name them.
pub fn watch(
    c: &Client,
    names: &[String],
    until: &[String],
    wait: Duration,
    cancel: &AtomicBool,
) -> Result<(Vec<String>, Option<Hit>), String> {
    let live = c.agents()?;
    let seats: Vec<(String, String)> = if names.is_empty() {
        live.iter()
            .map(|a| {
                (
                    a.name.clone().unwrap_or_else(|| a.pane_id.clone()),
                    a.pane_id.clone(),
                )
            })
            .collect()
    } else {
        names
            .iter()
            .map(|n| {
                live.iter()
                    .find(|a| a.name.as_deref() == Some(n) || a.pane_id == *n)
                    .map(|a| (n.clone(), a.pane_id.clone()))
                    .ok_or_else(|| format!("wait: no live agent {n:?}"))
            })
            .collect::<Result<_, _>>()?
    };
    if seats.is_empty() {
        return Err("wait: no live agents to watch".to_owned());
    }
    let subs: Vec<Value> = seats
        .iter()
        .map(|(_, p)| json!({"type": "pane.agent_status_changed", "pane_id": p}))
        .collect();
    let start = Instant::now();
    let hit = c.subscribe(json!({"subscriptions": subs}), wait, cancel, |ev| {
        if ev.get("event").and_then(Value::as_str) != Some("pane.agent_status_changed") {
            return None;
        }
        let data = ev.get("data")?;
        let pane = data.get("pane_id")?.as_str()?;
        let status = data.get("agent_status")?.as_str()?;
        let (name, _) = seats.iter().find(|(_, p)| p == pane)?;
        until.iter().any(|u| u == status).then(|| Hit {
            agent: name.clone(),
            status: status.to_owned(),
            waited: start.elapsed(),
        })
    })?;
    Ok((seats.into_iter().map(|(n, _)| n).collect(), hit))
}

fn secs(d: Duration) -> String {
    format!("{:.1}s", d.as_secs_f64())
}

/// The reply lines, in `agentboard wait`'s `matched:` / `timeout:` shape; an agent
/// hit says `agent` so a combined wait shows which side fired.
pub fn hit_line(h: &Hit) -> String {
    format!(
        "matched: agent {} -> {} after {}",
        h.agent,
        h.status,
        secs(h.waited)
    )
}

pub fn timeout_line(wait: Duration, watched: &[String]) -> String {
    format!(
        "timeout: no agent transition after {}s (watching {})",
        wait.as_secs(),
        watched.join(", ")
    )
}

pub fn hit_json(h: &Hit) -> Value {
    json!({"matched": true, "agent": h.agent, "status": h.status, "waited_ms": h.waited.as_millis() as u64})
}

pub fn timeout_json(wait: Duration, watched: &[String]) -> Value {
    json!({"matched": false, "timeout_s": wait.as_secs(), "watched": watched})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn until_takes_csv_and_repeats_and_refuses_typos() {
        assert_eq!(
            parse_until(&v(&["idle,done", "working"])).unwrap(),
            v(&["idle", "done", "working"])
        );
        assert!(
            parse_until(&v(&["idel"]))
                .unwrap_err()
                .contains("bad agent status \"idel\"")
        );
        assert!(
            parse_until(&v(&[" , "]))
                .unwrap_err()
                .contains("empty --until")
        );
    }
}
