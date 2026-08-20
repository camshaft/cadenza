//! The handler-chain registry (`design/cadenza-platform.md` §3/§4).
//!
//! A contract's handler is a **chain** of reducer identifiers, and this is the data structure that holds
//! those chains: given a contract-id, find the chain that answers it. That is the whole of dispatch — no
//! enumerated effect kinds, no family strings, no per-effect branches; nothing recognizes a request by a
//! name, only by the handler a contract-id resolves to (§4). The chains are the **system reducer's** state
//! (§4): the system reducer shepherds each effect and uses these chains to route it. This module is the
//! chains as a plain data structure, separate from the system reducer that drives reducers with them, and
//! separate from the kernel's own event-reducer override registry (§3), which maps a contract to the
//! system-reducer implementation that governs it.
//!
//! A chain is an ordered list of reducer identifiers (§3, "handlers chain") — a stack of interceptors: a
//! rate limiter wrapping an HTTP handler, an authorizer wrapping a credential mint, a logger wrapping
//! anything. A reducer identifier is a [`Hash`] (the reducer's id); this structure stores identifiers, not
//! reducer instances — resolving an id to a live reducer is the system reducer's job. It keeps each chain in
//! the order it was given and interprets nothing about it: which end a request enters and how it bubbles is
//! the system reducer's semantics, not this structure's.
//!
//! Two operations back the built-in lifecycle effects (§7):
//! - [`set_handler`](HandlerRegistry::set_handler) installs or **replaces** the whole chain for a contract
//!   — how a session is upgraded over time (a handler added, a chain extended or reordered) without
//!   respawning it. Setting an empty chain removes the registration, so the two states a contract can be in
//!   are "has a non-empty chain" or "unregistered"; there is no registered-but-empty limbo.
//! - [`contracts_for`](HandlerRegistry::contracts_for) is the reverse lookup behind `list-handlers`: the
//!   contracts a given reducer appears in a chain for. It returns only contract-ids (the surface); the
//!   chains themselves — the concrete reducer identifiers, the middleware behind a contract — are never
//!   exposed by it, matching the spec's rule that a peer sees the interface a reducer has, not how it is
//!   implemented.
//!
//! Dispatch is [`resolve`](HandlerRegistry::resolve): a contract-id maps to its chain, or to `None`, which
//! the system reducer turns into `Err(MissingHandler)` (§4). The system reducer holds this in its own
//! key-value state and reads it synchronously mid-fold, so the operations are plain synchronous methods.

use crate::Hash;
use std::collections::HashMap;

/// The handler-chain registry: contract-id -> the ordered chain of reducer identifiers that answers it
/// (§3/§4). Part of the system reducer's state, with a single owner (the system reducer folding one event
/// at a time), so its mutator takes `&mut self`.
#[derive(Debug, Default, Clone)]
pub struct HandlerRegistry {
    /// contract-id -> chain of reducer ids. An entry is always a non-empty chain: `set_handler` removes the
    /// key when handed an empty chain, so a present key means "this contract has a handler" with no empty
    /// special case for `resolve` to consider.
    chains: HashMap<Hash, Vec<Hash>>,
}

impl HandlerRegistry {
    /// An empty registry — nothing is registered, so every contract resolves to `None`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Install or replace the handler `chain` for `contract`, returning the chain previously registered (if
    /// any). This is the one registration primitive (§3/§4) and the `set-handler` lifecycle effect (§7): the
    /// caller passes the whole chain and it replaces what was there — a handler added, a chain extended or
    /// reordered — never a merge. Takes the chain by value because it is stored, not borrowed.
    ///
    /// An **empty** chain removes the registration (equivalent to "no handler"): after it, `contract`
    /// resolves to `None` and no longer appears in any reducer's [`contracts_for`](Self::contracts_for). So
    /// there is no registered-but-empty state — a contract either has a non-empty chain or is unregistered.
    pub fn set_handler(&mut self, contract: Hash, chain: Vec<Hash>) -> Option<Vec<Hash>> {
        if chain.is_empty() {
            self.chains.remove(&contract)
        } else {
            self.chains.insert(contract, chain)
        }
    }

    /// Resolve `contract` to its handler chain — the one dispatch lookup (§4). `Some(chain)` is the ordered
    /// reducer identifiers to route through (always non-empty); `None` means no handler is registered, which
    /// the system reducer reports as `Err(MissingHandler)`. Borrows the stored chain; the system reducer
    /// reads it to drive the reducers it names.
    #[must_use]
    pub fn resolve(&self, contract: Hash) -> Option<&[Hash]> {
        self.chains.get(&contract).map(Vec::as_slice)
    }

