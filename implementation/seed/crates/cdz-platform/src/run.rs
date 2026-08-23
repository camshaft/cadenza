//! Running a program once, as a pure function (`design/cadenza-platform.md` §3).
//!
//! `run(program-hash, contract, input) -> output` is the kernel's own primitive — not a userspace reducer —
//! because only the kernel can make it *pure*, and purity is what makes it cacheable. It is shaped like
//! spawn + one addressed request + terminate, but with two things a spawn does not fix:
//!
//! - **No capabilities.** The program runs with an empty capability set — every effect it attempts is
//!   denied. Here that is literal: the emitted requests are dropped (never routed, so no response ever
//!   folds), so all a program can do is *return* its output — a `Break` whose reason is the output value —
//!   not read the world, message a peer, arm a timer, or write durable state.
//! - **Null birth.** Its id, parent, spawn-nonce, and host are canonical-null and identical on every
//!   invocation (there is no `spawned` notification, and the delivered message's `from` is a fixed null
//!   [`Origin`]), so the program cannot vary on *who* ran it, *where*, or *when* — there is nothing to
//!   observe but the input.
//!
//! With no capabilities and no birth to observe, and a reducer being deterministic, the output is a pure
//! function of `(program-hash, input)`, so a [`Runner`] **memoizes** it: an LRU keyed by
//! `(program-hash, input-hash) -> output`, and a hit skips execution entirely. Because the birth is nulled
//! too, the result does not depend on which node ran it — the cache is content-addressed and
//! node-independent. The first use is validation (§4): compile a schema to a validator and check a payload,
//! both as pure runs.

use crate::{
    Bytes, ContractId, Hash, HashTag, HostId, Message, Origin, Outcome, ProgramHash, ProgramStore,
    ReducerId, ReducerKind, SpawnContext,
};
use futures_util::FutureExt; // catch_unwind, to turn a fold trap into a RunError rather than unwinding
use std::collections::HashMap;
use std::sync::Mutex;

/// Why a pure [`run`](Runner::run) did not produce an output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunError {
    /// The store does not hold (or cannot instantiate) the program.
    UnknownProgram,
    /// The program folded without terminating — it returned `Continue` (a pure run has no capabilities, so
    /// awaiting an effect can never make progress) instead of a `Break` carrying its output.
    DidNotReturn,
    /// The fold trapped, exhausted its fuel, or otherwise failed to return — an uncontrolled fold-failure.
    Faulted,
}

/// The pure-function runner (`design/cadenza-platform.md` §3): instantiates a program with no capabilities
/// and a null birth, folds one input, returns the output, and memoizes the result. Holds the program store
/// it instantiates through and an LRU cache keyed by `(program-hash, input-hash)`.
pub struct Runner<P: ProgramStore + ?Sized> {
    programs: std::sync::Arc<P>,
    cache: Mutex<Cache>,
}

impl<P: ProgramStore + ?Sized> Runner<P> {
    /// The default memo capacity — the number of distinct `(program, input)` results kept before the
    /// least-recently-used is evicted.
    pub const DEFAULT_CAPACITY: usize = 1024;

    /// A runner over `programs` with the [default memo capacity](Runner::DEFAULT_CAPACITY).
    #[must_use]
    pub fn new(programs: std::sync::Arc<P>) -> Self {
        Self::with_capacity(programs, Self::DEFAULT_CAPACITY)
    }

    /// A runner whose memo holds at most `capacity` results (`capacity` of 0 disables memoization —
    /// every run executes).
    #[must_use]
    pub fn with_capacity(programs: std::sync::Arc<P>, capacity: usize) -> Self {
        Self {
            programs,
            cache: Mutex::new(Cache::new(capacity)),
        }
    }

