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
        // genesis interpret schedules fires a real Prim.run perform. kind=1 → [Append, Exec] → 2 performs.
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
            performed, 2,
            "kind=1 → [Append, Exec] → each op fired a real perform → 2"
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
}
