//! Recording store decorators — a [`KvStore`] and a [`BlobStore`] that log every call
//! (`design/cadenza-platform.md` §7/§8).
//!
//! The design makes the two stores swappable trait objects, so observation is a decorator, not a
//! change to the kernel: wrap the real backend and, on each call, append a record to the shared
//! [`ObservationLog`] before returning the backend's answer unchanged. The wrapped store behaves
//! exactly as the inner one — same values, same order — so recording never alters a run, only observes
//! it (§9). Each decorator is constructed for one reducer (its [`Origin`]), so a recorded call carries
//! who made it; a run gives each reducer its own recording store over the one shared log, and the log
//! interleaves them in the global [`seq`](super::observation::Record::seq) order.
//!
//! The event tap lives here too ([`RecordingReducer`] / [`RecordingProgramStore`]): the same
//! decorator idea applied to a reducer rather than a store — record every event a reducer folds,
//! emits, or closes with, then defer to the wrapped reducer unchanged.

use super::observation::{
    ArgProbeCall, BlobOp, Entry, EventKind, EventOp, GraphOp, KvOp, ObservationLog, ProvOp,
    RejectedCall, RunCall,
};
use crate::{
    ArgProbeSink, BlobStore, Bytes, ContractId, Delivered, Delivery, Dir, EdgeKind, Hash, HostId,
    KeyRange, KvKeyScan, KvScan, KvStore, Message, Notification, Origin, Outcome, ProgramHash,
    ProgramStore, Provenance, Reducer, ReducerGraph, ReducerId, RejectedSink, Request, Response,
    RunError, RunSink, SpawnContext, Str,
};
use async_trait::async_trait;
use futures_util::FutureExt as _; // catch_unwind — record an uncontrolled fold failure (§10) before it unwinds
use std::any::Any;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

/// A [`KvStore`] that records every operation to an [`ObservationLog`], then defers to the wrapped
/// store. Generic over the inner backend, so it decorates any `KvStore` — the in-memory one for tests,
/// a persistent one in a fuller harness. It is itself a `KvStore`, so it drops in wherever a store is
/// expected (behind an `Arc`/`Box`, as the platform holds its backends).
pub struct RecordingKvStore<K> {
    inner: K,
    owner: Origin,
    log: ObservationLog,
    now: fn() -> u64,
}

impl<K> RecordingKvStore<K> {
    /// Wrap `inner`, attributing every recorded call to `owner` (the reducer whose store this is),
    /// appending to `log`, and stamping each record with `now` — pass the runtime's clock
    /// (`Runtime::now`) so records carry deterministic simulated time under the bach simulator.
    pub fn new(inner: K, owner: Origin, log: ObservationLog, now: fn() -> u64) -> Self {
        Self {
            inner,
            owner,
            log,
            now,
        }
    }

    /// Recover the wrapped store — e.g. to read the in-memory backend's final state after a run.
    pub fn into_inner(self) -> K {
        self.inner
    }

    fn record(&self, op: KvOp) {
        self.log.record((self.now)(), self.owner, Entry::Kv(op));
    }
}

#[async_trait]
impl<K: KvStore> KvStore for RecordingKvStore<K> {
    async fn get(&self, key: &[u8]) -> Option<Bytes> {
        let value = self.inner.get(key).await;
        self.record(KvOp::Get {
            key: Bytes::copy_from_slice(key),
            hit: value.is_some(),
        });
        value
    }

    async fn put(&mut self, key: Bytes, value: Bytes) {
        // Record the write (O(1) Bytes clones), then apply it. `put` has no outcome to observe, so the
        // order relative to the backend call does not matter; recording first keeps the write and its
        // record adjacent even if the backend were to yield.
        self.record(KvOp::Put {
            key: key.clone(),
            value: value.clone(),
        });
        self.inner.put(key, value).await;
    }

    async fn delete(&mut self, key: &[u8]) -> bool {
        let existed = self.inner.delete(key).await;
        self.record(KvOp::Delete {
            key: Bytes::copy_from_slice(key),
            existed,
        });
        existed
    }

