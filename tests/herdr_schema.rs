//! Read-only live contract check; the Python helper derives requests from Rust
//! source and uses only ping and `herdr api schema --output`.
use std::process::Command;

fn check(mode: Option<&str>) {
    let mut command = Command::new("python3");
    command
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .arg("tests/herdr_schema.py");
    if let Some(mode) = mode {
        command.arg(mode);
    }
    let output = command.output().expect("run schema guard with python3");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    print!("{stdout}");
    eprint!("{stderr}");
    assert!(
        output.status.success(),
        "herdr schema guard: {stdout}{stderr}"
    );
}

#[test]
fn source_inventory_and_schema_comparison() {
    check(Some("--self-test"));
}

#[test]
fn live_schema_matches_manager_requests() {
    check(None);
}
