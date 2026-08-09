//! End-to-end tests for `cdz chor FILE` — the choreography-projection sidecar (project a global protocol
//! into one self-contained program per actor).
//!
//! The full projection is heavyweight + environment-dependent (it needs the `implementation/choreography`
//! package src dir discoverable and runs the RUST compiler on a generated driver), and its core logic is
//! already unit-tested inline in `main.rs` (`chor_module_is_rec_degraded`, the rec/var stubs, etc.). What is
//! pinned HERE is the ENVIRONMENT-INDEPENDENT CLI layer of `cdz chor` — argument parsing and the read-error
//! contract — which had no e2e coverage at all (this file owns that region after adopting the render-
//! compilable-marker comment fix, PR #1198). These run anywhere (no store, no choreography package, no rust
//! toolchain needed), so they hold in CI's bare `test` job as well as the full gate.

use std::process::Command;

/// Run `cdz <args…>`, returning (exit_ok, stdout, stderr).
fn run(args: &[&str]) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let out = Command::new(exe).args(args).output().expect("spawn cdz");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

#[test]
fn chor_on_an_unreadable_file_errors_cleanly_naming_the_command() {
    // A nonexistent protocol source → a clean `cdz chor: cannot read <path>` on stderr + non-zero exit, NOT a
    // panic or an opaque failure. This is the read-error contract of `run_chor`'s first step, independent of
    // whether the choreography package / rust toolchain is present, so it pins the CLI-layer behavior a script
    // relies on. Use an absolute path that cannot exist.
    let (ok, _out, err) = run(&["chor", "/no/such/choreography/protocol.cdz"]);
    assert!(!ok, "an unreadable protocol file must fail (non-zero exit)");
    assert!(
        err.contains("cdz chor: cannot read") && err.contains("protocol.cdz"),
        "the read error names the command + the unreadable path: {err}"
    );
}

#[test]
fn chor_with_no_file_argument_is_a_usage_error() {
    // `<FILE>` is required — omitting it is a clap usage error (exit 2), distinct from a runtime failure
    // (exit 1). Pins that the subcommand declares its required positional so a bare `cdz chor` guides the user.
    let exe = env!("CARGO_BIN_EXE_cdz");
    let out = Command::new(exe).arg("chor").output().expect("spawn cdz");
    let code = out.status.code();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        code,
        Some(2),
        "a missing required <FILE> is a clap usage error (exit 2): stderr={err}"
    );
    assert!(
        err.contains("<FILE>") || err.contains("required"),
        "the usage error names the missing FILE argument: {err}"
    );
}
