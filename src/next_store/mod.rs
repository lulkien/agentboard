//! LMDB store under construction. The current binary still uses its legacy
//! `src/store` module until the CLI and MCP adapters cut over together.
//! Heed permits one open environment per path in a process; share a `Board`
//! handle when a process has multiple callers.

pub mod keys;
mod open;
mod tasks;

pub use open::{Board, BoardOptions, Error, Metadata, ResetOutcome, Result};
pub use tasks::{Added, Agent, UpdateInput, Updated};

#[cfg(test)]
mod tests;
