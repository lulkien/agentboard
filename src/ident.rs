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
    line_for(tools, COMMIT, checkout_stamp().as_deref())
}

/// Store identity without a dependency on the MCP adapter. The build derives the
/// count from tool attributes; the server test compares it to the actual router.
/// Pure formatting: store records must never consult git or perform I/O.
pub fn stamp() -> String {
    let tools = surface_tools();
    format!("{NAME} {VERSION} tools={tools} commit={COMMIT} backend={BACKEND}")
}

/// Compiled from the router; no argument or environment variable can override it.
pub fn surface_tools() -> i64 {
    env!("AGENTBOARD_TOOL_COUNT")
        .parse()
        .expect("derived tool count")
}

pub const SURFACE_REMEDY: &str = "reinstall the seat binary, then reload the seat's MCP server";

pub fn surface_refusal(client: Option<i64>, board: Option<i64>) -> String {
    format!(
        "board surface mismatch: seat tools={}; board tools={}; {SURFACE_REMEDY}",
        client.map_or_else(|| "none".to_owned(), |n| n.to_string()),
        board.map_or_else(|| "none".to_owned(), |n| n.to_string())
    )
}

/// Bump for incompatible schema OR tool-surface rules. Never auto-upgrade a board.
pub const GENERATION: i64 = 2;

// Resolve the build checkout, never the caller's unrelated working directory.
// Cargo rerun hints cannot update an artifact that is already installed/running;
// HEAD may also be a symbolic ref whose contents stay unchanged across commits.
fn checkout_stamp() -> Option<String> {
    let output = crate::store::proc::run(
        std::process::Command::new("git")
            .args(["describe", "--always", "--dirty", "--abbrev=8"])
            .current_dir(env!("CARGO_MANIFEST_DIR")),
    )
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn line_for(tools: usize, commit: &str, checkout: Option<&str>) -> String {
    let freshness = match checkout {
        Some(head) if head != commit => format!(" (stale, HEAD is {head})"),
        Some(_) => String::new(),
        None => " (unverified, build checkout unavailable)".to_owned(),
    };
    format!("{NAME} {VERSION} tools={tools} commit={commit}{freshness} backend={BACKEND}")
}

#[cfg(test)]
mod tests {
    use super::line_for;

    #[test]
    fn stale_and_unverifiable_builds_are_explicit() {
        // Isolate PATH in a child test process; never mutate the test runner's env.
        if std::env::var_os("AGENTBOARD_TEST_STAMP_CHILD").is_some() {
            for _ in 0..20 {
                assert!(!super::stamp().contains("unverified"));
            }
            return;
        }
        let dir = std::env::temp_dir().join(format!("agentboard-stamp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let git = dir.join("git");
        std::fs::write(
            &git,
            "#!/bin/sh\necho called >> \"$AGENTBOARD_TEST_GIT_COUNTER\"\nexit 1\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o700)).unwrap();
        let counter = dir.join("calls");
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "ident::tests::stale_and_unverifiable_builds_are_explicit",
            ])
            .env("AGENTBOARD_TEST_STAMP_CHILD", "1")
            .env("AGENTBOARD_TEST_GIT_COUNTER", &counter)
            .env("PATH", &dir)
            .status()
            .unwrap();
        assert!(status.success());
        assert!(!counter.exists(), "repeated stamp() must not spawn git");
        std::fs::remove_dir_all(dir).unwrap();

        assert!(
            line_for(23, "aaaaaaaa", Some("bbbbbbbb"))
                .contains("commit=aaaaaaaa (stale, HEAD is bbbbbbbb)")
        );
        assert!(
            line_for(23, "aaaaaaaa-dirty", Some("aaaaaaaa")).contains("(stale, HEAD is aaaaaaaa)")
        );
        assert!(!line_for(23, "aaaaaaaa", Some("aaaaaaaa")).contains("stale"));
        assert!(line_for(23, "aaaaaaaa", None).contains("(unverified,"));
    }
}
