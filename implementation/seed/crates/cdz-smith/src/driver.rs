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
    /// Programs where a compile/run HUNG (hit the per-call timeout) — captured as a hang-witness (seq-203).
    pub hangs: u64,
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

/// The CADENZA-BACKEND equivalence sweep (operator seq-184): for each generated program, compare the
/// DIRECT wasm value against the `--target cadenza` round-trip value ([`crate::cadenza_diff::cadenza_diff`]).
/// A divergence is a cadenza-backend miscompile — filed like a wasm-vs-rust differential finding. Uses a
/// fresh scratch dir per program (cleaned between iterations).
#[cfg(feature = "differential")]
pub fn cadenza_differential_sweep(
    cfg: &Config,
    store: &std::path::Path,
    cdz: &std::path::Path,
    count: u64,
) -> std::io::Result<DiffStats> {
    use crate::cadenza_diff::{CzDiff, cadenza_diff};
    let fstore = FindingStore::open(&cfg.findings_dir)?;
    let mut stats = DiffStats::default();
    let mut rng = SplitMix64::new(cfg.run_seed);
    let tmp = std::env::temp_dir().join(format!("cdz-smith-cadenza-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);
    for i in 0..count {
        let seed = rng.next();
        // Use the COERCING generator (astgen), NOT `program_for_seed` (the text MUTATOR): the mutator
        // produces many compiler-HANGING / declining programs (each then hits the per-cdz-call timeout,
        // making the sweep glacial + mostly non-comparable). The coercing grammar produces clean,
        // TERMINATING, value-comparable programs — the same shapes the lean-differential grades — so the
        // cadenza value-eq check runs fast + actually compares. Derive 64 entropy bytes from the seed.
        let mut ent = Vec::with_capacity(64);
        let mut r = SplitMix64::new(seed);
        for _ in 0..8 {
            ent.extend_from_slice(&r.next().to_le_bytes());
        }
        let source = crate::astgen::generate_coerced(&ent).source;
        // Fresh scratch for this program (avoid a stale artifact leaking across iterations).
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::create_dir_all(&tmp);
        match cadenza_diff(cdz, &source, store, &tmp) {
            CzDiff::Agree => stats.agreed += 1,
            CzDiff::Mismatch { direct, cadenza } => {
                stats.mismatched += 1;
                let detail = format!("cadenza-equiv: direct={direct} cadenza={cadenza}");
                let finding = Finding {
                    category: Category::Differential,
                    program: source.clone(),
                    crash: None,
                    detail: Some(detail),
                    commit: cfg.commit.clone(),
                };
                file_and_tally(
                    &fstore,
                    &finding,
                    &mut stats.new_buckets,
                    &mut stats.duplicate_hits,
                    seed,
                    "cadenza-equiv mismatch",
                );
            }
            // A HANG: the compiler/runtime hit the per-call timeout on this program (seq-203). CAPTURE it as
            // a hang-witness (persisted, deduped by bucket) so it is investigated, never silently skipped.
            CzDiff::Hang { at } => {
                stats.hangs += 1;
                let finding = Finding {
                    category: Category::Timeout,
                    program: source.clone(),
                    crash: None,
                    detail: Some(format!("cadenza-sweep HANG at {at} (per-call timeout)")),
                    commit: cfg.commit.clone(),
                };
                file_and_tally(
                    &fstore,
                    &finding,
                    &mut stats.new_buckets,
                    &mut stats.duplicate_hits,
                    seed,
                    "compiler HANG (hang-witness)",
                );
            }
        }
        if cfg.progress_every != 0 && (i + 1).is_multiple_of(cfg.progress_every) {
            eprintln!(
                "[cdz-smith] cadenza-differential {}/{count} | {} agreed, {} mismatched, {} hangs ({} buckets)",
                i + 1,
                stats.agreed,
                stats.mismatched,
                stats.hangs,
                stats.new_buckets,
            );
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(stats)
}

// ── the LEAN symbolic-equivalence cadenza sweep (S4b/T2) ────────────────────────────────────────
//
// The SYMBOLIC complement to `cadenza_differential_sweep` (the sampled value-eq net): for each program
// build an `(equiv <orig> <cadenza-roundtrip>)` trial ([`crate::cadenza_diff::equiv_trial_for`]) and let
// v-lean-oracle's oracle PROVE the cadenza round-trip preserves meaning for ALL inputs. A `(holds)` is a
// forall-inputs proof (far stronger than a sampled agree); a `(skip "equiv: normalized-but-different")` is
// a STRONG suspected cadenza-backend miscompile. We do NOT file on the symbolic skip alone — we CONFIRM it
// with the sampled `cadenza_diff` first (sound: the symbolic oracle's normal form can differ for a reason
// the runtime values don't), and file only a confirmed divergence.

/// How the oracle judged one `(equiv P P')` trial (see [`classify_equiv_verdict`]).
#[cfg(feature = "differential")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EquivClass {
    /// `(holds)` — PROVEN functionally equivalent for all inputs (the cadenza backend preserved meaning).
    Proven,
    /// `(skip "equiv: boundary: …")` — the oracle hit its incompleteness limit (let/match/collections/
    /// calls/recursion); degrade to the sampled cadenza-diff net. Not a bug.
    Boundary,
    /// `(skip "equiv: normalized-but-different")` — both sides fully normalized yet differ: a STRONG
    /// suspected cadenza-backend miscompile, to CONFIRM with a sampled run before filing.
    SuspectedDivergence,
    /// `(skip "…not (trial …)")` — the oracle predates the `(equiv …)` node (#5719); it cannot judge
    /// equiv trials at all. The sweep aborts rather than miscounting every trial as a boundary skip.
    StaleOracle,
}

/// Classify an oracle verdict for an `(equiv …)` trial. Pure — the routing brain of [`equiv_cadenza_sweep`],
/// unit-tested without an oracle. An `(equiv …)` trial should only ever yield `(holds)` / `(skip …)`; a
/// `Mismatch` is not part of the equiv protocol, so treat it conservatively as a suspected divergence.
#[cfg(feature = "differential")]
pub fn classify_equiv_verdict(v: &crate::lean::Verdict) -> EquivClass {
    use crate::lean::Verdict;
    match v {
        Verdict::Holds => EquivClass::Proven,
        Verdict::Skip(r) if r.contains("not (trial") => EquivClass::StaleOracle,
        Verdict::Skip(r) if r.contains("normalized-but-different") => {
            EquivClass::SuspectedDivergence
        }
        Verdict::Skip(_) => EquivClass::Boundary,
        // Not an equiv-protocol verdict — be conservative and treat as suspected (still sampled-confirmed).
        Verdict::Mismatch(_) => EquivClass::SuspectedDivergence,
    }
}

/// Tallies for one symbolic-equivalence cadenza sweep.
#[cfg(feature = "differential")]
#[derive(Default, Debug, Clone)]
pub struct EquivStats {
    /// Trials the oracle judged (comparable — a clean cadenza round-trip yielded an equiv trial).
    pub trials: u64,
    /// PROVEN equivalent for all inputs.
    pub proven: u64,
    /// Oracle-incompleteness skips (degrade to the sampled net).
    pub boundary: u64,
    /// Suspected divergences the sampled `cadenza_diff` CONFIRMED (filed as findings).
    pub confirmed_divergences: u64,
    /// Suspected divergences where the sampled values AGREED — a SYMBOLIC FALSE-POSITIVE (the oracle's
    /// normal forms differ but the runtime values match). v-lean-oracle's normalizer-incompleteness
    /// metric; the `(orig, program1)` pair is collected for their triage. Not filed.
    pub symbolic_false_positives: u64,
    /// Suspected divergences the sampled net could NOT compare (param'd main / skip / hang) — inherent,
    /// not a symbolic false-positive and not a divergence. Not filed.
    pub uncomparable_confirm: u64,
    /// Programs with no comparable equiv trial (source unparseable / cadenza round-trip declined or hung).
    pub not_comparable: u64,
    /// New finding buckets created this sweep.
    pub new_buckets: u64,
    /// Existing buckets re-hit.
    pub duplicate_hits: u64,
    /// Set if the oracle cannot judge `(equiv …)` (pre-#5719) — the sweep aborted early. A rebuilt
    /// `.#oracle-lean` is required.
    pub stale_oracle: bool,
}

/// Run the symbolic-equivalence cadenza sweep over `count` coerced programs: build an `(equiv <orig>
/// <cadenza-roundtrip>)` trial per program, judge in batches via `judge_batch_items`, and route each
/// verdict. A `SuspectedDivergence` is CONFIRMED with the sampled [`crate::cadenza_diff::cadenza_confirm`]
/// before filing (never file blind on the symbolic skip): a sampled DIVERGENCE is filed; sampled
/// VALUES-AGREE is a symbolic false-positive whose `(orig, program1)` render pair is pushed to
/// `false_positives` for v-lean-oracle's normalizer triage; sampled uncomparable is inherent.
/// `store` is the runtime store, `cdz` the binary (round-trip + confirm), `oracle` the equiv-aware oracle.
#[cfg(feature = "differential")]
pub fn equiv_cadenza_sweep(
    cfg: &Config,
    store: &std::path::Path,
    cdz: &std::path::Path,
    oracle: &std::path::Path,
    count: u64,
    false_positives: &mut Vec<(String, String)>,
    boundary_reasons: &mut Vec<String>,
) -> std::io::Result<EquivStats> {
    use crate::cadenza_diff::{ConfirmOutcome, cadenza_confirm, equiv_trial_for};
    use crate::lean::{BatchItem, judge_batch_items};

    let fstore = FindingStore::open(&cfg.findings_dir)?;
    let mut stats = EquivStats::default();
    let mut rng = SplitMix64::new(cfg.run_seed);
    let tmp = std::env::temp_dir().join(format!("cdz-smith-equiv-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);

    const BATCH: usize = 32;
    let mut batch_srcs: Vec<String> = Vec::new();
    let mut batch_trials: Vec<crate::lean::EquivTrial> = Vec::new();

    for i in 0..count {
        let seed = rng.next();
        // Coercing generator (same terminating, value-comparable grammar the sampled sweep uses).
        let mut ent = Vec::with_capacity(64);
        let mut r = SplitMix64::new(seed);
        for _ in 0..8 {
            ent.extend_from_slice(&r.next().to_le_bytes());
        }
        let source = crate::astgen::generate_coerced(&ent).source;

        // Fresh scratch per program; the round-trip writes p.sexp/mid.ast here.
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::create_dir_all(&tmp);
        match equiv_trial_for(cdz, &source, &tmp) {
            Some(trial) => {
                batch_trials.push(trial);
                batch_srcs.push(source);
            }
            None => stats.not_comparable += 1,
        }

        let last = i + 1 == count;
        if batch_trials.len() >= BATCH || (last && !batch_trials.is_empty()) {
            let items: Vec<BatchItem> =
                batch_trials.iter().cloned().map(BatchItem::Equiv).collect();
            let verdicts = judge_batch_items(oracle, &items)?;
            for ((src, trial), v) in batch_srcs.iter().zip(&batch_trials).zip(&verdicts) {
                match classify_equiv_verdict(v) {
                    EquivClass::StaleOracle => {
                        stats.stale_oracle = true;
                        eprintln!(
                            "[cdz-smith] equiv-cadenza: oracle predates the (equiv …) node (#5719) — \
                             rebuild `.#oracle-lean`; aborting sweep."
                        );
                        let _ = std::fs::remove_dir_all(&tmp);
                        return Ok(stats);
                    }
                    EquivClass::Proven => {
                        stats.trials += 1;
                        stats.proven += 1;
                    }
                    EquivClass::Boundary => {
                        stats.trials += 1;
                        stats.boundary += 1;
                        // Capture the oracle's skip REASON (carries the boundary category, e.g.
                        // "boundary: unmodeled head set" / "…recursion") so the caller can histogram the
                        // biggest cannotProve category — the coverage-prioritization signal v-lean-oracle wants.
                        if let crate::lean::Verdict::Skip(r) = v {
                            boundary_reasons.push(r.clone());
                        }
                    }
                    EquivClass::SuspectedDivergence => {
                        stats.trials += 1;
                        // CONFIRM with the sampled net before filing (sound: don't file on the symbolic
                        // skip alone). A fresh scratch for the confirm run.
                        let _ = std::fs::remove_dir_all(&tmp);
                        let _ = std::fs::create_dir_all(&tmp);
                        match cadenza_confirm(cdz, src, store, &tmp) {
                            ConfirmOutcome::Divergence { direct, cadenza } => {
                                stats.confirmed_divergences += 1;
                                let detail = format!(
                                    "cadenza-equiv (symbolic normalized-but-different, sampled-CONFIRMED): \
                                     direct={direct} cadenza={cadenza}"
                                );
                                let finding = Finding {
                                    category: Category::Differential,
                                    program: src.clone(),
                                    crash: None,
                                    detail: Some(detail),
                                    commit: cfg.commit.clone(),
                                };
                                file_and_tally(
                                    &fstore,
                                    &finding,
                                    &mut stats.new_buckets,
                                    &mut stats.duplicate_hits,
                                    0,
                                    "cadenza-equiv divergence (symbolic+sampled)",
                                );
                            }
                            // Sampled values AGREE while the oracle's normal forms differ = a SYMBOLIC
                            // FALSE-POSITIVE. Collect the (orig, program1) pair for v-lean-oracle's
                            // normalizer triage — render the round-trip AST as S-EXPR (the form the oracle
                            // consumes + the `.program1.sexp` extension implies), not surface syntax.
                            ConfirmOutcome::ValuesAgree => {
                                stats.symbolic_false_positives += 1;
                                let cadenza_render =
                                    cadenza_syntax::sexpr::print_pretty_width(&trial.cadenza, 100);
                                false_positives.push((src.clone(), cadenza_render));
                            }
                            // Sampled net couldn't compare (param'd main / skip / hang) — inherent.
                            ConfirmOutcome::Uncomparable | ConfirmOutcome::Hang => {
                                stats.uncomparable_confirm += 1;
                            }
                        }
                    }
                }
            }
            batch_trials.clear();
            batch_srcs.clear();
        }

        if cfg.progress_every != 0 && (i + 1).is_multiple_of(cfg.progress_every) {
            eprintln!(
                "[cdz-smith] equiv-cadenza {}/{count} | {} proven, {} boundary, {} confirmed-div, {} symbolic-fp, {} uncomparable, {} not-comparable",
                i + 1,
                stats.proven,
                stats.boundary,
                stats.confirmed_divergences,
                stats.symbolic_false_positives,
                stats.uncomparable_confirm,
                stats.not_comparable,
            );
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
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
    generate(&seed_entropy(seed)).source
}

/// Expand a `u64` seed into a DIVERSE generator byte stream via splitmix64. The generator makes one
/// choice per byte as it descends, so it needs a long stream of INDEPENDENT bytes to explore the shape
/// space; the previous `seed.to_le_bytes().repeat(8)` fed only 8 DISTINCT bytes cycled, which correlated
/// every choice and collapsed the whole sweep to ~136 distinct programs over thousands of seeds (the
/// "bottomed-out / tiny language subset" the operator flagged). splitmix64 gives 256 well-distributed
/// bytes — matching the diverse expansion the cadenza-differential / cadenza-equiv paths already use —
/// so a single-seed change now yields a genuinely different program. Deterministic in the seed. When the
/// generator wants more than 256 bytes its cursor bottoms out at 0 (bounded tail), same as before.
fn seed_entropy(seed: u64) -> Vec<u8> {
    let mut ent = Vec::with_capacity(256);
    let mut r = SplitMix64::new(seed);
    for _ in 0..32 {
        ent.extend_from_slice(&r.next().to_le_bytes());
    }
    ent
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

    /// The equiv verdict router (pure): `(holds)` → Proven; a boundary skip → Boundary (degrade to the
    /// sampled net); a `normalized-but-different` skip → SuspectedDivergence (sampled-confirm before
    /// filing); a stale-oracle `not (trial …)` skip → StaleOracle (abort); a stray `Mismatch` →
    /// conservatively SuspectedDivergence (still sampled-confirmed, so never a blind file).
    #[cfg(feature = "differential")]
    #[test]
    fn classify_equiv_verdict_routes_each_verdict() {
        use crate::lean::Verdict;
        assert_eq!(classify_equiv_verdict(&Verdict::Holds), EquivClass::Proven);
        assert_eq!(
            classify_equiv_verdict(&Verdict::Skip(
                "equiv: boundary: let/match not modeled".into()
            )),
            EquivClass::Boundary
        );
        assert_eq!(
            classify_equiv_verdict(&Verdict::Skip("equiv: normalized-but-different".into())),
            EquivClass::SuspectedDivergence
        );
        assert_eq!(
            classify_equiv_verdict(&Verdict::Skip("batch: node is not (trial …)".into())),
            EquivClass::StaleOracle
        );
        assert_eq!(
            classify_equiv_verdict(&Verdict::Mismatch("unexpected".into())),
            EquivClass::SuspectedDivergence
        );
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
