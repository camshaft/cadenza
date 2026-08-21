//! Program instantiation — turning a [`ProgramHash`] into a fresh [`Reducer`] (`design/cadenza-platform.md`
//! §3/§4).
//!
//! Routing an event resolves a contract to the **program** its event reducer is spawned from (a
//! [`ProgramHash`], via the [`EventRegistry`](crate::EventRegistry)). To actually run it the kernel must
//! **instantiate** that program into a live reducer — and, because the system reducer is instantiated once
//! per event (§4), it does so afresh each time. In the finished platform a program hash resolves to a wasm
//! module in the content-addressed store, instantiated into a reducer instance. This is the native, pre-wasm
//! stand-in for exactly that step: a program hash maps to a factory that builds a fresh [`Reducer`], so the
//! runtime can spawn the event reducer a contract routes to without a wasm loader yet.
//!
//! A factory is `Fn`, not `FnOnce` — one registered program spawns many instances (one per event), each a
//! fresh reducer with its own state. The default event reducer is registered here at setup under the hash
//! the [`EventRegistry`](crate::EventRegistry) defaults to.

use crate::{ProgramHash, Reducer};
use std::collections::HashMap;

/// A factory that builds a fresh [`Reducer`] instance — the native stand-in for instantiating a program's
/// wasm module. `Fn` (reusable) because a program is instantiated once per event, so its factory is called
/// many times; `Send + Sync` so the runtime that holds it stays `Send + Sync`.
type Factory = Box<dyn Fn() -> Box<dyn Reducer> + Send + Sync>;

/// The instantiable programs, each keyed by its [`ProgramHash`] — the native analogue of the
/// content-addressed store of wasm modules. [`spawn`](Self::spawn) builds a fresh reducer from a program's
/// factory; the runtime spawns the event reducer a contract routes to through here.
#[derive(Default)]
pub struct Programs {
    factories: HashMap<ProgramHash, Factory>,
}

impl Programs {
    /// An empty set of programs — nothing instantiable yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register the factory that instantiates `program`, replacing any factory already registered under
    /// that hash. The factory builds a fresh reducer each time it is called (once per event a program
    /// governs), so it captures only what every instance shares — never per-instance mutable state.
    pub fn register(
        &mut self,
        program: ProgramHash,
        factory: impl Fn() -> Box<dyn Reducer> + Send + Sync + 'static,
    ) {
        self.factories.insert(program, Box::new(factory));
    }

    /// Instantiate `program` into a fresh reducer, or `None` if no factory is registered under that hash
    /// (a misconfiguration: a contract routed to a program the kernel cannot instantiate).
    #[must_use]
    pub fn spawn(&self, program: ProgramHash) -> Option<Box<dyn Reducer>> {
        self.factories.get(&program).map(|factory| factory())
    }

    /// Whether a factory is registered for `program`.
    #[must_use]
    pub fn contains(&self, program: ProgramHash) -> bool {
        self.factories.contains_key(&program)
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

#[cfg(test)]
mod tests {
    use super::Programs;
    use crate::{Hash, Message, Notification, Outcome, ProgramHash, Reducer, Request, Response};

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

    #[test]
    fn spawn_instantiates_a_registered_program_and_is_none_otherwise() {
        let mut programs = Programs::new();
        assert!(programs.is_empty());
        programs.register(prog(b"default-event"), || Box::new(Counting { seen: 0 }));
        assert!(programs.contains(prog(b"default-event")));
        assert_eq!(programs.len(), 1);
        assert!(programs.spawn(prog(b"default-event")).is_some());
        // A program with no registered factory cannot be instantiated.
        assert!(programs.spawn(prog(b"unregistered")).is_none());
    }

    #[tokio::test]
    async fn each_spawn_is_a_fresh_instance_with_its_own_state() {
        let mut programs = Programs::new();
        programs.register(prog(b"p"), || Box::new(Counting { seen: 0 }));
        // Two spawns are independent: stepping one does not affect the other (per-event instantiation).
        let mut a = programs.spawn(prog(b"p")).expect("registered");
        let b = programs.spawn(prog(b"p")).expect("registered");
        a.on_message(Message {
            id: crate::ContractId::from_hash(Hash::of(b"c")),
            payload: crate::Bytes::new(),
            from: crate::Origin {
                reducer: crate::ReducerId::from_hash(Hash::of(b"r")),
                host: crate::HostId::from_hash(Hash::of(b"h")),
            },
            continuation_token: crate::Bytes::new(),
        })
        .await;
        // `b` is a separate instance; if they shared state this would be observable, but they do not.
        drop((a, b));
    }

    #[test]
    fn register_replaces_the_factory_for_a_program() {
        let mut programs = Programs::new();
        programs.register(prog(b"p"), || Box::new(Counting { seen: 0 }));
        programs.register(prog(b"p"), || Box::new(Counting { seen: 99 }));
        // Still one program; the second factory won.
        assert_eq!(programs.len(), 1);
        assert!(programs.spawn(prog(b"p")).is_some());
    }
}
