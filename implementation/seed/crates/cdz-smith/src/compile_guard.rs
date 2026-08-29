//! An in-process COMPILE-hang watchdog for the differential sweeps.
//!
//! The wasm RUN side is already wall-clock-bounded — `cdz_run` arms an epoch deadline, so a runaway
//! guest loop TRAPS (surfaces as [`crate::differential::Side::Trap`]) rather than spinning forever.
//! But the COMPILE side — `rcdzc::compile_component`, called IN-PROCESS by every in-process sweep
//! ([`crate::differential::differential`], [`crate::differential::lean_differential_sweep`],
//! [`crate::differential::run_ast_corpus_sweep`]) — is a native Rust call with NO guard: a compiler
//! non-termination (the S164/S179-class self-app const-eval loop that #5626 fixed, or any future
//! regression) would wedge the whole sweep indefinitely, and `catch_unwind` cannot interrupt a
//! runaway loop.
//!
//! The subprocess `cadenza_diff` sweep bounds the whole `cdz` call with `timeout -s KILL`, so it is
//! already covered (seq-203). This module gives the IN-PROCESS sweeps the same protection with a
//! watchdog THREAD — the exact pattern [`crate::driver::run`] uses for the fuzz loop: a sweep
//! [`install`]s the watchdog once, then every compile runs under [`guard`], which publishes the
//! program source + a deadline. If the deadline passes without the compile finishing, the compile
//! HUNG — the watchdog files a [`crate::finding::Category::Timeout`] finding for the published source
//! and `abort()`s (a wedged native thread cannot be killed) so the cron relaunches against a fresh
//! build, exactly as the fuzz-path watchdog does.
//!
//! [`guard`] is a NO-OP when the watchdog is not installed (unit tests, single-shot repro), so it is
//! safe to leave on the hot compile path unconditionally.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::finding::{Category, Finding, FindingStore};

/// The watchdog's shared view of the compile in flight (mirrors [`crate::driver`]'s `Progress`, but
/// keyed on the program SOURCE — the sweeps hold a source string, not a PRNG seed).
struct CompileWatch {
    /// The source of the compile in flight, so the watchdog can FILE it on a hang.
    current_source: Mutex<String>,
    /// Bumped after each guarded compile completes. If it stops advancing while a deadline passes,
    /// the current compile is wedged.
    heartbeat: AtomicU64,
    /// Deadline (ns since `epoch`) for the current compile; 0 = no compile in flight.
    deadline_ns: AtomicU64,
    /// The epoch the deadline/heartbeat clocks measure from.
    epoch: Instant,
    /// Where a hang-witness `Timeout` finding is filed.
    findings_dir: PathBuf,
    /// The compiler commit the finding is attributed to.
    commit: String,
    /// The per-compile wall-clock budget; a compile exceeding it is a hang.
    timeout: Duration,
}

static WATCH: OnceLock<Arc<CompileWatch>> = OnceLock::new();

/// The per-compile wall-clock budget for the in-process compile watchdog. A single small-program
/// `rcdzc` compile is milliseconds; this is a HANG threshold, not a latency target, so it is
/// deliberately generous (a legitimate compile never approaches it even under heavy host load) while
/// still bounding a true non-termination. Override with `CDZ_SMITH_COMPILE_TIMEOUT` (seconds; 0 keeps
/// the default rather than disabling — the whole point is that a hang cannot wedge the sweep).
pub fn compile_timeout() -> Duration {
    let secs = std::env::var("CDZ_SMITH_COMPILE_TIMEOUT")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|&s| s > 0)
        .unwrap_or(60);
    Duration::from_secs(secs)
}

/// Install the in-process compile watchdog ONCE for the current process, spawning its background
/// thread. Idempotent — a second call is a no-op (the first install's config wins), so a sweep can
/// call it unconditionally at its entry point. `findings_dir`/`commit` attribute a hang-witness
/// finding; `timeout` is the per-compile budget (see [`compile_timeout`]).
pub fn install(findings_dir: PathBuf, commit: String, timeout: Duration) {
    let _ = WATCH.get_or_init(|| {
        let watch = Arc::new(CompileWatch {
            current_source: Mutex::new(String::new()),
            heartbeat: AtomicU64::new(0),
            deadline_ns: AtomicU64::new(0),
            epoch: Instant::now(),
            findings_dir,
            commit,
            timeout,
        });
        spawn(watch.clone());
        watch
    });
}

