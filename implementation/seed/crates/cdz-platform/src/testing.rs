//! Test-support and the integration-test harness (`design/cadenza-platform.md` §9).
//!
//! Swappable pieces for exercising the platform in tests, plus the observation log and the run-to-
//! quiescence driver an integration test is built from. None of it is part of the production surface —
//! it is behind the `testing` feature (on automatically under `cfg(test)`); enable that feature to reach
//! it from another crate (the integration-test binary layers over this).
//!
//! The pieces:
//! - [`program`] — a [`ProgramStore`](crate::ProgramStore) backed by registered Rust factories, for
//!   wiring native reducer fixtures into the runtime without the CAS-plus-wasm loader.
//! - [`ObservationLog`] — the design's observation log (§9) made concrete: one ordered, cheaply-clonable
//!   log of [`Record`]s, each answering who ([`Origin`](crate::Origin)) / what ([`Entry`]) / when (the
//!   runtime clock). A checker reads only these platform-level facts, so it never assumes a program's
//!   language — the log is language-neutral, the harness's core contract.
//! - [`RecordingKvStore`] / [`RecordingBlobStore`] — decorators over the swappable store backends (§7/§8)
//!   that record every call, then defer to the wrapped backend unchanged.
//! - [`RecordingReducer`] / [`RecordingProgramStore`] — the event tap: wrap the program store the kernel
//!   instantiates reducers through, and every reducer records the events it folds, emits, and closes with
//!   (§3/§4/§10) — the whole system's event flow, with no change to the kernel.
//! - [`Harness`] — the run-to-quiescence driver: spawn a named reducer set, deliver initial events, drive
//!   the platform under the bach simulator to quiescence deterministically, and return the [`Run`] (the
//!   log plus the name→id assignment) for a checker.
//! - [`HarnessSpec`] — a whole run decoded from a **Cadenza binary AST**: the executable's input is one
//!   Cadenza value describing the blobs (inline or by path), the tasks to spawn, and the system reducer, so
//!   the harness is driven by a serializable value rather than a command line.
//! - [`serialize_log`] / [`deserialize_log`] — the observation log as a **Cadenza value**: the full-fidelity,
//!   language-neutral log a checker reads (§9), where the human [`render`] is lossy. Every id, token, and
//!   payload crosses byte-exact, so a checker can assert on values the rendered text elides; symmetric with
//!   the harness input, which is itself a Cadenza value ([`HarnessSpec`]).
//! - [`check_contract`] / [`verdict_contract`] — the checker **protocol** contracts (§9): the operator's
//!   design is that the end-of-run checker is a reducer-shaped guest, delivered the whole log via a `check`
//!   message and emitting a `verdict` request. These are the two contract ids that carry that protocol
//!   (real Cadenza schemas, `contracts/{check,verdict}.cdz`).
//! - [`Checker`] / [`CheckOutcome`] — the assertion side: read a [`Run`] and decide pass/fail. The native
//!   realization of the checker contract (a wasm checker implements the same judgement over a serialized
//!   log later); any `Fn(&Run) -> CheckOutcome` is a checker.

mod checker;
mod checker_protocol;
mod harness;
mod log_value;
mod observation;
mod recording;
mod spec;

/// The [`program`](crate::program) module's test helpers — `program::Store`, a program store backed by
/// registered Rust factories instead of the content-addressed store.
pub use crate::program::testing as program;
pub use checker::{CheckOutcome, Checker};
pub use checker_protocol::{check_contract, verdict_contract};
pub use harness::{Harness, Parent, Run, SpawnSpec};
pub use log_value::{deserialize as deserialize_log, serialize as serialize_log};
pub use observation::{
    BlobOp, Entry, EventKind, EventOp, KvOp, ObservationLog, Record, SpawnInfo, render,
};
pub use recording::{
    RecordingBlobStore, RecordingKvStore, RecordingProgramStore, RecordingReducer,
};
pub use spec::{BlobSource, BlobSpec, DeliveryEvent, HarnessSpec, SpecError};

#[cfg(test)]
mod tests;
