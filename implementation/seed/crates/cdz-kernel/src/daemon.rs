//! The daemon loop (minimal-kernel re-charter, rung KC-daemon — the "start the daemon" half).
//!
//! Operator fork-5: the CLI (1) bootstraps the log + injects genesis ([`crate::boot`]), then (2) STARTS THE
//! DAEMON. This module is (2): the daemon's per-event step — read the log, find the latest genesis `program`
//! ([`crate::boot::latest_program`], no hardcoded genesis), and drive an interpret turn on the landed event
//! ([`crate::kernel::run_interpret`], which compiles the program + runs it + executes its host-ops). The
//! daemon understands NO events; the `program` (Cadenza) decides everything. This ties the boot half (genesis
//! lookup) + the kernel half (compile+run) into the loop spine.
//!
//! [`tick`] is one step (pure over the [`crate::Log`] + the runtime store, testable with a `FileLog` + an
//! injected genesis); a real daemon calls it per landed event. The event-source loop (subscribe/poll the log
//! for new events, advance a cursor) is a thin driver on top — [`tick`] is the load-bearing per-event core.

use crate::Log;
use anyhow::{anyhow, Result};

/// The outcome of one daemon tick: the program that ran + the number of host-ops the interpret scheduled for
/// the event (via the counting executor — [`crate::kernel::run_interpret`]). A caller/loop records this; a
/// perform-executor variant would instead execute the ops (K1c). `None` program means the log has no genesis
/// yet (the CLI hasn't injected one — nothing to run).
#[derive(Debug, PartialEq, Eq)]
pub struct Tick {
    pub op_count: i64,
}

/// Run ONE daemon step for a landed event of kind `event_kind`: read the log tail, find the LATEST genesis
/// `program` source ([`crate::boot::latest_program`] — no hardcoded genesis, self-superseding), and drive an
/// interpret turn ([`crate::kernel::run_interpret`], which compiles the program to a provider + runs it via a
/// peer executor over the value-heap `runtime`). Returns the [`Tick`] outcome. Errors if the log has no
/// genesis program yet (the CLI must inject one first) or the interpret run fails. `runtime` is the value-heap
/// wasm (resolve via [`crate::kernel::find_runtime_for`] on the compiled program; a caller skips if absent).
pub fn tick(log: &impl Log, event_kind: i64, runtime: Vec<u8>) -> Result<Tick> {
    let program = latest_or_err(log)?;
    let run = crate::kernel::run_interpret(&program, event_kind, runtime)?;
    Ok(Tick {
        op_count: run.op_count,
    })
}

/// Run ONE daemon step that EXECUTES the ops (not just counts them): same genesis lookup as [`tick`], but
/// drives the PERFORMING executor ([`crate::kernel::run_interpret_performing`], K1c) so each host-op the
/// interpret schedules fires a real `Prim.run` perform (the exec/http/log primitives, in-program-mocked at
/// this rung). Returns the number of ops PERFORMED. This is the daemon's real execution step — the counting
/// [`tick`] is the projection/inspection variant; a live daemon uses this to actually run the plan. (Binding
/// `Prim` to the real host primitives instead of the mock is the K1c→host-primitive rung.)
pub fn tick_performing(log: &impl Log, event_kind: i64, runtime: Vec<u8>) -> Result<i64> {
    let program = latest_or_err(log)?;
    crate::kernel::run_interpret_performing(&program, event_kind, runtime)
}

/// The event kind prefix a performed host-op is RECORDED under in the log (`prim-exec`/`prim-http`/
/// `prim-append`). Each perform appends one such event carrying the op's String payload — the §2.3
/// recorded-effect trail (like `fold`'s `model-request`/`model-response`), so a re-fold replays the plan.
pub const PRIM_RECORD_PREFIX: &str = "prim-";

