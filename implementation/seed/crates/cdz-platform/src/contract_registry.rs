//! Resolving a built-in protocol contract by name (`design/cadenza-platform.md` §1/§4).
//!
//! Routing is by contract-id — the hash of a contract's declaration — never by string name (§1). But the
//! handful of contracts the platform itself defines (the *protocol* contracts: deliver, timer, spawned,
//! lifecycle) have a stable well-known name, and it is convenient for a tool that describes a run in text —
//! the integration-test harness's spec, say — to refer to one of them by that name rather than by pasting
//! its 33 raw hash bytes. This registry is the single place that maps a protocol contract's name to its id,
//! so the id computation stays in one place (each `*_contract()` constructor) and a name can never resolve
//! to a stale or hand-copied hash.
//!
//! Only the protocol contracts resolve by name. A user contract's identity is the hash of its declaration
//! (§1); it has no platform-known name, so it is never name-resolvable — it is referenced by its id
//! directly. `contract_id_by_name` returns `None` for any name it does not define, so a caller that also
//! accepts a raw id can fall through to that.

use crate::ContractId;

/// Resolve one of the platform's built-in **protocol** contracts by its short name, or `None` if the name
/// is not a protocol contract.
///
/// The recognized names are exactly the platform protocol contracts (§4):
/// - `"deliver"` — the one contract the kernel privileges: injecting an event into a reducer's log.
/// - `"timer"` — a reducer arms a timer that fires back after a delay.
/// - `"spawned"` — the notification a supervisor receives when a child is spawned.
/// - `"lifecycle"` — the notification carrying a reducer's lifecycle transitions.
///
/// A user-defined contract is identified by the hash of its declaration and has no platform-known name, so
/// it never resolves here; reference it by its id.
#[must_use]
pub fn contract_id_by_name(name: &str) -> Option<ContractId> {
    Some(match name {
        "deliver" => crate::deliver_contract(),
        "timer" => crate::timer_contract(),
        "spawned" => crate::spawned_contract(),
        "lifecycle" => crate::lifecycle_contract(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::contract_id_by_name;

    #[test]
    fn resolves_every_protocol_contract_to_its_id() {
        // Each name resolves to exactly the id its `*_contract()` constructor derives — the single source
        // of the id, so a name can never drift to a stale hash.
        assert_eq!(
            contract_id_by_name("deliver"),
            Some(crate::deliver_contract())
        );
        assert_eq!(contract_id_by_name("timer"), Some(crate::timer_contract()));
        assert_eq!(
            contract_id_by_name("spawned"),
            Some(crate::spawned_contract())
        );
        assert_eq!(
            contract_id_by_name("lifecycle"),
            Some(crate::lifecycle_contract())
        );
    }

    #[test]
    fn distinct_protocol_contracts_have_distinct_ids() {
        // A sanity check that the names are not all mapping to one contract: the four protocol ids are
        // pairwise distinct (their declarations differ, so their hashes do).
        let ids = [
            contract_id_by_name("deliver").unwrap(),
            contract_id_by_name("timer").unwrap(),
            contract_id_by_name("spawned").unwrap(),
            contract_id_by_name("lifecycle").unwrap(),
        ];
        for (i, a) in ids.iter().enumerate() {
            for b in &ids[i + 1..] {
                assert_ne!(a, b, "protocol contract ids must be distinct");
            }
        }
    }

    #[test]
    fn an_unknown_name_does_not_resolve() {
        // A user contract has no platform-known name; only the protocol names resolve.
        assert_eq!(contract_id_by_name("temp.celsius"), None);
        assert_eq!(contract_id_by_name(""), None);
        assert_eq!(contract_id_by_name("cdz-platform.deliver"), None);
    }
}
