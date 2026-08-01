//! Map a kernel [`Event`] to/from a Cadenza AST ([`cadenza_ast::ast::Arenas`]) so the durable log can
//! be encoded through the ONE shared, spec-pinned canonical codec (`cadenza_ast::codec`, header
//! `cdzast\x00\x01`, ast-encoding.md) instead of the bespoke `Event::encode` (§19a: the kernel, the
//! text front-end, and rcdzc all share one wire format rather than parallel encodings that drift).
//!
//! This is the MAPPING layer `log_store` frames each event through — `log_store::append` calls
//! [`encode`] and `decode_frames` calls [`decode`], so the durable log IS this shared-codec form. An
//! exhaustive round-trip test proves every event variant survives Event → Arenas → bytes → Arenas →
//! Event unchanged, and that a malformed encoding is classified (not panicked on) as an `Err`. Note the
//! torn-vs-corrupt SPLIT is the FRAMING layer's job, not this codec's error type: `decode_frames`
//! detects a torn tail by a short read (a truncated length-prefix/body — it never even calls `decode`),
//! and treats ANY `decode` failure on a COMPLETE frame as corruption. So this codec's error only means
//! "this whole frame is bad" (→ Corrupt); the torn side is decided before decode is reached.
//!
//! Encoding shape (an s-expr form per event, head = a `Name` tag):
//! - `(event <seq> <cause> <body>)` — `seq` an Int; `cause` is `(none)` or `(hash <bytes>)`.
//! - body is one of:
//!   `(genesis <hash>)` ·
//!   `(inbound <family:str> <version:int> <payload>)` ·
//!   `(dispatched <id:int> <kind> <target:str> <idem:hash> <deadline>)` ·
//!   `(effect-result <id:int> <outcome>)` ·
//!   `(timer-armed <id:int> <deadline_ms:int>)` ·
//!   `(timer-fired <id:int> <fired_ms:int>)` ·
//!   `(authz-denied <id:int> <reason:str>)` ·
//!   `(closed <payload>)`
//! - `<kind>` a `Name` atom (shell/http/model/now/timer/emit); `<deadline>` = `(none)` | `(ms <int>)`;
//!   `<payload>` = `(inline <bytes>)` | `(blob <hash>)`; `<outcome>` = `(ok <payload-opt>)` |
//!   `(err <str>)` | `(timed-out)`; `<payload-opt>` = `(none)` | `(some <payload>)`.
//!
//! Hashes + inline payloads ride as `Leaf::Bytes` (arbitrary bytes, not necessarily UTF-8 — the right
//! value form). u64 ids/versions/deadlines ride as `Leaf::Int` (a `BigInt`, decoded back with a range
//! check so an out-of-u64 value is a clean `Corrupt`, never a panic).

use crate::effect::{EffectId, EffectKind, Payload};
use crate::event::{ContentType, EffectOutcome, Event, EventBody};
use crate::hash::Hash;
use cadenza_ast::ast::{Arenas, Builder, Leaf, Radix, Struct, StructId};
use cadenza_ast::codec;
use num_bigint::BigInt;

