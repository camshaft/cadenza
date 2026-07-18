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
fn emit_appends_a_trigger_event_and_refuses_reserved_kinds() {
    // The minimal event SOURCE: `emit <log> <kind> <payload>` appends an external trigger event (so the live
    // daemon has something to perform). It REFUSES reserved kinds (`program` = genesis, `prim-*` = the daemon's
    // own effect records) so an operator can't forge a genesis or a fake effect. Store-independent (no compile).
    let log = unique("emit-log").with_extension("log");
    let _ = std::fs::remove_file(&log);
    Command::new(bin())
        .args(["bootstrap", log.to_str().unwrap()])
        .output()
        .expect("bootstrap");

    // A free-kind trigger appends successfully.
    let out = Command::new(bin())
        .args(["emit", log.to_str().unwrap(), "message", "hello"])
        .output()
        .expect("run cdz-agent emit");
    assert!(
        out.status.success(),
        "emit a trigger: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("emitted `message` event"),
        "emit reports the appended event"
    );

    // A reserved kind is refused (exit non-zero, loud error) — including `policy` (written via emit-policy).
    for reserved in ["program", "prim-exec", "policy"] {
        let out = Command::new(bin())
            .args(["emit", log.to_str().unwrap(), reserved, "x"])
            .output()
            .expect("run cdz-agent emit reserved");
        assert!(
            !out.status.success(),
            "emit of reserved kind `{reserved}` must fail"
        );
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("reserved event kind"),
            "emit of `{reserved}` reports the reserved-kind refusal"
        );
    }
    let _ = std::fs::remove_file(&log);
}

#[test]
fn emit_policy_appends_a_cedar_capability_doc_to_the_log() {
    // The operator's write-Cedar-docs-to-the-log entry point: `emit-policy <log> <policy.cedar>` reads a Cedar
    // policy file and appends it as a `policy` event (the counterpart of inject-genesis for programs). The
    // daemon's tick_hosted_log_policy later retrieves + evaluates it to attenuate each invocation. Store-
    // independent (no compile).
    let log = unique("emitpol-log").with_extension("log");
    let pol = unique("emitpol-policy").with_extension("cedar");
    let _ = std::fs::remove_file(&log);
    std::fs::write(
        &pol,
        r#"permit(principal, action == Action::"prim:append", resource);"#,
    )
    .unwrap();
    Command::new(bin())
        .args(["bootstrap", log.to_str().unwrap()])
        .output()
        .expect("bootstrap");

    let out = Command::new(bin())
        .args(["emit-policy", log.to_str().unwrap(), pol.to_str().unwrap()])
        .output()
        .expect("run cdz-agent emit-policy");
    assert!(
        out.status.success(),
        "emit-policy appends a Cedar doc: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("appended capability policy"),
        "emit-policy reports the appended policy"
    );
    let _ = std::fs::remove_file(&log);
    let _ = std::fs::remove_file(&pol);
}

