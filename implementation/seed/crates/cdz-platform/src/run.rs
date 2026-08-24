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
    Bytes, Contract, ContractId, Hash, HashTag, HostId, Message, Origin, Outcome, ProgramHash,
    ProgramStore, ReducerId, ReducerKind, Request, SpawnContext,
};
use cadenza_ast::ast::{Builder, StructId};
use cadenza_ast::codec;
use futures_util::FutureExt; // catch_unwind, to turn a fold trap into a RunError rather than unwinding
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

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

        // Null birth + empty capabilities: a canonical-null id and the `Pure` kind, so a wasm store wires no
        // host imports (§3 empty capability set) — the program cannot read the world or write durable state,
        // which is what makes its output a pure function of the input and its memoization sound. Identical on
        // every invocation.
        let reducer = self
            .programs
            .spawn(
                program,
                SpawnContext {
                    id: null_run_id(),
                    kind: ReducerKind::Pure,
                },
            )
            .await
            .ok_or(RunError::UnknownProgram)?;

        let output = drive_pure(reducer, contract, input).await?;
        self.cache
            .lock()
            .expect("run memo lock")
            .put(key, output.clone());
        Ok(output)
    }
}

/// Fold one input into a freshly-instantiated pure reducer and return its output — the pure-execution
/// semantics shared by both invocation forms of `run` (the [`Runner`] effect above and the synchronous
/// `run` host import). The input is delivered as one addressed [`Message`] with a null [`Origin`] and no
/// continuation token; a fold trap is caught as [`RunError::Faulted`] rather than unwinding (§3
/// fold-failure). Because a pure reducer has no capabilities, every effect it emits is denied — the requests
/// are dropped, never routed, so no response ever folds — and the only output is the fold's result: a
/// `Break` whose reason is the output value. A `Continue` means the program tried to await an effect (which
/// can never answer here) instead of returning, so it produced no output ([`RunError::DidNotReturn`]).
pub(crate) async fn drive_pure(
    mut reducer: Box<dyn crate::Reducer>,
    contract: ContractId,
    input: Bytes,
) -> Result<Bytes, RunError> {
    let message = Message {
        id: contract,
        payload: input,
        from: null_origin(),
        continuation_token: Bytes::new(),
    };
    let folded = std::panic::AssertUnwindSafe(reducer.on_message(message))
        .catch_unwind()
        .await;
    match folded {
        Ok((_denied_requests, Outcome::Break { reason, .. })) => Ok(reason),
        Ok((_requests, Outcome::Continue)) => Err(RunError::DidNotReturn),
        Err(_panic) => Err(RunError::Faulted),
    }
}

/// The canonical-null reducer-id every pure run is born under — fixed, so a run cannot observe *who* ran it.
/// Shared with the synchronous `run` host import so its sub-program has the same null birth as the effect form.
pub(crate) fn null_run_id() -> ReducerId {
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
/// capacity evicts the front. Small and lock-guarded — a memo, not a hot path. Shared by both invocation
/// forms of `run`: the [`Runner`] (the effect) holds one, and the synchronous `run` host import holds one on
/// the instantiation core (so a host's pure runs share a memo across the fold that called `run`).
pub(crate) struct Cache {
    map: HashMap<(ProgramHash, Hash), Bytes>,
    order: Vec<(ProgramHash, Hash)>,
    capacity: usize,
}

impl Cache {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: Vec::new(),
            capacity,
        }
    }

    /// The default memo capacity — the number of distinct `(program, input)` results kept before the
    /// least-recently-used is evicted (matches [`Runner::DEFAULT_CAPACITY`]). Used by the synchronous `run`
    /// host import's memo (behind the `host` feature; the [`Runner`] uses its own constant).
    #[cfg(feature = "host")]
    pub(crate) const DEFAULT_CAPACITY: usize = Runner::<dyn ProgramStore>::DEFAULT_CAPACITY;

    pub(crate) fn get(&mut self, key: &(ProgramHash, Hash)) -> Option<Bytes> {
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

    pub(crate) fn put(&mut self, key: (ProgramHash, Hash), value: Bytes) {
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

// ── The run contract (§3/§4): run as an EFFECT ──────────────────────────────────────────────────────────
// Alongside the direct [`Runner`] (which the synchronous host-call form uses), run is also an ordinary
// effect: a reducer emits a [`Run`] request against the [`run_contract`], and the output comes back as the
// response (a [`RunOutput`]), correlated by the request's own continuation-token — the async, event-mediated
// form (design §3). The value shape is the generated `crate::contracts::run` schema; the two hashes and the
// input/output cross as `Bytes`.

/// The run contract: a [`Request`](crate::Request) against it runs a program as a pure function (§3), and the
/// output returns as the response. Its id is the hash of the declared schema — the compiler-checked
/// [`crate::contracts::run`] module generated from `contracts/run.cdz` — built once and cached.
#[must_use]
pub fn run_contract() -> ContractId {
    static RUN: OnceLock<Contract> = OnceLock::new();
    RUN.get_or_init(crate::contracts::run::contract).id()
}

/// A run request (the run contract's input, §3): run `program` with `input`, delivered against `contract`.
/// This is what the payload of a run [`Request`](crate::Request) carries; the output comes back as the
/// response correlated by the request's standard continuation-token, so nothing else rides in the value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    /// The program to run, by content hash.
    pub program: ProgramHash,
    /// The contract-id `input` is delivered against (the contract the program answers).
    pub contract: ContractId,
    /// The input value, opaque bytes (the program decodes it against `contract`).
    pub input: Bytes,
}

/// The output of a run (the run contract's output, §3): the value the program returned, opaque bytes (the
/// caller decodes it against `contract`'s output type). Delivered back as the response to a [`Run`] request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunOutput {
    /// The run program's output value, opaque bytes.
    pub output: Bytes,
}