/// Run ONE daemon step that PERFORMS the ops via a REAL host primitive and RECORDS each perform in the log
/// (the K1c→host-primitive rung). Same genesis lookup as [`tick`], but drives the HOSTED executor
/// ([`crate::kernel::run_interpret_hosted`]): each scheduled `Prim.exec`/`Prim.http`/`Prim.append` is answered
/// by a real closure that (1) appends a `prim-<op>` event carrying the op's String payload to `log` (the
/// recorded-effect trail, like [`crate::fold::drive_one_turn`]), and (2) returns a per-op scalar the fold sums.
///
/// This first cut is a SAFE record-only primitive — it logs each perform, no subprocess/network. Live `exec`
/// (subprocess) / `http` (network) cross the capability boundary and must be gated behind Cedar authorization
/// (the charter's on-behalf-of model) before they run for real; this rung proves the daemon→hosted→real-
/// closure→log-event path with a primitive that only records. Returns the summed per-op result. `log` is shared
/// with the recording closure via `Arc<Mutex>` (the closure runs inside the wasmtime host call, must be
/// `Send + Sync + 'static`); the daemon is single-threaded so the lock is uncontended.
pub fn tick_hosted<L>(
    log: std::sync::Arc<std::sync::Mutex<L>>,
    event_kind: i64,
    runtime: Vec<u8>,
) -> Result<i64>
where
    L: Log + Send + 'static,
{
    let program = {
        let l = log.lock().map_err(|_| anyhow!("log mutex poisoned"))?;
        crate::boot::latest_program(&l.tail(0)?).ok_or_else(|| {
            anyhow!("no genesis program in the log (CLI must inject one before the daemon runs)")
        })?
    };
    // The real (record-only) primitive: append a `prim-<op>` event carrying the payload, return a per-op
    // scalar (Append→1, Exec→2, Http→3 — the same tags the mock used, so behavior is comparable). An append
    // failure can't surface through the `-> i64` host-op contract, so it's a best-effort record (a gap shows
    // as a missing event, not a wrong result), matching `fold::drive_one_turn`'s recording discipline.
    let prim = {
        let log = std::sync::Arc::clone(&log);
        move |op: &str, payload: String| -> i64 {
            if let Ok(mut l) = log.lock() {
                let _ = l.append(&format!("{PRIM_RECORD_PREFIX}{op}"), payload.as_bytes());
            }
            match op {
                "exec" => 2,
                "http" => 3,
                "append" => 1,
                _ => 0,
            }
        }
    };
    crate::kernel::run_interpret_hosted(&program, event_kind, runtime, prim)
}

