//! The continuous driver: generate → compile → classify → shrink → file, forever (or for a batch).
//!
//! # Seeds
//!
//! Each iteration draws a fresh seed from a splitmix64 PRNG (seeded from the wall clock + a counter,
//! or from a fixed value for reproducibility). The seed both *drives the generator* and *names the
//! run*: logging the seed of a finding lets anyone reproduce the exact program with
//! `program_for_seed(seed)`, independent of the compiler version.
//!
//! # Hang detection (the timeout oracle)
//!
//! A panic unwinds and is caught in-process; a runaway loop does NOT — `catch_unwind` never gets
//! control. So the driver arms a **watchdog**: before each compile it publishes the current
//! seed + a deadline to a shared cell and bumps a heartbeat; a background thread that sees the
//! deadline pass without a heartbeat advance concludes the compile hung, files a `Timeout` finding
//! (it has the seed, so it can regenerate the program), and aborts the process. The cron relaunches
//! against a fresh build. Aborting (rather than trying to kill the compile thread — Rust can't) is
//! the honest way to escape a wedged native thread.
//!
//! The `once` entry point below doesn't arm the watchdog; it is meant for reproducing a single
//! program and for a subprocess the operator can wrap in its own `timeout(1)`.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::finding::{Category, Finding, FindingStore};
use crate::generator::generate;
use crate::oracle::{CrashInfo, Verdict, compile_catching};

/// Configuration for a fuzzing run.
#[derive(Clone, Debug)]
pub struct Config {
    /// Stop after this many programs (`None` = run until the process is stopped).
    pub iterations: Option<u64>,
    /// PRNG seed for the run. A fixed value makes the whole batch reproducible.
    pub run_seed: u64,
    /// Per-compile wall-clock budget; a compile exceeding it is a `Timeout` finding.
    pub timeout: Duration,
    /// Where findings are written (the `failures/` directory).
    pub findings_dir: PathBuf,
    /// The compiler commit findings are attributed to.
    pub commit: String,
    /// Print a progress line every N iterations (0 = quiet).
    pub progress_every: u64,
}

impl Config {
    /// A batch run of `iterations` programs, discovering the failures dir from the working
    /// directory, seeded from the wall clock.
    pub fn batch(iterations: u64) -> std::io::Result<Self> {
        let cwd = std::env::current_dir()?;
        let store = FindingStore::discover(&cwd)?;
        Ok(Config {
            iterations: Some(iterations),
            run_seed: wallclock_seed(),
            timeout: Duration::from_secs(10),
            findings_dir: store.dir().to_path_buf(),
            commit: detect_commit(),
            progress_every: 1000,
        })
    }
}

/// Tallies for one run.
#[derive(Default, Debug, Clone)]
pub struct Stats {
    pub compiled: u64,
    pub declined: u64,
    pub parse_errors: u64,
    pub crashes: u64,
    pub invalid_wasm: u64,
    pub timeouts: u64,
    pub new_buckets: u64,
    pub duplicate_hits: u64,
}

impl Stats {
    pub fn total(&self) -> u64 {
        self.compiled
            + self.declined
            + self.parse_errors
            + self.crashes
            + self.invalid_wasm
            + self.timeouts
    }
}

// The watchdog's shared view of what the main loop is currently doing.
struct Progress {
    // The seed of the compile in flight (so the watchdog can regenerate it on a hang).
    current_seed: AtomicU64,
    // Monotonic heartbeat: bumped after every compile completes. If it stops advancing while a
    // deadline passes, the current compile is wedged.
    heartbeat: AtomicU64,
    // Deadline (ns since an epoch Instant) for the current compile; 0 = no compile in flight.
    deadline_ns: AtomicU64,
}

