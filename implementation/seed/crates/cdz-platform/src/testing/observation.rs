//! The observation log — what happened during a platform run, recorded for a checker to assert over
//! (`design/cadenza-platform.md` §9).
//!
//! The design calls this an *observation log*: a deployment may record every event a reducer folds,
//! for inspection or analysis, and it does not back a reducer's state, so recording it changes only
//! what can be audited, never how a reducer behaves. This crate makes that log concrete for tests.
//!
//! The log is one ordered sequence of [`Record`]s. Each record answers three questions and nothing
//! else: **when** it happened ([`Record::time_ns`], the runtime clock — deterministic under the bach
//! simulator), **who** caused it ([`Record::source`], a reducer on a host — an [`Origin`]), and
//! **what** happened ([`Record::entry`]). The `what` is a platform-level fact observable of *any*
//! reducer — a key-value or blob store call today, a delivered or emitted event in a later slice —
//! never a program's internal shape. A checker reads only these facts, so it never assumes a program
//! is Cadenza, Rust, or anything: the log is language-neutral, which is the whole point of the harness.
//!
//! The log is a cheaply-clonable handle over shared, append-only state. Many recording stores (one per
//! reducer) share the one handle, so the log is a single global order across every reducer — the
//! [`Record::seq`] is that order, assigned as each record is appended. A checker takes a
//! [`snapshot`](ObservationLog::snapshot) of the records once the run is quiescent.

use crate::{Bytes, ContractId, Error, Hash, Origin};
use std::ops::Bound;
use std::sync::{Arc, Mutex};

/// One entry in the observation log: who did what, and when. Cheap to clone — every buffer it carries
/// is [`Bytes`] (an O(1) refcount bump), so a checker snapshots the whole log without deep-copying.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// This record's position in the one global order of observations, assigned on append (the first
    /// record is `0`). It is the total order across every reducer sharing the log, so a checker can
    /// reason about "before" and "after" without comparing timestamps (which can tie).
    pub seq: u64,
    /// The runtime clock, in nanoseconds, at the moment the operation was recorded. Wall-clock under
    /// the production runtime; deterministic simulated time under the bach simulator, so a test folds
    /// the same times every run. Timestamps may tie (two ops at the same simulated instant); use
    /// [`seq`](Record::seq) for a strict order.
    pub time_ns: u64,
    /// Who caused this: the acting reducer and the host it ran on. For a store call it is the reducer
    /// whose store was touched; for an event (a later slice) it is the event's emitter.
    pub source: Origin,
    /// What happened.
    pub entry: Entry,
}

/// What a [`Record`] observed. A closed set of platform-level facts — the operations the platform
/// itself mediates for a reducer, observable without decoding a payload or knowing the program's
/// language. Key-value and blob store calls are here now; delivered and emitted events arrive with the
/// event tap (a later slice), extending this enum without changing what is already recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// A call to the reducer's key-value store (§7).
    Kv(KvOp),
    /// A call to the content-addressed blob store (§8).
    Blob(BlobOp),
    /// An event the reducer folded, emitted, or closed with (§3/§4/§10).
    Event(EventOp),
}

/// A key-value store operation, with enough of its outcome to assert over — whether a `get` hit, what
/// a `put` wrote, whether a `delete` found anything (`design/cadenza-platform.md` §7). Keys and values
/// are the raw [`Bytes`] the reducer used; the checker interprets them, the log does not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KvOp {
    /// `get(key)` — `hit` is whether the store held an entry for `key`.
    Get { key: Bytes, hit: bool },
    /// `put(key, value)` — the value written, last-write-wins.
    Put { key: Bytes, value: Bytes },
    /// `delete(key)` — `existed` is whether an entry was present and is now gone.
    Delete { key: Bytes, existed: bool },
    /// A range or prefix scan — its inclusive/exclusive bounds, and whether it was the keys-only
    /// variant. The scan *invocation* is recorded (who scanned what range, when), not each streamed
    /// pair: a scan is a lazy stream a reducer may drain partially, so the range is the observable
    /// fact; a checker that needs the contents reads the store's own state.
    Scan {
        lower: Bound<Bytes>,
        upper: Bound<Bytes>,
        keys_only: bool,
    },
}

