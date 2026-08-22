//! The event tap — recording every event a reducer folds, emits, or closes with, without touching the
//! kernel (`design/cadenza-platform.md` §3/§4/§9).
//!
//! The kernel instantiates *every* reducer — ordinary, event, and the per-event system reducers it
//! spawns to route effects — through the [`ProgramStore`] the harness hands [`TaskSystem`]. That is the
//! seam: [`RecordingProgramStore`] wraps a program store and wraps each reducer it instantiates in a
//! [`RecordingReducer`], so every fold in the whole system flows through a decorator that records it.
//! No hook in the kernel's routing is needed — the observation log is assembled entirely from the
//! reducer boundary the harness already owns.
//!
//! [`RecordingReducer`] records three things per fold, in causal order: the event delivered into it, the
//! requests it emitted, and (if it terminated) its close. It attributes each to the reducer's own id,
//! which it learns from its **birth** — a reducer's first event is a `spawned` notification carrying its
//! id and parent (§3), so the decorator reads its id from that notification before recording anything
//! else, and every later record is attributed correctly. The tap is behavior-preserving: it records,
//! then defers to the wrapped reducer and returns its result unchanged (§9).
//!
//! [`TaskSystem`]: cdz_platform::TaskSystem

use crate::log::{Entry, EventKind, EventOp, ObservationLog};
use async_trait::async_trait;
use cdz_platform::{
    HostId, Message, Notification, Origin, Outcome, ProgramHash, ProgramStore, Reducer, ReducerId,
    Request, Response, Spawned, spawned_contract,
};

/// A [`Reducer`] that records every event it folds, every request it emits, and its close to an
/// [`ObservationLog`], then defers to the wrapped reducer. Attributes records to the reducer's own id,
/// learned from its birth notification (§3).
pub struct RecordingReducer {
    inner: Box<dyn Reducer>,
    program: ProgramHash,
    /// The reducer's own id, learned from its birth `spawned` notification. `None` only before that
    /// first notification is folded — which, because birth is always the first event, never coincides
    /// with a recorded event in practice.
    me: Option<ReducerId>,
    host: HostId,
    log: ObservationLog,
    now: fn() -> u64,
}

impl RecordingReducer {
    /// Wrap `inner` (instantiated from `program`), recording to `log`, stamping records with `now` (the
    /// runtime clock), and attributing them to `host` plus the reducer's own learned id.
    pub fn new(
        inner: Box<dyn Reducer>,
        program: ProgramHash,
        host: HostId,
        log: ObservationLog,
        now: fn() -> u64,
    ) -> Self {
        Self {
            inner,
            program,
            me: None,
            host,
            log,
            now,
        }
    }

    /// This reducer as an [`Origin`] — its learned id on this host. Before birth is folded the id is not
    /// yet known; fall back to the program hash reinterpreted as an id (a stable, documented placeholder
    /// that birth-first ordering makes unreachable for a real recorded event).
    fn source(&self) -> Origin {
        Origin {
            reducer: self
                .me
                .unwrap_or_else(|| ReducerId::from_hash(self.program.hash())),
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
            Err(e) => (bytes::Bytes::new(), Some(*e)),
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
            continuation_token: bytes::Bytes::new(),
            payload: notification.payload.clone(),
            error: None,
        });
        let (requests, outcome) = self.inner.on_notification(notification).await;
        self.record_output(&requests, &outcome);
        (requests, outcome)
    }
}

/// A [`ProgramStore`] that wraps every reducer it instantiates in a [`RecordingReducer`], so every fold
/// in the system is recorded to one [`ObservationLog`]. Hand this to [`TaskSystem::new`] in place of the
/// real program store and the whole run is observed — no kernel change.
///
/// [`TaskSystem::new`]: cdz_platform::TaskSystem::new
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
    async fn spawn(&self, program: ProgramHash) -> Option<Box<dyn Reducer>> {
        let inner = self.inner.spawn(program).await?;
        Some(Box::new(RecordingReducer::new(
            inner,
            program,
            self.host,
            self.log.clone(),
            self.now,
        )))
    }

    async fn contains(&self, program: ProgramHash) -> bool {
        self.inner.contains(program).await
    }
}
