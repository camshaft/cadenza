//! Events and the per-session log.
//!
//! The log is the append-only, ordered record of everything that happened in a session. It IS the
//! state (§14a) — KV is a derived projection, snapshots are checkpoints of it. Every event is wrapped
//! in a thin **envelope** carrying the fields the review said must exist from day one: `cause` (the
//! causal parent, for the DAG §5), a `content_type` tag the kernel routes on but never interprets
//! (§9b), and — later — a signature + producer identity (§10; carried as optional now, unverified in
//! v0). The kernel treats the payload as opaque; only reducers/executors interpret it.

use crate::effect::{EffectId, EffectKind, Payload};
use crate::hash::Hash;

/// Position of an event within a session log: a dense 0-based index. Total order within a session
/// (§3); there is no global order across sessions (§4b).
pub type SeqNo = u64;

/// A structured content-type tag (§9b): `family` + `version`, so tolerant readers can match on family
/// and range-check version. The kernel carries it opaquely for routing/filtering; it is a HINT, never
/// a trusted type assertion (§9b boundary).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ContentType {
    pub family: String,
    pub version: u32,
}

/// The event body — what actually happened. This is the v0 vocabulary; it grows as features land.
/// Crucially it distinguishes the three obligation-bearing kinds the review (S1) said must be durable
/// LOG events, not ephemeral kernel metadata: `Dispatched`, `EffectResult`, and `TimerArmed`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum EventBody {
    /// The session's first event: names the reducer to fold with (by content hash) and the initial
    /// capability grant. Portable/self-describing (§3 genesis).
    Genesis { reducer: Hash },

    /// An inbound message delivered into this session (from a peer via `Emit`, from a broker/ingress,
    /// or from the operator). The reducer folds it. Opaque payload.
    Inbound {
        content_type: ContentType,
        payload: Payload,
    },

    /// **DURABLE dispatch record (§16c-S1).** Written to the authoritative log BEFORE the effect is
    /// routed to an executor. This is what makes crash recovery correct: on restart, a `Dispatched`
    /// with no matching `EffectResult` is a known in-flight obligation to re-drive (idempotently) or
    /// fail — never silently double-fire or drop.
    Dispatched {
        id: EffectId,
        kind: EffectKind,
        target: String,
        /// Idempotency key (§16c-S1/D): re-driving a dispatch with the same key must not double-apply.
        /// For naturally-idempotent effects this can equal the id; for side-effecting ones the
        /// executor dedups on it.
        idempotency_key: Hash,
        /// Absolute deadline for the auto-timeout (§9d), as a wall-clock ms anchor (§16c-S5) so it
        /// survives failover/migration — the reducer still never reads the clock.
        deadline_ms: Option<u64>,
        /// The reducer's OWN opaque continuation token for this effect, if it supplied one (§19e). A WASM
        /// `ComponentReducer` correlates continuations by this token, never by the kernel `EffectId`
        /// (which stays kernel-internal). Recording it HERE — in the durable Dispatched frame — is the
        /// §19e hard guard: the `EffectId ↔ token` bridge map is session state that MUST rebuild from the
        /// LOG on recovery, so the token can't live only in a volatile map. `None` for effects from a
        /// reducer that doesn't use a token (e.g. the in-process Rust `Reducer` trait), which is every
        /// dispatch until the wasm-reducer bridge (§19e slice 2) populates it.
        token: Option<Vec<u8>>,
    },

    /// The result of a previously-`Dispatched` effect, correlated by `id` (§16c-S4). The reducer
    /// resumes its continuation for that id.
    EffectResult {
        id: EffectId,
        result: EffectOutcome,
        /// The reducer's continuation token for this effect (§19b/§19e (B)), COPIED from `id`'s
        /// `Dispatched` frame when the result is recorded. It rides the result event so a WASM
        /// `ComponentReducer`'s `fold` reads it back as the guest's `resumes` — without `fold` ever
        /// touching the log/map (fold stays a pure function of `(event, kv)`). `None` = the dispatch
        /// carried no token (a Rust reducer that correlates by `EffectId`). Derived from the durable
        /// Dispatched frame, so it's the same live and on replay.
        token: Option<Vec<u8>>,
    },

    /// A timer was armed. Durable (§16c-S1/S5) with an ABSOLUTE deadline so any node can compute the
    /// remaining time after failover. The kernel injects a `TimerFired` at the deadline.
    TimerArmed {
        id: EffectId,
        deadline_ms: u64,
        /// The reducer's OWN opaque continuation token for the timer effect (§19e), recorded here in the
        /// durable arming frame — the timer analogue of `Dispatched.token`. When the timer fires, the
        /// kernel copies this onto the `TimerFired` event so a WASM `ComponentReducer` reads it back as
        /// the guest's `resumes` (slice 2b-iii). `None` = a token-free reducer (the in-process Rust
        /// `Reducer`, which correlates by `EffectId`). Recording it in the durable frame (not a volatile
        /// map) is the same §19e recovery guard as `Dispatched.token`.
        token: Option<Vec<u8>>,
    },

    /// A timer fired (or an effect deadline elapsed). Carries the recorded fire time so replay reads a
    /// frozen fact and never consults a clock (§9c).
    TimerFired {
        id: EffectId,
        fired_ms: u64,
        /// The reducer's continuation token for the timer (§19b/§19e (B)), COPIED from `id`'s
        /// `TimerArmed` frame when the fire is recorded — the timer analogue of `EffectResult.token`. It
        /// rides the fire event so a WASM `ComponentReducer`'s `fold` reads it back as the guest's
        /// `resumes` without touching the log/map (fold stays pure). `None` = the timer was armed
        /// token-free. Derived from the durable arming frame, so it's the same live and on replay.
        token: Option<Vec<u8>>,
    },

    /// An authorization decision the kernel made about a requested effect — logged so an audit can
    /// replay not just what happened but whether it was permitted (§10). `denied` requests never reach
    /// an executor.
    AuthzDenied {
        id: EffectId,
        reason: String,
        /// The reducer's continuation token for the DENIED effect (§19b/§19e (B)), moved from the
        /// effect request the reducer emitted. A denial is a terminal outcome the reducer resumes on
        /// (`resumes_effect` recognizes it), so the token rides the denial event exactly as it rides an
        /// `EffectResult` — the guest's `fold` reads it back as `resumes` to run its denial branch.
        /// Unlike a dispatch/timer, there is NO prior durable frame to copy from (a denial is recorded
        /// BEFORE any `Dispatched`/`TimerArmed` — the effect never ran), so the token comes straight from
        /// the requesting `Effect`. `None` = a token-free reducer.
        token: Option<Vec<u8>>,
    },

    /// The session closed. `outcome` is opaque; if the session had a parent, the kernel delivers a
    /// completion signal to it (§6 supervision).
    Closed { outcome: Payload },
}

