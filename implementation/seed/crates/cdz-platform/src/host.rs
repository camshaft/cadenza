//! The wasm-runtime host (`design/cadenza-platform.md` §3) — behind the `host` feature, off by default.
//!
//! `wasmtime` instantiates a reducer component and drives it through the WIT world (`wit/world.wit`): the
//! host provides the imports — `state`, `blobs`, `identity`, and, for an event reducer, the `graph`,
//! `deliver`, and `program-of` reads — and calls the guest's `on-message`/`on-response`/`on-notification`
//! exports. Every host import is async (`async: true` below), so a disk- or network-backed backend never
//! blocks the host thread while a reducer awaits it; the guest sees the calls as ordinary.
//!
//! **Executor-agnostic.** wasmtime's async here is fiber-based and needs only *some* executor to poll its
//! futures — not tokio's reactor (a reducer component is pure compute plus host-import calls, no OS I/O). So
//! instantiating and driving a reducer works the same under tokio (production) and under the Bach discrete-
//! event simulator (deterministic tests) — verified end to end: a real component spawned, folded a message,
//! called the `identity` import, and returned its step under `bach::sim`. This is what lets the integration
//! harness drive a wasm reducer set to quiescence *deterministically* over the Bach runtime (§9).
//!
//! The module holds, bottom to top: the generated host bindings for the event-reducer world (`bindgen!`);
//! the [`HostState`] the imports read and write, with a `Host` impl per interface (`identity`, `blobs`,
//! `state`, and the privileged `graph`) backing them on the swappable [`KvStore`](crate::KvStore) /
//! [`BlobStore`](crate::BlobStore) / [`ReducerGraph`](crate::ReducerGraph); the event ↔ WIT conversion layer
//! that translates the crate's [`Message`](crate::Message)/[`Response`](crate::Response)/[`Outcome`] to and
//! from the WIT records; the [`WasmReducer`] driver that composes those around a wasmtime call to fold an
//! event; and [`WasmProgramStore`], the production [`ProgramStore`](crate::ProgramStore) that loads a
//! program's component from the content-addressed store, composes its content-addressed dependencies (the
//! value-heap runtime, …) from the store, and instantiates it as a reducer.
//!
//! The privileged event-reducer imports each read a node-shared capability threaded into [`HostState`]: the
//! routing `graph`, the `deliver` mechanism ([`Delivery`](crate::Delivery)), and the `program-of` provenance
//! read ([`Provenance`](crate::Provenance)). Each is set during node assembly (the store's `with_*` builders)
//! and defaults to a null object ([`NoDelivery`](crate::NoDelivery) / [`NoProvenance`](crate::NoProvenance)),
//! so the import path never branches on absence — an ordinary reducer is never wired these imports anyway.
#![allow(dead_code)]

// Generated host bindings for the event-reducer world (the superset: the ordinary reducer imports plus the
// privileged `graph`/`deliver`/`provenance`). The ordinary reducer world is the same guest export with the
// privileged imports absent, so this projection covers both.
wasmtime::component::bindgen!({
    world: "event-reducer-world",
    path: "wit/world.wit",
    // The store host imports (blobs.get/put, state.get/put/delete) are TRAPPABLE: with the `trappable`
    // flag their generated Host methods return `wasmtime::Result<T>`, and an `Err` traps — unwinding the
    // guest execution — instead of being lowered to a value. A genuine miss is still `Ok(None)`; only a
    // real BACKEND FAILURE (I/O, auth) traps. The guest never observes the failure (the WIT surface stays
    // option-returning); the reducer driver catches the trap, distinguishes it (a `HostBackendError`) from
    // a guest panic, and the caller aborts + retries the fold — the transaction is the unit of correctness,
    // so no state derived from a phantom read/write is ever committed. The name filters are the FULLY
    // QUALIFIED `namespace:package/interface/func` form (a bare `get` matches no lookup key and the macro
    // hard-errors on an unused rule); each keeps `async` since a name rule REPLACES (not adds to) the
    // `default` rule.
    imports: {
        default: async,
        "cadenza:platform/state/get": async | trappable,
        "cadenza:platform/state/put": async | trappable,
        "cadenza:platform/state/delete": async | trappable,
        "cadenza:platform/blobs/get": async | trappable,
        "cadenza:platform/blobs/put": async | trappable,
    },
    exports: { default: async },
    // Derive equality on the generated records/variants so the conversion layer can be asserted field-for-
    // field in tests; every WIT type here is over bytes/enums, so `PartialEq`/`Eq` are well-defined.
    additional_derives: [PartialEq, Eq],
});

// TEST-ONLY host bindings for the arg-value-capture conformance world (`wit/test/arg-probe.wit`, §9). Its
// own package (`cadenza:test-arg-probe`), physically separate from the platform world, so it can never leak
// into a real reducer's vocabulary. In its own module so its regenerated `cadenza::platform::guest` export
// bindings don't collide with the platform world's above; the host only implements the `arg-probe` import.
// The world imports `arg-probe` and re-exports `cadenza:platform/guest`, resolved cross-package from
// `wit/world.wit` (the same `--dep` the world-artifact uses), so both WIT files are on the path.
mod arg_probe_world {
    wasmtime::component::bindgen!({
        world: "cadenza:test-arg-probe/arg-probe-world",
        path: ["wit/world.wit", "wit/test/arg-probe.wit"],
        imports: { default: async },
        exports: { default: async },
        // Reuse the platform world's generated types for the shared `cadenza:platform` interfaces (reducer
        // envelope + value types) instead of regenerating distinct copies — so this world's `guest` export
        // returns the SAME `Step`/`Message`/… the reducer path uses, and both drive through one code path.
        with: {
            "cadenza:platform/reducer": crate::host::cadenza::platform::reducer,
            "cadenza:platform/types": crate::host::cadenza::platform::types,
        },
    });
}

// --- the TEST-ONLY arg-probe host import (§9 arg-value capture) ---
use crate::contract_value::{bare_ctor, record, uint_leaf, unit};
use arg_probe_world::cadenza::test_arg_probe::arg_probe as ap;
use cadenza_ast::ast::{Builder, CompoundCtor, Leaf, Radix, StructId};
use cadenza_ast::codec;

/// A signed-integer value leaf. The value model's integer is arbitrary-precision (`Leaf::Int`); the target
/// type fixes width/signedness. Used for `mixed.big` (s64) and `probe-record.tag` (s64).
fn int_leaf(b: &mut Builder, value: i64) -> StructId {
    b.atom_leaf(Leaf::Int {
        value: cadenza_ast::ast::IntValue::from_i64(value),
        radix: Radix::Dec,
    })
}

/// A `mixed` variant as its canonical bare-constructor Value form, using the CADENZA constructor names
/// (CamelCase = the bindgen variant names): `(Absent unit)` / `(Small <u8>)` / `(Big <s64>)`. A NULLARY
/// variant renders as the constructor applied to the `unit` atom — `(Absent unit)`, not `(Absent)` — matching
/// the compiler's `Value.encode` (a multi-constructor nullary variant is `(Ctor unit)`, see `codec.rs`).
fn mixed_value(b: &mut Builder, m: &ap::Mixed) -> StructId {
    match m {
        ap::Mixed::Absent => {
            let u = unit(b);
            bare_ctor(b, "Absent", vec![u])
        }
        ap::Mixed::Small(x) => {
            let p = uint_leaf(b, u64::from(*x));
            bare_ctor(b, "Small", vec![p])
        }
        ap::Mixed::Big(x) => {
            let p = int_leaf(b, *x);
            bare_ctor(b, "Big", vec![p])
        }
    }
}

/// A `narrow` variant: `(Absent unit)` / `(A <u8>)` / `(B <u16>)` — the nullary `Absent` carries the `unit`
/// atom, matching the compiler's `Value.encode` of a nullary multi-constructor variant.
fn narrow_value(b: &mut Builder, n: &ap::Narrow) -> StructId {
    match n {
        ap::Narrow::Absent => {
            let u = unit(b);
            bare_ctor(b, "Absent", vec![u])
        }
        ap::Narrow::A(x) => {
            let p = uint_leaf(b, u64::from(*x));
            bare_ctor(b, "A", vec![p])
        }
        ap::Narrow::B(x) => {
            let p = uint_leaf(b, u64::from(*x));
            bare_ctor(b, "B", vec![p])
        }
    }
}

/// Encode a received `probe-record` to canonical Value bytes: the bare `(record (= v <mixed>) (= tag <s64>))`
/// — byte-for-byte what a Cadenza checker's `Value.encode` of the same value produces. No root ascription:
/// the value-codec migration dropped the `(: value Type)` frame (decode is type-directed by the caller), so
/// the record IS the root, matching the guest's bare `Value.encode`.
fn encode_probe_record(r: &ap::ProbeRecord) -> Vec<u8> {
    let mut b = Builder::new();
    let v = mixed_value(&mut b, &r.v);
    let tag = int_leaf(&mut b, r.tag);
    let rec = record(&mut b, vec![("v", v), ("tag", tag)]);
    codec::encode(&b.finish(rec))
}

/// Encode the received `list<narrow>` to canonical Value bytes: the bare M2 NATIVE `Ctor(List)` list —
/// byte-for-byte what a Cadenza checker's `Value.encode` of the same `List(Narrow)` produces. Native, not
/// the legacy name-headed `(list …)`: the guest runtime's `decode_value` REQUIRES the native ctor-leaf head
/// and rejects a name/string head, and the checker byte-matches this against the guest's native
/// `Value.encode` — so a name-headed list neither decodes nor byte-matches (mirrors `contract_value::record`
/// / `log_value::list_value`). No root ascription: the value-codec migration dropped the `(: value (List
/// Narrow))` frame, so the native list IS the root, matching the guest's bare `Value.encode` (decode is
/// type-directed by the caller and never read the erased type token).
fn encode_narrow_list(items: &[ap::Narrow]) -> Vec<u8> {
    let mut b = Builder::new();
    let vals: Vec<StructId> = items.iter().map(|n| narrow_value(&mut b, n)).collect();
    let list = b.compound(CompoundCtor::List, &vals);
    codec::encode(&b.finish(list))
}

impl ap::Host for HostState {
    /// The TEST-ONLY `arg-probe.probe` host import (§9): encode the received `r` + `items` to canonical Value
    /// bytes and forward to the [`ArgProbeSink`], so a conformance checker asserts the marshalled ARG VALUES
    /// byte-for-byte — making a mixed-width variant-payload miscompile observable. `None` sink (the common
    /// case) → skip entirely, zero cost.
    async fn probe(&mut self, r: ap::ProbeRecord, items: Vec<ap::Narrow>) {
        if let Some(sink) = &self.arg_probe {
            sink.record(&encode_probe_record(&r), &encode_narrow_list(&items));
        }
    }
}

use crate::{
    ArgProbeSink, BlobStore, Bytes, ContractId, Delivered, Delivery, EdgeKind, Error, Hash, HostId,
    KvStore, Message, Notification, Origin, Outcome, ReducerGraph, ReducerId, RejectedSink,
    Request, ResourceLimits, Response, RunSink,
};
use std::sync::Arc;
use std::time::Duration;

// The generated WIT reducer/value types, aliased to disambiguate from the crate's own same-named types
// (`Message`, `Response`, `Notification`, `Request`, `Outcome`, `Origin`, `Error`). The conversions below
// translate between the two: the crate types the runtime speaks and the WIT records the guest folds.
use cadenza::platform::reducer as wit_reducer;
use cadenza::platform::types as wit_types;

/// A reducer-id or edge-kind crosses the WIT boundary as its raw hash bytes; a value that is not exactly
/// `Hash::LEN` bytes names nothing (`ReducerId`/`EdgeKind`'s `TryFrom<&[u8]>` rejects it), so it converts to
/// `None` and the graph op treats it as a miss. Naming the miss-on-malformed intent once keeps the graph
/// call sites reading as plain lookups.
fn to_reducer(bytes: &[u8]) -> Option<ReducerId> {
    ReducerId::try_from(bytes).ok()
}
fn to_kind(bytes: &[u8]) -> Option<EdgeKind> {
    EdgeKind::try_from(bytes).ok()
}
fn from_reducers(ids: Vec<ReducerId>) -> Vec<Vec<u8>> {
    ids.into_iter()
        .map(|id| id.hash().as_bytes().to_vec())
        .collect()
}

/// The host state threaded through a running reducer component's wasmtime store — what the host imports read
/// and write on the reducer's behalf. For now it carries the reducer's own id (the `identity` import) and the
/// content-addressed store (the `blobs` import); the key-value store and — for an event reducer — the
/// graph/deliver/provenance are added as those imports are implemented. (The `blobs` store is owned here for
/// now; wiring it to the one shared node-wide store is a later assembly step.)
struct HostState {
    /// This reducer's id (§3), returned by the `identity` import.
    id: ReducerId,
    /// The content-addressed store (§8), backing the `blobs` import.
    blobs: Box<dyn BlobStore>,
    /// The reducer's own key-value state (§7), backing the `state` import.
    kv: Box<dyn KvStore>,
    /// The one shared reducer graph (§3), backing the privileged `graph` import — an event reducer both reads
    /// and updates it to route and supervise. Shared (an `Arc`), since it is the node-wide routing substrate,
    /// not per-reducer; an ordinary reducer holds the handle but its linker never wires the `graph` import.
    graph: Arc<dyn ReducerGraph>,
    /// The node-side provenance backing the privileged `program-of` import (§4) — which program a reducer
    /// runs. Always present ([`NoProvenance`](crate::NoProvenance) when the node has not wired a real one),
    /// so the import path never branches on its absence; an ordinary reducer never has the import anyway.
    provenance: Arc<dyn Provenance>,
    /// The node-side delivery backing the privileged `deliver` import (§4) — injecting an event into a
    /// reducer's mailbox, the routing act. Always present ([`NoDelivery`](crate::NoDelivery) when the node has
    /// not wired a real one), so the import path never branches on its absence; an ordinary reducer never has
    /// the import anyway.
    delivery: Arc<dyn Delivery>,
    /// The pure-run capability backing the synchronous `run` host import (§3) — the shared [`Instantiator`],
    /// which both instantiates the sub-program and holds the pure-run memo, reached without a path back to the
    /// store (acyclic). Every reducer carries it, pure ones included: `run` sits on the ordinary world's floor
    /// and a pure program may call `run` to compose other pure programs. `None` only where no run capability is
    /// wired at all (a bare [`HostState`] built for a non-run test).
    run: Option<Arc<Instantiator>>,
    /// Where a host call the boundary REJECTED (a raw `list<u8>` arg that failed to parse into its typed
    /// id/kind) is recorded, so no host call is silently unobserved (§9). `None` when no observing node wired a
    /// recorder — the common production case — so the parse-guard path branches on the `Option` (no vtable
    /// call, no always-present `Arc`) and the raw-arg capture is dropped entirely on the disabled path.
    rejected: Option<Arc<dyn RejectedSink>>,
    /// Where a `run` host call (the pure-run primitive) is recorded — which program/contract, the input, and
    /// the result — so a conformance run can observe that a reducer invoked `run` (§9), which leaves no
    /// `step.requests` entry otherwise. `None` when no observing node wired a recorder (the run path pays zero
    /// then). Mirrors [`rejected`](Self::rejected).
    run_sink: Option<Arc<dyn RunSink>>,
    /// Where a TEST-ONLY `arg-probe.probe` host call is recorded — the received `probe-record` and
    /// `list<narrow>`, each canonical-`Value.encode`d — so an arg-value-capture conformance run asserts the
    /// marshalled ARG VALUES byte-for-byte (§9). `None` outside the arg-capture conformance world (the common
    /// case), so an ordinary reducer pays zero. Mirrors [`run_sink`](Self::run_sink).
    arg_probe: Option<Arc<dyn ArgProbeSink>>,
    /// The per-reducer linear-memory limiter the wasm store enforces (see `arm_store_safety`): a ceiling on
    /// linear memory so one guest cannot exhaust host RAM and take down the process. Lives here because a wasm
    /// [`Store`]'s limiter projects from its data (`Store::limiter`); the store enforces the limits it holds.
    /// Built from [`resource_limits`](Self::resource_limits) at assembly.
    limits: wasmtime::StoreLimits,
    /// This reducer's **effective** resource limits — the node's [`ResourceLimits`] with any per-spawn budget
    /// already resolved (`ResourceLimits::resolve_for_spawn`, clamped to the node ceiling). `arm_store_safety`
    /// reads the compute bounds (`yield_every`/`max_yields`) from here, so the store is armed with *this*
    /// reducer's budget, not a node-uniform one; [`limits`](Self::limits) is the memory half of the same.
    resource_limits: ResourceLimits,
}

impl cadenza::platform::identity::Host for HostState {
    async fn id(&mut self) -> Vec<u8> {
        self.id.hash().as_bytes().to_vec()
    }
}

impl cadenza::platform::run::Host for HostState {
    async fn run(
        &mut self,
        program: Vec<u8>,
        contract: Vec<u8>,
        input: Vec<u8>,
    ) -> Result<Vec<u8>, wit_types::Error> {
        // A malformed program/contract hash names no program — there is nothing to run, so it is a `faulted`
        // run (no answer at all), the same category as a program that crashes or never returns.
        let program =
            ProgramHash::try_from(program.as_slice()).map_err(|_| wit_types::Error::Faulted)?;
        let contract = to_contract(&contract).ok_or(wit_types::Error::Faulted)?;
        // Every reducer instantiated by the store carries the run capability (the shared instantiation core);
        // `None` only in a bare test HostState, where there is nothing to answer with — a `faulted` run.
        let inst = self.run.as_ref().ok_or(wit_types::Error::Faulted)?;
        let input = Bytes::from(input);
        let result = inst.run_pure(program, contract, input.clone()).await;
        // Record the run act (§9) — which program/contract, the input, and the outcome (Ok or a RunError
        // category) — before mapping to the WIT result, so a conformance run observes the run. `None` when no
        // recorder is wired, so a node without one pays zero here (`program`/`contract` are `Copy` ids, so this
        // reuses them after `run_pure`). `input` is a ref-counted `Bytes` (its clone is O(1)).
        if let Some(run_sink) = &self.run_sink {
            run_sink.record(
                program.hash().as_bytes(),
                contract.hash().as_bytes(),
                input.as_ref(),
                &result,
            );
        }
        match result {
            Ok(output) => Ok(output.to_vec()),
            // No program to run maps to `missing-handler`; a fault or a program that never returned is the
            // general `faulted` — the same mapping the run effect uses (§3/§4).
            Err(RunError::UnknownProgram) => Err(wit_types::Error::MissingHandler),
            Err(RunError::DidNotReturn | RunError::Faulted) => Err(wit_types::Error::Faulted),
        }
    }
}

/// A host-import BACKEND failure (a [`BlobStore`](crate::BlobStore) / [`KvStore`](crate::KvStore) I/O or
/// auth error), raised as a wasmtime trap so it UNWINDS the guest execution without the guest observing it
/// (the WIT surface stays option-returning; a failure is never lowered to a value). The reducer driver
/// distinguishes this from a guest-originated trap (panic / unreachable / abort) by downcasting the trap's
/// error: a `HostBackendError` means "the transaction could not complete — abort + retry the fold", whereas
/// a guest trap is a genuine reducer fault. See [`is_host_backend_trap`].
#[derive(Debug)]
pub struct HostBackendError {
    /// The host op that failed (e.g. `"state.get"`).
    pub op: &'static str,
    /// The backend error's message.
    pub source: String,
}

impl HostBackendError {
    /// Build the wasmtime trap error for a failed host-import backend op.
    fn trap(op: &'static str, source: &dyn std::fmt::Display) -> wasmtime::Error {
        wasmtime::Error::new(HostBackendError {
            op,
            source: source.to_string(),
        })
    }
}

impl std::fmt::Display for HostBackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "host backend error in {}: {}", self.op, self.source)
    }
}

impl std::error::Error for HostBackendError {}

/// Whether a guest-call error is a [`HostBackendError`] trap (a backend I/O/auth failure that unwound the
/// guest) — anywhere in its source chain — as opposed to a guest-originated trap. The reducer driver uses
/// this to tell the caller "abort + retry the fold" vs "the reducer faulted".
pub fn is_host_backend_trap(err: &wasmtime::Error) -> bool {
    err.chain().any(|e| e.is::<HostBackendError>())
}

impl cadenza::platform::blobs::Host for HostState {
    async fn get(&mut self, hash: Vec<u8>) -> wasmtime::Result<Option<Vec<u8>>> {
        // A malformed hash (not exactly `Hash::LEN` bytes) names nothing, so it reads back as absent
        // (`Ok(None)`) — a total, non-trapping outcome, not a backend error.
        let Some(hash) = <[u8; Hash::LEN]>::try_from(hash.as_slice())
            .ok()
            .map(Hash::from_bytes)
        else {
            return Ok(None);
        };
        // A genuine miss is `Ok(None)`; a BACKEND ERROR traps (unwinds the guest) rather than masquerading
        // as absent — the guest never sees it; the driver classifies the trap + the fold retries.
        match self.blobs.get(hash).await {
            Ok(found) => Ok(found.map(|bytes| bytes.to_vec())),
            Err(e) => Err(HostBackendError::trap("blobs.get", &e)),
        }
    }

    async fn put(&mut self, bytes: Vec<u8>) -> wasmtime::Result<Vec<u8>> {
        // A failed persist TRAPS (unwinds the guest) instead of silently returning the hash of un-stored
        // bytes — a phantom write must not let the fold commit.
        match self.blobs.put(Bytes::from(bytes)).await {
            Ok(hash) => Ok(hash.as_bytes().to_vec()),
            Err(e) => Err(HostBackendError::trap("blobs.put", &e)),
        }
    }
}

impl cadenza::platform::state::Host for HostState {
    async fn get(&mut self, key: Vec<u8>) -> wasmtime::Result<Option<Vec<u8>>> {
        // `Ok(None)` = genuine miss; `Err` = backend failure → trap (unwind the guest), never a phantom miss.
        match self.kv.get(&key).await {
            Ok(found) => Ok(found.map(|value| value.to_vec())),
            Err(e) => Err(HostBackendError::trap("state.get", &e)),
        }
    }

    async fn put(&mut self, key: Vec<u8>, value: Vec<u8>) -> wasmtime::Result<()> {
        match self.kv.put(Bytes::from(key), Bytes::from(value)).await {
            Ok(()) => Ok(()),
            Err(e) => Err(HostBackendError::trap("state.put", &e)),
        }
    }

    async fn delete(&mut self, key: Vec<u8>) -> wasmtime::Result<()> {
        // The WIT `delete` reports nothing; the store's whether-it-was-present is not surfaced. A backend
        // failure traps.
        match self.kv.delete(&key).await {
            Ok(_existed) => Ok(()),
            Err(e) => Err(HostBackendError::trap("state.delete", &e)),
        }
    }
}

// Each method parses its `list<u8>` node/edge-kind arg into a `ReducerId`/`EdgeKind`; on a malformed one it
// returns the empty/false result (total, graceful) AND records the rejected call to `self.rejected` with the
// raw argument bytes, so the call is observed (§9) even though it never reached the recordable `self.graph`
// capability below the parse. A well-formed call records via the graph decorator as usual; only the rejected
// path is recorded here. The record is gated on `self.rejected` being `Some`, so when no recorder is wired the
// parse-guard path pays ZERO allocation; when it is, the raw `Vec<u8>` args move into `Bytes` with no copy
// (`Bytes::from` is O(1)). `iface`/`op` name the WIT interface + method.
impl cadenza::platform::graph::Host for HostState {
    async fn insert(&mut self, node: Vec<u8>) -> bool {
        match to_reducer(&node) {
            Some(node) => self.graph.insert(node).await,
            None => {
                if let Some(rejected) = &self.rejected {
                    rejected.record("graph", "insert", &[Bytes::from(node)]);
                }
                false
            }
        }
    }

    async fn contains(&mut self, node: Vec<u8>) -> bool {
        match to_reducer(&node) {
            Some(node) => self.graph.contains(node).await,
            None => {
                if let Some(rejected) = &self.rejected {
                    rejected.record("graph", "contains", &[Bytes::from(node)]);
                }
                false
            }
        }
    }

    async fn remove(&mut self, node: Vec<u8>) -> bool {
        match to_reducer(&node) {
            Some(node) => self.graph.remove(node).await,
            None => {
                if let Some(rejected) = &self.rejected {
                    rejected.record("graph", "remove", &[Bytes::from(node)]);
                }
                false
            }
        }
    }

    async fn link(&mut self, source: Vec<u8>, target: Vec<u8>, kind: Vec<u8>) -> bool {
        match (to_reducer(&source), to_reducer(&target), to_kind(&kind)) {
            (Some(source), Some(target), Some(kind)) => self.graph.link(source, target, kind).await,
            _ => {
                if let Some(rejected) = &self.rejected {
                    rejected.record(
                        "graph",
                        "link",
                        &[Bytes::from(source), Bytes::from(target), Bytes::from(kind)],
                    );
                }
                false
            }
        }
    }

    async fn set_edges(
        &mut self,
        source: Vec<u8>,
        kind: Vec<u8>,
        targets: Vec<Vec<u8>>,
    ) -> Vec<Vec<u8>> {
        let (Some(source_id), Some(kind_id)) = (to_reducer(&source), to_kind(&kind)) else {
            if let Some(rejected) = &self.rejected {
                let mut raw = vec![Bytes::from(source), Bytes::from(kind)];
                raw.extend(targets.into_iter().map(Bytes::from));
                rejected.record("graph", "set-edges", &raw);
            }
            return Vec::new();
        };
        // A malformed target names nothing, so it is dropped from the chain rather than aborting the set.
        let targets = targets.iter().filter_map(|t| to_reducer(t)).collect();
        from_reducers(self.graph.set_edges(source_id, kind_id, targets).await)
    }