    fn scan(&self, range: KeyRange) -> KvScan<'_> {
        self.record(KvOp::Scan {
            lower: range.0.clone(),
            upper: range.1.clone(),
            keys_only: false,
        });
        self.inner.scan(range)
    }

    fn scan_keys(&self, range: KeyRange) -> KvKeyScan<'_> {
        self.record(KvOp::Scan {
            lower: range.0.clone(),
            upper: range.1.clone(),
            keys_only: true,
        });
        self.inner.scan_keys(range)
    }
}

/// A [`BlobStore`] that records every operation to an [`ObservationLog`], then defers to the wrapped
/// store. Like [`RecordingKvStore`], it is itself a `BlobStore` and observes without altering the
/// backend's answers.
pub struct RecordingBlobStore<B> {
    inner: B,
    owner: Origin,
    log: ObservationLog,
    now: fn() -> u64,
}

impl<B> RecordingBlobStore<B> {
    /// Wrap `inner`, attributing every recorded call to `owner`, appending to `log`, and stamping each
    /// record with `now` (the runtime's clock).
    pub fn new(inner: B, owner: Origin, log: ObservationLog, now: fn() -> u64) -> Self {
        Self {
            inner,
            owner,
            log,
            now,
        }
    }

    /// Recover the wrapped store.
    pub fn into_inner(self) -> B {
        self.inner
    }

    fn record(&self, op: BlobOp) {
        self.log.record((self.now)(), self.owner, Entry::Blob(op));
    }
}

#[async_trait]
impl<B: BlobStore> BlobStore for RecordingBlobStore<B> {
    async fn put(&mut self, bytes: Bytes, refs: &[Hash]) -> Hash {
        // The stored bytes are addressed by the hash, so the record keeps the hash and the byte length,
        // not the bytes again. Capture the length before the bytes move into the backend. The blob's `refs`
        // edges are forwarded to the backend but not recorded in the observation log — the log captures a
        // reducer's OBSERVABLE store ops (a guest's put/get/has), and edges are GC bookkeeping, not observable.
        let len = bytes.len();
        let hash = self.inner.put(bytes, refs).await;
        self.record(BlobOp::Put { hash, len });
        hash
    }

    async fn get(&self, hash: Hash) -> Option<Bytes> {
        let bytes = self.inner.get(hash).await;
        self.record(BlobOp::Get {
            hash,
            hit: bytes.is_some(),
        });
        bytes
    }

    async fn has(&self, hash: Hash) -> bool {
        let present = self.inner.has(hash).await;
        self.record(BlobOp::Has { hash, present });
        present
    }

    async fn delete(&mut self, hash: Hash) -> bool {
        // GC-only and not a guest-observable op, so it is forwarded to the backend but not logged (there is
        // no `Delete` observation and none is warranted — the checker asserts a guest's own store activity).
        self.inner.delete(hash).await
    }
}

/// A [`Delivery`] that records every **deliver** an event reducer routes to an [`ObservationLog`], then
/// defers to the wrapped delivery (`design/cadenza-platform.md` §4/§9). Wrapping the `deliver` host boundary
/// makes the routing ACT observable in isolation — recorded whether or not a target is running to receive it
/// — so a §4 dispatch conformance run asserts *what the reducer routed* (which primitive, contract, token,
/// payload, target) without a live listener. The send-side counterpart to [`RecordingReducer`]'s receive-side
/// [`Delivered`](EventOp::Delivered): here the record is an [`EventOp::Routed`] attributed to the routing
/// reducer (`owner`). Behind an `Arc` like every [`Delivery`], so the injected factory clones a shared base.
pub struct RecordingDelivery {
    inner: Arc<dyn Delivery>,
    owner: Origin,
    log: ObservationLog,
    now: fn() -> u64,
}

impl RecordingDelivery {
    /// Wrap `inner`, attributing every recorded deliver to `owner` (the reducer whose `deliver` import this
    /// backs), appending to `log`, and stamping each record with `now` (the runtime's clock, for
    /// deterministic simulated time under bach).
    pub fn new(
        inner: Arc<dyn Delivery>,
        owner: Origin,
        log: ObservationLog,
        now: fn() -> u64,
    ) -> Self {
        Self {
            inner,
            owner,
            log,
            now,
        }
    }

