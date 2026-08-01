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

/// A REAL local shell executor (feature `live-exec`) — the first executor that touches the world.
/// Runs an `EffectKind::Shell` request's `target` as a command line via `std::process::Command`,
/// returning the outcome the kernel folds back:
/// - exit 0 → `Ok(Some(stdout))`
/// - non-zero exit → `Err("exit <code>: <stderr>")`
/// - spawn failure → `Err`.
///
/// **Trust boundary:** this executor does NOT re-authorize. The kernel has already gated the effect's
/// resolved `target` against a resource-scoped capability (SEC-F1) before dispatching, so by the time a
/// request reaches here it is permitted. A `ShellExecutor` should only ever be handed effects for a
/// session whose capability constrains `Shell` targets (e.g. a command allow-list `Prefix`), never an
/// `Any` shell grant.
///
/// **Non-Shell effects** return an `Err` (in v0 a single executor handles one kind; the composite
/// router that dispatches by kind — WASI vs. model vs. peer — lands with the wasm host). **Idempotency
/// (§16c-S1):** a shell command is NOT generally idempotent, so a real deployment must dedup re-driven
/// dispatches on `idempotency_key`; v0's ShellExecutor executes unconditionally (single-node, no
/// crash-retry loop yet) and documents the key as the dedup handle for when retry lands.
#[cfg(feature = "live-exec")]
pub struct ShellExecutor;

#[cfg(feature = "live-exec")]
impl Executor for ShellExecutor {
    fn perform(&mut self, req: &EffectRequest, _idempotency_key: Hash) -> EffectOutcome {
        use crate::effect::EffectKind;
        if req.kind != EffectKind::Shell {
            return EffectOutcome::Err(format!(
                "ShellExecutor only handles Shell effects, got {:?}",
                req.kind
            ));
        }
        // Run via `sh -c <target>` so the target is a normal command line. The target was
        // capability-gated upstream (SEC-F1) — this executor trusts that authorization.
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(&req.target)
            .output();
        match output {
            Ok(out) if out.status.success() => EffectOutcome::Ok(Some(Payload::Inline(out.stdout))),
            Ok(out) => EffectOutcome::Err(format!(
                "exit {}: {}",
                out.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&out.stderr).trim()
            )),
            Err(e) => EffectOutcome::Err(format!("spawn failed: {e}")),
        }
    }
}
