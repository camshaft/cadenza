//! Bolero property target: a byte seed, through the generator, should never PANIC the compiler.
//!
//! This is the same generator + oracle the continuous driver uses, wired to `bolero` so we get, for
//! free: (1) shrinking — a failing seed is minimized to the smallest byte string that still panics;
//! (2) `cargo test` integration — it runs as a bounded randomized property; and (3) COVERAGE-GUIDED
//! fuzzing — the primary way we run this. `bolero` mutates a `&[u8]`, our `generate` decodes those
//! bytes into a structured, always-parseable program, and libFuzzer's SanitizerCoverage feedback
//! keeps the inputs that reach NEW compiler edges and mutates them — pushing past the type-checker
//! plateau into the backend, where the dense panic clusters live. Run it:
//!   cargo bolero test cdz_smith_never_panics --engine libfuzzer -T 10m --timeout 10s \
//!       -E-fork=1 -E-ignore_timeouts=1 -E-ignore_crashes=1   (nightly; see fuzz-cycle.sh)
//!   cargo test --test fuzz                                    (bounded random, no coverage)
//!
//! The property is deliberately ONLY "does not panic". A decline or a coded rejection is expected,
//! correct output — never a failure. A found panic is a compiler bug; bolero prints + saves the seed
//! as a crash artifact. Under `-fork=1` libFuzzer also isolates HANGS: a per-input `-timeout` kills
//! the child and saves a `timeout-*` artifact WITHOUT stopping the campaign — so the coverage-guided
//! path catches timeouts too (the standalone `cdz-smith fuzz` PRNG driver's watchdog remains as a
//! no-nightly fallback).
//!
//! Not `#[ignore]`d: cdz-smith is its OWN workspace (excluded from the seed workspace — see
//! Cargo.toml), so the gate's `cargo test` never compiles this crate; there's no shared suite to
//! wedge. A plain `cargo test --test fuzz` here runs a bounded random sample (no hang guard), which
//! is fine for a crate nothing else builds.

use bolero::check;
use cdz_smith::generator::generate;
use cdz_smith::oracle::{Verdict, compile_catching};

#[test]
fn cdz_smith_never_panics() {
    check!().with_type::<Vec<u8>>().for_each(|seed: &Vec<u8>| {
        let program = generate(seed);
        match compile_catching(&program.source) {
            // A crash escaping the compile path is a bug: fail the property so bolero shrinks the
            // seed and reports it.
            Verdict::Crash(info) => panic!(
                "compiler panicked at {}: {}\nprogram:\n{}",
                info.site.as_deref().unwrap_or("<unknown>"),
                info.message.lines().next().unwrap_or(""),
                program.source
            ),
            // Everything else is expected output — not a finding.
            Verdict::Compiled { .. } | Verdict::Declined { .. } | Verdict::ParseError(_) => {}
        }
    });
}