    async fn neighbors(
        &mut self,
        node: Vec<u8>,
        kind: Vec<u8>,
        dir: cadenza::platform::graph::Dir,
    ) -> Vec<Vec<u8>> {
        let (Some(node_id), Some(kind_id)) = (to_reducer(&node), to_kind(&kind)) else {
            if let Some(rejected) = &self.rejected {
                rejected.record(
                    "graph",
                    "neighbors",
                    &[Bytes::from(node), Bytes::from(kind)],
                );
            }
            return Vec::new();
        };
        from_reducers(self.graph.neighbors(node_id, kind_id, dir.into()).await)
    }

    async fn in_kinds(&mut self, node: Vec<u8>) -> Vec<Vec<u8>> {
        match to_reducer(&node) {
            Some(node) => self
                .graph
                .in_kinds(node)
                .await
                .into_iter()
                .map(|kind| kind.hash().as_bytes().to_vec())
                .collect(),
            None => {
                if let Some(rejected) = &self.rejected {
                    rejected.record("graph", "in-kinds", &[Bytes::from(node)]);
                }
                Vec::new()
            }
        }
    }

    async fn reach(
        &mut self,
        node: Vec<u8>,
        kind: Vec<u8>,
        dir: cadenza::platform::graph::Dir,
    ) -> Vec<Vec<u8>> {
        let (Some(node_id), Some(kind_id)) = (to_reducer(&node), to_kind(&kind)) else {
            if let Some(rejected) = &self.rejected {
                rejected.record("graph", "reach", &[Bytes::from(node), Bytes::from(kind)]);
            }
            return Vec::new();
        };
        from_reducers(self.graph.reach(node_id, kind_id, dir.into()).await)
    }
}

impl cadenza::platform::provenance::Host for HostState {
    async fn program_of(&mut self, reducer: Vec<u8>) -> Vec<u8> {
        // The WIT returns a program hash unconditionally; empty bytes encode absence — a malformed id or a
        // reducer that is not running (or, under NoProvenance, always). The guest reads empty as "no
        // provenance" (a well-formed program hash is never empty).
        let Ok(reducer_id) = ReducerId::try_from(reducer.as_slice()) else {
            if let Some(rejected) = &self.rejected {
                rejected.record("provenance", "program-of", &[Bytes::from(reducer)]);
            }
            return Vec::new();
        };
        match self.provenance.program_of(reducer_id).await {
            Some(program) => program.hash().as_bytes().to_vec(),
            None => Vec::new(),
        }
    }
}

// The deliver ops parse their `target` `list<u8>` into a `ReducerId` (and decode the WIT envelope); on a
// malformed one they return `false` AND record the rejected call to `self.rejected` (iface `deliver`, the op,
// the raw `target` bytes), so a malformed routing act is observed (§9) rather than silently dropped — the same
// completeness the graph ops get. The envelope is a structured WIT record, not a raw `list<u8>`, so only the
// raw `target` id is captured. Gated on `self.rejected` being `Some` (zero alloc when no recorder) with a zero-copy `Bytes::from`.
impl cadenza::platform::deliver::Host for HostState {
    async fn deliver_message(&mut self, target: Vec<u8>, event: wit_reducer::Message) -> bool {
        // A malformed target or a malformed event (an id that is not a hash, an origin that is not) names
        // nothing to deliver to or from, so it is a failed delivery — `false` — not a panic. The node-side
        // delivery reports whether a reducer is running under `target` and received it.
        let (Some(target_id), Some(message)) = (to_reducer(&target), message_from_wit(event))
        else {
            if let Some(rejected) = &self.rejected {
                rejected.record("deliver", "deliver-message", &[Bytes::from(target)]);
            }
            return false;
        };
        self.delivery
            .deliver(target_id, Delivered::Message(message))
            .await
    }

    async fn deliver_response(&mut self, target: Vec<u8>, event: wit_reducer::Response) -> bool {
        let (Some(target_id), Some(response)) = (to_reducer(&target), response_from_wit(event))
        else {
            if let Some(rejected) = &self.rejected {
                rejected.record("deliver", "deliver-response", &[Bytes::from(target)]);
            }
            return false;
        };
        self.delivery
            .deliver(target_id, Delivered::Response(response))
            .await
    }

    async fn deliver_notification(
        &mut self,
        target: Vec<u8>,
        event: wit_reducer::Notification,
    ) -> bool {
        let (Some(target_id), Some(notification)) =
            (to_reducer(&target), notification_from_wit(event))
        else {
            if let Some(rejected) = &self.rejected {
                rejected.record("deliver", "deliver-notification", &[Bytes::from(target)]);
            }
            return false;
        };
        self.delivery
            .deliver(target_id, Delivered::Notification(notification))
            .await
    }
}

impl From<cadenza::platform::graph::Dir> for crate::Dir {
    fn from(dir: cadenza::platform::graph::Dir) -> Self {
        match dir {
            cadenza::platform::graph::Dir::Outgoing => crate::Dir::Out,
            cadenza::platform::graph::Dir::Incoming => crate::Dir::In,
        }
    }
}

// ── The event ↔ WIT conversion layer (§3) ───────────────────────────────────────────────────────────────
// Driving a reducer component is: build the WIT event record the guest folds, call its export, and read back
// the WIT `step` it returns. The runtime speaks the crate's own strongly-typed events (`Message`, `Response`,
// `Notification`, `Request`, `Outcome`); the guest speaks the generated WIT records. These functions are the
// one place the two meet — the driver (the following slice) composes them around a wasmtime call, so the
// mapping is written and tested once here, independent of any guest.
//
// A typed id crosses the boundary as its raw hash bytes (§8). Outbound (crate → WIT) is total: a crate id is
// always a well-formed hash. Inbound (WIT → crate, decoding a guest's step) is fallible: a guest could emit a
// contract-id that is not `Hash::LEN` bytes, and the driver rejects the whole step rather than trusting it.

/// A reducer's step could not be decoded: the guest emitted bytes that name no valid id. The driver treats a
/// malformed step as a misbehaving guest and rejects it (rather than panicking); it never arises from a
/// well-formed component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StepError {
    /// A contract-id — on an emitted request, or a close reason's schema — was not exactly `Hash::LEN` bytes.
    MalformedContractId,
}

/// A contract-id read back from the guest: its raw hash bytes, or `None` if they do not name a hash.
fn to_contract(bytes: &[u8]) -> Option<ContractId> {
    Some(ContractId::from_hash(Hash::from_bytes(
        <[u8; Hash::LEN]>::try_from(bytes).ok()?,
    )))
}

// ── Outbound: the events the host delivers into the guest ──

fn origin_to_wit(origin: Origin) -> wit_types::Origin {
    wit_types::Origin {
        reducer: origin.reducer.hash().as_bytes().to_vec(),
        host: origin.host.hash().as_bytes().to_vec(),
    }
}

fn error_to_wit(error: Error) -> wit_types::Error {
    match error {
        Error::Timeout => wit_types::Error::Timeout,
        Error::MissingHandler => wit_types::Error::MissingHandler,
        Error::SchemaViolation => wit_types::Error::SchemaViolation,
        Error::Faulted => wit_types::Error::Faulted,
    }
}

fn message_to_wit(message: &Message) -> wit_reducer::Message {
    wit_reducer::Message {
        contract: message.id.hash().as_bytes().to_vec(),
        sender: origin_to_wit(message.from),
        payload: message.payload.to_vec(),
        token: message.continuation_token.to_vec(),
    }
}

fn response_to_wit(response: &Response) -> wit_reducer::Response {
    wit_reducer::Response {
        contract: response.id.hash().as_bytes().to_vec(),
        token: response.continuation_token.to_vec(),
        // A handler's domain error is an ordinary output value in `Ok`; only a runtime-level `Error` is `Err`.
        answer: match &response.payload {
            Ok(payload) => Ok(payload.to_vec()),
            Err(error) => Err(error_to_wit(*error)),
        },
    }
}

fn notification_to_wit(notification: &Notification) -> wit_reducer::Notification {
    wit_reducer::Notification {
        contract: notification.id.hash().as_bytes().to_vec(),
        payload: notification.payload.to_vec(),
    }
}

// ── Inbound: the step the guest returns ──

fn request_from_wit(request: wit_reducer::Request) -> Result<Request, StepError> {
    Ok(Request {
        id: to_contract(&request.contract).ok_or(StepError::MalformedContractId)?,
        payload: Bytes::from(request.payload),
        continuation_token: Bytes::from(request.token),
        // The WIT deadline is nanoseconds so the ABI carries no `Duration`; `None` is no deadline.
        deadline: request.deadline_nanos.map(Duration::from_nanos),
    })
}

fn outcome_from_wit(outcome: wit_reducer::Outcome) -> Result<Outcome, StepError> {
    match outcome {
        wit_reducer::Outcome::Continue => Ok(Outcome::Continue),
        wit_reducer::Outcome::Close(closed) => Ok(Outcome::Break {
            schema: to_contract(&closed.schema).ok_or(StepError::MalformedContractId)?,
            reason: Bytes::from(closed.reason),
        }),
    }
}

/// Decode a guest's [`step`](wit_reducer::Step) into the crate's `(requests, outcome)` product. Fails if any
/// emitted id is malformed; a well-formed guest never trips this.
fn step_from_wit(step: wit_reducer::Step) -> Result<(Vec<Request>, Outcome), StepError> {
    let requests = step
        .requests
        .into_iter()
        .map(request_from_wit)
        .collect::<Result<Vec<_>, _>>()?;
    Ok((requests, outcome_from_wit(step.outcome)?))
}

// ── Inbound: an event a privileged reducer hands to `deliver` ──
// The `deliver` host import (§4) takes a WIT event an event reducer built — the same three envelopes it would
// receive — and injects it into a target's log. These convert that WIT event back to the crate event the
// system delivers. Fallible on a malformed id (a contract-id, or an origin's reducer/host, not `Hash::LEN`
// bytes): the event names nothing, so the delivery fails (`false`) rather than trusting a bad value. The
// inverse of the outbound `*_to_wit` above, for the events that also flow the other way.

fn origin_from_wit(origin: wit_types::Origin) -> Option<Origin> {
    Some(Origin {
        reducer: ReducerId::try_from(origin.reducer.as_slice()).ok()?,
        host: HostId::try_from(origin.host.as_slice()).ok()?,
    })
}

fn error_from_wit(error: wit_types::Error) -> Error {
    match error {
        wit_types::Error::Timeout => Error::Timeout,
        wit_types::Error::MissingHandler => Error::MissingHandler,
        wit_types::Error::SchemaViolation => Error::SchemaViolation,
        wit_types::Error::Faulted => Error::Faulted,
    }
}

fn message_from_wit(message: wit_reducer::Message) -> Option<Message> {
    Some(Message {
        id: to_contract(&message.contract)?,
        payload: Bytes::from(message.payload),
        from: origin_from_wit(message.sender)?,
        continuation_token: Bytes::from(message.token),
    })
}

fn response_from_wit(response: wit_reducer::Response) -> Option<Response> {
    Some(Response {
        id: to_contract(&response.contract)?,
        continuation_token: Bytes::from(response.token),
        // The mirror of `response_to_wit`: an `Ok` payload is the contract's output value; an `Err` is a
        // runtime-level failure, total across the three `Error` variants.
        payload: match response.answer {
            Ok(payload) => Ok(Bytes::from(payload)),
            Err(error) => Err(error_from_wit(error)),
        },
    })
}

fn notification_from_wit(notification: wit_reducer::Notification) -> Option<Notification> {
    Some(Notification {
        id: to_contract(&notification.contract)?,
        payload: Bytes::from(notification.payload),
    })
}

// ── The wasm reducer driver (§3) ─────────────────────────────────────────────────────────────────────────
// Turning a reducer component into a live [`Reducer`](crate::Reducer): a wasmtime `Store` holding the
// component's [`HostState`] and an instantiated world. Folding an event is build-the-record → call-the-guest
// → decode-the-step, composing the conversions above. The host imports are async (the `bindgen!` above), so
// instantiation and every fold run on an async store — a disk/network-backed backend an import awaits never
// blocks the host thread.

use crate::{
    HashTag, ProgramHash, ProgramStore, Provenance, Reducer, ReducerKind, RunError, SpawnContext,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use wasmtime::component::types::ComponentItem;
use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, InstanceAllocationStrategy, PoolingAllocationConfig, Store};

/// The wasmtime [`Engine`] reducer components are compiled and run on: async (host imports may await) with the
/// component model enabled. One engine is shared by every reducer on a host — it is the compilation context,
/// cheap to clone, and holds no per-instance state.
///
/// **Instance allocation is POOLING, not the on-demand default.** The on-demand allocator `mmap`s a FRESH
/// linear memory for every reducer instantiation (every fold). Under a burst of concurrent independent folds
/// those per-fold `mmap`s serialize on the process-wide `mmap_lock`, a kernel convoy that (measured on the live
/// gateway rig) capped effective fold concurrency at ~8 regardless of the 64 available cores AND added ~26ms of
/// kernel syscall time to the per-request floor even with zero contention. The pooling allocator instead
/// pre-reserves a slab of linear-memory slots ONCE and REUSES them across folds (a slot is reset via `madvise`,
/// not a fresh `mmap`), so a burst of independent folds no longer contends the `mmap_lock`. This is purely an
/// allocation-strategy change: it parallelizes INDEPENDENT folds across pre-allocated slots and does not change
/// determinism — a single session's fold is still driven serially through `&mut Store`.
///
/// The pool is sized from the node's [`ResourceLimits`]: each pooled memory slot's growth ceiling
/// ([`PoolingAllocationConfig::max_memory_size`]) is the node's per-reducer `max_linear_memory_bytes` (the same
/// ceiling `reducer_store_limits`/`arm_store_safety` already enforce), and the per-slot address-space
/// RESERVATION ([`Config::memory_reservation`]) stays at least 4 GiB so wasm32 linear-memory bounds checks are
/// still elided. The concurrent-slot COUNTS keep wasmtime's vetted defaults (1000 each), which far exceed any
/// plausible gateway fold burst (target concurrency is ≤64) and absorb the extra core-instances/memories a
/// reducer that composes content-addressed deps (the value-heap runtime) transitively holds — the per-component
/// caps are left unlimited, so composition never fails to instantiate.
fn reducer_engine(limits: &ResourceLimits) -> Result<Engine, wasmtime::Error> {
    let mut config = Config::new();
    // Async so an awaiting host import (a disk/network KV or blob read) parks only the reducer, not the host
    // thread (§3/§9).
    config.async_support(true);
    config.wasm_component_model(true);
    // Epoch-based interruption so a long-running guest fold cannot monopolize an executor thread or stall the
    // runtime: with a periodic epoch ticker (driven per-runtime, see `ProgramStore::epoch_incrementer` +
    // `Runtime::drives_epoch_ticker`), each store's epoch deadline (see `arm_store_safety`) makes the guest
    // yield to the executor, and past a bound trap — a single runaway program then fails cleanly (a per-reducer
    // Crashed, §7) rather than taking down tokio. Cheap when un-ticked (an atomic the compiled code checks at
    // loop backedges/calls), so it is always enabled; only the ticker is runtime-gated.
    config.epoch_interruption(true);

    // Pooling instance allocator (see the doc above): pre-reserve + reuse a slab of linear-memory slots instead
    // of mmap'ing fresh memory per fold, removing the process-mmap_lock convoy that capped fold concurrency and
    // added the ~26ms/request kernel floor.
    let mem_ceiling = limits.max_linear_memory_bytes as u64;
    // Keep the per-slot address reservation at least 4 GiB — wasmtime's 64-bit default — so wasm32 memories skip
    // bounds checks; only grow it if a node configures a larger per-reducer ceiling (max_memory_size must be
    // <= memory_reservation, else Engine::new rejects the config).
    config.memory_reservation(mem_ceiling.max(4 * 1024 * 1024 * 1024));
    let mut pool = PoolingAllocationConfig::new();
    // Cap each pooled memory's growth ceiling to the node's per-reducer linear-memory limit — the same ceiling
    // arm_store_safety enforces via StoreLimits, so growth past it is rejected consistently. The reservation
    // above still elides bounds checks; this only bounds how far a memory may commit.
    pool.max_memory_size(limits.max_linear_memory_bytes);
    config.allocation_strategy(InstanceAllocationStrategy::Pooling(pool));

    Engine::new(&config)
}

/// Build the per-reducer wasm memory limits from the node's [`ResourceLimits`] (see `arm_store_safety`): bound
/// each linear memory to `limits.max_linear_memory_bytes` and trap on a growth that would exceed it.
/// Instance/table/memory counts keep wasmtime's finite defaults. The value comes from config, never a
/// hard-coded cap.
fn reducer_store_limits(limits: &ResourceLimits) -> wasmtime::StoreLimits {
    wasmtime::StoreLimitsBuilder::new()
        .memory_size(limits.max_linear_memory_bytes)
        .trap_on_grow_failure(true)
        .build()
}

/// Arm `store` with the per-reducer safety limits from the node's [`ResourceLimits`] so no single guest can
/// take down the host — the two ways one program could: monopolizing compute, and exhausting memory. Applied
/// to every reducer store (ordinary, event, and pure-run). Every bound comes from `limits` (config), never a
/// hard-coded module constant.
///
/// - **Compute (epoch preemption):** yield to the async executor every `limits.yield_every` epoch ticks of
///   guest compute (so a long fold can never monopolize a thread), and trap once a fold has yielded
///   `limits.max_yields` times (its cumulative compute budget). Inert until the engine's epoch is actually
///   ticked — the production runtime drives the ticker (at `limits.epoch_tick`); under the deterministic
///   simulator the epoch never advances, so a guest runs un-preempted with the harness's own wall-clock
///   timeout as the backstop.
/// - **Memory:** enforce the store's [`reducer_store_limits`] (the linear-memory ceiling), projecting from the
///   `HostState`'s own `limits` field (a wasm store's limiter must live in its data), built from the same
///   config at construction.
///
/// A breach of either bound traps, which the fold path turns into a per-reducer `Crashed` (§7), never a
/// process-wide failure.
///
/// The bounds come from the store's own [`HostState::resource_limits`] — this reducer's *effective* limits,
/// already resolved from any per-spawn request clamped to the node ceiling (`resolve_for_spawn`). So each
/// store is armed with its own budget, not a node-uniform value; the memory half is the `HostState::limits`
/// limiter, built from the same effective limits.
fn arm_store_safety(store: &mut Store<HostState>) {
    let limits = store.data().resource_limits;
    store.set_epoch_deadline(limits.yield_every);
    let yield_every = limits.yield_every;
    let mut yields_left = limits.max_yields;
    store.epoch_deadline_callback(move |_ctx| {
        if yields_left == 0 {
            // Budget exhausted — a runaway fold. Trap: the fold's `call_async` returns an error, which the
            // fold path turns into a per-reducer Crashed (§7), never a process-wide failure.
            Ok(wasmtime::UpdateDeadline::Interrupt)
        } else {
            yields_left -= 1;
            // Yield control to the async executor (so other tasks — and other reducers — make progress) and
            // extend the deadline for the next slice of this fold's compute.
            Ok(wasmtime::UpdateDeadline::Yield(yield_every))
        }
    });
    store.limiter(|host_state| &mut host_state.limits);
}

/// Wire the host imports a reducer of the given [`ReducerKind`] may hold into `linker`, each backed by the
/// [`HostState`] in the store. The kind decides the capability set (§3 trust root): EVERY reducer gets its own
/// state, the content-addressed store, and its own id, but only an event reducer gets the privileged imports —
/// the routing `graph`, the `deliver` primitive, and the `program-of` provenance read. This is the
/// least-privilege wiring the world design rests on: an ordinary reducer's linker simply has no `graph`
/// import, so a component that tries to import it fails to instantiate against that linker (the capability is
/// enforced by what the kernel wires, never a runtime check an ordinary reducer could attempt).
fn add_host_imports(
    linker: &mut Linker<HostState>,
    kind: ReducerKind,
) -> Result<(), wasmtime::Error> {
    // `run` is on the floor for EVERY reducer, a pure one included: a pure, deterministic, empty-effect
    // sub-run grants nothing observable, so a pure component may still call `run` to compose *other* pure
    // programs (§3) and stay pure itself. It is the only import a pure reducer gets — the rest of the empty
    // capability set (no state, no blobs, no peer, no timer, no durable write) is what keeps a pure run's
    // output a pure function of its input, so its memoization is sound; a component that tries to import
    // anything else fails to instantiate against the pure linker.
    cadenza::platform::run::add_to_linker::<_, HostData>(linker, |s| s)?;
    if matches!(kind, ReducerKind::Pure) {
        return Ok(());
    }
    // The rest of the floor every non-pure reducer stands on — its own state, the content-addressed store,
    // and its id.
    cadenza::platform::identity::add_to_linker::<_, HostData>(linker, |s| s)?;
    cadenza::platform::blobs::add_to_linker::<_, HostData>(linker, |s| s)?;
    cadenza::platform::state::add_to_linker::<_, HostData>(linker, |s| s)?;
    // Privileged: only an event reducer may read and mutate the routing substrate, read program provenance,
    // and deliver an event into a reducer's log (the routing act, §4).
    if matches!(kind, ReducerKind::Event) {
        cadenza::platform::graph::add_to_linker::<_, HostData>(linker, |s| s)?;
        cadenza::platform::provenance::add_to_linker::<_, HostData>(linker, |s| s)?;
        cadenza::platform::deliver::add_to_linker::<_, HostData>(linker, |s| s)?;
    }
    Ok(())
}

/// The `HasData` marker tying the generated host-import traits to [`HostState`] as the store data: every
/// import reads and writes the one `HostState` the store holds, so the projection is the identity.
struct HostData;
impl wasmtime::component::HasData for HostData {
    type Data<'a> = &'a mut HostState;
}

/// The engine and the wired host-import linkers — built once and shared by every reducer on a host. Nothing
/// here varies per reducer: the engine is the shared compilation context, and each linker is a fixed
/// capability set. There is one linker PER [`ReducerKind`] — the least-privilege split (§3): the ordinary
/// linker wires only state/blobs/identity, the event linker adds the privileged `graph` (and, later,
/// deliver/provenance). Both are built once and reused; instantiating against the linker for a reducer's kind
/// is what enforces its capabilities (an ordinary reducer instantiated against the ordinary linker cannot
/// resolve a `graph` import, so it simply cannot hold that capability).
///
/// Instantiation then reuses as much as possible: `preinstantiate` resolves a component's imports against the
/// kind's linker ONCE (an [`EventReducerWorldPre`] — the reusable, import-resolved form), and each reducer is a
/// cheap `instantiate` on a fresh store from that. What is NOT shared is the [`Store`]: it holds the instance's
/// live state — its [`HostState`] and the guest's linear memory — so it is inherently per-reducer. (The
/// per-program `Component`/[`EventReducerWorldPre`] is cached a layer up, by the program store keyed on the
/// program hash, so even `preinstantiate` runs once per program, not once per reducer.)
struct ReducerHost {
    engine: Engine,
    ordinary_linker: Linker<HostState>,
    event_linker: Linker<HostState>,
    /// The linker a pure [`run`](crate::Runner) reducer instantiates against — only the `run` import, so it
    /// may compose other pure programs but any effect, state, or world access it attempts cannot even resolve
    /// (§3 otherwise-empty capability set).
    pure_linker: Linker<HostState>,
    /// The linker a TEST-ONLY arg-probe-world guest (§9) instantiates against: only the `arg-probe` import
    /// (the guest exports the reducer `guest` but takes no platform capabilities). Its content-addressed deps
    /// (the value-heap runtime) compose per-spawn like any guest. Built like the others but never wired for a
    /// real reducer — reached only when a component imports `arg-probe`.
    arg_probe_linker: Linker<HostState>,
    /// The node's per-reducer resource limits (compute budget + memory ceiling + epoch tick) this host arms
    /// every reducer store with (`arm_store_safety`). Set once at assembly from the node's config — never a
    /// hard-coded cap.
    limits: ResourceLimits,
}

impl ReducerHost {
    /// Build the shared engine and wire one linker per reducer kind — once per host — carrying the node's
    /// resource `limits` to arm each reducer store with.
    fn new(limits: ResourceLimits) -> Result<Self, wasmtime::Error> {
        let engine = reducer_engine(&limits)?;
        let mut ordinary_linker = Linker::new(&engine);
        add_host_imports(&mut ordinary_linker, ReducerKind::Ordinary)?;
        let mut event_linker = Linker::new(&engine);
        add_host_imports(&mut event_linker, ReducerKind::Event)?;
        let mut pure_linker = Linker::new(&engine);
        add_host_imports(&mut pure_linker, ReducerKind::Pure)?; // wires only `run` — otherwise empty (§3)
        let mut arg_probe_linker = Linker::new(&engine);
        arg_probe_world::cadenza::test_arg_probe::arg_probe::add_to_linker::<_, HostData>(
            &mut arg_probe_linker,
            |s| s,
        )?;
        Ok(Self {
            engine,
            ordinary_linker,
            event_linker,
            pure_linker,
            arg_probe_linker,
            limits,
        })
    }

    /// The linker holding exactly the capabilities a reducer of `kind` is allowed.
    fn linker_for(&self, kind: ReducerKind) -> &Linker<HostState> {
        match kind {
            ReducerKind::Ordinary => &self.ordinary_linker,
            ReducerKind::Event => &self.event_linker,
            ReducerKind::Pure => &self.pure_linker,
        }
    }

