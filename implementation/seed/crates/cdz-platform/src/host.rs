//! The wasm-runtime host (`design/cadenza-platform.md` §3) — behind the `host` feature, off by default.
//!
//! `wasmtime` instantiates a reducer component and drives it through the WIT world (`wit/world.wit`): the
//! host provides the imports — `state`, `blobs`, `identity`, and, for an event reducer, the `graph`,
//! `deliver`, and `program-of` reads — and calls the guest's `on-message`/`on-response`/`on-notification`
//! exports. Every host import is async (`async: true` below), so a disk- or network-backed backend never
//! blocks the host thread while a reducer awaits it; the guest sees the calls as ordinary.
//!
//! This slice generates the host-side bindings for the (privileged) event-reducer world and confirms the WIT
//! ABI projects into valid wasmtime host bindings. Instantiating a component as a [`Reducer`](crate::Reducer)
//! and backing the imports over the in-memory [`KvStore`](crate::KvStore) / [`BlobStore`](crate::BlobStore)
//! is the following slice.
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
    BlobStore, Bytes, ContractId, EdgeKind, Error, Hash, KvStore, Message, Notification, Origin,
    Outcome, ReducerGraph, ReducerId, Request, Response,
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
}

impl cadenza::platform::identity::Host for HostState {
    async fn id(&mut self) -> Vec<u8> {
        self.id.hash().as_bytes().to_vec()
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

impl cadenza::platform::graph::Host for HostState {
    async fn insert(&mut self, node: Vec<u8>) -> bool {
        match to_reducer(&node) {
            Some(node) => self.graph.insert(node).await,
            None => false,
        }
    }

    async fn contains(&mut self, node: Vec<u8>) -> bool {
        match to_reducer(&node) {
            Some(node) => self.graph.contains(node).await,
            None => false,
        }
    }

    async fn remove(&mut self, node: Vec<u8>) -> bool {
        match to_reducer(&node) {
            Some(node) => self.graph.remove(node).await,
            None => false,
        }
    }

    async fn link(&mut self, source: Vec<u8>, target: Vec<u8>, kind: Vec<u8>) -> bool {
        match (to_reducer(&source), to_reducer(&target), to_kind(&kind)) {
            (Some(source), Some(target), Some(kind)) => self.graph.link(source, target, kind).await,
            _ => false,
        }
    }

    async fn set_edges(
        &mut self,
        source: Vec<u8>,
        kind: Vec<u8>,
        targets: Vec<Vec<u8>>,
    ) -> Vec<Vec<u8>> {
        let (Some(source), Some(kind)) = (to_reducer(&source), to_kind(&kind)) else {
            return Vec::new();
        };
        // A malformed target names nothing, so it is dropped from the chain rather than aborting the set.
        let targets = targets.iter().filter_map(|t| to_reducer(t)).collect();
        from_reducers(self.graph.set_edges(source, kind, targets).await)
    }

    async fn neighbors(
        &mut self,
        node: Vec<u8>,
        kind: Vec<u8>,
        dir: cadenza::platform::graph::Dir,
    ) -> Vec<Vec<u8>> {
        let (Some(node), Some(kind)) = (to_reducer(&node), to_kind(&kind)) else {
            return Vec::new();
        };
        from_reducers(self.graph.neighbors(node, kind, dir.into()).await)
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
            None => Vec::new(),
        }
    }

    async fn reach(
        &mut self,
        node: Vec<u8>,
        kind: Vec<u8>,
        dir: cadenza::platform::graph::Dir,
    ) -> Vec<Vec<u8>> {
        let (Some(node), Some(kind)) = (to_reducer(&node), to_kind(&kind)) else {
            return Vec::new();
        };
        from_reducers(self.graph.reach(node, kind, dir.into()).await)
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

// ── The wasm reducer driver (§3) ─────────────────────────────────────────────────────────────────────────
// Turning a reducer component into a live [`Reducer`](crate::Reducer): a wasmtime `Store` holding the
// component's [`HostState`] and an instantiated world. Folding an event is build-the-record → call-the-guest
// → decode-the-step, composing the conversions above. The host imports are async (the `bindgen!` above), so
// instantiation and every fold run on an async store — a disk/network-backed backend an import awaits never
// blocks the host thread.

use crate::{Reducer, ReducerKind};
use async_trait::async_trait;
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
    Engine::new(&config)
}

/// Wire the host imports a reducer of the given [`ReducerKind`] may hold into `linker`, each backed by the
/// [`HostState`] in the store. The kind decides the capability set (§3 trust root): EVERY reducer gets its own
/// state, the content-addressed store, and its own id, but only an event reducer gets the privileged imports —
/// the routing `graph` (and `deliver`/`provenance` once their node-shared context lands). This is the
/// least-privilege wiring the world design rests on: an ordinary reducer's linker simply has no `graph`
/// import, so a component that tries to import it fails to instantiate against that linker (the capability is
/// enforced by what the kernel wires, never a runtime check an ordinary reducer could attempt).
fn add_host_imports(
    linker: &mut Linker<HostState>,
    kind: ReducerKind,
) -> Result<(), wasmtime::Error> {
    // The floor every reducer stands on.
    cadenza::platform::identity::add_to_linker::<_, HostData>(linker, |s| s)?;
    cadenza::platform::blobs::add_to_linker::<_, HostData>(linker, |s| s)?;
    cadenza::platform::state::add_to_linker::<_, HostData>(linker, |s| s)?;
    // Privileged: only an event reducer may read and mutate the routing substrate (and, later, deliver and
    // read program provenance).
    if matches!(kind, ReducerKind::Event) {
        cadenza::platform::graph::add_to_linker::<_, HostData>(linker, |s| s)?;
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
}

impl ReducerHost {
    /// Build the shared engine and wire one linker per reducer kind — once per host.
    fn new() -> Result<Self, wasmtime::Error> {
        let engine = reducer_engine()?;
        let mut ordinary_linker = Linker::new(&engine);
        add_host_imports(&mut ordinary_linker, ReducerKind::Ordinary)?;
        let mut event_linker = Linker::new(&engine);
        add_host_imports(&mut event_linker, ReducerKind::Event)?;
        Ok(Self {
            engine,
            ordinary_linker,
            event_linker,
        })
    }

    /// The linker holding exactly the capabilities a reducer of `kind` is allowed.
    fn linker_for(&self, kind: ReducerKind) -> &Linker<HostState> {
        match kind {
            ReducerKind::Ordinary => &self.ordinary_linker,
            ReducerKind::Event => &self.event_linker,
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

    // ── The event ↔ WIT conversion layer ──
    use super::{StepError, message_to_wit, notification_to_wit, response_to_wit, step_from_wit};
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

    // The end-to-end driver test — instantiate the reducer-echo guest component, drive a message, assert the
    // echo + the identity import round-trip — is deferred to the slice that wires the guest component into the
    // reproducible nix build (operator: no committed .wasm fixture; the guest is built by cargo-component in
    // the wasm CI job). It loads the nix-built artifact rather than an `include_bytes!` of a checked-in file.
    // (Verified locally against a `cargo component build` of guests/reducer-echo before that wiring landed.)
}
