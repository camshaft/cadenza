//! The runtime's embedded wasm allocator — the "runtime ships its own allocator" ask, isolated so
//! it is swappable in this one file.
//!
//! The heap core (`lib.rs`) is allocator-agnostic: it only ever allocates through `Box`/`Vec`, i.e.
//! the global allocator. This module *is* that global allocator on wasm. We use talc's
//! single-threaded dynamic wasm allocator: a lock-free `GlobalAlloc` that requests pages from the
//! WebAssembly memory subsystem and grows linear memory on demand. Lock-free is correct here — a
//! component instance is single-threaded — and it keeps the allocator small and deterministic
//! (`opt-level="s"`, `lto`, `panic=abort`; see `Cargo.toml`).
//!
//! Only compiled for wasm (`lib.rs` gates `mod allocator` on `target_arch = "wasm32"`); native
//! `cargo test` keeps the system allocator, which is why the `Handle`-typed core is fully testable
//! natively without this module.
//!
//! Swapping allocators (e.g. to a bump/free-list of our own in Phase D, or back to std's dlmalloc)
//! is a change to this file alone — no core code references talc.

#[global_allocator]
static ALLOCATOR: talc::wasm::WasmDynamicTalc = talc::wasm::new_wasm_dynamic_allocator();
