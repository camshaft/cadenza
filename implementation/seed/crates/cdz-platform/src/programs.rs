//! Program instantiation — turning a [`ProgramHash`] into a fresh [`Reducer`] (`design/cadenza-platform.md`
//! §3/§4).
//!
//! Every reducer is driven by a **program**, named by content hash. Whenever the kernel spawns a reducer —
//! the event reducer a contract routes to (§4), a child reducer a session spawns (§7), any reducer at all —
//! it must instantiate that program into a live reducer instance. This is that step: a generic store from a
//! [`ProgramHash`] to a fresh [`Reducer`]. It is agnostic to what the program is and how it is realized.
//!
//! In production there is one realization: resolve the program's wasm component from the content-addressed
//! blob store and instantiate it into a wasm reducer. That backend does not exist yet. The only backend that
//! ships today is [`testing::Store`], gated behind `cfg(any(test, feature = "testing"))` — a map of program
//! hash to a Rust factory, for wiring reducers into the runtime in tests. It is deliberately not a general
//! "in-memory" backend: outside tests the store is always the CAS-plus-wasm loader.
//!
//! The operation is **async** because the production backend resolves bytes from the store before
//! instantiating, and awaiting a fetch must not block the runtime. The trait is [`async_trait`] so a backend
//! is a dyn-safe swappable trait object, runtime-agnostic (it only awaits): tokio in production, the Bach
//! simulator in deterministic tests.

use crate::{ProgramHash, Reducer};
use async_trait::async_trait;

/// A store that instantiates a reducer from its [`ProgramHash`] — the program that drives it. The production
/// backend resolves a wasm component from the blob store and instantiates it; tests use [`testing::Store`].
/// `Send + Sync` so it can be shared across the runtime's concurrent tasks behind a box/`Arc`.
#[async_trait]
pub trait ProgramStore: Send + Sync {
    /// Instantiate `program` into a fresh reducer, or `None` if the store cannot (an unknown program — a
    /// misconfiguration, since a routed program should always be instantiable). Each call yields a fresh
    /// instance with its own state: a program is instantiated once per event/spawn, so this is called many
    /// times for one program.
    async fn spawn(&self, program: ProgramHash) -> Option<Box<dyn Reducer>>;

    /// Whether `program` can be instantiated by this store — an inspection that does not build a reducer.
    async fn contains(&self, program: ProgramHash) -> bool;
}

/// A [`ProgramStore`] for tests: program hash to a Rust factory, so a test can make native reducers
/// instantiable without the CAS-plus-wasm loader. Gated behind `cfg(any(test, feature = "testing"))` — it is
/// test-only scaffolding, never a production backend (outside tests, programs always load from the store).
#[cfg(any(test, feature = "testing"))]
pub mod testing {
    use super::{ProgramStore, async_trait};
    use crate::{ProgramHash, Reducer};
    use std::collections::HashMap;

    /// A factory that builds a fresh [`Reducer`]. `Fn` (reusable — one program is instantiated many times,
    /// once per event/spawn); `Send + Sync` so the store stays so.
    type Factory = Box<dyn Fn() -> Box<dyn Reducer> + Send + Sync>;

    /// A test [`ProgramStore`]: each program hash mapped to a Rust factory. Register the reducers a test
    /// needs, then hand it to the runtime as the program store.
    #[derive(Default)]
    pub struct Store {
        factories: HashMap<ProgramHash, Factory>,
    }

    impl Store {
        /// An empty store — nothing instantiable yet.
        #[must_use]
        pub fn new() -> Self {
            Self::default()
        }

        /// Register the factory that instantiates `program`, replacing any already registered under that
        /// hash. The factory builds a fresh reducer each call (once per event/spawn), so it captures only
        /// what every instance shares — never per-instance mutable state.
        pub fn register(
            &mut self,
            program: ProgramHash,
            factory: impl Fn() -> Box<dyn Reducer> + Send + Sync + 'static,
        ) {
            self.factories.insert(program, Box::new(factory));
        }

        /// The number of registered programs.
        #[must_use]
        pub fn len(&self) -> usize {
            self.factories.len()
        }

        /// Whether no program is registered.
        #[must_use]
        pub fn is_empty(&self) -> bool {
            self.factories.is_empty()
        }
    }

    #[async_trait]
    impl ProgramStore for Store {
        async fn spawn(&self, program: ProgramHash) -> Option<Box<dyn Reducer>> {
            self.factories.get(&program).map(|factory| factory())
        }

        async fn contains(&self, program: ProgramHash) -> bool {
            self.factories.contains_key(&program)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ProgramStore;
    use super::testing::Store;
    use crate::{
        Bytes, ContractId, Hash, HostId, Message, Notification, Origin, Outcome, ProgramHash,
        Reducer, ReducerId, Request, Response,
    };

    fn prog(tag: &[u8]) -> ProgramHash {
        ProgramHash::from_hash(Hash::of(tag))
    }

    /// A reducer that carries a private counter, so two spawns from the same factory can be shown to be
    /// distinct instances (each starts at its own zero).
    struct Counting {
        seen: u64,
    }

    #[async_trait::async_trait]
    impl Reducer for Counting {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            self.seen += 1;
            (Vec::new(), Outcome::Continue)
        }
        async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
    }

    #[tokio::test]
    async fn spawn_instantiates_a_registered_program_and_is_none_otherwise() {
        let mut store = Store::new();
        assert!(store.is_empty());
        store.register(prog(b"default-event"), || Box::new(Counting { seen: 0 }));
        assert!(store.contains(prog(b"default-event")).await);
        assert_eq!(store.len(), 1);
        assert!(store.spawn(prog(b"default-event")).await.is_some());
        // A program with no registered factory cannot be instantiated.
        assert!(store.spawn(prog(b"unregistered")).await.is_none());
        assert!(!store.contains(prog(b"unregistered")).await);
    }

    #[tokio::test]
    async fn each_spawn_is_a_fresh_instance_with_its_own_state() {
        let mut store = Store::new();
        store.register(prog(b"p"), || Box::new(Counting { seen: 0 }));
        // Two spawns are independent: stepping one does not affect the other (per-event/spawn instantiation).
        let mut a = store.spawn(prog(b"p")).await.expect("registered");
        let b = store.spawn(prog(b"p")).await.expect("registered");
        a.on_message(Message {
            id: ContractId::from_hash(Hash::of(b"c")),
            payload: Bytes::new(),
            from: Origin {
                reducer: ReducerId::from_hash(Hash::of(b"r")),
                host: HostId::from_hash(Hash::of(b"h")),
            },
            continuation_token: Bytes::new(),
        })
        .await;
        // `b` is a separate instance; if they shared state this would be observable, but they do not.
        drop((a, b));
    }

    #[tokio::test]
    async fn register_replaces_the_factory_for_a_program() {
        let mut store = Store::new();
        store.register(prog(b"p"), || Box::new(Counting { seen: 0 }));
        store.register(prog(b"p"), || Box::new(Counting { seen: 99 }));
        // Still one program; the second factory won.
        assert_eq!(store.len(), 1);
        assert!(store.spawn(prog(b"p")).await.is_some());
    }
}
