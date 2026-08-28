//! `cdz-smith` — the fuzzer CLI.
//!
//! The PRIMARY fuzzing path is coverage-guided libFuzzer via `cargo bolero` (see `tests/fuzz.rs` +
//! `fuzz-cycle.sh`); it drives the same `generate()` + oracle this binary uses. This binary provides
//! the surrounding tooling: the no-nightly PRNG fallback loop, single-seed reproduction, and the
//! adapter that turns libFuzzer's crash/timeout artifacts into deduped findings.
//!
//! Subcommands:
//!   fuzz             [--iterations N] [--seed S] [--timeout SECS] [--findings DIR]
//!                      the PRNG-driver fallback loop (no coverage; watchdog aborts on a hang).
//!   once             <SEED>            generate + compile one seed; print the verdict (no filing).
//!   gen              <SEED>            print the generated program source for a seed.
//!   verify           <FILE|SEED>       recompile a filed `.sexp` (or a seed's program); verdict.
//!   triage-artifacts <CRASHES_DIR>     convert a libFuzzer artifacts dir → deduped findings.
//!
//! Deliberately dependency-light arg parsing (no clap) so the fuzzer binary stays small and its
//! panic surface is just the compiler's.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use cdz_smith::driver::{self, Config};
use cdz_smith::oracle::{Verdict, compile_catching};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("fuzz");
    match cmd {
        "fuzz" => cmd_fuzz(&args[1..]),
        #[cfg(feature = "differential")]
        "differential" => cmd_differential(&args[1..]),
        #[cfg(not(feature = "differential"))]
        "differential" => {
            eprintln!(
                "cdz-smith: the `differential` subcommand needs the `differential` feature \
                 (rebuild: `cargo run --features differential -- differential …`)."
            );
            ExitCode::from(2)
        }
        "seed-corpus" => cmd_seed_corpus(&args[1..]),
        #[cfg(feature = "differential")]
        "lean-differential" => cmd_lean_differential(&args[1..]),
        #[cfg(not(feature = "differential"))]
        "lean-differential" => {
            eprintln!(
                "cdz-smith: the `lean-differential` subcommand needs the `differential` feature \
                 (it runs the wasm backend via cdz-run) — rebuild: \
                 `cargo run --features differential -- lean-differential …`."
            );
            ExitCode::from(2)
        }
        #[cfg(feature = "differential")]
        "run-ast-corpus" => cmd_run_ast_corpus(&args[1..]),
        #[cfg(not(feature = "differential"))]
        "run-ast-corpus" => {
            eprintln!(
                "cdz-smith: the `run-ast-corpus` subcommand needs the `differential` feature \
                 (it runs the wasm backend via cdz-run) — rebuild: \
                 `cargo run --features differential -- run-ast-corpus …`."
            );
            ExitCode::from(2)
        }
        #[cfg(feature = "differential")]
        "host-declines" => cmd_host_declines(&args[1..]),
        #[cfg(not(feature = "differential"))]
        "host-declines" => {
            eprintln!(
                "cdz-smith: the `host-declines` subcommand needs the `differential` feature \
                 (it shares the decline-capture helpers) — rebuild: \
                 `cargo run --features differential -- host-declines …`."
            );
            ExitCode::from(2)
        }
        #[cfg(feature = "differential")]
        "verify-differential" => cmd_verify_differential(&args[1..]),
        #[cfg(not(feature = "differential"))]
        "verify-differential" => {
            eprintln!(
                "cdz-smith: the `verify-differential` subcommand needs the `differential` feature \
                 (it runs the wasm + rust backends) — rebuild: \
                 `cargo run --features differential -- verify-differential …`."
            );
            ExitCode::from(2)
        }
        "once" => cmd_once(&args[1..]),
        "gen" => cmd_gen(&args[1..]),
        "verify" => cmd_verify(&args[1..]),
        "triage-artifacts" => cmd_triage_artifacts(&args[1..]),
        "-h" | "--help" | "help" => {
            usage();
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("cdz-smith: unknown subcommand `{other}`\n");
            usage();
            ExitCode::from(2)
        }
    }
}

fn usage() {
    eprintln!(
        "cdz-smith — fuzz the reference compiler\n\
         \n\
         USAGE:\n\
         \x20 cdz-smith fuzz             [--iterations N] [--seed S] [--timeout SECS] [--findings DIR]\n\
         \x20 cdz-smith differential     [--count N] [--seed S] [--findings DIR] [--store DIR] [--cdz PATH]\n\
         \x20 cdz-smith seed-corpus      [--semantics DIR] [--out DIR]\n\
         \x20 cdz-smith run-ast-corpus   [--seeds DIR] [--store DIR]   (needs --features differential)\n\
         \x20 cdz-smith lean-differential [--count N] [--seed S] [--store DIR] [--oracle PATH] [--findings DIR] [--declines-dir DIR]\n\
         \x20 cdz-smith verify-differential <FILE.sexp | SEED> [--store DIR] [--cdz PATH] [--oracle PATH]\n\
         \x20 cdz-smith host-declines     [--count N] [--seed S] [--declines-dir DIR]   (WIT/host gap hunt → breaker)\n\
         \x20 cdz-smith once             <SEED>\n\
         \x20 cdz-smith gen              <SEED>\n\
         \x20 cdz-smith verify           <FILE.sexp | SEED>\n\
         \x20 cdz-smith triage-artifacts <CRASHES_DIR> [--findings DIR] [--commit SHA]\n"
    );
}