/// Why an `Arenas`/byte stream isn't a well-formed kernel event. Total: the mapping never panics on bad
/// input. Mirrors the shape a log reader needs — a torn write (from `decode_detailed`'s `Truncated`) vs.
/// structural corruption (everything else, incl. a valid AST that isn't a valid EVENT shape).
#[derive(Debug, PartialEq, Eq)]
pub enum EventAstError {
    /// The bytes weren't a decodable canonical AST at all — carries the codec's reason so a caller can
    /// tell a torn tail (`Truncated`) from corruption (every other variant).
    Codec(codec::DecodeError),
    /// The bytes decoded to a valid AST, but its SHAPE isn't a kernel event (wrong head, arity, a leaf
    /// where a form was expected, an int out of u64 range, an unknown tag). Always corruption.
    Shape(&'static str),
}

/// Encode an event to the shared canonical binary form (`cadenza_ast::codec::encode` of its AST).
pub fn encode(event: &Event) -> Vec<u8> {
    let mut b = Builder::new();
    let root = build_event(&mut b, event);
    codec::encode(&b.finish(root))
}

/// Decode an event from the shared canonical binary form. `Truncated` → `Codec` (torn); any other codec
/// error or a bad event shape → corruption. Total (never panics).
pub fn decode(bytes: &[u8]) -> Result<Event, EventAstError> {
    let arenas = codec::decode_detailed(bytes).map_err(EventAstError::Codec)?;
    read_event(&arenas, arenas.root)
}

// ---- encode (Event → Arenas) ------------------------------------------------------------------------

fn u64_leaf(b: &mut Builder, n: u64) -> StructId {
    b.atom_leaf(Leaf::Int {
        value: BigInt::from(n),
        radix: Radix::Dec,
    })
}

fn bytes_leaf(b: &mut Builder, bytes: &[u8]) -> StructId {
    b.atom_leaf(Leaf::Bytes(bytes.to_vec()))
}

fn str_leaf(b: &mut Builder, s: &str) -> StructId {
    b.atom_leaf(Leaf::Str(s.to_string()))
}

fn hash_form(b: &mut Builder, h: &Hash) -> StructId {
    let head = b.name("hash");
    let bytes = bytes_leaf(b, h.as_bytes());
    b.list(vec![head, bytes])
}

fn payload_form(b: &mut Builder, p: &Payload) -> StructId {
    match p {
        Payload::Inline(bytes) => {
            let head = b.name("inline");
            let v = bytes_leaf(b, bytes);
            b.list(vec![head, v])
        }
        Payload::Blob(h) => {
            let head = b.name("blob");
            let hf = hash_form(b, h);
            b.list(vec![head, hf])
        }
    }
}

fn opt_payload_form(b: &mut Builder, p: &Option<Payload>) -> StructId {
    match p {
        None => {
            let head = b.name("none");
            b.list(vec![head])
        }
        Some(p) => {
            let head = b.name("some");
            let pf = payload_form(b, p);
            b.list(vec![head, pf])
        }
    }
}

fn kind_atom(b: &mut Builder, kind: &EffectKind) -> StructId {
    b.name(match kind {
        EffectKind::Shell => "shell",
        EffectKind::Http => "http",
        EffectKind::Model => "model",
        EffectKind::Now => "now",
        EffectKind::Timer => "timer",
        EffectKind::Emit => "emit",
    })
}

fn outcome_form(b: &mut Builder, o: &EffectOutcome) -> StructId {
    match o {
        EffectOutcome::Ok(p) => {
            let head = b.name("ok");
            let pf = opt_payload_form(b, p);
            b.list(vec![head, pf])
        }
        EffectOutcome::Err(msg) => {
            let head = b.name("err");
            let m = str_leaf(b, msg);
            b.list(vec![head, m])
        }
        EffectOutcome::TimedOut => {
            let head = b.name("timed-out");
            b.list(vec![head])
        }
    }
}

fn opt_ms_form(b: &mut Builder, ms: Option<u64>) -> StructId {
    match ms {
        None => {
            let head = b.name("none");
            b.list(vec![head])
        }
        Some(v) => {
            let head = b.name("ms");
            let n = u64_leaf(b, v);
            b.list(vec![head, n])
        }
    }
}

fn body_form(b: &mut Builder, body: &EventBody) -> StructId {
    match body {
        EventBody::Genesis { reducer } => {
            let head = b.name("genesis");
            let h = hash_form(b, reducer);
            b.list(vec![head, h])
        }
        EventBody::Inbound {
            content_type,
            payload,
        } => {
            let head = b.name("inbound");
            let fam = str_leaf(b, &content_type.family);
            let ver = u64_leaf(b, content_type.version as u64);
            let pf = payload_form(b, payload);
            b.list(vec![head, fam, ver, pf])
        }
        EventBody::Dispatched {
            id,
            kind,
            target,
            idempotency_key,
            deadline_ms,
        } => {
            let head = b.name("dispatched");
            let idv = u64_leaf(b, id.0);
            let k = kind_atom(b, kind);
            let t = str_leaf(b, target);
            let idem = hash_form(b, idempotency_key);
            let dl = opt_ms_form(b, *deadline_ms);
            b.list(vec![head, idv, k, t, idem, dl])
        }
        EventBody::EffectResult { id, result } => {
            let head = b.name("effect-result");
            let idv = u64_leaf(b, id.0);
            let o = outcome_form(b, result);
            b.list(vec![head, idv, o])
        }
        EventBody::TimerArmed { id, deadline_ms } => {
            let head = b.name("timer-armed");
            let idv = u64_leaf(b, id.0);
            let dl = u64_leaf(b, *deadline_ms);
            b.list(vec![head, idv, dl])
        }
        EventBody::TimerFired { id, fired_ms } => {
            let head = b.name("timer-fired");
            let idv = u64_leaf(b, id.0);
            let fm = u64_leaf(b, *fired_ms);
            b.list(vec![head, idv, fm])
        }
        EventBody::AuthzDenied { id, reason } => {
            let head = b.name("authz-denied");
            let idv = u64_leaf(b, id.0);
            let r = str_leaf(b, reason);
            b.list(vec![head, idv, r])
        }
        EventBody::Closed { outcome } => {
            let head = b.name("closed");
            let pf = payload_form(b, outcome);
            b.list(vec![head, pf])
        }
    }
}

fn build_event(b: &mut Builder, event: &Event) -> StructId {
    let head = b.name("event");
    let seq = u64_leaf(b, event.seq);
    let cause = match &event.cause {
        None => {
            let h = b.name("none");
            b.list(vec![h])
        }
        Some(c) => hash_form(b, c),
    };
    let body = body_form(b, &event.body);
    b.list(vec![head, seq, cause, body])
}

// ---- decode (Arenas → Event) ------------------------------------------------------------------------

// A tiny read helper set over `Arenas`, all returning `EventAstError::Shape` on any mismatch so the
// mapping is total. `as_form(id, head)` (from cadenza_ast) checks the head Name AND returns the child
// slice, so head+arity are validated together.

fn shape(msg: &'static str) -> EventAstError {
    EventAstError::Shape(msg)
}

/// The children of a `(head …)` form, verifying the head — else a Shape error.
fn form<'a>(
    a: &'a Arenas,
    id: StructId,
    head: &'static str,
) -> Result<&'a [StructId], EventAstError> {
    a.as_form(id, head).ok_or(shape(head))
}

