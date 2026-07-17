//! `cdz-kernel` — the log-native agent-runtime microkernel (agent-runtime L1).
//!
//! The vision (`implementation/design/DESIGN-agent-runtime-vision.md`) is a log-native agent OS: a minimal
//! core that TAILS an ordered log, FOLDS it with a Cadenza program, and EXECUTES the effect-requests the
//! fold emits — appending every non-deterministic result (a model call, a clock, a build) back as an
//! immutable event, so the fold over `(request-event, response-event)` is pure and replayable (§2.3).
//!
//! This module (**L1a**) is the foundation: the [`Log`] abstraction and a deterministic file-backed
//! implementation. Later rungs build the fold owner on top (L1b), the replay-determinism gate (L1c), and a
//! DynamoDB backend behind the `aws` feature (L1d). The [`Log`] trait is the seam those rungs — and the
//! future DynamoDB write plane (§2.1: a many-writer ordering authority) — implement, so the fold owner is
//! written once against the trait and the backend swaps underneath.

use anyhow::Result;

/// A monotonic, gap-free sequence number assigned by the log on append — the total order the fold reads in.
/// The DynamoDB write plane (L1d) assigns this via a conditional write (the ordering + dedup authority,
/// vision §2.1); the file log assigns it by append position. `seq` starts at 0 for the first event.
pub type Seq = u64;

/// One immutable log event: its `seq` (assigned by the log), a `kind` tag (what the event IS — e.g. a model
/// request, a model response, a message), and an opaque `payload` (the event body; a later rung fixes the
/// concrete encoding — L1a treats it as bytes so the log is agnostic to the fold program's event schema).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub seq: Seq,
    pub kind: String,
    pub payload: Vec<u8>,
}

/// The append-only log: the single source of truth the whole runtime folds over. `append` adds an event and
/// returns its assigned `seq`; `tail` returns every event with `seq >= from` in order. Deliberately minimal
/// — a fold owner needs exactly "add an event" + "read the ordered tail from a cursor". The write plane is
/// decoupled from the fold plane (vision §2.2), so many writers may `append` concurrently while one owner
/// `tail`s; a backend enforces the ordering authority (the file impl is single-process, DynamoDB is the
/// multi-writer authority at L1d).
pub trait Log {
    /// Append an event with `kind` + `payload`, returning its assigned monotonic `seq`.
    fn append(&mut self, kind: &str, payload: &[u8]) -> Result<Seq>;

    /// Return every event with `seq >= from`, in ascending `seq` order. `tail(0)` returns the whole log.
    fn tail(&self, from: Seq) -> Result<Vec<Event>>;
}

/// A deterministic FILE-backed [`Log`] (the L1a backend — local, no network, CI-safe). Events are appended
/// as length-prefixed records to a single file, so the on-disk order IS the seq order and a fresh process
/// re-reads the same sequence. This is the stand-in for the DynamoDB log (L1d) while the fold owner + the
/// replay-determinism gate (L1b/L1c) are built against the [`Log`] trait — the same mock-first discipline
/// the Bedrock embedder shipped with.
pub mod file_log;
pub use file_log::FileLog;
