//! cdz-kernel — the v2 agent-harness kernel (from-scratch rebuild).
//!
//! A generic, minimal, if-this-then-that runtime. The kernel accepts signed content-addressed events
//! into an append-only per-session log, folds the current reducer over each event, authorizes the
//! effects it requests, dispatches them, and folds the results back in. It knows NOTHING about
//! Cadenza, agents, or models — those are reducers + executors. See `design/agent-harness-kernel.md`
//! for the full design and `§15b` for the v0 scope.
//!
//! ## v0.1 module map
//! - [`hash`] — content addressing (blake3), the one hashing primitive.
//! - [`effect`] — effect kinds/requests + **resource-scoped capabilities** (SEC-F1).
//! - [`event`] — events + the per-session log, with the **durable dispatch/result/timer** records (S1).
//! - [`kv`] — the session-attached KV; its root hash is a **free per-event snapshot** (§4).
//! - [`reducer`] — the pure-fold reducer contract (gap A); effect-await = KV continuation by id (S4).
//! - [`authz`] — the capability-set authorizer (SEC-F1 enforcement point).
//! - [`executor`] — the effect-execution contract (+ idempotency key for crash-safe re-drive).
//! - [`kernel`] — the core `fold → authorize → durably-dispatch → execute → fold-result` loop, plus
//!   `replay` (crash recovery: rebuild KV + open-obligation set from the log, no live re-execution).
//!
//! The adversarial-review invariants (design §16c) are load-bearing and enforced here, not bolted on:
//! durable dispatch before routing (S1), effect-id correlation with timeout-cancels (S4), resource-
//! scoped authorization (SEC-F1), absolute timer deadlines (S5), canonical/deterministic encoding
//! (S3/S8). Cross-version replay, multi-node, PKI, and Cadenza-native reducers are explicitly deferred.

pub mod authz;
pub mod effect;
pub mod event;
pub mod executor;
pub mod hash;
pub mod kernel;
pub mod kv;
pub mod log_store;
pub mod reducer;
#[cfg(feature = "wasm-reducer")]
pub mod wasm_host;
