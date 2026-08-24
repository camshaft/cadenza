//! The async runtime a task-based [`System`](crate::System) runs on (`design/cadenza-platform.md` §9).
//!
//! A [`Runtime`] is the small, static difference between the in-memory systems: spawning a task and a
//! mailbox channel. [`TokioRuntime`] in production, [`BachRuntime`] (the deterministic simulator) in tests.
//! It is a static trait (a generic bound, never `dyn`), so [`TaskSystem`](crate::TaskSystem) composes over
//! it with no dynamic dispatch and no duplicated logic. tokio and bach are runtimes; a durable-actor backend
//! is a whole [`System`](crate::System), not a `Runtime`.

use crate::Delivered;
use std::future::Future;
use std::time::Duration;

/// The async runtime a [`TaskSystem`](crate::TaskSystem) runs on: spawning a task and creating a reducer's
/// mailbox channel. Static, so the system composes over it with no dynamic dispatch.
pub trait Runtime: Send + Sync + 'static {
    /// A mailbox sender — cloneable so many peers can hold a handle to one reducer's mailbox.
    type Sender: Clone + Send + Sync + 'static;
    /// A mailbox receiver — the reducer's own end, drained by its loop.
    type Receiver: Send + 'static;

    /// Create a reducer's mailbox: an unbounded channel of delivered events.
    fn channel() -> (Self::Sender, Self::Receiver);
    /// Deliver an event into a mailbox. `false` if the receiving end is gone.
    fn send(sender: &Self::Sender, event: Delivered) -> bool;
    /// Receive the next delivered event, or `None` when the mailbox closes.
    fn recv(receiver: &mut Self::Receiver) -> impl Future<Output = Option<Delivered>> + Send + '_;
    /// Spawn `future` as a task on the runtime.
    fn spawn<F: Future<Output = ()> + Send + 'static>(future: F);
    /// A future that completes after `duration` on the runtime's clock — the wait a `fire-after` timer arms
    /// (§6). Real time under tokio; simulated, deterministic time under bach.
    fn sleep(duration: Duration) -> impl Future<Output = ()> + Send;
    /// The runtime's clock now, in nanoseconds. Stamped into a `Fired` event as the recorded fire time (§6),
    /// which a reducer folds without ever reading a clock itself. Real (wall-clock) time under tokio;
    /// simulated, deterministic time (since the simulation started) under bach — so a `fire-after` test folds
    /// the same recorded time every run. This is the runtime's own clock, distinct from the capability-gated
    /// `now` effect a program requests to learn the time.
    fn now() -> u64;

    /// Whether this runtime should drive the wasm engine's epoch ticker — the periodic
    /// [`increment_epoch`](wasmtime::Engine::increment_epoch) that makes a long-running guest fold yield to the
    /// executor and, past a bound, trap (so one runaway program cannot monopolize a thread or stall the
    /// runtime). True on the PRODUCTION runtime (`tokio`). False on `bach`: a periodic ticker is a
    /// forever-pending timer that would prevent the simulator ever reaching quiescence, so the deterministic
    /// path leaves the epoch un-advanced (guests run un-preempted under sim) and relies on the integration
    /// harness's own wall-clock timeout as the runaway backstop. Default false — only a real-time runtime opts
    /// in.
    fn drives_epoch_ticker() -> bool {
        false
    }
}

/// The production runtime: tokio tasks and channels.
pub struct TokioRuntime;

impl Runtime for TokioRuntime {
    type Sender = tokio::sync::mpsc::UnboundedSender<Delivered>;
    type Receiver = tokio::sync::mpsc::UnboundedReceiver<Delivered>;

    fn channel() -> (Self::Sender, Self::Receiver) {
        tokio::sync::mpsc::unbounded_channel()
    }
    fn send(sender: &Self::Sender, event: Delivered) -> bool {
        sender.send(event).is_ok()
    }
    fn recv(receiver: &mut Self::Receiver) -> impl Future<Output = Option<Delivered>> + Send + '_ {
        receiver.recv()
    }
    fn spawn<F: Future<Output = ()> + Send + 'static>(future: F) {
        tokio::spawn(future);
    }
    fn sleep(duration: Duration) -> impl Future<Output = ()> + Send {
        tokio::time::sleep(duration)
    }
    fn now() -> u64 {
        // Wall-clock nanoseconds since the Unix epoch, saturating (the epoch is always in the past, and u64
        // nanos covers year ~2554). A monotonic clock has no cross-node meaning; wall-clock is what a recorded
        // fire time means in production.
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
            .unwrap_or(0)
    }
    fn drives_epoch_ticker() -> bool {
        // Production: drive the epoch ticker so a runaway guest fold is preempted (yields, then traps past a
        // bound) rather than monopolizing a tokio worker thread.
        true
    }
}

/// The test runtime: bach-simulator tasks and channels (deterministic). Gated with the test-support code,
/// since bach only drives the reducers under test.
#[cfg(any(test, feature = "testing"))]
pub struct BachRuntime;

#[cfg(any(test, feature = "testing"))]
impl Runtime for BachRuntime {
    type Sender = bach::sync::mpsc::UnboundedSender<Delivered>;
    type Receiver = bach::sync::mpsc::UnboundedReceiver<Delivered>;

    fn channel() -> (Self::Sender, Self::Receiver) {
        bach::sync::mpsc::unbounded_channel()
    }
    fn send(sender: &Self::Sender, event: Delivered) -> bool {
        sender.send(event).is_ok()
    }
    fn recv(receiver: &mut Self::Receiver) -> impl Future<Output = Option<Delivered>> + Send + '_ {
        receiver.recv()
    }
    fn spawn<F: Future<Output = ()> + Send + 'static>(future: F) {
        bach::task::spawn(future);
    }
    fn sleep(duration: Duration) -> impl Future<Output = ()> + Send {
        bach::time::sleep(duration)
    }
    fn now() -> u64 {
        // Deterministic simulated time since the simulation started, in nanoseconds — so a `fire-after` test
        // folds the same recorded fire time every run.
        u64::try_from(bach::time::Instant::now().elapsed_since_start().as_nanos())
            .unwrap_or(u64::MAX)
    }
}
