//! `cdz-reducer-guest` — ONE generic reducer-world GUEST wrapper, parameterized per-target by a cargo
//! feature (operator directive 2026-09-10: "no crate per wasm target … generate on the fly in the nix
//! flake … they all export the same interface"). Each target decodes its request payload, runs its inner
//! function, and returns the result as the guest's `close` reason (design/DESIGN-reducer-targets.md §2/§4).
//!
//! This crate holds the target HANDLERS as pure `bytes -> bytes` functions (native-testable — B2). The
//! wit-bindgen `cadenza:platform/guest` export (`on-message` wrapping a handler in a `close` step) and the
//! wasm componentization are wired per-target by the nix flake (B3), so the handler here is exactly the body
//! a guest's `on-message` runs — decode `message.payload`, run the target fn, put the result in the close
//! reason. Keeping the handler pure + native keeps its logic gated by an ordinary Rust test rather than a
//! full wasm round-trip (the round-trip is B3's dedicated conformance check).
//!
//! Targets so far: `rcdzc.compile` (feature `target-rcdzc`). `sexpr`/`ml` parser targets add their own
//! features + `cadenza-syntax` dep later (B6/B7) — new features, NOT new crates.

pub mod request_wire;

/// The parser targets' `{ast, diagnostics}` response wire (used by `sexpr`/`ml`; always compiled so its
/// round-trip test guards the wire under any feature).
pub mod parse_result_wire;

/// The rcdzc compile target (feature `target-rcdzc`): decode a kinded-input bundle -> compile to a wasm
/// component -> encode the `{artifacts, diagnostics}` response envelope.
#[cfg(feature = "target-rcdzc")]
pub mod rcdzc_target;

/// The sexpr parser target (feature `target-sexpr`): s-expression source bytes -> AST bytes + diagnostics.
#[cfg(feature = "target-sexpr")]
pub mod sexpr_target;

/// The ml parser target (feature `target-ml`): ML source bytes -> AST bytes + (error-recovering) diagnostics.
#[cfg(feature = "target-ml")]
pub mod ml_target;

// The reducer-world `guest` export — the wasm component boundary (built for wasm32-unknown-unknown by
// cargo-component, WASI-free). `bindings` is the cargo-component-generated glue (`src/bindings.rs`,
// DO-NOT-EDIT); `guest` implements `on-message` by wrapping the active target's handler in a `close` step.
// BOTH are `wasm32`-only: the native member build (+ B2's unit tests) never compiles the export shims, so
// the workspace gate is unaffected; the wasm build + the B3 compile->instantiate->run round-trip validate them.
#[cfg(target_arch = "wasm32")]
#[allow(warnings)]
mod bindings;
#[cfg(target_arch = "wasm32")]
mod guest;