/// The latest genesis `program` source in `log`, or a loud error if none (the CLI must inject one first).
fn latest_or_err(log: &impl Log) -> Result<String> {
    let events = log.tail(0)?;
    crate::boot::latest_program(&events).ok_or_else(|| {
        anyhow!("no genesis program in the log (CLI must inject one before the daemon runs)")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FileLog;

    fn temp_log() -> (std::path::PathBuf, FileLog) {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let p =
            std::env::temp_dir().join(format!("cdz-kernel-daemon-{}-{n}.log", std::process::id()));
        let _ = std::fs::remove_file(&p);
        (p.clone(), FileLog::open(&p).unwrap())
    }

    /// A genesis interpret program: kind==1 → a 2-op plan, else → 1-op. (Same shape as the kernel fixture — a
    /// branch-built (List HostOp) that escapes as a heap handle.)
    const GENESIS: &str = "(do \
        (type HostOp (Append String) (Exec String) (Http String) (Noop Int64)) \
        (def (interpret (: kind Int64) (: turn Int64)) \
          (if (= kind 1) (list (Append \"ack\") (Exec \"handle\")) (list (Noop 0)))) \
        (export interpret))";

    #[test]
    fn a_tick_with_no_genesis_is_a_loud_error() {
        // The daemon can't run before the CLI injects a genesis program — a loud error, not a silent no-op.
        let (path, log) = temp_log();
        // No runtime needed to reach the no-genesis error (it's before compile) — pass empty bytes.
        assert!(
            tick(&log, 1, Vec::new()).is_err(),
            "no genesis program in the log → the daemon errors"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_tick_reads_the_genesis_and_drives_an_interpret_turn() {
        // The full daemon step: inject a genesis, then tick — the daemon reads the LATEST program from the log
        // (no hardcoded genesis) and drives an interpret turn on the event, returning its op-count.
        let (path, mut log) = temp_log();
        crate::boot::inject_genesis(&mut log, GENESIS).unwrap();

        // Resolve the runtime the genesis program requires (skip if the store is absent/stale).
        let store_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let provider = match crate::kernel::compile_interpret_provider(GENESIS) {
            Ok(p) => p,
            Err(e) => panic!("genesis compiles: {e}"),
        };
        let Some(runtime) = crate::kernel::find_runtime_for(&provider, store_root) else {
            eprintln!(
                "[cdz-kernel::daemon] value-heap runtime absent/stale; skipping the tick run"
            );
            return;
        };

        let t = tick(&log, 1, runtime.clone()).expect("the daemon ticks over the genesis");
        assert_eq!(
            t.op_count, 2,
            "kind=1 → the genesis interpret schedules [Append, Exec] → 2"
        );
        let t0 = tick(&log, 9, runtime).expect("the daemon ticks for a non-message event");
        assert_eq!(t0.op_count, 1, "kind=9 → [Noop] → 1");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_later_genesis_supersedes_the_first_the_daemon_runs_the_latest() {
        // Self-modification through the daemon: a later `program` event supersedes the first, and the daemon
        // runs the LATEST — proving behavior evolves without a kernel change. (Uses op-count as the observable:
        // the updated genesis schedules a DIFFERENT plan for the same event.)
        let (path, mut log) = temp_log();
        crate::boot::inject_genesis(&mut log, GENESIS).unwrap();
        // An updated genesis: kind==1 now → a 1-op plan (Noop), so the same event schedules fewer ops.
        let updated = "(do \
            (type HostOp (Append String) (Exec String) (Http String) (Noop Int64)) \
            (def (interpret (: kind Int64) (: turn Int64)) (list (Noop 0))) \
            (export interpret))";
        crate::boot::inject_genesis(&mut log, updated).unwrap();

        let store_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let provider = match crate::kernel::compile_interpret_provider(updated) {
            Ok(p) => p,
            Err(e) => panic!("updated genesis compiles: {e}"),
        };
        let Some(runtime) = crate::kernel::find_runtime_for(&provider, store_root) else {
            eprintln!("[cdz-kernel::daemon] runtime absent/stale; skipping");
            return;
        };
        let t = tick(&log, 1, runtime).expect("the daemon runs the latest genesis");
        assert_eq!(
            t.op_count, 1,
            "the daemon ran the UPDATED genesis (kind=1 → [Noop] → 1), not the original (would be 2)"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tick_performing_executes_each_op_not_just_counts() {
        // The daemon's real execution step: tick_performing drives the PERFORMING executor, so each op the
        // genesis interpret schedules fires a real Prim.run perform whose per-op RESULT is summed (Append→1,
        // Exec→2 — the real exec/http/log shape, each call returns its own value). kind=1 → [Append, Exec] →
        // 1+2 = 3 (proves both ops PERFORMED and their results came back through the cross-fn fold — NOT a
        // count, which is `tick`'s job). Mirrors the kernel's `the_performing_executor_fires_a_real_effect…`.
        let (path, mut log) = temp_log();
        crate::boot::inject_genesis(&mut log, GENESIS).unwrap();
        let store_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let provider = match crate::kernel::compile_interpret_provider(GENESIS) {
            Ok(p) => p,
            Err(e) => panic!("genesis compiles: {e}"),
        };
        let Some(runtime) = crate::kernel::find_runtime_for(&provider, store_root) else {
            eprintln!("[cdz-kernel::daemon] runtime absent/stale; skipping tick_performing");
            return;
        };
        let performed =
            tick_performing(&log, 1, runtime).expect("the daemon executes the genesis's ops");
        assert_eq!(
            performed, 3,
            "kind=1 → [Append(1), Exec(2)] → each op fired a real perform + its result summed → 3"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tick_performing_without_a_genesis_is_a_loud_error() {
        let (path, log) = temp_log();
        assert!(
            tick_performing(&log, 1, Vec::new()).is_err(),
            "no genesis → tick_performing errors (like tick)"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tick_hosted_performs_via_a_real_primitive_and_records_each_op_in_the_log() {
        // The K1c→host-primitive rung: tick_hosted drives the HOSTED executor (Prim bound to a REAL host
        // closure), so each op is PERFORMED by the daemon's record-only primitive AND appended to the log as a
        // `prim-<op>` event (the recorded-effect trail, like fold's model-request/response). kind=1 → [Append
        // "ack", Exec "handle"] → append("ack")→1 + exec("handle")→2 → sum 3, and the log gains `prim-append`
        // (payload "ack") + `prim-exec` (payload "handle") IN ORDER. This is the daemon actually executing via
        // a real host primitive (record-only cut; live exec/http gate behind Cedar), not the in-program mock.
        use std::sync::{Arc, Mutex};
        let (path, mut log0) = temp_log();
        crate::boot::inject_genesis(&mut log0, GENESIS).unwrap();
        let store_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let provider = match crate::kernel::compile_interpret_provider(GENESIS) {
            Ok(p) => p,
            Err(e) => panic!("genesis compiles: {e}"),
        };
        let Some(runtime) = crate::kernel::find_runtime_for(&provider, store_root) else {
            eprintln!("[cdz-kernel::daemon] runtime absent/stale; skipping tick_hosted");
            let _ = std::fs::remove_file(&path);
            return;
        };
        let log = Arc::new(Mutex::new(log0));
        let performed = tick_hosted(Arc::clone(&log), 1, runtime)
            .expect("the daemon performs the genesis's ops via a real host primitive");
        assert_eq!(
            performed, 3,
            "kind=1 → append(\"ack\")→1 + exec(\"handle\")→2 → summed by the fold = 3"
        );
        // The recorded-effect trail: the log gained a `prim-append`/`prim-exec` event per performed op, in
        // list order, each carrying the op's String payload.
        let events = log.lock().unwrap().tail(0).unwrap();
        let prim_events: Vec<(String, String)> = events
            .iter()
            .filter(|e| e.kind.starts_with(PRIM_RECORD_PREFIX))
            .map(|e| {
                (
                    e.kind.clone(),
                    String::from_utf8_lossy(&e.payload).into_owned(),
                )
            })
            .collect();
        assert_eq!(
            prim_events,
            vec![
                ("prim-append".to_string(), "ack".to_string()),
                ("prim-exec".to_string(), "handle".to_string()),
            ],
            "each performed op was recorded in the log as prim-<op> with its payload, in order"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tick_hosted_without_a_genesis_is_a_loud_error() {
        use std::sync::{Arc, Mutex};
        let (path, log) = temp_log();
        assert!(
            tick_hosted(Arc::new(Mutex::new(log)), 1, Vec::new()).is_err(),
            "no genesis → tick_hosted errors (like tick/tick_performing)"
        );
        let _ = std::fs::remove_file(&path);
    }
}
