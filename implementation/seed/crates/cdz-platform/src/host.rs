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
    imports: { default: async },
    exports: { default: async },
    // Derive equality on the generated records/variants so the conversion layer can be asserted field-for-
    // field in tests; every WIT type here is over bytes/enums, so `PartialEq`/`Eq` are well-defined.
    additional_derives: [PartialEq, Eq],
});

use crate::{
    BlobStore, Bytes, ContractId, Delivered, Delivery, EdgeKind, Error, Hash, HostId, KvStore,
    Message, Notification, Origin, Outcome, ReducerGraph, ReducerId, RejectedSink, Request,
    ResourceLimits, Response,
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
    /// id/kind) is recorded, so no host call is silently unobserved (§9). Always present
    /// ([`NoRejectedSink`](crate::NoRejectedSink) when no observing node wired one), so the parse-guard path
    /// records unconditionally without an `Option` to branch on.
    rejected: Arc<dyn RejectedSink>,
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
        match inst.run_pure(program, contract, Bytes::from(input)).await {
            Ok(output) => Ok(output.to_vec()),
            // No program to run maps to `missing-handler`; a fault or a program that never returned is the
            // general `faulted` — the same mapping the run effect uses (§3/§4).
            Err(RunError::UnknownProgram) => Err(wit_types::Error::MissingHandler),
            Err(RunError::DidNotReturn | RunError::Faulted) => Err(wit_types::Error::Faulted),
        }
    }
}

impl cadenza::platform::blobs::Host for HostState {
    async fn get(&mut self, hash: Vec<u8>) -> Option<Vec<u8>> {
        // A malformed hash (not exactly `Hash::LEN` bytes) names nothing, so it reads back as absent.
        let hash = Hash::from_bytes(<[u8; Hash::LEN]>::try_from(hash.as_slice()).ok()?);
        self.blobs.get(hash).await.map(|bytes| bytes.to_vec())
    }

    async fn put(&mut self, bytes: Vec<u8>) -> Vec<u8> {
        self.blobs.put(Bytes::from(bytes)).await.as_bytes().to_vec()
    }
}

impl cadenza::platform::state::Host for HostState {
    async fn get(&mut self, key: Vec<u8>) -> Option<Vec<u8>> {
        self.kv.get(&key).await.map(|value| value.to_vec())
    }

    async fn put(&mut self, key: Vec<u8>, value: Vec<u8>) {
        self.kv.put(Bytes::from(key), Bytes::from(value)).await;
    }

    async fn delete(&mut self, key: Vec<u8>) {
        // The WIT `delete` reports nothing; the key-value store's whether-it-was-present is not surfaced.
        self.kv.delete(&key).await;
    }
}

// Each method parses its `list<u8>` node/edge-kind arg into a `ReducerId`/`EdgeKind`; on a malformed one it
// returns the empty/false result (total, graceful) AND records the rejected call to `self.rejected` with the
// raw argument bytes, so the call is observed (§9) even though it never reached the recordable `self.graph`
// capability below the parse. A well-formed call records via the graph decorator as usual; only the rejected
// path is recorded here. The record is gated on `self.rejected.enabled()`, so when no recorder is wired the
// parse-guard path pays ZERO allocation; when it is, the raw `Vec<u8>` args move into `Bytes` with no copy
// (`Bytes::from` is O(1)). `iface`/`op` name the WIT interface + method.
impl cadenza::platform::graph::Host for HostState {
    async fn insert(&mut self, node: Vec<u8>) -> bool {
        match to_reducer(&node) {
            Some(node) => self.graph.insert(node).await,
            None => {
                if self.rejected.enabled() {
                    self.rejected
                        .record("graph", "insert", &[Bytes::from(node)]);
                }
                false
            }
        }
    }

    async fn contains(&mut self, node: Vec<u8>) -> bool {
        match to_reducer(&node) {
            Some(node) => self.graph.contains(node).await,
            None => {
                if self.rejected.enabled() {
                    self.rejected
                        .record("graph", "contains", &[Bytes::from(node)]);
                }
                false
            }
        }
    }

    async fn remove(&mut self, node: Vec<u8>) -> bool {
        match to_reducer(&node) {
            Some(node) => self.graph.remove(node).await,
            None => {
                if self.rejected.enabled() {
                    self.rejected
                        .record("graph", "remove", &[Bytes::from(node)]);
                }
                false
            }
        }
    }