/// The outcome of an executed effect: success payload, a failure, or a timeout (the §9d anti-stuck
/// path — a hung effect becomes a normal event, not a wedge).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum EffectOutcome {
    Ok(Option<Payload>),
    Err(String),
    /// The effect's deadline elapsed with no result. Per the v0 decision (§16c-S4), a timeout CANCELS
    /// the dispatch — the kernel guarantees no late `Ok`/`Err` for this id will ever be folded.
    TimedOut,
}

/// An event plus its envelope. The envelope fields are the day-one commitments from the review/§10:
/// `cause` for the causal DAG, room for `sig`/`producer` (added when multi-operator lands).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Event {
    pub seq: SeqNo,
    /// The causal parent — the event (possibly in another session) that led to this one (§5). `None`
    /// for genesis and for un-caused external ingress.
    pub cause: Option<Hash>,
    pub body: EventBody,
}

impl Event {
    /// Content-address this event's canonical bytes. This is what `cause` edges and the log's
    /// tamper-evidence point at. v0 uses a simple deterministic encoding; §16c-S3 requires this
    /// encoding be frozen/canonical, which we honor by keeping it explicit and total here.
    pub fn hash(&self) -> Hash {
        Hash::of(&self.encode())
    }

    /// Canonical byte encoding (frozen — §16c-S3). Deterministic: same event → same bytes → same hash.
    /// Kept intentionally simple and self-contained (no serde) so the canonical form is auditable in
    /// one place. This is the on-disk log format (durable log persistence, next slice): what `encode`
    /// writes, [`Event::decode`] reads back exactly.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.seq.to_le_bytes());
        match &self.cause {
            Some(h) => {
                out.push(1);
                out.extend_from_slice(h.as_bytes());
            }
            None => out.push(0),
        }
        encode_body(&self.body, &mut out);
        out
    }

    /// Decode an event from its canonical bytes (the inverse of [`Event::encode`]). Total: any
    /// malformed/truncated input yields `Err`, never a panic (the durable log must survive a torn
    /// write at the tail — the reader stops cleanly at the last well-formed event). Returns the event
    /// AND the number of bytes consumed, so a log reader can decode a concatenated stream.
    pub fn decode(bytes: &[u8]) -> Result<(Event, usize), DecodeError> {
        let mut c = Cursor::new(bytes);
        let seq = c.u64()?;
        let cause = match c.u8()? {
            0 => None,
            1 => Some(Hash::from_bytes(c.hash()?)),
            t => {
                return Err(DecodeError::BadTag {
                    field: "cause",
                    tag: t,
                })
            }
        };
        let body = decode_body(&mut c)?;
        Ok((Event { seq, cause, body }, c.pos))
    }
}

