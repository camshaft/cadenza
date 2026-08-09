//! cdz-smith — a fuzzer for the reference Cadenza compiler (`rcdzc`).
//!
//! # The pipeline
//!
//! ```text
//!   seed bytes ──gen──▶ s-expr program ──parse+encode──▶ binary AST ──oracle──▶ Verdict
//!                                                                                  │
//!                                                          Crash / Timeout ────────┘
//!                                                                  │
//!                                                          shrink + dedup ──▶ spec/semantics/failures/
//! ```
//!
//! Every stage is driven by a plain `&[u8]` seed (the fuzzer input), so the SAME generator +
//! oracle is exercised three ways with no divergence:
//!
//! * [`driver`] — a seeded-PRNG batch loop (the continuous, cron-driven mode);
//! * the `bolero` property target [`cdz_smith_never_panics`] (an in-crate `#[cfg(test)]` fn) —
//!   shrinking, `cargo test` integration, and optional coverage-guided libFuzzer via `cargo bolero`;
//! * a subprocess worker the driver spawns to isolate a HANG or a hard crash (segfault/OOM)
//!   that an in-process `catch_unwind` cannot survive.
//!
//! # What counts as a bug
//!
//! The compiler reports every "no" as DATA — a `Diagnostic` (a coded rejection) or an uncoded
//! decline ("not lowered yet"). Those are EXPECTED output, never a finding. A finding is:
//!
//! * a **crash** — an unwinding panic (`.unwrap()`/`.expect(`/`unreachable!`/`panic!`) escaping
//!   the compile path, observed by [`oracle::compile_catching`] via `catch_unwind`; or
//! * a **timeout** — a compile that does not finish inside a wall-clock budget (detected out of
//!   process, since `catch_unwind` cannot interrupt a runaway loop).
//!
//! Differential miscompiles (two backends disagreeing on a program's value) are a planned second
//! oracle; see [`oracle`].

/// The wasm-vs-rust differential oracle. Behind the off-by-default `differential` feature because it
/// depends on `cdz-run` (wasmtime), which must not link into the instrumented libFuzzer target — see
/// the crate's `[features]` in Cargo.toml.
#[cfg(feature = "differential")]
pub mod differential;
pub mod driver;
pub mod finding;
pub mod generator;
pub mod oracle;
pub mod triage;

pub use finding::{Category, Finding, FindingStore};
pub use generator::{Program, generate};
pub use oracle::{Verdict, compile_catching};
pub use triage::{TriageStats, triage_artifacts};

// ── the bolero property target ──────────────────────────────────────────────────────────────────
// The IN-CRATE `#[cfg(test)]` bolero target (relocated from `tests/fuzz.rs` per the no-integration-
// tests directive). Kept at the CRATE ROOT (no `mod` wrapper) on purpose: `cargo bolero` names a
// target by its full module path, so a wrapping module would rename it to `<mod>::cdz_smith_never_panics`
// and break the by-name `cargo bolero test cdz_smith_never_panics` invocation in `fuzz-cycle.sh`. At
// the root the name stays bare, so the harness invocation is unchanged. Same coverage — still catches
// compiler panics + invalid wasm — just not as a `tests/*.rs` file.
//
// Bolero property target: a byte seed, through the generator, should never PANIC the compiler.
//
// This is the same generator + oracle the continuous driver uses, wired to `bolero` so we get, for
// free: (1) shrinking — a failing seed is minimized to the smallest byte string that still panics;
// (2) `cargo test` integration — it runs as a bounded randomized property; and (3) COVERAGE-GUIDED
// fuzzing — the primary way we run this. `bolero` mutates a `&[u8]`, our `generate` decodes those
// bytes into a structured, always-parseable program, and libFuzzer's SanitizerCoverage feedback
// keeps the inputs that reach NEW compiler edges and mutates them — pushing past the type-checker
// plateau into the backend, where the dense panic clusters live. Run it:
//   cargo bolero test cdz_smith_never_panics --engine libfuzzer -T 10m --timeout 10s \
//       -E-fork=1 -E-ignore_timeouts=1 -E-ignore_crashes=1   (nightly; see fuzz-cycle.sh)
//   cargo test cdz_smith_never_panics                        (bounded random, no coverage)
//
// The property is deliberately ONLY "does not panic". A decline or a coded rejection is expected,
// correct output — never a failure. A found panic is a compiler bug; bolero prints + saves the seed
// as a crash artifact. Under `-fork=1` libFuzzer also isolates HANGS: a per-input `-timeout` kills
// the child and saves a `timeout-*` artifact WITHOUT stopping the campaign — so the coverage-guided
// path catches timeouts too (the standalone `cdz-smith fuzz` PRNG driver's watchdog remains as a
// no-nightly fallback).
#[cfg(test)]
#[test]
fn cdz_smith_never_panics() {
    use bolero::check;
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
            // The compiler reported success but emitted wasm that doesn't validate — a backend
            // miscompile. Also a bug: fail so the seed is saved as a crash artifact + shrunk.
            Verdict::InvalidWasm { detail, .. } => panic!(
                "compiler emitted INVALID wasm: {}\nprogram:\n{}",
                detail.lines().next().unwrap_or(""),
                program.source
            ),
            // Everything else is expected output — not a finding.
            Verdict::Compiled { .. } | Verdict::Declined { .. } | Verdict::ParseError(_) => {}
        }
    });
}