/// Run the fuzzer with the given config, arming the hang watchdog. Returns when the iteration
/// budget is exhausted (an unbounded run only returns if `iterations` is `Some`). On a hang the
/// process is aborted from the watchdog, so this does not return in that case.
pub fn run(cfg: &Config) -> std::io::Result<Stats> {
    let store = FindingStore::open(&cfg.findings_dir)?;
    let epoch = Instant::now();
    let progress = Arc::new(Progress {
        current_seed: AtomicU64::new(0),
        heartbeat: AtomicU64::new(0),
        deadline_ns: AtomicU64::new(0),
    });

    spawn_watchdog(progress.clone(), epoch, cfg.clone());

    // In-flight seed capture (opt-in via `CDZ_SMITH_INFLIGHT=<path>`): the watchdog can regenerate a
    // HANG's program from `progress.current_seed`, but a HARD abort (a compiler stack overflow / OOM —
    // SIGABRT, not an unwinding panic) bypasses both `catch_unwind` and the hang watchdog and takes the
    // whole process down WITHOUT filing anything, losing the culprit. When this path is set we write the
    // seed to it BEFORE each compile (overwritten every iteration), so after a hard abort the file holds
    // the exact seed that crashed — `cdz-smith gen <seed>` then reproduces it. Off by default (no I/O).
    let inflight = std::env::var_os("CDZ_SMITH_INFLIGHT");

    let mut stats = Stats::default();
    let mut rng = SplitMix64::new(cfg.run_seed);
    let mut i = 0u64;
    loop {
        if let Some(max) = cfg.iterations
            && i >= max
        {
            break;
        }
        let seed = rng.next();
        if let Some(path) = &inflight {
            let _ = std::fs::write(path, seed.to_string());
        }

        // Publish what we're about to do, then arm the deadline for the watchdog.
        progress.current_seed.store(seed, Ordering::SeqCst);
        let deadline = epoch.elapsed() + cfg.timeout;
        progress
            .deadline_ns
            .store(deadline.as_nanos() as u64, Ordering::SeqCst);

        let verdict = compile_seed(seed);

        // Disarm the deadline and beat the heart: this compile finished in time.
        progress.deadline_ns.store(0, Ordering::SeqCst);
        progress.heartbeat.fetch_add(1, Ordering::SeqCst);

        classify(&verdict, seed, cfg, &store, &mut stats);

        i += 1;
        if cfg.progress_every != 0 && i.is_multiple_of(cfg.progress_every) {
            eprintln!(
                "[cdz-smith] {i} programs | {} compiled, {} declined, {} crashes, {} invalid-wasm ({} buckets), {} timeouts",
                stats.compiled,
                stats.declined,
                stats.crashes,
                stats.invalid_wasm,
                stats.new_buckets,
                stats.timeouts
            );
        }
    }
    Ok(stats)
}

/// Generate a program from a seed and run it through the crash oracle.
fn compile_seed(seed: u64) -> Verdict {
    compile_catching(&program_for_seed(seed))
}

fn classify(verdict: &Verdict, seed: u64, cfg: &Config, store: &FindingStore, stats: &mut Stats) {
    match verdict {
        Verdict::Compiled { .. } => stats.compiled += 1,
        Verdict::Declined { .. } => stats.declined += 1,
        Verdict::ParseError(_) => stats.parse_errors += 1,
        Verdict::Crash(info) => {
            stats.crashes += 1;
            file_crash(seed, info, cfg, store, stats);
        }
        Verdict::InvalidWasm { detail, .. } => {
            stats.invalid_wasm += 1;
            file_invalid_wasm(seed, detail, cfg, store, stats);
        }
    }
}

/// File `finding` and tally the outcome: a New bucket bumps `new_buckets` and logs `NEW <label>
/// bucket → …`, a Duplicate bumps `dup_hits`, an error is logged. Shared by the crash/invalid-wasm
/// filers and the differential sweep (which count into `Stats` vs `DiffStats` fields respectively),
/// so the counters cross as `&mut u64` rather than a concrete stats type.
fn file_and_tally(
    store: &FindingStore,
    finding: &Finding,
    new_buckets: &mut u64,
    dup_hits: &mut u64,
    seed: u64,
    label: &str,
) {
    match store.file(finding) {
        Ok(crate::finding::Filed::New(path)) => {
            *new_buckets += 1;
            eprintln!(
                "[cdz-smith] NEW {label} bucket → {} (seed {seed})",
                path.display()
            );
        }
        Ok(crate::finding::Filed::Duplicate(_)) => *dup_hits += 1,
        Err(e) => eprintln!("[cdz-smith] failed to file finding: {e}"),
    }
}

fn file_crash(seed: u64, info: &CrashInfo, cfg: &Config, store: &FindingStore, stats: &mut Stats) {
    let raw = program_for_seed(seed);
    // Shrink while preserving the same crash site, so the filed reproducer is minimal.
    let target = info.site.as_deref().map(crate::finding::normalize_site);
    let program = crate::finding::shrink(&raw, target.as_deref());
    let finding = Finding {
        category: Category::Crash,
        program,
        crash: Some(info.clone()),
        detail: None,
        commit: cfg.commit.clone(),
    };
    file_and_tally(
        store,
        &finding,
        &mut stats.new_buckets,
        &mut stats.duplicate_hits,
        seed,
        "crash",
    );
}

fn file_invalid_wasm(
    seed: u64,
    detail: &str,
    cfg: &Config,
    store: &FindingStore,
    stats: &mut Stats,
) {
    let raw = program_for_seed(seed);
    // Shrink while preserving that the emitted component still fails to validate.
    let program = crate::finding::shrink_invalid_wasm(&raw);
    let finding = Finding {
        category: Category::InvalidWasm,
        program,
        crash: None,
        detail: Some(detail.to_string()),
        commit: cfg.commit.clone(),
    };
    file_and_tally(
        store,
        &finding,
        &mut stats.new_buckets,
        &mut stats.duplicate_hits,
        seed,
        "invalid-wasm",
    );
}

