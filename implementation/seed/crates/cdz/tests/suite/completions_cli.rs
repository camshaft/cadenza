//! End-to-end tests for `cdz completions <shell>` — the shell-completion-script generator (cargo
//! parity). Generated from the clap command tree, so a completion script always matches the real
//! subcommands/flags. Drives the built binary.

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
fn completions_generate_for_each_supported_shell() {
    // Each supported shell produces a non-empty script and exits 0.
    for shell in ["bash", "zsh", "fish", "elvish", "powershell"] {
        let (ok, out, err) = run(&["completions", shell]);
        assert!(ok, "cdz completions {shell} failed: {err}");
        assert!(
            out.trim().len() > 100,
            "the {shell} completion script is non-trivial: {} bytes",
            out.len()
        );
    }
}

#[test]
fn completions_reference_the_real_subcommands() {
    // The generated script is derived from the command tree, so it names the actual subcommands — a
    // guard that completions stay in sync (a renamed/dropped command would drop from the script).
    let (ok, out, err) = run(&["completions", "zsh"]);
    assert!(ok, "cdz completions zsh failed: {err}");
    for cmd in ["compile", "run", "build", "test", "check", "completions"] {
        assert!(
            out.contains(cmd),
            "the completion script references the `{cmd}` subcommand: (missing)"
        );
    }
}

#[test]
fn completions_reject_an_unknown_shell_naming_the_set() {
    // A bogus shell is a clap value error listing the valid shells — not a silent empty output.
    let (ok, _out, err) = run(&["completions", "notashell"]);
    assert!(!ok, "an unknown shell should fail");
    assert!(
        err.contains("invalid value") && err.contains("bash") && err.contains("zsh"),
        "error lists the valid shells: {err}"
    );
}
