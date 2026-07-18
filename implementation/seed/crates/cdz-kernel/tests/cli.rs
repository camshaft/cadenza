//! KC-bin — the `cdz-agent` CLI end-to-end (bootstrap → inject-genesis → run one daemon tick).
//!
//! Proves the operator's minimal-CLI (fork-5): the CLI does almost nothing — it bootstraps the log, injects
//! the genesis Cadenza program, and runs a daemon tick that compiles + runs it. Exercises the binary exactly
//! as an operator would, over a temp log + a real genesis program, no network. Gated behind runtime-store
//! availability (a runtime bump can stale the store) — SKIPS the `run` assertion rather than failing.

use std::process::Command;

/// A genesis interpret program: kind==1 → a 2-op plan, else 1-op (the same shape the kernel/daemon tests use).
const GENESIS: &str = "(do \
    (type HostOp (Append String) (Exec String) (Http String) (Noop Int64)) \
    (def (interpret (: kind Int64) (: turn Int64)) \
      (if (= kind 1) (list (Append \"ack\") (Exec \"handle\")) (list (Noop 0)))) \
    (export interpret))";

/// The compiled `cdz-agent` binary (cargo sets CARGO_BIN_EXE_<name> for integration tests).
fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_cdz-agent")
}

fn unique(tag: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("cdz-agent-it-{tag}-{}-{n}", std::process::id()))
}

#[test]
fn bootstrap_then_inject_then_run_drives_the_genesis() {
    let log = unique("log").with_extension("log");
    let prog = unique("genesis").with_extension("cdz");
    let _ = std::fs::remove_file(&log);
    std::fs::write(&prog, GENESIS).unwrap();

    // bootstrap → creates the log (0 events).
    let out = Command::new(bin())
        .args(["bootstrap", log.to_str().unwrap()])
        .output()
        .expect("run cdz-agent bootstrap");
    assert!(
        out.status.success(),
        "bootstrap failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("bootstrapped"),
        "bootstrap prints a confirmation"
    );

    // inject-genesis → appends the program at seq 0.
    let out = Command::new(bin())
        .args([
            "inject-genesis",
            log.to_str().unwrap(),
            prog.to_str().unwrap(),
        ])
        .output()
        .expect("run cdz-agent inject-genesis");
    assert!(
        out.status.success(),
        "inject-genesis failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("seq 0"),
        "the genesis lands at seq 0"
    );

    // run kind=1 → one daemon tick; interpret schedules 2 host-ops. Needs the value-heap runtime store; if it
    // is absent/stale the `run` prints a runtime-not-found error — treat that as a SKIP (not a CLI failure),
    // like the other runtime-gated tests. A store-present run must succeed + report 2 ops.
    let out = Command::new(bin())
        .args(["run", log.to_str().unwrap(), "1"])
        .output()
        .expect("run cdz-agent run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if out.status.success() {
        assert!(
            stdout.contains("scheduled 2 host-op(s)"),
            "kind=1 → the genesis interpret schedules 2 host-ops; got: {stdout}"
        );
    } else {
        assert!(
            stderr.contains("runtime not found"),
            "the only acceptable `run` failure is a missing runtime store (skip); got: {stderr}"
        );
        eprintln!("[cdz-agent it] value-heap runtime absent; skipped the `run` op-count assertion");
    }

    let _ = std::fs::remove_file(&log);
    let _ = std::fs::remove_file(&prog);
}

