//! The event registry (`design/cadenza-platform.md` §3) — the kernel's trust root.
//!
//! On an emitted effect the kernel must pick which **event reducer** shepherds it (§4). This registry is
//! that choice: a contract-id maps to the event-reducer program (by content hash) that governs its events,
//! with a **default** for every contract without an explicit entry. The default is itself a program
//! bootstrapped at setup, so the kernel ships with no dispatch logic — it only holds this table and looks up
//! the program to run.
//!
//! It is the **trust root**: the whole security model rests on the chosen event reducer being correct, so
//! installing or changing an override is the highest privilege in the system — settable only by the root
//! authority.
//!
//! [`resolve`](EventRegistry::resolve) is **async** and behind a trait so the table can be backed by more
//! than a local map: a durable, replicated store queried across the network answers the same lookup. The
//! in-memory [`InMemoryEventRegistry`] is a plain map for tests and single-process use; the mutators that
//! install overrides stay synchronous on it, since the privilege that guards them is the concern of the
//! kernel that owns it.

use crate::{ContractId, ProgramHash};
use async_trait::async_trait;
use std::collections::HashMap;

/// Which program a contract's event reducer is spawned from (§3). The one lookup the kernel makes on an
/// emitted effect — always yields a program (a default always applies), and is async so a backend can answer
/// it from a replicated store rather than a local map. `Send + Sync` so it is shared behind an `Arc`.
#[async_trait]
pub trait EventRegistry: Send + Sync {
    /// The program whose event reducer governs `contract`: its override if one is installed, otherwise the
    /// default. Always a program, never absent.
    async fn resolve(&self, contract: ContractId) -> ProgramHash;
}

/// An in-memory [`EventRegistry`] — a default program plus a map of contract-id overrides. For tests and
/// single-process use. The override mutators are synchronous (they edit the local map); the privilege that
/// only the root authority may call them is enforced by the kernel that owns the registry, not here.
#[derive(Debug, Clone)]
pub struct InMemoryEventRegistry {
    default: ProgramHash,
    overrides: HashMap<ContractId, ProgramHash>,
}

impl InMemoryEventRegistry {
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
    /// The highest-privilege operation in the system — a caller must be the root authority (enforced by the
    /// kernel that owns this registry, not here).
    pub fn set_override(
        &mut self,
        contract: ContractId,
        program: ProgramHash,
    ) -> Option<ProgramHash> {
        self.overrides.insert(contract, program)
    }

    /// Remove `contract`'s override so it falls back to the default, returning the override removed (if any).
    /// Also root-only.
    pub fn clear_override(&mut self, contract: ContractId) -> Option<ProgramHash> {
        self.overrides.remove(&contract)
    }

    /// The number of contracts with an explicit override (the default is not counted).
    #[must_use]
    pub fn overrides(&self) -> usize {
        self.overrides.len()
    }
}

#[async_trait]
impl EventRegistry for InMemoryEventRegistry {
    async fn resolve(&self, contract: ContractId) -> ProgramHash {
        self.overrides
            .get(&contract)
            .copied()
            .unwrap_or(self.default)
    }
}

#[cfg(test)]
mod tests {
    use super::{EventRegistry, InMemoryEventRegistry};
    use crate::{ContractId, ProgramHash};

    fn cid(tag: &str) -> ContractId {
        ContractId::of(tag.as_bytes())
    }
    fn prog(tag: &str) -> ProgramHash {
        ProgramHash::of(tag.as_bytes())
    }

    #[tokio::test]
    async fn every_contract_resolves_to_the_default_until_overridden() {
        let reg = InMemoryEventRegistry::new(prog("default-event-reducer"));
        assert_eq!(
            reg.resolve(cid("any.contract")).await,
            prog("default-event-reducer")
        );
        assert_eq!(
            reg.resolve(cid("other.contract")).await,
            prog("default-event-reducer")
        );
        assert_eq!(reg.default_reducer(), prog("default-event-reducer"));
        assert_eq!(reg.overrides(), 0);
    }

    #[tokio::test]
    async fn an_override_governs_only_its_contract() {
        let mut reg = InMemoryEventRegistry::new(prog("default"));
        assert!(
            reg.set_override(cid("session.spawn"), prog("custom"))
                .is_none()
        );
        assert_eq!(reg.resolve(cid("session.spawn")).await, prog("custom"));
        assert_eq!(reg.resolve(cid("http.get")).await, prog("default"));
        assert_eq!(reg.overrides(), 1);
    }

    #[tokio::test]
    async fn set_override_replaces_and_returns_the_prior() {
        let mut reg = InMemoryEventRegistry::new(prog("default"));
        reg.set_override(cid("c"), prog("first"));
        assert_eq!(
            reg.set_override(cid("c"), prog("second")),
            Some(prog("first"))
        );
        assert_eq!(reg.resolve(cid("c")).await, prog("second"));
        assert_eq!(reg.overrides(), 1);
    }

    #[tokio::test]
    async fn clearing_an_override_falls_back_to_the_default() {
        let mut reg = InMemoryEventRegistry::new(prog("default"));
        reg.set_override(cid("c"), prog("custom"));
        assert_eq!(reg.clear_override(cid("c")), Some(prog("custom")));
        assert_eq!(reg.resolve(cid("c")).await, prog("default"));
        assert_eq!(reg.overrides(), 0);
        assert_eq!(reg.clear_override(cid("c")), None);
    }

    /// The registry resolves under Cameron's Bach simulator, not just tokio — `resolve` is await-only and
    /// the in-memory map is runtime-agnostic, so Bach drives it unchanged (the seam for deterministic
    /// dispatch, where routing an effect resolves the event reducer for its contract).
    #[test]
    fn event_registry_resolves_under_the_bach_simulator() {
        use bach::ext::*;
        bach::sim(|| {
            async {
                let mut reg = InMemoryEventRegistry::new(prog("default"));
                reg.set_override(cid("http.get"), prog("custom"));
                assert_eq!(reg.resolve(cid("http.get")).await, prog("custom"));
                assert_eq!(reg.resolve(cid("other")).await, prog("default"));
            }
            .group("event-registry")
            .primary()
            .spawn();
        });
    }
}
