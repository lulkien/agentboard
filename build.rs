//! Injects this binary's build stamp: the commit it was compiled from.
//!
//! `git describe --always --dirty --abbrev=8` in the crate directory, or `unknown`
//! when there is no checkout to ask (a release tarball, a vendored build). The value
//! is emitted as `AGENTBOARD_COMMIT` and read with `env!` in `src/ident.rs`, so two
//! builds of the same crate version stay distinguishable in informational MCP
//! `initialize` and `board-build` output. This is a compile-time label, with no
//! runtime checkout check or compatibility decision. `AGENTBOARD_BUILD_COMMIT`
//! overrides it when a packager knows the revision and git does not.
use std::process::Command;

fn main() {
    println!("cargo:rustc-env=AGENTBOARD_COMMIT={}", describe());
    // A changed HEAD file can refresh the compile-time label on a later build.
    // This hint does not detect every branch-ref move or update installed builds.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-env-changed=AGENTBOARD_BUILD_COMMIT");
}

fn describe() -> String {
    if let Ok(given) = std::env::var("AGENTBOARD_BUILD_COMMIT") {
        let given = given.trim();
        if !given.is_empty() {
            return given.to_owned();
        }
    }
    let out = Command::new("git")
        .args(["describe", "--always", "--dirty", "--abbrev=8"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let stamp = String::from_utf8_lossy(&o.stdout).trim().to_owned();
            if stamp.is_empty() {
                "unknown".to_owned()
            } else {
                stamp
            }
        }
        _ => "unknown".to_owned(),
    }
}
