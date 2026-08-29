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
use super::log_value;
use super::observation::{Entry, EventKind, EventOp, Record};
use crate::contract_value::{as_ascribed, ascribe, bytes_leaf, read_bytes, read_hash};
use crate::{Bytes, Contract, ContractId, Delivered, HostId, Message, Origin, ReducerId, Str};
use cadenza_ast::ast::{Arenas, Builder, CompoundCtor, Leaf, Struct, StructId};
use cadenza_ast::codec;
use std::sync::{Arc, OnceLock};

/// The synthetic origin the harness stamps on the `check` message it delivers to a checker — the checker
/// was handed the log by the driver, not by a real peer reducer. A checker routes on the check contract,
/// not on `from`, so this is fixed and reproducible.
fn driver_origin() -> Origin {
    Origin {
        reducer: ReducerId::of(b"cdz-platform.harness.checker-driver"),
        host: HostId::of(b"cdz-platform.harness.checker-driver"),
    }
}

/// The contract that carries the whole observation log to the checker (§9). A `Message` on it, whose payload
/// is the serialized log, is delivered to the checker at end-of-run; the checker folds it via `on_message`.
/// Its id is the hash of the declared `check` schema (`contracts/check.cdz`), built and cached once.
#[must_use]
pub fn check_contract() -> ContractId {
    static CHECK: OnceLock<Contract> = OnceLock::new();
    CHECK.get_or_init(crate::contracts::check::contract).id()
}

/// The contract the checker emits its verdict on (§9): a `Request` whose payload is a `Verdict` record
/// `{ pass, messages }`. The harness reads this emitted request to decide the run's pass/fail. Its id is the
/// hash of the declared `verdict` schema (`contracts/verdict.cdz`), built and cached once.
#[must_use]
pub fn verdict_contract() -> ContractId {
    static VERDICT: OnceLock<Contract> = OnceLock::new();
    VERDICT
        .get_or_init(crate::contracts::verdict::contract)
        .id()
}

/// Encode the payload of a `check` Message: the serialized observation log plus the contract-id the checker
/// emits its verdict on, wrapped in the check envelope (`Envelope.Check({ log, verdict })`). The harness
/// passes `verdict` so the checker reads *where to report* from the same message that hands it the log —
/// neither hardcoding a hash nor computing it from a compiled-in schema. `log` is the output of
/// [`serialize_log`](super::serialize_log).
#[must_use]
pub fn encode_check(log: &[u8], verdict: ContractId) -> Bytes {
    let mut b = Builder::new();
    let log_leaf = bytes_leaf(&mut b, log);
    let verdict_leaf = bytes_leaf(&mut b, verdict.hash().as_bytes());
    let value = crate::contracts::check::envelope_check(
        &mut b,
        crate::contracts::check::EnvelopeCheck {
            log: log_leaf,
            verdict: verdict_leaf,
        },
    );
    // The checker guest `Value.decode`s this payload into an `Envelope`, so wrap it in the root ascription
    // the value decoder requires (`(: <value> Envelope)`); the type token is name-agnostic.
    let root = ascribe(&mut b, value, "Envelope");
    Bytes::from(codec::encode(&b.finish(root)))
}

/// Decode a `check` Message payload back to the serialized observation log it carries, or `None` if the
/// bytes are not a well-formed check envelope. Total — reads the `log` field of [`encode_check`]'s envelope.
#[must_use]
pub fn decode_check(bytes: &[u8]) -> Option<Bytes> {
    let arenas = codec::decode(bytes)?;
    let root = as_ascribed(&arenas, arenas.root).unwrap_or(arenas.root);
    let env = crate::contracts::check::as_envelope_check(&arenas, root)?;
    read_bytes(&arenas, env.log)
}

/// The verdict contract-id a check envelope carries — where the checker is told to emit its verdict. `None`
/// if the bytes are not a well-formed check envelope or its verdict field is not a hash. The wasm checker
/// guest reads this the same way (via the generated envelope reader), so it need not hardcode the id.
#[must_use]
pub fn decode_check_verdict(bytes: &[u8]) -> Option<ContractId> {
    let arenas = codec::decode(bytes)?;
    let root = as_ascribed(&arenas, arenas.root).unwrap_or(arenas.root);
    let env = crate::contracts::check::as_envelope_check(&arenas, root)?;
    Some(ContractId::from_hash(read_hash(&arenas, env.verdict)?))
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
    let value = crate::contracts::verdict::verdict_verdict(
        &mut b,
        crate::contracts::verdict::VerdictVerdict {
            pass: pass_leaf,
            messages: messages_leaf,
        },
    );
    // Symmetric with the verdict a guest emits (its `Value.encode` wraps the root), so a native verdict and
    // a guest verdict decode by the same path (`(: <value> Verdict)`, name-agnostic token).
    let root = ascribe(&mut b, value, "Verdict");
    Bytes::from(codec::encode(&b.finish(root)))
}