#[test]
fn authz_grant_and_revoke_surface_the_operator_lifecycle() {
    // The operator's can't-brick lifecycle (deny → auto authz-request → operator GRANT → later REVOKE):
    // `authz-grant <log> <permit.cedar> [--expiry-ms <ms>]` appends a narrow Cedar permit (optionally
    // time-boxed) as an `authz-grant` event; `authz-revoke <log> <grant-seq>` pulls it by seq. The daemon's
    // effective_policy fold unions live grants with the base policy and drops revoked/expired ones. This
    // asserts the CLI surface end-to-end (store-independent — no compile). The seq accounting: bootstrap adds
    // no event, so the first grant lands at seq 0, and the revoke names it.
    let log = unique("authz-log").with_extension("log");
    let permit = unique("authz-permit").with_extension("cedar");
    let _ = std::fs::remove_file(&log);
    std::fs::write(
        &permit,
        r#"permit(principal, action == Action::"prim:exec", resource);"#,
    )
    .unwrap();
    Command::new(bin())
        .args(["bootstrap", log.to_str().unwrap()])
        .output()
        .expect("bootstrap");

    // GRANT with an absolute expiry — reports the seq and the expiry.
    let out = Command::new(bin())
        .args([
            "authz-grant",
            log.to_str().unwrap(),
            permit.to_str().unwrap(),
            "--expiry-ms",
            "9999999999999",
        ])
        .output()
        .expect("run cdz-agent authz-grant");
    assert!(
        out.status.success(),
        "authz-grant appends a permit: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let grant_stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        grant_stdout.contains("granted capability at seq 0"),
        "authz-grant reports the grant seq: {grant_stdout}"
    );
    assert!(
        grant_stdout.contains("expires at"),
        "authz-grant with --expiry-ms reports the expiry: {grant_stdout}"
    );

    // REVOKE the grant by its seq.
    let out = Command::new(bin())
        .args(["authz-revoke", log.to_str().unwrap(), "0"])
        .output()
        .expect("run cdz-agent authz-revoke");
    assert!(
        out.status.success(),
        "authz-revoke pulls the grant: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("revoked grant at seq 0"),
        "authz-revoke reports the revoked grant seq"
    );

    // The reserved-kind guard now also covers the authz kinds — `emit` must refuse them.
    for reserved in ["authz-grant", "authz-revoke", "authz-request"] {
        let out = Command::new(bin())
            .args(["emit", log.to_str().unwrap(), reserved, "x"])
            .output()
            .expect("run cdz-agent emit reserved authz");
        assert!(
            !out.status.success(),
            "emit of reserved authz kind `{reserved}` must fail"
        );
    }

    let _ = std::fs::remove_file(&log);
    let _ = std::fs::remove_file(&permit);
}