/// The LEAN L2 differential sweep (S4b): generate terminating programs, run each under the WASM backend,
/// and judge (program + rcdzc output) batches with `oracle-check --batch-stream` (the Lean oracle as a
/// 3rd differential Side). A `mismatch` — the oracle's value/trap disagrees with rcdzc's — is a candidate
/// miscompile, filed as a `Differential` finding. `--count` programs (default 500), `--seed` (else
/// wall-clock), `--store` (runtime store; default beside the cdz target), `--oracle` (the `oracle-check`
/// binary; else `CDZ_SMITH_ORACLE_CHECK` / PATH — `nix build .#oracle-lean`), `--findings`. Exits 1 on
/// any mismatch.
#[cfg(feature = "differential")]
fn cmd_lean_differential(args: &[String]) -> ExitCode {
    let mut count: u64 = 500;
    let mut seed: Option<u64> = None;
    let mut store: Option<PathBuf> = None;
    let mut oracle: Option<PathBuf> = None;
    let mut findings: Option<PathBuf> = None;
    let mut declines_dir: Option<PathBuf> = None;

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--count" | "-n" => count = it.next().and_then(|s| s.parse().ok()).unwrap_or(count),
            "--seed" => seed = it.next().and_then(|s| parse_seed(s)),
            "--store" => store = it.next().map(PathBuf::from),
            "--oracle" => oracle = it.next().map(PathBuf::from),
            "--findings" => findings = it.next().map(PathBuf::from),
            "--declines-dir" => declines_dir = it.next().map(PathBuf::from),
            other => {
                eprintln!("cdz-smith lean-differential: unexpected arg `{other}`");
                return ExitCode::from(2);
            }
        }
    }

    // Resolve the oracle-check binary (flag > env/PATH). Without it the differential can't run.
    let oracle = match oracle
        .filter(|p| p.is_file())
        .or_else(cdz_smith::lean::discover_oracle_check)
    {
        Some(p) => p,
        None => {
            eprintln!(
                "cdz-smith lean-differential: no `oracle-check` found (build it — `nix build .#oracle-lean` \
                 → result/bin/oracle-check — and set CDZ_SMITH_ORACLE_CHECK or pass --oracle PATH)."
            );
            return ExitCode::FAILURE;
        }
    };
    let store = store.unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(4)
            .map(|repo| repo.join("target").join("cadenza-store"))
            .unwrap_or_else(|| PathBuf::from("target/cadenza-store"))
    });
    let findings_dir = match resolve_findings_dir(findings) {
        Ok(d) => d,
        Err(code) => return code,
    };
    let run_seed = seed.unwrap_or_else(driver::wallclock_seed);
    let commit = driver::detect_commit();

    // Generate `count` programs from varied entropy via the COERCING generator (astgen): every input
    // maps to a valid, type-correct Int64/compound program, so ~all trials are COMPARABLE by the oracle
    // (vs generator.rs's text grammar, which declines ~91% → not-comparable). Terminating by
    // construction, so the in-process wasm run cannot hang. This is the operator's directed mechanism —
    // coerce entropy → valid program — driving the differential densely over Lean's comparable domain.
    let mut rng = run_seed;
    let mut sources = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let mut bytes = Vec::with_capacity(24);
        for _ in 0..24 {
            rng = rng.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = rng;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            bytes.push(((z ^ (z >> 31)) >> 24) as u8);
        }
        sources.push(cdz_smith::astgen::generate_coerced(&bytes).source);
    }

    eprintln!(
        "[cdz-smith] lean-differential @{commit} | {count} programs | seed {run_seed} | store {} | oracle {} | findings → {}",
        store.display(),
        oracle.display(),
        findings_dir.display()
    );
    let mut mismatches: Vec<(String, String)> = Vec::new();
    let mut declines: Vec<(String, String)> = Vec::new();
    let stats = match cdz_smith::differential::lean_differential_sweep(
        &sources,
        &store,
        &oracle,
        200,
        &mut mismatches,
        &mut declines,
    ) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cdz-smith lean-differential: oracle run failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Bubble DECLINES up for the breaker decline→corpus hand-off (operator directive): dedup by signature
    // (CDZ code / normalized reason), then write ONE minimal repro `.sexp` + `.reason.txt` per distinct
    // gap into `--declines-dir`. Declines are EXPECTED output (never a bug) — this is a GAP inventory.
    let distinct_declines = if let Some(dir) = &declines_dir {
        match write_declines(dir, &declines) {
            Ok(n) => {
                eprintln!(
                    "[cdz-smith] {} declines ({} distinct) → {} (hand off to breaker)",
                    declines.len(),
                    n,
                    dir.display()
                );
                n
            }
            Err(e) => {
                eprintln!("cdz-smith lean-differential: cannot write declines dir: {e}");
                0
            }
        }
    } else {
        0
    };
    let _ = distinct_declines;

    // File each mismatch as a Differential finding.
    let mut new_buckets = 0usize;
    if !mismatches.is_empty() {
        match cdz_smith::FindingStore::open(&findings_dir) {
            Ok(store) => {
                for (source, detail) in &mismatches {
                    let finding = cdz_smith::Finding {
                        category: cdz_smith::Category::Differential,
                        program: source.clone(),
                        crash: None,
                        detail: Some(format!("lean-differential: {detail}")),
                        commit: commit.clone(),
                    };
                    if let Ok(cdz_smith::finding::Filed::New(path)) = store.file(&finding) {
                        new_buckets += 1;
                        eprintln!(
                            "[cdz-smith] FILED differential finding → {}",
                            path.display()
                        );
                    }
                }
            }
            Err(e) => eprintln!("cdz-smith lean-differential: cannot open findings dir: {e}"),
        }
    }

    eprintln!(
        "[cdz-smith] lean-differential done: {} trials | {} holds, {} mismatch ({} new buckets), {} skip | {} not-comparable",
        stats.trials, stats.holds, stats.mismatches, new_buckets, stats.skips, stats.not_comparable
    );
    if stats.mismatches > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// The HOST-DECLINE sweep: generate `count` HOST/EFFECT programs (see `hostgen`) and compile each,
