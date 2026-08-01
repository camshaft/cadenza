//! Executors — the things that actually perform effects (§2, §12).
//!
//! The kernel authorizes an effect, appends a durable `Dispatched` record, then hands the request to
//! an executor. Executors are uniform (§2): local WASI (shell/http), model invocation, a peer inbox, a
//! remote node — the kernel doesn't distinguish, it just routes. In v0 this is a trait with in-memory
//! test impls; real WASI/model executors land next.
//!
//! Idempotency (§16c-S1/D): an executor receives the dispatch's `idempotency_key`. For a
//! side-effecting executor, re-driving the same key after a crash must not double-apply — the executor
//! dedups on it. Naturally-idempotent executors can ignore it.

use crate::effect::{EffectRequest, Payload};
use crate::event::EffectOutcome;
use crate::hash::Hash;

/// Performs effects. Synchronous in v0 (the loop is single-threaded and deterministic to test); the
/// async/remote dispatch path is a later layer that preserves this contract.
pub trait Executor {
    /// Perform `req`. `idempotency_key` lets a side-effecting executor dedup a re-driven dispatch after
    /// a crash. Returns the outcome the kernel will fold back as an `EffectResult`.
    fn perform(&mut self, req: &EffectRequest, idempotency_key: Hash) -> EffectOutcome;
}

/// A recording test executor: performs nothing real, returns a canned outcome, and logs what it saw
/// (including whether an idempotency key was replayed). Lets kernel-loop tests assert the crash /
/// no-double-fire behavior deterministically.
#[derive(Default)]
pub struct RecordingExecutor {
    pub seen: Vec<(EffectRequest, Hash)>,
}

impl RecordingExecutor {
    pub fn new() -> Self {
        RecordingExecutor { seen: Vec::new() }
    }

    /// How many times a given idempotency key was performed — the double-fire detector for tests.
    pub fn times_performed(&self, key: Hash) -> usize {
        self.seen.iter().filter(|(_, k)| *k == key).count()
    }
}

impl Executor for RecordingExecutor {
    fn perform(&mut self, req: &EffectRequest, idempotency_key: Hash) -> EffectOutcome {
        self.seen.push((req.clone(), idempotency_key));
        EffectOutcome::Ok(Some(Payload::Inline(b"ok".to_vec())))
    }
}