    /// The [`EventOp::Routed`] a delivered `event` records — the send-side split of a [`Delivered`] into the
    /// same fields [`RecordingReducer`] records on receipt (a response's `Ok`/`Err` split into payload/error),
    /// plus the `target` routed to. Borrows `event` so the caller can still move it into the wrapped delivery.
    fn routed(target: ReducerId, event: &Delivered) -> EventOp {
        match event {
            Delivered::Message(m) => EventOp::Routed {
                kind: EventKind::Message,
                target,
                contract: m.id,
                continuation_token: m.continuation_token.clone(),
                payload: m.payload.clone(),
                error: None,
            },
            Delivered::Response(r) => {
                let (payload, error) = match &r.payload {
                    Ok(bytes) => (bytes.clone(), None),
                    Err(e) => (Bytes::new(), Some(*e)),
                };
                EventOp::Routed {
                    kind: EventKind::Response,
                    target,
                    contract: r.id,
                    continuation_token: r.continuation_token.clone(),
                    payload,
                    error,
                }
            }
            Delivered::Notification(n) => EventOp::Routed {
                kind: EventKind::Notification,
                target,
                contract: n.id,
                continuation_token: Bytes::new(),
                payload: n.payload.clone(),
                error: None,
            },
        }
    }
}

#[async_trait]
impl Delivery for RecordingDelivery {
    async fn deliver(&self, target: ReducerId, event: Delivered) -> bool {
        // Record the routing ACT (the observable §4 fact) before delegating, so it is captured regardless of
        // whether a target is running — the wrapped delivery's `bool` outcome is landing, not the act.
        self.log.record(
            (self.now)(),
            self.owner,
            Entry::Event(Self::routed(target, &event)),
        );
        self.inner.deliver(target, event).await
    }
}

/// A [`Provenance`] that records every privileged `program-of` read to an [`ObservationLog`], then defers to
/// the wrapped provenance (`design/cadenza-platform.md` §4/§9). Wrapping the `program-of` host boundary makes
/// the read observable — which reducer was queried, and what program the platform answered — so a conformance
/// run can assert a reducer's provenance query and the answer it got. Attributes each record to the querying
/// reducer (`owner`), and records the *answer* (like [`RecordingKvStore`]'s `get`), so it defers after the
/// call. Behind an `Arc` like every [`Provenance`], so the injected factory clones a shared base.
pub struct RecordingProvenance {
    inner: Arc<dyn Provenance>,
    owner: Origin,
    log: ObservationLog,
    now: fn() -> u64,
}

impl RecordingProvenance {
    /// Wrap `inner`, attributing every recorded `program-of` to `owner` (the reducer whose `program-of` import
    /// this backs), appending to `log`, and stamping each record with `now` (the runtime's clock).
    pub fn new(
        inner: Arc<dyn Provenance>,
        owner: Origin,
        log: ObservationLog,
        now: fn() -> u64,
    ) -> Self {
        Self {
            inner,
            owner,
            log,
            now,
        }
    }
}

#[async_trait]
impl Provenance for RecordingProvenance {
    async fn program_of(&self, reducer: ReducerId) -> Option<ProgramHash> {
        // Record the answer, not just the question — the query and what the platform returned (defer after the
        // call, like a kv `get`). The wrapped read is unchanged (§9).
        let program = self.inner.program_of(reducer).await;
        self.log.record(
            (self.now)(),
            self.owner,
            Entry::Provenance(ProvOp::ProgramOf { reducer, program }),
        );
        program
    }
}