    /// The contracts `reducer` is part of a handler chain for — the reverse lookup behind the `list-handlers`
    /// effect (§7). Returns just the contract-ids (the surface a peer is allowed to see), never the chains
    /// they front. The result is sorted ascending so it is deterministic regardless of the registry's
    /// internal (hash-map) order — replay and equality checks depend on that.
    ///
    /// A reducer that appears more than once across chains (or twice in one chain) yields each contract-id
    /// once; the caller learns which contracts it handles, not how many times it is wired in.
    #[must_use]
    pub fn contracts_for(&self, reducer: Hash) -> Vec<Hash> {
        let mut contracts: Vec<Hash> = self
            .chains
            .iter()
            .filter(|(_, chain)| chain.contains(&reducer))
            .map(|(contract, _)| *contract)
            .collect();
        contracts.sort_unstable();
        contracts
    }

    /// The number of contracts with a registered handler.
    #[must_use]
    pub fn len(&self) -> usize {
        self.chains.len()
    }

    /// Whether no contract has a registered handler.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.chains.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::HandlerRegistry;
    use crate::Hash;

    // Distinct hashes to stand in for contract-ids and reducer-ids; the registry only cares that they are
    // distinct Hashes, not what they hash.
    fn h(tag: &str) -> Hash {
        Hash::of(tag.as_bytes())
    }

    #[test]
    fn resolve_returns_the_chain_in_the_order_it_was_registered() {
        let mut reg = HandlerRegistry::new();
        let contract = h("http.get");
        // a chain of three: authz wraps rate-limit wraps the edge handler, say.
        let chain = vec![h("authz"), h("rate-limit"), h("http-edge")];
        assert!(reg.set_handler(contract, chain.clone()).is_none());
        // the registry preserves order exactly and interprets nothing about it.
        assert_eq!(reg.resolve(contract), Some(chain.as_slice()));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn an_unregistered_contract_resolves_to_none() {
        let reg = HandlerRegistry::new();
        assert_eq!(reg.resolve(h("nobody-answers")), None);
        assert!(reg.is_empty());
    }

    #[test]
    fn set_handler_replaces_the_whole_chain_and_returns_the_old_one() {
        let mut reg = HandlerRegistry::new();
        let contract = h("mint-credential");
        let first = vec![h("broker")];
        let replacement = vec![h("authz"), h("broker")]; // upgraded: authz middleware prepended.
        reg.set_handler(contract, first.clone());
        // replacing returns exactly the prior chain, and the new one wins entirely (no merge).
        assert_eq!(reg.set_handler(contract, replacement.clone()), Some(first));
        assert_eq!(reg.resolve(contract), Some(replacement.as_slice()));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn setting_an_empty_chain_removes_the_registration() {
        let mut reg = HandlerRegistry::new();
        let contract = h("timer.arm");
        let chain = vec![h("timer-edge")];
        reg.set_handler(contract, chain.clone());
        // removing returns the prior chain and leaves the contract unregistered — no empty-chain limbo.
        assert_eq!(reg.set_handler(contract, Vec::new()), Some(chain));
        assert_eq!(reg.resolve(contract), None);
        assert!(reg.is_empty());
        // removing an already-absent contract is a no-op returning None.
        assert_eq!(reg.set_handler(contract, Vec::new()), None);
    }

    #[test]
    fn contracts_for_is_the_reverse_lookup_sorted_and_deduplicated() {
        let mut reg = HandlerRegistry::new();
        let authz = h("authz");
        // authz is middleware in two different contracts' chains; the edge handlers differ.
        reg.set_handler(h("http.get"), vec![authz, h("http-edge")]);
        reg.set_handler(h("http.post"), vec![authz, h("http-edge")]);
        reg.set_handler(h("clock.now"), vec![h("clock-edge")]); // authz not in this one.

        let mut expected = vec![h("http.get"), h("http.post")];
        expected.sort_unstable();
        assert_eq!(reg.contracts_for(authz), expected);

        // a reducer that appears twice in one chain still yields its contract once.
        reg.set_handler(h("loop.contract"), vec![authz, h("mid"), authz]);
        assert!(reg.contracts_for(authz).contains(&h("loop.contract")));
        let n = reg
            .contracts_for(authz)
            .iter()
            .filter(|c| **c == h("loop.contract"))
            .count();
        assert_eq!(
            n, 1,
            "each contract appears once regardless of wiring count"
        );

        // a reducer wired nowhere handles nothing.
        assert_eq!(reg.contracts_for(h("orphan")), Vec::<Hash>::new());
    }

    #[test]
    fn contracts_for_reflects_removal() {
        let mut reg = HandlerRegistry::new();
        let edge = h("edge");
        reg.set_handler(h("a"), vec![edge]);
        reg.set_handler(h("b"), vec![edge]);
        assert_eq!(reg.contracts_for(edge).len(), 2);
        // removing one contract's chain drops it from the reverse lookup.
        reg.set_handler(h("a"), Vec::new());
        assert_eq!(reg.contracts_for(edge), vec![h("b")]);
    }
}