    /// Run `program` once as a pure function of `input` against `contract`, returning the output value (the
    /// program's `Break` reason) or a [`RunError`]. A repeated `(program, input)` returns the memoized
    /// output without re-executing. Deterministic and node-independent: the birth is nulled, so the result
    /// depends only on `(program-hash, input)`.
    ///
    /// `contract` is the contract-id the input is delivered against (the message's `id`); it is not part of
    /// the memo key, because a program is a pure function of its input and the contract is fixed by the
    /// program's declared answer — the same convention `run(program-hash, contract, input)` uses in §3.
    pub async fn run(
        &self,
        program: ProgramHash,
        contract: ContractId,
        input: Bytes,
    ) -> Result<Bytes, RunError> {
        let key = (program, Hash::of(HashTag::Blob, &input));
        // Memo hit — drop the lock before any await (never hold a std Mutex across `.await`).
        if let Some(output) = self.cache.lock().expect("run memo lock").get(&key) {
            return Ok(output);
        }

        // Null birth: a canonical-null id and kind (Ordinary — its capability set is empty regardless, as
        // no effect it emits is routed), identical on every invocation.
        let mut reducer = self
            .programs
            .spawn(
                program,
                SpawnContext {
                    id: null_run_id(),
                    kind: ReducerKind::Ordinary,
                },
            )
            .await
            .ok_or(RunError::UnknownProgram)?;

        // Deliver the input as one addressed message with a null origin and no continuation token, and
        // capture a fold trap as a RunError rather than unwinding (§3 fold-failure).
        let message = Message {
            id: contract,
            payload: input,
            from: null_origin(),
            continuation_token: Bytes::new(),
        };
        let folded = std::panic::AssertUnwindSafe(reducer.on_message(message))
            .catch_unwind()
            .await;

        // Every effect is denied: the emitted requests are dropped (never routed), so the only output is the
        // fold's result — a `Break` whose reason is the output value. A `Continue` means the program tried to
        // await an effect (which can never answer here) instead of returning, so it produced no output.
        let output = match folded {
            Ok((_denied_requests, Outcome::Break { reason, .. })) => reason,
            Ok((_requests, Outcome::Continue)) => return Err(RunError::DidNotReturn),
            Err(_panic) => return Err(RunError::Faulted),
        };
        self.cache
            .lock()
            .expect("run memo lock")
            .put(key, output.clone());
        Ok(output)
    }
}

/// The canonical-null reducer-id every pure run is born under — fixed, so a run cannot observe *who* ran it.
fn null_run_id() -> ReducerId {
    ReducerId::of(b"cdz-platform.run.null")
}

/// The canonical-null origin stamped on the delivered input — fixed reducer and host, so a run cannot
/// observe *where* it ran.
fn null_origin() -> Origin {
    Origin {
        reducer: null_run_id(),
        host: HostId::of(b"cdz-platform.run.null"),
    }
}

/// A bounded LRU cache of pure-run outputs, keyed by `(program-hash, input-hash)`. `order` holds keys
/// least-recently-used first; a `get`/`put` moves the key to the most-recently-used end, and an insert over
/// capacity evicts the front. Small and lock-guarded — a memo, not a hot path.
struct Cache {
    map: HashMap<(ProgramHash, Hash), Bytes>,
    order: Vec<(ProgramHash, Hash)>,
    capacity: usize,
}