/// A [`ReducerGraph`] that records every node-side graph call — read or write — to an [`ObservationLog`],
/// then defers to the wrapped graph (`design/cadenza-platform.md` §7/§9). Wrapping the graph host boundary
/// makes the routing substrate observable: which nodes/edges a reducer inspected or changed, and the result
/// — e.g. an event reducer's `neighbors(kind = for_contract(C))` read is exactly how it routes `C`, so a
/// conformance run asserts "read the chain for C, got handler H". Records the eight host-wired methods
/// (`insert`/`contains`/`remove`/`link`/`set_edges`/`neighbors`/`in_kinds`/`reach`); the trait's other
/// methods are Rust conveniences that decompose to these on the wrapped graph, so they are not double-counted.
/// Records the result (deferred after the call, like [`RecordingKvStore`]'s `get`), then returns it unchanged.
/// Behind an `Arc` like the shared graph, so the injected factory clones the one node-wide graph.
pub struct RecordingGraph {
    inner: Arc<dyn ReducerGraph>,
    owner: Origin,
    log: ObservationLog,
    now: fn() -> u64,
}

impl RecordingGraph {
    /// Wrap `inner`, attributing every recorded graph call to `owner`, appending to `log`, and stamping each
    /// record with `now` (the runtime's clock).
    pub fn new(
        inner: Arc<dyn ReducerGraph>,
        owner: Origin,
        log: ObservationLog,
        now: fn() -> u64,
    ) -> Self {
        Self {
            inner,
            owner,
            log,
            now,
        }
    }

    fn record(&self, op: GraphOp) {
        self.log.record((self.now)(), self.owner, Entry::Graph(op));
    }
}

#[async_trait]
impl ReducerGraph for RecordingGraph {
    async fn insert(&self, node: ReducerId) -> bool {
        let added = self.inner.insert(node).await;
        self.record(GraphOp::Insert { node, added });
        added
    }

    async fn contains(&self, node: ReducerId) -> bool {
        let present = self.inner.contains(node).await;
        self.record(GraphOp::Contains { node, present });
        present
    }

    async fn remove(&self, node: ReducerId) -> bool {
        let existed = self.inner.remove(node).await;
        self.record(GraphOp::Remove { node, existed });
        existed
    }

    async fn link(&self, from: ReducerId, to: ReducerId, kind: EdgeKind) -> bool {
        let added = self.inner.link(from, to, kind).await;
        self.record(GraphOp::Link {
            from,
            to,
            kind,
            added,
        });
        added
    }

    async fn set_edges(
        &self,
        from: ReducerId,
        kind: EdgeKind,
        targets: Vec<ReducerId>,
    ) -> Vec<ReducerId> {
        let recorded_targets = targets.clone();
        let prior = self.inner.set_edges(from, kind, targets).await;
        self.record(GraphOp::SetEdges {
            from,
            kind,
            targets: recorded_targets,
            prior: prior.clone(),
        });
        prior
    }

    async fn neighbors(&self, node: ReducerId, kind: EdgeKind, dir: Dir) -> Vec<ReducerId> {
        let result = self.inner.neighbors(node, kind, dir).await;
        self.record(GraphOp::Neighbors {
            node,
            kind,
            dir,
            result: result.clone(),
        });
        result
    }

    async fn in_kinds(&self, node: ReducerId) -> Vec<EdgeKind> {
        let result = self.inner.in_kinds(node).await;
        self.record(GraphOp::InKinds {
            node,
            result: result.clone(),
        });
        result
    }

    async fn reach(&self, node: ReducerId, kind: EdgeKind, dir: Dir) -> Vec<ReducerId> {
        let result = self.inner.reach(node, kind, dir).await;
        self.record(GraphOp::Reach {
            node,
            kind,
            dir,
            result: result.clone(),
        });
        result
    }
}

/// A [`RejectedSink`] that records a host call rejected at the argument-parse guard — a call the reducer
/// performed whose args did not parse to their typed form, so the host early-returned without reaching the
/// recordable capability (`design/cadenza-platform.md` §9). Appends an [`Entry::HostCallRejected`] to the
/// [`ObservationLog`] attributed to the reducer's [`Origin`], so no host call is silently unobservable
/// (log-every-host-call). The host boundary (an event reducer's parse-guard else-branch) calls
/// [`record`](RejectedSink::record); this sink turns that into an observation. Constructed per reducer, like
/// the store/graph decorators, and wired via [`WasmProgramStore::with_rejected`](crate::WasmProgramStore).
pub struct RecordingRejectedSink {
    owner: Origin,
    log: ObservationLog,
    now: fn() -> u64,
}