    /// Resolve `component`'s imports against the linker for `kind` once, yielding the reusable pre-instantiated
    /// world. A component that imports more than its kind is granted (an ordinary reducer importing `graph`)
    /// fails here — the capability split is enforced at link time. A program store caches this keyed on the
    /// program hash, so the import-linking work happens once per program; every reducer of that program then
    /// instantiates cheaply from it.
    fn preinstantiate(
        &self,
        component: &Component,
        kind: ReducerKind,
    ) -> Result<EventReducerWorldPre<HostState>, wasmtime::Error> {
        EventReducerWorldPre::new(self.linker_for(kind).instantiate_pre(component)?)
    }

    /// Instantiate a live reducer from a pre-instantiated world, backing its host imports with `host`. Only
    /// this per-reducer step allocates a fresh [`Store`] (the reducer's own state); the engine, linker, and
    /// `pre` are all shared. Async because the component model instantiates on an async store.
    async fn instantiate(
        &self,
        pre: &EventReducerWorldPre<HostState>,
        host: HostState,
    ) -> Result<WasmReducer, wasmtime::Error> {
        let mut store = Store::new(&self.engine, host);
        arm_store_safety(&mut store);
        let world = pre.instantiate_async(&mut store).await?;
        // The fast path composes NO content-addressed runtime (a no-deps guest), so there is no heap instance
        // to retain for the census.
        Ok(WasmReducer::Reducer {
            store,
            world,
            heap: None,
        })
    }
}

/// A [`Reducer`](crate::Reducer) backed by a wasm component: the wasmtime `Store` carrying its [`HostState`]
/// and the instantiated event-reducer world. Each folded event builds the WIT record, calls the matching
/// guest export, and decodes the returned step. It is `Send` but not `Sync` (a `Store` is not `Sync`) — which
/// is exactly what [`Reducer`](crate::Reducer) requires, since the runtime moves a reducer into its own task
/// and drives it only through `&mut` from there. Built by [`ReducerHost::instantiate`].
enum WasmReducer {
    /// A reducer-world guest — the ordinary/event/pure worlds (the production path).
    Reducer {
        store: Store<HostState>,
        world: EventReducerWorld,
        /// The composed value-heap runtime instance, retained so a debug leak-census can read its
        /// `live-objects` export ([`live_object_census`](WasmReducer::live_object_census)). `None` for a
        /// no-dependency guest (the fast path composes no runtime) or when capture was not requested. Cheap to
        /// hold — a wasmtime component [`Instance`](wasmtime::component::Instance) is a lightweight store
        /// handle; the instance itself lives in the store regardless, so retaining the handle costs nothing.
        heap: Option<wasmtime::component::Instance>,
    },
    /// A TEST-ONLY arg-probe-world guest (§9 arg-value capture): exports the same reducer `guest` (so the
    /// harness drives it identically) but imports `arg-probe` instead of the platform capabilities.
    ArgProbe {
        store: Store<HostState>,
        world: arg_probe_world::ArgProbeWorld,
    },
}

// The three entry points share the same shape: encode the event, call the guest, decode the step. A wasm
// trap or a guest returning a malformed step is a failed fold — it panics, which the system's per-fold
// `catch_unwind` turns into the reducer's `Crashed` lifecycle event (§7), the same as any other fold failure.
#[async_trait]
impl Reducer for WasmReducer {
    async fn on_message(
        &mut self,
        message: Message,
    ) -> Result<(Vec<Request>, Outcome), crate::ReducerFault> {
        let event = message_to_wit(&message);
        // Both worlds export the same `cadenza:platform/guest`, so the call is identical bar the world type.
        let call = match self {
            WasmReducer::Reducer { store, world, .. } => {
                world
                    .cadenza_platform_guest()
                    .call_on_message(store, &event)
                    .await
            }
            WasmReducer::ArgProbe { store, world } => {
                world
                    .cadenza_platform_guest()
                    .call_on_message(store, &event)
                    .await
            }
        };
        fold_result("on_message", call)
    }

    async fn on_response(
        &mut self,
        response: Response,
    ) -> Result<(Vec<Request>, Outcome), crate::ReducerFault> {
        let event = response_to_wit(&response);
        let call = match self {
            WasmReducer::Reducer { store, world, .. } => {
                world
                    .cadenza_platform_guest()
                    .call_on_response(store, &event)
                    .await
            }
            WasmReducer::ArgProbe { store, world } => {
                world
                    .cadenza_platform_guest()
                    .call_on_response(store, &event)
                    .await
            }
        };
        fold_result("on_response", call)
    }

    async fn on_notification(
        &mut self,
        notification: Notification,
    ) -> Result<(Vec<Request>, Outcome), crate::ReducerFault> {
        let event = notification_to_wit(&notification);
        let call = match self {
            WasmReducer::Reducer { store, world, .. } => {
                world
                    .cadenza_platform_guest()
                    .call_on_notification(store, &event)
                    .await
            }
            WasmReducer::ArgProbe { store, world } => {
                world
                    .cadenza_platform_guest()
                    .call_on_notification(store, &event)
                    .await
            }
        };
        fold_result("on_notification", call)
    }

    /// Read the composed value-heap runtime's `live-objects` census (mirrors cdz-run's `read_live_objects`,
    /// but async since the platform store is async). `None` unless this reducer retained a composed heap
    /// instance (the compose path) whose runtime exposes the census export (the debug-counters build). Reads
    /// the retained instance directly — the export is not re-exported by the guest world, so this handle is
    /// the only way to reach it (see [`Instantiator::bind_dependencies`]).
    async fn live_object_census(&mut self) -> Option<u32> {
        let WasmReducer::Reducer { store, heap, .. } = self else {
            return None;
        };
        let heap = heap.as_ref()?;
        let iface = heap.get_export_index(&mut *store, None, "cadenza:runtime/heap")?;
        let idx = heap.get_export_index(&mut *store, Some(&iface), "live-objects")?;
        let func = heap.get_func(&mut *store, idx)?;
        let mut out = [wasmtime::component::Val::U32(0)];
        func.call_async(&mut *store, &[], &mut out).await.ok()?;
        func.post_return_async(&mut *store).await.ok()?;
        match out.into_iter().next()? {
            wasmtime::component::Val::U32(n) => Some(n),
            _ => None,
        }
    }

    /// Arm/disarm the composed runtime's rc-trace recorder (`rc-trace-enable(on)`, runtime.wit:618) —
    /// the ATTRIBUTION complement to [`live_object_census`](Self::live_object_census). Requires the
    /// composed heap to be the rc-trace runtime build (`.#rctrace-runtime`, features debug-counters +
    /// rc-trace-export); a plain build lacks the export and this returns `None`. Enabling CLEARS the
    /// buffer + starts appending (mirrors cdz-run's `rc_trace_enable`). Diagnostic-only (env-gated test).
    async fn rc_trace_enable(&mut self, on: bool) -> Option<()> {
        let WasmReducer::Reducer { store, heap, .. } = self else {
            return None;
        };
        let heap = heap.as_ref()?;
        let iface = heap.get_export_index(&mut *store, None, "cadenza:runtime/debug-trace")?;
        let idx = heap.get_export_index(&mut *store, Some(&iface), "rc-trace-enable")?;
        let func = heap.get_func(&mut *store, idx)?;
        func.call_async(&mut *store, &[wasmtime::component::Val::Bool(on)], &mut [])
            .await
            .ok()?;
        func.post_return_async(&mut *store).await.ok()?;
        Some(())
    }

    /// Drain the composed runtime's rc-trace ring buffer (`rc-trace-drain() -> list<u8>`, runtime.wit:622)
    /// as the raw 20-byte-record byte array (decoded by the caller). Diagnostic-only (env-gated test).
    async fn rc_trace_drain(&mut self) -> Option<Vec<u8>> {
        let WasmReducer::Reducer { store, heap, .. } = self else {
            return None;
        };
        let heap = heap.as_ref()?;
        let iface = heap.get_export_index(&mut *store, None, "cadenza:runtime/debug-trace")?;
        let idx = heap.get_export_index(&mut *store, Some(&iface), "rc-trace-drain")?;
        let func = heap.get_func(&mut *store, idx)?;
        let mut out = [wasmtime::component::Val::Bool(false)];
        func.call_async(&mut *store, &[], &mut out).await.ok()?;
        func.post_return_async(&mut *store).await.ok()?;
        match out.into_iter().next()? {
            wasmtime::component::Val::List(items) => Some(
                items
                    .into_iter()
                    .filter_map(|v| match v {
                        wasmtime::component::Val::U8(b) => Some(b),
                        _ => None,
                    })
                    .collect(),
            ),
            _ => None,
        }
    }
}

/// Turn a guest reducer-export call result into the platform fold result, classifying a trap. A backend
/// [`HostBackendError`] trap (a `state`/`blobs` failure) becomes [`ReducerFault::HostBackend`](crate::ReducerFault::HostBackend)
/// (abort + retry the transaction); any other trap — a genuine guest panic/unreachable/abort — or a
/// malformed returned step becomes [`ReducerFault::Guest`](crate::ReducerFault::Guest) (the reducer
/// crashed). This is where the frozen option-returning WIT surface + the trapping host imports (§7) turn
/// into the driver's retry-vs-crash decision; nothing derived from a phantom read/write is ever returned.
fn fold_result(
    op: &'static str,
    call: wasmtime::Result<wit_reducer::Step>,
) -> Result<(Vec<Request>, Outcome), crate::ReducerFault> {
    use crate::ReducerFault;
    match call {
        Ok(step) => step_from_wit(step)
            .map_err(|e| ReducerFault::Guest(format!("{op} returned a malformed step: {e:?}"))),
        Err(e) if is_host_backend_trap(&e) => Err(ReducerFault::HostBackend(format!("{op}: {e}"))),
        Err(e) => Err(ReducerFault::Guest(format!("{op} trapped: {e}"))),
    }
}

// ── The wasm program store (§3/§8) ───────────────────────────────────────────────────────────────────────
// The production [`ProgramStore`]: resolve a program's wasm component from the content-addressed store and
// instantiate it as a [`WasmReducer`]. It carries no knowledge of any specific program — a program is data in
// the CAS, addressed by hash — so the same store runs whatever the input blobs define (a Cadenza reducer, a
// hand-written guest, anything targeting the reducer world).

/// A per-reducer backend the store hands each instance's [`HostState`]. The store does not create these — it
/// is given factories, so a caller (the integration harness) can inject recording-wrapped or shared backends
/// without the store knowing: the reducer's key-value state (§7) and its view of the content-addressed store
/// (§8, its `blobs` import) are produced per reducer id.
type BlobsFactory = Arc<dyn Fn(ReducerId) -> Box<dyn BlobStore> + Send + Sync>;
type KvFactory = Arc<dyn Fn(ReducerId) -> Box<dyn KvStore> + Send + Sync>;

/// The production [`ProgramStore`]: instantiate a reducer by loading its wasm component from the
/// content-addressed store and driving it with the wasm host (behind the `host` feature).
///
/// Addressing (§8): a program's component bytes are ordinary content in the one blob store, which keys on
/// content and ignores the hash's kind — so `spawn` fetches by the program hash directly and the store
/// resolves it to the same bytes a `Blob` hash over them would. Nothing here is program-specific: which
/// programs exist is the data seeded into the store.
///
/// What is shared vs per-reducer: the wasm engine + host-import linkers ([`ReducerHost`]) and the compiled
/// [`Component`] cache are shared across every reducer; the routing `graph` is the one node-wide substrate;
/// but each reducer gets its OWN state and store view, built by the injected factories so they can be
/// recording-wrapped or backed by a shared store as the caller decides.
/// A content-addressed component dependency a guest imports: the exact import name (which the linker matches
/// verbatim) and the [`Hash`] to fetch the dependency component from the content-addressed store by.
struct ComponentDep {
    import_name: String,
    hash: Hash,
}

/// The content address a dependency import name carries, as a store [`Hash`], or `None` if the name is not a
/// content-addressed dependency. The compiler emits a component dependency as an import whose name ends in
/// `+<addr>` — the dependency component's content hash in the canonical base62 text form ([`Hash`]'s
/// `Display`/`FromStr`, §8, the one tree-unified encoding): the full tagged hash, `Blob`-tagged for a
/// content-address (`cadenza:runtime/heap@0.0.0+<base62>`). Parse it back with [`Hash::from_str`]; the store
/// keys on the digest (ignoring the tag), so the parsed hash resolves the right content whatever its tag. A
/// platform host interface (`cadenza:platform/state` …) carries no `+…` and is served by [`add_host_imports`],
/// not from the store — so it yields `None` (no `+`, or a suffix that is not a valid base62 hash).
fn dependency_address(import_name: &str) -> Option<Hash> {
    import_name.rsplit_once('+')?.1.parse::<Hash>().ok()
}

/// The content-addressed component dependencies `component` imports — the imports the platform must resolve
/// from the store and compose in (as opposed to the platform host interfaces, wired by [`add_host_imports`]).
/// An import is a dependency when it is a component instance whose name carries a `+<hex>` content address.
fn component_dependencies(engine: &Engine, component: &Component) -> Vec<ComponentDep> {
    component
        .component_type()
        .imports(engine)
        .filter_map(|(name, item)| {
            if !matches!(item, ComponentItem::ComponentInstance(_)) {
                return None;
            }
            dependency_address(name).map(|hash| ComponentDep {
                import_name: name.to_string(),
                hash,
            })
        })
        .collect()
}

/// Whether `component` imports the TEST-ONLY `arg-probe` interface (§9) — i.e. it is an arg-probe-world guest,
/// which must instantiate against the arg-probe linker + world, not a reducer linker (which does not provide
/// `arg-probe`, so instantiation would fail). A real reducer never imports it.
fn imports_arg_probe(engine: &Engine, component: &Component) -> bool {
    component
        .component_type()
        .imports(engine)
        .any(|(name, _)| name.starts_with("cadenza:test-arg-probe/arg-probe"))
}

/// A sub-step of turning a program into a live reducer, for per-request init-cost attribution (the raw
/// material for a `wasm_instantiate_us{sub_step}` metric). The steps compose the `instantiate_program` cost:
/// `ComponentLoad` (fetch + compile-or-cache-hit the component from the content store), `BindDependencies`
/// (compose path only — instantiate each content-addressed dependency, e.g. the value-heap runtime, into the
/// store), and `WorldInstantiate` (instantiate the world onto a fresh store: linear-memory alloc + CoW image +
/// VMContext). A fast-path (dependency-free) instantiate fires `ComponentLoad` + `WorldInstantiate`; the
/// compose path adds `BindDependencies`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstantiateSubStep {
    /// Fetching + compiling (or cache-hitting) the program's component from the content-addressed store.
    ComponentLoad,
    /// Compose path only: instantiating each content-addressed dependency (the value-heap runtime, …) into the
    /// store — the per-request cost that cannot use the cached pre-instantiated fast path.
    BindDependencies,
    /// Instantiating the world onto a fresh store: linear-memory allocation + CoW image copy + VMContext setup.
    WorldInstantiate,
}

/// An embedder hook to record per-request reducer-instantiation sub-step timings. cdz-platform stays
/// DEP-FREE: it only times its own [`InstantiateSubStep`]s and calls this observer; the embedder (the Membrain
/// daemon) records the durations into its metrics reporter as `wasm_instantiate_us{sub_step}` (the
/// Prometheus-scrapable init-cost attribution the perf lane reads). A `None` observer — the default — pays
/// ZERO: no `Instant` is taken and no call is made, so an un-observed host has no measurement overhead.
/// Install one at node assembly with [`WasmProgramStore::with_instantiate_observer`].
pub trait InstantiateObserver: Send + Sync {
    /// Record that `sub_step` of a reducer instantiation took `elapsed`. Called once per sub-step per
    /// instantiation, on the instantiate path (router-spawn AND fold callers alike). Must be cheap + non-blocking
    /// (it runs inline on the instantiate path); record into a metric/counter, do not do I/O.
    fn record(&self, sub_step: InstantiateSubStep, elapsed: std::time::Duration);
}

/// The per-host instantiation core: the reducer-independent machinery for turning a program's content-
/// addressed bytes into a live reducer, and the pure-run capability the synchronous `run` host import (§3)
/// is served from. It holds only host-wide state — the wasm engine and per-kind linkers ([`ReducerHost`]),
/// the content-addressed store components load from, the compiled-component cache, and the pure-run memo —
/// and never references a per-reducer [`HostState`]. That acyclic shape is the point: a running reducer's
/// `run` host import instantiates the program it is handed as a *pure* sub-reducer, and that sub-reducer is
/// itself given the same `Arc<Instantiator>` so it too may call `run` (a pure computation composing pure
/// sub-computation, the design intent). The recursion lives in the call stack, not the object graph: the
/// core points only at leaf resources, never back at a `HostState` or the store, so there is no cycle and no
/// `Weak`. [`WasmProgramStore`] holds one and shares it.
struct Instantiator {
    host: ReducerHost,
    /// The content-addressed store components are loaded from (read-only here — `get` by hash).
    cas: Arc<dyn BlobStore>,
    /// Compiled components keyed by content digest — `Component::new` (the Cranelift compile) runs once per
    /// distinct component, not once per reducer, and a program and a dependency over the same bytes share the
    /// one entry (the digest, like the store, ignores the hash kind). A `Component` is cheaply clonable
    /// (internally reference-counted).
    compiled: Mutex<HashMap<[u8; Hash::DIGEST_LEN], Component>>,
    /// The pure-run memo (§3): a bounded LRU of `(program, input) -> output` shared by every synchronous
    /// `run` on this host, so a repeated pure run — including one a fold makes and one nested inside it —
    /// skips execution. Sound because a pure run is deterministic (empty capabilities, null birth).
    memo: Mutex<crate::run::Cache>,
    /// Optional embedder hook for per-request instantiate-cost attribution (`wasm_instantiate_us{sub_step}`).
    /// `None` (the default) pays zero — no `Instant`, no call. Set at assembly via
    /// [`WasmProgramStore::with_instantiate_observer`]. See [`InstantiateObserver`].
    observer: Option<Arc<dyn InstantiateObserver>>,
    /// TEST/DEBUG-ONLY component-byte overrides keyed by content digest: when [`component`](Self::component)
    /// resolves a hash present here, it uses these bytes INSTEAD of the content-addressed store — mirroring
    /// `cdz-run --runtime`, so a harness can compose an ALTERNATE value-heap runtime (e.g. the debug-counters
    /// build, whose content hash differs from the one a guest records) for a guest that imports the shipped
    /// runtime's hash, without recompiling the guest. Empty in production (set only at assembly via
    /// [`WasmProgramStore::with_component_override`]); a non-empty map deliberately breaks content-addressing,
    /// which is why it is a test/debug affordance, never a production path.
    component_overrides: HashMap<[u8; Hash::DIGEST_LEN], Bytes>,
}

impl Instantiator {
    /// Build the instantiation core: the shared engine and per-kind linkers ([`ReducerHost::new`], carrying the
    /// node's resource `limits`) over the content store `cas`, with an empty pure-run memo. Fails only if the
    /// wasm engine/linkers cannot be built.
    fn new(cas: Arc<dyn BlobStore>, limits: ResourceLimits) -> Result<Self, wasmtime::Error> {
        Ok(Self {
            host: ReducerHost::new(limits)?,
            cas,
            compiled: Mutex::new(HashMap::new()),
            memo: Mutex::new(crate::run::Cache::new(crate::run::Cache::DEFAULT_CAPACITY)),
            observer: None,
            component_overrides: HashMap::new(),
        })
    }

    /// Start timing an instantiate sub-step: `Some(Instant)` only if an observer is installed, so an
    /// un-observed host takes no clock reading (zero overhead). Pair with [`observe_end`](Self::observe_end).
    #[inline]
    fn observe_start(&self) -> Option<std::time::Instant> {
        self.observer.as_ref().map(|_| std::time::Instant::now())
    }

    /// Record the elapsed time of `step` to the observer, if one is installed and `start` was taken. A no-op
    /// when un-observed.
    #[inline]
    fn observe_end(&self, start: Option<std::time::Instant>, step: InstantiateSubStep) {
        if let (Some(obs), Some(t)) = (self.observer.as_ref(), start) {
            obs.record(step, t.elapsed());
        }
    }

    /// The compiled component whose bytes `hash` addresses, loaded from the store and cached by content
    /// digest. `None` if the store does not hold it (an unknown program/dependency) or the bytes are not a
    /// valid component. Used for both a program (by its [`ProgramHash`]) and a dependency (by its `Blob`
    /// hash) — the store and this cache key on the digest, so either resolves the same bytes (§8).
    async fn component(&self, hash: Hash) -> Option<Component> {
        let key = *hash.digest();
        if let Some(component) = self.compiled.lock().expect("compiled cache lock").get(&key) {
            return Some(component.clone());
        }
        // Compile outside the lock (Cranelift is slow); a concurrent duplicate compile of the same component
        // is harmless — the last insert wins and both yield an equivalent component.
        // This loader's contract is `Option` (None = can't load); a store error OR a genuine miss both read
        // as None here (`.ok().flatten()` collapses `Err`/`Ok(None)`).
        // A test/debug override (see `component_overrides`) substitutes bytes for this digest, bypassing the
        // content-addressed lookup entirely — so a harness can compose an alternate runtime the guest did not
        // record. Empty in production, so the common path is exactly the CAS `get` below.
        let bytes = match self.component_overrides.get(&key) {
            Some(overridden) => overridden.clone(),
            None => self.cas.get(hash).await.ok().flatten()?,
        };
        let component = Component::new(&self.host.engine, &bytes).ok()?;
        self.compiled
            .lock()
            .expect("compiled cache lock")
            .insert(key, component.clone());
        Some(component)
    }

    /// Resolve and compose `component`'s content-addressed dependencies into `linker`, instantiating each
    /// into `store`: fetch the dependency component from the store, recursively compose ITS dependencies,
    /// instantiate it, and alias its exported functions into `linker` under the exact import name the parent
    /// declared. This is what makes a Cadenza guest's `cadenza:runtime/heap@…+<hash>` import (and any other
    /// content-addressed component dependency) resolvable — the runtime and its peers come from the store,
    /// not a native host. `Box::pin` because it recurses across an `await` (a dependency of a dependency).
    ///
    /// `heap_out` captures the composed value-heap runtime instance (the `cadenza:runtime/heap` dependency) so
    /// a debug leak-census can later read its `live-objects` export — see
    /// [`WasmReducer::live_object_census`]. It captures only THIS level's heap dependency (the recursive call
    /// for a dependency's OWN sub-dependencies passes `&mut None`), and only the heap-runtime import; every
    /// other dependency is composed and dropped as before. `&mut None` (the production path) captures nothing.
    fn bind_dependencies<'a>(
        &'a self,
        store: &'a mut Store<HostState>,
        linker: &'a mut Linker<HostState>,
        component: &'a Component,
        heap_out: &'a mut Option<wasmtime::component::Instance>,
    ) -> Pin<Box<dyn Future<Output = Result<(), wasmtime::Error>> + Send + 'a>> {
        Box::pin(async move {
            for dep in component_dependencies(&self.host.engine, component) {
                let dep_component = self.component(dep.hash).await.ok_or_else(|| {
                    wasmtime::Error::msg(format!(
                        "reducer dependency {} is not in the content-addressed store",
                        dep.import_name
                    ))
                })?;
                // The dependency is instantiated against a linker holding only ITS OWN dependencies — a pure
                // content-addressed component (the value-heap runtime, NFC, …) takes no platform host
                // imports, only sub-dependencies from the store. The recursion captures no heap (its own
                // sub-deps are not this component's runtime).
                let mut dep_linker = Linker::new(&self.host.engine);
                let mut sub_heap = None;
                self.bind_dependencies(store, &mut dep_linker, &dep_component, &mut sub_heap)
                    .await?;
                let dep_instance = dep_linker
                    .instantiate_async(&mut *store, &dep_component)
                    .await?;
                alias_instance_exports(
                    store,
                    linker,
                    &dep.import_name,
                    &dep_component,
                    &dep_instance,
                )?;
                // Retain the value-heap runtime instance for the debug census (after aliasing, so the alias
                // above keeps its borrow). Only the heap-runtime import — other deps are not censused.
                if dep.import_name.contains("cadenza:runtime/heap") {
                    *heap_out = Some(dep_instance);
                }
            }
            Ok(())
        })
    }

    /// Instantiate `program` as a live reducer of `kind`, backing its host imports with `host_state`. The
    /// fast path reuses the cached per-kind pre-instantiated world; a component with content-addressed
    /// dependencies takes the compose path (a fresh per-spawn linker with those dependencies bound in).
    /// `None` if the program is not in the store or its component fails to instantiate.
    async fn instantiate_program(
        &self,
        program: ProgramHash,
        kind: ReducerKind,
        host_state: HostState,
    ) -> Option<Box<dyn Reducer>> {
        let load_t = self.observe_start();
        let component = self.component(program.hash()).await?;
        self.observe_end(load_t, InstantiateSubStep::ComponentLoad);
        let has_deps = !component_dependencies(&self.host.engine, &component).is_empty();
        let reducer = if imports_arg_probe(&self.host.engine, &component) {
            // TEST-ONLY arg-probe-world guest (§9): it imports `arg-probe` (not the platform capabilities) and
            // exports the same reducer `guest`, so it instantiates against the arg-probe linker + world.
            let mut store = Store::new(&self.host.engine, host_state);
            arm_store_safety(&mut store);
            let world = if has_deps {
                // Compose path: a fresh linker with `arg-probe` + the store-resolved deps (the value-heap
                // runtime a Cadenza guest imports).
                let mut linker = Linker::new(&self.host.engine);
                arg_probe_world::cadenza::test_arg_probe::arg_probe::add_to_linker::<_, HostData>(
                    &mut linker,
                    |s| s,
                )
                .ok()?;
                // The arg-probe guest is a test harness; it is not censused, so capture no heap instance.
                self.bind_dependencies(&mut store, &mut linker, &component, &mut None)
                    .await
                    .ok()?;
                arg_probe_world::ArgProbeWorld::instantiate_async(&mut store, &component, &linker)
                    .await
                    .ok()?
            } else {
                // Fast path: reuse the shared, pre-wired arg-probe linker.
                let pre = arg_probe_world::ArgProbeWorldPre::new(
                    self.host
                        .arg_probe_linker
                        .instantiate_pre(&component)
                        .ok()?,
                )
                .ok()?;
                pre.instantiate_async(&mut store).await.ok()?
            };
            WasmReducer::ArgProbe { store, world }
        } else if !has_deps {
            // Fast path: no content-addressed component dependencies, so reuse the cached, pre-instantiated
            // per-kind linker (the engine, linker, and pre are all shared).
            let pre = self.host.preinstantiate(&component, kind).ok()?;
            let world_t = self.observe_start();
            let reducer = self.host.instantiate(&pre, host_state).await.ok()?;
            self.observe_end(world_t, InstantiateSubStep::WorldInstantiate);
            reducer
        } else {
            // Compose path: the component imports dependencies (the value-heap runtime, …) that must be
            // resolved from the store and instantiated into THIS store, so a fresh per-spawn linker is built
            // (a store-bound dependency instance cannot be pre-instantiated). Host imports for the kind are
            // wired first (the capability split), then the dependencies composed in.
            let mut store = Store::new(&self.host.engine, host_state);
            arm_store_safety(&mut store);
            let mut linker = Linker::new(&self.host.engine);
            add_host_imports(&mut linker, kind).ok()?;
            let bind_t = self.observe_start();
            // Capture the composed value-heap runtime instance for the debug leak-census.
            let mut heap = None;
            self.bind_dependencies(&mut store, &mut linker, &component, &mut heap)
                .await
                .ok()?;
            self.observe_end(bind_t, InstantiateSubStep::BindDependencies);
            let world_t = self.observe_start();
            let world = EventReducerWorld::instantiate_async(&mut store, &component, &linker)
                .await
                .ok()?;
            self.observe_end(world_t, InstantiateSubStep::WorldInstantiate);
            WasmReducer::Reducer { store, world, heap }
        };
        Some(Box::new(reducer))
    }

    /// Whether the store holds `program`'s component bytes — a content lookup by the program hash (§8).
    async fn contains(&self, program: ProgramHash) -> bool {
        // A store error reads as "not present" — this bool contract can't surface it.
        self.cas.has(program.hash()).await.unwrap_or(false)
    }

    /// Run `program` once as a pure function of `input` against `contract` — the capability behind the
    /// synchronous `run` host import (§3). Instantiates the program as a pure reducer (empty capabilities,
    /// null birth) whose own `run` import is served by *this same* core, so it too may run pure
    /// sub-programs; folds the input; returns the output, memoized. `&Arc<Self>` because the pure sub-reducer
    /// is handed a clone of this core to recurse through — the recursion is on the stack, the object graph
    /// stays acyclic (the core never points back at a [`HostState`]).
    async fn run_pure(
        self: &Arc<Self>,
        program: ProgramHash,
        contract: ContractId,
        input: Bytes,
    ) -> Result<Bytes, RunError> {
        let key = (program, Hash::of(HashTag::Blob, &input));
        // Memo hit — drop the lock before any await (never hold a std Mutex across `.await`).
        if let Some(output) = self.memo.lock().expect("run memo lock").get(&key) {
            return Ok(output);
        }
        let host_state = null_host_state(
            crate::run::null_run_id(),
            Some(Arc::clone(self)),
            &self.host.limits,
        );
        let reducer = self
            .instantiate_program(program, ReducerKind::Pure, host_state)
            .await
            .ok_or(RunError::UnknownProgram)?;
        let output = crate::run::drive_pure(reducer, contract, input).await?;
        self.memo
            .lock()
            .expect("run memo lock")
            .put(key, output.clone());
        Ok(output)
    }
}

