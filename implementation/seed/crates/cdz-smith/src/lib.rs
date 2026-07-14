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
//! * a `bolero` property test (`tests/fuzz.rs`) — shrinking, `cargo test` integration, and
//!   optional coverage-guided libFuzzer via `cargo bolero`;
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

pub mod driver;
pub mod finding;
pub mod generator;
pub mod oracle;
pub mod triage;

pub use finding::{Category, Finding, FindingStore};
pub use generator::{Program, generate};
pub use oracle::{Verdict, compile_catching};
pub use triage::{TriageStats, triage_artifacts};