// ── the differential sweep (a SEPARATE, lower-cadence pass) ─────────────────────────────────────
//
// The differential oracle ([`crate::differential`]) shells `cdz run-rust`, which `rustc`-compiles
// every program — orders of magnitude slower than the in-process crash/validity oracles. So it is
// NOT run on the hot libFuzzer path (that would collapse throughput); instead it is a distinct sweep
// over a batch of seeds (or corpus programs), run at a lower cadence by the fuzz cycle. A mismatch is
// filed as a [`Category::Differential`] finding, shrunk to preserve the same disagreement.

/// Tallies for one differential sweep.
#[cfg(feature = "differential")]
#[derive(Default, Debug, Clone)]
pub struct DiffStats {
    /// Programs where both backends produced a comparable outcome and AGREED (incl. one-side declines).
    pub agreed: u64,
    /// Programs where the backends DISAGREED (a filed finding, modulo dedup).
    pub mismatched: u64,
    /// New buckets created this sweep.
    pub new_buckets: u64,
    /// Existing buckets re-hit this sweep.
    pub duplicate_hits: u64,
    /// Programs the oracle could not evaluate (e.g. `cdz run-rust` harness failure) — logged, skipped.
    pub unavailable: u64,
}

/// Run the differential oracle over `count` seeds drawn from `run_seed`, filing any mismatch. `store`
/// is the value-heap runtime store (for the wasm side); `cdz` is the `cdz` binary (for the rust side).
/// Findings land in `cfg.findings_dir` bucketed by disagreement. Returns the sweep tallies.
///
/// The FIRST `Unavailable` is not fatal (a single flaky spawn); but if EVERY program comes back
/// `Unavailable` the oracle is misconfigured (no runnable `cdz`, missing rlibs) — the caller should
/// notice `unavailable == count && agreed+mismatched == 0` and report the oracle as down rather than
/// trust a clean sweep.
#[cfg(feature = "differential")]
pub fn differential_sweep(
    cfg: &Config,
    store: &std::path::Path,
    cdz: &std::path::Path,
    count: u64,
) -> std::io::Result<DiffStats> {
    use crate::differential::{Diff, differential, shrink_differential};
    let fstore = FindingStore::open(&cfg.findings_dir)?;
    let mut stats = DiffStats::default();
    let mut rng = SplitMix64::new(cfg.run_seed);
    for i in 0..count {
        let seed = rng.next();
        let source = program_for_seed(seed);
        match differential(&source, store, cdz) {
            Diff::Agree => stats.agreed += 1,
            Diff::Unavailable(msg) => {
                stats.unavailable += 1;
                // Log only the first few to avoid flooding on a misconfigured oracle.
                if stats.unavailable <= 3 {
                    eprintln!("[cdz-smith] differential unavailable (seed {seed}): {msg}");
                }
            }
            Diff::Mismatch { kind, wasm, rust } => {
                stats.mismatched += 1;
                let shrunk = shrink_differential(&source, kind, store, cdz);
                let detail = format!("[{}] wasm={wasm} rust={rust}", kind.tag());
                let finding = Finding {
                    category: Category::Differential,
                    program: shrunk,
                    crash: None,
                    detail: Some(detail),
                    commit: cfg.commit.clone(),
                };
                let label = format!("differential ({} mismatch)", kind.tag());
                file_and_tally(
                    &fstore,
                    &finding,
                    &mut stats.new_buckets,
                    &mut stats.duplicate_hits,
                    seed,
                    &label,
                );
            }
        }
        if cfg.progress_every != 0 && (i + 1).is_multiple_of(cfg.progress_every) {
            eprintln!(
                "[cdz-smith] differential {}/{count} | {} agreed, {} mismatched ({} buckets), {} unavailable",
                i + 1,
                stats.agreed,
                stats.mismatched,
                stats.new_buckets,
                stats.unavailable
            );
        }
    }
    Ok(stats)
}