/// The [`HostState`] a pure sub-program is instantiated with. A pure reducer's linker wires only `run` (§3
/// empty-capability set otherwise), so the state/blobs/graph/provenance/delivery backends are never touched —
/// they exist only because a `Store` must carry a `HostState` — but `run` is threaded through: `run` is
/// `Some(inst)` so a pure program may itself call `run` to invoke other pure programs, the whole computation
/// staying deterministic.
fn null_host_state(
    id: ReducerId,
    run: Option<Arc<Instantiator>>,
    limits: &ResourceLimits,
) -> HostState {
    HostState {
        id,
        blobs: Box::new(crate::InMemoryBlobStore::new()),
        kv: Box::new(crate::InMemoryKvStore::new()),
        graph: Arc::new(crate::InMemoryReducerGraph::new()),
        provenance: Arc::new(crate::NoProvenance),
        delivery: Arc::new(crate::NoDelivery),
        run,
        rejected: None,
        run_sink: None,
        arg_probe: None,
        limits: reducer_store_limits(limits),
        resource_limits: *limits,
    }
}

/// Per-reducer injection points for the node-wide `graph` / `program-of` / `deliver` host capabilities,
/// uniform with the [`BlobsFactory`]/[`KvFactory`] that already build a reducer's `blobs`/`state` backends:
/// given the calling reducer's id, each produces the capability that reducer's host-import calls hit. This is
/// a general seam — the store does not know or care what a factory returns, so a caller can supply the shared
/// capability directly, a per-reducer variant, a fault-injecting stand-in, or a decorator, without the store
/// changing. Recording is one such use (and the one this seam was first needed for): the integration harness
/// supplies a factory that builds a decorator over a shared base, emitting each direct host call into the
/// observation log (`design/cadenza-platform.md` §9) attributed to the calling reducer — the same way
/// [`BlobsFactory`]/[`KvFactory`] build `RecordingBlobStore`/`RecordingKvStore`. The default factory hands out
/// the shared node-wide capability directly (an `Arc` clone), so a caller that injects nothing pays nothing.
///
/// These return `Arc<dyn _>` rather than the `Box<dyn _>` of [`BlobsFactory`]/[`KvFactory`] because they are
/// node-wide *shared* — every reducer sees the one reducer graph and the one node-side provenance/delivery,
/// so the factory hands out clones of a shared handle — whereas `blobs`/`state` are per-reducer *independent*
/// backends the factory news up fresh. Same factory shape; the return type reflects shared-vs-independent.
///
/// `identity` (`identity.id`, a pure who-am-I read) is deliberately NOT injectable (operator: identity queries
/// are not logged). `run` is not here yet: the run capability is the concrete [`Instantiator`], not a trait
/// object — trait-ifying it so a factory can wrap it is a follow-up.
type GraphFactory = Arc<dyn Fn(ReducerId) -> Arc<dyn ReducerGraph> + Send + Sync>;
type ProvenanceFactory = Arc<dyn Fn(ReducerId) -> Arc<dyn Provenance> + Send + Sync>;
type DeliveryFactory = Arc<dyn Fn(ReducerId) -> Arc<dyn Delivery> + Send + Sync>;
type RejectedSinkFactory = Arc<dyn Fn(ReducerId) -> Arc<dyn RejectedSink> + Send + Sync>;
type RunSinkFactory = Arc<dyn Fn(ReducerId) -> Arc<dyn RunSink> + Send + Sync>;
type ArgProbeSinkFactory = Arc<dyn Fn(ReducerId) -> Arc<dyn ArgProbeSink> + Send + Sync>;

pub struct WasmProgramStore {
    /// The shared per-host instantiation core (engine, linkers, content store, compiled cache, pure-run memo),
    /// held via `Arc` so a reducer's synchronous `run` host import can share it — both to instantiate the
    /// program it runs and as the run capability itself — without a cycle back to this store (the acyclic
    /// wiring the run import rests on). Cloned into each reducer's [`HostState`] as its `run` capability.
    inst: Arc<Instantiator>,
    /// Builds each reducer's `blobs` host-import backend (its view of the content store, §8).
    make_blobs: BlobsFactory,
    /// Builds each reducer's key-value state backend (§7).
    make_kv: KvFactory,
    /// Builds each reducer's view of the one node-wide reducer graph (§3) — the default hands out the shared
    /// graph directly; an injected factory may hand out a decorator over it (e.g. a recording one). Shared,
    /// not per-reducer independent.
    make_graph: GraphFactory,
    /// Builds each reducer's view of the node-side provenance its `program-of` import reads (§4) — the system,
    /// which knows every running reducer's program. Defaults to a factory over [`NoProvenance`](crate::NoProvenance)
    /// until set with [`with_provenance`](WasmProgramStore::with_provenance), so it is never absent, only null.
    make_provenance: ProvenanceFactory,
    /// Builds each reducer's view of the node-side delivery its `deliver` import routes through (§4) — the
    /// system, which injects an event into a reducer's mailbox. Defaults to a factory over
    /// [`NoDelivery`](crate::NoDelivery) until set with [`with_delivery`](WasmProgramStore::with_delivery), so
    /// it is never absent, only null. Injecting a decorating factory here can, for one, make the deliver ACT
    /// observable (§9).
    make_delivery: DeliveryFactory,
    /// Builds each reducer's sink for host calls the boundary rejected before the recordable capability (a
    /// malformed-arg `graph` op, §9). `None` until set with [`with_rejected`](WasmProgramStore::with_rejected),
    /// so a rejected call is dropped (each reducer's `rejected` is `None`) unless an observing node injects a
    /// recording sink — the disabled path then pays zero (no factory call, no `Arc`).
    make_rejected: Option<RejectedSinkFactory>,
    /// Builds each reducer's sink for `run` host calls (§9). `None` until set with
    /// [`with_run_sink`](WasmProgramStore::with_run_sink), so a run is not recorded (each reducer's `run_sink`
    /// is `None`) unless an observing node injects a recording sink.
    make_run_sink: Option<RunSinkFactory>,
    /// Builds each reducer's sink for the TEST-ONLY `arg-probe.probe` host call (§9 arg-value capture). `None`
    /// until set with [`with_arg_probe`](WasmProgramStore::with_arg_probe), so it is wired only for the
    /// arg-capture conformance world (each reducer's `arg_probe` is `None` otherwise). Mirrors `make_run_sink`.
    make_arg_probe: Option<ArgProbeSinkFactory>,
    /// The node-side delivery slot the running [`TaskSystem`](crate::TaskSystem) fills once it exists (§4) —
    /// empty until then (the store is built first). A `make_delivery` factory that wants the `deliver` host
    /// import to reach the live node wraps [`node_delivery_slot`](WasmProgramStore::node_delivery_slot) as its
    /// backend; [`set_node_delivery`](ProgramStore::set_node_delivery) fills THIS slot, so a forward lands in
    /// the target's mailbox rather than being dropped. Empty (a no-op) unless a system fills it.
    node_delivery: Arc<crate::NodeDeliverySlot>,
}

impl WasmProgramStore {
    /// A store loading components from `cas`, giving each reducer the state, content-store view, and reducer-
    /// graph view its factories build. Fails only if the wasm engine/linkers cannot be built
    /// ([`ReducerHost::new`]). Provenance and delivery default to factories over
    /// [`NoProvenance`](crate::NoProvenance)/[`NoDelivery`](crate::NoDelivery) until set with
    /// [`with_provenance`](WasmProgramStore::with_provenance)/[`with_delivery`](WasmProgramStore::with_delivery).
    /// `make_graph` mirrors `make_blobs`/`make_kv`: a plain caller passes `move |_id| graph.clone()` over its
    /// one shared graph; an injecting caller passes a factory that decorates it (recording is one such use).
    /// That one graph must be the SAME instance the routing system reads and mutates (whatever seeds edges and
    /// answers `neighbors` when a reducer forwards) — not a fresh `InMemoryReducerGraph::new()` per reducer. A
    /// reducer reads its own `graph` host-import to route (`neighbors(sender, contract, dir)`); if the store
    /// hands out a different instance than the one the system's spawns/edges populate, the reducer reads an
    /// empty graph and §4 forward routing silently finds nothing (no error — it just rejects instead of
    /// forwarding). Wire the node's routing graph through this factory (decorated or bare); the itest binary is
    /// the reference wiring.
    ///
    /// Uses the default [`ResourceLimits`]; a node that tunes its per-reducer compute/memory limits builds with
    /// [`with_resource_limits`](WasmProgramStore::with_resource_limits) instead.
    pub fn new(
        cas: Arc<dyn BlobStore>,
        make_blobs: BlobsFactory,
        make_kv: KvFactory,
        make_graph: GraphFactory,
    ) -> Result<Self, wasmtime::Error> {
        Self::with_resource_limits(
            cas,
            make_blobs,
            make_kv,
            make_graph,
            ResourceLimits::default(),
        )
    }

    /// A store as [`new`](WasmProgramStore::new), with the node's per-reducer resource `limits` (the compute
    /// budget, memory ceiling, and epoch tick) it arms every reducer store with. This is the config seam for
    /// the limits: a node sets its own values here from its own config, so they are never hard-coded in
    /// platform source. [`new`](WasmProgramStore::new) is the same with [`ResourceLimits::default`].
    pub fn with_resource_limits(
        cas: Arc<dyn BlobStore>,
        make_blobs: BlobsFactory,
        make_kv: KvFactory,
        make_graph: GraphFactory,
        limits: ResourceLimits,
    ) -> Result<Self, wasmtime::Error> {
        Ok(Self {
            inst: Arc::new(Instantiator::new(cas, limits)?),
            make_blobs,
            make_kv,
            make_graph,
            make_provenance: Arc::new(|_id| Arc::new(crate::NoProvenance) as Arc<dyn Provenance>),
            make_delivery: Arc::new(|_id| Arc::new(crate::NoDelivery) as Arc<dyn Delivery>),
            make_rejected: None,
            make_run_sink: None,
            make_arg_probe: None,
            node_delivery: Arc::new(crate::NodeDeliverySlot::new()),
        })
    }

    /// The node-side delivery slot a `make_delivery` factory wraps so the `deliver` host import reaches the
    /// live system (§4). Build the store, then pass this slot as the backend of `with_delivery`'s factory (a
    /// recording decorator over it, say); [`TaskSystem::new`](crate::TaskSystem) fills it once it runs the
    /// store, so a forwarded deliver lands in the target's mailbox instead of being dropped. Empty until then.
    #[must_use]
    pub fn node_delivery_slot(&self) -> Arc<crate::NodeDeliverySlot> {
        Arc::clone(&self.node_delivery)
    }

    /// Wire each reducer's view of the node-side provenance its `program-of` import reads (§4) — the system,
    /// which knows every running reducer's program. Set during node assembly, after the system exists (it
    /// holds this store as its program store, so the reference is broken with a `Weak` or set once at wiring
    /// time). Pass `move |_id| prov.clone()` for the plain shared provenance, or a factory that builds a
    /// per-reducer decorator — e.g. a recording one to make `program-of` calls observable (§9). Uniform with
    /// `make_blobs`.
    #[must_use]
    pub fn with_provenance(mut self, make_provenance: ProvenanceFactory) -> Self {
        self.make_provenance = make_provenance;
        self
    }

    /// Install an [`InstantiateObserver`] to record per-request instantiate sub-step timings
    /// (`wasm_instantiate_us{sub_step}`) — the embedder (the Membrain daemon) records them into its metrics
    /// reporter. Set once at node ASSEMBLY, before any reducer is spawned: the observer lives on the shared
    /// instantiation core, which at assembly time is still uniquely held (refcount 1), so this mutates it in
    /// place; called after a reducer has cloned the core it is a no-op (guarded in debug). Un-observed hosts
    /// (the default, no observer) pay zero — no clock reading, no call.
    #[must_use]
    pub fn with_instantiate_observer(mut self, observer: Arc<dyn InstantiateObserver>) -> Self {
        match Arc::get_mut(&mut self.inst) {
            Some(inst) => inst.observer = Some(observer),
            None => debug_assert!(
                false,
                "with_instantiate_observer must be called at assembly, before the instantiation core is shared"
            ),
        }
        self
    }

    /// TEST/DEBUG-ONLY: substitute `bytes` for the component named by `hash`, so instantiation composes these
    /// bytes wherever that hash is a (transitive) dependency — INSTEAD of resolving `hash` from the
    /// content-addressed store. This deliberately breaks content-addressing (the whole point of the CAS), so
    /// it is only for a harness that needs to compose a runtime a guest did NOT record — e.g. swapping in the
    /// debug-counters value-heap runtime (whose content hash differs from the shipped runtime's) under a
    /// guest's `cadenza:runtime/heap@…+<shipped-hash>` import to census live-objects, without recompiling the
    /// guest against the debug hash. Mirrors `cdz-run --runtime`. Must be called at assembly, before the
    /// instantiation core is shared (like [`with_instantiate_observer`](Self::with_instantiate_observer)); a
    /// no-op with a `debug_assert` otherwise. NEVER used on a production path (`component_overrides` is empty).
    pub fn with_component_override(mut self, hash: Hash, bytes: Bytes) -> Self {
        match Arc::get_mut(&mut self.inst) {
            Some(inst) => {
                inst.component_overrides.insert(*hash.digest(), bytes);
            }
            None => debug_assert!(
                false,
                "with_component_override must be called at assembly, before the instantiation core is shared"
            ),
        }
        self
    }

    /// Wire each reducer's view of the node-side delivery its `deliver` import routes through (§4) — the
    /// system, which injects an event into a reducer's mailbox. Set during node assembly, after the system
    /// exists (as with [`with_provenance`](WasmProgramStore::with_provenance), the store↔system reference is
    /// broken with a `Weak` or set once at wiring time). Pass `move |_id| delivery.clone()` for the plain
    /// shared delivery, or a factory that builds a per-reducer decorator — e.g. a recording one to make the
    /// deliver ACT observable (§9). Uniform with `make_blobs`.
    #[must_use]
    pub fn with_delivery(mut self, make_delivery: DeliveryFactory) -> Self {
        self.make_delivery = make_delivery;
        self
    }

    /// Wire each reducer's sink for host calls the boundary rejected before the recordable capability (a
    /// malformed-arg `graph` op, §9). Pass `move |_id| sink.clone()` for a shared sink, or a factory that
    /// builds a per-reducer recording decorator so a rejected call becomes an observation attributed to the
    /// calling reducer. Unset (`None`) by default, so a rejected call is dropped and the parse-guard path pays
    /// zero; wiring a factory makes each reducer's `rejected` `Some`. Uniform with `make_blobs`.
    #[must_use]
    pub fn with_rejected(mut self, make_rejected: RejectedSinkFactory) -> Self {
        self.make_rejected = Some(make_rejected);
        self
    }

    /// Wire each reducer's sink for `run` host calls (§9). Pass `move |_id| sink.clone()` for a shared sink, or
    /// a factory that builds a per-reducer recording decorator so a run becomes an observation attributed to
    /// the calling reducer. Unset (`None`) by default, so a run is not recorded; wiring a factory makes each
    /// reducer's `run_sink` `Some`. Uniform with `with_rejected`.
    #[must_use]
    pub fn with_run_sink(mut self, make_run_sink: RunSinkFactory) -> Self {
        self.make_run_sink = Some(make_run_sink);
        self
    }

    /// Wire each reducer's sink for the TEST-ONLY `arg-probe.probe` host call (§9 arg-value capture). Pass
    /// `move |_id| sink.clone()` for a shared recording sink so the received (canonical-encoded) `probe-record`
    /// and `list<narrow>` become an observation a checker asserts byte-for-byte. Unset (`None`) by default, so
    /// it is wired only for the arg-capture conformance world; wiring a factory makes each reducer's
    /// `arg_probe` `Some`. Uniform with [`with_run_sink`](WasmProgramStore::with_run_sink).
    #[must_use]
    pub fn with_arg_probe(mut self, make_arg_probe: ArgProbeSinkFactory) -> Self {
        self.make_arg_probe = Some(make_arg_probe);
        self
    }
}

/// Alias every function a dependency instance exports into `linker` under `import_name` — the parent's
/// import is that instance, and the linker matches the name verbatim (so the `+<hash>` suffix is kept). The
/// dependency exports a single interface (the runtime's heap ops, NFC's transform, …); each of its functions
/// is forwarded to the live dependency instance via `func_new_async` (the reducer engine is async, so the
/// forwarded call runs on the async path), mirroring the value-heap composition `cdz-run` performs. The
/// function names come from the dependency's own type, so the wiring always matches the composed component.
fn alias_instance_exports(
    store: &mut Store<HostState>,
    linker: &mut Linker<HostState>,
    import_name: &str,
    dep_component: &Component,
    dep_instance: &wasmtime::component::Instance,
) -> Result<(), wasmtime::Error> {
    let engine = linker.engine().clone();
    let mut iface = linker.instance(import_name)?;
    for (export_name, item) in dep_component.component_type().exports(&engine) {
        let ComponentItem::ComponentInstance(inst) = item else {
            continue; // only interface (instance) exports carry the imported functions
        };
        let iface_idx = dep_instance
            .get_export_index(&mut *store, None, export_name)
            .ok_or_else(|| {
                wasmtime::Error::msg(format!("dependency missing export `{export_name}`"))
            })?;
        for (func_name, func_item) in inst.exports(&engine) {
            if !matches!(func_item, ComponentItem::ComponentFunc(_)) {
                continue;
            }
            let func_idx = dep_instance
                .get_export_index(&mut *store, Some(&iface_idx), func_name)
                .ok_or_else(|| wasmtime::Error::msg(format!("dependency missing `{func_name}`")))?;
            let func = dep_instance
                .get_func(&mut *store, func_idx)
                .ok_or_else(|| {
                    wasmtime::Error::msg(format!("dependency export `{func_name}` is not a func"))
                })?;
            // Forward asynchronously: the reducer engine has async support enabled (host imports may await a
            // disk/network backend), and wasmtime requires `call_async`/`post_return_async` — not the sync
            // `call` — for any func call under an async config. A composed dependency func (the value-heap
            // runtime's ops, which call into nfc) is invoked from inside a guest fold, on the async path, so
            // the sync `call` panics ("must use `call_async` when async support is enabled").
            iface.func_new_async(func_name, move |mut ctx, params, results| {
                Box::new(async move {
                    func.call_async(&mut ctx, params, results).await?;
                    func.post_return_async(&mut ctx).await?;
                    Ok(())
                })
            })?;
        }
    }
    Ok(())
}

#[async_trait]
impl ProgramStore for WasmProgramStore {
    async fn spawn(&self, program: ProgramHash, ctx: SpawnContext) -> Option<Box<dyn Reducer>> {
        // The store's job is to assemble the per-reducer HostState from its factories; turning the program's
        // bytes into a live reducer is the shared instantiation core's (which the reducer's own host-imports
        // also reach, without a cycle back here). Each factory builds this reducer's view of its capability
        // (default: the shared one; an injected factory: a per-reducer variant, e.g. a decorator that logs the
        // call attributed to its id, §9).
        // Resolve this reducer's effective limits: the node's, with any per-spawn budget clamped to the node
        // ceiling (a spawn can lower its own budget, never raise it above the node's). `None` inherits the
        // node's. The store is armed (compute + memory) from these, so the per-spawn budget actually reaches
        // the store rather than the node-uniform value.
        let effective = self.inst.host.limits.resolve_for_spawn(ctx.limits);
        let host_state = HostState {
            id: ctx.id,
            blobs: (self.make_blobs)(ctx.id),
            kv: (self.make_kv)(ctx.id),
            graph: (self.make_graph)(ctx.id),
            provenance: (self.make_provenance)(ctx.id),
            delivery: (self.make_delivery)(ctx.id),
            run: Some(Arc::clone(&self.inst)),
            rejected: self.make_rejected.as_ref().map(|f| f(ctx.id)),
            run_sink: self.make_run_sink.as_ref().map(|f| f(ctx.id)),
            arg_probe: self.make_arg_probe.as_ref().map(|f| f(ctx.id)),
            limits: reducer_store_limits(&effective),
            resource_limits: effective,
        };
        self.inst
            .instantiate_program(program, ctx.kind, host_state)
            .await
    }

    async fn contains(&self, program: ProgramHash) -> bool {
        self.inst.contains(program).await
    }

    fn epoch_incrementer(&self) -> Option<(Duration, Arc<dyn Fn() + Send + Sync>)> {
        // The kernel's epoch ticker drives this at the configured `epoch_tick` to preempt long-running guest
        // folds (see `arm_store_safety`). The engine is cheaply clonable (ref-counted) and shared by every
        // reducer on this host. Both the cadence and the incrementer come from the node's `ResourceLimits`.
        let engine = self.inst.host.engine.clone();
        let tick = self.inst.host.limits.epoch_tick;
        Some((tick, Arc::new(move || engine.increment_epoch())))
    }

    fn set_node_delivery(&self, delivery: Arc<dyn Delivery>) {
        // Fill the slot a `make_delivery` factory wrapped via `node_delivery_slot`, so from now the `deliver`
        // host import reaches the live system. Called once by `TaskSystem::new` after the system is built.
        self.node_delivery.set(delivery);
    }
}

#[cfg(test)]
mod tests {
    use super::HostState;
    // The `blobs` and `state` imports both have `get`/`put`, so use named trait aliases and fully-qualified
    // calls to disambiguate.
    use super::cadenza::platform::blobs::Host as Blobs;
    use super::cadenza::platform::graph::Dir;
    use super::cadenza::platform::graph::Host as Graph;
    use super::cadenza::platform::identity::Host as Identity;
    use super::cadenza::platform::state::Host as State;
    use crate::{
        Hash, HashTag, InMemoryBlobStore, InMemoryKvStore, InMemoryReducerGraph, ReducerId,
    };
    use std::sync::Arc;