/// A decode failure. The durable log reader treats any of these at the stream tail as "stop here"
/// (a torn final write), and anywhere else as genuine corruption to surface.
#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// Ran out of bytes mid-field (truncated / torn write).
    Truncated,
    /// A variant tag byte that isn't a known variant (corruption or a newer format — v0 rejects).
    BadTag { field: &'static str, tag: u8 },
    /// A length field claims more bytes than remain.
    BadLength,
    /// A string field wasn't valid UTF-8.
    BadUtf8,
}

/// A minimal forward-only byte cursor for decoding. Every read is bounds-checked → total decode.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Cursor { bytes, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(n).ok_or(DecodeError::BadLength)?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or(DecodeError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, DecodeError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, DecodeError> {
        let b = self.take(8)?;
        let mut arr = [0u8; 8];
        arr.copy_from_slice(b);
        Ok(u64::from_le_bytes(arr))
    }

    fn hash(&mut self) -> Result<[u8; 32], DecodeError> {
        let b = self.take(32)?;
        let mut arr = [0u8; 32];
        arr.copy_from_slice(b);
        Ok(arr)
    }

    /// Read a `u64` length prefix and convert to `usize`, FAILING with `BadLength` if it doesn't fit
    /// (PR#990 finding #3). The length comes from the untrusted durable log; a bare `as usize` would
    /// TRUNCATE on a 32-bit target (a huge length wraps small → mis-parse). This rejects the frame
    /// cleanly instead. A too-large length then errs one of two ways at the subsequent `take` (PR#993
    /// #3): `Truncated` if it merely exceeds the remaining bytes, or `BadLength` if `pos + len`
    /// overflows `usize` (take's own checked-add guard) — both clean rejections, never a panic or
    /// mis-parse.
    fn len(&mut self) -> Result<usize, DecodeError> {
        usize::try_from(self.u64()?).map_err(|_| DecodeError::BadLength)
    }

    /// A length-prefixed UTF-8 string (matches `encode_str`).
    fn string(&mut self) -> Result<String, DecodeError> {
        let len = self.len()?;
        let b = self.take(len)?;
        core::str::from_utf8(b)
            .map(|s| s.to_string())
            .map_err(|_| DecodeError::BadUtf8)
    }
}

fn decode_body(c: &mut Cursor) -> Result<EventBody, DecodeError> {
    let tag = c.u8()?;
    Ok(match tag {
        0 => EventBody::Genesis {
            reducer: Hash::from_bytes(c.hash()?),
        },
        1 => {
            let family = c.string()?;
            let version = c.u32()?;
            let payload = decode_payload(c)?;
            EventBody::Inbound {
                content_type: ContentType { family, version },
                payload,
            }
        }
        2 => {
            let id = EffectId(c.u64()?);
            let kind = decode_kind(c.u8()?)?;
            let target = c.string()?;
            let idempotency_key = Hash::from_bytes(c.hash()?);
            let deadline_ms = decode_opt_u64(c)?;
            let token = decode_opt_bytes(c)?;
            EventBody::Dispatched {
                id,
                kind,
                target,
                idempotency_key,
                deadline_ms,
                token,
            }
        }
        3 => EventBody::EffectResult {
            id: EffectId(c.u64()?),
            result: decode_outcome(c)?,
            token: decode_opt_bytes(c)?,
        },
        4 => EventBody::TimerArmed {
            id: EffectId(c.u64()?),
            deadline_ms: c.u64()?,
            token: decode_opt_bytes(c)?,
        },
        5 => EventBody::TimerFired {
            id: EffectId(c.u64()?),
            fired_ms: c.u64()?,
            token: decode_opt_bytes(c)?,
        },
        6 => EventBody::AuthzDenied {
            id: EffectId(c.u64()?),
            reason: c.string()?,
            token: decode_opt_bytes(c)?,
        },
        7 => EventBody::Closed {
            outcome: decode_payload(c)?,
        },
        t => {
            return Err(DecodeError::BadTag {
                field: "body",
                tag: t,
            })
        }
    })
}

