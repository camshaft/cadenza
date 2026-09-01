//! `cdz-rust-run` — run a Cadenza program's emitted RUST and grade it, the Rust-target analogue of
//! `cdz-run` (wasm). The dedicated home for the rust-backend exec runner, extracted from `xtask`'s
//! `run_program_rust` family (operator 2026-08-26: "make a dedicated crate for the rust runners; the
//! xtask is just getting so bloated"). It is the Rust exec phase of the corpus nix caching pipeline
//! (`design/DESIGN-corpus-nix-per-case-caching.md`), one of the gaps to close before the `xtask gate`
//! can be retired.
//!
//! The pieces: signature analysis (`sig`), driver generation (`driver` — export call + host-response
//! shims), the `rustc` compile+run (`run` — linking the pre-built `cdz_rt`/`cdz_num`/`cadenza_ast` rlibs),
//! and the outcome grade (`grade` — reusing the shared backend-independent `cdz-corpus-grade` compare, run
//! through a rust trial-runner). The `cdz-rust-run` bin (`main.rs`) is the `--grade` entry the nix per-case
//! rust exec layer shells out to. Deferred: the host-closure factory/consumer application + the async
//! `block_on` harness (the small closure/async corpus subset).

pub mod driver;
pub mod grade;
pub mod run;
pub mod sig;
pub mod value_doc;
