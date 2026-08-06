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
///
/// `family` is a `Cow<'static, str>` (operator Bytes/cheap-clone directive): the well-known effect families
/// are `&'static str` consts ([`crate::effect::effect_ct`]), so a content-type built from a kind holds
/// `Cow::Borrowed` — ZERO heap allocation on the hot effect path (an effect's `content_type.family` was a
/// fresh `String` per `EffectRequest::new` before this). A runtime-derived family (a decoded/inbound one)
/// holds `Cow::Owned`; both compare + deref to `&str` identically, so read/deref/compare callers are
/// unaffected — only a caller that ASSIGNS a `String` to `family` now needs `.into()` (`String` and
/// `&'static str` both `Into<Cow>`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ContentType {
    pub family: std::borrow::Cow<'static, str>,
    pub version: u32,
}

impl ContentType {
    /// The well-known **"report" content-type** (v1) — the fork-for-query summarize protocol contract
    /// (operator ruling (a), fork-for-query design). An ephemeral query fork ([`crate::kernel::Session::fork_for_query`])
    /// delivers an `Inbound` carrying THIS content-type; the reducer recognizes it (via [`ContentType::is_report`])
    /// and describes its own state — from local KV/goal/progress where it can (no model call, the operator's
    /// preference), or via a scoped model call where it must. The kernel never interprets it (§9b: content-type
    /// is a routing HINT, not a trusted assertion); it's the AGREED family string reducers key off, so a debug
    /// query means the same thing to every reducer.
    pub const REPORT_FAMILY: &'static str = "report";

    /// Construct the well-known report content-type (`{family: "report", version: 1}`) — see
    /// [`ContentType::REPORT_FAMILY`]. Use this to build the summarize-query message a fork is delivered.
    pub fn report() -> Self {
        ContentType {
            // A `&'static str` const → `Cow::Borrowed`, zero-alloc.
            family: std::borrow::Cow::Borrowed(Self::REPORT_FAMILY),
            version: 1,
        }
    }

    /// Is this the fork-for-query "report" family? Matches on `family` ONLY (version-tolerant per §9b —
    /// a reducer accepts any report version it understands, range-checking `version` itself if it cares),
    /// so a reducer's fold can branch a summarize-query cheaply: `if event_ct.is_report() { …describe self… }`.
    pub fn is_report(&self) -> bool {
        self.matches_family(Self::REPORT_FAMILY)
    }

    /// The tolerant-reader family match (§9b, design §"content-type"): does this content-type belong to
    /// `family`, IGNORING version? This is the primitive a reducer/router keys off — match the family,
    /// then range-check the version separately with [`ContentType::version_in`] if it cares. Keeping the
    /// two checks distinct is exactly the "known family, unknown version → defer/reject honestly" behavior
    /// the design calls for (a v1 reader must not decode a `family/v2` payload as garbage). It is also the
    /// ONE place the family comparison lives: all family-keyed matching (routing, authz, [`ContentType::is_report`])
    /// should go through this helper rather than compare `self.family` inline, so the check can't drift across sites.
    pub fn matches_family(&self, family: &str) -> bool {
        self.family == family
    }

    /// The version half of the tolerant-reader check: is `version` within the INCLUSIVE `[min, max]` range
    /// this reader understands? Pair with [`ContentType::matches_family`] — `matches_family(f) &&
    /// version_in(1, 3)` is "a known `f`, at a version I can handle." A `family` match with a version
    /// OUTSIDE the range is the "known family, unknown version" case the reader defers/rejects rather than
    /// misdecoding (§9b / design §content-type). Total: an empty range (`min > max`) is simply never in.
    pub fn version_in(&self, min: u32, max: u32) -> bool {
        min <= self.version && self.version <= max
    }
}