fn decode_kind(tag: u8) -> Result<EffectKind, DecodeError> {
    Ok(match tag {
        0 => EffectKind::Shell,
        1 => EffectKind::Http,
        2 => EffectKind::Model,
        3 => EffectKind::Now,
        4 => EffectKind::Timer,
        5 => EffectKind::Emit,
        t => {
            return Err(DecodeError::BadTag {
                field: "kind",
                tag: t,
            })
        }
    })
}

fn decode_opt_u64(c: &mut Cursor) -> Result<Option<u64>, DecodeError> {
    Ok(match c.u8()? {
        0 => None,
        1 => Some(c.u64()?),
        t => {
            return Err(DecodeError::BadTag {
                field: "opt_u64",
                tag: t,
            })
        }
    })
}

fn decode_opt_bytes(c: &mut Cursor) -> Result<Option<Vec<u8>>, DecodeError> {
    Ok(match c.u8()? {
        0 => None,
        1 => {
            let len = c.len()?;
            Some(c.take(len)?.to_vec())
        }
        t => {
            return Err(DecodeError::BadTag {
                field: "opt_bytes",
                tag: t,
            })
        }
    })
}

fn decode_payload(c: &mut Cursor) -> Result<Payload, DecodeError> {
    Ok(match c.u8()? {
        0 => {
            let len = c.len()?;
            Payload::Inline(c.take(len)?.to_vec())
        }
        1 => Payload::Blob(Hash::from_bytes(c.hash()?)),
        t => {
            return Err(DecodeError::BadTag {
                field: "payload",
                tag: t,
            })
        }
    })
}

fn decode_outcome(c: &mut Cursor) -> Result<EffectOutcome, DecodeError> {
    Ok(match c.u8()? {
        0 => EffectOutcome::Ok(match c.u8()? {
            0 => None,
            1 => Some(decode_payload(c)?),
            t => {
                return Err(DecodeError::BadTag {
                    field: "outcome_ok",
                    tag: t,
                })
            }
        }),
        1 => EffectOutcome::Err(c.string()?),
        2 => EffectOutcome::TimedOut,
        t => {
            return Err(DecodeError::BadTag {
                field: "outcome",
                tag: t,
            })
        }
    })
}

/// Encode an event body canonically. A leading tag byte per variant keeps the form unambiguous and
/// stable. (When the wire format migrates to canonical s-expr per the design, this is the one place
/// that changes — and it must stay frozen thereafter.)
fn encode_body(body: &EventBody, out: &mut Vec<u8>) {
    match body {
        EventBody::Genesis { reducer } => {
            out.push(0);
            out.extend_from_slice(reducer.as_bytes());
        }
        EventBody::Inbound {
            content_type,
            payload,
        } => {
            out.push(1);
            encode_str(&content_type.family, out);
            out.extend_from_slice(&content_type.version.to_le_bytes());
            encode_payload(payload, out);
        }
        EventBody::Dispatched {
            id,
            kind,
            target,
            idempotency_key,
            deadline_ms,
            token,
        } => {
            out.push(2);
            out.extend_from_slice(&id.0.to_le_bytes());
            out.push(kind_tag(kind));
            encode_str(target, out);
            out.extend_from_slice(idempotency_key.as_bytes());
            encode_opt_u64(*deadline_ms, out);
            encode_opt_bytes(token.as_deref(), out);
        }
        EventBody::EffectResult { id, result, token } => {
            out.push(3);
            out.extend_from_slice(&id.0.to_le_bytes());
            encode_outcome(result, out);
            encode_opt_bytes(token.as_deref(), out);
        }
        EventBody::TimerArmed {
            id,
            deadline_ms,
            token,
        } => {
            out.push(4);
            out.extend_from_slice(&id.0.to_le_bytes());
            out.extend_from_slice(&deadline_ms.to_le_bytes());
            encode_opt_bytes(token.as_deref(), out);
        }
        EventBody::TimerFired {
            id,
            fired_ms,
            token,
        } => {
            out.push(5);
            out.extend_from_slice(&id.0.to_le_bytes());
            out.extend_from_slice(&fired_ms.to_le_bytes());
            encode_opt_bytes(token.as_deref(), out);
        }
        EventBody::AuthzDenied { id, reason, token } => {
            out.push(6);
            out.extend_from_slice(&id.0.to_le_bytes());
            encode_str(reason, out);
            encode_opt_bytes(token.as_deref(), out);
        }
        EventBody::Closed { outcome } => {
            out.push(7);
            encode_payload(outcome, out);
        }
    }
}