fn read_u64(a: &Arenas, id: StructId) -> Result<u64, EventAstError> {
    match a.get(id) {
        Struct::Atom(leaf) => match a.leaf(*leaf) {
            Leaf::Int { value, .. } => {
                u64::try_from(value).map_err(|_| shape("int out of u64 range"))
            }
            _ => Err(shape("expected int leaf")),
        },
        _ => Err(shape("expected atom")),
    }
}

fn read_bytes(a: &Arenas, id: StructId) -> Result<Vec<u8>, EventAstError> {
    match a.get(id) {
        Struct::Atom(leaf) => match a.leaf(*leaf) {
            Leaf::Bytes(bytes) => Ok(bytes.clone()),
            _ => Err(shape("expected bytes leaf")),
        },
        _ => Err(shape("expected atom")),
    }
}

fn read_str(a: &Arenas, id: StructId) -> Result<String, EventAstError> {
    a.as_str(id)
        .map(|s| s.to_string())
        .ok_or(shape("expected str"))
}

fn read_hash(a: &Arenas, id: StructId) -> Result<Hash, EventAstError> {
    let [bytes] = form(a, id, "hash")? else {
        return Err(shape("hash arity"));
    };
    let raw = read_bytes(a, *bytes)?;
    let arr: [u8; 32] = raw.try_into().map_err(|_| shape("hash not 32 bytes"))?;
    Ok(Hash::from_bytes(arr))
}

/// Head name of a form (for dispatching on a variant tag).
fn head_of(a: &Arenas, id: StructId) -> Result<&str, EventAstError> {
    a.head_name(id).ok_or(shape("expected a headed form"))
}

fn read_payload(a: &Arenas, id: StructId) -> Result<Payload, EventAstError> {
    match head_of(a, id)? {
        "inline" => {
            let [v] = form(a, id, "inline")? else {
                return Err(shape("inline arity"));
            };
            Ok(Payload::Inline(read_bytes(a, *v)?))
        }
        "blob" => {
            let [h] = form(a, id, "blob")? else {
                return Err(shape("blob arity"));
            };
            Ok(Payload::Blob(read_hash(a, *h)?))
        }
        _ => Err(shape("unknown payload tag")),
    }
}

fn read_opt_payload(a: &Arenas, id: StructId) -> Result<Option<Payload>, EventAstError> {
    match head_of(a, id)? {
        "none" => Ok(None),
        "some" => {
            let [p] = form(a, id, "some")? else {
                return Err(shape("some arity"));
            };
            Ok(Some(read_payload(a, *p)?))
        }
        _ => Err(shape("unknown opt-payload tag")),
    }
}

fn read_kind(a: &Arenas, id: StructId) -> Result<EffectKind, EventAstError> {
    match a.as_name(id).ok_or(shape("expected kind name"))? {
        "shell" => Ok(EffectKind::Shell),
        "http" => Ok(EffectKind::Http),
        "model" => Ok(EffectKind::Model),
        "now" => Ok(EffectKind::Now),
        "timer" => Ok(EffectKind::Timer),
        "emit" => Ok(EffectKind::Emit),
        _ => Err(shape("unknown effect kind")),
    }
}