/// Decode a `verdict` Request payload into a [`CheckOutcome`], or `None` if the bytes are not a well-formed
/// verdict. A `pass` verdict is [`CheckOutcome::Pass`]; a failing one carries its messages as the fail
/// reasons — the bridge from the wire verdict a checker emits to the native verdict the harness reports.
#[must_use]
pub fn decode_verdict(bytes: &[u8]) -> Option<CheckOutcome> {
    let arenas = codec::decode(bytes)?;
    // A verdict from a Cadenza guest is `Value.encode`d (root ascription); a native `encode_verdict` matches.
    let root = as_ascribed(&arenas, arenas.root).unwrap_or(arenas.root);
    let v = crate::contracts::verdict::as_verdict_verdict(&arenas, root)?;
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

/// The `check` [`Message`] that delivers a whole observation log to a checker to fold and verdict on: a
/// message on the [`check_contract`] whose payload is the serialized log
/// ([`serialize_log`](super::serialize_log)). Deliver this to a spawned checker reducer — it folds it via
/// `on_message` and emits a [`verdict_contract`] request. The two ends of the checker protocol: this is what
/// goes in, [`verdict_in`] reads what comes out.
#[must_use]
pub fn check_message(log: &[Record]) -> Delivered {
    Delivered::Message(Message {
        id: check_contract(),
        payload: encode_check(&log_value::serialize(log), verdict_contract()),
        from: driver_origin(),
        continuation_token: Bytes::new(),
    })
}

/// The verdict a checker emitted while folding, read from an observation log: the payload of the first
/// [`Emitted`](EventOp::Emitted) request on the [`verdict_contract`], decoded into a [`CheckOutcome`].
/// `None` if the checker emitted no verdict — it never reported, which a caller treats as a failed check.
/// This is how the harness (a driver, not a reducer, so it cannot receive a reply) reads the checker's
/// judgement out of the run it drove.
#[must_use]
pub fn verdict_in(records: &[Record]) -> Option<CheckOutcome> {
    let verdict = verdict_contract();
    records.iter().find_map(|r| match &r.entry {
        Entry::Event(EventOp::Emitted {
            contract, payload, ..
        }) if *contract == verdict => decode_verdict(payload),
        _ => None,
    })
}

/// Diagnose *why* a checker produced no verdict, from its run's observation log — turning the bare "the
/// checker emitted no verdict" into an actionable reason. Called by [`run_checker`](super::run_checker) when
/// [`verdict_in`] is `None`; it reads what the checker actually *did* and reports the most specific cause:
/// it emitted a malformed verdict, its fold faulted, it emitted on the wrong contract, it closed without
/// reporting (the common path when `on_message` decodes neither the envelope nor the log and hits a close
/// arm), or it produced nothing at all (it never ran). Without this, a decode-failure and a never-ran
/// checker are indistinguishable — the difference this incident's triage turned on (§9).
#[must_use]
pub fn no_verdict_reason(records: &[Record]) -> String {
    let verdict = verdict_contract();

    // A verdict WAS emitted on the verdict contract, but its payload did not decode as a `Verdict`
    // (`verdict_in` skipped it) — the checker reported, but the verdict value itself is malformed.
    if records.iter().any(|r| {
        matches!(&r.entry, Entry::Event(EventOp::Emitted { contract, payload, .. })
            if *contract == verdict && decode_verdict(payload).is_none())
    }) {
        return "the checker emitted on the verdict contract, but its payload did not decode as a \
                Verdict(pass, messages) — the verdict value is malformed"
            .to_string();
    }

    // The checker's fold failed uncontrolled (panic / fuel) — it could not emit anything.
    if let Some((during, contract, reason)) = records.iter().find_map(|r| match &r.entry {
        Entry::Event(EventOp::Failed {
            during,
            contract,
            reason,
        }) => Some((*during, *contract, reason.clone())),
        _ => None,
    }) {
        return format!(
            "the checker faulted while folding the {} on {} (so it never emitted a verdict): {reason}",
            event_kind_name(during),
            short_id(&contract),
        );
    }

    // The checker emitted requests, but none on the verdict contract — it reported on the wrong contract.
    let other: Vec<String> = records
        .iter()
        .filter_map(|r| match &r.entry {
            Entry::Event(EventOp::Emitted { contract, .. }) if *contract != verdict => {
                Some(short_id(contract))
            }
            _ => None,
        })
        .collect();
    if !other.is_empty() {
        return format!(
            "the checker emitted no verdict; it emitted on {} instead of the verdict contract {}",
            other.join(", "),
            short_id(&verdict),
        );
    }

    // The checker closed (`Break`) without emitting a verdict — commonly its `on_message` decoded neither
    // the check envelope nor the log and hit a close arm (the delivered value did not decode).
    if let Some(schema) = records.iter().find_map(|r| match &r.entry {
        Entry::Event(EventOp::Closed { schema, .. }) => Some(*schema),
        _ => None,
    }) {
        return format!(
            "the checker closed (schema {}) without emitting a verdict — commonly its on_message decoded \
             neither the check envelope nor the log and hit a close arm",
            short_id(&schema),
        );
    }

    // No emit, close, or fault at all — the checker produced nothing, so it most likely never ran.
    "the checker emitted no verdict and produced no observations — it likely never ran (its bytes may not \
     be a valid reducer component)"
        .to_string()
}

/// The name of an [`EventKind`] for a diagnostic message.
fn event_kind_name(kind: EventKind) -> &'static str {
    match kind {
        EventKind::Message => "message",
        EventKind::Response => "response",
        EventKind::Notification => "notification",
    }
}