fn kind_tag(kind: &EffectKind) -> u8 {
    match kind {
        EffectKind::Shell => 0,
        EffectKind::Http => 1,
        EffectKind::Model => 2,
        EffectKind::Now => 3,
        EffectKind::Timer => 4,
        EffectKind::Emit => 5,
    }
}

fn encode_str(s: &str, out: &mut Vec<u8>) {
    out.extend_from_slice(&(s.len() as u64).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn encode_opt_u64(v: Option<u64>, out: &mut Vec<u8>) {
    match v {
        Some(n) => {
            out.push(1);
            out.extend_from_slice(&n.to_le_bytes());
        }
        None => out.push(0),
    }
}

/// Optional opaque byte string (a reducer's continuation token, §19e): `0` = None, `1` + u64-len +
/// bytes = Some. Same present-tag + length-prefix shape as the other opt/str encoders.
fn encode_opt_bytes(v: Option<&[u8]>, out: &mut Vec<u8>) {
    match v {
        Some(b) => {
            out.push(1);
            out.extend_from_slice(&(b.len() as u64).to_le_bytes());
            out.extend_from_slice(b);
        }
        None => out.push(0),
    }
}

fn encode_payload(p: &Payload, out: &mut Vec<u8>) {
    match p {
        Payload::Inline(bytes) => {
            out.push(0);
            out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
            out.extend_from_slice(bytes);
        }
        Payload::Blob(h) => {
            out.push(1);
            out.extend_from_slice(h.as_bytes());
        }
    }
}

fn encode_outcome(o: &EffectOutcome, out: &mut Vec<u8>) {
    match o {
        EffectOutcome::Ok(p) => {
            out.push(0);
            match p {
                Some(p) => {
                    out.push(1);
                    encode_payload(p, out);
                }
                None => out.push(0),
            }
        }
        EffectOutcome::Err(msg) => {
            out.push(1);
            encode_str(msg, out);
        }
        EffectOutcome::TimedOut => out.push(2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn genesis() -> Event {
        Event {
            seq: 0,
            cause: None,
            body: EventBody::Genesis {
                reducer: Hash::of(b"reducer-v1"),
            },
        }
    }

    #[test]
    fn event_hash_is_deterministic() {
        assert_eq!(genesis().hash(), genesis().hash());
    }

    // A FULLY-FIXED event (every field a literal, no Hash::of indirection) whose encoded bytes are
    // pinned by the frozen-encoding golden test below.
    fn golden_event() -> Event {
        Event {
            seq: 42,
            cause: Some(Hash::from_bytes([0xABu8; 32])),
            body: EventBody::Dispatched {
                id: EffectId(7),
                kind: EffectKind::Http,
                target: "https://ok.host/p".into(),
                idempotency_key: Hash::from_bytes([0xCDu8; 32]),
                deadline_ms: Some(1000),
                token: Some(b"step-1".to_vec()),
            },
        }
    }

    #[test]
    fn frozen_encoding_is_byte_stable_golden() {
        // §16c-S3: the encoding is FROZEN — "same event → same bytes → same hash." The round-trip test
        // proves decode(encode(x)) == x, but NOT that encode(x) yields the SAME bytes over time: a
        // refactor could change the byte layout while keeping round-trip intact, silently invalidating
        // every persisted log's content-address hashes + `cause` edges (a session that recovers under a
        // new layout would compute different hashes for the same history). This golden pin is the
        // tripwire: if it fails, the on-disk format changed — that MUST be a conscious, versioned
        // migration (e.g. the planned swap to the shared `cadenza-ast` codec), never an accident.
        //
        // Byte layout of `golden_event()` (little-endian; the frozen §16c-S3 format):
        //   seq=42                → 42,0,0,0,0,0,0,0        (u64 LE)
        //   cause=Some(0xAB..)    → 1, then 32×0xAB(171)    (tag + Hash)
        //   body tag=Dispatched   → 2
        //   id=7                  → 7,0,0,0,0,0,0,0         (u64 LE)
        //   kind=Http             → 1                       (kind tag)
        //   target len=17         → 17,0,0,0,0,0,0,0        (u64 LE len)
        //   target="https://ok.host/p" → its 17 UTF-8 bytes
        //   idempotency_key=0xCD.. → 32×0xCD(205)           (Hash)
        //   deadline_ms=Some(1000) → 1, then 1000 as u64 LE (232,3,0,0,0,0,0,0)
        //   token=Some("step-1")  → 1, then 6 as u64 LE, then "step-1" (§19e — CONSCIOUS format bump:
        //                            the Dispatched frame now carries the reducer's continuation token
        //                            so the EffectId↔token map rebuilds from the log on recover)
        let expected: &[u8] = &[
            42, 0, 0, 0, 0, 0, 0, 0, // seq
            1, // cause: Some
            171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171,
            171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171,
            171, // cause hash
            2,   // body tag = Dispatched
            7, 0, 0, 0, 0, 0, 0, 0, // id
            1, // kind = Http
            17, 0, 0, 0, 0, 0, 0, 0, // target len
            104, 116, 116, 112, 115, 58, 47, 47, 111, 107, 46, 104, 111, 115, 116, 47,
            112, // "https://ok.host/p"
            205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205,
            205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205,
            205, // idempotency_key hash
            1, 232, 3, 0, 0, 0, 0, 0, 0, // deadline_ms: Some(1000)
            1, 6, 0, 0, 0, 0, 0, 0, 0, // token: Some, len 6
            115, 116, 101, 112, 45, 49, // "step-1"
        ];
        assert_eq!(
            golden_event().encode(),
            expected,
            "FROZEN ENCODING CHANGED — the on-disk log format is not byte-stable. If this is an \
             intentional format change it MUST be a versioned migration (§16c-S3), not an accident."
        );
        // And the content-address hash the log/cause-edges depend on is pinned too.
        assert_eq!(
            golden_event().hash().to_hex(),
            "e9b6ff706b67c632eb61a64be8522ec2f613455337a0cbf46e700074b428aee0",
            "FROZEN event hash changed — persisted `cause` edges + content addresses would break."
        );
    }

    #[test]
    fn seq_change_changes_hash() {
        let mut e = genesis();
        e.seq = 1;
        assert_ne!(e.hash(), genesis().hash());
    }

    #[test]
    fn distinct_bodies_hash_distinctly() {
        let a = Event {
            seq: 5,
            cause: None,
            body: EventBody::TimerFired {
                id: EffectId(1),
                fired_ms: 100,
                token: None,
            },
        };
        let b = Event {
            seq: 5,
            cause: None,
            body: EventBody::TimerFired {
                id: EffectId(1),
                fired_ms: 101,
                token: None,
            },
        };
        assert_ne!(a.hash(), b.hash());
    }

    #[test]
    fn cause_edge_is_part_of_identity() {
        let mut e = genesis();
        e.body = EventBody::Closed {
            outcome: Payload::Inline(vec![]),
        };
        let without = e.hash();
        e.cause = Some(Hash::of(b"parent"));
        assert_ne!(e.hash(), without);
    }

    /// Every event body variant, for exhaustive round-trip coverage. If a new variant is added,
    /// `encode_body`/`decode_body` must both handle it — this list is the reminder.
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
                    token: Some(b"resume-tok".to_vec()),
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
                    token: None,
                },
            },
            Event {
                seq: 5,
                cause: None,
                body: EventBody::EffectResult {
                    id: EffectId(7),
                    result: EffectOutcome::Ok(Some(Payload::Inline(b"body".to_vec()))),
                    token: Some(b"resume-tok".to_vec()),
                },
            },
            Event {
                seq: 6,
                cause: None,
                body: EventBody::EffectResult {
                    id: EffectId(8),
                    result: EffectOutcome::Ok(None),
                    token: None,
                },
            },
            Event {
                seq: 7,
                cause: None,
                body: EventBody::EffectResult {
                    id: EffectId(9),
                    result: EffectOutcome::Err("boom".into()),
                    token: None,
                },
            },
            Event {
                seq: 8,
                cause: None,
                body: EventBody::EffectResult {
                    id: EffectId(9),
                    result: EffectOutcome::TimedOut,
                    token: None,
                },
            },
            Event {
                seq: 9,
                cause: None,
                // Some(token): exercise the §19e slice-2b-iii token codec on the arming frame.
                body: EventBody::TimerArmed {
                    id: EffectId(10),
                    deadline_ms: 999,
                    token: Some(b"timer-tok".to_vec()),
                },
            },
            Event {
                seq: 10,
                cause: None,
                body: EventBody::TimerFired {
                    id: EffectId(10),
                    fired_ms: 1000,
                    token: Some(b"timer-tok".to_vec()),
                },
            },
            Event {
                seq: 11,
                cause: None,
                // Some(token) on a denial too: a denied effect's continuation token rides the denial.
                body: EventBody::AuthzDenied {
                    id: EffectId(11),
                    reason: "no capability".into(),
                    token: Some(b"denied-tok".to_vec()),
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
    fn encode_decode_round_trips_every_variant() {
        for e in all_variants() {
            let bytes = e.encode();
            let (decoded, consumed) = Event::decode(&bytes).expect("decode");
            assert_eq!(decoded, e, "round-trip mismatch for {e:?}");
            assert_eq!(
                consumed,
                bytes.len(),
                "must consume exactly the encoded bytes"
            );
        }
    }

    #[test]
    fn decode_reports_offset_so_a_stream_can_be_walked() {
        // Two events concatenated (the on-disk log shape): decode the first, then the second from the
        // reported offset. This is what the durable-log reader will do.
        let events = all_variants();
        let mut stream = Vec::new();
        for e in &events[..2] {
            stream.extend_from_slice(&e.encode());
        }
        let (e0, n0) = Event::decode(&stream).unwrap();
        let (e1, _n1) = Event::decode(&stream[n0..]).unwrap();
        assert_eq!(e0, events[0]);
        assert_eq!(e1, events[1]);
    }

    #[test]
    fn truncated_input_errs_never_panics() {
        // A torn write at the tail: every proper prefix of a valid encoding must Err, not panic
        // (totality — the durable log must survive a partial final write).
        for e in all_variants() {
            let bytes = e.encode();
            for cut in 0..bytes.len() {
                let _ = Event::decode(&bytes[..cut]); // must not panic
            }
        }
    }

    #[test]
    fn corrupt_tag_is_rejected() {
        let mut bytes = genesis().encode();
        // seq(8) + cause-tag(1) = offset 9 is the body variant tag; set it to an unknown value.
        bytes[9] = 250;
        assert!(matches!(
            Event::decode(&bytes),
            Err(DecodeError::BadTag { .. })
        ));
    }

    #[test]
    fn oversized_length_prefix_errs_never_panics() {
        // PR#990 finding #3: a length prefix from the untrusted log that is enormous (here u64::MAX)
        // must fail cleanly (BadLength on 32-bit where it can't fit usize; Truncated on 64-bit where it
        // exceeds the remaining bytes) — NEVER panic or wrap-truncate into a mis-parse. Build an
        // Inbound whose content_type.family length prefix is u64::MAX.
        // Inbound encoding: seq(8) + cause-tag(1=0) + body-tag(1) + family-len(8) + ...
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1u64.to_le_bytes()); // seq
        bytes.push(0); // cause: None
        bytes.push(1); // body tag = Inbound
        bytes.extend_from_slice(&u64::MAX.to_le_bytes()); // family length = absurd
        bytes.extend_from_slice(b"anything");
        match Event::decode(&bytes) {
            Err(DecodeError::BadLength) | Err(DecodeError::Truncated) => {}
            other => panic!("expected BadLength/Truncated for an oversized length, got {other:?}"),
        }
    }
}