#[test]
fn perform_executes_the_plan_and_sums_per_op_results() {
    // The `perform` verb is the daemon's real EXECUTION step (K1c): unlike `run` (which COUNTS the scheduled
    // ops), `perform` drives `daemon::tick_performing` so each op fires a real `Prim.run` perform, and the fold
    // SUMS the per-op results — kind=1 → [Append(1), Exec(2)] → 3 (proves each op performed AND its result came
    // back through the cross-fn fold, NOT a count). Same store-gated SKIP as `run` (missing runtime → skip).
    let log = unique("perform-log").with_extension("log");
    let prog = unique("perform-genesis").with_extension("cdz");
    let _ = std::fs::remove_file(&log);
    std::fs::write(&prog, GENESIS).unwrap();

    Command::new(bin())
        .args(["bootstrap", log.to_str().unwrap()])
        .output()
        .expect("bootstrap");
    Command::new(bin())
        .args([
            "inject-genesis",
            log.to_str().unwrap(),
            prog.to_str().unwrap(),
        ])
        .output()
        .expect("inject-genesis");

    let out = Command::new(bin())
        .args(["perform", log.to_str().unwrap(), "1"])
        .output()
        .expect("run cdz-agent perform");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if out.status.success() {
        assert!(
            stdout.contains("summed per-op result = 3"),
            "kind=1 → perform executes [Append(1), Exec(2)] and sums to 3; got: {stdout}"
        );
    } else {
        assert!(
            stderr.contains("runtime not found"),
            "the only acceptable `perform` failure is a missing runtime store (skip); got: {stderr}"
        );
        eprintln!(
            "[cdz-agent it] value-heap runtime absent; skipped the `perform` result assertion"
        );
    }

    let _ = std::fs::remove_file(&log);
    let _ = std::fs::remove_file(&prog);
}

#[test]
fn hosted_performs_via_real_primitives_and_records_prim_events() {
    // The `hosted` verb is the daemon's real-PRIMITIVE step (K1c→host): drives `daemon::tick_hosted`, binding
    // Prim to a real host closure — each performed op is RECORDED in the log as a `prim-<op>` event and its
    // per-op result summed. kind=1 → [Append(1), Exec(2)] → sum 3 AND 2 prim events recorded. Same store-gated
    // SKIP as the other verbs (missing runtime → skip).
    let log = unique("hosted-log").with_extension("log");
    let prog = unique("hosted-genesis").with_extension("cdz");
    let _ = std::fs::remove_file(&log);
    std::fs::write(&prog, GENESIS).unwrap();

    Command::new(bin())
        .args(["bootstrap", log.to_str().unwrap()])
        .output()
        .expect("bootstrap");
    Command::new(bin())
        .args([
            "inject-genesis",
            log.to_str().unwrap(),
            prog.to_str().unwrap(),
        ])
        .output()
        .expect("inject-genesis");

    let out = Command::new(bin())
        .args(["hosted", log.to_str().unwrap(), "1"])
        .output()
        .expect("run cdz-agent hosted");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if out.status.success() {
        assert!(
            stdout.contains("summed per-op result = 3"),
            "kind=1 → hosted performs [Append(1), Exec(2)] via real primitives, sums to 3; got: {stdout}"
        );
        assert!(
            stdout.contains("recorded 2 prim event(s)"),
            "kind=1 → each performed op recorded a prim-<op> event → 2 recorded; got: {stdout}"
        );
    } else {
        assert!(
            stderr.contains("runtime not found"),
            "the only acceptable `hosted` failure is a missing runtime store (skip); got: {stderr}"
        );
        eprintln!(
            "[cdz-agent it] value-heap runtime absent; skipped the `hosted` result assertion"
        );
    }

    let _ = std::fs::remove_file(&log);
    let _ = std::fs::remove_file(&prog);
}

#[test]
fn unknown_subcommand_prints_usage_and_fails() {
    let out = Command::new(bin())
        .arg("frobnicate")
        .output()
        .expect("run cdz-agent with a bad subcommand");
    assert!(
        !out.status.success(),
        "an unknown subcommand exits non-zero"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("usage:"),
        "an unknown subcommand prints usage"
    );
}

#[test]
fn run_without_a_genesis_is_a_loud_error() {
    let log = unique("empty").with_extension("log");
    let _ = std::fs::remove_file(&log);
    Command::new(bin())
        .args(["bootstrap", log.to_str().unwrap()])
        .output()
        .expect("bootstrap");
    // run on a log with no genesis → loud error (the CLI must inject one first).
    let out = Command::new(bin())
        .args(["run", log.to_str().unwrap(), "1"])
        .output()
        .expect("run cdz-agent run");
    assert!(
        !out.status.success(),
        "run without a genesis exits non-zero"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("no genesis program"),
        "run without a genesis reports the missing genesis"
    );
    let _ = std::fs::remove_file(&log);
}
