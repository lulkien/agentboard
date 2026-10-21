//! Which build this is, in one line: crate version, tool count, commit, backend.
//!
//! One string, produced by one function, printed by the CLI `board-build` verb, the
//! MCP `board_build` tool, and the MCP `initialize` reply. A pre-gate binary and HEAD
//! then differ where a client, a seat and a log can all see it, without a second call
//! and without trusting either build's claim about itself.
//!
//! The tool count is passed in by whoever owns the tool registry
//! (`server::tool_count`): it is derived from the router the client is served, never
//! written down here.

pub const NAME: &str = "agentboard";
/// The crate version, so the line agrees with `Cargo.toml` and `--version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// `git describe --always --dirty`, injected by `build.rs`; `unknown` off a checkout.
pub const COMMIT: &str = env!("AGENTBOARD_COMMIT");

/// The backend belongs in the identity: one commit built for sqlite and for surreal
/// serves two different boards.
pub const BACKEND: &str = if cfg!(feature = "sqlite") {
    "sqlite"
} else {
    "surreal"
};

/// `agentboard 0.1.0 tools=23 commit=1a2b3c4d-dirty backend=surreal`
pub fn line(tools: usize) -> String {
    format!("{NAME} {VERSION} tools={tools} commit={COMMIT} backend={BACKEND}")
}
