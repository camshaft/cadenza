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

use super::observation::{BlobOp, Entry, EventKind, EventOp, KvOp, ObservationLog};
use crate::{
    BlobStore, Bytes, Hash, HostId, KeyRange, KvKeyScan, KvScan, KvStore, Message, Notification,
    Origin, Outcome, ProgramHash, ProgramStore, Reducer, ReducerId, Request, Response,
    SpawnContext, Spawned, spawned_contract,
};
use async_trait::async_trait;

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
    async fn put(&mut self, bytes: Bytes) -> Hash {
        // The stored bytes are addressed by the hash, so the record keeps the hash and the byte length,
        // not the bytes again. Capture the length before the bytes move into the backend.
        let len = bytes.len();
        let hash = self.inner.put(bytes).await;
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
}

/// A [`Reducer`] that records every event it folds, every request it emits, and its close to an
/// [`ObservationLog`], then defers to the wrapped reducer (`design/cadenza-platform.md` §3/§4/§10).
/// Attributes records to the reducer's own id, learned from its birth notification (§3).
///
/// The kernel instantiates every reducer through the [`ProgramStore`] the harness hands the system, so
/// wrapping that store (see [`RecordingProgramStore`]) wraps every reducer in one of these — capturing
/// the whole system's event flow with no change to the kernel's routing.
pub struct RecordingReducer {
    inner: Box<dyn Reducer>,
    /// The reducer's own id, learned from its birth `spawned` notification — a reducer's first event
    /// (§3). Every recorded event is attributed to it.
    me: Option<ReducerId>,
    host: HostId,
    log: ObservationLog,
    now: fn() -> u64,
}

impl RecordingReducer {
    /// Wrap `inner`, recording to `log`, stamping records with `now` (the runtime clock), and attributing
    /// them to `host` plus the reducer's own id, learned from its birth notification.
    pub fn new(
        inner: Box<dyn Reducer>,
        host: HostId,
        log: ObservationLog,
        now: fn() -> u64,
    ) -> Self {
        Self {
            inner,
            me: None,
            host,
            log,
            now,
        }
    }

    /// This reducer as an [`Origin`] — its id on this host. The id is learned from the reducer's birth
    /// `spawned` notification, which the runtime always delivers as its first event (§3), before any
    /// message or response; so by the time anything is recorded the id is known. If it is somehow not —
    /// a reducer recording before its birth — that is a broken invariant, so panic rather than attribute
    /// the event to the wrong reducer.
    fn source(&self) -> Origin {
        Origin {
            reducer: self.me.expect(
                "a reducer's id is set before it records: its birth `spawned` notification is always \
                 its first event (design/cadenza-platform.md §3)",
            ),
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
}

#[async_trait]
impl Reducer for RecordingReducer {
    async fn on_message(&mut self, message: Message) -> (Vec<Request>, Outcome) {
        self.record(EventOp::Delivered {
            kind: EventKind::Message,
            contract: message.id,
            from: Some(message.from),
            continuation_token: message.continuation_token.clone(),
            payload: message.payload.clone(),
            error: None,
        });
        let (requests, outcome) = self.inner.on_message(message).await;
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
        self.record(EventOp::Delivered {
            kind: EventKind::Response,
            contract: response.id,
            from: None,
            continuation_token: response.continuation_token.clone(),
            payload,
            error,
        });
        let (requests, outcome) = self.inner.on_response(response).await;
        self.record_output(&requests, &outcome);
        (requests, outcome)
    }

    async fn on_notification(&mut self, notification: Notification) -> (Vec<Request>, Outcome) {
        // Learn our own id from the birth notification (§3), before recording, so even birth is attributed
        // to the right reducer. Birth is a reducer's first event, so this resolves `me` up front.
        if self.me.is_none()
            && notification.id == spawned_contract()
            && let Some(spawned) = Spawned::decode(&notification.payload)
        {
            self.me = Some(spawned.id);
        }
        self.record(EventOp::Delivered {
            kind: EventKind::Notification,
            contract: notification.id,
            from: None,
            continuation_token: Bytes::new(),
            payload: notification.payload.clone(),
            error: None,
        });
        let (requests, outcome) = self.inner.on_notification(notification).await;
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
        let inner = self.inner.spawn(program, ctx).await?;
        Some(Box::new(RecordingReducer::new(
            inner,
            self.host,
            self.log.clone(),
            self.now,
        )))
    }

    async fn contains(&self, program: ProgramHash) -> bool {
        self.inner.contains(program).await
    }
}