impl RecordingRejectedSink {
    /// A sink attributing every rejected call to `owner`, appending to `log`, and stamping each record with
    /// `now` (the runtime's clock).
    pub fn new(owner: Origin, log: ObservationLog, now: fn() -> u64) -> Self {
        Self { owner, log, now }
    }
}

impl RejectedSink for RecordingRejectedSink {
    fn record(&self, iface: &str, op: &str, raw_args: &[Bytes]) {
        self.log.record(
            (self.now)(),
            self.owner,
            Entry::HostCallRejected(RejectedCall {
                iface: Str::from(iface),
                op: Str::from(op),
                raw_args: raw_args.to_vec(),
            }),
        );
    }
}

/// A [`RunSink`] that records a synchronous pure-`run` host call (§3) — the sub-program and contract ids, the
/// input, and the run's outcome — to the [`ObservationLog`] attributed to the reducer's [`Origin`]. A `run` is
/// hosted but leaves no `step.requests` entry, so without this sink a checker cannot observe that a reducer
/// invoked `run` (`design/cadenza-platform.md` §9, log-every-host-call). Maps the crate-level [`RunError`]
/// category to the [`RunCall`] `error` string the WIT `result<payload, error>` surfaces
/// (`UnknownProgram`→`missing-handler`, `DidNotReturn`/`Faulted`→`faulted`). Constructed per reducer, like the
/// store/graph decorators, and wired via [`WasmProgramStore::with_run_sink`](crate::WasmProgramStore).
pub struct RecordingRun {
    owner: Origin,
    log: ObservationLog,
    now: fn() -> u64,
}

impl RecordingRun {
    /// A sink attributing every `run` call to `owner`, appending to `log`, and stamping each record with `now`.
    pub fn new(owner: Origin, log: ObservationLog, now: fn() -> u64) -> Self {
        Self { owner, log, now }
    }
}

impl RunSink for RecordingRun {
    fn record(
        &self,
        program: &[u8],
        contract: &[u8],
        input: &[u8],
        result: &Result<Bytes, RunError>,
    ) {
        // The 3→2 category map the WIT boundary applies: a successful run carries its output bytes, a failed
        // one names its category. `missing-handler` is the unknown-program case; `faulted` covers both a run
        // that trapped and one that never returned.
        let (output, error) = match result {
            Ok(out) => (Some(out.clone()), None),
            Err(RunError::UnknownProgram) => (None, Some(Str::from("missing-handler"))),
            Err(RunError::DidNotReturn | RunError::Faulted) => (None, Some(Str::from("faulted"))),
        };
        self.log.record(
            (self.now)(),
            self.owner,
            Entry::Run(RunCall {
                program: Bytes::copy_from_slice(program),
                contract: Bytes::copy_from_slice(contract),
                input: Bytes::copy_from_slice(input),
                output,
                error,
            }),
        );
    }
}

/// An [`ArgProbeSink`] that records a call to the test-only `arg-probe` host — the two received arguments,
/// each already canonical-`Value.encode`d to bytes by the host, appended as an [`Entry::ArgProbe`] on the one
/// shared log (`design/cadenza-platform.md` §9). Constructed per reducer, like [`RecordingRun`]: it attributes
/// the call to `owner` and stamps each record with `now`. This is what makes a wrong mixed-width arg marshal
/// observable — a checker asserts the recorded values byte-for-byte.
pub struct RecordingArgProbe {
    owner: Origin,
    log: ObservationLog,
    now: fn() -> u64,
}
impl RecordingArgProbe {
    /// A sink attributing every `arg-probe.probe` call to `owner`, appending to `log`, stamping with `now`.
    pub fn new(owner: Origin, log: ObservationLog, now: fn() -> u64) -> Self {
        Self { owner, log, now }
    }
}
impl ArgProbeSink for RecordingArgProbe {
    fn record(&self, record: &[u8], items: &[u8]) {
        self.log.record(
            (self.now)(),
            self.owner,
            Entry::ArgProbe(ArgProbeCall {
                record: Bytes::copy_from_slice(record),
                items: Bytes::copy_from_slice(items),
            }),
        );
    }
}