impl Run {
    /// Build the request value into `b` — a value of the schema type `Request` (`Run(Record …)`), so it
    /// type-ascribes against the contract's schema. The shape is entirely the generated builder's; this only
    /// supplies the two hash byte-leaves and the input.
    fn build(&self, b: &mut Builder) -> StructId {
        use crate::contract_value as v;
        use crate::contracts::run as c;
        let program = v::bytes_leaf(b, self.program.hash().as_bytes());
        let contract = v::bytes_leaf(b, self.contract.hash().as_bytes());
        let input = v::bytes_leaf(b, &self.input);
        c::request_run(
            b,
            c::RequestRun {
                program,
                contract,
                input,
            },
        )
    }

    /// Encode the request as a Cadenza value in the canonical binary form. The inverse of [`decode`](Self::decode).
    #[must_use]
    pub fn encode(&self) -> Bytes {
        let mut b = Builder::new();
        let value = self.build(&mut b);
        let root = crate::contract_value::ascribe(&mut b, value, "Request");
        Bytes::from(codec::encode(&b.finish(root)))
    }

    /// The [`Request`](crate::Request) a reducer emits to run `self` as an effect: against the
    /// [`run_contract`], with the request as its payload; the [`RunOutput`] comes back as the response
    /// carrying `continuation_token`. Carries no deadline (a caller adds one if it wants a bound).
    #[must_use]
    pub fn into_request(self, continuation_token: Bytes) -> Request {
        Request {
            id: run_contract(),
            payload: self.encode(),
            continuation_token,
            deadline: None,
        }
    }

    /// Decode a run request from a Cadenza value, or `None` if the bytes are not a well-formed `Run` value (a
    /// malformed hash leaf, wrong shape). Total — a malformed request is rejected, never a panic.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        use crate::contract_value as v;
        use crate::contracts::run as c;
        let arenas = codec::decode(bytes)?;
        let root = v::as_ascribed(&arenas, arenas.root)?;
        let r = c::as_request_run(&arenas, root)?;
        Some(Self {
            program: ProgramHash::try_from(v::read_bytes(&arenas, r.program)?.as_ref()).ok()?,
            contract: ContractId::try_from(v::read_bytes(&arenas, r.contract)?.as_ref()).ok()?,
            input: v::read_bytes(&arenas, r.input)?,
        })
    }
}

impl RunOutput {
    /// Encode the output as a Cadenza value (`Output(Bytes)`) in the canonical binary form — what the run
    /// effect's response payload carries. The inverse of [`decode`](Self::decode).
    #[must_use]
    pub fn encode(&self) -> Bytes {
        use crate::contract_value as v;
        use crate::contracts::run as c;
        let mut b = Builder::new();
        let output = v::bytes_leaf(&mut b, &self.output);
        let value = c::output_output(&mut b, output);
        let root = v::ascribe(&mut b, value, "Output");
        Bytes::from(codec::encode(&b.finish(root)))
    }

    /// Decode an output from a Cadenza value, or `None` if the bytes are not a well-formed `Output` value.
    /// Total.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        use crate::contract_value as v;
        use crate::contracts::run as c;
        let arenas = codec::decode(bytes)?;
        let root = v::as_ascribed(&arenas, arenas.root)?;
        let inner = c::as_output_output(&arenas, root)?;
        Some(Self {
            output: v::read_bytes(&arenas, inner)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Run, RunError, RunOutput, Runner, run_contract};
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

    #[test]
    fn a_run_request_round_trips_through_the_codec() {
        // The run effect's request: program/contract/input recovered exactly through encode/decode, and the
        // request is emitted against the run contract with the caller's continuation-token.
        let run = Run {
            program: prog(b"the-program"),
            contract: cid(b"the-contract"),
            input: Bytes::from_static(b"the-input"),
        };
        assert_eq!(Run::decode(&run.encode()), Some(run.clone()));
        let req = run.clone().into_request(Bytes::from_static(b"tok"));
        assert_eq!(req.id, run_contract());
        assert_eq!(req.continuation_token, Bytes::from_static(b"tok"));
        assert_eq!(Run::decode(&req.payload), Some(run));
        // A malformed hash leaf (wrong length) is a rejected request, not a panic.
        assert_eq!(Run::decode(b"not a run value"), None);
    }

    #[test]
    fn a_run_output_round_trips_through_the_codec() {
        let out = RunOutput {
            output: Bytes::from_static(b"the-result"),
        };
        assert_eq!(RunOutput::decode(&out.encode()), Some(out));
        assert_eq!(RunOutput::decode(b"not an output"), None);
    }

    #[test]
    fn the_run_contract_id_is_stable() {
        // The contract-id is the hash of the declared schema, derived once and stable across calls.
        assert_eq!(run_contract(), run_contract());
        assert_ne!(run_contract(), crate::timer_contract());
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
