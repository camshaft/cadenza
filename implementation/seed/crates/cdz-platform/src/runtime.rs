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
    /// A future that completes after `duration` on the runtime's clock — the one time primitive the system
    /// needs, so a `fire-after` effect can wake a reducer later (§6). Real time under tokio; simulated,
    /// deterministic time under bach.
    fn sleep(duration: Duration) -> impl Future<Output = ()> + Send;
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
}