/// A [`Reducer`] that records every event it folds, every request it emits, and its close to an
/// [`ObservationLog`], then defers to the wrapped reducer (`design/cadenza-platform.md` §3/§4/§10).
/// Attributes records to the reducer's own id, which the kernel provides at spawn (§3).
///
/// The kernel instantiates every reducer through the [`ProgramStore`] the harness hands the system, so
/// wrapping that store (see [`RecordingProgramStore`]) wraps every reducer in one of these — capturing
/// the whole system's event flow with no change to the kernel's routing.
pub struct RecordingReducer {
    inner: Box<dyn Reducer>,
    /// The reducer's own id — known at construction from the [`SpawnContext`](crate::SpawnContext) the
    /// kernel passes to `ProgramStore::spawn` (the id it derived from the genesis, §3). Every recorded
    /// event is attributed to it.
    id: ReducerId,
    host: HostId,
    log: ObservationLog,
    now: fn() -> u64,
}

impl RecordingReducer {
    /// Wrap `inner` — the reducer with id `id` on `host` — recording to `log`, stamping records with `now`
    /// (the runtime clock). The id comes from the spawn context, so it is known before the first fold and
    /// every record (including the birth notification) is attributed correctly.
    pub fn new(
        inner: Box<dyn Reducer>,
        id: ReducerId,
        host: HostId,
        log: ObservationLog,
        now: fn() -> u64,
    ) -> Self {
        Self {
            inner,
            id,
            host,
            log,
            now,
        }
    }

    /// This reducer as an [`Origin`] — its id on this host. The id is fixed at construction from the spawn
    /// context (§3), so there is no unknown-id case.
    fn source(&self) -> Origin {
        Origin {
            reducer: self.id,
            host: self.host,
        }
    }

    fn record(&self, op: EventOp) {
        self.log
            .record((self.now)(), self.source(), Entry::Event(op));
    }

    /// Record the requests a fold emitted and, if it closed, its close — the output side of a fold, in
    /// order. Called after every entry point with what the wrapped reducer returned.
    fn record_output(&self, requests: &[Request], outcome: &Outcome) {
        for request in requests {
            self.record(EventOp::Emitted {
                contract: request.id,
                payload: request.payload.clone(),
                continuation_token: request.continuation_token.clone(),
                has_deadline: request.deadline.is_some(),
            });
        }
        if let Outcome::Break { schema, reason } = outcome {
            self.record(EventOp::Closed {
                schema: *schema,
                reason: reason.clone(),
            });
        }
    }

    /// Resolve a fold caught with [`catch_unwind`](futures_util::FutureExt::catch_unwind): return its
    /// result, or — if the fold panicked (an uncontrolled failure, §3/§10) — record a
    /// [`Failed`](EventOp::Failed) event naming the event whose fold failed and the panic reason, then
    /// resume the unwind so the runtime handles the crash as it otherwise would (a `crashed` lifecycle
    /// event to watchers). Called after the fold future resolves, so it no longer borrows `inner`.
    fn folded_or_record_failure(
        &self,
        folded: std::thread::Result<(Vec<Request>, Outcome)>,
        during: EventKind,
        contract: ContractId,
    ) -> (Vec<Request>, Outcome) {
        match folded {
            Ok(result) => result,
            Err(panic) => {
                self.record(EventOp::Failed {
                    during,
                    contract,
                    reason: panic_reason(&*panic),
                });
                std::panic::resume_unwind(panic);
            }
        }
    }
}

/// The message carried by a fold panic, for the [`Failed`](EventOp::Failed) record — the `&str` or
/// `String` a `panic!` produced, or a fixed note when the payload is neither.
fn panic_reason(panic: &(dyn Any + Send)) -> Str {
    if let Some(s) = panic.downcast_ref::<&str>() {
        Str::from(*s)
    } else if let Some(s) = panic.downcast_ref::<String>() {
        Str::from(s.as_str())
    } else {
        Str::from_static("reducer fold panicked")
    }
}