/// collecting the compiler's DECLINES — the WIT/host-boundary GAPS the operator wants bubbled to breaker.
/// This is a pure decline hunt: it uses only the crash/decline oracle ([`compile_catching`]) — no wasm run,
/// no Lean oracle — so a `--declines-dir` writes deduped minimal repros exactly like the differential path.
/// `--count` (default 2000), `--seed` (else wall-clock), `--declines-dir` (the breaker hand-off dir). Feature-
/// gated with the other campaign subcommands only to share `write_declines`/`decline_signature` (it runs no
/// wasm itself). Exits 0 always (declines are EXPECTED output, never a finding).
#[cfg(feature = "differential")]
fn cmd_host_declines(args: &[String]) -> ExitCode {
    let mut count: u64 = 2000;
    let mut seed: Option<u64> = None;
    let mut declines_dir: Option<PathBuf> = None;

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--count" | "-n" => count = it.next().and_then(|s| s.parse().ok()).unwrap_or(count),
            "--seed" => seed = it.next().and_then(|s| parse_seed(s)),
            "--declines-dir" => declines_dir = it.next().map(PathBuf::from),
            other => {
                eprintln!("cdz-smith host-declines: unexpected arg `{other}`");
                return ExitCode::from(2);
            }
        }
    }

    let run_seed = seed.unwrap_or_else(driver::wallclock_seed);
    eprintln!(
        "[cdz-smith] host-declines @{} | {count} programs | seed {run_seed}",
        driver::detect_commit()
    );

    // Coerce varied entropy → host/effect programs; compile each; collect DECLINES as (source, RAW reason).
    let mut rng = run_seed;
    let mut declines: Vec<(String, String)> = Vec::new();
    let mut compiled = 0usize;
    let mut declined = 0usize;
    let mut other = 0usize;
    for _ in 0..count {
        let mut bytes = Vec::with_capacity(12);
        for _ in 0..12 {
            rng = rng.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = rng;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            bytes.push(((z ^ (z >> 31)) >> 24) as u8);
        }
        let program = cdz_smith::hostgen::generate_host(&bytes);
        match compile_catching(&program.source) {
            Verdict::Compiled { .. } => compiled += 1,
            Verdict::Declined { code, message } => {
                declined += 1;
                // RAW reason (breaker routes on the actual error): the CDZ code (if any) + full message,
                // so `decline_signature` groups coded declines by code and uncoded host gaps by prefix.
                let reason = match code {
                    Some(c) => format!("{c}: {message}"),
                    None => message,
                };
                declines.push((program.source, reason));
            }
            // A crash / invalid-wasm on a host program IS a bug — but the standalone fuzz/differential
            // targets own that; here we only inventory declines. Count + note; do not file.
            other_v => {
                other += 1;
                eprintln!("[cdz-smith] host-declines: non-decline outcome {other_v:?}");
            }
        }
    }

    let distinct = if let Some(dir) = &declines_dir {
        match write_declines(dir, &declines) {
            Ok(n) => {
                eprintln!(
                    "[cdz-smith] {declined} declines ({n} distinct new/updated) → {} (hand off to breaker)",
                    dir.display()
                );
                n
            }
            Err(e) => {
                eprintln!("cdz-smith host-declines: cannot write declines dir: {e}");
                0
            }
        }
    } else {
        0
    };
    eprintln!(
        "[cdz-smith] host-declines done: {compiled} compiled, {declined} declined, {other} other | {distinct} distinct decline signatures"
    );
    ExitCode::SUCCESS
}

