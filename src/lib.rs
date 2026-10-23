//! Domain rules for the next agentboard store. The current CLI/MCP binary keeps
//! using its existing backend until the adapter cutover is ready.

pub mod domain;

#[path = "next_store/mod.rs"]
pub mod store;
