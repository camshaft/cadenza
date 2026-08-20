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

use crate::{ContractId, ProgramHash};
use std::collections::HashMap;

/// Maps a contract-id to the **program** its event reducer is spawned from (a component by content hash),
/// over a default program that governs every contract without an explicit override (§3).
/// [`resolve`](Self::resolve) therefore always yields a program — there is no "no event reducer" state,
/// because the default always applies.
#[derive(Debug, Clone)]
pub struct EventRegistry {
    /// The program whose event reducer governs any contract without an explicit override — a wasm module
    /// bootstrapped at setup.
    default: ProgramHash,
    /// contract-id -> the program that overrides the default for that contract.
    overrides: HashMap<ContractId, ProgramHash>,
}

impl EventRegistry {
    /// A registry whose `default` program governs every contract, with no overrides yet.
    #[must_use]
    pub fn new(default: ProgramHash) -> Self {
        Self {
            default,
            overrides: HashMap::new(),
        }
    }

    /// The default program — the one whose event reducer governs any contract without an override.
    #[must_use]
    pub fn default_reducer(&self) -> ProgramHash {
        self.default
    }

    /// Install or replace the program that governs `contract`, returning the override it replaced (if any).
    /// This is the highest-privilege operation in the system — the security model depends on the chosen
    /// program being correct — so a caller must be the root authority (enforced by the kernel that owns this
    /// registry, not here).
    pub fn set_override(
        &mut self,
        contract: ContractId,
        program: ProgramHash,
    ) -> Option<ProgramHash> {
        self.overrides.insert(contract, program)
    }

    /// Remove `contract`'s override so it falls back to the default, returning the override that was removed
    /// (if any). Also root-only.
    pub fn clear_override(&mut self, contract: ContractId) -> Option<ProgramHash> {
        self.overrides.remove(&contract)
    }

    /// The program whose event reducer governs `contract`: its override if one is installed, otherwise the
    /// default. This is the kernel's lookup on an emitted effect — always a program, never absent.
    #[must_use]
    pub fn resolve(&self, contract: ContractId) -> ProgramHash {
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
    use crate::{ContractId, Hash, ProgramHash};

    fn cid(tag: &str) -> ContractId {
        ContractId::from_hash(Hash::of(tag.as_bytes()))
    }
    fn prog(tag: &str) -> ProgramHash {
        ProgramHash::from_hash(Hash::of(tag.as_bytes()))
    }

    #[test]
    fn every_contract_resolves_to_the_default_until_overridden() {
        let reg = EventRegistry::new(prog("default-event-reducer"));
        assert_eq!(
            reg.resolve(cid("any.contract")),
            prog("default-event-reducer")
        );
        assert_eq!(
            reg.resolve(cid("other.contract")),
            prog("default-event-reducer")
        );
        assert_eq!(reg.default_reducer(), prog("default-event-reducer"));
        assert_eq!(reg.overrides(), 0);
    }

    #[test]
    fn an_override_governs_only_its_contract() {
        let mut reg = EventRegistry::new(prog("default"));
        assert!(
            reg.set_override(cid("session.spawn"), prog("custom"))
                .is_none()
        );
        // the overridden contract resolves to the custom program; everything else stays on the default.
        assert_eq!(reg.resolve(cid("session.spawn")), prog("custom"));
        assert_eq!(reg.resolve(cid("http.get")), prog("default"));
        assert_eq!(reg.overrides(), 1);
    }

    #[test]
    fn set_override_replaces_and_returns_the_prior() {
        let mut reg = EventRegistry::new(prog("default"));
        reg.set_override(cid("c"), prog("first"));
        // replacing returns exactly the prior override, and the new one wins.
        assert_eq!(
            reg.set_override(cid("c"), prog("second")),
            Some(prog("first"))
        );
        assert_eq!(reg.resolve(cid("c")), prog("second"));
        assert_eq!(reg.overrides(), 1);
    }

    #[test]
    fn clearing_an_override_falls_back_to_the_default() {
        let mut reg = EventRegistry::new(prog("default"));
        reg.set_override(cid("c"), prog("custom"));
        // clearing returns the removed override and the contract goes back to the default.
        assert_eq!(reg.clear_override(cid("c")), Some(prog("custom")));
        assert_eq!(reg.resolve(cid("c")), prog("default"));
        assert_eq!(reg.overrides(), 0);
        // clearing an absent override is a no-op.
        assert_eq!(reg.clear_override(cid("c")), None);
    }
}
