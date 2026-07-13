//! The shared native runtime interface for the Rust backend's emitted code.
//!
//! A Cadenza module compiled to Rust (`rcdzc --target rust`/`rust-async`) links against this crate
//! instead of carrying its own copy of the runtime traits. Two things live here so an application
//! defines them ONCE and every emitted module shares them:
//!
//! - [`CdzEnv`] — the gas/yield capability the async/gas-metered backend threads through every emitted
//!   function. Previously each async module emitted its OWN `CdzEnv` trait, so two modules had two
//!   incompatible env types and an application had to implement the capability once per module. With a
//!   single shared trait, the application implements it once and every module interoperates.
//!
//! Later increments add the VALUE-RUNTIME seam (`CdzRuntime` / `CdzRuntimeAsync`) — a trait the emitted
//! code calls for compound operations (list/string/bytes/map/set) so the CALLER chooses the
//! representation (a `Vec`, a persistent vector, an arena) — plus a default `RcRuntime` wiring.
//!
//! Dep-free and `no_std`-friendly in spirit (uses only `core::future`), so linking it into an existing
//! Rust codebase adds no transitive weight.

/// The gas/yield capability the async, gas-metered Rust backend threads through every emitted function.
///
/// An emitted `async fn` awaits `env.consume(1)` at entry, so the host meters fuel and MAY perform a
/// cooperative yield inside `consume` (return control to the executor after accounting) — a runaway or
/// long-running computation is then bounded at the granularity of a call. `consume` returns
/// `impl Future` (RPITIT) rather than being written `async fn` in the trait, so an implementor needs no
/// `async_trait` dependency and the emitted call site stays lint-clean.
///
/// A typical implementation increments a counter and, past a budget, either never resolves the future
/// (the executor drops the task) or panics — the emitted code is agnostic to that policy; it only
/// awaits the charge.
pub trait CdzEnv {
    /// Charge `gas` units of fuel; the returned future MAY yield cooperatively before resolving.
    fn consume(&mut self, gas: u64) -> impl core::future::Future<Output = ()>;
}
