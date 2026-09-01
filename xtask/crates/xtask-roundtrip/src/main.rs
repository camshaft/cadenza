//! `xtask-roundtrip` — round-trip every corpus program through the syntax surfaces (`sexpr` must
//! reproduce the exact binary; `ml` must reach a fixed point `ml(ml(x)) == ml(x)`), a guard on
//! `cadenza-syntax` independent of the compiler. Carved out of the xtask monolith into its own crate
//! (v-xtask-decompose). Tools come from `CDZ_SEED_BIN_DIR` (the nix app injects the warm nix-built
//! cdz/cdz-corpus) — no cargo build; the repo root from `CDZ_REPO_ROOT` (else cwd).

use std::path::{Path, PathBuf};
use xtask_support::{CorpusRecord, convert_bytes, default_corpus_files, read_corpus, to_binary};

fn main() {
    let repo = xtask_support::repo_root();
    // The nix-built pipeline tools (cdz for surface conversions, cdz-corpus for record extraction), from
    // the dir the `apps.roundtrip` wrapper points CDZ_SEED_BIN_DIR at. Falls back to `<repo>/target/debug`
    // for a bare `cargo run -p xtask-roundtrip` (dev), where a prior `cargo build` left the bins.
    let bin_dir = xtask_support::seed_bin_dir(&repo);
    let cdz = bin_dir.join("cdz");
    let corpus = bin_dir.join("cdz-corpus");

    // Positional args are the corpus files to check; empty = the whole corpus.
    let files: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let files = if files.is_empty() {
        default_corpus_files(&repo)
    } else {
        files
    };

    // Gather every case, then round-trip in PARALLEL — each case only reads the tool paths + spawns its
    // own conversions, so the ~1025 cases × 2 surfaces are embarrassingly parallel.
    let records: Vec<CorpusRecord> = files
        .iter()
        .flat_map(|file| read_corpus(&corpus, file))
        .collect();
    let per_case = roundtrip_all_parallel(&cdz, records);

    let (mut ok, mut fail) = (0u32, 0u32);
    let mut failures: Vec<String> = Vec::new();
    for case in per_case {
        if case.counted_ok {
            ok += 1;
        }
        fail += case.failures.len() as u32;
        failures.extend(case.failures);
    }

    println!("\nroundtrip: {ok} programs ok, {fail} failures");
    if !failures.is_empty() {
        println!();
        for f in failures.iter().take(40) {
            println!("  FAIL  {f}");
        }
        if failures.len() > 40 {
            println!("  … and {} more", failures.len() - 40);
        }
        std::process::exit(1);
    }
}

/// One case's round-trip outcome: whether it counted as an `ok` program, and any failure messages.
struct RoundtripCase {
    counted_ok: bool,
    failures: Vec<String>,
}

/// Round-trip every record in PARALLEL, one [`RoundtripCase`] per record in the SAME order as
/// `records`. A `std::thread::scope` worker pool (no new dependency) pulling from a shared atomic
/// cursor; each case only reads `cdz_bin` and spawns its own conversions (no shared mutable state).
fn roundtrip_all_parallel(cdz_bin: &Path, records: Vec<CorpusRecord>) -> Vec<RoundtripCase> {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let n = records.len();
    let slots: Vec<Mutex<Option<RoundtripCase>>> = (0..n).map(|_| Mutex::new(None)).collect();
    let cursor = AtomicUsize::new(0);
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .max(1);

    std::thread::scope(|scope| {
        for _ in 0..workers {
            let cursor = &cursor;
            let slots = &slots;
            let records = &records;
            scope.spawn(move || {
                loop {
                    let i = cursor.fetch_add(1, Ordering::Relaxed);
                    if i >= records.len() {
                        break;
                    }
                    let rec = &records[i];
                    let mut failures = Vec::new();
                    // The reference: the program's canonical binary AST. If it fails, the case is not
                    // counted ok.
                    let counted_ok = match to_binary(cdz_bin, &rec.program) {
                        None => {
                            failures.push(format!("{}: sexpr→binary failed", rec.description));
                            false
                        }
                        Some(bin0) => {
                            // `sexpr` is STRICT (byte-identical); `ml` must reach a FIXED POINT
                            // (`ml(ml(x)) == ml(x)`) — the ML surface is allowed a one-time
                            // semantics-preserving canonicalization, so idempotence, not strict equality.
                            match roundtrip_via(cdz_bin, &bin0, "sexpr") {
                                Some(bin1) if bin1 == bin0 => {}
                                Some(_) => failures
                                    .push(format!("{}: binary≠binary via sexpr", rec.description)),
                                None => failures.push(format!(
                                    "{}: round-trip via sexpr errored",
                                    rec.description
                                )),
                            }
                            match roundtrip_via(cdz_bin, &bin0, "ml") {
                                Some(bin1) => match roundtrip_via(cdz_bin, &bin1, "ml") {
                                    Some(bin2) if bin2 == bin1 => {}
                                    Some(_) => failures.push(format!(
                                        "{}: ml round-trip not idempotent (ml(ml(x)) != ml(x))",
                                        rec.description
                                    )),
                                    None => failures.push(format!(
                                        "{}: second ml round-trip errored",
                                        rec.description
                                    )),
                                },
                                None => failures.push(format!(
                                    "{}: round-trip via ml errored",
                                    rec.description
                                )),
                            }
                            true
                        }
                    };
                    *slots[i].lock().unwrap() = Some(RoundtripCase {
                        counted_ok,
                        failures,
                    });
                }
            });
        }
    });

    slots
        .into_iter()
        .map(|slot| {
            slot.into_inner()
                .unwrap()
                .expect("every case round-tripped")
        })
        .collect()
}

/// binary → <surface> text → binary, returning the re-encoded bytes (to compare to the original).
fn roundtrip_via(cdz_bin: &Path, bin0: &[u8], surface: &str) -> Option<Vec<u8>> {
    let text = convert_bytes(cdz_bin, bin0, "binary", surface)?;
    convert_bytes(cdz_bin, &text, surface, "binary")
}
