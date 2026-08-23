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

use super::checker::CheckOutcome;
use crate::contract_value::{bytes_leaf, read_bytes};
use crate::{Bytes, Contract, ContractId, Str};
use cadenza_ast::ast::{Arenas, Builder, Leaf, Struct, StructId};
use cadenza_ast::codec;
use std::sync::{Arc, OnceLock};

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

/// Encode the payload of a `check` Message: the serialized observation log wrapped in the check envelope
/// (`Envelope.Check(<log>)`). This is what the harness delivers to the checker; `log` is the output of
/// [`serialize_log`](super::serialize_log).
#[must_use]
pub fn encode_check(log: &[u8]) -> Bytes {
    let mut b = Builder::new();
    let inner = bytes_leaf(&mut b, log);
    let root = crate::contracts::check::envelope_check(&mut b, inner);
    Bytes::from(codec::encode(&b.finish(root)))
}

/// Decode a `check` Message payload back to the serialized observation log it carries, or `None` if the
/// bytes are not a well-formed check envelope. Total — the inverse of [`encode_check`].
#[must_use]
pub fn decode_check(bytes: &[u8]) -> Option<Bytes> {
    let arenas = codec::decode(bytes)?;
    let inner = crate::contracts::check::as_envelope_check(&arenas, arenas.root)?;
    read_bytes(&arenas, inner)
}

/// Encode the payload of a `verdict` Request: the checker's pass/fail plus its messages
/// (`Verdict.Verdict({ pass, messages })`). This is what a checker emits and the harness reads.
#[must_use]
pub fn encode_verdict(pass: bool, messages: &[Str]) -> Bytes {
    let mut b = Builder::new();
    let pass_leaf = b.atom_leaf(Leaf::Bool(pass));
    let items: Vec<StructId> = messages
        .iter()
        .map(|m| b.atom_leaf(Leaf::Str(Arc::from(m.as_str()))))
        .collect();
    let messages_leaf = string_list(&mut b, items);
    let root = crate::contracts::verdict::verdict_verdict(
        &mut b,
        crate::contracts::verdict::VerdictVerdict {
            pass: pass_leaf,
            messages: messages_leaf,
        },
    );
    Bytes::from(codec::encode(&b.finish(root)))
}

/// Decode a `verdict` Request payload into a [`CheckOutcome`], or `None` if the bytes are not a well-formed
/// verdict. A `pass` verdict is [`CheckOutcome::Pass`]; a failing one carries its messages as the fail
/// reasons — the bridge from the wire verdict a checker emits to the native verdict the harness reports.
#[must_use]
pub fn decode_verdict(bytes: &[u8]) -> Option<CheckOutcome> {
    let arenas = codec::decode(bytes)?;
    let v = crate::contracts::verdict::as_verdict_verdict(&arenas, arenas.root)?;
    let pass = read_bool(&arenas, v.pass)?;
    let messages = read_string_list(&arenas, v.messages)?;
    Some(if pass {
        CheckOutcome::Pass
    } else {
        CheckOutcome::Fail {
            reasons: messages
                .into_iter()
                .map(|s| s.as_str().to_string())
                .collect(),
        }
    })
}

/// A `("list" e…)` value — the canonical list constructor (string head).
fn string_list(b: &mut Builder, items: Vec<StructId>) -> StructId {
    let head = b.atom_leaf(Leaf::Str(Arc::from("list")));
    b.list(std::iter::once(head).chain(items).collect())
}

/// Read a `Bool` leaf, or `None` if `id` is not one.
fn read_bool(arenas: &Arenas, id: StructId) -> Option<bool> {
    match arenas.get(id) {
        Struct::Atom(leaf) => match arenas.leaf(*leaf) {
            Leaf::Bool(v) => Some(*v),
            _ => None,
        },
        Struct::List(_) => None,
    }
}

/// Read a `List(String)` value into its strings, or `None` if `id` is not a string list. Accepts both the
/// string head `("list" …)` and the bare name head `(list …)`.
fn read_string_list(arenas: &Arenas, id: StructId) -> Option<Vec<Str>> {
    let items = arenas
        .as_ctor_form(id, "list")
        .or_else(|| arenas.as_form(id, "list"))?;
    items
        .iter()
        .map(|&e| arenas.as_str(e).map(Str::from))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        check_contract, decode_check, decode_verdict, encode_check, encode_verdict,
        verdict_contract,
    };
    use crate::Str;
    use crate::testing::CheckOutcome;

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

    #[test]
    fn a_check_payload_round_trips_the_serialized_log() {
        // The check Message carries the whole serialized log as opaque bytes; it reads back byte-exact.
        let log = b"\x00serialized-observation-log\xff";
        assert_eq!(
            decode_check(&encode_check(log)).as_deref(),
            Some(log.as_slice())
        );
        // Not a check envelope → rejected, not a panic.
        assert_eq!(decode_check(b"junk"), None);
    }

    #[test]
    fn a_passing_verdict_round_trips_to_pass() {
        let outcome = decode_verdict(&encode_verdict(true, &[])).expect("a well-formed verdict");
        assert_eq!(outcome, CheckOutcome::Pass);
        // Messages on a pass are dropped (Pass carries none) — the verdict is the pass/fail decision.
        let outcome = decode_verdict(&encode_verdict(true, &[Str::from("note")])).expect("verdict");
        assert_eq!(outcome, CheckOutcome::Pass);
    }

    #[test]
    fn a_failing_verdict_round_trips_its_messages_as_reasons() {
        let messages = [Str::from("token != own id"), Str::from("wrong contract")];
        let outcome = decode_verdict(&encode_verdict(false, &messages)).expect("verdict");
        assert_eq!(
            outcome,
            CheckOutcome::Fail {
                reasons: vec!["token != own id".to_string(), "wrong contract".to_string()],
            }
        );
    }

    #[test]
    fn a_malformed_verdict_is_rejected() {
        assert_eq!(decode_verdict(b"not a verdict"), None);
    }
}
