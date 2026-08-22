//! A cancellation scope for spawned futures.
//!
//! [`CancelScope`] is a generic utility — it knows nothing about timers, reducers, or any runtime. It holds a
//! set of in-flight futures and cancels the ones still running when it is dropped. [`wrap`](CancelScope::wrap)
//! takes a future and returns a future to run: spawn the returned future however you like (the scope never
//! spawns anything itself), and the scope can cancel it later. The returned future removes itself from the
//! scope once it settles, so a scope that wraps many futures over its lifetime holds only the ones still
//! running and does not grow without bound.
//!
//! Cancellation is a plain-futures concern: each wrapped future is an [`abortable`](futures_util::future::abortable),
//! so nothing here depends on how or where the future is executed.

use futures_util::future::AbortHandle;
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// A set of running futures that are cancelled when the scope is dropped. Cheap to construct; share the work
/// of cancelling a group of related tasks by wrapping each through the same scope and dropping it to cancel
/// whatever is left.
#[derive(Default)]
pub(crate) struct CancelScope {
    /// The futures still running, keyed by a per-scope sequence number so each can remove exactly its own
    /// entry when it settles.
    running: Arc<Mutex<HashMap<u64, AbortHandle>>>,
    /// The next wrapped future's key.
    next: AtomicU64,
}

impl CancelScope {
    /// An empty scope — nothing wrapped yet.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Wrap `future` so this scope can cancel it, returning the future to run. Run it however you like (the
    /// scope spawns nothing); if the scope is dropped before it settles, it is aborted at its next poll. The
    /// returned future removes itself from the scope once it settles — whether it ran to completion or was
    /// aborted — so the scope holds only futures still running.
    pub(crate) fn wrap<F>(&self, future: F) -> impl Future<Output = ()> + Send + 'static
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let key = self.next.fetch_add(1, Ordering::Relaxed);
        let (task, cancel) = futures_util::future::abortable(future);
        self.running
            .lock()
            .expect("cancel-scope lock")
            .insert(key, cancel);
        let running = Arc::clone(&self.running);
        async move {
            // Run to completion — or to `Err(Aborted)` if the scope was dropped first — then drop this entry
            // so the scope does not accumulate settled futures.
            let _ = task.await;
            running.lock().expect("cancel-scope lock").remove(&key);
        }
    }
}

impl Drop for CancelScope {
    fn drop(&mut self) {
        for cancel in self.running.lock().expect("cancel-scope lock").values() {
            cancel.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CancelScope;
    use core::time::Duration;

    /// Test-only: how many wrapped futures are still running.
    impl CancelScope {
        fn pending(&self) -> usize {
            self.running.lock().expect("cancel-scope lock").len()
        }
    }

    #[test]
    fn dropping_the_scope_cancels_a_still_running_future() {
        use bach::ext::*;
        bach::sim(|| {
            async {
                let (tx, mut rx) = bach::sync::mpsc::unbounded_channel();
                let scope = CancelScope::new();
                // A future that would report after 50ms — but the scope is dropped first, so it never does.
                let task = scope.wrap(async move {
                    bach::time::sleep(Duration::from_millis(50)).await;
                    let _ = tx.send(());
                });
                bach::task::spawn(task);
                drop(scope);
                bach::time::sleep(Duration::from_millis(100)).await;
                assert!(
                    rx.try_recv().is_err(),
                    "the wrapped future was cancelled when the scope dropped"
                );
            }
            .group("cancel")
            .primary()
            .spawn();
        });
    }

    #[test]
    fn a_completed_future_removes_itself_from_the_scope() {
        use bach::ext::*;
        bach::sim(|| {
            async {
                let (tx, mut rx) = bach::sync::mpsc::unbounded_channel();
                let scope = CancelScope::new();
                let task = scope.wrap(async move {
                    let _ = tx.send(());
                });
                assert_eq!(scope.pending(), 1, "wrapped, not yet run");
                bach::task::spawn(task);
                // It runs to completion (reports), then removes itself from the scope.
                assert_eq!(rx.recv().await, Some(()));
                bach::time::sleep(Duration::from_millis(1)).await;
                assert_eq!(scope.pending(), 0, "a settled future leaves the scope");
            }
            .group("cancel")
            .primary()
            .spawn();
        });
    }
}
