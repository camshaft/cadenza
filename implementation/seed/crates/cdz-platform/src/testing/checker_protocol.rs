//! The checker protocol contracts (`design/cadenza-platform.md` §9).
//!
//! The operator's design for the end-of-run wasm checker reuses the reducer interface: the checker is a
//! reducer-shaped guest that is delivered the whole observation log and emits a pass/fail verdict, both as
//! ordinary contract values. Two contracts carry that protocol, defined as real Cadenza schemas in
//! `contracts/check.cdz` and `contracts/verdict.cdz` (codegen-validated, their ids the hash of the declared
//! schema — the same discipline as the built-in `deliver`/`timer`/`spawned` contracts):
//!
//! - [`check_contract`] — the harness delivers a `Message` on this contract to the checker at end-of-run;
//!   its payload is the serialized observation log ([`serialize_log`](super::serialize_log)).
//! - [`verdict_contract`] — the checker emits a `Request` on this contract to report the run's pass/fail
//!   with messages; the harness reads it from the observation stream (it is a driver, not a reducer, so it
//!   cannot receive a reply — a separate emitted contract is the mechanism).
//!
//! These are the ids that route the two events; the value builders/readers for their schemas are generated
//! into `crate::contracts::{check, verdict}`. The native checker-driver that spawns a checker, delivers the
//! log, and reads the verdict is built over these.

use crate::{Contract, ContractId, Str};
use std::sync::OnceLock;

/// The contract that carries the whole observation log to the checker (§9). A `Message` on it, whose payload
/// is the serialized log, is delivered to the checker at end-of-run; the checker folds it via `on_message`.
/// Its id is the hash of the declared `check` schema (`contracts/check.cdz`), built and cached once.
#[must_use]
pub fn check_contract() -> ContractId {
    static CHECK: OnceLock<Contract> = OnceLock::new();
    CHECK
        .get_or_init(|| {
            Contract::new(
                Str::from_static("cdz-platform.check"),
                crate::contracts::check::schema,
                "Envelope",
                "Ack",
            )
        })
        .id()
}

/// The contract the checker emits its verdict on (§9): a `Request` whose payload is a `Verdict` record
/// `{ pass, messages }`. The harness reads this emitted request to decide the run's pass/fail. Its id is the
/// hash of the declared `verdict` schema (`contracts/verdict.cdz`), built and cached once.
#[must_use]
pub fn verdict_contract() -> ContractId {
    static VERDICT: OnceLock<Contract> = OnceLock::new();
    VERDICT
        .get_or_init(|| {
            Contract::new(
                Str::from_static("cdz-platform.verdict"),
                crate::contracts::verdict::schema,
                "Verdict",
                "Ack",
            )
        })
        .id()
}

#[cfg(test)]
mod tests {
    use super::{check_contract, verdict_contract};

    #[test]
    fn each_contract_id_is_stable_and_distinct() {
        // A contract id is the hash of its declared schema — stable across calls (cached), and distinct
        // between the two protocol contracts and from the built-in deliver contract.
        assert_eq!(check_contract(), check_contract());
        assert_eq!(verdict_contract(), verdict_contract());
        assert_ne!(check_contract(), verdict_contract());
        assert_ne!(check_contract(), crate::deliver_contract());
        assert_ne!(verdict_contract(), crate::deliver_contract());
    }
}