/// The watchdog thread: if the armed deadline passes without the heartbeat advancing, the current
/// compile has hung — file a timeout finding for its seed and abort the process.
fn spawn_watchdog(progress: Arc<Progress>, epoch: Instant, cfg: Config) {
    std::thread::Builder::new()
        .name("cdz-smith-watchdog".into())
        .spawn(move || {
            let mut last_beat = progress.heartbeat.load(Ordering::SeqCst);
            let mut last_beat_at = epoch.elapsed();
            loop {
                std::thread::sleep(Duration::from_millis(200));
                let beat = progress.heartbeat.load(Ordering::SeqCst);
                if beat != last_beat {
                    last_beat = beat;
                    last_beat_at = epoch.elapsed();
                    continue;
                }
                let deadline_ns = progress.deadline_ns.load(Ordering::SeqCst);
                if deadline_ns == 0 {
                    // No compile armed (we're between iterations); reset the stall timer.
                    last_beat_at = epoch.elapsed();
                    continue;
                }
                let now = epoch.elapsed();
                // Fire once the armed deadline is past AND we've genuinely stalled for a full budget.
                if now.as_nanos() as u64 > deadline_ns
                    && now.saturating_sub(last_beat_at) > cfg.timeout
                {
                    let seed = progress.current_seed.load(Ordering::SeqCst);
                    file_timeout(seed, &cfg);
                    eprintln!(
                        "[cdz-smith] TIMEOUT: compile of seed {seed} exceeded {:?}; aborting so the cron restarts.",
                        cfg.timeout
                    );
                    // We cannot safely unwind a wedged native thread — abort.
                    std::process::abort();
                }
            }
        })
        .expect("spawn watchdog thread");
}

fn file_timeout(seed: u64, cfg: &Config) {
    let program = program_for_seed(seed);
    if let Ok(store) = FindingStore::open(&cfg.findings_dir) {
        let finding = Finding {
            category: Category::Timeout,
            program,
            crash: None,
            detail: None,
            commit: cfg.commit.clone(),
        };
        let _ = store.file(&finding);
    }
}

/// Run a single seed and return its verdict (no watchdog, no filing) — for `once`/repro and tests.
pub fn once(seed: u64) -> Verdict {
    compile_seed(seed)
}

/// Regenerate the program source for a seed (for `verify`/repro tooling and the watchdog).
pub fn program_for_seed(seed: u64) -> String {
    // Widen the 8-byte seed into a longer generator byte string so the generator has material to
    // make interesting choices from; the repeat is deterministic in the seed.
    generate(&seed.to_le_bytes().repeat(8)).source
}

// ── PRNG + environment helpers ────────────────────────────────────────────────────────────────

/// splitmix64 — a tiny, fast, well-distributed PRNG. Deterministic given the run seed.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        SplitMix64 { state: seed }
    }
    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
}

/// A run seed from the wall clock (only used when the caller doesn't pin one). Public so the CLI's
/// `--seed`-less default reuses it rather than keeping a byte-identical twin.
pub fn wallclock_seed() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x1234_5678_9abc_def0)
}

/// Best-effort short commit of the compiler under test (for finding attribution). Reads
/// `CDZ_SMITH_COMMIT` if set (the cron passes it), else shells to `git`, else "unknown".
pub fn detect_commit() -> String {
    if let Ok(c) = std::env::var("CDZ_SMITH_COMMIT")
        && !c.trim().is_empty()
    {
        return c.trim().to_string();
    }
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix_is_deterministic_and_varied() {
        let mut a = SplitMix64::new(42);
        let mut b = SplitMix64::new(42);
        let xs: Vec<u64> = (0..5).map(|_| a.next()).collect();
        let ys: Vec<u64> = (0..5).map(|_| b.next()).collect();
        assert_eq!(xs, ys, "same seed → same stream");
        assert!(xs.windows(2).all(|w| w[0] != w[1]), "stream varies");
    }

    // `#[ignore]` by default: `run()` arms the watchdog, which calls `process::abort()` on a hang —
    // fine for the standalone fuzzer, but inside `cargo test` (which the gate/CI invoke) a compiler
    // change that made one of these programs loop would abort the whole test binary and wedge the
    // gate. Run it on demand as an integration smoke test:
    //   cargo test -p cdz-smith --lib a_short_bounded_run -- --ignored
    #[test]
    #[ignore = "runs the real loop incl. the aborting watchdog — invoke explicitly, see comment"]
    fn a_short_bounded_run_completes_and_accounts_for_every_verdict() {
        // A tiny in-process batch against a scratch findings dir. The point isn't to FIND a bug
        // (we can't rely on one existing on any given compiler), it's that a run of the whole loop
        // terminates, never panics itself, and every verdict is accounted for.
        let tmp = std::env::temp_dir().join(format!("cdz-smith-run-{}", std::process::id()));
        let cfg = Config {
            iterations: Some(200),
            run_seed: 0xDEADBEEF,
            timeout: Duration::from_secs(60),
            findings_dir: tmp.clone(),
            commit: "test".into(),
            progress_every: 0,
        };
        let stats = run(&cfg).unwrap();
        assert_eq!(stats.total(), 200, "every iteration is accounted for");
        // The generator only emits parseable text, so parse errors should be ~0.
        assert_eq!(stats.parse_errors, 0, "generator emitted unparseable text");
        std::fs::remove_dir_all(&tmp).ok();
    }
}
