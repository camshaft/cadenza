//! `cdz-num` — the Cadenza arbitrary-precision numeric library for the RUST BACKEND.
//!
//! This crate exists so the `rcdzc` rust backend can emit programs that use the SAME bignum (`Big`) the
//! wasm runtime uses — reuse, not a second implementation (the operator-directed numeric-tower approach).
//!
//! # Why a SOURCE share (`#[path]`), not a shared linked crate
//! The obvious design — extract `bigint.rs` into a crate that BOTH `cdz-runtime` and this backend depend
//! on — was tried and REVERTED (#459): adding a second crate to `cdz-runtime`'s cross-crate-LTO set made
//! the frozen runtime wasm CROSS-MACHINE NON-DETERMINISTIC (a crate unit's `-Cmetadata` is baked into the
//! LTO'd symbol layout and varies by build env, so three machines produced three different
//! `REQUIRED_RUNTIME_HASH` values; debug matched because dev has no cross-crate LTO merge). v-runtime
//! (frozen-hash owner) RULED: keep `Big` PHYSICALLY IN `cdz-runtime` as a plain module (its single-crate
//! LTO stays reproducible, hash stays `def9d173`, nothing to re-freeze) and share the SOURCE here.
//!
//! So this crate brings in `cdz-runtime/src/bigint.rs` VERBATIM as a `#[path]` module: `Big` is compiled
//! as part of THIS crate's own build (a normal rlib the gate links via `--extern`), while `cdz-runtime`
//! keeps the exact same file as its own module in its own single-crate build. Zero SOURCE duplication
//! (one file, two compiles), and `cdz-runtime`'s wasm build is untouched — no determinism regression, no
//! hash churn. See [[runtime-cross-arch-determinism]] and DESIGN-rust-runtime-trait.md.

// The `#[path]`-included `bigint.rs` is v-runtime's SOURCE, physically owned by `cdz-runtime` (an
// excluded standalone workspace with its own lint posture — AND its bytes feed the frozen wasm content
// hash, so it must NOT be reformatted here). Pulling it into THIS (main-workspace) crate would otherwise
// subject it to `clippy -D warnings` (it trips a few style lints the excluded crate never gated) and to
// `cargo fmt --all` (which would rewrite its compact struct literals, churning the frozen hash). So this
// crate-level `#![allow(...)]` silences the borrowed source's style lints, and the module carries
// `#[rustfmt::skip]` (below) to leave the shared file byte-for-byte — neither touches bigint.rs.
#![allow(
    clippy::needless_range_loop,
    clippy::should_implement_trait,
    clippy::neg_multiply
)]

// `bigint.rs` writes fully-qualified `alloc::…` paths (it was authored as a `no_std` module of the
// `no_std` runtime), so `alloc` must be in scope here. Bringing it in explicitly works under `std` too
// (a `std` crate can always name `alloc`), so this crate needs no `#![no_std]` — it is a plain host rlib.
extern crate alloc;

// The bignum, verbatim from the runtime, brought in via `#[path]` as a real module file. `#[path]` (not
// `include!`) because `bigint.rs` opens with `//!` INNER doc comments (module docs): `include!` pastes
// tokens inline and rejects a leading `//!` (E0753 "inner doc comments can only appear before items"),
// whereas `#[path = "…"] mod big;` treats the file AS module `big`'s own source — exactly the role it
// plays in `cdz-runtime` — where its `//!` module docs are valid. `#[rustfmt::skip]` keeps `cargo fmt`
// from rewriting the shared file. Same file, same semantics, no edits to the shared source.
#[rustfmt::skip]
#[path = "../../cdz-runtime/src/bigint.rs"]
pub mod big;

// The rust backend emits `cdz_num::Big`, so surface `Big` at the crate root (the submodule is an
// include-mechanics detail, not part of the API shape the emit targets).
pub use big::Big;