#[async_trait]
impl Reducer for RecordingReducer {
    async fn on_message(&mut self, message: Message) -> (Vec<Request>, Outcome) {
        let contract = message.id;
        self.record(EventOp::Delivered {
            kind: EventKind::Message,
            contract,
            from: Some(message.from),
            continuation_token: message.continuation_token.clone(),
            payload: message.payload.clone(),
            error: None,
        });
        let folded = AssertUnwindSafe(self.inner.on_message(message))
            .catch_unwind()
            .await;
        let (requests, outcome) =
            self.folded_or_record_failure(folded, EventKind::Message, contract);
        self.record_output(&requests, &outcome);
        (requests, outcome)
    }

    async fn on_response(&mut self, response: Response) -> (Vec<Request>, Outcome) {
        // A response carries its result in the payload: `Ok` bytes, or an `Err` runtime failure (§3). Split
        // it so the record shows the answer or the failure without wrapping.
        let (payload, error) = match &response.payload {
            Ok(bytes) => (bytes.clone(), None),
            Err(e) => (Bytes::new(), Some(*e)),
        };
        let contract = response.id;
        self.record(EventOp::Delivered {
            kind: EventKind::Response,
            contract,
            from: None,
            continuation_token: response.continuation_token.clone(),
            payload,
            error,
        });
        let folded = AssertUnwindSafe(self.inner.on_response(response))
            .catch_unwind()
            .await;
        let (requests, outcome) =
            self.folded_or_record_failure(folded, EventKind::Response, contract);
        self.record_output(&requests, &outcome);
        (requests, outcome)
    }

    async fn on_notification(&mut self, notification: Notification) -> (Vec<Request>, Outcome) {
        let contract = notification.id;
        self.record(EventOp::Delivered {
            kind: EventKind::Notification,
            contract,
            from: None,
            continuation_token: Bytes::new(),
            payload: notification.payload.clone(),
            error: None,
        });
        let folded = AssertUnwindSafe(self.inner.on_notification(notification))
            .catch_unwind()
            .await;
        let (requests, outcome) =
            self.folded_or_record_failure(folded, EventKind::Notification, contract);
        self.record_output(&requests, &outcome);
        (requests, outcome)
    }
}

/// A [`ProgramStore`] that wraps every reducer it instantiates in a [`RecordingReducer`], so every fold
/// in the system is recorded to one [`ObservationLog`]. Hand this to `TaskSystem::new` in place of the
/// real program store and the whole run is observed — no kernel change.
pub struct RecordingProgramStore<P> {
    inner: P,
    host: HostId,
    log: ObservationLog,
    now: fn() -> u64,
}

impl<P> RecordingProgramStore<P> {
    /// Wrap `inner`, recording every instantiated reducer's folds to `log`, attributed to `host`, stamped
    /// with `now` (pass the runtime's `Runtime::now`).
    pub fn new(inner: P, host: HostId, log: ObservationLog, now: fn() -> u64) -> Self {
        Self {
            inner,
            host,
            log,
            now,
        }
    }
}

#[async_trait]
impl<P: ProgramStore> ProgramStore for RecordingProgramStore<P> {
    async fn spawn(&self, program: ProgramHash, ctx: SpawnContext) -> Option<Box<dyn Reducer>> {
        // Capture the reducer's id from the spawn context before delegating, so the RecordingReducer knows
        // it up front (no birth-learning needed).
        let id = ctx.id;
        let inner = self.inner.spawn(program, ctx).await?;
        Some(Box::new(RecordingReducer::new(
            inner,
            id,
            self.host,
            self.log.clone(),
            self.now,
        )))
    }

    async fn contains(&self, program: ProgramHash) -> bool {
        self.inner.contains(program).await
    }

    fn set_node_delivery(&self, delivery: Arc<dyn Delivery>) {
        // Forward to the wrapped store so a `TaskSystem` running THROUGH this decorator still wires the
        // `deliver` host import to the live node (the recording layer observes deliveries, it does not back them).
        self.inner.set_node_delivery(delivery);
    }
}
