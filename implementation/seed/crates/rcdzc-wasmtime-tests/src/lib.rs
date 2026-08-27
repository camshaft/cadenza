//! Relocated wasmtime-driven behavior tests for the `rcdzc` compiler.
//!
//! This crate exists so `rcdzc` can drop its `wasmtime` dev-dependency (operator directive
//! 2026-08-27): the compiler crate must not dev-dep wasmtime, but its behavior tests that instantiate
//! and RUN the emitted component under wasmtime are legitimate coverage. They live here instead — this
//! crate is allowed to dev-dep wasmtime and drives the compiler through its public API
//! ([`rcdzc::compile_component`], [`rcdzc::codec`], [`rcdzc::ast`]).
//!
//! The whole crate is test-only (`#![cfg(test)]`): in a normal `cargo build --workspace` the lib is
//! empty, so the seed build and the corpus gate never compile wasmtime on account of this crate.
//! `cargo test -p rcdzc-wasmtime-tests` builds the tests and reuses the already-compiled wasmtime rlib.
//!
//! [`common`] carries the shared drivers the relocated tests use — the same helpers that lived beside
//! the tests in `rcdzc/src/tests.rs` (`FromVal`, `run_returns`, the `parse`/`compile` bridge). Each
//! group of relocated tests is its own submodule (`smoke` is the first, proving the wiring end-to-end).

#![cfg(test)]

mod common;
mod smoke;
