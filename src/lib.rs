//! Shared task board for a team of CLI agents: domain rules, an embedded LMDB
//! store, a CLI and an MCP server on one board.

pub mod cli;
pub mod domain;
pub mod render;
pub mod server;

pub mod store;