/// The event body — what actually happened. This is the v0 vocabulary; it grows as features land.
/// Crucially it distinguishes the three obligation-bearing kinds the review (S1) said must be durable
/// LOG events, not ephemeral kernel metadata: `Dispatched`, `EffectResult`, and `TimerArmed`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum EventBody {
    /// The session's first event (§3 genesis): names the reducer to fold with (by content hash), a
    /// caller-supplied per-spawn `spawn_nonce` (entropy), and optional `parent` provenance.
    ///
    /// `spawn_nonce` (§lifecycle I2 / operator "hash of spawn-time + entropy" ruling): the kernel is
    /// clock-free + entropy-free (§9c), so the HOST mints this at spawn (32 random bytes via getrandom)
    /// and passes it in. It lives in the durable seq-0 event, so it's replay-deterministic (recovery reads
    /// it from the log, NEVER re-mints — a re-mint would change the genesis hash = a different SessionId on
    /// recovery = corruption). It is what makes `genesis_hash()` per-SESSION unique: without it, two
    /// sessions over the same reducer produced an identical Genesis event → identical SessionId → registry
    /// collision (the gap v-agent-harness-host pinned).
    ///
    /// `parent` (§6 supervision / lifecycle I2): `Some(<parent's genesis hash>)` for a session SPAWNED by
    /// another (via `lifecycle/spawn`), `None` for a root/top-level session. Baking the parent into the
    /// hashed seq-0 body makes the child's id self-certify its provenance (the child genesis-hash is
    /// provenance-dependent); the durable `Spawned{child_hash}` edge in the PARENT's log is the other half
    /// of the same relation (I6's descendant-authority + §8's cascade walk it).
    Genesis {
        reducer: Hash,
        spawn_nonce: Hash,
        parent: Option<Hash>,
    },

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
        /// The dispatched effect's content-type FAMILY (seq-39) — the authoritative identity of what was
        /// dispatched. Recorded ALONGSIDE `kind` because a register-by-string / control family (e.g.
        /// `control/capabilities`) has NO distinguishing `EffectKind` variant — its `kind` is a
        /// placeholder (`Emit`), so `kind` alone can't tell such a dispatch apart from a real emit on the
        /// recovery path. Persisting the family makes recovery classify an open dispatch deterministically
        /// (re-answer `control/capabilities` inline vs. re-drive a real emit), and is the direction the
        /// effect model is migrating onto (family is the source of truth, `kind` the legacy tag).
        family: std::sync::Arc<str>,
        /// The resolved target argument (URL / session-id / command). `Arc<str>` (operator cheap-clone
        /// directive): the dispatch frame is CLONED as it threads through record/replay/status, and the
        /// source [`crate::effect::EffectRequest::target`] is ALREADY `Arc<str>`, so recording it here is an
        /// O(1) refcount bump (`req.target.clone()`) instead of a fresh `String` alloc per dispatch. Derefs
        /// to `&str`, so readers are unaffected.
        target: std::sync::Arc<str>,
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

    /// A FOLD FAILED — the reducer trapped / exhausted fuel / failed to instantiate while folding an
    /// event (error-resilience / supervision direction). §17 totality means a bad reducer can NEVER
    /// panic the kernel loop; instead the failure is CAPTURED as this first-class log event rather than
    /// vanishing into a silent empty fold ("errors into the void"). `reason` is a human/diagnostic string
    /// (the guest trap message / fuel-exhaustion / instantiate error). `caused_event` is the hash of the
    /// event whose fold failed (so a supervisor can see WHAT the reducer choked on). v0 RECORDS it (a
    /// supervisor reading the log sees the failure); it is NOT itself folded (no recursion — a fold that
    /// fails can't be handed back to the same failing reducer). A future supervision slice lets a parent
    /// react (restart/retry/escalate) to a child's FoldFailed.
    FoldFailed { reason: String, caused_event: Hash },

    /// The session closed, carrying a STRUCTURED [`CloseOutcome`] (success-with-payload vs
    /// failure-with-reason). §6 supervision slice-1 (operator directive): a supervisor must be able to tell a
    /// clean completion from a failure to decide restart/retry/escalate — an opaque payload couldn't express
    /// that first-class. When the session had a parent, the kernel delivers this outcome to it as a
    /// `child-completed` signal (slices 2-3, pending the operator's design review).
    Closed { outcome: CloseOutcome },

    /// **DURABLE terminal marker (§lifecycle I1).** A session was TERMINATED by another session (the
    /// `lifecycle/terminate` effect), as distinct from [`Closed`](Self::Closed) (a session ending
    /// ITSELF). Once this is the log TAIL the kernel refuses every further fold ([`KernelError::
    /// FoldRefused`](crate::kernel::KernelError::FoldRefused)) — a first-class guard, not a host
    /// convention, so a terminated session can't be re-driven even by a buggy host. The log + KV are
    /// RETAINED (queryable, frozen); the terminality is durable + replay-stable (a recovered session
    /// whose tail is `Terminated` stays terminated). Terminal: there is no un-terminate (recovery from
    /// a bad state is a fresh spawn, §7).
    ///
    /// `by` is the terminating controller's session identity (its genesis hash = its SessionId); `reason`
    /// is a human/diagnostic string. The supervision-tree authority that gates WHO may terminate is a
    /// host/Cedar concern (I6, `ResourcePredicate::DescendantOf`); this event just records the durable
    /// fact + who did it.
    Terminated { by: Hash, reason: String },
}