    fn host(id: ReducerId) -> HostState {
        HostState {
            id,
            blobs: Box::new(InMemoryBlobStore::new()),
            kv: Box::new(InMemoryKvStore::new()),
            graph: Arc::new(InMemoryReducerGraph::new()),
            provenance: Arc::new(crate::NoProvenance),
            delivery: Arc::new(crate::NoDelivery),
            run: None,
            rejected: None,
            run_sink: None,
            arg_probe: None,
            limits: super::reducer_store_limits(&super::ResourceLimits::default()),
            resource_limits: super::ResourceLimits::default(),
        }
    }

    /// The raw hash bytes of a reducer-id / edge-kind, as they cross the WIT boundary.
    fn rid_bytes(tag: &[u8]) -> Vec<u8> {
        ReducerId::of(tag).hash().as_bytes().to_vec()
    }
    fn kind_bytes(tag: &[u8]) -> Vec<u8> {
        Hash::of(HashTag::SystemProperty, tag).as_bytes().to_vec()
    }

    /// The arg-probe Host impl encodes the received args to the canonical Value form the itest checker
    /// asserts against — locking the byte-for-byte contract (§9): the CADENZA constructor casing
    /// (`Absent`/`Small`/`Big`, `Absent`/`A`/`B`), the record field names + order (`v`, `tag`), and the
    /// integer payloads. A pure-fn invariant of the encoder (no guest drive) — the non-vacuous marshal gate
    /// is the itest conformance run; this guards the encoding a wrong ctor name/field/int would silently break.
    #[test]
    fn arg_probe_encodes_the_canonical_value_form() {
        use super::ap;
        use crate::contract_value::{as_bare_ctor, read_uint, record_field};
        use cadenza_ast::ast::CompoundCtor;
        use cadenza_ast::codec;

        // probe-record { v: Big(5), tag: 42 } -> bare (record (= v (Big 5)) (= tag 42)) at the root: no
        // ascription frame (operator directive 2026-09-12), so the record reads structurally from the root.
        let bytes = super::encode_probe_record(&ap::ProbeRecord {
            v: ap::Mixed::Big(5),
            tag: 42,
        });
        let arenas = codec::decode(&bytes).expect("probe-record decodes");
        let v = record_field(&arenas, arenas.root, "v").expect("field v");
        let big = as_bare_ctor(&arenas, v, "Big").expect("v is (Big _)");
        assert_eq!(read_uint(&arenas, big[0]), Some(5), "Big payload");
        let tag = record_field(&arenas, arenas.root, "tag").expect("field tag");
        assert_eq!(read_uint(&arenas, tag), Some(42), "tag");

        // list<narrow> [A(7), Absent, B(300)] -> bare <native Ctor(List)>[(A 7) (Absent unit) (B 300)]
        // at the root (no ascription frame; the list reads structurally from the root).
        let bytes =
            super::encode_narrow_list(&[ap::Narrow::A(7), ap::Narrow::Absent, ap::Narrow::B(300)]);
        let arenas = codec::decode(&bytes).expect("list decodes");
        let elems = arenas
            .compound_form_of(arenas.root, CompoundCtor::List)
            .expect("native Ctor(List) list");
        assert_eq!(elems.len(), 3, "three narrow elements");
        let a = as_bare_ctor(&arenas, elems[0], "A").expect("(A _)");
        assert_eq!(read_uint(&arenas, a[0]), Some(7), "A payload");
        // A NULLARY multi-constructor variant is `(Absent unit)` — the constructor applied to the `unit` atom,
        // exactly what a Cadenza checker's `Value.encode` of the same value produces (`codec.rs`). It is
        // `Absent`, not `None` (a `None` ctor would shadow Option.None in the Cadenza checker).
        let absent = as_bare_ctor(&arenas, elems[1], "Absent").expect("(Absent unit)");
        assert_eq!(absent.len(), 1, "the nullary variant carries the unit atom");
        assert!(
            crate::contract_value::is_unit(&arenas, absent[0]),
            "the Absent payload is the unit atom"
        );
        let b_case = as_bare_ctor(&arenas, elems[2], "B").expect("(B _)");
        assert_eq!(read_uint(&arenas, b_case[0]), Some(300), "B payload");
    }

    #[tokio::test]
    async fn identity_returns_the_reducers_own_id() {
        // The `identity` host import hands the guest its own reducer-id, as the id's raw hash bytes.
        let id = ReducerId::of(b"me");
        let mut host = host(id);
        assert_eq!(Identity::id(&mut host).await, id.hash().as_bytes().to_vec());
    }

    /// A HostState whose `run` capability is a real (empty) [`Instantiator`] — no programs in its store, so a
    /// run resolves the error paths without a wasm program. The success path (a run returns a pure program's
    /// output) needs a real wasm pure component and is covered by the reducer-world guest e2e, not natively.
    fn host_with_empty_run() -> HostState {
        let inst = Arc::new(
            super::Instantiator::new(
                Arc::new(InMemoryBlobStore::new()),
                super::ResourceLimits::default(),
            )
            .expect("wasm engine"),
        );
        let mut h = host(ReducerId::of(b"caller"));
        h.run = Some(inst);
        h
    }

    #[tokio::test]
    async fn run_host_import_maps_errors_to_the_runtime_error() {
        use super::cadenza::platform::run::Host as Run;
        let mut h = host_with_empty_run();
        // A program not in the store cannot be run — mapped to `missing-handler`.
        let absent = crate::ProgramHash::of(b"absent");
        assert_eq!(
            Run::run(
                &mut h,
                absent.hash().as_bytes().to_vec(),
                [0u8; Hash::LEN].to_vec(),
                b"x".to_vec(),
            )
            .await,
            Err(super::wit_types::Error::MissingHandler)
        );
        // A malformed program hash names no program — a faulted run, not a panic.
        assert_eq!(
            Run::run(&mut h, b"not-a-hash".to_vec(), vec![], vec![]).await,
            Err(super::wit_types::Error::Faulted)
        );
        // No run capability wired at all is also a faulted run (a bare HostState).
        let mut bare = host(ReducerId::of(b"caller"));
        assert_eq!(
            Run::run(
                &mut bare,
                crate::ProgramHash::of(b"p").hash().as_bytes().to_vec(),
                [0u8; Hash::LEN].to_vec(),
                vec![],
            )
            .await,
            Err(super::wit_types::Error::Faulted)
        );
    }

