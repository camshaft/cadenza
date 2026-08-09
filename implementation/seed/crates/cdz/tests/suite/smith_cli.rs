//! End-to-end tests for `cdz smith` (alias `cdz fuzz`) — the PASSTHROUGH mount of the standalone
//! `cdz-smith` fuzzer/differential driver. Unlike `cdz run`/`cdz corpus`/`cdz calc` (which are FOLDED in
//! as linked libraries), `cdz smith` is exec-not-link ON PURPOSE: `cdz-smith` is a SEPARATE cargo
//! workspace (its `bolero` + wasmtime/cranelift dep tree cannot co-resolve into `cdz`'s lockfile), so
//! `cdz smith <args…>` locates and execs the sibling `cdz-smith` binary and forwards argv + exit code.
//!
//! The standalone `cdz-smith` binary may or may not be BUILT in a given test environment (it is a
//! separate `cargo build -p cdz-smith`, not produced by `cargo test -p cdz`). So these tests pin the
//! PASSTHROUGH CONTRACT robustly against BOTH states: `cdz smith` must EITHER pass through to the bin
//! (when present) OR fail with the clear actionable "build it" error (when absent) — never a panic, a
//! clap misparse of the forwarded args, or a silent success. The mount + argv-forwarding + alias are
//! what this vertical owns; the fuzzer's behavior itself is tested in the cdz-smith workspace.

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

/// True if the combined output is the clean "cdz-smith not built" passthrough error — the expected
/// outcome when the separate-workspace binary is absent. It must name the binary AND the build command
/// (an actionable fix), not just fail opaquely.
fn is_clean_not_found(stderr: &str) -> bool {
    stderr.contains("cdz-smith")
        && stderr.contains("cargo build -p cdz-smith")
        && stderr.contains("cdz:")
}

#[test]
fn smith_is_a_passthrough_not_a_clap_misparse() {
    // `cdz smith --help` forwards `--help` to the standalone bin. Whatever the outcome, it must NOT be
    // `cdz`'s OWN clap rejecting `--help` as an unknown arg, and must NOT panic. Two valid outcomes:
    //   - the bin is present → it handled `--help` (passthrough worked), OR
    //   - the bin is absent → our clean actionable not-found error.
    let (ok, out, err) = run(&["smith", "--help"]);
    assert!(
        !err.contains("panic") && !err.contains("RUST_BACKTRACE"),
        "no panic on the passthrough: {err}"
    );
    if ok {
        // Passthrough reached the bin and it succeeded — `cdz` did not eat the arg.
        assert!(
            !err.contains("unexpected argument"),
            "`--help` was forwarded to cdz-smith, not parsed by cdz: {err}"
        );
        let _ = out;
    } else {
        // The bin isn't built in this env: our error must be the clear, actionable one — NOT a clap
        // usage error from `cdz` itself failing to parse the trailing args.
        assert!(
            is_clean_not_found(&err),
            "absent cdz-smith yields the actionable build hint, not an opaque failure: {err}"
        );
    }
}

#[test]
fn smith_forwards_arbitrary_trailing_args_without_parsing_them() {
    // Flags that are NOT `cdz` flags (and a leading-hyphen value) must pass through untouched — the
    // `trailing_var_arg` + `allow_hyphen_values` contract. If `cdz` tried to parse `--iters`/`--seed` it
    // would clap-error with "unexpected argument"; the passthrough must never produce that. We assert ONLY
    // the cdz-SIDE contract (cdz doesn't eat the args), NOT what the bin does with them: a CO-PRESENT
    // `cdz-smith` may itself reject an unknown flag and exit non-zero, which is the bin's business, not a
    // cdz parse failure — so a non-success is fine as long as it isn't cdz's own "unexpected argument".
    let (_ok, _out, err) = run(&["smith", "--iters", "3", "--seed", "-1", "--not-a-cdz-flag"]);
    assert!(
        !err.contains("unexpected argument") && !err.contains("panic"),
        "arbitrary trailing args are forwarded verbatim, not parsed by cdz: {err}"
    );
    // The error (if any) must NOT be cdz's own clap usage error — it's EITHER the bin's own output (present)
    // OR our clean not-found (absent). Both acceptable; a cdz-level parse failure is not.
    assert!(
        !err.contains("error: unrecognized") && !err.contains("Usage: cdz smith"),
        "a non-success is the bin's own output or the actionable not-found, not a cdz parse error: {err}"
    );
}

#[test]
fn fuzz_is_an_alias_for_smith() {
    // `cdz fuzz` is the discoverability alias and must mount the SAME passthrough (not be an unknown
    // subcommand). It behaves identically to `cdz smith`.
    let (ok, _out, err) = run(&["fuzz", "--help"]);
    assert!(
        !err.contains("unrecognized subcommand") && !err.contains("panic"),
        "`cdz fuzz` is a valid alias of `cdz smith`, not an unknown command: {err}"
    );
    if !ok {
        assert!(
            is_clean_not_found(&err),
            "the alias shares the same actionable not-found error when the bin is absent: {err}"
        );
    }
}

#[test]
fn smith_appears_in_the_command_tree() {
    // The passthrough must be discoverable in `cdz --help` (the one-binary story: every tool reachable
    // under `cdz`). clap lists it in the Commands section.
    let (ok, out, _err) = run(&["--help"]);
    assert!(ok, "cdz --help succeeds");
    assert!(
        out.contains("smith"),
        "`smith` is listed as a subcommand in `cdz --help`: {out}"
    );
}
