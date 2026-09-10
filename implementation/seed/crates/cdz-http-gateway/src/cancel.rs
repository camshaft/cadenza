//! A cancellation scope for the gateway's spawned effect tasks.
//!
//! [`CancelScope`] holds the set of in-flight futures a session spawned (a dispatched child drive, a deadline
//! timer) and aborts the ones still running when it is dropped. [`wrap`](CancelScope::wrap) takes a future and
//! returns a future to run: spawn the returned future however you like (the scope never spawns anything), and
//! the scope can cancel it later. The returned future removes itself from the scope once it settles, so a
//! scope that wraps many futures over its lifetime holds only the ones still running (no unbounded growth).
//!
//! Cancellation is a plain-futures concern — each wrapped future is an [`abortable`](futures_util::future::abortable),
//! independent of how or where it runs. This is a small gateway-local utility (the operator chose gateway-side
//! cancellation over widening `cdz-platform`'s API surface, keeping the shared platform surface minimal); it
//! mirrors `cdz-platform`'s internal `CancelScope`, which the reducer loop uses for the same purpose.

use futures_util::future::AbortHandle;
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// A set of running futures aborted when the scope is dropped. Cheap to construct; cancel a group of related
/// tasks by wrapping each through the same scope and dropping it to abort whatever is still running.
#[derive(Default)]
pub struct CancelScope {
    /// The futures still running, keyed by a per-scope sequence number so each removes exactly its own entry
    /// when it settles.
    running: Arc<Mutex<HashMap<u64, AbortHandle>>>,
    /// The next wrapped future's key.
    next: AtomicU64,
}

impl CancelScope {
    /// An empty scope — nothing wrapped yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Wrap `future` so this scope can cancel it, returning the future to run. Run it however you like (the
    /// scope spawns nothing); if the scope is dropped before it settles, it is aborted at its next poll. The
    /// returned future removes itself from the scope once it settles — whether it completed or was aborted — so
    /// the scope holds only futures still running.
    pub fn wrap<F>(&self, future: F) -> impl Future<Output = ()> + Send + 'static
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

    impl CancelScope {
        fn pending(&self) -> usize {
            self.running.lock().expect("cancel-scope lock").len()
        }
    }

    #[tokio::test]
    async fn dropping_the_scope_aborts_a_still_running_future() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let scope = CancelScope::new();
        // A future that would report after 50ms — but the scope is dropped first, so it never does.
        let task = scope.wrap(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let _ = tx.send(());
        });
        tokio::spawn(task);
        drop(scope);
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        assert!(
            rx.try_recv().is_err(),
            "the wrapped future was aborted when the scope dropped"
        );
    }

    #[tokio::test]
    async fn a_completed_future_removes_itself_from_the_scope() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let scope = CancelScope::new();
        let task = scope.wrap(async move {
            let _ = tx.send(());
        });
        assert_eq!(scope.pending(), 1, "wrapped, not yet run");
        tokio::spawn(task);
        assert_eq!(rx.recv().await, Some(()));
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        assert_eq!(scope.pending(), 0, "a settled future leaves the scope");
    }
}