fn read_outcome(a: &Arenas, id: StructId) -> Result<EffectOutcome, EventAstError> {
    match head_of(a, id)? {
        "ok" => {
            let [p] = form(a, id, "ok")? else {
                return Err(shape("ok arity"));
            };
            Ok(EffectOutcome::Ok(read_opt_payload(a, *p)?))
        }
        "err" => {
            let [m] = form(a, id, "err")? else {
                return Err(shape("err arity"));
            };
            Ok(EffectOutcome::Err(read_str(a, *m)?))
        }
        "timed-out" => Ok(EffectOutcome::TimedOut),
        _ => Err(shape("unknown outcome tag")),
    }
}

fn read_opt_ms(a: &Arenas, id: StructId) -> Result<Option<u64>, EventAstError> {
    match head_of(a, id)? {
        "none" => Ok(None),
        "ms" => {
            let [n] = form(a, id, "ms")? else {
                return Err(shape("ms arity"));
            };
            Ok(Some(read_u64(a, *n)?))
        }
        _ => Err(shape("unknown deadline tag")),
    }
}

fn read_body(a: &Arenas, id: StructId) -> Result<EventBody, EventAstError> {
    Ok(match head_of(a, id)? {
        "genesis" => {
            let [h] = form(a, id, "genesis")? else {
                return Err(shape("genesis arity"));
            };
            EventBody::Genesis {
                reducer: read_hash(a, *h)?,
            }
        }
        "inbound" => {
            let [fam, ver, pf] = form(a, id, "inbound")? else {
                return Err(shape("inbound arity"));
            };
            let version = u32::try_from(read_u64(a, *ver)?)
                .map_err(|_| shape("content-type version out of u32 range"))?;
            EventBody::Inbound {
                content_type: ContentType {
                    family: read_str(a, *fam)?,
                    version,
                },
                payload: read_payload(a, *pf)?,
            }
        }
        "dispatched" => {
            let [idv, k, t, idem, dl] = form(a, id, "dispatched")? else {
                return Err(shape("dispatched arity"));
            };
            EventBody::Dispatched {
                id: EffectId(read_u64(a, *idv)?),
                kind: read_kind(a, *k)?,
                target: read_str(a, *t)?,
                idempotency_key: read_hash(a, *idem)?,
                deadline_ms: read_opt_ms(a, *dl)?,
            }
        }
        "effect-result" => {
            let [idv, o] = form(a, id, "effect-result")? else {
                return Err(shape("effect-result arity"));
            };
            EventBody::EffectResult {
                id: EffectId(read_u64(a, *idv)?),
                result: read_outcome(a, *o)?,
            }
        }
        "timer-armed" => {
            let [idv, dl] = form(a, id, "timer-armed")? else {
                return Err(shape("timer-armed arity"));
            };
            EventBody::TimerArmed {
                id: EffectId(read_u64(a, *idv)?),
                deadline_ms: read_u64(a, *dl)?,
            }
        }
        "timer-fired" => {
            let [idv, fm] = form(a, id, "timer-fired")? else {
                return Err(shape("timer-fired arity"));
            };
            EventBody::TimerFired {
                id: EffectId(read_u64(a, *idv)?),
                fired_ms: read_u64(a, *fm)?,
            }
        }
        "authz-denied" => {
            let [idv, r] = form(a, id, "authz-denied")? else {
                return Err(shape("authz-denied arity"));
            };
            EventBody::AuthzDenied {
                id: EffectId(read_u64(a, *idv)?),
                reason: read_str(a, *r)?,
            }
        }
        "closed" => {
            let [pf] = form(a, id, "closed")? else {
                return Err(shape("closed arity"));
            };
            EventBody::Closed {
                outcome: read_payload(a, *pf)?,
            }
        }
        _ => return Err(shape("unknown event body tag")),
    })
}

