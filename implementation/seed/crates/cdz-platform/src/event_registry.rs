//! The event registry (`design/cadenza-platform.md` §3) — the kernel's trust root.
//!
//! On an emitted effect the kernel must pick which **system reducer** shepherds it (§4). This registry is
//! that choice: a contract-id maps to the event-reducer implementation (by content hash) that governs its
//! events, with a **default** for every contract without an explicit entry. The default is itself a wasm
//! module bootstrapped at setup, so the kernel ships with no dispatch logic — it only holds this table and
//! looks up the reducer to run.
//!
//! It is the **trust root**: the whole security model rests on the chosen system reducer being correct, so
//! installing or changing an override is the highest privilege in the system — settable only by the root
//! authority. This structure just stores the table; the privilege is enforced by the kernel that owns it,
//! the way the deliver primitive is granted only to the reducer this registry names.
//!
//! Reached by direct read (the kernel resolves a contract mid-dispatch), so the operations are plain
//! synchronous methods; the mutator takes `&mut self`.

use crate::Hash;
use std::collections::HashMap;

/// Maps a contract-id to the event-reducer implementation (by content hash) that shepherds its events, over
/// a default that governs every contract without an explicit override (§3). [`resolve`](Self::resolve)
/// therefore always yields a reducer — there is no "no event reducer" state, because the default always
/// applies.
#[derive(Debug, Clone)]
pub struct EventRegistry {
    /// The event reducer that governs any contract without an explicit override — a wasm module bootstrapped
    /// at setup.
    default: Hash,
    /// contract-id -> the event reducer that overrides the default for that contract.
    overrides: HashMap<Hash, Hash>,
}

impl EventRegistry {
    /// A registry whose `default` event reducer governs every contract, with no overrides yet.
    #[must_use]
    pub fn new(default: Hash) -> Self {
        Self {
            default,
            overrides: HashMap::new(),
        }
    }

    /// The default event reducer — the one that governs any contract without an override.
    #[must_use]
    pub fn default_reducer(&self) -> Hash {
        self.default
    }

    /// Install or replace the event reducer that governs `contract`, returning the override it replaced (if
    /// any). This is the highest-privilege operation in the system — the security model depends on the
    /// chosen reducer being correct — so a caller must be the root authority (enforced by the kernel that
    /// owns this registry, not here).
    pub fn set_override(&mut self, contract: Hash, event_reducer: Hash) -> Option<Hash> {
        self.overrides.insert(contract, event_reducer)
    }

    /// Remove `contract`'s override so it falls back to the default, returning the override that was removed
    /// (if any). Also root-only.
    pub fn clear_override(&mut self, contract: Hash) -> Option<Hash> {
        self.overrides.remove(&contract)
    }

    /// The event reducer that governs `contract`: its override if one is installed, otherwise the default.
    /// This is the kernel's lookup on an emitted effect — always a reducer, never absent.
    #[must_use]
    pub fn resolve(&self, contract: Hash) -> Hash {
        self.overrides
            .get(&contract)
            .copied()
            .unwrap_or(self.default)
    }

    /// The number of contracts with an explicit override (the default is not counted).
    #[must_use]
    pub fn overrides(&self) -> usize {
        self.overrides.len()
    }
}

#[cfg(test)]
mod tests {
    use super::EventRegistry;
    use crate::Hash;

    fn h(tag: &str) -> Hash {
        Hash::of(tag.as_bytes())
    }

    #[test]
    fn every_contract_resolves_to_the_default_until_overridden() {
        let reg = EventRegistry::new(h("default-event-reducer"));
        assert_eq!(reg.resolve(h("any.contract")), h("default-event-reducer"));
        assert_eq!(reg.resolve(h("other.contract")), h("default-event-reducer"));
        assert_eq!(reg.default_reducer(), h("default-event-reducer"));
        assert_eq!(reg.overrides(), 0);
    }

    #[test]
    fn an_override_governs_only_its_contract() {
        let mut reg = EventRegistry::new(h("default"));
        assert!(reg.set_override(h("session.spawn"), h("custom")).is_none());
        // the overridden contract resolves to the custom reducer; everything else stays on the default.
        assert_eq!(reg.resolve(h("session.spawn")), h("custom"));
        assert_eq!(reg.resolve(h("http.get")), h("default"));
        assert_eq!(reg.overrides(), 1);
    }

    #[test]
    fn set_override_replaces_and_returns_the_prior() {
        let mut reg = EventRegistry::new(h("default"));
        reg.set_override(h("c"), h("first"));
        // replacing returns exactly the prior override, and the new one wins.
        assert_eq!(reg.set_override(h("c"), h("second")), Some(h("first")));
        assert_eq!(reg.resolve(h("c")), h("second"));
        assert_eq!(reg.overrides(), 1);
    }

    #[test]
    fn clearing_an_override_falls_back_to_the_default() {
        let mut reg = EventRegistry::new(h("default"));
        reg.set_override(h("c"), h("custom"));
        // clearing returns the removed override and the contract goes back to the default.
        assert_eq!(reg.clear_override(h("c")), Some(h("custom")));
        assert_eq!(reg.resolve(h("c")), h("default"));
        assert_eq!(reg.overrides(), 0);
        // clearing an absent override is a no-op.
        assert_eq!(reg.clear_override(h("c")), None);
    }
}
