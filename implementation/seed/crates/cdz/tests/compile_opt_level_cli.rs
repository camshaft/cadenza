//! End-to-end tests for `cdz compile --opt-level` — the optimization-LEVEL request surface.
//!
//! v-core-opt owns the OptLevel→passes MECHANISM (which passes run at which tier); this vertical owns
//! how the level is REQUESTED. These tests pin the CLI contract: every level compiles, both `O2`/`o2`
//! spellings parse, a bogus level is rejected with the choices, and — the invariant that matters while
//! the pass pipeline is still being migrated — a no-flag compile equals the declared default. They do
//! NOT assert bytes DIFFER per level (that's v-core-opt's mechanism to grow); they assert the flag is
//! wired, parsed, and threaded to the compiler.

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

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cdz-opt-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

const PROG: &str = "(module m (def (add (: a Int64) (: b Int64)) (+ a b)) (export add))";

#[test]
fn every_opt_level_compiles() {
    // Each of O0..O3 produces a component — the flag is accepted and threaded through to the compiler
    // at every tier (a level that failed to parse or thread would error here).
    let dir = temp_dir("levels");
    let src = dir.join("add.sexp");
    std::fs::write(&src, PROG).unwrap();
    for lvl in ["O0", "O1", "O2", "O3"] {
        let out = dir.join(format!("{lvl}.wasm"));
        let (ok, _o, err) = run(&[
            "compile",
            src.to_str().unwrap(),
            "--opt-level",
            lvl,
            "-o",
            out.to_str().unwrap(),
        ]);
        assert!(ok, "compile at {lvl} failed: {err}");
        assert!(out.is_file(), "no component at {lvl}: {err}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn opt_level_accepts_both_upper_and_lower_case() {
    // The design taxonomy names levels `O0..O3`; clap's value spelling is lower-case `o0..o3`. Both
    // parse (uppercase via a value alias), so neither a doc example nor a habit-typed `-O2` surprises.
    let dir = temp_dir("case");
    let src = dir.join("add.sexp");
    std::fs::write(&src, PROG).unwrap();
    for lvl in ["O2", "o2"] {
        let out = dir.join("x.wasm");
        let (ok, _o, err) = run(&[
            "compile",
            src.to_str().unwrap(),
            "--opt-level",
            lvl,
            "-o",
            out.to_str().unwrap(),
        ]);
        assert!(ok, "compile at `{lvl}` failed: {err}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_opt_level_flag_equals_the_declared_default() {
    // The one invariant that MUST hold regardless of the pass pipeline's state: a no-flag compile picks
    // the declared default (O1). Since the default level is O1, `cdz compile` (no flag) and `cdz compile
    // --opt-level O1` must produce byte-identical artifacts — proving the default is wired, not dropped.
    let dir = temp_dir("default");
    let src = dir.join("add.sexp");
    std::fs::write(&src, PROG).unwrap();
    let no_flag = dir.join("noflag.wasm");
    let o1 = dir.join("o1.wasm");
    let (ok1, _o, e1) = run(&[
        "compile",
        src.to_str().unwrap(),
        "-o",
        no_flag.to_str().unwrap(),
    ]);
    assert!(ok1, "no-flag compile failed: {e1}");
    let (ok2, _o, e2) = run(&[
        "compile",
        src.to_str().unwrap(),
        "--opt-level",
        "O1",
        "-o",
        o1.to_str().unwrap(),
    ]);
    assert!(ok2, "O1 compile failed: {e2}");
    let a = std::fs::read(&no_flag).unwrap();
    let b = std::fs::read(&o1).unwrap();
    assert_eq!(
        a, b,
        "no-flag compile must equal --opt-level O1 (the default)"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_invalid_opt_level_is_rejected_with_the_choices() {
    // A bogus level is a clap value error listing the valid choices — a helpful CLI, and a guard that
    // the flag validates rather than silently passing garbage to the compiler.
    let dir = temp_dir("bad");
    let src = dir.join("add.sexp");
    std::fs::write(&src, PROG).unwrap();
    let (ok, _o, err) = run(&[
        "compile",
        src.to_str().unwrap(),
        "--opt-level",
        "O9",
        "-o",
        dir.join("x.wasm").to_str().unwrap(),
    ]);
    assert!(!ok, "an invalid opt-level should fail");
    assert!(
        err.contains("invalid value") && err.contains("o0"),
        "error lists the valid choices: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