/// A contract-id rendered short for a diagnostic: its base62 hash, elided after a prefix (base62 is ASCII,
/// so a byte prefix is a char boundary).
fn short_id(id: &ContractId) -> String {
    let s = id.to_string();
    match s.get(..10) {
        Some(prefix) if prefix.len() < s.len() => format!("{prefix}…"),
        _ => s,
    }
}

/// A `(list e…)` value — the canonical NAME-headed list a guest `Value.decode`s (the verdict's `messages`
/// field), matching the form a Cadenza guest's own `Value.encode` produces.
fn string_list(b: &mut Builder, items: Vec<StructId>) -> StructId {
    let head = b.name("list");
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
    // All three list spellings incl. the M2 native ctor-leaf head (rcdzc-compiled checker values).
    let items = arenas.compound_form_of(id, CompoundCtor::List)?;
    items
        .iter()
        .map(|&e| arenas.as_str(e).map(Str::from))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        check_contract, check_message, decode_check, decode_check_verdict, decode_verdict,
        encode_check, encode_verdict, no_verdict_reason, verdict_contract, verdict_in,
    };
    use crate::testing::{CheckOutcome, Entry, EventKind, EventOp, Record, deserialize_log};
    use crate::{Bytes, ContractId, Delivered, HostId, Origin, ReducerId, Str};

    /// A one-record observation log — an emitted event, enough to prove the whole log survives wrapping.
    fn a_log() -> Vec<Record> {
        vec![Record {
            seq: 0,
            time_ns: 5,
            source: Origin {
                reducer: ReducerId::of(b"agent"),
                host: HostId::of(b"node"),
            },
            entry: Entry::Event(EventOp::Emitted {
                contract: ContractId::of(b"http.get"),
                payload: Bytes::from_static(b"url"),
                continuation_token: Bytes::from_static(b"c"),
                has_deadline: false,
            }),
        }]
    }

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
    fn a_check_payload_round_trips_the_serialized_log_and_the_verdict_contract() {
        // The check Message carries the whole serialized log as opaque bytes plus the verdict contract-id;
        // both read back exactly. The verdict-id is how the checker learns where to emit its verdict.
        let log = b"\x00serialized-observation-log\xff";
        let verdict = ContractId::of(b"where-to-report");
        let payload = encode_check(log, verdict);
        assert_eq!(decode_check(&payload).as_deref(), Some(log.as_slice()));
        assert_eq!(decode_check_verdict(&payload), Some(verdict));
        // Not a check envelope → rejected, not a panic.
        assert_eq!(decode_check(b"junk"), None);
        assert_eq!(decode_check_verdict(b"junk"), None);
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

    #[test]
    fn check_message_delivers_the_serialized_log_on_the_check_contract() {
        // The check Message is on the check contract and carries the whole log; unwrapping its payload and
        // deserializing recovers the exact log the checker will fold.
        let log = a_log();
        let Delivered::Message(m) = check_message(&log) else {
            panic!("a check is delivered as a Message");
        };
        assert_eq!(m.id, check_contract());
        let inner = decode_check(&m.payload).expect("a check envelope");
        assert_eq!(deserialize_log(&inner), Some(log));
    }

    #[test]
    fn verdict_in_reads_the_emitted_verdict_from_the_log() {
        // A checker's verdict is read from the Emitted request it made on the verdict contract.
        let emitted = |payload: Bytes| Record {
            seq: 0,
            time_ns: 0,
            source: Origin {
                reducer: ReducerId::of(b"checker"),
                host: HostId::of(b"node"),
            },
            entry: Entry::Event(EventOp::Emitted {
                contract: verdict_contract(),
                payload,
                continuation_token: Bytes::new(),
                has_deadline: false,
            }),
        };
        let fail = vec![emitted(encode_verdict(false, &[Str::from("nope")]))];
        assert_eq!(
            verdict_in(&fail),
            Some(CheckOutcome::Fail {
                reasons: vec!["nope".to_string()],
            })
        );
        let pass = vec![emitted(encode_verdict(true, &[]))];
        assert_eq!(verdict_in(&pass), Some(CheckOutcome::Pass));
        // An emit on a different contract is not a verdict; a log with no verdict yields None.
        let other = vec![Record {
            entry: Entry::Event(EventOp::Emitted {
                contract: ContractId::of(b"other"),
                payload: Bytes::new(),
                continuation_token: Bytes::new(),
                has_deadline: false,
            }),
            ..emitted(Bytes::new())
        }];
        assert_eq!(verdict_in(&other), None);
        assert_eq!(verdict_in(&[]), None);
    }

    /// A record whose entry is `entry`, at seq 0 from a fixed source — enough for the no-verdict diagnosis.
    fn rec(entry: Entry) -> Record {
        Record {
            seq: 0,
            time_ns: 0,
            source: Origin {
                reducer: ReducerId::of(b"checker"),
                host: HostId::of(b"node"),
            },
            entry,
        }
    }

    #[test]
    fn no_verdict_reason_reports_a_close_without_a_verdict() {
        // The common path: the checker's on_message hit a close arm (e.g. the delivered value did not
        // decode) and emitted nothing — the ambiguity this incident's triage turned on.
        let reason = no_verdict_reason(&[rec(Entry::Event(EventOp::Closed {
            schema: check_contract(),
            reason: Bytes::from_static(b"undecodable"),
        }))]);
        assert!(reason.contains("closed"), "{reason}");
        assert!(reason.contains("close arm"), "{reason}");
    }

    #[test]
    fn no_verdict_reason_reports_a_fault() {
        let reason = no_verdict_reason(&[rec(Entry::Event(EventOp::Failed {
            during: EventKind::Message,
            contract: check_contract(),
            reason: Str::from("index out of bounds"),
        }))]);
        assert!(reason.contains("faulted"), "{reason}");
        assert!(reason.contains("index out of bounds"), "{reason}");
    }

    #[test]
    fn no_verdict_reason_reports_an_emit_on_the_wrong_contract() {
        // The checker emitted, but on some other contract — not the verdict contract.
        let reason = no_verdict_reason(&[rec(Entry::Event(EventOp::Emitted {
            contract: ContractId::of(b"some.other.contract"),
            payload: Bytes::from_static(b"x"),
            continuation_token: Bytes::new(),
            has_deadline: false,
        }))]);
        assert!(
            reason.contains("wrong") || reason.contains("instead of"),
            "{reason}"
        );
    }

    #[test]
    fn no_verdict_reason_reports_a_malformed_verdict() {
        // An emit ON the verdict contract, but the payload is not a decodable Verdict.
        let reason = no_verdict_reason(&[rec(Entry::Event(EventOp::Emitted {
            contract: verdict_contract(),
            payload: Bytes::from_static(b"not a verdict value"),
            continuation_token: Bytes::new(),
            has_deadline: false,
        }))]);
        assert!(reason.contains("malformed"), "{reason}");
    }

    #[test]
    fn no_verdict_reason_reports_a_checker_that_never_ran() {
        // No emit, close, or fault — the checker produced nothing.
        let reason = no_verdict_reason(&[]);
        assert!(reason.contains("never ran"), "{reason}");
    }

    #[test]
    fn no_verdict_reason_prefers_a_real_emit_over_a_close() {
        // A checker that emitted on the wrong contract AND closed: the wrong-contract emit is the more
        // actionable signal, so it wins over the generic close.
        let records = vec![
            rec(Entry::Event(EventOp::Emitted {
                contract: ContractId::of(b"wrong"),
                payload: Bytes::new(),
                continuation_token: Bytes::new(),
                has_deadline: false,
            })),
            rec(Entry::Event(EventOp::Closed {
                schema: check_contract(),
                reason: Bytes::new(),
            })),
        ];
        let reason = no_verdict_reason(&records);
        assert!(reason.contains("instead of"), "{reason}");
    }
}