/// Decline signatures BREAKER has already TRIAGED (legitimate compile-time rejections or tracked frontier
/// gaps — not bugs) and is tracking/flip-watching — so we stop re-surfacing them in the hand-off:
/// * `CDZ0304` — a const out-of-range shift count (must be `0..=63`); correctly rejected. Corpus-pinned
///   by breaker in #4895 (`(<< 5 -1)` → CDZ0304).
/// * `delegating-more-than-one-host-effect` — multi-interface host delegation is a not-yet-emitted
///   frontier gap (one interface per envelope); breaker-tracked + flip-watched.
/// * `the-host-operation-has-a-result` — a compound/Bytes/String host RESULT on a bare `(effect …)` has
///   no component-boundary form emitted yet (crosses on the world-driven path); breaker-tracked.
/// * `the-host-operation-has-an-argument` — a compound (Tuple/List) host ARGUMENT on a bare effect has no
///   component-boundary form yet (arg-side face of the result gap); breaker-tracked + flip-watched.
/// * `a-closures-parameter-type-has-no` — a higher-order (closure) host ARGUMENT: its parameter type has
///   no machine representation; breaker-tracked + flip-watched (flips when callback/resource support lands).
///
/// (Breaker triage notes, 2026-08-28.)
#[cfg(feature = "differential")]
const FILTERED_DECLINE_SIGNATURES: &[&str] = &[
    "CDZ0304",
    "delegating-more-than-one-host-effect",
    "the-host-operation-has-a-result",
    "the-host-operation-has-an-argument",
    "a-closures-parameter-type-has-no",
];

/// Dedup declines by signature and write ONE minimal (shortest) repro `.sexp` + `.reason.txt` per
/// distinct signature into `dir` — the breaker decline→corpus gap hand-off producer. Keeps the shortest
/// repro seen for each signature ACROSS runs (skips overwriting when an existing repro is already no
/// longer), so the dir accumulates a minimal repro per distinct gap. Returns the count of distinct
/// signatures. Declines are EXPECTED output (never a bug) — this is a tracked gap inventory for breaker.
#[cfg(feature = "differential")]
fn write_declines(dir: &std::path::Path, declines: &[(String, String)]) -> std::io::Result<usize> {
    use std::collections::HashMap;
    if declines.is_empty() {
        return Ok(0);
    }
    std::fs::create_dir_all(dir)?;
    // Shortest source + its reason per signature (this run).
    let mut best: HashMap<String, (&str, &str)> = HashMap::new();
    for (src, reason) in declines {
        // Skip HARNESS declines (not compiler gaps): a `cdz-run` "runtime … not in store" is a missing
        // value-heap-runtime blob (we sweep with `--store /nonexistent` for speed), and its
        // content-hash makes every instance a UNIQUE signature — it would flood breaker with non-gaps.
        // Breaker wants real front-end/backend declines (CDZ-coded / "not lowered"), so drop store misses.
        if reason.contains("not in store") {
            continue;
        }
        let sig = cdz_smith::differential::decline_signature(reason);
        // Skip signatures breaker already triaged as legitimate + corpus-pinned (not gaps).
        if FILTERED_DECLINE_SIGNATURES.contains(&sig.as_str()) {
            continue;
        }
        best.entry(sig)
            .and_modify(|e| {
                if src.len() < e.0.len() {
                    *e = (src, reason);
                }
            })
            .or_insert((src, reason));
    }
    for (sig, (src, reason)) in &best {
        let sexp = dir.join(format!("decline-{sig}.smith.sexp"));
        // Keep the globally-shortest repro: skip if an existing one is already no longer than this.
        if let Ok(existing) = std::fs::read_to_string(&sexp)
            && existing.trim().len() <= src.trim().len()
        {
            continue;
        }
        std::fs::write(&sexp, format!("{}\n", src.trim()))?;
        std::fs::write(
            dir.join(format!("decline-{sig}.reason.txt")),
            format!("{}\n", reason.trim()),
        )?;
    }
    Ok(best.len())
}