    async fn link(&mut self, source: Vec<u8>, target: Vec<u8>, kind: Vec<u8>) -> bool {
        match (to_reducer(&source), to_reducer(&target), to_kind(&kind)) {
            (Some(source), Some(target), Some(kind)) => self.graph.link(source, target, kind).await,
            _ => {
                if self.rejected.enabled() {
                    self.rejected.record(
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
            if self.rejected.enabled() {
                let mut raw = vec![Bytes::from(source), Bytes::from(kind)];
                raw.extend(targets.into_iter().map(Bytes::from));
                self.rejected.record("graph", "set-edges", &raw);
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
            if self.rejected.enabled() {
                self.rejected.record(
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
                if self.rejected.enabled() {
                    self.rejected
                        .record("graph", "in-kinds", &[Bytes::from(node)]);
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
            if self.rejected.enabled() {
                self.rejected
                    .record("graph", "reach", &[Bytes::from(node), Bytes::from(kind)]);
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
            if self.rejected.enabled() {
                self.rejected
                    .record("provenance", "program-of", &[Bytes::from(reducer)]);
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
// raw `target` id is captured. Gated on `enabled()` (zero alloc when no recorder) with a zero-copy `Bytes::from`.
impl cadenza::platform::deliver::Host for HostState {
    async fn deliver_message(&mut self, target: Vec<u8>, event: wit_reducer::Message) -> bool {
        // A malformed target or a malformed event (an id that is not a hash, an origin that is not) names
        // nothing to deliver to or from, so it is a failed delivery — `false` — not a panic. The node-side
        // delivery reports whether a reducer is running under `target` and received it.
        let (Some(target_id), Some(message)) = (to_reducer(&target), message_from_wit(event))
        else {
            if self.rejected.enabled() {
                self.rejected
                    .record("deliver", "deliver-message", &[Bytes::from(target)]);
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
            if self.rejected.enabled() {
                self.rejected
                    .record("deliver", "deliver-response", &[Bytes::from(target)]);
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
            if self.rejected.enabled() {
                self.rejected
                    .record("deliver", "deliver-notification", &[Bytes::from(target)]);
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
use wasmtime::{Config, Engine, Store};

/// The wasmtime [`Engine`] reducer components are compiled and run on: async (host imports may await) with the
/// component model enabled. One engine is shared by every reducer on a host — it is the compilation context,
/// cheap to clone, and holds no per-instance state.
fn reducer_engine() -> Result<Engine, wasmtime::Error> {
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
    /// The node's per-reducer resource limits (compute budget + memory ceiling + epoch tick) this host arms
    /// every reducer store with (`arm_store_safety`). Set once at assembly from the node's config — never a
    /// hard-coded cap.
    limits: ResourceLimits,
}

impl ReducerHost {
    /// Build the shared engine and wire one linker per reducer kind — once per host — carrying the node's
    /// resource `limits` to arm each reducer store with.
    fn new(limits: ResourceLimits) -> Result<Self, wasmtime::Error> {
        let engine = reducer_engine()?;
        let mut ordinary_linker = Linker::new(&engine);
        add_host_imports(&mut ordinary_linker, ReducerKind::Ordinary)?;
        let mut event_linker = Linker::new(&engine);
        add_host_imports(&mut event_linker, ReducerKind::Event)?;
        let mut pure_linker = Linker::new(&engine);
        add_host_imports(&mut pure_linker, ReducerKind::Pure)?; // wires only `run` — otherwise empty (§3)
        Ok(Self {
            engine,
            ordinary_linker,
            event_linker,
            pure_linker,
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
        Ok(WasmReducer { store, world })
    }
}

/// A [`Reducer`](crate::Reducer) backed by a wasm component: the wasmtime `Store` carrying its [`HostState`]
/// and the instantiated event-reducer world. Each folded event builds the WIT record, calls the matching
/// guest export, and decodes the returned step. It is `Send` but not `Sync` (a `Store` is not `Sync`) — which
/// is exactly what [`Reducer`](crate::Reducer) requires, since the runtime moves a reducer into its own task
/// and drives it only through `&mut` from there. Built by [`ReducerHost::instantiate`].
struct WasmReducer {
    store: Store<HostState>,
    world: EventReducerWorld,
}

// The three entry points share the same shape: encode the event, call the guest, decode the step. A wasm
// trap or a guest returning a malformed step is a failed fold — it panics, which the system's per-fold
// `catch_unwind` turns into the reducer's `Crashed` lifecycle event (§7), the same as any other fold failure.
#[async_trait]
impl Reducer for WasmReducer {
    async fn on_message(&mut self, message: Message) -> (Vec<Request>, Outcome) {
        let event = message_to_wit(&message);
        let step = self
            .world
            .cadenza_platform_guest()
            .call_on_message(&mut self.store, &event)
            .await
            .expect("reducer on_message trapped");
        step_from_wit(step).expect("reducer returned a malformed step")
    }

    async fn on_response(&mut self, response: Response) -> (Vec<Request>, Outcome) {
        let event = response_to_wit(&response);
        let step = self
            .world
            .cadenza_platform_guest()
            .call_on_response(&mut self.store, &event)
            .await
            .expect("reducer on_response trapped");
        step_from_wit(step).expect("reducer returned a malformed step")
    }

    async fn on_notification(&mut self, notification: Notification) -> (Vec<Request>, Outcome) {
        let event = notification_to_wit(&notification);
        let step = self
            .world
            .cadenza_platform_guest()
            .call_on_notification(&mut self.store, &event)
            .await
            .expect("reducer on_notification trapped");
        step_from_wit(step).expect("reducer returned a malformed step")
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
        })
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
        let bytes = self.cas.get(hash).await?;
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
    fn bind_dependencies<'a>(
        &'a self,
        store: &'a mut Store<HostState>,
        linker: &'a mut Linker<HostState>,
        component: &'a Component,
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
                // imports, only sub-dependencies from the store.
                let mut dep_linker = Linker::new(&self.host.engine);
                self.bind_dependencies(store, &mut dep_linker, &dep_component)
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
        let component = self.component(program.hash()).await?;
        let reducer = if component_dependencies(&self.host.engine, &component).is_empty() {
            // Fast path: no content-addressed component dependencies, so reuse the cached, pre-instantiated
            // per-kind linker (the engine, linker, and pre are all shared).
            let pre = self.host.preinstantiate(&component, kind).ok()?;
            self.host.instantiate(&pre, host_state).await.ok()?
        } else {
            // Compose path: the component imports dependencies (the value-heap runtime, …) that must be
            // resolved from the store and instantiated into THIS store, so a fresh per-spawn linker is built
            // (a store-bound dependency instance cannot be pre-instantiated). Host imports for the kind are
            // wired first (the capability split), then the dependencies composed in.
            let mut store = Store::new(&self.host.engine, host_state);
            arm_store_safety(&mut store);
            let mut linker = Linker::new(&self.host.engine);
            add_host_imports(&mut linker, kind).ok()?;
            self.bind_dependencies(&mut store, &mut linker, &component)
                .await
                .ok()?;
            let world = EventReducerWorld::instantiate_async(&mut store, &component, &linker)
                .await
                .ok()?;
            WasmReducer { store, world }
        };
        Some(Box::new(reducer))
    }

    /// Whether the store holds `program`'s component bytes — a content lookup by the program hash (§8).
    async fn contains(&self, program: ProgramHash) -> bool {
        self.cas.has(program.hash()).await
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
        rejected: Arc::new(crate::NoRejectedSink),
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
    /// malformed-arg `graph` op, §9). Defaults to a factory over [`NoRejectedSink`](crate::NoRejectedSink)
    /// until set with [`with_rejected`](WasmProgramStore::with_rejected), so a rejected call is dropped unless
    /// an observing node injects a recording sink.
    make_rejected: RejectedSinkFactory,
}

impl WasmProgramStore {
    /// A store loading components from `cas`, giving each reducer the state, content-store view, and reducer-
    /// graph view its factories build. Fails only if the wasm engine/linkers cannot be built
    /// ([`ReducerHost::new`]). Provenance and delivery default to factories over
    /// [`NoProvenance`](crate::NoProvenance)/[`NoDelivery`](crate::NoDelivery) until set with
    /// [`with_provenance`](WasmProgramStore::with_provenance)/[`with_delivery`](WasmProgramStore::with_delivery).
    /// `make_graph` mirrors `make_blobs`/`make_kv`: a plain caller passes `move |_id| graph.clone()` over its
    /// one shared graph; an injecting caller passes a factory that decorates it (recording is one such use).
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
            make_rejected: Arc::new(|_id| Arc::new(crate::NoRejectedSink) as Arc<dyn RejectedSink>),
        })
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
    /// calling reducer. The default drops rejected calls ([`NoRejectedSink`](crate::NoRejectedSink)). Uniform
    /// with `make_blobs`.
    #[must_use]
    pub fn with_rejected(mut self, make_rejected: RejectedSinkFactory) -> Self {
        self.make_rejected = make_rejected;
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
            rejected: (self.make_rejected)(ctx.id),
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
            rejected: Arc::new(crate::NoRejectedSink),
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
    async fn blobs_round_trip_and_a_malformed_hash_is_absent() {
        let mut host = host(ReducerId::of(b"me"));
        // `put` stores the bytes and returns their content hash; `get` reads them back by that hash.
        let hash = Blobs::put(&mut host, b"a blob".to_vec()).await;
        assert_eq!(
            Blobs::get(&mut host, hash).await.as_deref(),
            Some(b"a blob".as_slice())
        );
        // A hash the store does not hold reads back as absent, and so does a malformed (wrong-length) hash.
        assert_eq!(
            Blobs::get(&mut host, b"not a real hash".to_vec()).await,
            None
        );
    }

    #[tokio::test]
    async fn state_get_put_delete() {
        let mut host = host(ReducerId::of(b"me"));
        // Absent key reads back as nothing; put then get returns the value; delete removes it.
        assert_eq!(State::get(&mut host, b"k".to_vec()).await, None);
        State::put(&mut host, b"k".to_vec(), b"v".to_vec()).await;
        assert_eq!(
            State::get(&mut host, b"k".to_vec()).await.as_deref(),
            Some(b"v".as_slice())
        );
        State::delete(&mut host, b"k".to_vec()).await;
        assert_eq!(State::get(&mut host, b"k".to_vec()).await, None);
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
        host.rejected = sink.clone();

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
        host.rejected = sink.clone();

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
        let mut cas = InMemoryBlobStore::new();
        let bytes = b"not a valid wasm component".to_vec();
        let blob = cas.put(Bytes::from(bytes.clone())).await;
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
}