fn read_event(a: &Arenas, id: StructId) -> Result<Event, EventAstError> {
    let [seq, cause, body] = form(a, id, "event")? else {
        return Err(shape("event arity"));
    };
    let cause = match head_of(a, *cause)? {
        "none" => None,
        "hash" => Some(read_hash(a, *cause)?),
        _ => return Err(shape("unknown cause tag")),
    };
    Ok(Event {
        seq: read_u64(a, *seq)?,
        cause,
        body: read_body(a, *body)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_variants() -> Vec<Event> {
        let h = Hash::of(b"x");
        let ct = ContentType {
            family: "message".into(),
            version: 3,
        };
        vec![
            Event {
                seq: 0,
                cause: None,
                body: EventBody::Genesis { reducer: h },
            },
            Event {
                seq: 1,
                cause: Some(h),
                body: EventBody::Inbound {
                    content_type: ct.clone(),
                    payload: Payload::Inline(b"hello".to_vec()),
                },
            },
            Event {
                seq: 2,
                cause: None,
                body: EventBody::Inbound {
                    content_type: ct,
                    payload: Payload::Blob(h),
                },
            },
            Event {
                seq: 3,
                cause: Some(h),
                body: EventBody::Dispatched {
                    id: EffectId(7),
                    kind: EffectKind::Http,
                    target: "https://ok.host/p".into(),
                    idempotency_key: h,
                    deadline_ms: Some(12345),
                },
            },
            Event {
                seq: 4,
                cause: None,
                body: EventBody::Dispatched {
                    id: EffectId(8),
                    kind: EffectKind::Shell,
                    target: "cargo test".into(),
                    idempotency_key: h,
                    deadline_ms: None,
                },
            },
            Event {
                seq: 5,
                cause: None,
                body: EventBody::EffectResult {
                    id: EffectId(7),
                    result: EffectOutcome::Ok(Some(Payload::Inline(b"body".to_vec()))),
                },
            },
            Event {
                seq: 6,
                cause: None,
                body: EventBody::EffectResult {
                    id: EffectId(8),
                    result: EffectOutcome::Ok(None),
                },
            },
            Event {
                seq: 7,
                cause: None,
                body: EventBody::EffectResult {
                    id: EffectId(9),
                    result: EffectOutcome::Err("boom".into()),
                },
            },
            Event {
                seq: 8,
                cause: None,
                body: EventBody::EffectResult {
                    id: EffectId(9),
                    result: EffectOutcome::TimedOut,
                },
            },
            Event {
                seq: 9,
                cause: None,
                body: EventBody::TimerArmed {
                    id: EffectId(10),
                    deadline_ms: 999,
                },
            },
            Event {
                seq: 10,
                cause: None,
                body: EventBody::TimerFired {
                    id: EffectId(10),
                    fired_ms: 1000,
                },
            },
            Event {
                seq: 11,
                cause: None,
                body: EventBody::AuthzDenied {
                    id: EffectId(11),
                    reason: "no capability".into(),
                },
            },
            Event {
                seq: 12,
                cause: None,
                body: EventBody::Closed {
                    outcome: Payload::Inline(vec![]),
                },
            },
        ]
    }

    #[test]
    fn every_variant_round_trips_through_the_shared_codec() {
        for e in all_variants() {
            let bytes = encode(&e);
            let back = decode(&bytes).unwrap_or_else(|err| panic!("decode {e:?}: {err:?}"));
            assert_eq!(back, e, "round-trip mismatch for {e:?}");
        }
    }

    #[test]
    fn truncated_input_is_classified_torn_never_panics() {
        // Every proper prefix of a valid encoding must be an Err (Codec, often Truncated), never a panic
        // — the torn-vs-corrupt split log recovery relies on. (We only assert no-panic + Err here; the
        // precise Truncated-vs-other boundary is cadenza_ast's own contract, tested there.)
        for e in all_variants() {
            let bytes = encode(&e);
            for cut in 0..bytes.len() {
                assert!(
                    decode(&bytes[..cut]).is_err(),
                    "prefix {cut} of {e:?} must Err, not decode"
                );
            }
        }
    }

    #[test]
    fn truncated_maps_to_codec_torn_and_a_valid_nonevent_ast_is_shape() {
        // A 0-byte input ran out mid-header → cadenza_ast Truncated → our Codec(Truncated) (a torn tail).
        match decode(&[]) {
            Err(EventAstError::Codec(codec::DecodeError::Truncated)) => {}
            other => panic!("empty input should be Codec(Truncated), got {other:?}"),
        }
        // A perfectly valid AST that isn't an EVENT shape → Shape (corruption), not a panic. Encode a
        // bare name `nope` as a standalone AST and feed its bytes in.
        let mut b = Builder::new();
        let root = b.name("nope");
        let non_event = codec::encode(&b.finish(root));
        match decode(&non_event) {
            Err(EventAstError::Shape(_)) => {}
            other => panic!("a valid non-event AST should be Shape, got {other:?}"),
        }
    }
}