/// How a session CLOSED — the structured terminal outcome a supervisor acts on (§6 supervision, operator
/// directive: "child-completed carries a structured outcome, success vs failure-with-reason"). Distinguishing
/// success from failure is the minimum a one-for-one supervisor needs to choose restart/retry/escalate; the
/// prior opaque `Payload` couldn't express it. A sum (no sentinel — mirrors [`EffectOutcome`]).
///
/// **Wire compat (both codecs):** `Success` encodes BYTE-IDENTICALLY to the legacy `Closed { outcome: Payload }`
/// (no wrapper tag — a bare payload), so an old `Closed` stream decodes as `Success` unchanged and its event
/// hash / cause edges are preserved; `Failure` takes a fresh discriminant a legacy Payload never produced
/// (binary tag `2`; textual head `failure`). A future arm (e.g. `Cancelled`) likewise takes a fresh unused
/// tag/head — additive — but this is NOT a blanket "tolerant decoder ignores unknowns": both codecs are
/// tag-discriminated and REJECT an unknown tag/head as corruption (the frozen-codec contract).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CloseOutcome {
    /// The session finished its goal cleanly. `payload` is the (opaque) result it produced — what a parent
    /// consumes on a successful child-completed.
    Success(Payload),
    /// The session terminated in failure. `reason` is a human/diagnostic string (why it gave up / what
    /// broke) — what a supervisor logs and reacts to (restart/retry/escalate).
    Failure(String),
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

    /// The next byte WITHOUT consuming it (for a tag-discriminated sum whose variant then re-reads the tag —
    /// e.g. `CloseOutcome`, whose `Success` delegates to `decode_payload` which re-reads the 0/1 payload tag).
    fn peek_u8(&self) -> Result<u8, DecodeError> {
        self.bytes
            .get(self.pos)
            .copied()
            .ok_or(DecodeError::Truncated)
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
            spawn_nonce: Hash::from_bytes(c.hash()?),
            parent: match c.u8()? {
                0 => None,
                1 => Some(Hash::from_bytes(c.hash()?)),
                t => {
                    return Err(DecodeError::BadTag {
                        field: "genesis.parent",
                        tag: t,
                    })
                }
            },
        },
        1 => {
            let family = c.string()?;
            let version = c.u32()?;
            let payload = decode_payload(c)?;
            EventBody::Inbound {
                content_type: ContentType {
                    family: family.into(),
                    version,
                },
                payload,
            }
        }
        2 => {
            let id = EffectId(c.u64()?);
            let kind = decode_kind(c.u8()?)?;
            let family = c.string()?;
            let target = c.string()?;
            let idempotency_key = Hash::from_bytes(c.hash()?);
            let deadline_ms = decode_opt_u64(c)?;
            let token = decode_opt_bytes(c)?;
            EventBody::Dispatched {
                id,
                kind,
                family: family.into(),
                target: target.into(),
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
            outcome: decode_close_outcome(c)?,
        },
        8 => EventBody::FoldFailed {
            reason: c.string()?,
            caused_event: Hash::from_bytes(c.hash()?),
        },
        9 => EventBody::Terminated {
            by: Hash::from_bytes(c.hash()?),
            reason: c.string()?,
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
            Payload::Inline(c.take(len)?.to_vec().into())
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
        EventBody::Genesis {
            reducer,
            spawn_nonce,
            parent,
        } => {
            out.push(0);
            out.extend_from_slice(reducer.as_bytes());
            out.extend_from_slice(spawn_nonce.as_bytes());
            // parent: presence byte + 32 hash bytes when Some (mirrors the Event.cause Option<Hash> wire).
            match parent {
                Some(p) => {
                    out.push(1);
                    out.extend_from_slice(p.as_bytes());
                }
                None => out.push(0),
            }
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
            family,
            target,
            idempotency_key,
            deadline_ms,
            token,
        } => {
            out.push(2);
            out.extend_from_slice(&id.0.to_le_bytes());
            out.push(kind_tag(kind));
            encode_str(family, out);
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
            encode_close_outcome(outcome, out);
        }
        EventBody::FoldFailed {
            reason,
            caused_event,
        } => {
            out.push(8);
            encode_str(reason, out);
            out.extend_from_slice(caused_event.as_bytes());
        }
        EventBody::Terminated { by, reason } => {
            out.push(9);
            out.extend_from_slice(by.as_bytes());
            encode_str(reason, out);
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

// BACKWARD-COMPATIBLE with the legacy `Closed { outcome: Payload }` wire (frozen-codec discipline;
// fix-forward on #1938 which shipped a wire-breaking wrapper tag): legacy encoded a bare `Payload` after the
// `7` tag (its own inner tag 0=Inline / 1=Blob). So `Success` encodes BYTE-IDENTICALLY to a legacy Payload
// (NO extra wrapper tag) — an old `Closed` stream decodes as `Success` unchanged, its event hash preserved —
// and `Failure` takes a FRESH tag `2` that a legacy Payload's leading byte never was.
fn encode_close_outcome(o: &CloseOutcome, out: &mut Vec<u8>) {
    match o {
        // No wrapper tag: emit exactly the legacy Payload bytes (leading 0 or 1). Old readers/hashes match.
        CloseOutcome::Success(p) => encode_payload(p, out),
        CloseOutcome::Failure(reason) => {
            out.push(2);
            encode_str(reason, out);
        }
    }
}

fn decode_close_outcome(c: &mut Cursor) -> Result<CloseOutcome, DecodeError> {
    // Peek the leading byte WITHOUT consuming it: 0/1 = a legacy Payload → Success (so old streams parse);
    // 2 = the new Failure tag. `decode_payload` re-reads the 0/1 tag itself, so Success delegates to it.
    Ok(match c.peek_u8()? {
        0 | 1 => CloseOutcome::Success(decode_payload(c)?),
        2 => {
            c.u8()?; // consume the Failure tag
            CloseOutcome::Failure(c.string()?)
        }
        t => {
            return Err(DecodeError::BadTag {
                field: "close_outcome",
                tag: t,
            })
        }
    })
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
                spawn_nonce: Hash::of(b"nonce-v1"),
                parent: None,
            },
        }
    }

    #[test]
    fn event_hash_is_deterministic() {
        assert_eq!(genesis().hash(), genesis().hash());
    }

    #[test]
    fn report_content_type_is_the_well_known_report_family() {
        let ct = ContentType::report();
        assert_eq!(ct.family, "report");
        assert_eq!(ct.family, ContentType::REPORT_FAMILY);
        assert_eq!(ct.version, 1);
        assert!(ct.is_report());
    }

    #[test]
    fn is_report_matches_family_only_and_rejects_other_families() {
        // Version-tolerant (§9b): a different report version is still "a report" — the reducer
        // range-checks version itself if it cares.
        let other_version = ContentType {
            family: "report".into(),
            version: 7,
        };
        assert!(other_version.is_report());
        // A different family is NOT a report.
        let message = ContentType {
            family: "message".into(),
            version: 1,
        };
        assert!(!message.is_report());
    }

    #[test]
    fn matches_family_and_version_in_are_the_tolerant_reader_split() {
        // The §9b tolerant-reader split: family match is version-INDEPENDENT, and the version range is a
        // SEPARATE inclusive check — so "known family, unknown version" is representable (match family,
        // fail version) rather than collapsing into one decode-or-die test.
        let ct = ContentType {
            family: "model-request".into(),
            version: 2,
        };
        // Family match ignores version...
        assert!(ct.matches_family("model-request"));
        assert!(!ct.matches_family("model-response"));
        // ...and the version range is inclusive on both ends.
        assert!(ct.version_in(1, 3));
        assert!(ct.version_in(2, 2));
        assert!(!ct.version_in(3, 5)); // below the range
        assert!(!ct.version_in(0, 1)); // above the range
                                       // The "known family, unknown version" case a v1 reader must DEFER, not misdecode: family matches
                                       // but the version is outside what it handles — the two checks disagree, which is the whole point.
        assert!(ct.matches_family("model-request") && !ct.version_in(1, 1));
        // Totality: an empty/backwards range is simply never satisfied (no panic).
        assert!(!ct.version_in(5, 1));
        // is_report is exactly matches_family(REPORT_FAMILY) — one source of truth.
        let r = ContentType::report();
        assert_eq!(r.is_report(), r.matches_family(ContentType::REPORT_FAMILY));
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
                family: EffectKind::Http.family().into(),
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
        //
        // §19e continuation-token format bumps (ALL DELIBERATE — the §16c-S3 format is PRE-RELEASE, no
        // durable log stream predates them, so extending body variants in place is an intentional call,
        // not an accidental break; PR#1132 review): tag 2 (Dispatched, above) + tag 3 (EffectResult)
        // gained a trailing opt-bytes `token` in slice 2b-i, and tags 4/5/6 (TimerArmed/TimerFired/
        // AuthzDenied) gained the same in slice 2b-iii — each an appended trailing opt-bytes field. If
        // durable-log persistence ships AND gains a real external consumer before the next such change, a
        // further bump must instead use a new tag or a version/length framing layer (so old streams can't
        // desync); until then, in-place extension with this golden pin as the byte-stability tripwire is
        // the deliberate, documented policy. tag 2 (Dispatched) ALSO gained a `family` str after `kind`
        // (host-capability-discovery follow-up): a register-by-string/control dispatch has no distinguishing
        // kind, so recovery needs the family — same deliberate pre-release in-place extension.
        let expected: &[u8] = &[
            42, 0, 0, 0, 0, 0, 0, 0, // seq
            1, // cause: Some
            171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171,
            171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171,
            171, // cause hash
            2,   // body tag = Dispatched
            7, 0, 0, 0, 0, 0, 0, 0, // id
            1, // kind = Http
            4, 0, 0, 0, 0, 0, 0, 0, // family len
            104, 116, 116, 112, // "http"
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
            "9d12eae713d354981db668c8e4d32029754c9fdf1942b582f87404bc1f157a66",
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
            outcome: CloseOutcome::Success(Payload::Inline(vec![].into())),
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
                body: EventBody::Genesis {
                    reducer: h,
                    spawn_nonce: Hash::of(b"spawn-nonce"),
                    parent: Some(Hash::of(b"parent-genesis")),
                },
            },
            Event {
                seq: 1,
                cause: Some(h),
                body: EventBody::Inbound {
                    content_type: ct.clone(),
                    payload: Payload::Inline(b"hello".to_vec().into()),
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
                    family: EffectKind::Http.family().into(),
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
                    family: EffectKind::Shell.family().into(),
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
                    result: EffectOutcome::Ok(Some(Payload::Inline(b"body".to_vec().into()))),
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
                    outcome: CloseOutcome::Success(Payload::Inline(vec![].into())),
                },
            },
            Event {
                // Both CloseOutcome arms in the frozen per-variant harness (fix-forward on #1938): the Failure
                // arm (its own reason string + fresh tag) is exercised by the frozen round-trip net.
                seq: 120,
                cause: None,
                body: EventBody::Closed {
                    outcome: CloseOutcome::Failure("goal abandoned: retries exhausted".to_string()),
                },
            },
            Event {
                seq: 13,
                cause: None,
                body: EventBody::FoldFailed {
                    reason: "wasm reducer trapped: unreachable".to_string(),
                    caused_event: Hash::of(b"the event whose fold failed"),
                },
            },
            Event {
                seq: 14,
                cause: Some(Hash::of(b"terminate-cause")),
                body: EventBody::Terminated {
                    by: Hash::of(b"controller-session"),
                    reason: "operator kill".to_string(),
                },
            },
        ]
    }

    #[test]
    fn legacy_closed_payload_stream_decodes_as_success_unchanged() {
        // FROZEN-WIRE COMPAT (fix-forward on #1938, which shipped a wire-breaking wrapper tag): a Closed
        // stream written BEFORE the CloseOutcome change was push(7) + a bare Payload (its own inner tag
        // 0=Inline / 1=Blob). CloseOutcome::Success must encode BYTE-IDENTICALLY to that so an old log decodes
        // as Success — same bytes, same event hash. Build the legacy stream by hand + prove decode + re-encode.
        let payload = Payload::Inline(b"legacy-result".to_vec().into());
        let (seq, cause) = (5u64, Hash::of(b"parent-close"));
        let mut legacy = Vec::new();
        legacy.extend_from_slice(&seq.to_le_bytes());
        legacy.push(1); // cause present
        legacy.extend_from_slice(cause.as_bytes());
        legacy.push(7); // Closed body tag
        encode_payload(&payload, &mut legacy); // bare Payload, NO wrapper tag (the legacy shape)

        // 1. The legacy stream decodes to Closed{Success(the same payload)} — NOT a parse error.
        let (decoded, n) = Event::decode(&legacy).expect("legacy Closed stream still decodes");
        assert_eq!(n, legacy.len(), "consumes exactly the legacy bytes");
        assert_eq!(
            decoded.body,
            EventBody::Closed {
                outcome: CloseOutcome::Success(payload.clone()),
            },
            "a legacy Closed{{Payload}} stream decodes as Success (backward-compatible)"
        );

        // 2. The NEW encoder produces byte-identical output for that Success → old readers + event hashes match.
        let new_event = Event {
            seq,
            cause: Some(cause),
            body: EventBody::Closed {
                outcome: CloseOutcome::Success(payload),
            },
        };
        assert_eq!(
            new_event.encode(),
            legacy,
            "Success re-encodes byte-identically to the legacy Closed{{Payload}} wire (no wrapper tag)"
        );

        // 3. Failure takes a FRESH tag (2) a legacy Payload never led with (0/1) → no collision, round-trips.
        let fail = Event {
            seq: 6,
            cause: None,
            body: EventBody::Closed {
                outcome: CloseOutcome::Failure("boom".to_string()),
            },
        };
        let (rt, _) = Event::decode(&fail.encode()).unwrap();
        assert_eq!(rt, fail, "Failure round-trips on its fresh tag");
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
