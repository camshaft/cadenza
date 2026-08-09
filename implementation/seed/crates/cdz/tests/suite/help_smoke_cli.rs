//! Smoke tests for the unified `cdz` subcommand TREE — the one-binary surface.
//!
//! `cdz` folds many tools into one binary (front-end, compiler, runner, corpus, the semantic queries,
//! the LSP). As more standalone bins get folded in, the risk is a refactor SILENTLY dropping, renaming,
//! or breaking a subcommand — the whole point of the one-binary story is that every tool stays reachable
//! under `cdz`. These tests guard that surface WITHOUT hard-coding the full (growing) command list:
//!
//!   1. A STABLE CORE set of subcommands must always be present (the commands that define the tool —
//!      removing one is a breaking change, not a routine edit).
//!   2. EVERY subcommand clap lists in `cdz --help` must itself answer `cdz <cmd> --help` with exit 0 —
//!      so a command mounted with a broken arg definition (a clap conflict, a bad default) is caught even
//!      though we don't enumerate it here. This is what makes the test future-proof: a newly folded
//!      command is covered automatically.
//!   3. Top-level `--help` and `-V`/`--version` succeed; a bogus subcommand fails with a non-zero exit.
//!
//! Deliberately merge-order-independent: it asserts only the core (long-merged) commands by name, and
//! discovers the rest dynamically, so it lands cleanly regardless of which fold merges first.

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

/// Parse the subcommand NAMES out of `cdz --help`. clap prints a `Commands:` section, one command per
/// line as `  <name>  <about…>`; the name is the first whitespace-delimited token of an indented line.
/// The section ends at the next unindented header (`Options:`). Returns every listed command except the
/// built-in `help`.
fn subcommand_names() -> Vec<String> {
    let (ok, out, err) = run(&["--help"]);
    assert!(ok, "cdz --help should succeed: {err}");
    let mut names = Vec::new();
    let mut in_commands = false;
    for line in out.lines() {
        if line.starts_with("Commands:") {
            in_commands = true;
            continue;
        }
        if in_commands {
            // The section ends at the next top-level header (a non-indented, non-empty line).
            if !line.starts_with(' ') && !line.trim().is_empty() {
                break;
            }
            if let Some(first) = line.split_whitespace().next()
                && line.starts_with("  ")
                && first != "help"
            {
                names.push(first.to_string());
            }
        }
    }
    names
}

#[test]
fn the_stable_core_subcommands_are_present() {
    // These commands DEFINE the toolchain — a build that drops one is broken, not refactored. (New
    // folds ADD commands; this set is the floor, deliberately not the full list so it doesn't churn.)
    let names = subcommand_names();
    for core in [
        "convert", "fmt", "query", "rewrite", "compile", "run", "test", "type", "check", "fix",
    ] {
        assert!(
            names.iter().any(|n| n == core),
            "core subcommand `{core}` missing from `cdz --help`; found: {names:?}"
        );
    }
}

#[test]
fn every_listed_subcommand_answers_its_own_help() {
    // Dynamic coverage: whatever `cdz --help` lists, `cdz <cmd> --help` must succeed. Catches a command
    // mounted with a broken clap definition (arg conflict, bad default) without enumerating them here —
    // so a freshly folded subcommand is protected the moment it appears in the tree.
    let names = subcommand_names();
    assert!(
        names.len() >= 10,
        "expected many subcommands, found {}: {names:?}",
        names.len()
    );
    for name in &names {
        let (ok, _out, err) = run(&[name, "--help"]);
        assert!(ok, "`cdz {name} --help` should succeed but failed: {err}");
    }
}

#[test]
fn top_level_help_and_version_succeed() {
    let (help_ok, help_out, _) = run(&["--help"]);
    assert!(help_ok, "cdz --help failed");
    assert!(
        help_out.contains("Commands:"),
        "help lists a Commands section: {help_out}"
    );
    // `--version` must not just exit 0 — it must actually PRINT the program name and the crate version
    // (a regression that printed an empty string, or dropped the version, would slip an exit-code-only
    // check). clap renders `<bin> <version>`; pin both halves against the compiled crate version.
    let (v_ok, v_out, _) = run(&["--version"]);
    assert!(v_ok, "cdz --version failed");
    let want_version = env!("CARGO_PKG_VERSION");
    assert!(
        v_out.contains("cdz") && v_out.contains(want_version),
        "`cdz --version` prints the name and crate version `{want_version}`: {v_out:?}"
    );
    // `-V` is the short alias and must report the same version string as `--version`.
    let (short_ok, short_out, _) = run(&["-V"]);
    assert!(short_ok, "cdz -V failed");
    assert_eq!(
        short_out.trim(),
        v_out.trim(),
        "`cdz -V` reports the same version line as `cdz --version`"
    );
}

#[test]
fn top_level_long_help_describes_the_project_workflow() {
    // `cdz --help` should make the cargo-analogue PROJECT workflow discoverable — a user shouldn't have to
    // already know `new`/`build`/`run`/`test` exist. The long_about names the project lifecycle.
    let (ok, out, err) = run(&["--help"]);
    assert!(ok, "cdz --help failed: {err}");
    // The long_about mentions the project verbs so `--help` surfaces the workflow, not just a flat list.
    assert!(
        out.contains("Project.cdz") && out.contains("scaffold"),
        "long help describes the project workflow (Project.cdz + scaffold): {out}"
    );
}

#[test]
fn an_unknown_subcommand_fails() {
    // A bogus subcommand is a usage error: non-zero exit, a clap error on stderr. Guards that the tree
    // actually rejects garbage rather than silently doing nothing.
    let (ok, _out, err) = run(&["definitely-not-a-subcommand"]);
    assert!(!ok, "an unknown subcommand should fail");
    assert!(
        err.contains("error") || err.contains("unrecognized") || err.contains("Usage"),
        "unknown subcommand yields a usage error: {err}"
    );
}
