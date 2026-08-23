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
//! - [`Harness`] — the run-to-quiescence driver: spawn a reducer set, deliver initial events, drive the
//!   platform under the bach simulator to quiescence deterministically, and return the log for a checker.

mod harness;
mod observation;
mod recording;

/// The [`program`](crate::program) module's test helpers — `program::Store`, a program store backed by
/// registered Rust factories instead of the content-addressed store.
pub use crate::program::testing as program;
pub use harness::Harness;
pub use observation::{BlobOp, Entry, EventKind, EventOp, KvOp, ObservationLog, Record};
pub use recording::{
    RecordingBlobStore, RecordingKvStore, RecordingProgramStore, RecordingReducer,
};

#[cfg(test)]
mod tests;