    #[tokio::test]
    async fn a_run_call_is_recorded_via_the_run_sink() {
        // The RunSink seam (§9): a `run` host call is recorded — program/contract/input + the run's result —
        // so a conformance run can observe that a reducer invoked `run` (it leaves no `step.requests` entry).
        // Exercised on the error path (an empty run store → the program is absent → `RunError::UnknownProgram`),
        // which still reaches the hook after `run_pure`; the Ok path needs a real wasm program (covered e2e by
        // the conformance run v-platform-itest builds on this seam).
        use super::cadenza::platform::run::Host as Run;
        use super::{RunError, RunSink};
        use std::sync::Mutex;

        // (program, contract, input, is_ok) captured per recorded run.
        type Captured = (Vec<u8>, Vec<u8>, Vec<u8>, bool);
        #[derive(Default)]
        struct Capturing {
            calls: Mutex<Vec<Captured>>,
        }
        impl RunSink for Capturing {
            fn record(
                &self,
                program: &[u8],
                contract: &[u8],
                input: &[u8],
                result: &Result<Bytes, RunError>,
            ) {
                self.calls.lock().unwrap().push((
                    program.to_vec(),
                    contract.to_vec(),
                    input.to_vec(),
                    result.is_ok(),
                ));
            }
        }

        let sink = Arc::new(Capturing::default());
        let mut h = host_with_empty_run();
        h.run_sink = Some(sink.clone() as Arc<dyn RunSink>);

        let program = crate::ProgramHash::of(b"absent");
        let contract = crate::ContractId::of(b"c");
        assert_eq!(
            Run::run(
                &mut h,
                program.hash().as_bytes().to_vec(),
                contract.hash().as_bytes().to_vec(),
                b"the-input".to_vec(),
            )
            .await,
            Err(super::wit_types::Error::MissingHandler),
        );
        let calls = sink.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "the run call is recorded once");
        assert_eq!(calls[0].0, program.hash().as_bytes(), "program id bytes");
        assert_eq!(calls[0].1, contract.hash().as_bytes(), "contract id bytes");
        assert_eq!(calls[0].2, b"the-input", "input bytes verbatim");
        assert!(
            !calls[0].3,
            "the absent-program run is recorded as an Err(RunError), not Ok"
        );
    }

    #[tokio::test]
    async fn blobs_round_trip_and_a_malformed_hash_is_absent() {
        let mut host = host(ReducerId::of(b"me"));
        // `put` stores the bytes and returns their content hash; `get` reads them back by that hash.
        let hash = Blobs::put(&mut host, b"a blob".to_vec()).await.unwrap();
        assert_eq!(
            Blobs::get(&mut host, hash).await.unwrap().as_deref(),
            Some(b"a blob".as_slice())
        );
        // A hash the store does not hold reads back as absent, and so does a malformed (wrong-length) hash.
        assert_eq!(
            Blobs::get(&mut host, b"not a real hash".to_vec())
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn state_get_put_delete() {
        let mut host = host(ReducerId::of(b"me"));
        // Absent key reads back as nothing; put then get returns the value; delete removes it.
        assert_eq!(State::get(&mut host, b"k".to_vec()).await.unwrap(), None);
        State::put(&mut host, b"k".to_vec(), b"v".to_vec())
            .await
            .unwrap();
        assert_eq!(
            State::get(&mut host, b"k".to_vec())
                .await
                .unwrap()
                .as_deref(),
            Some(b"v".as_slice())
        );
        State::delete(&mut host, b"k".to_vec()).await.unwrap();
        assert_eq!(State::get(&mut host, b"k".to_vec()).await.unwrap(), None);
    }

    #[tokio::test]
    async fn graph_insert_link_and_read_back() {
        let mut host = host(ReducerId::of(b"me"));
        let (a, b, kind) = (rid_bytes(b"a"), rid_bytes(b"b"), kind_bytes(b"edge"));
        assert!(Graph::insert(&mut host, a.clone()).await);
        assert!(Graph::insert(&mut host, b.clone()).await);
        assert!(Graph::link(&mut host, a.clone(), b.clone(), kind.clone()).await);
        // `a`'s outgoing `kind` neighbours are `[b]`; `b`'s incoming are `[a]`.
        assert_eq!(
            Graph::neighbors(&mut host, a.clone(), kind.clone(), Dir::Outgoing).await,
            vec![b.clone()]
        );
        assert_eq!(
            Graph::neighbors(&mut host, b, kind, Dir::Incoming).await,
            vec![a]
        );
        // A malformed (wrong-length) node names nothing.
        assert!(!Graph::contains(&mut host, b"not a hash".to_vec()).await);
    }

    #[tokio::test]
    async fn a_malformed_graph_arg_is_recorded_as_a_rejected_call_not_silently_dropped() {
        // The observation-completeness seam (§9): a `graph` op whose raw `list<u8>` arg fails to parse returns
        // the empty/false result BUT records the rejected call to the injected `RejectedSink` with the raw
        // bytes — so it is observed even though it never reached the recordable `self.graph`. A well-formed call
        // does NOT hit the sink (it records via the graph decorator instead). This locks that no host call is
        // silently unobservable, per the log-all-host-calls invariant.
        use super::RejectedSink;
        use std::sync::Mutex;

        #[derive(Default)]
        struct Capturing {
            calls: Mutex<Vec<(String, String, Vec<Bytes>)>>,
        }
        impl RejectedSink for Capturing {
            fn record(&self, iface: &str, op: &str, raw_args: &[Bytes]) {
                self.calls.lock().unwrap().push((
                    iface.to_string(),
                    op.to_string(),
                    raw_args.to_vec(),
                ));
            }
        }

        let sink = Arc::new(Capturing::default());
        let mut host = host(ReducerId::of(b"me"));
        host.rejected = Some(sink.clone() as Arc<dyn RejectedSink>);

        // A malformed node (not a 33-byte hash) with a well-formed kind: the guard fails on the node, so
        // `neighbors` returns [] and records the rejected call with BOTH raw args verbatim.
        let bad_node = vec![1u8, 2, 3];
        let kind = kind_bytes(b"edge");
        assert!(
            Graph::neighbors(&mut host, bad_node.clone(), kind.clone(), Dir::Outgoing)
                .await
                .is_empty()
        );
        let malformed_link_kind = b"nope".to_vec();
        assert!(
            !Graph::link(
                &mut host,
                rid_bytes(b"a"),
                rid_bytes(b"b"),
                malformed_link_kind.clone(),
            )
            .await
        );
        // A WELL-FORMED read does not record a rejection (it reaches `self.graph`).
        assert!(
            Graph::neighbors(&mut host, rid_bytes(b"a"), kind.clone(), Dir::Outgoing)
                .await
                .is_empty()
        );

        let calls = sink.calls.lock().unwrap();
        assert_eq!(calls.len(), 2, "only the two malformed calls are recorded");
        assert_eq!(
            calls[0],
            (
                "graph".to_string(),
                "neighbors".to_string(),
                vec![Bytes::from(bad_node), Bytes::from(kind)],
            ),
            "the rejected neighbors call carries iface, op, and the raw args verbatim"
        );
        assert_eq!(calls[1].1, "link");
        assert_eq!(
            calls[1].2,
            vec![
                Bytes::from(rid_bytes(b"a")),
                Bytes::from(rid_bytes(b"b")),
                Bytes::from(malformed_link_kind),
            ]
        );
    }

    #[tokio::test]
    async fn a_malformed_deliver_or_provenance_arg_is_recorded_too() {
        // The same observation-completeness seam (§9) covers the OTHER parsing host ifaces: a `deliver` op with
        // a malformed target and a `provenance.program-of` with a malformed reducer-id each return their
        // empty/false result AND record the rejected call (iface, op, raw target/reducer bytes), so no parsing
        // host call is silently unobservable.
        use super::RejectedSink;
        use super::cadenza::platform::deliver::Host as Deliver;
        use super::cadenza::platform::provenance::Host as Provenance;
        use std::sync::Mutex;

        #[derive(Default)]
        struct Capturing {
            calls: Mutex<Vec<(String, String, Vec<Bytes>)>>,
        }
        impl RejectedSink for Capturing {
            fn record(&self, iface: &str, op: &str, raw_args: &[Bytes]) {
                self.calls.lock().unwrap().push((
                    iface.to_string(),
                    op.to_string(),
                    raw_args.to_vec(),
                ));
            }
        }

        let sink = Arc::new(Capturing::default());
        let mut host = host(ReducerId::of(b"me"));
        host.rejected = Some(sink.clone() as Arc<dyn RejectedSink>);

        // provenance.program-of with a short (non-hash) reducer id: returns empty AND records the rejection.
        let bad_reducer = vec![9u8, 9, 9];
        assert!(
            Provenance::program_of(&mut host, bad_reducer.clone())
                .await
                .is_empty()
        );
        // deliver-message with a malformed target: returns false AND records the rejection (raw target only —
        // the envelope is a structured WIT record). A well-formed envelope isolates the failure to the target.
        let bad_target = vec![1u8, 2];
        let msg = wit_reducer::Message {
            contract: cid(b"c").hash().as_bytes().to_vec(),
            sender: super::origin_to_wit(Origin {
                reducer: ReducerId::of(b"peer"),
                host: HostId::of(b"h"),
            }),
            payload: b"p".to_vec(),
            token: b"t".to_vec(),
        };
        assert!(!Deliver::deliver_message(&mut host, bad_target.clone(), msg).await);

        let calls = sink.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls[0],
            (
                "provenance".to_string(),
                "program-of".to_string(),
                vec![Bytes::from(bad_reducer)],
            )
        );
        assert_eq!(
            calls[1],
            (
                "deliver".to_string(),
                "deliver-message".to_string(),
                vec![Bytes::from(bad_target)],
            )
        );
    }

    #[tokio::test]
    async fn provenance_reports_the_program_a_reducer_runs() {
        use super::cadenza::platform::provenance::Host as WitProvenance;
        use crate::{ProgramHash, Provenance};

        // A stand-in for the node's provenance: one known reducer → its program.
        struct MockProvenance {
            known: ReducerId,
            program: ProgramHash,
        }
        #[async_trait::async_trait]
        impl Provenance for MockProvenance {
            async fn program_of(&self, reducer: ReducerId) -> Option<ProgramHash> {
                (reducer == self.known).then_some(self.program)
            }
        }

        let known = ReducerId::of(b"peer");
        let program = ProgramHash::of(b"peer-program");
        let mut state = host(ReducerId::of(b"me"));
        state.provenance = Arc::new(MockProvenance { known, program });

        // A running reducer's program comes back as its raw hash bytes.
        assert_eq!(
            WitProvenance::program_of(&mut state, known.hash().as_bytes().to_vec()).await,
            program.hash().as_bytes().to_vec()
        );
        // An unknown reducer, a malformed id, and a host with no provenance wired all report absence (empty).
        assert!(
            WitProvenance::program_of(&mut state, rid_bytes(b"stranger"))
                .await
                .is_empty()
        );
        assert!(
            WitProvenance::program_of(&mut state, b"not a hash".to_vec())
                .await
                .is_empty()
        );
        let mut bare = host(ReducerId::of(b"me"));
        assert!(
            WitProvenance::program_of(&mut bare, known.hash().as_bytes().to_vec())
                .await
                .is_empty()
        );
    }

    // ── The event ↔ WIT conversion layer ──
    use super::{StepError, message_to_wit, notification_to_wit, response_to_wit, step_from_wit};
    use super::{message_from_wit, notification_from_wit, response_from_wit};
    use super::{wit_reducer, wit_types};
    use crate::{
        Bytes, ContractId, Error, HostId, Message, Notification, Origin, Outcome, Response,
    };
    use std::time::Duration;

    fn cid(tag: &[u8]) -> ContractId {
        ContractId::of(tag)
    }

    #[test]
    fn a_message_maps_every_field_and_stamps_the_origin() {
        let message = Message {
            id: cid(b"inbound"),
            payload: Bytes::from_static(b"the-input"),
            from: Origin {
                reducer: ReducerId::of(b"peer"),
                host: HostId::of(b"host-a"),
            },
            continuation_token: Bytes::from_static(b"tok"),
        };
        let wit = message_to_wit(&message);
        assert_eq!(wit.contract, message.id.hash().as_bytes().to_vec());
        assert_eq!(wit.payload, b"the-input");
        assert_eq!(wit.token, b"tok");
        // The origin is carried as the sender's two raw hashes — the kernel-stamped provenance a reducer
        // authenticates on.
        assert_eq!(wit.sender.reducer, ReducerId::of(b"peer").hash().as_bytes());
        assert_eq!(wit.sender.host, HostId::of(b"host-a").hash().as_bytes());
    }

    #[test]
    fn a_response_carries_an_ok_payload_or_a_runtime_error() {
        // An answered request: the output value rides in `Ok`.
        let ok = response_to_wit(&Response {
            id: cid(b"c"),
            continuation_token: Bytes::from_static(b"t"),
            payload: Ok(Bytes::from_static(b"out")),
        });
        assert_eq!(ok.answer, Ok(b"out".to_vec()));
        // A runtime-level failure is `Err`, distinct from a handler's domain error (which would be an `Ok`
        // value). Both crate errors map to their WIT counterpart.
        let timeout = response_to_wit(&Response {
            id: cid(b"c"),
            continuation_token: Bytes::from_static(b"t"),
            payload: Err(Error::Timeout),
        });
        assert_eq!(timeout.answer, Err(wit_types::Error::Timeout));
        let missing = response_to_wit(&Response {
            id: cid(b"c"),
            continuation_token: Bytes::from_static(b"t"),
            payload: Err(Error::MissingHandler),
        });
        assert_eq!(missing.answer, Err(wit_types::Error::MissingHandler));
    }

    #[test]
    fn a_notification_maps_its_contract_and_payload() {
        let wit = notification_to_wit(&Notification {
            id: cid(b"lifecycle"),
            payload: Bytes::from_static(b"spawned"),
        });
        assert_eq!(wit.contract, cid(b"lifecycle").hash().as_bytes().to_vec());
        assert_eq!(wit.payload, b"spawned");
    }

    #[test]
    fn a_step_decodes_its_requests_and_a_continue_outcome() {
        let step = wit_reducer::Step {
            requests: vec![wit_reducer::Request {
                contract: cid(b"downstream").hash().as_bytes().to_vec(),
                payload: b"req".to_vec(),
                token: b"corr".to_vec(),
                deadline_nanos: Some(1_500),
            }],
            outcome: wit_reducer::Outcome::Continue,
        };
        let (requests, outcome) = step_from_wit(step).expect("well-formed step");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].id, cid(b"downstream"));
        assert_eq!(requests[0].payload, Bytes::from_static(b"req"));
        assert_eq!(requests[0].continuation_token, Bytes::from_static(b"corr"));
        // The nanosecond deadline round-trips back to a `Duration`.
        assert_eq!(requests[0].deadline, Some(Duration::from_nanos(1_500)));
        assert_eq!(outcome, Outcome::Continue);
    }

    #[test]
    fn a_close_outcome_decodes_its_typed_reason() {
        let step = wit_reducer::Step {
            requests: Vec::new(),
            outcome: wit_reducer::Outcome::Close(wit_reducer::Closed {
                schema: cid(b"done").hash().as_bytes().to_vec(),
                reason: b"finished".to_vec(),
            }),
        };
        let (requests, outcome) = step_from_wit(step).expect("well-formed step");
        assert!(requests.is_empty());
        assert_eq!(
            outcome,
            Outcome::Break {
                schema: cid(b"done"),
                reason: Bytes::from_static(b"finished"),
            }
        );
    }

    #[test]
    fn a_step_naming_a_malformed_id_is_rejected() {
        // A request whose contract-id is not `Hash::LEN` bytes: a misbehaving guest, so the whole step is
        // rejected rather than trusted.
        let bad_request = wit_reducer::Step {
            requests: vec![wit_reducer::Request {
                contract: b"not a real hash".to_vec(),
                payload: Vec::new(),
                token: Vec::new(),
                deadline_nanos: None,
            }],
            outcome: wit_reducer::Outcome::Continue,
        };
        assert_eq!(
            step_from_wit(bad_request),
            Err(StepError::MalformedContractId)
        );
        // The same guard applies to a close reason's schema.
        let bad_close = wit_reducer::Step {
            requests: Vec::new(),
            outcome: wit_reducer::Outcome::Close(wit_reducer::Closed {
                schema: b"nope".to_vec(),
                reason: Vec::new(),
            }),
        };
        assert_eq!(
            step_from_wit(bad_close),
            Err(StepError::MalformedContractId)
        );
    }

    // ── Inbound: an event a privileged reducer hands to `deliver` ──

    #[test]
    fn a_message_from_wit_maps_every_field_and_its_origin() {
        // The inverse of `message_to_wit`: a WIT message an event reducer built decodes to the crate message
        // the system delivers, contract-id and origin recovered from their raw hash bytes.
        let wit = wit_reducer::Message {
            contract: cid(b"inbound").hash().as_bytes().to_vec(),
            sender: super::origin_to_wit(Origin {
                reducer: ReducerId::of(b"peer"),
                host: HostId::of(b"host-a"),
            }),
            payload: b"the-input".to_vec(),
            token: b"tok".to_vec(),
        };
        let message = message_from_wit(wit).expect("a well-formed message");
        assert_eq!(message.id, cid(b"inbound"));
        assert_eq!(message.payload, Bytes::from_static(b"the-input"));
        assert_eq!(message.continuation_token, Bytes::from_static(b"tok"));
        assert_eq!(message.from.reducer, ReducerId::of(b"peer"));
        assert_eq!(message.from.host, HostId::of(b"host-a"));
    }

    #[test]
    fn a_response_from_wit_carries_an_ok_payload_or_a_runtime_error() {
        let ok = response_from_wit(wit_reducer::Response {
            contract: cid(b"c").hash().as_bytes().to_vec(),
            token: b"t".to_vec(),
            answer: Ok(b"out".to_vec()),
        })
        .expect("a well-formed response");
        assert_eq!(ok.payload, Ok(Bytes::from_static(b"out")));
        // Each WIT error maps back to its crate counterpart — total across the three variants (so the
        // response-delivery path never has an untranslatable error).
        for (wit, crate_err) in [
            (wit_types::Error::Timeout, Error::Timeout),
            (wit_types::Error::MissingHandler, Error::MissingHandler),
            (wit_types::Error::SchemaViolation, Error::SchemaViolation),
            (wit_types::Error::Faulted, Error::Faulted),
        ] {
            let r = response_from_wit(wit_reducer::Response {
                contract: cid(b"c").hash().as_bytes().to_vec(),
                token: b"t".to_vec(),
                answer: Err(wit),
            })
            .expect("a well-formed error response");
            assert_eq!(r.payload, Err(crate_err));
        }
    }

    #[test]
    fn a_notification_from_wit_maps_its_contract_and_payload() {
        let note = notification_from_wit(wit_reducer::Notification {
            contract: cid(b"lifecycle").hash().as_bytes().to_vec(),
            payload: b"spawned".to_vec(),
        })
        .expect("a well-formed notification");
        assert_eq!(note.id, cid(b"lifecycle"));
        assert_eq!(note.payload, Bytes::from_static(b"spawned"));
    }

    #[test]
    fn a_malformed_id_or_origin_makes_an_inbound_event_none() {
        // A contract-id that is not `Hash::LEN` bytes names nothing, so the event does not decode.
        assert!(
            message_from_wit(wit_reducer::Message {
                contract: b"not a hash".to_vec(),
                sender: super::origin_to_wit(Origin {
                    reducer: ReducerId::of(b"peer"),
                    host: HostId::of(b"host-a"),
                }),
                payload: Vec::new(),
                token: Vec::new(),
            })
            .is_none()
        );
        // So does a malformed origin — a sender whose reducer bytes are not a hash.
        assert!(
            message_from_wit(wit_reducer::Message {
                contract: cid(b"ok").hash().as_bytes().to_vec(),
                sender: wit_types::Origin {
                    reducer: b"nope".to_vec(),
                    host: HostId::of(b"host-a").hash().as_bytes().to_vec(),
                },
                payload: Vec::new(),
                token: Vec::new(),
            })
            .is_none()
        );
        assert!(
            response_from_wit(wit_reducer::Response {
                contract: b"nope".to_vec(),
                token: Vec::new(),
                answer: Ok(Vec::new()),
            })
            .is_none()
        );
        assert!(
            notification_from_wit(wit_reducer::Notification {
                contract: b"nope".to_vec(),
                payload: Vec::new(),
            })
            .is_none()
        );
    }

    // ── The `deliver` host import ──
    #[tokio::test]
    async fn deliver_routes_each_event_kind_to_the_target_and_declines_gracefully() {
        use super::cadenza::platform::deliver::Host as Deliver;
        use crate::{Delivered, Delivery};
        use std::sync::Mutex as StdMutex;

        // A stand-in node delivery: record every (target, event) it is handed, and report the target received
        // it — so the test observes both what the host converted and that it routed through the delivery.
        #[derive(Default)]
        struct MockDelivery {
            delivered: StdMutex<Vec<(ReducerId, Delivered)>>,
        }
        #[async_trait::async_trait]
        impl Delivery for MockDelivery {
            async fn deliver(&self, target: ReducerId, event: Delivered) -> bool {
                self.delivered.lock().unwrap().push((target, event));
                true
            }
        }

        let delivery = Arc::new(MockDelivery::default());
        let mut state = host(ReducerId::of(b"event-reducer"));
        state.delivery = delivery.clone();
        let target = ReducerId::of(b"next-handler");

        // A message, a response, and a notification each convert and route to the target, reporting delivered.
        assert!(
            Deliver::deliver_message(
                &mut state,
                target.hash().as_bytes().to_vec(),
                wit_reducer::Message {
                    contract: cid(b"http.get").hash().as_bytes().to_vec(),
                    sender: super::origin_to_wit(Origin {
                        reducer: ReducerId::of(b"caller"),
                        host: HostId::of(b"node"),
                    }),
                    payload: b"req".to_vec(),
                    token: b"k".to_vec(),
                },
            )
            .await
        );
        assert!(
            Deliver::deliver_response(
                &mut state,
                target.hash().as_bytes().to_vec(),
                wit_reducer::Response {
                    contract: cid(b"http.get").hash().as_bytes().to_vec(),
                    token: b"k".to_vec(),
                    answer: Ok(b"200".to_vec()),
                },
            )
            .await
        );
        assert!(
            Deliver::deliver_notification(
                &mut state,
                target.hash().as_bytes().to_vec(),
                wit_reducer::Notification {
                    contract: cid(b"lifecycle").hash().as_bytes().to_vec(),
                    payload: b"exited".to_vec(),
                },
            )
            .await
        );

        {
            // Read the recorded deliveries in a scope so the guard drops before the next `.await`s.
            let delivered = delivery.delivered.lock().unwrap();
            assert_eq!(delivered.len(), 3, "all three kinds routed to the delivery");
            assert!(delivered.iter().all(|(t, _)| *t == target));
            assert!(matches!(delivered[0], (_, Delivered::Message(_))));
            assert!(matches!(delivered[1], (_, Delivered::Response(_))));
            assert!(matches!(delivered[2], (_, Delivered::Notification(_))));
        }

        // A malformed target names no reducer, so the delivery is not attempted (false, nothing recorded).
        assert!(
            !Deliver::deliver_message(
                &mut state,
                b"not a hash".to_vec(),
                wit_reducer::Message {
                    contract: cid(b"c").hash().as_bytes().to_vec(),
                    sender: super::origin_to_wit(Origin {
                        reducer: ReducerId::of(b"caller"),
                        host: HostId::of(b"node"),
                    }),
                    payload: Vec::new(),
                    token: Vec::new(),
                },
            )
            .await
        );
        assert_eq!(
            delivery.delivered.lock().unwrap().len(),
            3,
            "a malformed target records no new delivery"
        );

        // With no real delivery wired (the NoDelivery default), a well-formed deliver reports not-delivered.
        let mut bare = host(ReducerId::of(b"event-reducer"));
        assert!(
            !Deliver::deliver_notification(
                &mut bare,
                target.hash().as_bytes().to_vec(),
                wit_reducer::Notification {
                    contract: cid(b"lifecycle").hash().as_bytes().to_vec(),
                    payload: Vec::new(),
                },
            )
            .await
        );
    }

    // ── The wasm program store ──
    use super::WasmProgramStore;
    use crate::{BlobStore, KvStore, ProgramHash, ProgramStore, ReducerKind, SpawnContext};

    fn wasm_program_store(cas: Arc<dyn BlobStore>) -> WasmProgramStore {
        // Fresh per-reducer backends; a real harness injects recording-wrapped ones instead. The graph factory
        // hands out the one shared graph (a plain `move |_id| graph.clone()`), mirroring make_blobs/make_kv.
        let graph: Arc<dyn super::ReducerGraph> = Arc::new(InMemoryReducerGraph::new());
        WasmProgramStore::new(
            cas,
            Arc::new(|_id| Box::new(InMemoryBlobStore::new()) as Box<dyn BlobStore>),
            Arc::new(|_id| Box::new(InMemoryKvStore::new()) as Box<dyn KvStore>),
            Arc::new(move |_id| graph.clone()),
        )
        .expect("build the wasm program store")
    }

    fn ord(id: &[u8]) -> SpawnContext {
        SpawnContext {
            id: ReducerId::of(id),
            kind: ReducerKind::Ordinary,
            limits: None,
        }
    }

    #[tokio::test]
    async fn resolves_a_program_by_its_blob_addressed_bytes_and_declines_gracefully() {
        // Seed the CAS with some bytes as an ordinary blob — the way an input program blob is seeded.
        let cas = InMemoryBlobStore::new();
        let bytes = b"not a valid wasm component".to_vec();
        let blob = cas.put(Bytes::from(bytes.clone())).await.unwrap();
        // The program is the Program-tagged view of those same bytes; it shares the blob's digest, so the
        // content-keyed store resolves it (the tag is ignored).
        let program = ProgramHash::of(&bytes);
        assert_eq!(program.hash().digest(), blob.digest());

        let store = wasm_program_store(Arc::new(cas));
        // `contains` finds it (the store keys on content, so the program hash hits the seeded bytes)...
        assert!(store.contains(program).await);
        // ...but the bytes are not a valid component, so `spawn` declines with `None` rather than panicking.
        assert!(store.spawn(program, ord(b"r")).await.is_none());

        // A program never seeded is absent and unspawnable.
        let unknown = ProgramHash::of(b"never stored");
        assert!(!store.contains(unknown).await);
        assert!(store.spawn(unknown, ord(b"r")).await.is_none());
    }

    #[tokio::test]
    async fn spawn_invokes_each_capability_factory_with_the_reducers_id() {
        // The #3197/#3199 injection seam: `spawn` assembles the reducer's `HostState` by calling
        // `make_graph`/`make_provenance`/`make_delivery` (uniform with `make_blobs`/`make_kv`) with the
        // reducer's id, so a caller may inject a per-reducer capability — a decorator, a recording wrapper, a
        // stand-in — without the store knowing. This locks that the factories ARE invoked, per-reducer, with
        // the spawn id: a regression reverting to a shared field (dropping the per-id call) would otherwise
        // pass every other test. The factories run during `HostState` assembly, before instantiation, so an
        // absent program (`spawn -> None`) still exercises the seam. (That the returned capability is what the
        // guest's host calls actually hit is proven by the harness recording runs, which need a live guest.)
        use std::sync::Mutex;

        let graph_ids = Arc::new(Mutex::new(Vec::new()));
        let prov_ids = Arc::new(Mutex::new(Vec::new()));
        let deliv_ids = Arc::new(Mutex::new(Vec::new()));

        let g = Arc::clone(&graph_ids);
        let make_graph: Arc<dyn Fn(ReducerId) -> Arc<dyn super::ReducerGraph> + Send + Sync> =
            Arc::new(move |id| {
                g.lock().unwrap().push(id);
                Arc::new(InMemoryReducerGraph::new())
            });
        let p = Arc::clone(&prov_ids);
        let make_prov: Arc<dyn Fn(ReducerId) -> Arc<dyn super::Provenance> + Send + Sync> =
            Arc::new(move |id| {
                p.lock().unwrap().push(id);
                Arc::new(crate::NoProvenance)
            });
        let d = Arc::clone(&deliv_ids);
        let make_deliv: Arc<dyn Fn(ReducerId) -> Arc<dyn super::Delivery> + Send + Sync> =
            Arc::new(move |id| {
                d.lock().unwrap().push(id);
                Arc::new(crate::NoDelivery)
            });

        let cas: Arc<dyn BlobStore> = Arc::new(InMemoryBlobStore::new());
        let store = WasmProgramStore::new(
            cas,
            Arc::new(|_id| Box::new(InMemoryBlobStore::new()) as Box<dyn BlobStore>),
            Arc::new(|_id| Box::new(InMemoryKvStore::new()) as Box<dyn KvStore>),
            make_graph,
        )
        .expect("build the wasm program store")
        .with_provenance(make_prov)
        .with_delivery(make_deliv);

        // Spawn two absent programs with distinct ids: each declines with `None`, but the capability factories
        // were already called to build the `HostState`. Each factory must have seen exactly those ids, in
        // order — the seam runs once per reducer, keyed on its spawn id.
        assert!(
            store
                .spawn(ProgramHash::of(b"absent-a"), ord(b"reducer-a"))
                .await
                .is_none()
        );
        assert!(
            store
                .spawn(ProgramHash::of(b"absent-b"), ord(b"reducer-b"))
                .await
                .is_none()
        );

        let want = vec![ReducerId::of(b"reducer-a"), ReducerId::of(b"reducer-b")];
        assert_eq!(
            *graph_ids.lock().unwrap(),
            want,
            "make_graph called per reducer id"
        );
        assert_eq!(
            *prov_ids.lock().unwrap(),
            want,
            "make_provenance called per reducer id"
        );
        assert_eq!(
            *deliv_ids.lock().unwrap(),
            want,
            "make_delivery called per reducer id"
        );
    }

    #[tokio::test]
    async fn a_runaway_guest_is_preempted_it_yields_then_traps_rather_than_hanging() {
        // The preemption mechanism (`reducer_engine` epoch_interruption + `arm_store_safety`): a guest that
        // never returns must not monopolize the executor thread — it yields — and must eventually trap once its
        // compute budget is spent, so a runaway fold fails cleanly instead of hanging the runtime. This proves
        // both halves against real wasmtime with a minimal forever-looping core module, the same
        // yield-then-trap callback shape `arm_store_safety` installs, and an epoch ticker like the kernel's:
        //   - if the yield did NOT return control to the executor, the ticker task (below) would never run on
        //     this current-thread runtime, the epoch would never advance, and the call would hang forever —
        //     so the test completing at all proves the anti-monopolization yield;
        //   - the assertion proves the budget-exhaustion trap.
        use wasmtime::{Config, Engine, Instance, Module, Store, UpdateDeadline};

        let mut config = Config::new();
        config.async_support(true);
        config.epoch_interruption(true);
        let engine = Engine::new(&config).expect("engine");
        // A function that never returns.
        let wasm = wat::parse_str(r#"(module (func (export "spin") (loop br 0)))"#).expect("wat");
        let module = Module::from_binary(&engine, &wasm).expect("module");

        let mut store = Store::new(&engine, ());
        // The same policy shape as `arm_store_safety`, with a tiny budget so the test is fast.
        store.set_epoch_deadline(1);
        let mut yields_left = 3u64;
        store.epoch_deadline_callback(move |_ctx| {
            if yields_left == 0 {
                Ok(UpdateDeadline::Interrupt)
            } else {
                yields_left -= 1;
                Ok(UpdateDeadline::Yield(1))
            }
        });

        // The epoch ticker on a DEDICATED OS thread, exactly as the kernel drives it (see
        // `TaskSystem::start_epoch_ticker`): it advances the epoch even while the spinning guest holds the
        // async worker thread — a ticker on the runtime's own pool would be starved by that very spin (the
        // current-thread deadlock this replaces). `stop` ends the thread when the test is done.
        let ticker_engine = engine.clone();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ticker_stop = Arc::clone(&stop);
        let ticker = std::thread::spawn(move || {
            while !ticker_stop.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(1));
                ticker_engine.increment_epoch();
            }
        });

        let instance = Instance::new_async(&mut store, &module, &[])
            .await
            .expect("instantiate");
        let spin = instance
            .get_typed_func::<(), ()>(&mut store, "spin")
            .expect("export");
        let result = spin.call_async(&mut store, ()).await;
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        ticker.join().ok();
        assert!(
            result.is_err(),
            "a runaway guest must trap once its epoch budget is exhausted, not run forever"
        );
    }

    #[tokio::test]
    async fn the_reducer_engine_pools_instances_serving_many_concurrent_live_memories() {
        // The pooling instance allocator (`reducer_engine`): a burst of INDEPENDENT folds must instantiate from
        // a pre-reserved, reused linear-memory slab rather than mmap fresh memory per fold — the per-fold mmap
        // the on-demand default did serialized on the process mmap_lock, capping fold concurrency ~8 and adding
        // the ~26ms/request kernel floor this switch removes. Prove BOTH halves against real wasmtime:
        //   - the engine builds with the pooling strategy (a mis-sized config would error at `Engine::new`);
        //   - many instances — each with its OWN linear memory it writes then reads back — are held ALIVE AT
        //     ONCE and all round-trip their own memory. Holding them concurrently is the assertion: it needs
        //     that many pooled slots simultaneously, so an under-sized pool (too few total_memories /
        //     total_core_instances / total_stacks) would fail to instantiate here rather than silently in prod.
        // This is the exact shape of a concurrent gateway fold burst; determinism is unchanged (independent
        // instances across slots, never one session's fold parallelized).
        use wasmtime::{Instance, Module, Store};

        let engine = super::reducer_engine(&super::ResourceLimits::default())
            .expect("pooling engine builds");
        // A tiny core module with its own linear memory + a fn that stores then loads a value — so each instance
        // genuinely uses (and isolates) a pooled memory slot.
        let wasm = wat::parse_str(
            r#"(module
                 (memory (export "mem") 1)
                 (func (export "roundtrip") (param i32) (result i32)
                   (i32.store (i32.const 0) (local.get 0))
                   (i32.load (i32.const 0))))"#,
        )
        .expect("wat");
        let module = Module::from_binary(&engine, &wasm).expect("module");

        // Instantiate many instances and HOLD THEM ALL ALIVE (a Vec of live stores) — more than the ~64 target
        // concurrency — so the pool must serve that many slots simultaneously.
        const CONCURRENT: i32 = 96;
        let mut live = Vec::with_capacity(CONCURRENT as usize);
        for _ in 0..CONCURRENT {
            let mut store = Store::new(&engine, ());
            // The engine enables epoch_interruption (arm_store_safety arms real stores + the runtime ticks the
            // epoch); this isolated test drives no ticker, so a deadline past the never-advancing epoch keeps
            // the round-trip from tripping the default (0 >= 0) interrupt.
            store.set_epoch_deadline(u64::MAX);
            let instance = Instance::new_async(&mut store, &module, &[])
                .await
                .expect("instantiate from the pool");
            live.push((store, instance));
        }
        // Each live instance round-trips a distinct value through its OWN memory — proving the pooled slots are
        // isolated, not aliased.
        for (i, (store, instance)) in live.iter_mut().enumerate() {
            let roundtrip = instance
                .get_typed_func::<i32, i32>(&mut *store, "roundtrip")
                .expect("export");
            assert_eq!(
                roundtrip
                    .call_async(&mut *store, i as i32)
                    .await
                    .expect("call"),
                i as i32,
                "each pooled instance round-trips its own memory in isolation"
            );
        }
    }

    #[tokio::test]
    async fn a_pooled_reducer_grows_its_memory_up_to_the_ceiling_and_is_rejected_past_it() {
        // Pins the pooling sizing invariant (reducer_engine): max_memory_size = the node's per-reducer
        // max_linear_memory_bytes, with the per-slot address reservation kept >= 4 GiB. A pooled memory must
        // still GROW WITHIN its slot up to that ceiling (the "pooling memories never move" rule only bites when
        // growth would exceed the RESERVATION, which is >> the ceiling here — so it must NOT false-reject a
        // legitimate grow) AND memory.grow past the ceiling must return -1 (capped, not OOMing the host). This
        // is the exact mis-sizing regression class flagged for pooling configs; the sibling
        // `a_guest_that_exhausts_memory_traps...` covers only the DEFAULT on-demand engine + StoreLimits, not
        // this pooling path.
        use wasmtime::{Instance, Module, Store};

        // A small, page-aligned ceiling (4 wasm pages = 256 KiB) so the test needs no real memory.
        const PAGE: usize = 64 * 1024;
        let limits = super::ResourceLimits {
            max_linear_memory_bytes: 4 * PAGE,
            ..super::ResourceLimits::default()
        };
        let engine = super::reducer_engine(&limits).expect("pooling engine builds");
        // memory.grow returns the previous page count on success, or -1 (0xffff_ffff as i32) on failure.
        let wasm = wat::parse_str(
            r#"(module
                 (memory 1)
                 (func (export "grow") (param i32) (result i32) (memory.grow (local.get 0))))"#,
        )
        .expect("wat");
        let module = Module::from_binary(&engine, &wasm).expect("module");

        let mut store = Store::new(&engine, ());
        // The engine enables epoch_interruption; no ticker here, so a deadline past the never-advancing epoch
        // keeps grow from tripping the default (0 >= 0) interrupt.
        store.set_epoch_deadline(u64::MAX);
        let instance = Instance::new_async(&mut store, &module, &[])
            .await
            .expect("instantiate from the pool");
        let grow = instance
            .get_typed_func::<i32, i32>(&mut store, "grow")
            .expect("export");

        // Start at 1 page; grow by 2 -> 3 pages, within the 4-page ceiling. memory.grow returns the previous
        // size (1), NOT -1 — the pooled slot commits within its reservation without moving.
        assert_eq!(
            grow.call_async(&mut store, 2)
                .await
                .expect("grow within ceiling"),
            1,
            "a pooled memory must grow up to its max_memory_size ceiling (no false 'never move' reject)"
        );
        // Now at 3 pages; grow by 2 more -> 5 pages, PAST the 4-page ceiling. Capped: memory.grow returns -1.
        assert_eq!(
            grow.call_async(&mut store, 2)
                .await
                .expect("grow call returns"),
            -1,
            "a pooled memory growing past its max_memory_size ceiling is rejected (-1), not granted"
        );
    }

    #[test]
    fn the_instantiate_observer_records_sub_steps_only_when_installed() {
        // The dep-free instantiate-cost hook (InstantiateObserver): an un-observed host pays ZERO (no clock
        // reading, observe_end a no-op), and an installed observer receives exactly the sub-steps recorded, in
        // order. This pins the hook contract the Membrain embedder wires to wasm_instantiate_us{sub_step}; the
        // sub-steps firing during a real instantiate_program are exercised end-to-end on the rig / itest once an
        // observer is installed there.
        use super::{InstantiateObserver, InstantiateSubStep, Instantiator};
        use std::sync::Mutex;
        use std::time::Duration;

        #[derive(Default)]
        struct Rec(Mutex<Vec<InstantiateSubStep>>);
        impl InstantiateObserver for Rec {
            fn record(&self, sub_step: InstantiateSubStep, _elapsed: Duration) {
                self.0.lock().expect("rec lock").push(sub_step);
            }
        }

        // Un-observed (the default): no clock is taken and observe_end is a no-op (zero overhead).
        let inst = Instantiator::new(
            Arc::new(InMemoryBlobStore::new()),
            super::ResourceLimits::default(),
        )
        .expect("engine");
        assert!(
            inst.observe_start().is_none(),
            "no observer → no clock reading"
        );
        inst.observe_end(None, InstantiateSubStep::ComponentLoad); // no-op, must not panic

        // Observed: each sub-step is recorded, in order.
        let rec = Arc::new(Rec::default());
        let mut inst2 = Instantiator::new(
            Arc::new(InMemoryBlobStore::new()),
            super::ResourceLimits::default(),
        )
        .expect("engine");
        inst2.observer = Some(rec.clone() as Arc<dyn InstantiateObserver>);
        let t = inst2.observe_start();
        assert!(t.is_some(), "observer installed → clock taken");
        inst2.observe_end(t, InstantiateSubStep::ComponentLoad);
        inst2.observe_end(inst2.observe_start(), InstantiateSubStep::BindDependencies);
        inst2.observe_end(inst2.observe_start(), InstantiateSubStep::WorldInstantiate);
        assert_eq!(
            *rec.0.lock().expect("rec lock"),
            vec![
                InstantiateSubStep::ComponentLoad,
                InstantiateSubStep::BindDependencies,
                InstantiateSubStep::WorldInstantiate
            ],
            "the observer receives exactly the recorded sub-steps, in order"
        );

        // The WasmProgramStore builder installs the observer into the shared core at assembly.
        let graph: Arc<dyn super::ReducerGraph> = Arc::new(InMemoryReducerGraph::new());
        let store = super::WasmProgramStore::new(
            Arc::new(InMemoryBlobStore::new()),
            Arc::new(|_id| Box::new(InMemoryBlobStore::new()) as Box<dyn BlobStore>),
            Arc::new(|_id| Box::new(InMemoryKvStore::new()) as Box<dyn KvStore>),
            Arc::new(move |_id| graph.clone()),
        )
        .expect("build store")
        .with_instantiate_observer(Arc::new(Rec::default()));
        assert!(
            store.inst.observer.is_some(),
            "with_instantiate_observer installs the observer on the shared instantiation core"
        );
    }

    #[test]
    fn a_guest_that_exhausts_memory_traps_rather_than_ooming_the_host() {
        // The memory half of `arm_store_safety` (`reducer_store_limits`): a guest that grows its linear memory
        // past the ceiling must trap — a clean per-reducer failure — rather than exhaust host RAM and take the
        // process down. Proven with the same policy shape (memory_size + trap_on_grow_failure) applied to a
        // module that tries to grow far past a tiny test ceiling.
        use wasmtime::{Config, Engine, Instance, Module, Store, StoreLimitsBuilder};

        let engine = Engine::new(&Config::new()).expect("engine");
        // Starts at 1 page; the exported function tries to grow by 1000 pages (~64 MiB), far past the ceiling.
        let wasm = wat::parse_str(
            r#"(module (memory 1) (func (export "grow") (drop (memory.grow (i32.const 1000)))))"#,
        )
        .expect("wat");
        let module = Module::from_binary(&engine, &wasm).expect("module");

        // A tiny 2-page ceiling with trap-on-grow — the same policy shape as `reducer_store_limits`, sized down
        // so the test needs no real memory.
        let limits = StoreLimitsBuilder::new()
            .memory_size(2 * 64 * 1024)
            .trap_on_grow_failure(true)
            .build();
        let mut store = Store::new(&engine, limits);
        store.limiter(|l| l);

        let instance = Instance::new(&mut store, &module, &[]).expect("instantiate");
        let grow = instance
            .get_typed_func::<(), ()>(&mut store, "grow")
            .expect("export");
        assert!(
            grow.call(&mut store, ()).is_err(),
            "a guest growing memory past its ceiling must trap, not exhaust host RAM"
        );
    }

    #[test]
    fn a_configured_resource_limit_actually_reaches_the_store_not_a_hard_coded_default() {
        // The operator's requirement (no hard-coded caps): a node's configured `ResourceLimits` must actually
        // flow through, not be shadowed by a baked-in constant. Build the store with a non-default epoch tick
        // and assert the `epoch_incrementer` the kernel's ticker drives reports THAT cadence — proof the config
        // seam is live end to end (`with_resource_limits` → `Instantiator` → `ReducerHost.limits` →
        // `epoch_incrementer`), and that it differs from the default (so the value is genuinely varied, not
        // ignored). The compute/memory budgets ride the same `limits`, armed per store by `arm_store_safety`.
        use crate::ResourceLimits;
        use std::time::Duration;

        let configured = ResourceLimits {
            epoch_tick: Duration::from_millis(7),
            ..ResourceLimits::default()
        };
        let graph: Arc<dyn super::ReducerGraph> = Arc::new(InMemoryReducerGraph::new());
        let store = WasmProgramStore::with_resource_limits(
            Arc::new(InMemoryBlobStore::new()),
            Arc::new(|_id| Box::new(InMemoryBlobStore::new()) as Box<dyn BlobStore>),
            Arc::new(|_id| Box::new(InMemoryKvStore::new()) as Box<dyn KvStore>),
            Arc::new(move |_id| graph.clone()),
            configured,
        )
        .expect("build the wasm program store");

        let (tick, _increment) = store
            .epoch_incrementer()
            .expect("the wasm store has an epoch incrementer");
        assert_eq!(
            tick,
            Duration::from_millis(7),
            "the ticker uses the CONFIGURED epoch tick, not a hard-coded default"
        );
        // The default constructor uses the default tick — so a configured value genuinely changes behavior.
        let (default_tick, _) = wasm_program_store(Arc::new(InMemoryBlobStore::new()))
            .epoch_incrementer()
            .expect("incrementer");
        assert_eq!(default_tick, ResourceLimits::default().epoch_tick);
        assert_ne!(
            tick, default_tick,
            "a configured tick differs from the default — the config is not ignored"
        );
    }

    #[test]
    fn a_dependency_import_name_resolves_to_its_content_address() {
        use super::dependency_address;
        // A dependency import carries the dep component's content hash in canonical base62 (§8, `Hash` Display)
        // after `+`; it must resolve to the same content the store keys under (the digest), whatever the tag.
        let dep_bytes = b"the value-heap runtime component";
        let dep = Hash::of(HashTag::Blob, dep_bytes);
        let import = format!("cadenza:runtime/heap@0.0.0+{dep}"); // Hash `Display` is base62
        let parsed = dependency_address(&import).expect("a +<base62> import is a dependency");
        assert_eq!(
            parsed.digest(),
            dep.digest(),
            "resolves to the dep's content in the store"
        );
        // A platform host interface carries no `+…` — it is served by the host, not the store.
        assert!(dependency_address("cadenza:platform/state").is_none());
        assert!(dependency_address("cadenza:platform/identity").is_none());
        // A malformed suffix (not a valid base62 hash) names no content.
        assert!(dependency_address("dep:x/y@1.0.0+not a valid base62 hash!").is_none());
    }

    // The end-to-end driver test — seed the reducer-echo guest component's bytes into the store, spawn its
    // ProgramHash, drive a message, assert the echo + the identity import round-trip — is the slice that wires
    // the guest component into the reproducible nix build (operator: no committed .wasm fixture; the guest is
    // built by cargo-component in the wasm CI job and its bytes flow in as an input blob, not a fixture fn).
    // (The driver + this store were verified locally against a `cargo component build` of guests/reducer-echo,
    // and — see the module docs — the whole instantiate-and-drive path was verified to run under `bach::sim`,
    // not just tokio, so the integration harness's deterministic bach-driven run over this store is sound.)
    //
    // The dependency-composition path (`bind_dependencies` / `alias_instance_exports`) is exercised end to end
    // by that same slice using a component that imports the value-heap runtime: only a Cadenza-compiled guest
    // carries the `cadenza:runtime/heap@…+<hash>` content-addressed import (cargo-component uses semver, not a
    // content hash, so it cannot reproduce the convention), so the behavioural test lands with v-rust-backend's
    // first runtime-importing guest. The address parsing + dependency detection are unit-tested above, and the
    // instantiate-and-alias mirrors the value-heap composition `cdz-run` performs against real components.

    // ── Warm per-fold EXECUTION-cost measurement (operator: "how expensive is it to run a single reducer
    // function? if it's not like 200µs it's not fast enough") ──────────────────────────────────────────────
    //
    // Measures the cost to EXECUTE one reducer fold (a single `on_message` invocation) on a WARM instance —
    // instantiate ONCE, then drive N folds through the exact production fold path (`message_to_wit` encode →
    // the wasmtime component `call_on_message` lift/lower + guest execute → `step_from_wit` decode). This is
    // the per-invocation execution cost ISOLATED from instantiate (measured once, up front) and from the
    // durable/consensus round-trip (there is none here — a bare in-memory store, no network, no reply-wait).
    //
    // `#[ignore]` + env-gated: there is no committed `.wasm` reducer fixture (the Cadenza reducer-echo guest is
    // built by the wasm CI job / nix `reducerEchoComponent`). The real Cadenza guest imports the value-heap
    // runtime, which itself imports the nfc component — the whole content-addressed closure must be in the CAS
    // for the compose path to resolve. Point `CDZ_REDUCER_ECHO_WASM` at the reducer-echo component and
    // `CDZ_COMPONENT_STORE_DIR` at a nix `cdz-component-store` directory holding that closure (its `*.wasm`
    // files are named by content hash; seeding extras is harmless — the CAS keys by content). Run with
    // `--nocapture` to see the report:
    //   CDZ_REDUCER_ECHO_WASM=/nix/store/…-cdz-platform-reducer-echo-cdz-component \
    //   CDZ_COMPONENT_STORE_DIR=/nix/store/…-cdz-component-store \
    //     cargo test -p cdz-platform --features host --release -- --ignored --nocapture warm_per_fold
    #[tokio::test]
    #[ignore = "env-gated micro-benchmark; needs CDZ_REDUCER_ECHO_WASM + CDZ_COMPONENT_STORE_DIR"]
    async fn warm_per_fold_execution_cost_of_a_single_reducer_function() {
        use std::time::{Duration, Instant};

        let Ok(path) = std::env::var("CDZ_REDUCER_ECHO_WASM") else {
            eprintln!(
                "CDZ_REDUCER_ECHO_WASM unset — skipping the per-fold execution-cost measurement"
            );
            return;
        };
        let bytes = std::fs::read(&path).expect("read the reducer-echo wasm component");

        // Seed the reducer-echo component AND the whole component-store closure (heap-runtime + nfc + …) into
        // the CAS. Spawn pays the whole instantiate cost (composing the dependency graph) up front — so the
        // timed loop below measures only fold EXECUTION on the resulting warm instance.
        let cas = InMemoryBlobStore::new();
        cas.put(Bytes::from(bytes.clone())).await.unwrap();
        let mut seeded = 0usize;
        if let Ok(dir) = std::env::var("CDZ_COMPONENT_STORE_DIR") {
            for entry in std::fs::read_dir(&dir).expect("read component-store dir") {
                let p = entry.expect("dir entry").path();
                if p.extension().and_then(|e| e.to_str()) == Some("wasm") {
                    cas.put(Bytes::from(std::fs::read(&p).unwrap()))
                        .await
                        .unwrap();
                    seeded += 1;
                }
            }
        }
        eprintln!("seeded reducer-echo + {seeded} component-store component(s) into the CAS");

        let program = ProgramHash::of(&bytes);
        let store = wasm_program_store(Arc::new(cas));
        let Some(mut reducer) = store.spawn(program, ord(b"bench-reducer")).await else {
            eprintln!(
                "spawn DECLINED — is CDZ_COMPONENT_STORE_DIR the closure matching this reducer-echo build?"
            );
            return;
        };

        // A minimal well-formed message the echo guest folds cleanly (it copies the fields back as one request
        // and continues — no validation, no kv writes, no host-import round-trips: pure fold compute).
        let base = crate::Message {
            id: crate::ContractId::of(b"echo-contract"),
            payload: Bytes::from_static(b"ping"),
            from: crate::Origin {
                reducer: crate::ReducerId::of(b"caller"),
                host: crate::HostId::of(b"node"),
            },
            continuation_token: Bytes::from_static(b"tok"),
        };

        // Sanity: one fold on a fresh instance succeeds (one echoed request, Continue) before we time it.
        let (reqs, outcome) = reducer
            .on_message(base.clone())
            .await
            .expect("the fold succeeds");
        assert_eq!(reqs.len(), 1, "echo guest emits exactly one request");
        assert_eq!(outcome, crate::Outcome::Continue);
        drop(reducer);

        // Production drives ONE fold per instance: the gateway instantiates the reducer per request, and the
        // composed value-heap runtime is not reset between folds — so a reused instance accumulates heap
        // allocations and eventually traps (`realloc: beyond end of memory`). The honest "cost to run a single
        // reducer function" is therefore the FIRST fold on a freshly instantiated component. We re-instantiate
        // per sample and time ONLY `on_message` (the spawn/instantiate is untimed — that is the separately
        // measured init cost, not the operator's execution question here).

        // Warm-up on fresh instances (settle caches / cranelift code / pooling slabs) — discarded.
        for _ in 0..1_000 {
            let mut r = store
                .spawn(program, ord(b"bench-reducer"))
                .await
                .expect("warm-up spawn");
            let _ = r.on_message(base.clone()).await.expect("warm-up fold");
        }

        const N: usize = 5_000;

        // Attribution part A — the host-side ENCODE (`message_to_wit`) in isolation, so we can attribute the
        // full-fold cost between encode and the wasmtime call+decode.
        let encode_start = Instant::now();
        for _ in 0..N {
            let wit = super::message_to_wit(&base);
            std::hint::black_box(&wit);
        }
        let encode_total = encode_start.elapsed();

        // The fold itself: fresh instance per sample, time ONLY `on_message` (encode + wasmtime call + decode).
        let mut samples: Vec<Duration> = Vec::with_capacity(N);
        for _ in 0..N {
            let mut r = store
                .spawn(program, ord(b"bench-reducer"))
                .await
                .expect("spawn a fresh instance");
            let msg = base.clone();
            let t = Instant::now();
            let out = r.on_message(msg).await.expect("timed fold");
            samples.push(t.elapsed());
            std::hint::black_box(&out);
            drop(r);
        }

        // Discriminator: cold (first) fold on a FRESH instance vs warm (subsequent) folds on the SAME instance.
        // If the warm fold is far cheaper, the fresh-instance cost is first-touch overhead (page-faulting the
        // pooling memory slab / first value-heap use), not steady-state fold compute — which changes the lever.
        // Bounded folds per instance (the composed value-heap is not reset between folds and eventually traps).
        const INSTANCES: usize = 500;
        const FOLDS_PER: usize = 6;
        let mut cold_first: Vec<Duration> = Vec::with_capacity(INSTANCES);
        let mut warm_rest: Vec<Duration> = Vec::with_capacity(INSTANCES * (FOLDS_PER - 1));
        for _ in 0..INSTANCES {
            let mut r = store
                .spawn(program, ord(b"bench-reducer"))
                .await
                .expect("spawn a fresh instance");
            for f in 0..FOLDS_PER {
                let msg = base.clone();
                let t = Instant::now();
                match r.on_message(msg).await {
                    Ok(out) => {
                        let dt = t.elapsed();
                        std::hint::black_box(&out);
                        if f == 0 {
                            cold_first.push(dt)
                        } else {
                            warm_rest.push(dt)
                        }
                    }
                    // A later fold may trap once the un-reset heap fills — stop folding this instance.
                    Err(_) => break,
                }
            }
        }
        let med = |v: &mut Vec<Duration>| {
            v.sort_unstable();
            v.get(v.len() / 2).copied().unwrap_or_default()
        };
        let cold_med = med(&mut cold_first);
        let warm_med = med(&mut warm_rest);

        // Lever discriminator: does per-fold cost scale with payload SIZE? O(bytes) ⇒ the message's byte-lists
        // are copied across the component boundary with per-element overhead (lever: bulk copy). O(1) ⇒ a fixed
        // per-call boundary cost independent of size (lever: cut per-fold boundary crossings). Fresh instance
        // per fold (production per-request model); small sample per size.
        let mut size_rows: Vec<(usize, Duration, usize)> = Vec::new();
        for &sz in &[4usize, 256, 4096, 65536] {
            let payload = Bytes::from(vec![0x5au8; sz]);
            let m = crate::Message {
                id: crate::ContractId::of(b"echo-contract"),
                payload,
                from: crate::Origin {
                    reducer: crate::ReducerId::of(b"caller"),
                    host: crate::HostId::of(b"node"),
                },
                continuation_token: Bytes::from_static(b"tok"),
            };
            let mut v: Vec<Duration> = Vec::with_capacity(400);
            for _ in 0..400 {
                let mut r = store
                    .spawn(program, ord(b"bench-reducer"))
                    .await
                    .expect("spawn a fresh instance");
                let msg = m.clone();
                let t = Instant::now();
                match r.on_message(msg).await {
                    Ok(out) => {
                        v.push(t.elapsed());
                        std::hint::black_box(&out);
                    }
                    Err(_) => break,
                }
            }
            let n = v.len();
            size_rows.push((sz, med(&mut v), n));
        }

        // LIFT-vs-LOWER attribution sweep. The echo fold has TWO size-scaling per-byte value-heap loops in one
        // `on_message`: (i) the incoming payload LIFT (host list<u8> → heap Bytes) and (ii) the outgoing request
        // payload LOWER (heap Bytes → host list<u8>, nested in the returned Step record). The seq-916 bulk-copy
        // lever lands in two slices — the LIFT first (`bytes-new` on the record-param), the nested-Bytes LOWER
        // second (`bytes-read` in the SpillRecord canon-writer) — so the acceptance number should HALVE, then
        // halve again. To verify WHICH loop collapsed (not just the net), sweep the echo's `on_notification`:
        // it is INERT (returns `requests = []`), so it drives the SAME incoming-payload lift with NO outgoing
        // payload. If its slope tracks the `on_message` slope's incoming half, notification-slope ≈ the LIFT
        // per-byte cost and (message − notification) ≈ the outgoing LOWER cost — clean per-loop attribution.
        // (If the guest instead dead-code-eliminates the unused notification payload, the notification slope
        // reads ≈ 0 — itself a useful datum: the lift is not exercised when the value is unused.)
        let mut note_size_rows: Vec<(usize, Duration, usize)> = Vec::new();
        for &sz in &[4usize, 256, 4096, 65536] {
            let payload = Bytes::from(vec![0x5au8; sz]);
            let note = crate::Notification {
                id: crate::ContractId::of(b"echo-contract"),
                payload,
            };
            let mut v: Vec<Duration> = Vec::with_capacity(400);
            for _ in 0..400 {
                let mut r = store
                    .spawn(program, ord(b"bench-reducer"))
                    .await
                    .expect("spawn a fresh instance");
                let n = note.clone();
                let t = Instant::now();
                match r.on_notification(n).await {
                    Ok(out) => {
                        v.push(t.elapsed());
                        std::hint::black_box(&out);
                    }
                    Err(_) => break,
                }
            }
            let n = v.len();
            note_size_rows.push((sz, med(&mut v), n));
        }

        samples.sort_unstable();
        let pct = |p: f64| samples[((p * (N as f64 - 1.0)).round() as usize).min(N - 1)];
        let sum: Duration = samples.iter().sum();
        let mean = sum / N as u32;
        let encode_mean = encode_total / N as u32;
        // Full-fold mean minus the isolated encode ≈ the wasmtime call (lift/lower + guest execute) + decode.
        let call_decode_mean = mean.saturating_sub(encode_mean);

        let us = |d: Duration| d.as_secs_f64() * 1e6;
        eprintln!(
            "── per-fold reducer EXECUTION cost (N={N}, reducer-echo, fresh instance/fold, warm engine) ──"
        );
        eprintln!(
            "  fold (on_message)  mean={:.2}µs  p50={:.2}µs  p90={:.2}µs  p99={:.2}µs  min={:.2}µs  max={:.2}µs",
            us(mean),
            us(pct(0.50)),
            us(pct(0.90)),
            us(pct(0.99)),
            us(samples[0]),
            us(samples[N - 1])
        );
        eprintln!(
            "  attribution: encode(message_to_wit)={:.2}µs  call+decode(wasmtime lift/lower+guest+decode)={:.2}µs",
            us(encode_mean),
            us(call_decode_mean)
        );
        eprintln!(
            "  cold-vs-warm: cold(fresh-instance fold #1) median={:.2}µs  warm(same-instance fold #2..{})  median={:.2}µs  ⇒ fresh-instance overhead≈{:.2}µs",
            us(cold_med),
            FOLDS_PER,
            us(warm_med),
            us(cold_med.saturating_sub(warm_med))
        );
        for (sz, m, n) in &size_rows {
            if *n == 0 {
                eprintln!(
                    "  payload-size sweep: {sz:>6} B payload ⇒ TRAPPED (un-reset heap exhausted on the first fold)"
                );
            } else {
                eprintln!(
                    "  payload-size sweep: {:>6} B payload ⇒ fold median={:.2}µs (≈{:.2}µs/byte over the 4 B base)",
                    sz,
                    us(*m),
                    if *sz > 4 {
                        (us(*m) - us(size_rows[0].1)) / (*sz as f64 - 4.0)
                    } else {
                        0.0
                    }
                );
            }
        }
        for (sz, m, n) in &note_size_rows {
            if *n == 0 {
                eprintln!(
                    "  LIFT-only sweep (on_notification, inert): {sz:>6} B payload ⇒ TRAPPED"
                );
            } else {
                eprintln!(
                    "  LIFT-only sweep (on_notification, inert): {:>6} B payload ⇒ fold median={:.2}µs (≈{:.2}µs/byte over the 4 B base)",
                    sz,
                    us(*m),
                    if *sz > 4 {
                        (us(*m) - us(note_size_rows[0].1)) / (*sz as f64 - 4.0)
                    } else {
                        0.0
                    }
                );
            }
        }
        // Per-loop attribution: the top-vs-base per-byte slope of each sweep. on_message = LIFT+LOWER,
        // on_notification = LIFT only ⇒ (message − notification) ≈ the outgoing-LOWER per-byte cost.
        let slope = |rows: &[(usize, Duration, usize)]| -> Option<f64> {
            let base = rows.iter().find(|(_, _, n)| *n > 0)?;
            let top = rows.iter().rev().find(|(_, _, n)| *n > 0)?;
            (top.0 > base.0).then(|| (us(top.1) - us(base.1)) / (top.0 as f64 - base.0 as f64))
        };
        match (slope(&size_rows), slope(&note_size_rows)) {
            (Some(msg), Some(note)) => eprintln!(
                "  LIFT-vs-LOWER attribution: on_message={msg:.4}µs/B (LIFT+LOWER)  on_notification={note:.4}µs/B (LIFT)  ⇒ incoming-LIFT≈{note:.4}µs/B  outgoing-LOWER≈{:.4}µs/B",
                (msg - note).max(0.0)
            ),
            _ => eprintln!(
                "  LIFT-vs-LOWER attribution: unavailable (a sweep trapped before spanning 4 B..64 KiB)"
            ),
        }
        eprintln!(
            "  operator bar: {} the ~200µs bar (p50={:.2}µs)",
            if us(pct(0.50)) < 200.0 {
                "CLEARS"
            } else {
                "OVER"
            },
            us(pct(0.50))
        );

        // The measurement asserts nothing about the absolute number (that is the operator's bar, reported
        // above) — only that the fold path stayed sane (finite, non-zero) so a broken run is not read as a win.
        assert!(
            us(mean) > 0.0 && us(mean) < 1e6,
            "per-fold mean is implausible: {:.2}µs",
            us(mean)
        );
    }

    /// Shared setup for the env-gated reducer-echo benches: seed the guest + its whole component-store closure
    /// (heap-runtime + nfc + …) into a fresh CAS, spawn ONE instance, and hand it back with a minimal
    /// fold-clean message. `None` (env unset / closure mismatch) so a caller early-returns. See
    /// [`warm_per_fold_execution_cost_of_a_single_reducer_function`] for the env vars + how to obtain the
    /// fixtures (nix reducer-echo component + a cdz-component-store dir).
    async fn seed_and_spawn_reducer_echo() -> Option<(Box<dyn crate::Reducer>, crate::Message)> {
        let path = std::env::var("CDZ_REDUCER_ECHO_WASM").ok()?;
        let bytes = std::fs::read(&path).ok()?;
        let cas = InMemoryBlobStore::new();
        cas.put(Bytes::from(bytes.clone())).await.unwrap();
        if let Ok(dir) = std::env::var("CDZ_COMPONENT_STORE_DIR") {
            for entry in std::fs::read_dir(&dir).expect("read component-store dir") {
                let p = entry.expect("dir entry").path();
                if p.extension().and_then(|e| e.to_str()) == Some("wasm") {
                    cas.put(Bytes::from(std::fs::read(&p).unwrap()))
                        .await
                        .unwrap();
                }
            }
        }
        let program = ProgramHash::of(&bytes);
        let store = wasm_program_store(Arc::new(cas));
        let reducer = store.spawn(program, ord(b"reclaim-witness")).await?;
        let base = crate::Message {
            id: crate::ContractId::of(b"echo-contract"),
            payload: Bytes::from_static(b"ping"),
            from: crate::Origin {
                reducer: crate::ReducerId::of(b"caller"),
                host: crate::HostId::of(b"node"),
            },
            continuation_token: Bytes::from_static(b"tok"),
        };
        Some((reducer, base))
    }

    // Reclaim regression gate (seq-916, now LIVE-GREEN): a reducer-export `on_message` fold used to leak a FIXED
    // set of value-heap SHELLS per fold — the decoded incoming-msg envelope shells AND the constructed step
    // shells (rc-trace: 9 un-dropped nodes, ZERO drops, payload-independent), so on a REUSED instance the shells
    // accumulated until wasm `memory.grow` could no longer satisfy a realloc and the fold trapped (~2000 folds at
    // the default 256 MiB ceiling). The reducer-export shell-drop reclaim landed (envelope-decode site-a #9061 +
    // outgoing forward-dup site-b: v-core-opt binder_is_param gate #9128 + escape-query #9135), so a reused
    // instance now folds the full cap with FLAT value-heap memory and NO trap. This drives one reused instance
    // and GATES that: it survives the cap AND never traps — a re-emergence of the per-fold shell leak would
    // re-accumulate and trap this RED. Env-gated like the sibling bench: SKIPS cleanly when the fixtures are
    // unset.
    #[tokio::test]
    async fn a_reused_reducer_instance_folds_the_cap_without_accumulating_shells() {
        let Some((mut reducer, base)) = seed_and_spawn_reducer_echo().await else {
            eprintln!(
                "CDZ_REDUCER_ECHO_WASM/CDZ_COMPONENT_STORE_DIR unset or closure mismatch — skipping the reclaim witness"
            );
            return;
        };

        // Above the ~2000-fold trap point the leak used to hit at the default 256 MiB ceiling, so a regression
        // that re-accumulates shells traps well within the cap. Post-reclaim this folds the full cap (~cap×fold).
        const CAP: usize = 20_000;
        let mut survived = 0usize;
        let mut trap: Option<String> = None;
        for _ in 0..CAP {
            match reducer.on_message(base.clone()).await {
                Ok(_) => survived += 1,
                Err(e) => {
                    trap = Some(format!("{e:?}"));
                    break;
                }
            }
        }
        match &trap {
            Some(e) => eprintln!(
                "reclaim gate: reused instance TRAPPED after {survived} fold(s) — per-fold shell leak REGRESSED: {e}"
            ),
            None => eprintln!(
                "reclaim gate: reused instance folded all {CAP} without trapping (reclaim net-0, flat value-heap)"
            ),
        }
        // The reclaim is landed (site-a #9061 + site-b #9128/#9135), so the reused instance must fold the whole
        // cap and never trap. `survived > 0` also catches a broken fixture (zero folds with no trap = setup
        // error, not a reclaim result); `trap.is_none()` is the regression gate proper — a re-emergent per-fold
        // shell leak re-accumulates and traps within the cap.
        assert!(
            survived > 0,
            "no fold ran — fixture/closure setup error, not a reclaim result"
        );
        assert!(
            trap.is_none(),
            "REGRESSION: reused instance TRAPPED after {survived}/{CAP} folds — the reducer-export shell-drop reclaim (envelope-decode site-a #9061 / outgoing forward-dup site-b #9128/#9135) regressed and per-fold value-heap shells are accumulating again: {trap:?}"
        );
    }

    // The runtime-override seam (`WasmProgramStore::with_component_override`): substituting a component for a
    // dependency HASH lets instantiation compose a runtime the guest did NOT record — the foundation for the
    // planned host-run-nets-live-objects-0 gate (swap in the debug-counters value-heap runtime, whose content
    // hash differs from the shipped runtime's, under a guest's shipped-hash import to census live-objects
    // without recompiling the guest). This proves the mechanism WITHOUT needing the debug runtime: it supplies
    // the (release) heap runtime that is deliberately absent from the CAS, purely via the override.
    #[tokio::test]
    #[ignore = "env-gated seam test; needs CDZ_REDUCER_ECHO_WASM + CDZ_COMPONENT_STORE_DIR"]
    async fn with_component_override_composes_a_runtime_the_guest_did_not_record() {
        let Ok(path) = std::env::var("CDZ_REDUCER_ECHO_WASM") else {
            eprintln!("CDZ_REDUCER_ECHO_WASM unset — skipping the override seam test");
            return;
        };
        let Ok(dir) = std::env::var("CDZ_COMPONENT_STORE_DIR") else {
            eprintln!("CDZ_COMPONENT_STORE_DIR unset — skipping the override seam test");
            return;
        };
        let guest = std::fs::read(&path).expect("read the reducer-echo component");

        // The guest's value-heap runtime dependency (its `cadenza:runtime/heap@…+<hash>` import).
        let engine = super::reducer_engine(&super::ResourceLimits::default()).expect("engine");
        let component =
            wasmtime::component::Component::from_binary(&engine, &guest).expect("parse component");
        let deps = super::component_dependencies(&engine, &component);
        let heap = deps
            .iter()
            .find(|d| d.import_name.contains("cadenza:runtime/heap"))
            .expect("the reducer-echo guest imports the value-heap runtime");
        let heap_hash = heap.hash;

        // The heap component's bytes (the dir file named by that content hash) — used as the OVERRIDE, and
        // deliberately NOT seeded into the CAS so a plain spawn cannot resolve it.
        let heap_file = std::path::Path::new(&dir).join(format!("{heap_hash}.wasm"));
        let heap_bytes =
            std::fs::read(&heap_file).expect("the heap component file in the component-store dir");

        // Seed the guest + every OTHER component (nfc, …) but NOT the heap.
        let cas: Arc<dyn BlobStore> = {
            let cas = InMemoryBlobStore::new();
            cas.put(Bytes::from(guest.clone())).await.unwrap();
            for entry in std::fs::read_dir(&dir).expect("read component-store dir") {
                let p = entry.expect("dir entry").path();
                if p.extension().and_then(|e| e.to_str()) == Some("wasm") && p != heap_file {
                    cas.put(Bytes::from(std::fs::read(&p).unwrap()))
                        .await
                        .unwrap();
                }
            }
            Arc::new(cas)
        };
        let program = ProgramHash::of(&guest);

        // Control: with the heap absent from the CAS and NO override, the compose path can't resolve the
        // runtime dependency, so spawn declines.
        assert!(
            wasm_program_store(Arc::clone(&cas))
                .spawn(program, ord(b"seam"))
                .await
                .is_none(),
            "control: heap absent from CAS + no override ⇒ spawn must decline"
        );

        // With the override: the same absent hash resolves to the supplied bytes, so the guest composes and
        // spawns — proving the override bypasses content-addressing to supply a runtime the CAS lacks — and
        // the composed runtime is functional (a real fold succeeds).
        let store = wasm_program_store(Arc::clone(&cas))
            .with_component_override(heap_hash, Bytes::from(heap_bytes));
        let mut reducer = store
            .spawn(program, ord(b"seam"))
            .await
            .expect("override supplies the heap runtime ⇒ spawn succeeds");
        let (reqs, outcome) = reducer
            .on_message(crate::Message {
                id: crate::ContractId::of(b"echo-contract"),
                payload: Bytes::from_static(b"ping"),
                from: crate::Origin {
                    reducer: crate::ReducerId::of(b"caller"),
                    host: crate::HostId::of(b"node"),
                },
                continuation_token: Bytes::from_static(b"tok"),
            })
            .await
            .expect("a fold on the override-composed runtime succeeds");
        assert_eq!(reqs.len(), 1, "echo guest emits exactly one request");
        assert_eq!(outcome, crate::Outcome::Continue);
    }

    // The host-run-nets-live-objects-0 GATE (operator seq-916 host-owns-and-frees assignment): a leak-clean
    // reducer fold must return the composed value-heap's live-cell count to its PRE-FOLD baseline (v-runtime's
    // trusted-signal invariant — immortals are excluded from the count, so net-0 is achievable; delta-per-fold
    // is the robust form even if a stateful reducer has a nonzero steady baseline). Composes the DEBUG-COUNTERS
    // runtime via the override seam (the shipped runtime's live-objects is always 0) and reads it through the
    // retained heap instance ([`WasmReducer::live_object_census`]). This censuses the REAL platform fold path
    // (spawn → on_message through WasmReducer), so it also catches a future Mode-2 host-held value-heap handle
    // that a host forgot to free.
    //
    // NET-0 ACHIEVED (assertion now PASSES): a fold returns the composed value-heap to its pre-fold baseline on
    // all three entry points. The reducer-export shell-drop landed in stages: site-a (#9061, the envelope-decode
    // drop) took the inert paths on_notification/on_response to 0, and site-b (v-core-opt's binder_is_param
    // parent-dup gate #9128 + v-cdz-wasm-codegen's dup_sites-conditional projection-borrow escape-query #9135)
    // took on_message 7 → 0 by removing the surplus parent dups on the forwarded wrapper-cell payload (one child
    // dup per field, no parent dups). Confirmed on landed main: on_message 0, on_notification 0, on_response 0,
    // delta-per-fold 0 across 5 folds (history: 13/6/8 on 2026-09-15 → 7/3/5 after the envelope reclaim → 0/0/0).
    // (An earlier site-b, #9101, was reverted #9104: an ALWAYS-borrow reclassification under-retained + trapped a
    // partition fold (UAF); the landed fix is per-site — borrow only where dup_sites says so — with corpus pins
    // #9125/#9130 guarding the escaping+consumed partition-fold shape.)
    // This is a LIVE regression gate now (no longer fails-by-design): a re-emergence of the envelope OR the
    // outgoing forward-dup leak makes a fold net nonzero and this test RED. It also catches a future Mode-2
    // host-held value-heap handle a host forgot to free.
    //
    // Censuses ALL THREE fold entry points (on_message on a reused instance for the accumulation signal;
    // on_notification + on_response each on a fresh instance). The two inert paths (requests=[]) isolate the
    // ENVELOPE-DECODE reclaim from on_message's envelope+step reclaim, so the gate catches a regression scoped
    // to on_message alone — one that would leave on_notification / on_response silently leaking their envelope
    // shells — not just the on_message path.
    //
    // Env-gated (needs the reducer-echo guest + component-store closure + the debug-counters runtime): it SKIPS
    // cleanly when CDZ_REDUCER_ECHO_WASM / CDZ_COMPONENT_STORE_DIR / CDZ_DEBUG_RUNTIME_WASM are unset, so it is
    // safe in the routine gate (which does not supply them) and GATES net-0 wherever the fixtures are provided.
    #[tokio::test]
    async fn a_reducer_fold_nets_live_objects_to_its_pre_fold_baseline() {
        let (Ok(path), Ok(dir), Ok(dbg)) = (
            std::env::var("CDZ_REDUCER_ECHO_WASM"),
            std::env::var("CDZ_COMPONENT_STORE_DIR"),
            std::env::var("CDZ_DEBUG_RUNTIME_WASM"),
        ) else {
            eprintln!(
                "census gate env unset (need CDZ_REDUCER_ECHO_WASM + CDZ_COMPONENT_STORE_DIR + CDZ_DEBUG_RUNTIME_WASM) — skipping"
            );
            return;
        };
        let guest = std::fs::read(&path).expect("read the reducer-echo component");
        let debug_heap = std::fs::read(&dbg).expect("read the debug-counters runtime component");

        // The guest's value-heap runtime dependency hash — overridden to the debug-counters build so
        // live-objects is a real census (the shipped build reports 0). Its nfc sub-dep (same hash as release)
        // resolves from the seeded component-store dir.
        let engine = super::reducer_engine(&super::ResourceLimits::default()).expect("engine");
        let component =
            wasmtime::component::Component::from_binary(&engine, &guest).expect("parse component");
        let heap_hash = super::component_dependencies(&engine, &component)
            .into_iter()
            .find(|d| d.import_name.contains("cadenza:runtime/heap"))
            .expect("the reducer-echo guest imports the value-heap runtime")
            .hash;

        let cas = InMemoryBlobStore::new();
        cas.put(Bytes::from(guest.clone())).await.unwrap();
        for entry in std::fs::read_dir(&dir).expect("read component-store dir") {
            let p = entry.expect("dir entry").path();
            if p.extension().and_then(|e| e.to_str()) == Some("wasm") {
                cas.put(Bytes::from(std::fs::read(&p).unwrap()))
                    .await
                    .unwrap();
            }
        }
        let program = ProgramHash::of(&guest);
        let store = wasm_program_store(Arc::new(cas))
            .with_component_override(heap_hash, Bytes::from(debug_heap));
        let mut reducer = store
            .spawn(program, ord(b"census"))
            .await
            .expect("spawn the guest composed against the debug-counters runtime");

        let base = crate::Message {
            id: crate::ContractId::of(b"echo-contract"),
            payload: Bytes::from_static(b"ping"),
            from: crate::Origin {
                reducer: crate::ReducerId::of(b"caller"),
                host: crate::HostId::of(b"node"),
            },
            continuation_token: Bytes::from_static(b"tok"),
        };

        // Pre-fold baseline on the fresh instance (immortals excluded ⇒ typically 0 for the stateless echo).
        let baseline = reducer.live_object_census().await.expect(
            "the debug-counters runtime exposes live-objects (is CDZ_DEBUG_RUNTIME_WASM the debug build?)",
        );
        eprintln!("census gate: pre-fold baseline live-objects = {baseline}");

        // After each fold the count returns to baseline (net-0 per fold). A few folds suffice to detect a
        // per-fold delta; a regressed reclaim would show a positive drift that compounds every fold.
        const FOLDS: usize = 5;
        let mut last = baseline;
        let mut leaked = false;
        for i in 1..=FOLDS {
            let _ = reducer
                .on_message(base.clone())
                .await
                .expect("fold succeeds");
            last = reducer.live_object_census().await.expect("census reads");
            if last != baseline {
                leaked = true;
            }
            eprintln!(
                "census gate: after fold {i}: live-objects = {last} (delta from baseline = {})",
                last as i64 - baseline as i64
            );
        }
        let per_fold = (last as i64 - baseline as i64) / FOLDS as i64;
        eprintln!(
            "census gate: on_message per-fold net ≈ {per_fold} value-heap cell(s) (envelope+step; net-0 as of site-a #9061 + site-b #9128/#9135 — nonzero here is a regression)"
        );

        // Payload-SIZE independence of net-0. The reclaim SHELLS are structural (envelope + step) and
        // payload-independent, but the payload BYTES cross the boundary via the bulk-copy path (bytes-new on
        // lift / bytes-read on lower, #9058) — a DISTINCT reclaim from the shells. A regression that leaked a
        // large payload's bulk-copy handle (e.g. a bytes-new backing buffer not dropped) would be payload-size
        // DEPENDENT and invisible to the fixed 4-byte "ping" folds above. Fold the same reused instance across a
        // size sweep and require net-0 after each — this pins the bulk-bytes marshaling reclaim across sizes,
        // guarding the #9058 lever from a large-payload leak the shell census would miss.
        let mut sweep_max_delta: i64 = 0;
        for payload_bytes in [0usize, 256, 4096] {
            let sized = crate::Message {
                payload: Bytes::from(vec![0x61u8; payload_bytes]),
                ..base.clone()
            };
            let _ = reducer
                .on_message(sized)
                .await
                .expect("sized-payload fold succeeds");
            let after = reducer.live_object_census().await.expect("census reads");
            sweep_max_delta = sweep_max_delta.max(after as i64 - baseline as i64);
            eprintln!(
                "census gate: after {payload_bytes}-byte-payload fold: live-objects delta from baseline = {}",
                after as i64 - baseline as i64
            );
        }
        assert!(
            sweep_max_delta == 0,
            "REGRESSION: a reducer fold leaked value-heap cells for a non-trivial payload (max delta {sweep_max_delta} across 0/256/4096-byte payloads while the 4-byte folds netted 0) — the bulk-bytes marshaling reclaim (bytes-new/bytes-read, #9058) is payload-size-DEPENDENT; a large-payload bulk-copy handle is not being dropped"
        );

        // Per-ENTRY-POINT breakdown (seq-916 shell-drop fix SCOPE). The echo's on_response / on_notification are
        // INERT (return requests=[]): a fold of either DECODES the incoming envelope but emits NO outgoing
        // step/request shells. Censusing each on a FRESH instance isolates the ENVELOPE-DECODE shell leak from
        // on_message's envelope+step leak. If these inert paths also net nonzero, the shell-drop reclaim must
        // live on the SHARED envelope-decode emit — a fix scoped to on_message alone would leave on_response /
        // on_notification leaking, and this gate (which now folds all three into `leaked`) catches that.
        let census_fresh_fold = |label: &'static str| async {
            let mut r = store
                .spawn(program, ord(label.as_bytes()))
                .await
                .expect("spawn a fresh instance for the entry-point census");
            let base = r.live_object_census().await.expect("census reads");
            (r, base)
        };

        let (mut note_reducer, note_base) = census_fresh_fold("census-note").await;
        let _ = note_reducer
            .on_notification(crate::Notification {
                id: crate::ContractId::of(b"echo-contract"),
                payload: Bytes::from_static(b"ping"),
            })
            .await
            .expect("notification fold succeeds");
        let note_delta = note_reducer
            .live_object_census()
            .await
            .expect("census reads") as i64
            - note_base as i64;

        let (mut resp_reducer, resp_base) = census_fresh_fold("census-resp").await;
        let _ = resp_reducer
            .on_response(crate::Response {
                id: crate::ContractId::of(b"echo-contract"),
                continuation_token: Bytes::from_static(b"tok"),
                payload: Ok(Bytes::from_static(b"pong")),
            })
            .await
            .expect("response fold succeeds");
        let resp_delta = resp_reducer
            .live_object_census()
            .await
            .expect("census reads") as i64
            - resp_base as i64;

        if note_delta != 0 || resp_delta != 0 {
            leaked = true;
        }
        eprintln!(
            "census gate: per-entry-point shell attribution — on_message≈{per_fold}/fold (envelope+step), on_notification={note_delta}, on_response={resp_delta} (both inert ⇒ envelope-only) ⇒ step/request-shell portion ≈ {} cell(s)",
            per_fold - note_delta
        );

        // The trusted-signal invariant: a fold nets the composed value-heap to its pre-fold baseline on all three
        // entry points. Now PASSES (net-0 landed); a nonzero delta means a shell-drop regressed.
        assert!(
            !leaked,
            "REGRESSION: a reducer fold leaked value-heap cells (on_message≈{per_fold}/fold, on_notification={note_delta}, on_response={resp_delta}) — the reducer-export shell-drop reclaim (envelope-decode site-a #9061 / outgoing forward-dup site-b #9128/#9135) regressed; census not net-0"
        );
    }

    // The O(n) borrow-thread-accumulator reclaim tripwire (v-core-opt co-verify for the back-edge/TCO
    // borrow-dup family). The accum guest's `on_message` runs a bounded loop that BORROWS an owned List
    // (`List.len`) while THREADING it (`List.push`) — the single-use borrowed-length inlines into the back-edge
    // call arg, co-locating acc's borrow with its consume, which is the per-iteration borrow-forced dup the
    // `drop_old_borrowed` reclaim (#9172, trunk e8808d610e) fixes. Pre-fix each fold leaked ~O(loop) value-heap
    // cells, so a REUSED instance grew MONOTONICALLY across folds; post-fix a fold nets to baseline. This drives
    // one reused instance across N folds and GATES that the census stays FLAT (delta 0 after every fold) — a
    // re-emergence of the borrow-thread dup shows monotonic per-fold growth and REDs this.
    //
    // Distinct from `a_reducer_fold_nets_live_objects_to_its_pre_fold_baseline`: that guards the envelope +
    // outgoing-forward-dup SHELLS of a straight echo forward (a constant per-fold residue if it regresses);
    // THIS exercises a LOOP BACK-EDGE end-to-end through the reducer forward path, so it is the independent
    // platform-path SCALING confirmation (net-0 vs O(n)) that a single-fold delta cannot make — complementing
    // v-core-opt's corpus 09-functions:608 and v-memory-safety's rc-gate. Env-gated on CDZ_REDUCER_ECHO_ACCUM_WASM
    // + CDZ_COMPONENT_STORE_DIR + CDZ_DEBUG_RUNTIME_WASM; SKIPS cleanly when unset (safe in the routine gate).
    #[tokio::test]
    async fn a_reused_reducer_folding_an_accumulator_loop_guest_stays_net_zero() {
        let (Ok(path), Ok(dir), Ok(dbg)) = (
            std::env::var("CDZ_REDUCER_ECHO_ACCUM_WASM"),
            std::env::var("CDZ_COMPONENT_STORE_DIR"),
            std::env::var("CDZ_DEBUG_RUNTIME_WASM"),
        ) else {
            eprintln!(
                "accum census env unset (need CDZ_REDUCER_ECHO_ACCUM_WASM + CDZ_COMPONENT_STORE_DIR + CDZ_DEBUG_RUNTIME_WASM) — skipping"
            );
            return;
        };
        let guest = std::fs::read(&path).expect("read the accumulator-loop reducer component");
        let debug_heap = std::fs::read(&dbg).expect("read the debug-counters runtime component");

        // Compose the guest against the debug-counters value-heap runtime (its live-objects is a real census;
        // the shipped build reports 0), resolving the rest of its closure from the seeded component-store dir.
        let engine = super::reducer_engine(&super::ResourceLimits::default()).expect("engine");
        let component =
            wasmtime::component::Component::from_binary(&engine, &guest).expect("parse component");
        let heap_hash = super::component_dependencies(&engine, &component)
            .into_iter()
            .find(|d| d.import_name.contains("cadenza:runtime/heap"))
            .expect("the accumulator guest imports the value-heap runtime")
            .hash;

        let cas = InMemoryBlobStore::new();
        cas.put(Bytes::from(guest.clone())).await.unwrap();
        for entry in std::fs::read_dir(&dir).expect("read component-store dir") {
            let p = entry.expect("dir entry").path();
            if p.extension().and_then(|e| e.to_str()) == Some("wasm") {
                cas.put(Bytes::from(std::fs::read(&p).unwrap()))
                    .await
                    .unwrap();
            }
        }
        let program = ProgramHash::of(&guest);
        let store = wasm_program_store(Arc::new(cas))
            .with_component_override(heap_hash, Bytes::from(debug_heap));
        let mut reducer = store
            .spawn(program, ord(b"accum-census"))
            .await
            .expect("spawn the accumulator guest composed against the debug-counters runtime");

        let base = crate::Message {
            id: crate::ContractId::of(b"echo-contract"),
            payload: Bytes::from_static(b"ping"),
            from: crate::Origin {
                reducer: crate::ReducerId::of(b"caller"),
                host: crate::HostId::of(b"node"),
            },
            continuation_token: Bytes::from_static(b"tok"),
        };

        let baseline = reducer.live_object_census().await.expect(
            "the debug-counters runtime exposes live-objects (is CDZ_DEBUG_RUNTIME_WASM the debug build?)",
        );
        eprintln!("accum census: pre-fold baseline live-objects = {baseline}");

        // N folds on a REUSED instance. The loop's per-iteration borrow-forced dup (pre-#9172) accumulates
        // ~O(loop) cells PER FOLD, so a regression shows MONOTONIC growth over N; post-fix each fold nets to
        // baseline (flat). N=100 makes O(n) growth unmistakable against net-0.
        const FOLDS: usize = 100;
        let mut last = baseline;
        let mut max_delta: i64 = 0;
        for _ in 1..=FOLDS {
            let _ = reducer
                .on_message(base.clone())
                .await
                .expect("accumulator fold succeeds");
            last = reducer.live_object_census().await.expect("census reads");
            max_delta = max_delta.max(last as i64 - baseline as i64);
        }
        let total_delta = last as i64 - baseline as i64;
        let per_fold = total_delta / FOLDS as i64;
        eprintln!(
            "accum census: after {FOLDS} folds live-objects delta from baseline = {total_delta} (per-fold ≈ {per_fold}, max {max_delta}; net-0 as of the drop_old_borrowed reclaim #9172 — monotonic growth here is a regression)"
        );

        assert!(
            total_delta == 0,
            "REGRESSION: a reused reducer folding the accumulator-loop guest grew the value-heap by {total_delta} cell(s) over {FOLDS} folds (per-fold ≈ {per_fold}, max {max_delta}) — the O(n) per-iteration borrow-forced-dup reclaim (drop_old_borrowed #9172) regressed; the borrowed-then-threaded owned accumulator is leaking again"
        );
    }

    // ATTRIBUTION harness (v-memory-safety, site-b node-kind rc-trace for v-cdz-wasm-codegen): drive ONE
    // on_message fold on reducer-echo composed against the RC-TRACE runtime (.#rctrace-runtime), drain the
    // per-node ALLOC/DUP/DROP trace, and print the LEAKED node KINDS + residual RCs. Disambiguates the
    // on_message residual (~7): a payload/leaf leaking RC>1 = a forwarded-dup imbalance (site-a interaction);
    // RC==1 unreached = an op_drop-doesn't-cascade-lists gap; the step-record TOP leaking = an
    // emit_result_spill gap. `#[ignore]` + env-gated (needs CDZ_RCTRACE_RUNTIME_WASM, the debug-trace twin of
    // the runtime the guest imports, + the reducer-echo guest + component store).
    #[tokio::test]
    #[ignore = "env-gated rc-trace attribution; needs CDZ_REDUCER_ECHO_WASM + CDZ_COMPONENT_STORE_DIR + CDZ_RCTRACE_RUNTIME_WASM"]
    async fn on_message_rc_trace_node_kinds() {
        let (Ok(path), Ok(dir), Ok(rct)) = (
            std::env::var("CDZ_REDUCER_ECHO_WASM"),
            std::env::var("CDZ_COMPONENT_STORE_DIR"),
            std::env::var("CDZ_RCTRACE_RUNTIME_WASM"),
        ) else {
            eprintln!("rc-trace attribution env unset — skipping");
            return;
        };
        let guest = std::fs::read(&path).expect("read the reducer-echo component");
        let rctrace_heap = std::fs::read(&rct).expect("read the rc-trace runtime component");

        let engine = super::reducer_engine(&super::ResourceLimits::default()).expect("engine");
        let component =
            wasmtime::component::Component::from_binary(&engine, &guest).expect("parse component");
        let heap_hash = super::component_dependencies(&engine, &component)
            .into_iter()
            .find(|d| d.import_name.contains("cadenza:runtime/heap"))
            .expect("the reducer-echo guest imports the value-heap runtime")
            .hash;

        let cas = InMemoryBlobStore::new();
        cas.put(Bytes::from(guest.clone())).await.unwrap();
        for entry in std::fs::read_dir(&dir).expect("read component-store dir") {
            let p = entry.expect("dir entry").path();
            if p.extension().and_then(|e| e.to_str()) == Some("wasm") {
                cas.put(Bytes::from(std::fs::read(&p).unwrap()))
                    .await
                    .unwrap();
            }
        }
        let program = ProgramHash::of(&guest);
        let store = wasm_program_store(Arc::new(cas))
            .with_component_override(heap_hash, Bytes::from(rctrace_heap));
        let mut reducer = store
            .spawn(program, ord(b"rctrace"))
            .await
            .expect("spawn the guest composed against the rc-trace runtime (is CDZ_RCTRACE_RUNTIME_WASM the rctrace build?)");

        let base = crate::Message {
            id: crate::ContractId::of(b"echo-contract"),
            payload: Bytes::from_static(b"ping"),
            from: crate::Origin {
                reducer: crate::ReducerId::of(b"caller"),
                host: crate::HostId::of(b"node"),
            },
            continuation_token: Bytes::from_static(b"tok"),
        };

        reducer
            .rc_trace_enable(true)
            .await
            .expect("rc-trace-enable (is the composed heap the rctrace build?)");
        let _ = reducer.on_message(base).await.expect("fold succeeds");
        let buf = reducer
            .rc_trace_drain()
            .await
            .expect("rc-trace-drain reads");

        // Decode the flat 20-byte records inline (op, tag, freed, node, rc_before, rc_after, cascade).
        const REC: usize = 20;
        assert!(
            buf.len().is_multiple_of(REC),
            "ragged rc-trace drain: {} bytes",
            buf.len()
        );
        let le = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        // per node: (last rc_after seen, tag byte, ever-freed, ever-cascade-reached)
        use std::collections::BTreeMap;
        let mut nodes: BTreeMap<u32, (u32, u8, bool, bool, bool)> = BTreeMap::new(); // (rc_after,tag,alloc,freed_or_immortal,cascade_seen)
        // Per-node full event history (op, rc_before, rc_after, cascade_target) in emission order,
        // plus a global alloc-order rank so we can read construction order (guest step-tree vs wrapper rebuild).
        let mut events: BTreeMap<u32, Vec<(u8, u32, u32, u32)>> = BTreeMap::new();
        let mut alloc_rank: BTreeMap<u32, usize> = BTreeMap::new();
        let mut next_alloc = 0usize;
        for rec in buf.chunks_exact(REC) {
            let (op, tag, freed) = (rec[0], rec[1], rec[2] != 0);
            let (node, rc_before, rc_after) = (le(&rec[4..8]), le(&rec[8..12]), le(&rec[12..16]));
            let cascade_raw = le(&rec[16..20]);
            let cascade = cascade_raw != 0xFFFF_FFFF;
            events
                .entry(node)
                .or_default()
                .push((op, rc_before, rc_after, cascade_raw));
            let e = nodes.entry(node).or_insert((0, tag, false, false, false));
            e.0 = rc_after;
            e.1 = tag;
            match op {
                0 => {
                    // ALLOC
                    e.2 = true;
                    alloc_rank.entry(node).or_insert_with(|| {
                        let r = next_alloc;
                        next_alloc += 1;
                        r
                    });
                }
                2 if freed => e.3 = true, // DROP freed
                3 => e.3 = true,          // MARK_IMMORTAL (left census legitimately)
                _ => {}
            }
            if op == 2 && cascade {
                e.4 = true;
            }
        }
        let tagname = |t: u8| match t {
            0 => "Leaf",
            1 => "Sum",
            2 => "Compound",
            _ => "Other",
        };
        let mut leaked: Vec<_> = nodes
            .iter()
            .filter(|(_, (_, _, alloc, done, _))| *alloc && !*done)
            .collect();
        leaked.sort_by_key(|(n, _)| **n);
        eprintln!(
            "rc-trace attribution: on_message fold — {} total nodes, {} LEAKED:",
            nodes.len(),
            leaked.len()
        );
        let opname = |o: u8| match o {
            0 => "ALLOC",
            1 => "DUP",
            2 => "DROP",
            3 => "IMMORTAL",
            _ => "op?",
        };
        for (node, (rc, tag, _, _, cascade)) in &leaked {
            let rank = alloc_rank.get(node).copied().unwrap_or(usize::MAX);
            eprintln!(
                "  LEAK node#{node} kind={} residual_rc={rc} cascade_reached={cascade} alloc_order={rank}",
                tagname(*tag)
            );
            // Full event history: count DUPs vs DROPs to distinguish "dup'd twice" from
            // "dup'd once + missing intermediate drop", and expose the cascade target of each drop.
            if let Some(evs) = events.get(node) {
                let dups = evs.iter().filter(|(o, ..)| *o == 1).count();
                let drops = evs.iter().filter(|(o, ..)| *o == 2).count();
                eprintln!("      history ({dups} DUP, {drops} DROP):");
                for (o, rcb, rca, casc) in evs {
                    let ct = if *casc == 0xFFFF_FFFF {
                        String::new()
                    } else {
                        format!(" cascade->#{casc}")
                    };
                    eprintln!("        {:<8} rc {rcb}->{rca}{ct}", opname(*o));
                }
            }
        }
        eprintln!(
            "rc-trace attribution HINT: a Leaf(=Bytes)/record leaking rc>1 ⇒ forwarded-dup imbalance (site-a interaction); rc==1 & cascade_reached=false ⇒ op_drop didn't cascade (runtime/emit gap); a Compound with no drop at all + others reached ⇒ emit_result_spill top-drop gap"
        );
    }
}
