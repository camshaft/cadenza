//! `cdz-rust-run` — run a Cadenza program's emitted RUST and grade it, the Rust-target analogue of
//! `cdz-run` (wasm). The dedicated home for the rust-backend exec runner, extracted from `xtask`'s
//! `run_program_rust` family (operator 2026-08-26: "make a dedicated crate for the rust runners; the
//! xtask is just getting so bloated"). It is the Rust exec phase of the corpus nix caching pipeline
//! (`design/DESIGN-corpus-nix-per-case-caching.md`), one of the gaps to close before the `xtask gate`
//! can be retired.
//!
//! Built up incrementally. This increment lands the PURE, process-free foundation — parsing the emitted
//! Rust function signatures (`sig`) — which the driver generation + call marshalling depend on. Later
//! increments add: driver generation (export call + host-response shims + closure/factory application),
//! the `rustc` invocation (linking the pre-built `cdz_rt`/`cdz_num`/`cadenza_ast` rlibs), the run, and the
//! outcome grade.

pub mod driver;
pub mod sig;
