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
//! - [`event`] — events + the per-session log, with the **durable dispatch/result/timer** records (S1);
//!   frozen canonical encoding (S3), byte-stability golden-pinned.
//! - [`event_ast`] — maps an [`event::Event`] to/from a Cadenza AST so the log IS encoded through the
//!   **shared** `cadenza-ast` canonical codec (§19a), not a bespoke one; a `decode` failure means "this
//!   whole frame is bad" (→ Corrupt). (The torn-vs-corrupt split is `log_store`'s FRAMING layer, not
//!   this codec — see [`log_store`]. `log_store` uses this codec — the swap landed.)
//! - [`kv`] — the session-attached KV; its root hash is a **free per-event snapshot** (§4), and
//!   `encode`/`decode` make that snapshot **restorable** from CAS, not just addressable.
//! - [`reducer`] — the pure-fold reducer contract (gap A); effect-await = KV continuation by id (S4).
//! - [`authz`] — the capability-set authorizer (SEC-F1 enforcement point).
//! - [`executor`] — the effect-execution contract (+ idempotency key for crash-safe re-drive); a
//!   [`executor::CompositeExecutor`] routes by kind so a session serves multiple effect kinds.
//! - [`blob`] — the content-addressable blob store (CAS): `hash → bytes`, self-verifying on disk. The
//!   §4 large-payload store + the substrate for resolving a reducer's declared component deps by hash.
//! - [`component_store`] — resolve a component's bytes from an on-disk content-addressed store (v-nix's
//!   `componentStore`: `<hash>.wasm` + a `runtime.toml` name→hash manifest): by HASH (a reducer's `+<hash>`
//!   dep) or by manifest NAME (the runtime's bare `cadenza:nfc/normalize` inter-runtime import). The
//!   resolution half of the §19e/§23 transitive-dep compose; content-verifies every fetch.
//! - [`log_store`] — the durable append-only disk log: length-framed events, torn-tail-tolerant
//!   recovery (`Clean`/`TornTail`/`Corrupt`), heal-a-torn-tail via `truncate_to`.
//! - [`wasm_host`] — the wasmtime component host (§19b): binds `wit/reducer.wit`, runs a reducer as a
//!   real wasm component (fuel-bounded fold), and resolves declared component deps by hash (§23 —
//!   runtime-agnostic, no hard-coded runtime). Also the generic multi-export [`wasm_host::invoke_component`]
//!   primitive (resolve→invoke→artifact-set, operator seq 107/108) and its [`wasm_host::Artifact`].
//! - [`selector`] — the artifact OUTPUT-ROUTING decision (operator seq 108/109): a caller-supplied
//!   selector program routes each invoked artifact to a [`selector::Sink`] (session-response | CAS),
//!   matching ONLY opaque `kind`/`name` strings — the host knows nothing about what produced them.
//! - [`ast_marshal`] — generic marshalling between a wasmtime component `Val` and the cadenza tagged-AST
//!   wire (operator seq 107, "binary format = AST encoding"): [`ast_marshal::val_to_ast`] turns a result
//!   `Val` of ANY WIT shape into a self-describing AST value; [`ast_marshal::ast_to_val`] is the dual
//!   (AST bytes + a WIT type → `Val`, for marshalling args IN).
//! - [`heap_marshal`] — the reducer-boundary INPUT marshalling for the option-C handle-lowered MODE (§19e):
//!   WHEN a Cadenza reducer is invoked via `apply(u32,u32,u32)->u32` over the shared `cadenza:runtime/heap`,
//!   the host marshals the `(content-type, payload, resumes)` fold inputs INTO value-heap handles via
//!   [`wasm_host::HeapHandle`]'s build ops first. A tested helper staged AHEAD of its wiring — the current
//!   live path is still `wasm_host`'s WIT-structural `bindgen!` fold.apply. Sorted-field records + sums.
//! - [`heap_unmarshal`] — the READ dual of [`heap_marshal`], same option-C handle-lowered MODE (§19e): WHEN
//!   a reducer is invoked via the handle-ABI, projects its returned `list<effect-request>` value-heap handle
//!   back into a `Vec` of [`wasm_host::EffectRequest`] (the WIT-boundary type, not the kernel's
//!   [`effect::EffectRequest`]) via [`wasm_host::HeapHandle`]'s read ops. Also staged ahead of wiring.
//!   The `kind` enum-disc field boxes like an int (→ `get-int`), NOT a sum handle (v-rust-backend
//!   authoritative); sorted-field records + `Some=0`/`None=1` option decode.
//! - [`kernel`] — the core `fold → authorize → durably-dispatch → execute → fold-result` loop, plus
//!   `replay`/`recover` (crash recovery: rebuild KV + open-obligation set from the log, no live
//!   re-execution) and `time_out_effect` (the "or time out" half of the S4 recovery contract).
//!
//! The adversarial-review invariants (design §16c) are load-bearing and enforced here, not bolted on:
//! durable dispatch before routing (S1), effect-id correlation with timeout-cancels (S4), resource-
//! scoped authorization (SEC-F1), absolute timer deadlines (S5), canonical/deterministic encoding
//! (S3/S8). Cross-version replay, multi-node, PKI, and Cadenza-native reducers are explicitly deferred.

pub mod ast_marshal;
pub mod authz;
pub mod blob;
pub mod component_store;
pub mod effect;
pub mod event;
pub mod event_ast;
pub mod executor;
pub mod hash;
pub mod heap_marshal;
pub mod heap_unmarshal;
pub mod kernel;
pub mod kv;
pub mod log_store;
pub mod name_store;
pub mod reducer;
pub mod schema_resolver;
pub mod selector;
pub mod wasm_host;