/// The DIFFERENTIAL sweep: run `count` seeds through BOTH backends and file any value disagreement.
/// A separate, lower-cadence pass (it `rustc`-compiles each program via `cdz run-rust`, far slower
/// than the in-process oracles), so it is NOT part of `fuzz`. `--store` overrides the runtime store
/// (default: the workspace `target/cadenza-store`); `--cdz` overrides the `cdz` binary (default:
/// `CDZ_SMITH_CDZ` or a discovered `target/{release,debug}/cdz`). If no `cdz` is found the sweep
/// cannot run and exits FAILURE without filing anything.
#[cfg(feature = "differential")]
fn cmd_differential(args: &[String]) -> ExitCode {
    let mut count: u64 = 1000;
    let mut seed: Option<u64> = None;
    let mut findings: Option<PathBuf> = None;
    let mut store: Option<PathBuf> = None;
    let mut cdz: Option<PathBuf> = None;

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--count" | "-n" => count = it.next().and_then(|s| s.parse().ok()).unwrap_or(count),
            "--seed" => seed = it.next().and_then(|s| parse_seed(s)),
            "--findings" => findings = it.next().map(PathBuf::from),
            "--store" => store = it.next().map(PathBuf::from),
            "--cdz" => cdz = it.next().map(PathBuf::from),
            other => {
                eprintln!("cdz-smith differential: unexpected arg `{other}`");
                return ExitCode::from(2);
            }
        }
    }

    // Resolve the `cdz` binary for the rust side (flag > env/discovery). Without it the oracle can't run.
    let cdz = match cdz
        .filter(|p| p.is_file())
        .or_else(cdz_smith::differential::discover_cdz)
    {
        Some(p) => p,
        None => {
            eprintln!(
                "cdz-smith differential: no `cdz` binary found (build it — `cargo build --release --bin cdz` — \
                 or set CDZ_SMITH_CDZ / pass --cdz PATH). Its dir must also hold libcdz_rt/libcdz_num rlibs."
            );
            return ExitCode::FAILURE;
        }
    };
    // Resolve the runtime store (flag > default beside the workspace target).
    let store = store.unwrap_or_else(|| {
        cdz.parent()
            .and_then(|p| p.parent()) // target/<profile>/cdz → target/
            .map(|t| t.join("cadenza-store"))
            .unwrap_or_else(|| PathBuf::from("target/cadenza-store"))
    });

    let findings_dir = match resolve_findings_dir(findings) {
        Ok(d) => d,
        Err(code) => return code,
    };

    let cfg = Config {
        iterations: Some(count),
        run_seed: seed.unwrap_or_else(driver::wallclock_seed),
        timeout: Duration::from_secs(10),
        findings_dir: findings_dir.clone(),
        commit: driver::detect_commit(),
        progress_every: 100,
    };

    eprintln!(
        "[cdz-smith] differential @{} | seed {} | count {} | store {} | cdz {} | findings → {}",
        cfg.commit,
        cfg.run_seed,
        count,
        store.display(),
        cdz.display(),
        findings_dir.display()
    );
    match driver::differential_sweep(&cfg, &store, &cdz, count) {
        Ok(stats) => {
            eprintln!(
                "[cdz-smith] differential done: {} agreed, {} mismatched ({} new buckets, {} dup hits), {} unavailable",
                stats.agreed,
                stats.mismatched,
                stats.new_buckets,
                stats.duplicate_hits,
                stats.unavailable
            );
            // Every program unavailable = the oracle never actually compared anything (misconfigured).
            if stats.agreed + stats.mismatched == 0 && stats.unavailable == count {
                eprintln!(
                    "cdz-smith differential: oracle never ran (all {count} unavailable) — check `cdz run-rust` + rlibs"
                );
                return ExitCode::FAILURE;
            }
            // A new bucket exits non-zero so a cron wrapper can notice a fresh miscompile.
            if stats.new_buckets > 0 {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            eprintln!("cdz-smith: differential sweep failed: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Triage ONE reproducer through the differential, in-process: run it under the wasm backend and the
/// rust backend, print each `Side` + the wasm-vs-rust `Diff`, and (with `--oracle`) also judge it with
/// the Lean L2 oracle. This is the per-source counterpart to the `differential` / `lean-differential`
/// sweeps — the tool for triaging a filed finding (or any program) WITHOUT re-running a whole campaign.
/// Exits non-zero iff a disagreement fired (wasm-vs-rust mismatch or a Lean mismatch).
#[cfg(feature = "differential")]
fn cmd_verify_differential(args: &[String]) -> ExitCode {
    let mut file: Option<String> = None;
    let mut store: Option<PathBuf> = None;
    let mut cdz: Option<PathBuf> = None;
    let mut oracle: Option<PathBuf> = None;

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--store" => store = it.next().map(PathBuf::from),
            "--cdz" => cdz = it.next().map(PathBuf::from),
            "--oracle" => oracle = it.next().map(PathBuf::from),
            other if !other.starts_with('-') && file.is_none() => file = Some(other.to_string()),
            other => {
                eprintln!("cdz-smith verify-differential: unexpected arg `{other}`");
                return ExitCode::from(2);
            }
        }
    }

    let Some(arg) = file else {
        eprintln!("cdz-smith verify-differential: expected a FILE.sexp or a SEED");
        return ExitCode::from(2);
    };
    let source = match resolve_source(&arg, "verify-differential") {
        Ok(s) => s,
        Err(code) => return code,
    };

    // The `cdz` binary (rust side) is OPTIONAL: findings from `lean-differential` only exercise the
    // in-process wasm side + the Lean oracle, and `cdz` (+ its rlibs) is a heavy separate build. If it
    // is not discoverable we still run the wasm side + Lean judge and just report the rust side as
    // unavailable — so the tool works for the common (wasm/Lean) triage case.
    let cdz = cdz
        .filter(|p| p.is_file())
        .or_else(cdz_smith::differential::discover_cdz);
    let store = store.unwrap_or_else(|| {
        cdz.as_ref()
            .and_then(|c| c.parent())
            .and_then(|p| p.parent())
            .map(|t| t.join("cadenza-store"))
            .unwrap_or_else(|| PathBuf::from("target/cadenza-store"))
    });

    use cdz_smith::differential::{Diff, compare, run_rust, run_wasm};
    let wasm = run_wasm(&source, &store);
    println!("wasm: {}", describe_side(&wasm));
    let mut disagreed = false;
    match &cdz {
        None => println!(
            "rust: <unavailable> no `cdz` binary (build `cargo build --release --bin cdz` or pass --cdz); \
             skipping wasm-vs-rust"
        ),
        Some(cdz) => match run_rust(cdz, &source) {
            Ok(rust) => {
                println!("rust: {}", describe_side(&rust));
                match compare(&wasm, &rust) {
                    Diff::Agree => println!("wasm-vs-rust: AGREE"),
                    Diff::Mismatch { kind, wasm, rust } => {
                        disagreed = true;
                        println!(
                            "wasm-vs-rust: MISMATCH [{}] — wasm={wasm} rust={rust}",
                            kind.tag()
                        );
                    }
                    Diff::Unavailable(e) => println!("wasm-vs-rust: UNAVAILABLE — {e}"),
                }
            }
            Err(e) => println!("rust: <unavailable> {e}"),
        },
    }

    // With --oracle, also judge the single program with the Lean L2 oracle (a 1-trial batch).
    if let Some(oracle) = oracle {
        if !oracle.is_file() {
            println!(
                "lean: <skipped> --oracle {} is not a file",
                oracle.display()
            );
        } else {
            let mut mm: Vec<(String, String)> = Vec::new();
            let mut dcl: Vec<(String, String)> = Vec::new();
            match cdz_smith::differential::lean_differential_sweep(
                std::slice::from_ref(&source),
                &store,
                &oracle,
                1,
                &mut mm,
                &mut dcl,
            ) {
                Ok(s) if s.mismatches > 0 => {
                    disagreed = true;
                    let detail = mm.first().map(|(_, d)| d.as_str()).unwrap_or("");
                    println!("lean: MISMATCH — {detail}");
                }
                Ok(s) if s.holds > 0 => println!("lean: HOLDS"),
                Ok(s) if s.skips > 0 => {
                    println!("lean: SKIP (a construct the oracle does not model)")
                }
                Ok(_) => println!("lean: NOT-COMPARABLE (no comparable wasm output)"),
                Err(e) => println!("lean: <error> {e}"),
            }
        }
    }

    if disagreed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// A short human label for a [`cdz_smith::differential::Side`] (the module's own `describe_side` is
/// private).
#[cfg(feature = "differential")]
fn describe_side(s: &cdz_smith::differential::Side) -> String {
    use cdz_smith::differential::Side;
    match s {
        Side::Value(v) => format!("value {v}"),
        Side::Trap(t) => format!("trap {t}"),
        Side::Declined(d) => format!("declined {d}"),
        Side::ArtifactError(e) => format!("artifact-error {e}"),
    }
}

/// Convert a libFuzzer crashes/artifacts dir into deduped findings in the failures queue.
fn cmd_triage_artifacts(args: &[String]) -> ExitCode {
    let mut crashes_dir: Option<PathBuf> = None;
    let mut findings: Option<PathBuf> = None;
    let mut commit: Option<String> = None;

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--findings" => findings = it.next().map(PathBuf::from),
            "--commit" => commit = it.next().cloned(),
            other if !other.starts_with('-') => crashes_dir = Some(PathBuf::from(other)),
            other => {
                eprintln!("cdz-smith triage-artifacts: unexpected arg `{other}`");
                return ExitCode::from(2);
            }
        }
    }

    let Some(crashes_dir) = crashes_dir else {
        eprintln!("cdz-smith triage-artifacts: expected a CRASHES_DIR");
        return ExitCode::from(2);
    };

    let findings_dir = match resolve_findings_dir(findings) {
        Ok(d) => d,
        Err(code) => return code,
    };

    let store = match cdz_smith::finding::FindingStore::open(&findings_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "cdz-smith: cannot open findings dir {}: {e}",
                findings_dir.display()
            );
            return ExitCode::FAILURE;
        }
    };
    let commit = commit.unwrap_or_else(driver::detect_commit);

    match cdz_smith::triage::triage_artifacts(&crashes_dir, &store, &commit) {
        Ok(stats) => {
            eprintln!(
                "[cdz-smith] triaged {} artifact(s): {} new bucket(s), {} dup hit(s), {} phantom (did not reproduce — fork-mode noise, discarded) → {}",
                stats.artifacts_seen,
                stats.new_buckets,
                stats.duplicate_hits,
                stats.not_reproduced,
                findings_dir.display()
            );
            if stats.new_buckets > 0 {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            eprintln!("cdz-smith triage-artifacts: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_fuzz(args: &[String]) -> ExitCode {
    let mut iterations: Option<u64> = None;
    let mut seed: Option<u64> = None;
    let mut timeout_secs: u64 = 10;
    let mut findings: Option<PathBuf> = None;

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--iterations" | "-n" => iterations = it.next().and_then(|s| s.parse().ok()),
            "--seed" => seed = it.next().and_then(|s| parse_seed(s)),
            "--timeout" => timeout_secs = it.next().and_then(|s| s.parse().ok()).unwrap_or(10),
            "--findings" => findings = it.next().map(PathBuf::from),
            other => {
                eprintln!("cdz-smith fuzz: unexpected arg `{other}`");
                return ExitCode::from(2);
            }
        }
    }

    // Resolve the findings dir: explicit flag, else discover spec/semantics/failures from cwd.
    let findings_dir = match resolve_findings_dir(findings) {
        Ok(d) => d,
        Err(code) => return code,
    };

    let cfg = Config {
        iterations,
        run_seed: seed.unwrap_or_else(driver::wallclock_seed),
        timeout: Duration::from_secs(timeout_secs),
        findings_dir: findings_dir.clone(),
        commit: driver::detect_commit(),
        progress_every: 1000,
    };

    eprintln!(
        "[cdz-smith] fuzzing @{} | seed {} | timeout {}s | findings → {}",
        cfg.commit,
        cfg.run_seed,
        timeout_secs,
        findings_dir.display()
    );
    match driver::run(&cfg) {
        Ok(stats) => {
            eprintln!(
                "[cdz-smith] done: {} programs | {} crashes, {} invalid-wasm ({} new buckets, {} dup hits) | {} timeouts",
                stats.total(),
                stats.crashes,
                stats.invalid_wasm,
                stats.new_buckets,
                stats.duplicate_hits,
                stats.timeouts
            );
            // A batch run that surfaced a NEW bucket exits non-zero so a CI/cron wrapper can notice.
            if stats.new_buckets > 0 {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            eprintln!("cdz-smith: run failed: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Seed the fuzz corpus with the semantics-corpus ASTs (S2). Extracts every `(input <program>)` from
/// `spec/semantics/*.sexp`, encodes each to canonical binary-AST bytes, and writes a deduped,
/// content-hashed seed corpus for the binary-AST entropy target (`cdz_smith_ast_never_panics`).
/// `--semantics` overrides the corpus dir (default: discover `spec/semantics` from cwd); `--out`
/// overrides the seed dir (default: `<semantics>/../../implementation/seed/crates/cdz-smith/corpus/
/// ast-seeds`, i.e. this crate's `corpus/ast-seeds`).
/// Run the wasm backend over the AST seed corpus (S3): run every `*.ast` seed through `run_wasm_ast`
/// and print an outcome tally (value / trap / declined). Demonstrates the operator's "run the wasm
/// backend on the semantics-corpus AST seeds" end to end. `--seeds` overrides the seed dir (default:
/// this crate's `corpus/ast-seeds` — populate it via `seed-corpus`); `--store` overrides the runtime
/// store (default: `<repo>/target/cadenza-store`; pass `$CDZ_STORE` to use the nix component store so
/// seeds that import a runtime resolve it). Pure scalars need no store.
#[cfg(feature = "differential")]
fn cmd_run_ast_corpus(args: &[String]) -> ExitCode {
    let mut seeds: Option<PathBuf> = None;
    let mut store: Option<PathBuf> = None;

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--seeds" => seeds = it.next().map(PathBuf::from),
            "--store" => store = it.next().map(PathBuf::from),
            other => {
                eprintln!("cdz-smith run-ast-corpus: unexpected arg `{other}`");
                return ExitCode::from(2);
            }
        }
    }

    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let seeds_dir = seeds.unwrap_or_else(|| manifest.join("corpus").join("ast-seeds"));
    let store_dir = store.unwrap_or_else(|| {
        // implementation/seed/crates/cdz-smith → repo root is 4 ancestors up.
        manifest
            .ancestors()
            .nth(4)
            .map(|repo| repo.join("target").join("cadenza-store"))
            .unwrap_or_else(|| PathBuf::from("target/cadenza-store"))
    });

    match cdz_smith::differential::run_ast_corpus_sweep(&seeds_dir, &store_dir) {
        Ok(s) => {
            eprintln!(
                "[cdz-smith] run-ast-corpus: {} seed(s) from {} | store {} → {} value, {} trap, {} declined",
                s.seeds,
                seeds_dir.display(),
                store_dir.display(),
                s.values,
                s.traps,
                s.declined
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("cdz-smith run-ast-corpus: failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_seed_corpus(args: &[String]) -> ExitCode {
    let mut semantics: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--semantics" => semantics = it.next().map(PathBuf::from),
            "--out" => out = it.next().map(PathBuf::from),
            other => {
                eprintln!("cdz-smith seed-corpus: unexpected arg `{other}`");
                return ExitCode::from(2);
            }
        }
    }

    let semantics_dir = match semantics {
        Some(d) => d,
        None => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            match cdz_smith::seeds::discover_semantics_dir(&cwd) {
                Some(d) => d,
                None => {
                    eprintln!(
                        "cdz-smith seed-corpus: could not find spec/semantics from {} — pass --semantics DIR",
                        cwd.display()
                    );
                    return ExitCode::from(2);
                }
            }
        }
    };
    // Default seed dir = this crate's corpus/ast-seeds, resolved from the crate root at build time.
    let out_dir = out.unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("corpus")
            .join("ast-seeds")
    });

    match cdz_smith::seeds::write_seed_corpus(&semantics_dir, &out_dir) {
        Ok(stats) => {
            eprintln!(
                "[cdz-smith] seed-corpus: scanned {} file(s) in {} → wrote {} seed(s) ({} dup collapsed) to {}",
                stats.files,
                semantics_dir.display(),
                stats.written,
                stats.duplicates,
                out_dir.display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("cdz-smith seed-corpus: failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_once(args: &[String]) -> ExitCode {
    let Some(seed) = args.first().and_then(|s| parse_seed(s)) else {
        eprintln!("cdz-smith once: expected a SEED");
        return ExitCode::from(2);
    };
    let src = driver::program_for_seed(seed);
    println!("--- program (seed {seed}) ---\n{src}\n--- verdict ---");
    report(&driver::once(seed))
}

fn cmd_gen(args: &[String]) -> ExitCode {
    let Some(seed) = args.first().and_then(|s| parse_seed(s)) else {
        eprintln!("cdz-smith gen: expected a SEED");
        return ExitCode::from(2);
    };
    print!("{}", driver::program_for_seed(seed));
    ExitCode::SUCCESS
}

fn cmd_verify(args: &[String]) -> ExitCode {
    let Some(arg) = args.first() else {
        eprintln!("cdz-smith verify: expected a FILE.sexp or a SEED");
        return ExitCode::from(2);
    };
    let source = match resolve_source(arg, "verify") {
        Ok(s) => s,
        Err(code) => return code,
    };
    report(&compile_catching(&source))
}

/// Resolve a `verify`-style argument into program source: a path to a reproducer file, a bare SEED
/// (decimal / `0x`-hex, mapped through the PRNG generator), or a finding NAME relative to the discovered
/// `spec/semantics/failures` store (so `foo.smith.sexp` works from anywhere in the repo). `subcmd` is
/// only for the error text. Shared by `verify` and `verify-differential`.
fn resolve_source(arg: &str, subcmd: &str) -> Result<String, ExitCode> {
    if std::path::Path::new(arg).exists() {
        return std::fs::read_to_string(arg).map_err(|e| {
            eprintln!("cdz-smith {subcmd}: cannot read {arg}: {e}");
            ExitCode::FAILURE
        });
    }
    if let Some(seed) = parse_seed(arg) {
        return Ok(driver::program_for_seed(seed));
    }
    // Try resolving relative to a discovered failures dir (matching the finding note's suggested command).
    match cdz_smith::finding::FindingStore::discover(
        &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    ) {
        Ok(store) => {
            let p = store.dir().join(arg);
            std::fs::read_to_string(&p).map_err(|_| {
                eprintln!(
                    "cdz-smith {subcmd}: `{arg}` is neither a file, a seed, nor a finding in {}",
                    store.dir().display()
                );
                ExitCode::from(2)
            })
        }
        Err(_) => {
            eprintln!("cdz-smith {subcmd}: `{arg}` is neither a readable file nor a seed");
            Err(ExitCode::from(2))
        }
    }
}

/// Print a verdict; exit non-zero iff it is a finding (a crash), so `verify` doubles as a check.
fn report(v: &Verdict) -> ExitCode {
    match v {
        Verdict::Compiled { component_len } => {
            println!("COMPILED ({component_len} bytes) — not a bug");
            ExitCode::SUCCESS
        }
        Verdict::Declined { code, message } => {
            println!(
                "DECLINED [{}] — not a bug: {message}",
                code.as_deref().unwrap_or("uncoded")
            );
            ExitCode::SUCCESS
        }
        Verdict::ParseError(e) => {
            println!("PARSE ERROR — not a compiler finding: {e}");
            ExitCode::from(3)
        }
        Verdict::Crash(info) => {
            println!(
                "CRASH — a bug\n  site:    {}\n  message: {}",
                info.site.as_deref().unwrap_or("<unknown>"),
                info.message.lines().next().unwrap_or("")
            );
            ExitCode::from(1)
        }
        Verdict::InvalidWasm {
            detail,
            component_len,
        } => {
            println!(
                "INVALID WASM — a backend miscompile\n  component: {component_len} bytes\n  validator: {}",
                detail.lines().next().unwrap_or("")
            );
            ExitCode::from(1)
        }
    }
}

/// Accept a seed as decimal or `0x`-prefixed hex.
fn parse_seed(s: &str) -> Option<u64> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok()
    } else {
        s.parse().ok()
    }
}

/// Resolve the findings directory: an explicit `--findings DIR` wins; otherwise discover the
/// `spec/semantics/failures` store from the cwd. On discovery failure, print the error and hand back
/// `ExitCode::FAILURE` so the caller can `return` it. Shared by `fuzz`/`differential`/`triage-artifacts`.
fn resolve_findings_dir(explicit: Option<PathBuf>) -> Result<PathBuf, ExitCode> {
    match explicit {
        Some(d) => Ok(d),
        None => match cdz_smith::finding::FindingStore::discover(
            &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        ) {
            Ok(store) => Ok(store.dir().to_path_buf()),
            Err(e) => {
                eprintln!("cdz-smith: could not locate spec/semantics/failures: {e}");
                Err(ExitCode::FAILURE)
            }
        },
    }
}