/// Run `compile` under the installed watchdog: publish `source` + arm the deadline, run the compile,
/// then disarm and beat the heart (this compile finished in time). If the watchdog is NOT installed
/// (unit tests, `once`/repro), just runs `compile` unguarded — so this is safe to wrap every compile.
pub fn guard<T>(source: &str, compile: impl FnOnce() -> T) -> T {
    let Some(w) = WATCH.get() else {
        return compile();
    };
    // Publish what we're about to compile so the watchdog can file it on a hang.
    if let Ok(mut s) = w.current_source.lock() {
        s.clear();
        s.push_str(source);
    }
    let deadline = w.epoch.elapsed() + w.timeout;
    w.deadline_ns
        .store(deadline.as_nanos() as u64, Ordering::SeqCst);

    let out = compile();

    // Disarm the deadline and beat the heart: this compile finished in time.
    w.deadline_ns.store(0, Ordering::SeqCst);
    w.heartbeat.fetch_add(1, Ordering::SeqCst);
    out
}

/// The watchdog thread: if the armed deadline passes without the heartbeat advancing, the current
/// compile has hung — file a `Timeout` hang-witness for its source and abort the process so the cron
/// relaunches. Mirrors [`crate::driver`]'s `spawn_watchdog`.
fn spawn(watch: Arc<CompileWatch>) {
    std::thread::Builder::new()
        .name("cdz-smith-compile-watchdog".into())
        .spawn(move || {
            let mut last_beat = watch.heartbeat.load(Ordering::SeqCst);
            let mut last_beat_at = watch.epoch.elapsed();
            loop {
                std::thread::sleep(Duration::from_millis(200));
                let beat = watch.heartbeat.load(Ordering::SeqCst);
                if beat != last_beat {
                    last_beat = beat;
                    last_beat_at = watch.epoch.elapsed();
                    continue;
                }
                let deadline_ns = watch.deadline_ns.load(Ordering::SeqCst);
                if deadline_ns == 0 {
                    // No compile armed (between compiles); reset the stall timer.
                    last_beat_at = watch.epoch.elapsed();
                    continue;
                }
                let now = watch.epoch.elapsed();
                // Fire once the armed deadline is past AND we've genuinely stalled for a full budget.
                if now.as_nanos() as u64 > deadline_ns
                    && now.saturating_sub(last_beat_at) > watch.timeout
                {
                    let source = watch
                        .current_source
                        .lock()
                        .map(|s| s.clone())
                        .unwrap_or_default();
                    file_hang(&watch, source);
                    eprintln!(
                        "[cdz-smith] COMPILE HANG: rcdzc::compile_component exceeded {:?}; aborting so the cron restarts.",
                        watch.timeout
                    );
                    // We cannot safely unwind a wedged native thread — abort.
                    std::process::abort();
                }
            }
        })
        .expect("spawn compile watchdog thread");
}

/// File the hung program as a `Timeout` hang-witness (best-effort — we abort immediately after).
fn file_hang(watch: &CompileWatch, source: String) {
    if let Ok(store) = FindingStore::open(&watch.findings_dir) {
        let finding = Finding {
            category: Category::Timeout,
            program: source,
            crash: None,
            detail: Some("in-process compile HANG (rcdzc::compile_component watchdog)".to_string()),
            commit: watch.commit.clone(),
        };
        let _ = store.file(&finding);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Uninstalled, `guard` is a transparent pass-through — it runs the closure and returns its value
    /// with no watchdog (the unit-test / single-shot-repro path). This also documents that the compile
    /// path is safe to wrap unconditionally.
    #[test]
    fn guard_is_a_passthrough_when_uninstalled() {
        // NOTE: this test process never calls `install`, so `WATCH` is unset here.
        let mut ran = false;
        let out = guard("(do (def (main) 1) (export main))", || {
            ran = true;
            42
        });
        assert!(ran, "the closure must run");
        assert_eq!(out, 42, "guard returns the closure's value unchanged");
    }

    /// `compile_timeout` reads a sane default when the env override is absent (the fuzzer's normal
    /// path). The default is a HANG threshold, not a latency target — generous but finite.
    #[test]
    fn compile_timeout_has_a_finite_default() {
        // In the unit-test process `CDZ_SMITH_COMPILE_TIMEOUT` is not set, so this exercises the
        // default arm. (A `0`/garbage override is filtered back to the default so a hang can never
        // disable the guard by zeroing the budget — see the `.filter(|&s| s > 0)` in the source.)
        let t = compile_timeout();
        assert!(
            t >= Duration::from_secs(1) && t <= Duration::from_secs(600),
            "compile timeout is a finite hang threshold, got {t:?}"
        );
    }
}