/// A content-addressed blob store operation, with its outcome (`design/cadenza-platform.md` §8). The
/// hash is the content address; `put` records the hash it minted and the byte length stored (not the
/// bytes again — they are addressed by the hash), `get`/`has` record whether the hash was present.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlobOp {
    /// `put(bytes)` — the content hash returned and the number of bytes stored under it.
    Put { hash: Hash, len: usize },
    /// `get(hash)` — `hit` is whether the store held bytes for `hash`.
    Get { hash: Hash, hit: bool },
    /// `has(hash)` — `present` is the answer.
    Has { hash: Hash, present: bool },
}

/// Which entry point folded a delivered event (`design/cadenza-platform.md` §3) — the observable
/// distinction the runtime routes on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// `on_message` — an effect performed on the reducer, carrying its source `Origin`.
    Message,
    /// `on_response` — a reply to a request the reducer performed.
    Response,
    /// `on_notification` — an unsolicited control-plane event (birth, a lifecycle event, a new handler).
    Notification,
}

/// An event observed at a reducer (`design/cadenza-platform.md` §3/§4/§10). The three things a fold does
/// that are observable at the reducer boundary: an event is delivered into it, it emits requests, and it
/// may close itself. These are exactly what the design's observation log records — the event vocabulary
/// of §10, seen without decoding a payload or knowing the program's language. The reducer this happened
/// at is the enclosing [`Record::source`]; every buffer is [`Bytes`] (O(1) clone).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventOp {
    /// An event delivered into the reducer's mailbox and folded by it. `kind` is the entry point;
    /// `contract` is the event's contract-id. `from` is the emitter's `Origin` — present only for a
    /// message (a response's or notification's source is the runtime). `continuation_token` correlates a
    /// message/response (empty for a notification). `payload` is the delivered value, or a response's
    /// `Ok` bytes; `error` is `Some` for a response that delivered a runtime failure (`Err`) instead.
    Delivered {
        kind: EventKind,
        contract: ContractId,
        from: Option<Origin>,
        continuation_token: Bytes,
        payload: Bytes,
        error: Option<Error>,
    },
    /// A request the reducer emitted from a fold — an effect it performs, a timer it arms, or (for an
    /// event reducer) a deliver it routes. `contract` is the request's contract-id, `payload` its input
    /// value, `continuation_token` the token it will correlate the response by, and `has_deadline`
    /// whether it set a per-request deadline.
    Emitted {
        contract: ContractId,
        payload: Bytes,
        continuation_token: Bytes,
        has_deadline: bool,
    },
    /// The reducer terminated itself, returning `Break(schema, reason)` — the typed reason for its
    /// closure (§3). A clean completion and a failure are both closes, distinguished only by the reason.
    Closed { schema: ContractId, reason: Bytes },
}

/// The observation log: a cheaply-clonable handle over shared, append-only records. Clone it to hand
/// the same log to several recording stores (each clone shares the one underlying buffer), then
/// [`snapshot`](ObservationLog::snapshot) it once the run is quiescent to read what happened.
///
/// The shared buffer is behind a [`std::sync::Mutex`], not a runtime-specific lock, so the log is
/// runtime-agnostic: it records identically whether the platform runs on tokio or under the bach
/// simulator. A record append is a short critical section (assign a sequence number, push), never
/// held across an await, so it does not stall the async runtime.
#[derive(Clone, Default)]
pub struct ObservationLog {
    records: Arc<Mutex<Vec<Record>>>,
}

impl ObservationLog {
    /// A fresh, empty log.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append an observation: stamp it with the next sequence number and store it. Returns the `seq`
    /// assigned. Called by the recording stores; a checker never appends.
    pub fn record(&self, time_ns: u64, source: Origin, entry: Entry) -> u64 {
        let mut records = self.lock();
        let seq = records.len() as u64;
        records.push(Record {
            seq,
            time_ns,
            source,
            entry,
        });
        seq
    }

    /// A snapshot of every record so far, in order — what a checker reads. Each record clones cheaply
    /// (its buffers are `Bytes`), and the snapshot is detached from the log, so the run may continue
    /// (or the log drop) without affecting it.
    #[must_use]
    pub fn snapshot(&self) -> Vec<Record> {
        self.lock().clone()
    }

    /// The number of records logged so far.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Whether nothing has been recorded yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Lock the shared buffer. A poisoned lock means a recorder panicked mid-append, which a test run
    /// treats as a failed run — recover the buffer rather than cascading the poison, since the records
    /// already appended are still the truth of what happened.
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Record>> {
        self.records
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