impl Cache {
    fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: Vec::new(),
            capacity,
        }
    }

    fn get(&mut self, key: &(ProgramHash, Hash)) -> Option<Bytes> {
        let value = self.map.get(key)?.clone();
        self.touch(key);
        Some(value)
    }

    /// Move `key` to the most-recently-used end of the order (it is known present).
    fn touch(&mut self, key: &(ProgramHash, Hash)) {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            let key = self.order.remove(pos);
            self.order.push(key);
        }
    }

    fn put(&mut self, key: (ProgramHash, Hash), value: Bytes) {
        if self.capacity == 0 {
            return; // memoization disabled
        }
        if self.map.insert(key, value).is_none() {
            self.order.push(key);
            if self.order.len() > self.capacity {
                let evicted = self.order.remove(0);
                self.map.remove(&evicted);
            }
        } else {
            self.touch(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RunError, Runner};
    use crate::{
        Bytes, ContractId, Message, Notification, Outcome, ProgramHash, Reducer, Request, Response,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn cid(tag: &[u8]) -> ContractId {
        ContractId::of(tag)
    }
    fn prog(tag: &[u8]) -> ProgramHash {
        ProgramHash::of(tag)
    }

    /// A reducer that, on its message, returns `Break` whose reason is the input with a fixed suffix — a
    /// pure transform of the input into an output.
    struct Doubler;
    #[async_trait::async_trait]
    impl Reducer for Doubler {
        async fn on_message(&mut self, m: Message) -> (Vec<Request>, Outcome) {
            let mut out = m.payload.to_vec();
            out.extend_from_slice(&m.payload); // output = input ++ input
            (
                Vec::new(),
                Outcome::Break {
                    schema: cid(b"out"),
                    reason: Bytes::from(out),
                },
            )
        }
        async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
    }

    /// A reducer that never terminates on a message (emits an effect and Continues) — a program that tries
    /// to reach the world, which a pure run denies.
    struct Waiter;
    #[async_trait::async_trait]
    impl Reducer for Waiter {
        async fn on_message(&mut self, m: Message) -> (Vec<Request>, Outcome) {
            let req = Request {
                id: m.id,
                payload: m.payload,
                continuation_token: Bytes::from_static(b"k"),
                deadline: None,
            };
            (vec![req], Outcome::Continue)
        }
        async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
    }

    /// A reducer that traps on a message — an uncontrolled fold-failure.
    struct Trapper;
    #[async_trait::async_trait]
    impl Reducer for Trapper {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            panic!("trapper trapped");
        }
        async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
    }

    #[tokio::test]
    async fn run_returns_the_programs_output() {
        let mut store = crate::testing::program::Store::new();
        store.register(prog(b"doubler"), || Box::new(Doubler));
        let runner = Runner::new(Arc::new(store));
        let out = runner
            .run(prog(b"doubler"), cid(b"c"), Bytes::from_static(b"ab"))
            .await
            .expect("a pure output");
        assert_eq!(out, Bytes::from_static(b"abab"));
    }

    #[tokio::test]
    async fn a_repeated_run_is_memoized_and_skips_execution() {
        // A factory that counts how many reducer instances it builds; a memo hit must NOT instantiate again.
        let built = Arc::new(AtomicUsize::new(0));
        let built_f = built.clone();
        let mut store = crate::testing::program::Store::new();
        store.register(prog(b"doubler"), move || {
            built_f.fetch_add(1, Ordering::SeqCst);
            Box::new(Doubler)
        });
        let runner = Runner::new(Arc::new(store));

        let first = runner
            .run(prog(b"doubler"), cid(b"c"), Bytes::from_static(b"x"))
            .await
            .unwrap();
        let second = runner
            .run(prog(b"doubler"), cid(b"c"), Bytes::from_static(b"x"))
            .await
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(
            built.load(Ordering::SeqCst),
            1,
            "the second run hit the memo and did not instantiate"
        );
        // A different input is a distinct key: it executes (instantiates) again.
        runner
            .run(prog(b"doubler"), cid(b"c"), Bytes::from_static(b"y"))
            .await
            .unwrap();
        assert_eq!(built.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_program_that_does_not_return_is_an_error() {
        let mut store = crate::testing::program::Store::new();
        store.register(prog(b"waiter"), || Box::new(Waiter));
        let runner = Runner::new(Arc::new(store));
        assert_eq!(
            runner
                .run(prog(b"waiter"), cid(b"c"), Bytes::from_static(b"in"))
                .await,
            Err(RunError::DidNotReturn)
        );
    }

    #[tokio::test]
    async fn a_trapping_program_is_a_faulted_run() {
        let mut store = crate::testing::program::Store::new();
        store.register(prog(b"trapper"), || Box::new(Trapper));
        let runner = Runner::new(Arc::new(store));
        assert_eq!(
            runner
                .run(prog(b"trapper"), cid(b"c"), Bytes::from_static(b"in"))
                .await,
            Err(RunError::Faulted)
        );
    }

    #[tokio::test]
    async fn an_unknown_program_is_an_error() {
        let store = crate::testing::program::Store::new(); // nothing registered
        let runner = Runner::new(Arc::new(store));
        assert_eq!(
            runner.run(prog(b"absent"), cid(b"c"), Bytes::new()).await,
            Err(RunError::UnknownProgram)
        );
    }

    #[tokio::test]
    async fn a_zero_capacity_runner_still_computes_but_does_not_memoize() {
        let built = Arc::new(AtomicUsize::new(0));
        let built_f = built.clone();
        let mut store = crate::testing::program::Store::new();
        store.register(prog(b"doubler"), move || {
            built_f.fetch_add(1, Ordering::SeqCst);
            Box::new(Doubler)
        });
        let runner = Runner::with_capacity(Arc::new(store), 0);
        for _ in 0..3 {
            runner
                .run(prog(b"doubler"), cid(b"c"), Bytes::from_static(b"x"))
                .await
                .unwrap();
        }
        assert_eq!(
            built.load(Ordering::SeqCst),
            3,
            "no memoization at capacity 0"
        );
    }
}