#[test]
fn schedule_create_and_cancel_register_and_drop_a_timer() {
    // The operator scheduling entry points (counterpart of emit-policy/authz for timers): schedule-create
    // registers a one-shot/periodic timer that fires a trigger-kind; schedule-cancel drops it by id. Asserts
    // the CLI surface (store-independent — no compile). A reserved trigger-kind is refused (can't forge kernel
    // bookkeeping via a timer).
    let log = unique("sched-log").with_extension("log");
    let _ = std::fs::remove_file(&log);
    Command::new(bin())
        .args(["bootstrap", log.to_str().unwrap()])
        .output()
        .expect("bootstrap");

    // A PERIODIC create with a payload.
    let out = Command::new(bin())
        .args([
            "schedule-create",
            log.to_str().unwrap(),
            "heartbeat",
            "hb-tick",
            "--first-ms",
            "1000",
            "--period-ms",
            "5000",
            "--payload",
            "ping",
        ])
        .output()
        .expect("run cdz-agent schedule-create");
    assert!(
        out.status.success(),
        "schedule-create registers a timer: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let so = String::from_utf8_lossy(&out.stdout);
    assert!(
        so.contains("scheduled `heartbeat`"),
        "reports the schedule id: {so}"
    );
    assert!(
        so.contains("periodic"),
        "reports periodic when --period-ms is given: {so}"
    );

    // CANCEL by id.
    let out = Command::new(bin())
        .args(["schedule-cancel", log.to_str().unwrap(), "heartbeat"])
        .output()
        .expect("run cdz-agent schedule-cancel");
    assert!(
        out.status.success(),
        "schedule-cancel drops the timer: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("cancelled schedule `heartbeat`"),
        "schedule-cancel reports the cancelled id"
    );

    // A reserved trigger-kind is refused.
    let out = Command::new(bin())
        .args([
            "schedule-create",
            log.to_str().unwrap(),
            "evil",
            "program", // reserved (genesis) — must be refused
            "--first-ms",
            "0",
        ])
        .output()
        .expect("run cdz-agent schedule-create reserved");
    assert!(
        !out.status.success(),
        "a reserved trigger-kind must be refused"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("reserved trigger-kind"),
        "reports the reserved-trigger refusal"
    );

    // emit still refuses schedule-* kinds too.
    for reserved in ["schedule-create", "schedule-cancel", "schedule-fire"] {
        let out = Command::new(bin())
            .args(["emit", log.to_str().unwrap(), reserved, "x"])
            .output()
            .expect("run cdz-agent emit reserved schedule");
        assert!(
            !out.status.success(),
            "emit of reserved schedule kind `{reserved}` must fail"
        );
    }

    let _ = std::fs::remove_file(&log);
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
            stdout.contains("2 performed + 0 denied"),
            "kind=1 ungated → both ops performed, none denied → 2 performed + 0 denied; got: {stdout}"
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
fn hosted_with_policies_gates_each_op_on_the_external_cedar_policy() {
    // The capability boundary from the CLI: `hosted --policies <file>` authorizes each op against an EXTERNAL
    // Cedar policy before performing. The policy permits ONLY prim:append → kind=1 [Append, Exec] → append
    // performed (1) + exec DENIED (0) → sum 1, 1 performed + 1 denied. Proves the operator can gate the agent's
    // real primitives from the command line. Store-gated skip like the other verbs.
    let log = unique("hostedpol-log").with_extension("log");
    let prog = unique("hostedpol-genesis").with_extension("cdz");
    let pol = unique("hostedpol-policy").with_extension("cedar");
    let _ = std::fs::remove_file(&log);
    std::fs::write(&prog, GENESIS).unwrap();
    // External root policy: permit ONLY prim:append; prim:exec is deny-by-default.
    std::fs::write(
        &pol,
        r#"permit(principal, action == Action::"prim:append", resource);"#,
    )
    .unwrap();

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
        .args([
            "hosted",
            log.to_str().unwrap(),
            "1",
            "--policies",
            pol.to_str().unwrap(),
        ])
        .output()
        .expect("run cdz-agent hosted --policies");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if out.status.success() {
        assert!(
            stdout.contains("summed per-op result = 1"),
            "policy permits only append → append(1) + exec denied(0) → sum 1; got: {stdout}"
        );
        assert!(
            stdout.contains("1 performed + 1 denied"),
            "append performed, exec denied → 1 performed + 1 denied; got: {stdout}"
        );
    } else {
        assert!(
            stderr.contains("runtime not found"),
            "the only acceptable failure is a missing runtime store (skip); got: {stderr}"
        );
        eprintln!(
            "[cdz-agent it] value-heap runtime absent; skipped the `hosted --policies` assertion"
        );
    }

    let _ = std::fs::remove_file(&log);
    let _ = std::fs::remove_file(&prog);
    let _ = std::fs::remove_file(&pol);
}

#[test]
fn start_runs_the_daemon_loop_bounded_and_stops_cleanly() {
    // The `start` verb runs the LIVE daemon loop (poll → perform → sleep), bounded by --max-rounds so the run
    // terminates deterministically. On a genesis-only log the loop performs nothing (the `program` event is
    // skipped — not a trigger) and stops cleanly. Store-gated: open_and_resolve needs the value-heap runtime;
    // a missing store surfaces as a runtime-not-found error (treated as SKIP, like run/perform/hosted).
    let log = unique("start-log").with_extension("log");
    let prog = unique("start-genesis").with_extension("cdz");
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

    // poll 0ms, exactly 1 round → deterministic termination.
    let out = Command::new(bin())
        .args(["start", log.to_str().unwrap(), "0", "--max-rounds", "1"])
        .output()
        .expect("run cdz-agent start");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if out.status.success() {
        assert!(
            stdout.contains("daemon polling") && stdout.contains("stopped cleanly"),
            "start runs the bounded loop and stops cleanly; got: {stdout}"
        );
    } else {
        assert!(
            stderr.contains("runtime not found"),
            "the only acceptable `start` failure is a missing runtime store (skip); got: {stderr}"
        );
        eprintln!("[cdz-agent it] value-heap runtime absent; skipped the `start` loop assertion");
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
