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

    // A reserved kind is refused (exit non-zero, loud error) — including `policy` (written via emit-policy) and
    // `daemon-cursor` (the daemon's private resume bookmark: an operator must not forge it via emit, else a
    // poison cursor could corrupt crash-recovery / auto-resume).
    for reserved in ["program", "prim-exec", "policy", "daemon-cursor"] {
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

    // authz-requests on a fresh log → none (requests are daemon-auto-emitted on a denied op; a bare log has
    // none). The populated fold is covered by policy.rs's unit test (needs denied ops the daemon emits).
    let out = Command::new(bin())
        .args(["authz-requests", log.to_str().unwrap()])
        .output()
        .expect("run cdz-agent authz-requests");
    assert!(out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("no authorization requests"),
        "a fresh log has no standing authz-requests"
    );

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

    // A ZERO --period-ms is refused up front (it would divide-by-zero the daemon's `due` fold every tick — a
    // can't-brick violation). The error steers the operator to omit --period-ms for a one-shot.
    let out = Command::new(bin())
        .args([
            "schedule-create",
            log.to_str().unwrap(),
            "zero",
            "z-tick",
            "--first-ms",
            "0",
            "--period-ms",
            "0",
        ])
        .output()
        .expect("run cdz-agent schedule-create zero-period");
    assert!(
        !out.status.success(),
        "a zero --period-ms must be refused (would brick the due fold)"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--period-ms must be a POSITIVE integer"),
        "reports the positive-period requirement: {}",
        String::from_utf8_lossy(&out.stderr)
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
fn schedule_list_shows_active_schedules_and_reflects_supersede_and_cancel() {
    // The operator read verb: schedule-list folds the active set (newest create per id minus cancelled). An
    // empty log → "no active schedules"; after two creates → both listed with cadence + trigger; after a
    // cancel → the cancelled one drops. Read-only, store-independent (no compile).
    let log = unique("sched-list-log").with_extension("log");
    let _ = std::fs::remove_file(&log);
    Command::new(bin())
        .args(["bootstrap", log.to_str().unwrap()])
        .output()
        .expect("bootstrap");

    // Empty → "no active schedules".
    let out = Command::new(bin())
        .args(["schedule-list", log.to_str().unwrap()])
        .output()
        .expect("run cdz-agent schedule-list (empty)");
    assert!(out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("no active schedules"),
        "empty log lists no schedules"
    );

    // Two schedules: a periodic and a one-shot.
    Command::new(bin())
        .args([
            "schedule-create",
            log.to_str().unwrap(),
            "hb",
            "heartbeat",
            "--first-ms",
            "1000",
            "--period-ms",
            "5000",
        ])
        .output()
        .expect("create periodic");
    Command::new(bin())
        .args([
            "schedule-create",
            log.to_str().unwrap(),
            "once",
            "wake",
            "--first-ms",
            "2000",
        ])
        .output()
        .expect("create one-shot");

    let out = Command::new(bin())
        .args(["schedule-list", log.to_str().unwrap()])
        .output()
        .expect("run cdz-agent schedule-list (two)");
    let so = String::from_utf8_lossy(&out.stdout);
    assert!(so.contains("2 active schedule(s)"), "both listed: {so}");
    assert!(
        so.contains("hb") && so.contains("every 5000ms") && so.contains("heartbeat"),
        "periodic shown: {so}"
    );
    assert!(
        so.contains("once") && so.contains("one-shot") && so.contains("wake"),
        "one-shot shown: {so}"
    );

    // Cancel the periodic → only the one-shot remains.
    Command::new(bin())
        .args(["schedule-cancel", log.to_str().unwrap(), "hb"])
        .output()
        .expect("cancel hb");
    let out = Command::new(bin())
        .args(["schedule-list", log.to_str().unwrap()])
        .output()
        .expect("run cdz-agent schedule-list (after cancel)");
    let so = String::from_utf8_lossy(&out.stdout);
    assert!(
        so.contains("1 active schedule(s)"),
        "one remains after cancel: {so}"
    );
    assert!(!so.contains("hb"), "the cancelled periodic is gone: {so}");
    assert!(so.contains("once"), "the one-shot remains: {so}");

    let _ = std::fs::remove_file(&log);
}

#[test]
fn cursor_reports_the_resume_high_water_mark_and_unprocessed_backlog() {
    // The `cursor` read verb (companion to crash-recovery): reports the daemon's resume high-water mark and the
    // unprocessed backlog. Store-independent (a pure latest_cursor fold — no compile). A fresh log has no
    // recorded cursor → reports "process the whole log"; after a daemon-cursor is recorded, reports the mark +
    // how many events remain beyond it. We write the daemon-cursor via a bounded `start` run (store-gated) OR
    // fall back to asserting only the no-cursor branch when the store is absent.
    let log = unique("cursor-log").with_extension("log");
    let prog = unique("cursor-genesis").with_extension("cdz");
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
    // A trigger the daemon would process (seq 1).
    Command::new(bin())
        .args(["emit", log.to_str().unwrap(), "trigger", "go"])
        .output()
        .expect("emit trigger");

    // Before any daemon run: no cursor recorded → the whole-log branch.
    let out = Command::new(bin())
        .args(["cursor", log.to_str().unwrap()])
        .output()
        .expect("run cdz-agent cursor (fresh)");
    assert!(
        out.status.success(),
        "cursor is read-only + always succeeds: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let so = String::from_utf8_lossy(&out.stdout);
    assert!(
        so.contains("no daemon-cursor recorded yet") && so.contains("1 pending trigger(s)"),
        "fresh log → 1 pending trigger (genesis is reserved, the emitted trigger is pending); got: {so}"
    );

    // Run one bounded daemon round to record a daemon-cursor (store-gated — skip the mark assertion if absent).
    let started = Command::new(bin())
        .args(["start", log.to_str().unwrap(), "0", "--max-rounds", "1"])
        .output()
        .expect("run cdz-agent start");
    if !started.status.success() {
        assert!(
            String::from_utf8_lossy(&started.stderr).contains("runtime not found"),
            "only acceptable failure is a missing store (skip); got: {}",
            String::from_utf8_lossy(&started.stderr)
        );
        eprintln!(
            "[cdz-agent it] value-heap runtime absent; skipped the recorded-cursor assertion"
        );
        let _ = std::fs::remove_file(&log);
        let _ = std::fs::remove_file(&prog);
        return;
    }
    // After the round: cursor reports the recorded high-water mark (past genesis+trigger), backlog 0.
    let out = Command::new(bin())
        .args(["cursor", log.to_str().unwrap()])
        .output()
        .expect("run cdz-agent cursor (after run)");
    let so = String::from_utf8_lossy(&out.stdout);
    assert!(
        so.contains("resume high-water mark =") && so.contains("0 pending trigger(s)"),
        "after a round: reports the mark with 0 pending triggers beyond it; got: {so}"
    );

    let _ = std::fs::remove_file(&log);
    let _ = std::fs::remove_file(&prog);
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
        // The exec op's scalar result is the record-only tag (2) in the default build, but its subprocess EXIT
        // CODE under `--features live-exec` (127 for the not-found `handle` payload here — environment-dependent),
        // so the summed literal (3) is only asserted in the default build. In BOTH builds the STRUCTURAL invariant
        // holds: both ops performed, none denied. (Mirrors daemon.rs's `genesis_hosted_sum`/record-only gating.)
        #[cfg(not(feature = "live-exec"))]
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
fn replay_re_folds_a_recorded_hosted_turn_with_no_live_effect() {
    // The `replay` verb is the §2.3 time-travel proof from the CLI: after a `hosted` run RECORDS a turn's
    // prim-result trail, `replay` RE-FOLDS the same log answering each op from that trail — reproducing the
    // identical summed result (3) with `missing = 0` (faithful, deterministic) and performing NO live effect.
    // Same store-gated SKIP as the other verbs (missing runtime → skip).
    let log = unique("replay-log").with_extension("log");
    let prog = unique("replay-genesis").with_extension("cdz");
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

    // First RECORD the turn via `hosted` (writes the prim-result-* trail replay reads).
    let hosted = Command::new(bin())
        .args(["hosted", log.to_str().unwrap(), "1"])
        .output()
        .expect("run cdz-agent hosted");
    if !hosted.status.success() {
        // No runtime store → skip (same gate as the other verbs); replay has nothing to re-fold.
        assert!(
            String::from_utf8_lossy(&hosted.stderr).contains("runtime not found"),
            "the only acceptable failure is a missing runtime store (skip); got: {}",
            String::from_utf8_lossy(&hosted.stderr)
        );
        eprintln!("[cdz-agent it] value-heap runtime absent; skipped the `replay` assertion");
        let _ = std::fs::remove_file(&log);
        let _ = std::fs::remove_file(&prog);
        return;
    }

    // Now REPLAY: re-fold the recorded turn — same summed result, missing=0, faithful.
    let out = Command::new(bin())
        .args(["replay", log.to_str().unwrap(), "1"])
        .output()
        .expect("run cdz-agent replay");
    assert!(
        out.status.success(),
        "replay of a recorded turn succeeds: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // The recorded sum embeds exec's result, which is record-only (2 → sum 3) in the default build but the
    // environment-dependent subprocess exit code under `--features live-exec`; assert the literal only in the
    // default build. The build-INDEPENDENT invariant — a faithful, deterministic re-fold with 0 missing —
    // holds in BOTH: replay reproduces WHATEVER the live turn recorded, from the prim-result trail alone.
    #[cfg(not(feature = "live-exec"))]
    assert!(
        stdout.contains("summed per-op result = 3"),
        "replay reproduces the recorded sum (3) from the prim-result trail; got: {stdout}"
    );
    assert!(
        stdout.contains("0 missing") && stdout.contains("faithful"),
        "a faithful replay reports 0 missing (deterministic re-fold); got: {stdout}"
    );

    let _ = std::fs::remove_file(&log);
    let _ = std::fs::remove_file(&prog);
}

#[test]
fn fork_copies_a_history_into_a_new_timeline_with_and_without_a_cutoff() {
    // The `fork` verb branches a recorded history into a NEW log (vision §3 fork/hand-off/time-travel). Fully
    // store-independent (no compile) — it just copies events. Build a src log (genesis + 2 triggers), fork the
    // FULL history into one dst, and fork with --upto 2 (exclusive) into another → the cutoff branch holds only
    // the genesis + first trigger. The genesis survives the fork (the branch is a valid log the daemon runs).
    let src = unique("fork-src").with_extension("log");
    let full = unique("fork-full").with_extension("log");
    let cut = unique("fork-cut").with_extension("log");
    let prog = unique("fork-genesis").with_extension("cdz");
    for p in [&src, &full, &cut] {
        let _ = std::fs::remove_file(p);
    }
    std::fs::write(&prog, GENESIS).unwrap();

    Command::new(bin())
        .args(["bootstrap", src.to_str().unwrap()])
        .output()
        .expect("bootstrap");
    Command::new(bin())
        .args([
            "inject-genesis",
            src.to_str().unwrap(),
            prog.to_str().unwrap(),
        ])
        .output()
        .expect("inject-genesis"); // seq 0
    Command::new(bin())
        .args(["emit", src.to_str().unwrap(), "trigger", "a"])
        .output()
        .expect("emit a"); // seq 1
    Command::new(bin())
        .args(["emit", src.to_str().unwrap(), "trigger", "b"])
        .output()
        .expect("emit b"); // seq 2

    // FULL fork → all 3 events copied.
    let out = Command::new(bin())
        .args(["fork", src.to_str().unwrap(), full.to_str().unwrap()])
        .output()
        .expect("run cdz-agent fork full");
    assert!(
        out.status.success(),
        "fork (full) succeeds: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("copied 3 event(s)"),
        "the full fork copies all 3 events; got: {}",
        String::from_utf8_lossy(&out.stdout)
    );

    // CUTOFF fork --upto 2 (exclusive) → only seq 0 + 1 copied.
    let out = Command::new(bin())
        .args([
            "fork",
            src.to_str().unwrap(),
            cut.to_str().unwrap(),
            "--upto",
            "2",
        ])
        .output()
        .expect("run cdz-agent fork cutoff");
    assert!(
        out.status.success(),
        "fork (cutoff) succeeds: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let so = String::from_utf8_lossy(&out.stdout);
    assert!(
        so.contains("copied 2 event(s)") && so.contains("up to seq 2, exclusive"),
        "the cutoff fork copies only the 2 pre-cutoff events; got: {so}"
    );
    // The cutoff branch still carries the genesis → `run` finds a program (store-gated like the run verbs).
    let out = Command::new(bin())
        .args(["run", cut.to_str().unwrap(), "1"])
        .output()
        .expect("run on the forked branch");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success() || stderr.contains("runtime not found"),
        "the forked branch is a valid log: run succeeds or only skips on a missing store; got: {stderr}"
    );

    for p in [&src, &full, &cut, &prog] {
        let _ = std::fs::remove_file(p);
    }
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
fn a_denied_op_files_an_authz_request_that_authz_requests_lists_end_to_end() {
    // The can't-brick loop, observable through the real binary end-to-end: run `hosted --policies` with a policy
    // that permits ONLY prim:append, so kind=1's prim:exec is DENIED — the daemon auto-emits an authz-request for
    // the denied op. Then `authz-requests` must LIST that standing ask (the operator sees what's waiting on a
    // grant). This is the safety-critical property (a denial is never silent — it leaves a grantable request in
    // the log) proven through the CLI, not just the lib fold. Store-gated SKIP like the other daemon-path verbs.
    let log = unique("cantbrick-log").with_extension("log");
    let prog = unique("cantbrick-genesis").with_extension("cdz");
    let pol = unique("cantbrick-policy").with_extension("cedar");
    let _ = std::fs::remove_file(&log);
    std::fs::write(&prog, GENESIS).unwrap();
    // Permit ONLY prim:append → prim:exec is denied → an authz-request is auto-filed for exec.
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
    if !out.status.success() {
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("runtime not found"),
            "the only acceptable failure is a missing runtime store (skip); got: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        eprintln!(
            "[cdz-agent it] value-heap runtime absent; skipped the can't-brick e2e assertion"
        );
        let _ = std::fs::remove_file(&log);
        let _ = std::fs::remove_file(&prog);
        let _ = std::fs::remove_file(&pol);
        return;
    }

    // The denied exec left a standing authz-request → authz-requests lists it (op = exec).
    let out = Command::new(bin())
        .args(["authz-requests", log.to_str().unwrap()])
        .output()
        .expect("run cdz-agent authz-requests");
    assert!(
        out.status.success(),
        "authz-requests runs: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let so = String::from_utf8_lossy(&out.stdout);
    assert!(
        so.contains("authorization request(s):") && so.contains("prim:exec"),
        "the denied exec filed an authz-request that authz-requests lists (can't-brick, observable): {so}"
    );

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
fn start_fires_a_due_schedule_end_to_end_through_the_live_daemon() {
    // The scheduling read-side END TO END through the CLI: a due schedule (first_ms=0 → always due vs the wall
    // clock) is registered, then `start --max-rounds 1` runs one live daemon round. fire_due_schedules appends
    // the schedule's TRIGGER event + a schedule-fire record BEFORE run_once drains, so after the round the log
    // holds both. This exercises the full clock→due→append-trigger→fire path the unit tests cover in pieces,
    // but here through the real binary + daemon loop. Store-gated (open_and_resolve needs the value-heap
    // runtime): a missing store surfaces as runtime-not-found → SKIP (like run/perform/hosted/start).
    let log = unique("sched-fire-log").with_extension("log");
    let prog = unique("sched-fire-genesis").with_extension("cdz");
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
    // A one-shot due immediately (first_ms=0), firing a `wake` trigger with a payload.
    Command::new(bin())
        .args([
            "schedule-create",
            log.to_str().unwrap(),
            "boot-timer",
            "wake",
            "--first-ms",
            "0",
            "--payload",
            "rise",
        ])
        .output()
        .expect("schedule-create");

    // One bounded round of the live daemon.
    let out = Command::new(bin())
        .args(["start", log.to_str().unwrap(), "0", "--max-rounds", "1"])
        .output()
        .expect("run cdz-agent start");
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        assert!(
            stderr.contains("runtime not found"),
            "the only acceptable failure is a missing runtime store (skip); got: {stderr}"
        );
        eprintln!("[cdz-agent it] value-heap runtime absent; skipped the scheduled-fire assertion");
        let _ = std::fs::remove_file(&log);
        let _ = std::fs::remove_file(&prog);
        return;
    }

    // The round fired the due schedule. Verify the DURABLE effect: read the raw log and confirm the daemon
    // wrote both the `wake` trigger (with its payload) and a `schedule-fire` record for the schedule id.
    let raw = std::fs::read(&log).expect("read the log file");
    let text = String::from_utf8_lossy(&raw);
    assert!(
        text.contains("wake"),
        "the due schedule's trigger event (`wake`) was appended by fire_due_schedules"
    );
    assert!(
        text.contains("rise"),
        "the trigger carries the schedule's payload (`rise`)"
    );
    assert!(
        text.contains("schedule-fire"),
        "a schedule-fire record was appended (advances the occurrence counter)"
    );

    let _ = std::fs::remove_file(&log);
    let _ = std::fs::remove_file(&prog);
}

#[test]
fn start_from_a_resume_cursor_does_not_re_perform_processed_history() {
    // Crash-recovery: a restarted daemon must NOT re-drain the whole log from 0 and re-perform every historical
    // trigger (an at-most-once violation). `start` reports its final cursor; restarting with `--from <cursor>`
    // resumes past the processed events. Here: emit a trigger, run once (performs it → records prim events),
    // capture the reported cursor, then restart with --from <cursor> and assert NO new prim-record events were
    // appended (the already-processed trigger is not re-performed). Store-gated skip like the sibling verbs.
    let log = unique("resume-log").with_extension("log");
    let prog = unique("resume-genesis").with_extension("cdz");
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
    // A trigger the daemon will PERFORM (kind=1 → [Append, Exec]).
    Command::new(bin())
        .args(["emit", log.to_str().unwrap(), "trigger", "go"])
        .output()
        .expect("emit trigger");

    // First run: one bounded round performs the trigger and reports a final cursor.
    let out = Command::new(bin())
        .args(["start", log.to_str().unwrap(), "0", "--max-rounds", "1"])
        .output()
        .expect("run cdz-agent start (first)");
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("runtime not found"),
            "the only acceptable failure is a missing runtime store (skip); got: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        eprintln!("[cdz-agent it] value-heap runtime absent; skipped the resume-cursor assertion");
        let _ = std::fs::remove_file(&log);
        let _ = std::fs::remove_file(&prog);
        return;
    }
    // Parse the reported resume cursor from "stopped cleanly at cursor N".
    let cursor: u64 = stdout
        .split("at cursor ")
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("start must report its final cursor; got: {stdout}"));
    assert!(
        cursor >= 2,
        "cursor advanced past genesis(0) + trigger(1); got {cursor}"
    );

    // Count prim-record events after the first run (the performed trigger's effect trail).
    let count_prim = || {
        let raw = std::fs::read(&log).expect("read log");
        String::from_utf8_lossy(&raw).matches("prim-").count()
    };
    let after_first = count_prim();
    assert!(
        after_first > 0,
        "the first run performed the trigger (recorded prim events)"
    );

    // RESTART with --from <cursor>: the already-processed trigger must NOT be re-performed.
    let out = Command::new(bin())
        .args([
            "start",
            log.to_str().unwrap(),
            "0",
            "--from",
            &cursor.to_string(),
            "--max-rounds",
            "1",
        ])
        .output()
        .expect("run cdz-agent start (resume)");
    assert!(
        out.status.success(),
        "the resumed daemon runs: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let after_resume = count_prim();
    assert_eq!(
        after_resume, after_first,
        "resuming with --from {cursor} re-performed NO history (prim-record count unchanged: {after_first} → {after_resume})"
    );

    let _ = std::fs::remove_file(&log);
    let _ = std::fs::remove_file(&prog);
}

#[test]
fn start_auto_resumes_from_the_recorded_cursor_without_an_explicit_from() {
    // Crash-recovery WITHOUT --from: after a round processes a trigger, the daemon durably records a
    // `daemon-cursor` high-water mark in the log. A RESTART with NO --from AUTO-RESUMES from that mark (so
    // recovery is automatic even after a crash that printed no cursor) and re-performs NOTHING. Store-gated skip.
    let log = unique("autoresume-log").with_extension("log");
    let prog = unique("autoresume-genesis").with_extension("cdz");
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
    Command::new(bin())
        .args(["emit", log.to_str().unwrap(), "trigger", "go"])
        .output()
        .expect("emit trigger");

    // First run performs the trigger AND records a daemon-cursor mark.
    let out = Command::new(bin())
        .args(["start", log.to_str().unwrap(), "0", "--max-rounds", "1"])
        .output()
        .expect("run cdz-agent start (first)");
    if !out.status.success() {
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("runtime not found"),
            "the only acceptable failure is a missing runtime store (skip); got: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        eprintln!("[cdz-agent it] value-heap runtime absent; skipped the auto-resume assertion");
        let _ = std::fs::remove_file(&log);
        let _ = std::fs::remove_file(&prog);
        return;
    }
    let raw = std::fs::read(&log).expect("read log");
    let text = String::from_utf8_lossy(&raw);
    assert!(
        text.contains("daemon-cursor"),
        "the first run recorded a daemon-cursor high-water mark"
    );
    let count_prim = || {
        let raw = std::fs::read(&log).expect("read log");
        String::from_utf8_lossy(&raw).matches("prim-").count()
    };
    let after_first = count_prim();
    assert!(after_first > 0, "the first run performed the trigger");

    // RESTART with NO --from: must auto-resume from the recorded cursor and re-perform nothing.
    let out = Command::new(bin())
        .args(["start", log.to_str().unwrap(), "0", "--max-rounds", "1"])
        .output()
        .expect("run cdz-agent start (auto-resume)");
    assert!(
        out.status.success(),
        "the auto-resumed daemon runs: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        count_prim(),
        after_first,
        "auto-resume (no --from) re-performed NO history — the recorded daemon-cursor was honored"
    );

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
