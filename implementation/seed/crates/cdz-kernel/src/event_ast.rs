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
//! then, on a COMPLETE frame, maps ANY `decode` failure to `Corrupt`. That "decode-error → Corrupt" rule
//! is `decode_frames`' complete-frame CONVENTION, not `decode`'s own contract: `decode` called directly
//! on truncated bytes can legitimately return the codec's `Truncated` ("not enough bytes"), which is only
//! ever `Corrupt` because the framing layer has already ruled out truncation before it calls `decode`.
//!
//! Encoding shape (an s-expr form per event, head = a `Name` tag):
//! - `(event <seq> <cause> <body>)` — `seq` an Int; `cause` is `(none)` or `(hash <bytes>)`.
//! - body is one of:
//!   `(genesis <hash>)` ·
//!   `(inbound <family:str> <version:int> <payload>)` ·
//!   `(dispatched <id:int> <kind> <target:str> <idem:hash> <deadline> <token>)` ·
//!   `(effect-result <id:int> <outcome> <token>)` ·
//!   `(timer-armed <id:int> <deadline_ms:int>)` ·
//!   `(timer-fired <id:int> <fired_ms:int>)` ·
//!   `(authz-denied <id:int> <reason:str>)` ·
//!   `(closed <payload>)`
//! - `<kind>` a `Name` atom (shell/http/model/now/timer/emit); `<deadline>` = `(none)` | `(ms <int>)`;
//!   `<payload>` = `(inline <bytes>)` | `(blob <hash>)`; `<outcome>` = `(ok <payload-opt>)` |
//!   `(err <str>)` | `(timed-out)`; `<payload-opt>` = `(none)` | `(some <payload>)`; `<token>` =
//!   `(none)` | `(token <bytes>)`.
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

/// Encode a [`CapabilityManifest`](crate::effect::CapabilityManifest) to the `capabilities-manifest`
/// payload — the Cadenza binary-sexpr a `control/capabilities` query answer / genesis-seed / change-push
/// carries (host-capability-discovery I4, shape agreed with design-host-capabilities). The kernel
/// PRODUCES this; the guest DECODES it (the kernel never re-parses its own control payloads). Shape:
///   `(capabilities-manifest <version:int=1> (entries (entry <family:str> <grant> <scope>) ...))`
///   `<grant>` = `(granted)` | `(denied)` | `(absent)` — Name-HEADED nullary lists, so the pending
///     `(requestable …)` split + future fields append with NO wire break (open sums; a tolerant guest
///     matches the head and ignores/denies an unknown one).
///   `<scope>` = `(none)` | `(some <predicate>)`; `<predicate>` = `(any)` | `(exact <str>)` |
///     `(one-of <str>…)` | `(host-in <str>…)` | `(prefix <str>)` | `(descendant-of <controller-hex>)` (I6
///     supervision marker) — mirrors [`crate::effect::ResourcePredicate`].
/// Entries are emitted in the manifest's order (the projection uses `effect_ct::ALL` order) so the bytes
/// are stable/replay-safe. The leading `<version:int>` INSIDE the payload (=1) lets a decoder range-check
/// without re-reading the envelope's `content_type.version`.
pub fn encode_capability_manifest(manifest: &crate::effect::CapabilityManifest) -> Vec<u8> {
    use crate::effect::{GrantState, ResourcePredicate};
    let mut b = Builder::new();
    let head = b.name("capabilities-manifest");
    let version = u64_leaf(&mut b, 1);
    let entries_head = b.name("entries");
    let mut entry_forms = vec![entries_head];
    for e in &manifest.entries {
        let e_head = b.name("entry");
        let fam = str_leaf(&mut b, &e.family);
        let grant = {
            let g = match e.grant {
                GrantState::Granted => b.name("granted"),
                GrantState::Denied => b.name("denied"),
                GrantState::Absent => b.name("absent"),
            };
            b.list(vec![g])
        };
        let scope = match &e.scope {
            None => {
                let none = b.name("none");
                b.list(vec![none])
            }
            Some(pred) => {
                let pred_form = match pred {
                    ResourcePredicate::Any => {
                        let h = b.name("any");
                        b.list(vec![h])
                    }
                    ResourcePredicate::Exact(s) => {
                        let h = b.name("exact");
                        let v = str_leaf(&mut b, s);
                        b.list(vec![h, v])
                    }
                    ResourcePredicate::OneOf(set) => {
                        let h = b.name("one-of");
                        let mut items = vec![h];
                        for s in set {
                            items.push(str_leaf(&mut b, s));
                        }
                        b.list(items)
                    }
                    ResourcePredicate::HostIn(hosts) => {
                        let h = b.name("host-in");
                        let mut items = vec![h];
                        for s in hosts {
                            items.push(str_leaf(&mut b, s));
                        }
                        b.list(items)
                    }
                    ResourcePredicate::Prefix(p) => {
                        let h = b.name("prefix");
                        let v = str_leaf(&mut b, p);
                        b.list(vec![h, v])
                    }
                    ResourcePredicate::DescendantOf(controller) => {
                        // I6 supervision-tree marker: ride the controller's hash as its hex string, so a
                        // discovery reader sees which controller a lifecycle/* grant is scoped under (the
                        // kernel's own admits() never green-lights it — the host freezes the descendant set).
                        let h = b.name("descendant-of");
                        // Build the leaf from the OWNED hex directly — str_leaf(&str) would re-clone it via
                        // to_string(), a needless second alloc of the 64-char hex (github-liaison #2443 c2).
                        let v = b.atom_leaf(Leaf::Str(controller.to_hex().into()));
                        b.list(vec![h, v])
                    }
                };
                let some = b.name("some");
                b.list(vec![some, pred_form])
            }
        };
        entry_forms.push(b.list(vec![e_head, fam, grant, scope]));
    }
    let entries = b.list(entry_forms);
    let root = b.list(vec![head, version, entries]);
    codec::encode(&b.finish(root))
}

/// Encode an HTTP effect request payload with NO headers — the ergonomic `(method, body)` path, in the
/// shared cadenza binary-sexpr (§9b one-wire-format; the SAME codec as [`encode_capability_manifest`]).
/// This lives kernel-side (effect-schema domain) so the ONE impl is shared by the host's HttpExecutor
/// decode AND any future Cadenza reducer. Emits the SAME 3-field shape as
/// [`encode_http_request_with_headers`] with an empty headers list — so the 2-arg and 3-arg forms are one
/// wire format, and adding headers to a caller is a call-site change, not a wire change. KEPT 2-ARG
/// (didn't fold headers into this signature) so existing callers compile unchanged — additive-for-callers,
/// coordinated with v-agent-harness-host (their queued HTTP work calls this 2-arg; they migrate to the
/// with-headers variant at their pace).
pub fn encode_http_request(method: &str, body: Option<&[u8]>) -> Vec<u8> {
    encode_http_request_with_headers(method, &[], body)
}

/// Encode an HTTP effect request payload carrying HEADERS. Shape:
///   `(http-request (method <name>) (headers ((<k:str> <v:str>) …)) (body <payload-opt>))`
/// `<name>` is a Name atom (open sum — a new method appends with no wire break). `headers` is an ordered
/// list of `(name value)` string pairs (HTTP header values are UTF-8 text; order preserved, repeats OK).
/// `<payload-opt>` = `(none)` | `(some <bytes>)`.
pub fn encode_http_request_with_headers(
    method: &str,
    headers: &[(String, String)],
    body: Option<&[u8]>,
) -> Vec<u8> {
    let mut b = Builder::new();
    let head = b.name("http-request");
    let method_form = {
        let h = b.name("method");
        let m = b.name(method);
        b.list(vec![h, m])
    };
    let headers_form = {
        let h = b.name("headers");
        let pairs = encode_header_pairs(&mut b, headers);
        b.list(vec![h, pairs])
    };
    let body_form = {
        let h = b.name("body");
        let opt = encode_opt_body(&mut b, body);
        b.list(vec![h, opt])
    };
    let root = b.list(vec![head, method_form, headers_form, body_form]);
    codec::encode(&b.finish(root))
}

/// Decode an HTTP effect request payload → `(method, body)`, DROPPING headers — the ergonomic path that
/// mirrors the 2-arg [`encode_http_request`]. KEPT as a 2-tuple (didn't widen to `(method, headers, body)`)
/// so existing decode callers destructure unchanged — additive-for-DECODERS, the dual of the encoder's
/// additive-for-callers. A caller that needs headers uses [`decode_http_request_with_headers`]. Total:
/// non-conforming bytes are an `Err` (never a panic).
pub fn decode_http_request(bytes: &[u8]) -> Result<(String, Option<Vec<u8>>), EventAstError> {
    let (method, _headers, body) = decode_http_request_with_headers(bytes)?;
    Ok((method, body))
}

/// Decode an HTTP effect request payload → `(method, headers, body)` — the full decoder. Total:
/// non-conforming bytes are an `Err` (never a panic). Method is a `String` (the caller maps it to its own
/// enum, with a catch-all — the Name-headed open-sum contract). Accepts BOTH shapes the encoders emit: the
/// 3-field `(http-request (method) (headers) (body))` (both [`encode_http_request`] and
/// [`encode_http_request_with_headers`] emit this — the no-headers encoder writes an empty headers list)
/// AND the legacy 2-field `(http-request (method) (body))` (decodes with empty headers), so a payload from
/// any historical no-headers encoder still decodes.
#[allow(clippy::type_complexity)]
pub fn decode_http_request_with_headers(
    bytes: &[u8],
) -> Result<(String, Vec<(String, String)>, Option<Vec<u8>>), EventAstError> {
    let a = codec::decode_detailed(bytes).map_err(EventAstError::Codec)?;
    let root = a
        .as_form(a.root, "http-request")
        .ok_or(shape("http-request head"))?;
    let (method_f, headers_f, body_f) = match root {
        [m, h, b] => (*m, Some(*h), *b),
        [m, b] => (*m, None, *b),
        _ => return Err(shape("http-request arity")),
    };
    // (method <name>)
    let method_kids = a.as_form(method_f, "method").ok_or(shape("method form"))?;
    let [mname] = method_kids else {
        return Err(shape("method arity"));
    };
    let method = a
        .as_name(*mname)
        .ok_or(shape("method not a name"))?
        .to_string();
    // (headers ((k v) …)) — empty on the legacy form.
    let headers = match headers_f {
        Some(hf) => {
            let hkids = a.as_form(hf, "headers").ok_or(shape("headers form"))?;
            let [pairs] = hkids else {
                return Err(shape("headers arity"));
            };
            read_header_pairs(&a, *pairs)?
        }
        None => Vec::new(),
    };
    // (body <payload-opt>)
    let body_kids = a.as_form(body_f, "body").ok_or(shape("body form"))?;
    let [opt] = body_kids else {
        return Err(shape("body arity"));
    };
    let body = read_opt_body(&a, *opt)?;
    Ok((method, headers, body))
}

/// Encode an HTTP effect RESULT payload — the response the executor produces and the reducer decodes.
/// Shape: `(http-response (status <int>) (headers ((<k:str> <v:str>) …)) (body <bytes>))`. `status` is the
/// numeric HTTP status carried as a `u16` on the wire (an Int). This is a transport codec: it faithfully
/// round-trips ANY `u16` and does NOT enforce the HTTP semantic range (100–599) — asserting a valid status
/// is the caller's responsibility. Same §9b shared-codec rationale.
pub fn encode_http_response(status: u16, headers: &[(String, String)], body: &[u8]) -> Vec<u8> {
    let mut b = Builder::new();
    let head = b.name("http-response");
    let status_form = {
        let h = b.name("status");
        let s = u64_leaf(&mut b, status as u64);
        b.list(vec![h, s])
    };
    let headers_form = {
        let h = b.name("headers");
        let pairs = encode_header_pairs(&mut b, headers);
        b.list(vec![h, pairs])
    };
    let body_form = {
        let h = b.name("body");
        let v = bytes_leaf(&mut b, body);
        b.list(vec![h, v])
    };
    let root = b.list(vec![head, status_form, headers_form, body_form]);
    codec::encode(&b.finish(root))
}

/// Decode an HTTP response payload encoded by [`encode_http_response`] → `(status, headers, body)`. Total.
/// A status outside `u16` range is a clean `Shape` error, never a panic.
#[allow(clippy::type_complexity)]
pub fn decode_http_response(
    bytes: &[u8],
) -> Result<(u16, Vec<(String, String)>, Vec<u8>), EventAstError> {
    let a = codec::decode_detailed(bytes).map_err(EventAstError::Codec)?;
    let root = a
        .as_form(a.root, "http-response")
        .ok_or(shape("http-response head"))?;
    let [status_f, headers_f, body_f] = root else {
        return Err(shape("http-response arity"));
    };
    let status_kids = a.as_form(*status_f, "status").ok_or(shape("status form"))?;
    let [sv] = status_kids else {
        return Err(shape("status arity"));
    };
    let status = u16::try_from(read_u64(&a, *sv)?).map_err(|_| shape("status out of u16 range"))?;
    let hkids = a
        .as_form(*headers_f, "headers")
        .ok_or(shape("headers form"))?;
    let [pairs] = hkids else {
        return Err(shape("headers arity"));
    };
    let headers = read_header_pairs(&a, *pairs)?;
    let body_kids = a.as_form(*body_f, "body").ok_or(shape("body form"))?;
    let [v] = body_kids else {
        return Err(shape("body arity"));
    };
    let body = read_bytes(&a, *v)?;
    Ok((status, headers, body))
}

/// Encode a §4c mutable-name `set(name, hash)` log entry — the PAYLOAD of one append to a name's
/// append-only value-over-time log (`system/compiler/latest → <hash>` etc.). Shape:
///   `(name-set (name <str>) (hash <hash>))`
/// `name` is the full mutable-name string (its PREFIX is what the write-authority parse gates — §4c point
/// 2); `hash` is the content-address the name now points at. This is the NON-CRYPTO half (concierge
/// greenlit 2026-08-03): the signature envelope (`producer`/`signature`, §10) rides the EVENT envelope
/// around this payload, layered once the operator approves the signing scheme — this codec is unchanged by
/// that (additive: the signed log is these entries + an envelope, not a different payload). Same §9b
/// shared-codec rationale as the other payload codecs here.
pub fn encode_name_set(name: &str, hash: &Hash) -> Vec<u8> {
    let mut b = Builder::new();
    let head = b.name("name-set");
    let name_form = {
        let h = b.name("name");
        let v = str_leaf(&mut b, name);
        b.list(vec![h, v])
    };
    let hash_f = {
        let h = b.name("hash");
        let hf = hash_form(&mut b, hash);
        b.list(vec![h, hf])
    };
    let root = b.list(vec![head, name_form, hash_f]);
    codec::encode(&b.finish(root))
}

/// Decode a §4c `set(name, hash)` log entry encoded by [`encode_name_set`] → `(name, hash)`. Total:
/// non-conforming bytes are a clean `Shape` error, never a panic (the bytes ride a durable log / an
/// untrusted store, so a malformed entry must fail cleanly, not crash the resolver).
pub fn decode_name_set(bytes: &[u8]) -> Result<(String, Hash), EventAstError> {
    let a = codec::decode_detailed(bytes).map_err(EventAstError::Codec)?;
    let [name_f, hash_f] = a
        .as_form(a.root, "name-set")
        .ok_or(shape("name-set head"))?
    else {
        return Err(shape("name-set arity"));
    };
    let name_kids = a.as_form(*name_f, "name").ok_or(shape("name form"))?;
    let [nv] = name_kids else {
        return Err(shape("name arity"));
    };
    let name = read_str(&a, *nv)?;
    let hash_kids = a.as_form(*hash_f, "hash").ok_or(shape("hash form"))?;
    let [hv] = hash_kids else {
        return Err(shape("hash-wrapper arity"));
    };
    let hash = read_hash(&a, *hv)?;
    Ok((name, hash))
}

/// Encode a §4c session-directory GROUP add/remove op (I2 durable frame) as canonical AST:
///
///   `(member-op (name <str>) (add <0|1>) (member <hash>) (tag <origin-hash> <seq-u64>))`
///
/// The durable counterpart to a [`crate::name_store::MemberOp`] — a group name's log persists these frames
/// (one per add/remove), and snapshot/replay round-trips the whole OR-set through them. `add` is a 0/1 flag
/// (no bool-leaf in this codec); `tag` is the `(origin, seq)` that makes each add unique + a remove precise.
/// Same shared-codec + total-decode discipline as [`encode_name_set`] (the bytes ride a durable log).
pub fn encode_member_op(name: &str, add: bool, member: &Hash, tag: &(Hash, u64)) -> Vec<u8> {
    let mut b = Builder::new();
    let head = b.name("member-op");
    let name_form = {
        let h = b.name("name");
        let v = str_leaf(&mut b, name);
        b.list(vec![h, v])
    };
    let add_form = {
        let h = b.name("add");
        let v = u64_leaf(&mut b, add as u64);
        b.list(vec![h, v])
    };
    let member_form = {
        let h = b.name("member");
        let mf = hash_form(&mut b, member);
        b.list(vec![h, mf])
    };
    let tag_form = {
        let h = b.name("tag");
        let origin = hash_form(&mut b, &tag.0);
        let seq = u64_leaf(&mut b, tag.1);
        b.list(vec![h, origin, seq])
    };
    let root = b.list(vec![head, name_form, add_form, member_form, tag_form]);
    codec::encode(&b.finish(root))
}

/// Decode a `member-op` frame encoded by [`encode_member_op`] → `(name, add, member, tag)`. Total:
/// non-conforming bytes are a clean `Shape` error, never a panic (durable-log / untrusted-store bytes).
pub fn decode_member_op(bytes: &[u8]) -> Result<(String, bool, Hash, (Hash, u64)), EventAstError> {
    let a = codec::decode_detailed(bytes).map_err(EventAstError::Codec)?;
    let [name_f, add_f, member_f, tag_f] = a
        .as_form(a.root, "member-op")
        .ok_or(shape("member-op head"))?
    else {
        return Err(shape("member-op arity"));
    };
    let name = {
        let [nv] = a.as_form(*name_f, "name").ok_or(shape("name form"))? else {
            return Err(shape("name arity"));
        };
        read_str(&a, *nv)?
    };
    let add = {
        let [av] = a.as_form(*add_f, "add").ok_or(shape("add form"))? else {
            return Err(shape("add arity"));
        };
        read_u64(&a, *av)? != 0
    };
    let member = {
        let [mv] = a.as_form(*member_f, "member").ok_or(shape("member form"))? else {
            return Err(shape("member arity"));
        };
        read_hash(&a, *mv)?
    };
    let tag = {
        let [ov, sv] = a.as_form(*tag_f, "tag").ok_or(shape("tag form"))? else {
            return Err(shape("tag arity"));
        };
        (read_hash(&a, *ov)?, read_u64(&a, *sv)?)
    };
    Ok((name, add, member, tag))
}

/// Encode a §4c session-directory `store/resolve-all` RESULT — a group's current membership (add-wins fold)
/// as canonical AST:
///
///   `(members (member <hash>) (member <hash>) …)`
///
/// The result payload the drive loop returns for a `store/resolve-all` (§4c I3b), so a guest reader decodes
/// the frozen member set through this ONE codec (the group analogue of a `store/resolve` returning a single
/// `name-set` hash). `members` is passed in ASCENDING-hash order (the `BTreeSet<Hash>` iteration order from
/// [`crate::name_store::NameStore::resolve_all`]) so the encoded bytes are BYTE-STABLE — a frozen membership
/// content-addresses identically, and a multicast (§8) fan-out over the decoded set is deterministic. An empty
/// group encodes to `(members)` (a live group whose members were all removed — distinct from a NoSuchName
/// error, which the drive loop folds as an `Err` outcome, never an empty `members`). Same shared-codec +
/// total-decode discipline as [`encode_member_op`] (the bytes ride the durable result log).
pub fn encode_members<'a>(members: impl IntoIterator<Item = &'a Hash>) -> Vec<u8> {
    let mut b = Builder::new();
    let head = b.name("members");
    let mut items = vec![head];
    for m in members {
        let h = b.name("member");
        let mf = hash_form(&mut b, m);
        items.push(b.list(vec![h, mf]));
    }
    let root = b.list(items);
    codec::encode(&b.finish(root))
}

/// Decode a `members` frame encoded by [`encode_members`] → the member hashes in ENCODED order.
/// [`encode_members`] preserves the CALLER's iteration order (it does NOT sort), so byte-stability is a
/// caller contract — feed a canonical (e.g. `BTreeSet`) order and a round-trip reproduces the same bytes.
/// Total, distinguishing the two [`EventAstError`] classes: bytes that don't parse as a canonical codec
/// frame → [`EventAstError::Codec`]; a well-formed frame with the wrong head / a non-`member` child / a bad
/// hash → [`EventAstError::Shape`]. Never a panic (durable-log / untrusted-store bytes). Returns a `Vec` (not
/// a set) so a caller can observe the exact encoded order; collecting into a `BTreeSet` recovers the set.
pub fn decode_members(bytes: &[u8]) -> Result<Vec<Hash>, EventAstError> {
    let a = codec::decode_detailed(bytes).map_err(EventAstError::Codec)?;
    let member_forms = a.as_form(a.root, "members").ok_or(shape("members head"))?;
    let mut out = Vec::with_capacity(member_forms.len());
    for &mf in member_forms {
        let [hv] = a.as_form(mf, "member").ok_or(shape("member form"))? else {
            return Err(shape("member arity"));
        };
        out.push(read_hash(&a, *hv)?);
    }
    Ok(out)
}

/// Encode a §lifecycle-I7 `ChildExited` supervision signal as canonical AST — the payload the HOST emits into
/// a terminated/closed child's PARENT inbox (`(child, CloseOutcome)`), for the parent's userspace supervisor
/// reducer to fold (restart/escalate). Shape:
///
///   `(child-exited (child <hash>) <close-outcome>)`
///
/// where `<close-outcome>` is the SAME form the `Closed` event uses (§6 supervision slice-1): a bare payload
/// form (`(inline …)`/`(blob …)`) for `Success`, or `(failure <str>)` for `Failure` — decoded by the shared
/// `read_close_outcome`. Giving the prelude ONE authoritative encode/decode pair (like [`encode_member_op`]/
/// [`encode_name_set`]) keeps the host emit + prelude consume in lockstep on a stable wire, rather than an
/// ad-hoc host-side framing the prelude would have to duplicate. Same shared-codec + total-decode discipline.
///
/// This codec is FAMILY-AGNOSTIC — it encodes only the payload. The host emits it as an INBOUND (host-as-
/// sender, reusing the Emit/inbox path) into the parent's inbox with content-type family
/// `"lifecycle/child-exited"` (confirmed with v-agent-harness-host — parallels the `lifecycle/*` session-
/// control partition, clearer than bare "supervision"). NOTE it is NOT an authz-gated `lifecycle/*` EFFECT
/// the parent performs (no [`is_lifecycle_family`](crate::effect::effect_ct::is_lifecycle_family) authz path):
/// the family string is just the Inbound content-type the prelude supervisor matches on before calling
/// [`decode_child_exited`]. So: `Inbound{ family="lifecycle/child-exited", version=1, payload=encode_child_exited(child, outcome) }`.
pub fn encode_child_exited(child: &Hash, outcome: &crate::event::CloseOutcome) -> Vec<u8> {
    let mut b = Builder::new();
    let head = b.name("child-exited");
    let child_form = {
        let h = b.name("child");
        let hf = hash_form(&mut b, child);
        b.list(vec![h, hf])
    };
    // The close-outcome form — byte-identical to the `Closed` event's outcome encoding (bare payload for
    // Success, `(failure <str>)` for Failure), so the ONE `read_close_outcome` decodes both.
    let outcome_form = match outcome {
        crate::event::CloseOutcome::Success(p) => payload_form(&mut b, p),
        crate::event::CloseOutcome::Failure(reason) => {
            let h = b.name("failure");
            let r = str_leaf(&mut b, reason);
            b.list(vec![h, r])
        }
    };
    let root = b.list(vec![head, child_form, outcome_form]);
    codec::encode(&b.finish(root))
}

/// Decode a `child-exited` frame encoded by [`encode_child_exited`] → `(child, CloseOutcome)`. Total, and it
/// distinguishes the two [`EventAstError`] classes (per the contract at the top of this file): bytes that
/// don't parse as a canonical codec frame → [`EventAstError::Codec`]; a well-formed frame whose SHAPE is
/// wrong (bad head/arity, a non-32-byte hash, an unknown close-outcome head) → [`EventAstError::Shape`].
/// Never a panic — the bytes ride the durable inbox / an untrusted sender. Reuses `read_close_outcome` so
/// the outcome accepts the SAME shapes the `Closed` event does.
pub fn decode_child_exited(
    bytes: &[u8],
) -> Result<(Hash, crate::event::CloseOutcome), EventAstError> {
    let a = codec::decode_detailed(bytes).map_err(EventAstError::Codec)?;
    let [child_f, outcome_f] = a
        .as_form(a.root, "child-exited")
        .ok_or(shape("child-exited head"))?
    else {
        return Err(shape("child-exited arity"));
    };
    let child = {
        let [hv] = a.as_form(*child_f, "child").ok_or(shape("child form"))? else {
            return Err(shape("child arity"));
        };
        read_hash(&a, *hv)?
    };
    let outcome = read_close_outcome(&a, *outcome_f)?;
    Ok((child, outcome))
}

/// One CONTENT BLOCK within a conversation turn (§GAP-1 M1). A turn's content is an ORDERED list of these,
/// so a single assistant turn can carry text AND tool-calls, and a tool turn can carry tool-results — the
/// shape the agentic loop needs to CORRELATE results back to the model (v-agent-harness-host transport
/// constraint: Bedrock `toolUseId` must round-trip verbatim response→request). An OPEN sum: an unknown
/// block head is a host/reducer concern, decoded structurally here.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ContentBlock {
    /// Plain text (a user prompt, an assistant's prose, a system instruction).
    Text(String),
    /// An assistant tool-CALL: `id` (the transport's tool-use id, round-tripped verbatim), the tool `name`,
    /// and opaque `input` bytes (the call's arguments — host owns the JSON⟷Bedrock boundary, O4).
    ToolCall {
        id: String,
        name: String,
        input: Vec<u8>,
    },
    /// A tool-RESULT turn's block: the `id` MUST equal the originating [`ToolCall`](Self::ToolCall)'s `id`
    /// (so the model correlates it), plus opaque `result` bytes (what the dispatched effect produced).
    ToolResult { id: String, result: Vec<u8> },
}

/// One conversation turn in a tool-calling model request (§GAP-1 M1). `role` ∈ {system, user, assistant,
/// tool} — an OPEN sum (tolerant: the kernel carries the string, the host maps it to the transport's role
/// vocab; an unknown role is a host concern, not a decode error). `content` is an ORDERED list of
/// [`ContentBlock`]s (text / tool-call / tool-result), so an assistant turn can carry tool-calls and a tool
/// turn can carry results — the loop-closing shape (tool-use-id round-trips through the conversation).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ChatMessage {
    pub role: String,
    pub content: Vec<ContentBlock>,
}

/// A tool definition offered to the model this turn (§GAP-1 M1): a `name` the model calls back by, plus an
/// opaque `schema` (JSON-schema bytes the kernel does NOT interpret — the host maps it to the transport's
/// tool-config; O4: the kernel carries bytes, the host owns the JSON⟷Bedrock boundary).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ToolDef {
    pub name: String,
    pub schema: Vec<u8>,
}

/// A tool-calling model request (§GAP-1 M1 — the keystone of the self-hosting agent loop). The reducer
/// builds this from its accumulated conversation (KV state) + the tools it offers, and emits it as the
/// `model` effect's payload; the host transport decodes it and maps to Bedrock `Converse` w/ `toolConfig`.
/// ADDITIVE to the opaque `model` effect: a single-shot invoke is the degenerate `tools=[]` case, and a
/// legacy opaque body still works (the drive loop hands the transport whatever payload the reducer emitted).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ModelRequest {
    /// The model id — also the effect TARGET (authz gates on it), duplicated in-payload so the transport
    /// reads one self-contained request.
    pub model: String,
    /// The conversation so far, oldest→newest (the reducer's fold state).
    pub messages: Vec<ChatMessage>,
    /// The tools offered THIS turn (empty = a plain no-tools completion).
    pub tools: Vec<ToolDef>,
    /// Optional cap on generated tokens (`None` = transport default). Additive room for more opts later.
    pub max_tokens: Option<u64>,
}

/// Encode one [`ContentBlock`] as its headed AST form (shared by the message encoder): `(text <str>)` |
/// `(tool-call (id <str>) (name <str>) (input <bytes>))` | `(tool-result (id <str>) (result <bytes>))`.
fn content_block_form(b: &mut Builder, block: &ContentBlock) -> StructId {
    match block {
        ContentBlock::Text(t) => {
            let h = b.name("text");
            let v = str_leaf(b, t);
            b.list(vec![h, v])
        }
        ContentBlock::ToolCall { id, name, input } => {
            let h = b.name("tool-call");
            let id_f = {
                let ih = b.name("id");
                let iv = str_leaf(b, id);
                b.list(vec![ih, iv])
            };
            let name_f = {
                let nh = b.name("name");
                let nv = str_leaf(b, name);
                b.list(vec![nh, nv])
            };
            let input_f = {
                let ph = b.name("input");
                let pv = bytes_leaf(b, input);
                b.list(vec![ph, pv])
            };
            b.list(vec![h, id_f, name_f, input_f])
        }
        ContentBlock::ToolResult { id, result } => {
            let h = b.name("tool-result");
            let id_f = {
                let ih = b.name("id");
                let iv = str_leaf(b, id);
                b.list(vec![ih, iv])
            };
            let result_f = {
                let rh = b.name("result");
                let rv = bytes_leaf(b, result);
                b.list(vec![rh, rv])
            };
            b.list(vec![h, id_f, result_f])
        }
    }
}

/// Decode one [`ContentBlock`] from its headed AST form — the dual of [`content_block_form`]. An unknown
/// head is a `Shape` error (the reducer/host defines the block vocab; the codec pins the M1 set).
fn read_content_block(a: &Arenas, id: StructId) -> Result<ContentBlock, EventAstError> {
    match head_of(a, id)? {
        "text" => {
            let [v] = a.as_form(id, "text").ok_or(shape("text form"))? else {
                return Err(shape("text arity"));
            };
            Ok(ContentBlock::Text(read_str(a, *v)?))
        }
        "tool-call" => {
            let [id_f, name_f, input_f] =
                a.as_form(id, "tool-call").ok_or(shape("tool-call form"))?
            else {
                return Err(shape("tool-call arity (want (id)(name)(input))"));
            };
            let [iv] = a.as_form(*id_f, "id").ok_or(shape("tool-call id form"))? else {
                return Err(shape("tool-call id arity"));
            };
            let [nv] = a
                .as_form(*name_f, "name")
                .ok_or(shape("tool-call name form"))?
            else {
                return Err(shape("tool-call name arity"));
            };
            let [pv] = a.as_form(*input_f, "input").ok_or(shape("input form"))? else {
                return Err(shape("input arity"));
            };
            Ok(ContentBlock::ToolCall {
                id: read_str(a, *iv)?,
                name: read_str(a, *nv)?,
                input: read_bytes(a, *pv)?,
            })
        }
        "tool-result" => {
            let [id_f, result_f] = a
                .as_form(id, "tool-result")
                .ok_or(shape("tool-result form"))?
            else {
                return Err(shape("tool-result arity (want (id)(result))"));
            };
            let [iv] = a.as_form(*id_f, "id").ok_or(shape("tool-result id form"))? else {
                return Err(shape("tool-result id arity"));
            };
            let [rv] = a.as_form(*result_f, "result").ok_or(shape("result form"))? else {
                return Err(shape("result arity"));
            };
            Ok(ContentBlock::ToolResult {
                id: read_str(a, *iv)?,
                result: read_bytes(a, *rv)?,
            })
        }
        _ => Err(shape("unknown content-block head")),
    }
}

/// Encode a §GAP-1 M1 tool-calling [`ModelRequest`] as canonical AST — the `model` effect's structured
/// payload. Shape:
///
///   `(model-request (model <str>)
///      (messages (message (role <str>) (content <block> …)) …)
///      (tools (tool (name <str>) (schema <bytes>)) …) (opts (max-tokens <int>)?))`
///
/// where `<block>` = `(text <str>)` | `(tool-call (id)(name)(input <bytes>))` | `(tool-result (id)(result
/// <bytes>))` — so an assistant turn carries tool-CALLs and a tool turn carries tool-RESULTs, and the
/// tool-use `id` ROUND-TRIPS through the conversation (v-agent-harness-host constraint: Bedrock toolUseId
/// must correlate result→call). The reducer builds it; the host transport decodes it
/// ([`decode_model_request`]) and maps to Bedrock Converse + toolConfig. `schema`/`input`/`result` ride as
/// opaque bytes (kernel doesn't interpret — O4). `opts` is a headed form so it grows additively (only
/// `max-tokens` in M1). Same shared-codec + total-decode discipline as the other event_ast codecs.
pub fn encode_model_request(req: &ModelRequest) -> Vec<u8> {
    let mut b = Builder::new();
    let head = b.name("model-request");
    let model_form = {
        let h = b.name("model");
        let v = str_leaf(&mut b, &req.model);
        b.list(vec![h, v])
    };
    let messages_form = {
        let h = b.name("messages");
        let mut items = vec![h];
        for m in &req.messages {
            let mh = b.name("message");
            let role = {
                let rh = b.name("role");
                let rv = str_leaf(&mut b, &m.role);
                b.list(vec![rh, rv])
            };
            let content = {
                let ch = b.name("content");
                let mut blocks = vec![ch];
                for blk in &m.content {
                    blocks.push(content_block_form(&mut b, blk));
                }
                b.list(blocks)
            };
            items.push(b.list(vec![mh, role, content]));
        }
        b.list(items)
    };
    let tools_form = {
        let h = b.name("tools");
        let mut items = vec![h];
        for t in &req.tools {
            let th = b.name("tool");
            let name = {
                let nh = b.name("name");
                let nv = str_leaf(&mut b, &t.name);
                b.list(vec![nh, nv])
            };
            let schema = {
                let sh = b.name("schema");
                let sv = bytes_leaf(&mut b, &t.schema);
                b.list(vec![sh, sv])
            };
            items.push(b.list(vec![th, name, schema]));
        }
        b.list(items)
    };
    let opts_form = {
        let h = b.name("opts");
        let mut items = vec![h];
        if let Some(mt) = req.max_tokens {
            let mth = b.name("max-tokens");
            let mtv = u64_leaf(&mut b, mt);
            items.push(b.list(vec![mth, mtv]));
        }
        b.list(items)
    };
    let root = b.list(vec![head, model_form, messages_form, tools_form, opts_form]);
    codec::encode(&b.finish(root))
}

/// Decode a `model-request` frame encoded by [`encode_model_request`] → a [`ModelRequest`]. Total,
/// distinguishing the two [`EventAstError`] classes: non-parseable codec bytes → [`EventAstError::Codec`];
/// a well-formed frame with a bad head/arity / non-string field / bad opt → [`EventAstError::Shape`]. Never
/// a panic (the bytes ride the durable effect log / a guest reducer). An absent `max-tokens` (empty `opts`)
/// decodes to `None`.
pub fn decode_model_request(bytes: &[u8]) -> Result<ModelRequest, EventAstError> {
    let a = codec::decode_detailed(bytes).map_err(EventAstError::Codec)?;
    let [model_f, messages_f, tools_f, opts_f] = a
        .as_form(a.root, "model-request")
        .ok_or(shape("model-request head"))?
    else {
        return Err(shape("model-request arity"));
    };
    let model = {
        let [mv] = a.as_form(*model_f, "model").ok_or(shape("model form"))? else {
            return Err(shape("model arity"));
        };
        read_str(&a, *mv)?
    };
    // messages: (messages (message (role <str>) (content <str>)) …)
    let messages = {
        let msg_forms = a
            .as_form(*messages_f, "messages")
            .ok_or(shape("messages form"))?;
        let mut out = Vec::with_capacity(msg_forms.len());
        for &mf in msg_forms {
            let [role_f, content_f] = a.as_form(mf, "message").ok_or(shape("message form"))? else {
                return Err(shape("message arity (want (role)(content))"));
            };
            let [rv] = a.as_form(*role_f, "role").ok_or(shape("role form"))? else {
                return Err(shape("role arity"));
            };
            // content is a variable-length list of blocks: (content <block> …)
            let block_forms = a
                .as_form(*content_f, "content")
                .ok_or(shape("content form"))?;
            let mut content = Vec::with_capacity(block_forms.len());
            for &bf in block_forms {
                content.push(read_content_block(&a, bf)?);
            }
            out.push(ChatMessage {
                role: read_str(&a, *rv)?,
                content,
            });
        }
        out
    };
    // tools: (tools (tool (name <str>) (schema <bytes>)) …)
    let tools = {
        let tool_forms = a.as_form(*tools_f, "tools").ok_or(shape("tools form"))?;
        let mut out = Vec::with_capacity(tool_forms.len());
        for &tf in tool_forms {
            let [name_f, schema_f] = a.as_form(tf, "tool").ok_or(shape("tool form"))? else {
                return Err(shape("tool arity (want (name)(schema))"));
            };
            let [nv] = a.as_form(*name_f, "name").ok_or(shape("tool name form"))? else {
                return Err(shape("tool name arity"));
            };
            let [sv] = a.as_form(*schema_f, "schema").ok_or(shape("schema form"))? else {
                return Err(shape("schema arity"));
            };
            out.push(ToolDef {
                name: read_str(&a, *nv)?,
                schema: read_bytes(&a, *sv)?,
            });
        }
        out
    };
    // opts: (opts (max-tokens <int>)?) — only max-tokens in M1; an absent child = None.
    let max_tokens = {
        let opt_forms = a.as_form(*opts_f, "opts").ok_or(shape("opts form"))?;
        let mut mt = None;
        for &of in opt_forms {
            if let Some([v]) = a.as_form(of, "max-tokens") {
                mt = Some(read_u64(&a, *v)?);
            }
            // Unknown opt heads are IGNORED (additive-forward: an older decoder tolerates a newer opt).
        }
        mt
    };
    Ok(ModelRequest {
        model,
        messages,
        tools,
        max_tokens,
    })
}

/// A tool-calling model RESPONSE (§GAP-1 M2) — what the host transport produces from the LLM (mapping
/// Bedrock Converse output back) and the reducer folds to drive the next loop step. The reducer matches on
/// `stop_reason` (a RAW STRING, never a kernel enum — Bedrock adds stopReason values over time; the kernel
/// carries the string verbatim and the reducer decides, so a new AWS value folds as not-`tool_use`=done) +
/// walks `content` (assistant text and/or tool-calls the reducer must dispatch as effects).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ModelResponse {
    /// The transport's raw stop reason (e.g. `end_turn`, `tool_use`, `max_tokens`, `stop_sequence`, …). A
    /// free string — the kernel does NOT validate the set (host/reducer own the vocab). `tool_use` ⇒ the
    /// reducer dispatches the `content`'s tool-calls; anything else ⇒ the turn is done.
    pub stop_reason: String,
    /// The assistant's output blocks, in order: [`ContentBlock::Text`] prose and/or
    /// [`ContentBlock::ToolCall`]s (id + name + opaque input bytes). A response never carries a
    /// `ToolResult` (that's the reducer's next REQUEST turn), but the shared [`ContentBlock`] grammar keeps
    /// ONE block codec across request+response.
    pub content: Vec<ContentBlock>,
}

/// Encode a §GAP-1 M2 [`ModelResponse`] as canonical AST — the `model` effect's structured RESULT payload
/// (the host builds it from the LLM; the reducer folds it). Shape:
///
///   `(model-response (stop-reason <str>) (content <block> …))`
///
/// where `<block>` is the SHARED `content_block_form` grammar (`(text …)` / `(tool-call (id)(name)(input
/// <bytes>))`; a response won't emit `(tool-result …)` but the codec accepts the shared set). `stop-reason`
/// is a RAW STRING — never narrowed to an enum, so a future transport stop-reason decodes fine and the
/// reducer folds it. Same shared-codec + total-decode discipline as the other event_ast codecs.
pub fn encode_model_response(resp: &ModelResponse) -> Vec<u8> {
    let mut b = Builder::new();
    let head = b.name("model-response");
    let stop_form = {
        let h = b.name("stop-reason");
        let v = str_leaf(&mut b, &resp.stop_reason);
        b.list(vec![h, v])
    };
    let content_form = {
        let h = b.name("content");
        let mut blocks = vec![h];
        for blk in &resp.content {
            blocks.push(content_block_form(&mut b, blk));
        }
        b.list(blocks)
    };
    let root = b.list(vec![head, stop_form, content_form]);
    codec::encode(&b.finish(root))
}

/// Decode a `model-response` frame encoded by [`encode_model_response`] → a [`ModelResponse`]. Total,
/// distinguishing the two [`EventAstError`] classes: non-parseable codec bytes → [`EventAstError::Codec`];
/// a well-formed frame with a bad head/arity / non-string stop-reason / bad content block →
/// [`EventAstError::Shape`]. Never a panic (the bytes ride the durable effect-result log / the host).
pub fn decode_model_response(bytes: &[u8]) -> Result<ModelResponse, EventAstError> {
    let a = codec::decode_detailed(bytes).map_err(EventAstError::Codec)?;
    let [stop_f, content_f] = a
        .as_form(a.root, "model-response")
        .ok_or(shape("model-response head"))?
    else {
        return Err(shape("model-response arity"));
    };
    let stop_reason = {
        let [sv] = a
            .as_form(*stop_f, "stop-reason")
            .ok_or(shape("stop-reason form"))?
        else {
            return Err(shape("stop-reason arity"));
        };
        read_str(&a, *sv)?
    };
    let content = {
        let block_forms = a
            .as_form(*content_f, "content")
            .ok_or(shape("content form"))?;
        let mut out = Vec::with_capacity(block_forms.len());
        for &bf in block_forms {
            out.push(read_content_block(&a, bf)?);
        }
        out
    };
    Ok(ModelResponse {
        stop_reason,
        content,
    })
}

/// One metric sample a reducer publishes (§operator-Q3 metrics-publish). `kind` is a RAW STRING
/// (counter/gauge/timing/… — the reducer + host metric backend own the vocab; the kernel carries it, never
/// validates). `value` is an f64. `labels` are ordered `(key, val)` string pairs; the HOST applies its
/// cardinality bound + guest-string safety (a host concern, not the kernel's — the kernel carries opaque
/// key/val bytes). This is the reducer-facing wire shape; metric SEMANTICS live in the reducer (policy on
/// the log, minimize-kernel-logic).
#[derive(Clone, PartialEq, Debug)]
pub struct MetricSample {
    pub name: String,
    pub kind: String,
    pub value: f64,
    /// The metric UNIT (operator Q3 round-3) — a RAW STRING vocab (`count`/`bytes`/`milliseconds`/`seconds`/
    /// `percent`/`none`), parity with `kind`: the kernel carries it, never validates; the host maps it onto
    /// its backend unit model (e.g. the s2n-quic-dc Registry `Unit` enum). Empty string = unspecified (the
    /// host defaults it, e.g. Count). Encoded as an OPTIONAL `(unit <str>)` sub-form so the landed 4-field
    /// `metric-publish` (#2545, no unit) still decodes — an absent unit reads as `""` (backward-compatible).
    pub unit: String,
    pub labels: Vec<(String, String)>,
}

/// Encode a §operator-Q3 [`MetricSample`] as canonical AST — the `metric/publish` effect's payload. Shape:
///
///   `(metric-publish (name <str>) (kind <str>) (value <bytes:f64-le>) (labels (label (key <str>) (val <str>)) …))`
///
/// `value` rides as the 8 IEEE-754 little-endian bytes of the f64 (EXACT + portable — the codec has no float
/// leaf, and bytes avoid any parse/round-trip rounding). `kind` is a raw string (host/reducer own the vocab).
/// The reducer builds it; the host `MetricExecutor` decodes it ([`decode_metric_publish`]) and records into
/// the metrics Registry (applying its own cardinality/safety bound on the labels). Same shared-codec +
/// total-decode discipline as the other event_ast codecs.
/// Build one sample's `(name … kind … value … labels …)` field forms under a caller-supplied head — the
/// shared body of both the single `(metric-publish …)` and each `(metric-sample …)` element in a batch
/// (operator #2545 review: publish >1 metric per effect). Returns the headed list.
fn metric_sample_form(b: &mut Builder, head_name: &'static str, m: &MetricSample) -> StructId {
    let head = b.name(head_name);
    let name_form = {
        let h = b.name("name");
        let v = str_leaf(b, &m.name);
        b.list(vec![h, v])
    };
    let kind_form = {
        let h = b.name("kind");
        let v = str_leaf(b, &m.kind);
        b.list(vec![h, v])
    };
    let value_form = {
        let h = b.name("value");
        // f64 as its 8 IEEE-754 LE bytes — exact, portable (no float leaf in the codec).
        let v = bytes_leaf(b, &m.value.to_le_bytes());
        b.list(vec![h, v])
    };
    let labels_form = {
        let h = b.name("labels");
        let mut items = vec![h];
        for (k, val) in &m.labels {
            let lh = b.name("label");
            let key_f = {
                let kh = b.name("key");
                let kv = str_leaf(b, k);
                b.list(vec![kh, kv])
            };
            let val_f = {
                let vh = b.name("val");
                let vv = str_leaf(b, val);
                b.list(vec![vh, vv])
            };
            items.push(b.list(vec![lh, key_f, val_f]));
        }
        b.list(items)
    };
    // UNIT (operator Q3 round-3): append an OPTIONAL `(unit <str>)` AFTER labels, ONLY when non-empty — so a
    // unit-less sample encodes BYTE-IDENTICALLY to the landed 4-field #2545 form (no wire break); decode is
    // arity-tolerant (4 children = no unit → "", 5 = read it).
    let mut fields = vec![head, name_form, kind_form, value_form, labels_form];
    if !m.unit.is_empty() {
        let h = b.name("unit");
        let v = str_leaf(b, &m.unit);
        fields.push(b.list(vec![h, v]));
    }
    b.list(fields)
}

pub fn encode_metric_publish(m: &MetricSample) -> Vec<u8> {
    let mut b = Builder::new();
    let root = metric_sample_form(&mut b, "metric-publish", m);
    codec::encode(&b.finish(root))
}

/// Encode a BATCH of metric samples as one payload (operator #2545 review — "publish more than one metric
/// at a time in a single effect"): `(metrics-publish (metric-sample …) …)`, each element the same body as a
/// single [`encode_metric_publish`]. Lets a reducer emit ONE `metric/publish` effect for all the metrics it
/// records in a turn (fewer effects, one host record-batch), instead of one effect per sample. Empty batch =
/// `(metrics-publish)`. The host `MetricExecutor` decodes via [`decode_metrics_publish`] + records each.
pub fn encode_metrics_publish(samples: &[MetricSample]) -> Vec<u8> {
    let mut b = Builder::new();
    let head = b.name("metrics-publish");
    let mut items = vec![head];
    for m in samples {
        items.push(metric_sample_form(&mut b, "metric-sample", m));
    }
    let root = b.list(items);
    codec::encode(&b.finish(root))
}

/// Decode a `metric-publish` frame encoded by [`encode_metric_publish`] → a [`MetricSample`]. Total,
/// distinguishing the two [`EventAstError`] classes: non-parseable codec bytes → [`EventAstError::Codec`];
/// a well-formed frame with a bad head/arity / non-string field / a `value` that isn't exactly 8 bytes →
/// [`EventAstError::Shape`]. Never a panic (the bytes ride the durable effect log / a guest reducer).
/// Read one sample's fields from its headed form (`head_name` = `metric-publish` for a single, or
/// `metric-sample` for a batch element) → a [`MetricSample`]. The shared body of both decoders.
fn read_metric_sample(
    a: &Arenas,
    id: StructId,
    head_name: &'static str,
) -> Result<MetricSample, EventAstError> {
    // Arity-tolerant (operator Q3 round-3 unit): 4 fields = the landed #2545 shape (no unit → ""), 5 = an
    // appended optional `(unit <str>)`.
    let fields = a
        .as_form(id, head_name)
        .ok_or(shape("metric-sample head"))?;
    let (name_f, kind_f, value_f, labels_f, unit_f) = match fields {
        [name_f, kind_f, value_f, labels_f] => (name_f, kind_f, value_f, labels_f, None),
        [name_f, kind_f, value_f, labels_f, unit_f] => {
            (name_f, kind_f, value_f, labels_f, Some(unit_f))
        }
        _ => {
            return Err(shape(
                "metric-sample arity (want name,kind,value,labels[,unit])",
            ))
        }
    };
    let name = {
        let [nv] = a.as_form(*name_f, "name").ok_or(shape("name form"))? else {
            return Err(shape("name arity"));
        };
        read_str(a, *nv)?
    };
    let kind = {
        let [kv] = a.as_form(*kind_f, "kind").ok_or(shape("kind form"))? else {
            return Err(shape("kind arity"));
        };
        read_str(a, *kv)?
    };
    let value = {
        let [vv] = a.as_form(*value_f, "value").ok_or(shape("value form"))? else {
            return Err(shape("value arity"));
        };
        let raw = read_bytes(a, *vv)?;
        let arr: [u8; 8] = raw
            .as_slice()
            .try_into()
            .map_err(|_| shape("value not 8 f64-le bytes"))?;
        f64::from_le_bytes(arr)
    };
    let labels = {
        let label_forms = a.as_form(*labels_f, "labels").ok_or(shape("labels form"))?;
        let mut out = Vec::with_capacity(label_forms.len());
        for &lf in label_forms {
            let [key_f, val_f] = a.as_form(lf, "label").ok_or(shape("label form"))? else {
                return Err(shape("label arity (want (key)(val))"));
            };
            let [kv] = a.as_form(*key_f, "key").ok_or(shape("key form"))? else {
                return Err(shape("key arity"));
            };
            let [vv] = a.as_form(*val_f, "val").ok_or(shape("val form"))? else {
                return Err(shape("val arity"));
            };
            out.push((read_str(a, *kv)?, read_str(a, *vv)?));
        }
        out
    };
    // The optional unit form (absent = "" — the #2545 4-field shape, safe default).
    let unit = match unit_f {
        None => String::new(),
        Some(uf) => {
            let [uv] = a.as_form(*uf, "unit").ok_or(shape("unit form"))? else {
                return Err(shape("unit arity"));
            };
            read_str(a, *uv)?
        }
    };
    Ok(MetricSample {
        name,
        kind,
        value,
        unit,
        labels,
    })
}

pub fn decode_metric_publish(bytes: &[u8]) -> Result<MetricSample, EventAstError> {
    let a = codec::decode_detailed(bytes).map_err(EventAstError::Codec)?;
    read_metric_sample(&a, a.root, "metric-publish")
}

/// Decode a `metrics-publish` batch frame encoded by [`encode_metrics_publish`] → the samples in order
/// (operator #2545 review). Total, Codec-vs-Shape like the single decoder; an empty `(metrics-publish)`
/// decodes to an empty Vec (a live-but-empty batch, distinct from a decode Err).
pub fn decode_metrics_publish(bytes: &[u8]) -> Result<Vec<MetricSample>, EventAstError> {
    let a = codec::decode_detailed(bytes).map_err(EventAstError::Codec)?;
    let sample_forms = a
        .as_form(a.root, "metrics-publish")
        .ok_or(shape("metrics-publish head"))?;
    let mut out = Vec::with_capacity(sample_forms.len());
    for &sf in sample_forms {
        out.push(read_metric_sample(&a, sf, "metric-sample")?);
    }
    Ok(out)
}

/// One stage of a shell PIPELINE (the shell effect's structured-pipeline payload — operator security
/// directive, `shell` should MODEL pipes with EACH stage authorized). A stage is a `program` + its
/// `args` (a `Vec<String>`), the SAME structured `{program, args}` shape a single direct-exec command
/// splits its target into today — but explicit, so a pipeline stage is an authorized unit (not a
/// whitespace-split of one string) and metacharacters can never be re-interpreted (the CWE-78 direct-exec
/// discipline, extended to pipes: no `sh -c`, every token a literal arg). The HOST spawns each stage with
/// piped stdio and wires stdout→stdin; the KERNEL authorizes each stage's `program` as the resolved target
/// of one authz call (deny-all-if-any-denied), so the [`crate::authz::Authorize`] signature is unchanged —
/// the v0 predicate authorizer and a future Cedar-as-wasm authorizer both gate a pipeline with zero codec
/// knowledge (the per-stage program simply IS the target of each SEC-F1 gate).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ShellStage {
    pub program: String,
    pub args: Vec<String>,
}

/// A shell PIPELINE: an ORDERED list of [`ShellStage`]s whose stdout→stdin the host wires (`a | b | c`).
/// Rides the `shell` effect's PAYLOAD (unused by the bare-target single-command path today), so it is
/// ADDITIVE: a shell effect with NO payload keeps today's `target`-is-the-command behavior; a payload
/// carrying a `(shell-pipeline …)` selects the structured multi-stage path. A single-element pipeline is a
/// well-formed one-stage "pipeline" (the structured successor to a bare target); an empty pipeline is a
/// live-but-empty `(shell-pipeline)` (distinct from a decode Err — the kernel rejects it as an empty
/// command at authz/dispatch, never a panic).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ShellPipeline {
    pub stages: Vec<ShellStage>,
}

/// Build one stage's `(stage (program <str>) (args (arg <str>) …))` form. Shared by encode; args ride as an
/// ordered `(args (arg <str>) …)` list (each arg a literal string — no whitespace re-splitting, so an arg
/// with spaces survives, unlike the bare-target v0 path).
fn shell_stage_form(b: &mut Builder, s: &ShellStage) -> StructId {
    let head = b.name("stage");
    let program_form = {
        let h = b.name("program");
        let v = str_leaf(b, &s.program);
        b.list(vec![h, v])
    };
    let args_form = {
        let h = b.name("args");
        let mut items = vec![h];
        for arg in &s.args {
            let ah = b.name("arg");
            let av = str_leaf(b, arg);
            items.push(b.list(vec![ah, av]));
        }
        b.list(items)
    };
    b.list(vec![head, program_form, args_form])
}

/// Encode a [`ShellPipeline`] as canonical AST — the `shell` effect's structured-pipeline payload. Shape:
///
///   `(shell-pipeline (stage (program <str>) (args (arg <str>) …)) …)`
///
/// The reducer builds it (one stage per `program | program | …` segment); the HOST decodes it
/// ([`decode_shell_pipeline`]), spawns each stage with piped stdio, and wires stdout→stdin — no `sh -c`,
/// every arg a literal token. Same shared-codec + total-decode discipline as the other event_ast codecs.
pub fn encode_shell_pipeline(p: &ShellPipeline) -> Vec<u8> {
    let mut b = Builder::new();
    let head = b.name("shell-pipeline");
    let mut items = vec![head];
    for s in &p.stages {
        items.push(shell_stage_form(&mut b, s));
    }
    let root = b.list(items);
    codec::encode(&b.finish(root))
}

/// Read one `(stage (program <str>) (args (arg <str>) …))` form → a [`ShellStage`]. The shared body of the
/// pipeline decoder.
fn read_shell_stage(a: &Arenas, id: StructId) -> Result<ShellStage, EventAstError> {
    let [program_f, args_f] = a.as_form(id, "stage").ok_or(shape("stage head"))? else {
        return Err(shape("stage arity (want (program)(args))"));
    };
    let program = {
        let [pv] = a
            .as_form(*program_f, "program")
            .ok_or(shape("program form"))?
        else {
            return Err(shape("program arity"));
        };
        read_str(a, *pv)?
    };
    let args = {
        let arg_forms = a.as_form(*args_f, "args").ok_or(shape("args form"))?;
        let mut out = Vec::with_capacity(arg_forms.len());
        for &af in arg_forms {
            let [av] = a.as_form(af, "arg").ok_or(shape("arg form"))? else {
                return Err(shape("arg arity"));
            };
            out.push(read_str(a, *av)?);
        }
        out
    };
    Ok(ShellStage { program, args })
}

/// Decode a `shell-pipeline` frame encoded by [`encode_shell_pipeline`] → the ordered stages. Total,
/// Codec-vs-Shape like the other decoders: non-parseable bytes → [`EventAstError::Codec`]; a well-formed
/// frame with a bad head/arity / non-string field → [`EventAstError::Shape`]. An empty `(shell-pipeline)`
/// decodes to an empty stage list (a live-but-empty pipeline, distinct from a decode Err — the caller
/// rejects an empty command). Never a panic (the bytes ride the durable effect log / a guest reducer).
pub fn decode_shell_pipeline(bytes: &[u8]) -> Result<ShellPipeline, EventAstError> {
    let a = codec::decode_detailed(bytes).map_err(EventAstError::Codec)?;
    let stage_forms = a
        .as_form(a.root, "shell-pipeline")
        .ok_or(shape("shell-pipeline head"))?;
    let mut stages = Vec::with_capacity(stage_forms.len());
    for &sf in stage_forms {
        stages.push(read_shell_stage(&a, sf)?);
    }
    Ok(ShellPipeline { stages })
}

/// The RESULT of running a shell PIPELINE (operator security directive, Option A per v-ah-host): a
/// tagged union the host produces and a reducer consumes. Rides as the effect's `EffectOutcome::Ok`
/// PAYLOAD — a pipeline that RAN but whose stage exited nonzero DID perform, so it settles `Ok` with the
/// exit as DATA the reducer inspects (like a model refusal or an HTTP 500 body — a result, not a kernel
/// effect error). `EffectOutcome::Err` from the shell executor is reserved for a genuine host failure —
/// couldn't spawn the first program / a pipe-wiring OS error (nothing ran). `stdout`/`stderr` are BYTES
/// (a command's output isn't guaranteed UTF-8). `exit_code` is `i64` (a process exit code can be negative
/// — e.g. `-1` for a signal death / unknown).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ShellPipelineOutcome {
    /// Every stage exited 0 — the pipeline's final stdout.
    Ok { stdout: Vec<u8> },
    /// The FIRST stage that failed (a nonzero exit) — deny-all pipefail: no partial stdout, the failing
    /// stage's index + program + exit code + its stderr.
    Failed {
        stage_index: u64,
        program: String,
        exit_code: i64,
        stderr: Vec<u8>,
    },
}

/// Encode a [`ShellPipelineOutcome`] as canonical AST — the shell effect's `Ok` result payload for the
/// structured-pipeline path. Shapes:
///
///   `(shell-pipeline-outcome (ok (stdout <bytes>)))`
///   `(shell-pipeline-outcome (failed (stage-index <int>) (program <str>) (exit-code <int>) (stderr <bytes>)))`
///
/// The host produces it (all stages 0 → the ok arm with the final stdout; first nonzero exit → the failed
/// arm); a reducer decodes it ([`decode_shell_pipeline_outcome`]) and folds the result — inspecting the
/// exit/stderr and deciding retry/alternative as POLICY on the log (minimize-kernel-logic: the kernel never
/// classifies a tool's exit code).
pub fn encode_shell_pipeline_outcome(o: &ShellPipelineOutcome) -> Vec<u8> {
    let mut b = Builder::new();
    let head = b.name("shell-pipeline-outcome");
    let arm = match o {
        ShellPipelineOutcome::Ok { stdout } => {
            let ah = b.name("ok");
            let stdout_form = {
                let h = b.name("stdout");
                let v = bytes_leaf(&mut b, stdout);
                b.list(vec![h, v])
            };
            b.list(vec![ah, stdout_form])
        }
        ShellPipelineOutcome::Failed {
            stage_index,
            program,
            exit_code,
            stderr,
        } => {
            let ah = b.name("failed");
            let idx_form = {
                let h = b.name("stage-index");
                let v = u64_leaf(&mut b, *stage_index);
                b.list(vec![h, v])
            };
            let program_form = {
                let h = b.name("program");
                let v = str_leaf(&mut b, program);
                b.list(vec![h, v])
            };
            let exit_form = {
                let h = b.name("exit-code");
                let v = i64_leaf(&mut b, *exit_code);
                b.list(vec![h, v])
            };
            let stderr_form = {
                let h = b.name("stderr");
                let v = bytes_leaf(&mut b, stderr);
                b.list(vec![h, v])
            };
            b.list(vec![ah, idx_form, program_form, exit_form, stderr_form])
        }
    };
    let root = b.list(vec![head, arm]);
    codec::encode(&b.finish(root))
}

/// Decode a `shell-pipeline-outcome` frame encoded by [`encode_shell_pipeline_outcome`]. Total,
/// Codec-vs-Shape like the other decoders: non-parseable bytes → [`EventAstError::Codec`]; a well-formed
/// frame with a bad head/arm/arity / a wrong-typed field → [`EventAstError::Shape`]. Never a panic (the
/// bytes ride the durable effect log / a guest reducer).
pub fn decode_shell_pipeline_outcome(bytes: &[u8]) -> Result<ShellPipelineOutcome, EventAstError> {
    let a = codec::decode_detailed(bytes).map_err(EventAstError::Codec)?;
    let [arm] = a
        .as_form(a.root, "shell-pipeline-outcome")
        .ok_or(shape("shell-pipeline-outcome head"))?
    else {
        return Err(shape("shell-pipeline-outcome arity (want one arm)"));
    };
    match head_of(&a, *arm)? {
        "ok" => {
            let [stdout_f] = a.as_form(*arm, "ok").ok_or(shape("ok arm"))? else {
                return Err(shape("ok arm arity"));
            };
            let [sv] = a.as_form(*stdout_f, "stdout").ok_or(shape("stdout form"))? else {
                return Err(shape("stdout arity"));
            };
            Ok(ShellPipelineOutcome::Ok {
                stdout: read_bytes(&a, *sv)?,
            })
        }
        "failed" => {
            let [idx_f, program_f, exit_f, stderr_f] =
                a.as_form(*arm, "failed").ok_or(shape("failed arm"))?
            else {
                return Err(shape(
                    "failed arm arity (want stage-index,program,exit-code,stderr)",
                ));
            };
            let stage_index = {
                let [iv] = a
                    .as_form(*idx_f, "stage-index")
                    .ok_or(shape("stage-index form"))?
                else {
                    return Err(shape("stage-index arity"));
                };
                read_u64(&a, *iv)?
            };
            let program = {
                let [pv] = a
                    .as_form(*program_f, "program")
                    .ok_or(shape("program form"))?
                else {
                    return Err(shape("program arity"));
                };
                read_str(&a, *pv)?
            };
            let exit_code = {
                let [ev] = a
                    .as_form(*exit_f, "exit-code")
                    .ok_or(shape("exit-code form"))?
                else {
                    return Err(shape("exit-code arity"));
                };
                read_i64(&a, *ev)?
            };
            let stderr = {
                let [ev] = a.as_form(*stderr_f, "stderr").ok_or(shape("stderr form"))? else {
                    return Err(shape("stderr arity"));
                };
                read_bytes(&a, *ev)?
            };
            Ok(ShellPipelineOutcome::Failed {
                stage_index,
                program,
                exit_code,
                stderr,
            })
        }
        _ => Err(shape("unknown shell-pipeline-outcome arm (want ok|failed)")),
    }
}

/// One exported function's SIGNATURE in a component-signature descriptor (composable-component-calls
/// part-1, greenlit 2026-08-07): the func `name` + its `params` and `results` types. V0 carries each type
/// as OPAQUE `wit` BYTES (the wit/component-model type encoding, verbatim from the host's reflection) — a
/// reducer discovers the export NAME + arity (params.len()/results.len()) + the opaque per-type bytes, which
/// is enough to route/dispatch. A follow-up increment lowers those wit-type bytes to a Cadenza-decodable
/// Ast (v-metaprogramming owns the wit↔Ast type mapping); until then the reducer treats a type as an opaque
/// handle it can compare/pass-through, not introspect structurally.
/// (Decoded view — the operator "one AST" reshape r3737971653: each type is an INLINE sub-AST node in the
/// owning [`ComponentSignature`]'s [`Arenas`], NOT a separately-encoded byte leaf; a reducer walks each type
/// node IN PLACE, the same access pattern as every other Ast consumer.)
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ExportSig {
    pub name: String,
    /// Each param's type — a StructId into [`ComponentSignature::arenas`] (an inline `build_type` node).
    pub params: Vec<StructId>,
    /// Each result's type — a StructId into [`ComponentSignature::arenas`] (an inline `build_type` node).
    pub results: Vec<StructId>,
}

/// A component's SIGNATURE — its exported functions and their param/result types (the `control/signature`
/// query result), decoded as ONE walkable AST tree. The HOST reflects the target
/// ([`crate::wasm_host::component_signature_from_bytes`]) + produces the descriptor; a reducer decodes it
/// ([`decode_component_signature`]) to discover the callable surface before invoking. OWNS the decoded
/// [`Arenas`] so the per-type [`StructId`]s in each [`ExportSig`] stay valid — a reducer navigates
/// `sig.arenas` at those ids to walk each type ([`crate::ast_marshal::build_type`]'s vocab). Empty `exports`
/// = a component with no exported funcs (a live-but-empty descriptor, distinct from a decode Err). One
/// coherent tree — no per-field re-encoding (operator directive).
#[derive(Clone, Debug)]
pub struct ComponentSignature {
    /// The decoded descriptor arena — the whole `(component-signature …)` tree incl. every inline type node
    /// the [`ExportSig`] StructIds reference. A reducer walks types via this.
    pub arenas: Arenas,
    pub exports: Vec<ExportSig>,
}

// NOTE: the component-signature descriptor is ENCODED host-side by
// [`crate::wasm_host::component_signature_from_bytes`] (it needs wasmtime component reflection —
// `component_type().exports()` + each func's param/result `Type`s → `ast_marshal::build_type` inline into
// ONE arena). event_ast owns only the DECODE (the reducer-side reader below) — there's no
// `encode_component_signature(&ComponentSignature)` because there are no pre-built type-bytes to take; the
// types come from `build_type` on live wasmtime `Type`s, host-side. Wire shape (one coherent AST):
//   (component-signature (export (name <str>) (params (ty <type-ast>) …) (results (ty <type-ast>) …)) …)

/// Read one `(export (name <str>) (params (ty <type-ast>) …) (results (ty <type-ast>) …))` form → an
/// [`ExportSig`] whose param/result types are the INLINE type-node [`StructId`]s (the child of each `(ty …)`),
/// walked in place from the SAME arena — no per-type byte extraction (the operator "one tree" shape).
fn read_export_sig(a: &Arenas, id: StructId) -> Result<ExportSig, EventAstError> {
    let [name_f, params_f, results_f] = a.as_form(id, "export").ok_or(shape("export head"))? else {
        return Err(shape("export arity (want (name)(params)(results))"));
    };
    let name = {
        let [nv] = a.as_form(*name_f, "name").ok_or(shape("name form"))? else {
            return Err(shape("name arity"));
        };
        read_str(a, *nv)?
    };
    let read_ty_list =
        |list_f: StructId, head: &'static str| -> Result<Vec<StructId>, EventAstError> {
            let ty_forms = a.as_form(list_f, head).ok_or(shape("ty-list form"))?;
            let mut out = Vec::with_capacity(ty_forms.len());
            for &tf in ty_forms {
                let [ty_node] = a.as_form(tf, "ty").ok_or(shape("ty form"))? else {
                    return Err(shape("ty arity (want (ty <type-node>))"));
                };
                out.push(*ty_node);
            }
            Ok(out)
        };
    let params = read_ty_list(*params_f, "params")?;
    let results = read_ty_list(*results_f, "results")?;
    Ok(ExportSig {
        name,
        params,
        results,
    })
}

/// Decode a `component-signature` descriptor (produced by
/// [`crate::wasm_host::component_signature_from_bytes`]) → a [`ComponentSignature`] OWNING the decoded
/// [`Arenas`], each export's param/result types as inline type-node [`StructId`]s (the operator "one AST"
/// shape — walk one tree, no per-field bytes). Total, Codec-vs-Shape like the other decoders: non-parseable
/// bytes → [`EventAstError::Codec`]; a well-formed frame with a bad head/arity / non-str name →
/// [`EventAstError::Shape`]. An empty `(component-signature)` decodes to empty `exports`. Never a panic (the
/// bytes ride the durable effect log / a guest reducer).
///
/// CROSS-VOCAB HEAD INVARIANT (v-syntax audit, 2026-08-08): this decode dispatches CONTAINER-AWARE — it
/// walks from the TYPED ROOT (`as_form(root, "component-signature")`) and only then reads `export`/`name`/
/// `params`/`results` as CHILDREN of that known root. It must stay container-aware, because two of the
/// descriptor's NAME heads are SHARED with other vocabularies and disambiguated by container ALONE, not by a
/// distinct atom: `export` collides with the ML value-form's `export` head, and `name` collides with the
/// doc-form's `(name <str>)`. A head-only dispatch (classifying a naked `(export …)`/`(name …)` subtree out
/// of context) could conflate them; walking from the typed root cannot. (The type-CTOR vocabulary has no such
/// hazard — the descriptor's lowercase `Leaf::Str` ctor heads `"list"`/`"record"`/… are double-distinct from
/// the `(ty …)` payload's capitalized `Leaf::Name` heads `List`/`Record`/…, by both case and leaf-kind; the
/// lowercase name-head primitives `u32`/`string`/… don't overlap either.) Same containment contract the
/// doc-module/doc-item forms rely on, so no head rename was warranted.
pub fn decode_component_signature(bytes: &[u8]) -> Result<ComponentSignature, EventAstError> {
    let a = codec::decode_detailed(bytes).map_err(EventAstError::Codec)?;
    let export_forms = a
        .as_form(a.root, "component-signature")
        .ok_or(shape("component-signature head"))?
        .to_vec();
    let mut exports = Vec::with_capacity(export_forms.len());
    for ef in export_forms {
        exports.push(read_export_sig(&a, ef)?);
    }
    Ok(ComponentSignature { arenas: a, exports })
}

/// A decoded §4c store-snapshot frame — either a single-value `name-set` pointer or a group `member-op`.
/// This is the durable-snapshot restore discriminant: [`decode_store_frame`] decodes the shared codec
/// ONCE and routes on the frame's self-describing AST head, so [`crate::name_store::NameStore::from_snapshot_bytes`]
/// doesn't try-one-then-fall-back (which re-parses every pointer frame — github-liaison #2424 c2 perf).
#[derive(Debug, PartialEq, Eq)]
pub enum StoreFrame {
    /// A single-value pointer `set(name, hash)` (a `name-set` frame).
    NameSet { name: String, hash: Hash },
    /// A group OR-set op (a `member-op` frame): `(name, add, member, tag)`.
    MemberOp {
        name: String,
        add: bool,
        member: Hash,
        tag: (Hash, u64),
    },
}

/// Decode a store-snapshot frame with a SINGLE codec pass, routing on its AST head (`name-set` vs
/// `member-op`) — the restore-time discriminant for [`crate::name_store::NameStore::from_snapshot_bytes`].
/// Total: an unknown head / bad shape / non-AST bytes is a clean `Err` (durable-store bytes), never a panic.
pub fn decode_store_frame(bytes: &[u8]) -> Result<StoreFrame, EventAstError> {
    let a = codec::decode_detailed(bytes).map_err(EventAstError::Codec)?;
    match head_of(&a, a.root)? {
        "name-set" => {
            let [name_f, hash_f] = a
                .as_form(a.root, "name-set")
                .ok_or(shape("name-set head"))?
            else {
                return Err(shape("name-set arity"));
            };
            let [nv] = a.as_form(*name_f, "name").ok_or(shape("name form"))? else {
                return Err(shape("name arity"));
            };
            let [hv] = a.as_form(*hash_f, "hash").ok_or(shape("hash form"))? else {
                return Err(shape("hash-wrapper arity"));
            };
            Ok(StoreFrame::NameSet {
                name: read_str(&a, *nv)?,
                hash: read_hash(&a, *hv)?,
            })
        }
        "member-op" => {
            let [name_f, add_f, member_f, tag_f] = a
                .as_form(a.root, "member-op")
                .ok_or(shape("member-op head"))?
            else {
                return Err(shape("member-op arity"));
            };
            let [nv] = a.as_form(*name_f, "name").ok_or(shape("name form"))? else {
                return Err(shape("name arity"));
            };
            let [av] = a.as_form(*add_f, "add").ok_or(shape("add form"))? else {
                return Err(shape("add arity"));
            };
            let [mv] = a.as_form(*member_f, "member").ok_or(shape("member form"))? else {
                return Err(shape("member arity"));
            };
            let [ov, sv] = a.as_form(*tag_f, "tag").ok_or(shape("tag form"))? else {
                return Err(shape("tag arity"));
            };
            Ok(StoreFrame::MemberOp {
                name: read_str(&a, *nv)?,
                add: read_u64(&a, *av)? != 0,
                member: read_hash(&a, *mv)?,
                tag: (read_hash(&a, *ov)?, read_u64(&a, *sv)?),
            })
        }
        _ => Err(shape("unknown store-frame head")),
    }
}

/// Encode a `(none)` | `(some <bytes>)` optional body form (shared by the request encoders).
fn encode_opt_body(b: &mut Builder, body: Option<&[u8]>) -> StructId {
    match body {
        None => {
            let none = b.name("none");
            b.list(vec![none])
        }
        Some(bytes) => {
            let some = b.name("some");
            let v = bytes_leaf(b, bytes);
            b.list(vec![some, v])
        }
    }
}

/// Encode an ordered list of `(name value)` header pairs as `((<k> <v>) …)` — a list of 2-element string
/// lists. Order-preserving (HTTP allows repeated/ordered headers).
fn encode_header_pairs(b: &mut Builder, headers: &[(String, String)]) -> StructId {
    let mut items = Vec::with_capacity(headers.len());
    for (k, v) in headers {
        let kl = str_leaf(b, k);
        let vl = str_leaf(b, v);
        items.push(b.list(vec![kl, vl]));
    }
    b.list(items)
}

/// Decode the `((<k> <v>) …)` header-pairs list (the child of a `(headers …)` form) → ordered
/// `Vec<(String, String)>`. Each element must be a 2-element `(name value)` string list.
fn read_header_pairs(a: &Arenas, id: StructId) -> Result<Vec<(String, String)>, EventAstError> {
    let Struct::List(pairs) = a.get(id) else {
        return Err(shape("headers not a list"));
    };
    let mut out = Vec::with_capacity(pairs.len());
    for &p in pairs {
        let Struct::List(kv) = a.get(p) else {
            return Err(shape("header pair not a list"));
        };
        let [k, v] = kv.as_slice() else {
            return Err(shape("header pair arity (want (name value))"));
        };
        out.push((read_str(a, *k)?, read_str(a, *v)?));
    }
    Ok(out)
}

/// Decode the `(none)` | `(some <bytes>)` payload-opt used by [`decode_http_request`]'s body field.
fn read_opt_body(a: &Arenas, id: StructId) -> Result<Option<Vec<u8>>, EventAstError> {
    match head_of(a, id)? {
        "none" => Ok(None),
        "some" => {
            let [v] = form(a, id, "some")? else {
                return Err(shape("some arity"));
            };
            Ok(Some(read_bytes(a, *v)?))
        }
        _ => Err(shape("unknown body-opt tag")),
    }
}

// ---- encode (Event → Arenas) ------------------------------------------------------------------------

fn u64_leaf(b: &mut Builder, n: u64) -> StructId {
    b.atom_leaf(Leaf::Int {
        value: BigInt::from(n),
        radix: Radix::Dec,
    })
}

/// A SIGNED int leaf — for a value that can be negative (e.g. a process exit code, `-1` for signal death).
/// `read_i64` is the total decode (an out-of-i64 `BigInt` is a clean Shape error, never a panic).
fn i64_leaf(b: &mut Builder, n: i64) -> StructId {
    b.atom_leaf(Leaf::Int {
        value: BigInt::from(n),
        radix: Radix::Dec,
    })
}

fn bytes_leaf(b: &mut Builder, bytes: &[u8]) -> StructId {
    b.atom_leaf(Leaf::Bytes(bytes.to_vec()))
}

fn str_leaf(b: &mut Builder, s: &str) -> StructId {
    b.atom_leaf(Leaf::Str(s.to_string().into()))
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
    // The canonical family string (crate::effect::effect_ct) — one source of truth shared with the
    // executor router + Cedar action-map, so the wire name can't drift from them. Byte-identical to the
    // previous hardcoded strings (the golden round-trip test pins the bytes).
    b.name(kind.family())
}

fn outcome_form(b: &mut Builder, o: &EffectOutcome) -> StructId {
    match o {
        EffectOutcome::Ok(p) => {
            let head = b.name("ok");
            let pf = opt_payload_form(b, p);
            b.list(vec![head, pf])
        }
        EffectOutcome::Err {
            message,
            retryability,
        } => {
            let head = b.name("err");
            let m = str_leaf(b, message);
            // BACKWARD-COMPATIBLE (operator Q2): a Permanent error encodes as the BARE `(err <str>)` —
            // BYTE-IDENTICAL to a legacy pre-Q2 Err, so no durable-wire break for the common case. A
            // Retryable error appends `(retryability retryable)`; read_outcome is arity-tolerant (2 = legacy
            // Permanent, 3 = read the retryability form). A future arm just adds a retryability token.
            match retryability {
                crate::event::Retryability::Permanent => b.list(vec![head, m]),
                crate::event::Retryability::Retryable => {
                    let rh = b.name("retryability");
                    let rv = str_leaf(b, "retryable");
                    let rf = b.list(vec![rh, rv]);
                    b.list(vec![head, m, rf])
                }
            }
        }
        EffectOutcome::TimedOut => {
            let head = b.name("timed-out");
            b.list(vec![head])
        }
        // Deferred never reaches the DURABLE codec: it's a transient executor→kernel "no result yet" signal,
        // intercepted on the routed path before `record_result`, so it never becomes a logged `EffectResult`.
        EffectOutcome::Deferred => {
            unreachable!("EffectOutcome::Deferred is never logged — intercepted pre-record (userspace-effects I2)")
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

/// The reducer's optional continuation token (§19e): `(none)` | `(token <bytes>)`.
fn opt_bytes_form(b: &mut Builder, token: Option<&[u8]>) -> StructId {
    match token {
        None => {
            let head = b.name("none");
            b.list(vec![head])
        }
        Some(bytes) => {
            let head = b.name("token");
            let v = bytes_leaf(b, bytes);
            b.list(vec![head, v])
        }
    }
}

fn body_form(b: &mut Builder, body: &EventBody) -> StructId {
    match body {
        EventBody::Genesis {
            reducer,
            spawn_nonce,
            parent,
        } => {
            let head = b.name("genesis");
            let h = hash_form(b, reducer);
            let nonce = hash_form(b, spawn_nonce);
            // parent: (none) | (parent <hash>) — mirrors the opt_bytes_form none/present shape.
            let par = match parent {
                None => {
                    let none = b.name("none");
                    b.list(vec![none])
                }
                Some(p) => {
                    let ph = b.name("parent");
                    let pf = hash_form(b, p);
                    b.list(vec![ph, pf])
                }
            };
            b.list(vec![head, h, nonce, par])
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
            family,
            target,
            idempotency_key,
            deadline_ms,
            token,
        } => {
            let head = b.name("dispatched");
            let idv = u64_leaf(b, id.0);
            let k = kind_atom(b, kind);
            let fam = str_leaf(b, family);
            // Target is opaque bytes (Target=Bytes ruling) → a `Bytes` leaf, not a `Str` leaf (it may not be
            // UTF-8). This changes the Dispatched AST shape from the pre-ruling `Str` leaf.
            let t = bytes_leaf(b, target);
            let idem = hash_form(b, idempotency_key);
            let dl = opt_ms_form(b, *deadline_ms);
            let tok = opt_bytes_form(b, token.as_deref());
            // family rides AFTER kind (seq-39 identity). A pre-family log has a 6-element `dispatched`
            // form (no `fam`); the decoder tolerates both arities for backward compat.
            b.list(vec![head, idv, k, fam, t, idem, dl, tok])
        }
        EventBody::EffectResult { id, result, token } => {
            let head = b.name("effect-result");
            let idv = u64_leaf(b, id.0);
            let o = outcome_form(b, result);
            let tok = opt_bytes_form(b, token.as_deref());
            b.list(vec![head, idv, o, tok])
        }
        EventBody::TimerArmed {
            id,
            deadline_ms,
            token,
        } => {
            let head = b.name("timer-armed");
            let idv = u64_leaf(b, id.0);
            let dl = u64_leaf(b, *deadline_ms);
            let tok = opt_bytes_form(b, token.as_deref());
            b.list(vec![head, idv, dl, tok])
        }
        EventBody::TimerFired {
            id,
            fired_ms,
            token,
        } => {
            let head = b.name("timer-fired");
            let idv = u64_leaf(b, id.0);
            let fm = u64_leaf(b, *fired_ms);
            let tok = opt_bytes_form(b, token.as_deref());
            b.list(vec![head, idv, fm, tok])
        }
        EventBody::AuthzDenied { id, reason, token } => {
            let head = b.name("authz-denied");
            let idv = u64_leaf(b, id.0);
            let r = str_leaf(b, reason);
            let tok = opt_bytes_form(b, token.as_deref());
            b.list(vec![head, idv, r, tok])
        }
        EventBody::Closed { outcome } => {
            let head = b.name("closed");
            // BACKWARD-COMPATIBLE with legacy `(closed <payload>)` (fix-forward on #1938): Success encodes as
            // the BARE payload form (headed `inline`/`blob`) — byte-identical to a legacy Closed — while
            // Failure is `(failure <str>)`. Decode dispatches on the head, so an old `(closed (inline …))`
            // reads as Success unchanged, and a future arm (cancelled/escalated) just adds a new head.
            let of = match outcome {
                crate::event::CloseOutcome::Success(p) => payload_form(b, p),
                crate::event::CloseOutcome::Failure(reason) => {
                    let h = b.name("failure");
                    let r = str_leaf(b, reason);
                    b.list(vec![h, r])
                }
            };
            b.list(vec![head, of])
        }
        EventBody::FoldFailed {
            reason,
            caused_event,
        } => {
            let head = b.name("fold-failed");
            let r = str_leaf(b, reason);
            let ce = hash_form(b, caused_event);
            b.list(vec![head, r, ce])
        }
        EventBody::Terminated { by, reason } => {
            let head = b.name("terminated");
            let byf = hash_form(b, by);
            let r = str_leaf(b, reason);
            b.list(vec![head, byf, r])
        }
        EventBody::Spawned { child_hash } => {
            let head = b.name("spawned");
            let ch = hash_form(b, child_hash);
            b.list(vec![head, ch])
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

/// Read a SIGNED int leaf (companion to [`read_u64`] for values that may be negative, e.g. an exit code).
/// An out-of-i64 `BigInt` is a clean Shape error, never a panic.
fn read_i64(a: &Arenas, id: StructId) -> Result<i64, EventAstError> {
    match a.get(id) {
        Struct::Atom(leaf) => match a.leaf(*leaf) {
            Leaf::Int { value, .. } => {
                i64::try_from(value).map_err(|_| shape("int out of i64 range"))
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

/// Read an effect TARGET as bytes, tolerating BOTH leaf shapes: a `Bytes` leaf (the current shape after the
/// Target=Bytes ruling) OR a `Str` leaf (a PRE-ruling AST, where the target was marshalled as a `Str` leaf —
/// its UTF-8 bytes are the same target). This keeps a durable log / AST written before the ruling decodable
/// (backward compat), while the encoder now emits `Bytes`. A str's bytes ARE the target, so reading them as
/// bytes is lossless and identical to what the old `Str` path produced.
fn read_target_bytes(a: &Arenas, id: StructId) -> Result<Vec<u8>, EventAstError> {
    match a.get(id) {
        Struct::Atom(leaf) => match a.leaf(*leaf) {
            Leaf::Bytes(bytes) => Ok(bytes.clone()),
            Leaf::Str(s) => Ok(s.as_bytes().to_vec()),
            _ => Err(shape("expected bytes or str leaf for target")),
        },
        _ => Err(shape("expected atom for target")),
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
            Ok(Payload::Inline(read_bytes(a, *v)?.into()))
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
    let family = a.as_name(id).ok_or(shape("expected kind name"))?;
    // Route through the canonical family map (crate::effect::EffectKind::from_family) — one source of
    // truth with the encoder + router + Cedar action-map. An unknown family is a decode shape error.
    EffectKind::from_family(family).ok_or(shape("unknown effect kind"))
}

/// Decode a `(retryability <permanent|retryable>)` form → [`crate::event::Retryability`] (operator Q2).
/// An unknown token is a `Shape` error (fail-closed — don't guess retryable). Total.
fn read_retryability(
    a: &Arenas,
    id: StructId,
) -> Result<crate::event::Retryability, EventAstError> {
    let [v] = a
        .as_form(id, "retryability")
        .ok_or(shape("retryability form"))?
    else {
        return Err(shape("retryability arity"));
    };
    match a.as_str(*v) {
        Some("permanent") => Ok(crate::event::Retryability::Permanent),
        Some("retryable") => Ok(crate::event::Retryability::Retryable),
        _ => Err(shape("unknown retryability token")),
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
            // Arity-tolerant (operator Q2): `(err <str>)` = legacy/Permanent (2-form incl the head);
            // `(err <str> (retryability retryable))` = the typed retryable case. A missing retryability form
            // ⇒ Permanent (the safe default — an un-annotated error must not auto-retry, fail-closed).
            let kids = form(a, id, "err")?;
            let (m, retryability) = match kids {
                [m] => (m, crate::event::Retryability::Permanent),
                [m, rf] => (m, read_retryability(a, *rf)?),
                _ => {
                    return Err(shape(
                        "err arity (want (err <str>) or (err <str> <retryability>))",
                    ))
                }
            };
            Ok(EffectOutcome::Err {
                message: read_str(a, *m)?,
                retryability,
            })
        }
        "timed-out" => Ok(EffectOutcome::TimedOut),
        _ => Err(shape("unknown outcome tag")),
    }
}

/// Decode a `<close-outcome>` — accepts ALL THREE textual shapes that have existed across time, so no
/// persisted `Closed` fails to decode (the durable log is this codec): legacy pre-#1938 `(closed <payload>)`
/// (bare `inline`/`blob`) → `Success`; the #1938-window `(closed (success <payload>))` (wrapped) → `Success`;
/// the current (= legacy) bare form → `Success`; plus `(failure <str>)` → `Failure`. §6 supervision slice-1
/// (fix-forward on #1938's wire break). An unknown head returns [`EventAstError::Shape`] (the framing layer
/// maps that to a `Corrupt` recovery outcome; there is no distinct `Corrupt` variant here).
fn read_close_outcome(
    a: &Arenas,
    id: StructId,
) -> Result<crate::event::CloseOutcome, EventAstError> {
    match head_of(a, id)? {
        // A bare payload form (legacy pre-#1938 + the current write shape) = a successful close.
        "inline" | "blob" => Ok(crate::event::CloseOutcome::Success(read_payload(a, id)?)),
        // The #1938-WINDOW wrapped shape `(success <payload>)` — #1938 briefly WROTE this before the
        // fix-forward reverted to the bare form. Kept so a Closed persisted in that window still decodes
        // (accept-all-read, write-one — the durable log may hold any of the 3 shapes across time).
        "success" => {
            let [p] = form(a, id, "success")? else {
                return Err(shape("success arity"));
            };
            Ok(crate::event::CloseOutcome::Success(read_payload(a, *p)?))
        }
        "failure" => {
            let [m] = form(a, id, "failure")? else {
                return Err(shape("failure arity"));
            };
            Ok(crate::event::CloseOutcome::Failure(read_str(a, *m)?))
        }
        _ => Err(shape("unknown close-outcome tag")),
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

fn read_opt_bytes(a: &Arenas, id: StructId) -> Result<Option<Vec<u8>>, EventAstError> {
    match head_of(a, id)? {
        "none" => Ok(None),
        "token" => {
            let [v] = form(a, id, "token")? else {
                return Err(shape("token arity"));
            };
            Ok(Some(read_bytes(a, *v)?))
        }
        _ => Err(shape("unknown token tag")),
    }
}

fn read_body(a: &Arenas, id: StructId) -> Result<EventBody, EventAstError> {
    Ok(match head_of(a, id)? {
        "genesis" => {
            let [h, nonce, par] = form(a, id, "genesis")? else {
                return Err(shape("genesis arity"));
            };
            let parent = match head_of(a, *par)? {
                "none" => None,
                "parent" => {
                    let [pf] = form(a, *par, "parent")? else {
                        return Err(shape("genesis parent arity"));
                    };
                    Some(read_hash(a, *pf)?)
                }
                _ => return Err(shape("unknown genesis parent tag")),
            };
            EventBody::Genesis {
                reducer: read_hash(a, *h)?,
                spawn_nonce: read_hash(a, *nonce)?,
                parent,
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
                    family: read_str(a, *fam)?.into(),
                    version,
                },
                payload: read_payload(a, *pf)?,
            }
        }
        "dispatched" => {
            // Two arities for backward compat: 7 = the current `(dispatched id kind FAMILY target idem
            // deadline token)`, 6 = a pre-family log `(dispatched id kind target idem deadline token)`,
            // whose family is derived from the kind (a pre-family log only ever held built-in kinds — no
            // register-by-string/control dispatch existed before the family field, so kind→family is exact).
            match form(a, id, "dispatched")? {
                [idv, k, fam, t, idem, dl, tok] => EventBody::Dispatched {
                    id: EffectId(read_u64(a, *idv)?),
                    kind: read_kind(a, *k)?,
                    family: read_str(a, *fam)?.into(),
                    target: std::sync::Arc::from(read_target_bytes(a, *t)?.as_slice()),
                    idempotency_key: read_hash(a, *idem)?,
                    deadline_ms: read_opt_ms(a, *dl)?,
                    token: read_opt_bytes(a, *tok)?,
                },
                [idv, k, t, idem, dl, tok] => EventBody::Dispatched {
                    id: EffectId(read_u64(a, *idv)?),
                    kind: read_kind(a, *k)?,
                    family: read_kind(a, *k)?.family().into(),
                    target: std::sync::Arc::from(read_target_bytes(a, *t)?.as_slice()),
                    idempotency_key: read_hash(a, *idem)?,
                    deadline_ms: read_opt_ms(a, *dl)?,
                    token: read_opt_bytes(a, *tok)?,
                },
                _ => return Err(shape("dispatched arity")),
            }
        }
        "effect-result" => {
            let [idv, o, tok] = form(a, id, "effect-result")? else {
                return Err(shape("effect-result arity"));
            };
            EventBody::EffectResult {
                id: EffectId(read_u64(a, *idv)?),
                result: read_outcome(a, *o)?,
                token: read_opt_bytes(a, *tok)?,
            }
        }
        "timer-armed" => {
            let [idv, dl, tok] = form(a, id, "timer-armed")? else {
                return Err(shape("timer-armed arity"));
            };
            EventBody::TimerArmed {
                id: EffectId(read_u64(a, *idv)?),
                deadline_ms: read_u64(a, *dl)?,
                token: read_opt_bytes(a, *tok)?,
            }
        }
        "timer-fired" => {
            let [idv, fm, tok] = form(a, id, "timer-fired")? else {
                return Err(shape("timer-fired arity"));
            };
            EventBody::TimerFired {
                id: EffectId(read_u64(a, *idv)?),
                fired_ms: read_u64(a, *fm)?,
                token: read_opt_bytes(a, *tok)?,
            }
        }
        "authz-denied" => {
            let [idv, r, tok] = form(a, id, "authz-denied")? else {
                return Err(shape("authz-denied arity"));
            };
            EventBody::AuthzDenied {
                id: EffectId(read_u64(a, *idv)?),
                reason: read_str(a, *r)?,
                token: read_opt_bytes(a, *tok)?,
            }
        }
        "closed" => {
            let [of] = form(a, id, "closed")? else {
                return Err(shape("closed arity"));
            };
            EventBody::Closed {
                outcome: read_close_outcome(a, *of)?,
            }
        }
        "fold-failed" => {
            let [r, ce] = form(a, id, "fold-failed")? else {
                return Err(shape("fold-failed arity"));
            };
            EventBody::FoldFailed {
                reason: read_str(a, *r)?,
                caused_event: read_hash(a, *ce)?,
            }
        }
        "terminated" => {
            let [byf, r] = form(a, id, "terminated")? else {
                return Err(shape("terminated arity"));
            };
            EventBody::Terminated {
                by: read_hash(a, *byf)?,
                reason: read_str(a, *r)?,
            }
        }
        "spawned" => {
            let [ch] = form(a, id, "spawned")? else {
                return Err(shape("spawned arity"));
            };
            EventBody::Spawned {
                child_hash: read_hash(a, *ch)?,
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
    use crate::event::CloseOutcome;

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
                    target: "https://ok.host/p".as_bytes().into(),
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
                    target: "cargo test".as_bytes().into(),
                    idempotency_key: h,
                    deadline_ms: None,
                    token: None,
                },
            },
            // A dispatch whose family DIVERGES from kind.family(): kind is the `Emit` placeholder, the
            // family is `control/capabilities`. Pins that the codec preserves BOTH the kind AND the
            // separately-recorded family across a round-trip (the family field is not collapsed onto kind).
            Event {
                seq: 5,
                cause: Some(h),
                body: EventBody::Dispatched {
                    id: EffectId(9),
                    kind: EffectKind::Emit,
                    family: crate::effect::effect_ct::CAPABILITIES.into(),
                    target: "self".as_bytes().into(),
                    idempotency_key: h,
                    deadline_ms: None,
                    token: None,
                },
            },
            Event {
                seq: 6,
                cause: None,
                body: EventBody::EffectResult {
                    id: EffectId(7),
                    result: EffectOutcome::Ok(Some(Payload::Inline(b"body".to_vec().into()))),
                    token: Some(b"resume-tok".to_vec()),
                },
            },
            Event {
                seq: 7,
                cause: None,
                body: EventBody::EffectResult {
                    id: EffectId(8),
                    result: EffectOutcome::Ok(None),
                    token: None,
                },
            },
            Event {
                seq: 8,
                cause: None,
                body: EventBody::EffectResult {
                    id: EffectId(9),
                    result: EffectOutcome::err("boom"),
                    token: None,
                },
            },
            Event {
                seq: 9,
                cause: None,
                body: EventBody::EffectResult {
                    id: EffectId(9),
                    result: EffectOutcome::TimedOut,
                    token: None,
                },
            },
            Event {
                seq: 10,
                cause: None,
                body: EventBody::TimerArmed {
                    id: EffectId(10),
                    deadline_ms: 999,
                    token: Some(b"timer-tok".to_vec()),
                },
            },
            Event {
                seq: 11,
                cause: None,
                body: EventBody::TimerFired {
                    id: EffectId(10),
                    fired_ms: 1000,
                    token: Some(b"timer-tok".to_vec()),
                },
            },
            Event {
                seq: 12,
                cause: None,
                body: EventBody::AuthzDenied {
                    id: EffectId(11),
                    reason: "no capability".into(),
                    token: Some(b"denied-tok".to_vec()),
                },
            },
            Event {
                seq: 13,
                cause: None,
                body: EventBody::Closed {
                    outcome: CloseOutcome::Success(Payload::Inline(vec![].into())),
                },
            },
            Event {
                seq: 14,
                cause: None,
                body: EventBody::FoldFailed {
                    reason: "wasm reducer trapped: unreachable".to_string(),
                    caused_event: Hash::of(b"the event whose fold failed"),
                },
            },
            // Spawned PRECEDES Terminated (seq 15 < 16): per-event round-trip is order-independent, but this
            // avoids implying a spawn AFTER the terminal tail (which the "Terminated is the log tail" invariant forbids).
            Event {
                seq: 15,
                cause: Some(Hash::of(b"spawn-cause")),
                body: EventBody::Spawned {
                    child_hash: Hash::of(b"child-genesis"),
                },
            },
            Event {
                seq: 16,
                cause: Some(Hash::of(b"terminate-cause")),
                body: EventBody::Terminated {
                    by: Hash::of(b"controller-session"),
                    reason: "operator kill".to_string(),
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

    #[test]
    fn an_unknown_effect_family_decodes_to_shape_never_panics() {
        // The extensible-effects forward-compat seam (read_kind → EffectKind::from_family): a Dispatched
        // frame whose effect family is one this kernel version has NO built-in `EffectKind` variant for
        // (a future/extension effect type, or a corrupt family atom) must decode to a clean Shape error,
        // NEVER a panic — a newer peer's log entry can't brick an older reader (§17 totality). We build a
        // structurally-valid `dispatched` event AST by hand, identical to what `body_form` emits, but put
        // an unknown family name where `kind_atom` would write a canonical `effect_ct` string.
        let mut b = Builder::new();
        let head = b.name("dispatched");
        let idv = u64_leaf(&mut b, 7);
        let unknown_kind = b.name("summary"); // NOT a known effect_ct family → from_family returns None
        let target = str_leaf(&mut b, "s3://bucket/key");
        let idem = hash_form(&mut b, &Hash::of(b"idem"));
        let dl = opt_ms_form(&mut b, None);
        let tok = opt_bytes_form(&mut b, None);
        let body = b.list(vec![head, idv, unknown_kind, target, idem, dl, tok]);
        // Wrap in the top-level `(event <seq> <cause> <body>)` shape so only the family is off.
        let ev_head = b.name("event");
        let seq = u64_leaf(&mut b, 0);
        let no_cause = {
            let h = b.name("none");
            b.list(vec![h])
        };
        let root = b.list(vec![ev_head, seq, no_cause, body]);
        let bytes = codec::encode(&b.finish(root));

        // The unknown family is corruption to THIS reader (a shape error), surfaced as data — not a panic.
        match decode(&bytes) {
            Err(EventAstError::Shape(_)) => {}
            other => panic!("an unknown effect family must be a Shape error, got {other:?}"),
        }

        // Control: the SAME structure with a KNOWN family (http) decodes cleanly to that kind — proving
        // the test's frame is otherwise valid, so the failure above is the family alone, not the shape.
        let mut b2 = Builder::new();
        let head = b2.name("dispatched");
        let idv = u64_leaf(&mut b2, 7);
        let known_kind = b2.name(crate::effect::effect_ct::HTTP);
        let target = str_leaf(&mut b2, "s3://bucket/key");
        let idem = hash_form(&mut b2, &Hash::of(b"idem"));
        let dl = opt_ms_form(&mut b2, None);
        let tok = opt_bytes_form(&mut b2, None);
        let body = b2.list(vec![head, idv, known_kind, target, idem, dl, tok]);
        let ev_head = b2.name("event");
        let seq = u64_leaf(&mut b2, 0);
        let no_cause = {
            let h = b2.name("none");
            b2.list(vec![h])
        };
        let root = b2.list(vec![ev_head, seq, no_cause, body]);
        let ok_bytes = codec::encode(&b2.finish(root));
        match decode(&ok_bytes) {
            Ok(Event {
                body: EventBody::Dispatched { kind, .. },
                ..
            }) => assert_eq!(kind, EffectKind::Http),
            other => {
                panic!("the known-family control must decode to Dispatched(Http), got {other:?}")
            }
        }
    }

    #[test]
    fn a_pre_family_six_element_dispatched_frame_decodes_deriving_family_from_kind() {
        // Backward compat: a durable log written BEFORE the `family` field has a 6-element `dispatched`
        // form `(dispatched id kind target idem deadline token)`. It must still decode — deriving `family`
        // from `kind` (exact, because no register-by-string/control dispatch could exist before the field).
        // Build the legacy 6-element frame by hand (what body_form emitted pre-change).
        let mut b = Builder::new();
        let head = b.name("dispatched");
        let idv = u64_leaf(&mut b, 9);
        let kind = b.name(crate::effect::effect_ct::MODEL);
        let target = str_leaf(&mut b, "claude-x");
        let idem = hash_form(&mut b, &Hash::of(b"idem"));
        let dl = opt_ms_form(&mut b, None);
        let tok = opt_bytes_form(&mut b, None);
        let body = b.list(vec![head, idv, kind, target, idem, dl, tok]);
        let ev_head = b.name("event");
        let seq = u64_leaf(&mut b, 0);
        let no_cause = {
            let h = b.name("none");
            b.list(vec![h])
        };
        let root = b.list(vec![ev_head, seq, no_cause, body]);
        let bytes = codec::encode(&b.finish(root));

        match decode(&bytes) {
            Ok(Event {
                body: EventBody::Dispatched { kind, family, .. },
                ..
            }) => {
                assert_eq!(kind, EffectKind::Model);
                assert_eq!(
                    family.as_ref(),
                    crate::effect::effect_ct::MODEL,
                    "a pre-family frame derives family from its kind"
                );
            }
            other => panic!("legacy 6-element dispatched must decode, got {other:?}"),
        }
    }

    #[test]
    fn http_request_payload_round_trips_body_and_bodyless() {
        // The HTTP effect payload codec (shared with cdz-agent-host's HttpExecutor decode, §9b). The 2-arg
        // encoder (no headers) round-trips method + body; the 2-tuple `decode_http_request` drops headers.
        let (m, b) = decode_http_request(&encode_http_request("post", Some(b"{\"k\":1}")))
            .expect("post round-trips");
        assert_eq!(m, "post");
        assert_eq!(b.as_deref(), Some(&b"{\"k\":1}"[..]));

        let (m, b) =
            decode_http_request(&encode_http_request("get", None)).expect("get round-trips");
        assert_eq!(m, "get");
        assert_eq!(b, None);

        // An extension method name survives verbatim (Name-headed open sum).
        let (m, _) = decode_http_request(&encode_http_request("propfind", None)).unwrap();
        assert_eq!(m, "propfind");
    }

    #[test]
    fn decode_http_request_drops_headers_the_with_headers_variant_keeps_them() {
        // The additive-decoder split: the 2-tuple `decode_http_request` returns (method, body) and DROPS
        // headers; `decode_http_request_with_headers` returns the full (method, headers, body). Both decode
        // the SAME payload — the 2-tuple is just the ergonomic projection.
        let hdrs = vec![("x-trace".to_string(), "abc".to_string())];
        let payload = encode_http_request_with_headers("post", &hdrs, Some(b"{\"k\":1}"));

        let (m, b) = decode_http_request(&payload).expect("2-tuple decode");
        assert_eq!(m, "post");
        assert_eq!(b.as_deref(), Some(&b"{\"k\":1}"[..]));

        let (m2, h2, b2) = decode_http_request_with_headers(&payload).expect("3-tuple decode");
        assert_eq!(m2, "post");
        assert_eq!(h2, hdrs, "the with-headers variant keeps headers");
        assert_eq!(b2.as_deref(), Some(&b"{\"k\":1}"[..]));
    }

    #[test]
    fn http_request_with_headers_round_trips_ordered_pairs() {
        // The 3-arg with-headers encoder: method + ordered headers + body all round-trip; both the 2-arg
        // and 3-arg forms produce the SAME wire shape (a 2-arg payload decodes with empty headers).
        let hdrs = vec![
            ("content-type".to_string(), "application/json".to_string()),
            ("x-trace".to_string(), "abc".to_string()),
        ];
        let (m, h, b) = decode_http_request_with_headers(&encode_http_request_with_headers(
            "post",
            &hdrs,
            Some(b"{\"k\":1}"),
        ))
        .expect("with-headers round-trips");
        assert_eq!(m, "post");
        assert_eq!(h, hdrs, "headers round-trip in order");
        assert_eq!(b.as_deref(), Some(&b"{\"k\":1}"[..]));

        // The 2-arg encoder → empty headers via the with-headers decoder.
        let (_, h, _) =
            decode_http_request_with_headers(&encode_http_request("get", None)).unwrap();
        assert!(h.is_empty(), "2-arg encoder → empty headers");
    }

    #[test]
    fn http_response_payload_round_trips_status_headers_body() {
        let hdrs = vec![("content-length".to_string(), "3".to_string())];
        let (s, h, b) = decode_http_response(&encode_http_response(200, &hdrs, b"ok!"))
            .expect("200 round-trips");
        assert_eq!(s, 200);
        assert_eq!(h, hdrs);
        assert_eq!(&b[..], b"ok!");
        // A non-2xx status + empty headers/body round-trips too.
        let (s, h, b) = decode_http_response(&encode_http_response(404, &[], b"")).unwrap();
        assert_eq!(s, 404);
        assert!(h.is_empty());
        assert!(b.is_empty());
    }

    #[test]
    fn decode_http_request_and_response_are_total_on_garbage() {
        // Non-conforming bytes are a clean Err, never a panic (same posture as decode()).
        assert!(decode_http_request(b"not a cadenza sexpr").is_err());
        assert!(decode_http_response(b"not a cadenza sexpr").is_err());
        // A valid AST of the WRONG shape (a capabilities-manifest) is an Err for both, not a misread.
        let wrong =
            encode_capability_manifest(&crate::effect::CapabilityManifest { entries: vec![] });
        assert!(decode_http_request(&wrong).is_err());
        assert!(decode_http_response(&wrong).is_err());
    }

    #[test]
    fn name_set_payload_round_trips_name_and_hash() {
        // §4c set(name, hash) log-entry payload (non-crypto half): the full name string + the content
        // hash it points at round-trip through the shared codec.
        use crate::name_store::NameStore;
        let h = Hash::of(b"the compiler wasm bytes");
        let (name, hash) =
            decode_name_set(&encode_name_set(NameStore::COMPILER_LATEST, &h)).expect("round-trips");
        assert_eq!(name, NameStore::COMPILER_LATEST);
        assert_eq!(hash, h);

        // A scoped name with a different prefix round-trips identically (the prefix is the resolver's
        // authority concern, not the codec's — the codec is prefix-agnostic).
        let (name2, hash2) =
            decode_name_set(&encode_name_set("session/abc-123/scratch", &Hash::of(b"x"))).unwrap();
        assert_eq!(name2, "session/abc-123/scratch");
        assert_eq!(hash2, Hash::of(b"x"));
    }

    #[test]
    fn member_op_frame_round_trips_add_and_remove_with_tag() {
        // §4c session-directory I2 durable frame: a group add/remove op — name + add-flag + member hash +
        // (origin, seq) tag — round-trips through the shared codec (both add and remove arms).
        let member = Hash::of(b"member-session");
        let origin = Hash::of(b"origin-session");
        // ADD arm.
        let (n, add, m, tag) = decode_member_op(&encode_member_op(
            "session/room/lobby",
            true,
            &member,
            &(origin, 7),
        ))
        .expect("add frame round-trips");
        assert_eq!(n, "session/room/lobby");
        assert!(add, "the add flag round-trips as true");
        assert_eq!(m, member);
        assert_eq!(tag, (origin, 7));
        // REMOVE arm (add=false) round-trips identically.
        let (_n, add2, _m, tag2) = decode_member_op(&encode_member_op(
            "session/room/lobby",
            false,
            &member,
            &(origin, 7),
        ))
        .expect("remove frame round-trips");
        assert!(!add2, "the remove flag round-trips as false");
        assert_eq!(tag2, (origin, 7));
    }

    #[test]
    fn members_frame_round_trips_and_is_byte_stable_ascending() {
        // §4c I3b resolve-all result: the frozen member set round-trips through the members codec. Byte-
        // stability is a CALLER CONTRACT, not an internal guarantee: encode_members writes members in the
        // ORDER GIVEN (it does NOT sort), so the caller must feed a canonical order for the bytes to be
        // stable. The store always hands it an ascending BTreeSet, so a re-encode of the DECODED set (also
        // collected into a BTreeSet) is identical. Here we feed a BTreeSet both times to pin that canonical
        // (ascending) encoding.
        use std::collections::BTreeSet;
        let members: BTreeSet<Hash> = [Hash::of(b"A"), Hash::of(b"B"), Hash::of(b"C")]
            .into_iter()
            .collect();
        let bytes = encode_members(&members);
        let decoded = decode_members(&bytes).expect("members frame round-trips");
        assert_eq!(
            decoded.iter().copied().collect::<BTreeSet<Hash>>(),
            members,
            "the member set round-trips"
        );
        // Re-encoding the decoded set (collected back into a BTreeSet → ascending) reproduces the SAME bytes.
        let re: BTreeSet<Hash> = decoded.into_iter().collect();
        assert_eq!(
            encode_members(&re),
            bytes,
            "members encoding is byte-stable"
        );
    }

    #[test]
    fn members_frame_handles_empty_and_rejects_malformed() {
        // An empty group (all members removed) encodes to `(members)` and round-trips to an empty vec — this
        // is DISTINCT from a decode error (a live-but-empty group vs. corrupt bytes).
        let empty: [&Hash; 0] = [];
        let bytes = encode_members(empty);
        assert!(
            decode_members(&bytes)
                .expect("empty members round-trips")
                .is_empty(),
            "an empty membership decodes to an empty vec, not an error"
        );
        // The two EventAstError classes are distinguished, not conflated: non-decodable bytes → Codec;
        // a well-formed frame with the WRONG head (a member-op, not members) → Shape. Both non-panic.
        assert!(
            matches!(
                decode_members(b"not a members frame"),
                Err(EventAstError::Codec(_))
            ),
            "non-decodable bytes are a Codec error, not Shape"
        );
        let (m, o) = (Hash::of(b"m"), Hash::of(b"o"));
        assert!(
            matches!(
                decode_members(&encode_member_op("session/g", true, &m, &(o, 0))),
                Err(EventAstError::Shape(_))
            ),
            "a member-op frame is a Shape error (wrong head), not a members frame"
        );
    }

    #[test]
    fn child_exited_frame_round_trips_success_and_failure_and_rejects_malformed() {
        // §lifecycle I7: the ChildExited supervision signal (child + CloseOutcome) round-trips through the
        // canonical codec both arms — Success(payload) and Failure(reason) — so the prelude supervisor decodes
        // exactly what the host emits. CloseOutcome reuses the shared Closed-event outcome shape.
        use crate::effect::Payload;
        use crate::event::CloseOutcome;
        let child = Hash::of(b"child-session");
        // SUCCESS arm (payload rides).
        let ok = encode_child_exited(
            &child,
            &CloseOutcome::Success(Payload::Inline(b"result".to_vec().into())),
        );
        match decode_child_exited(&ok).expect("success child-exited round-trips") {
            (c, CloseOutcome::Success(Payload::Inline(p))) => {
                assert_eq!(c, child);
                assert_eq!(p.as_ref(), b"result");
            }
            other => panic!("expected Success(Inline), got {other:?}"),
        }
        // FAILURE arm (reason string rides).
        let fail = encode_child_exited(&child, &CloseOutcome::Failure("boom".to_string()));
        match decode_child_exited(&fail).expect("failure child-exited round-trips") {
            (c, CloseOutcome::Failure(reason)) => {
                assert_eq!(c, child);
                assert_eq!(reason, "boom");
            }
            other => panic!("expected Failure, got {other:?}"),
        }
        // The two EventAstError classes are distinguished, not conflated (matches the decode_child_exited
        // contract): bytes that don't parse as a codec frame → Codec; a well-formed frame with the WRONG head
        // (a members frame) → Shape. Both non-panic.
        assert!(
            matches!(
                decode_child_exited(b"not a child-exited frame"),
                Err(EventAstError::Codec(_))
            ),
            "non-decodable bytes are a Codec error, not Shape"
        );
        assert!(
            matches!(
                decode_child_exited(&encode_members([&child])),
                Err(EventAstError::Shape(_))
            ),
            "a well-formed members frame is a Shape error (wrong head), not a child-exited frame"
        );
    }

    #[test]
    fn model_request_round_trips_with_messages_tools_and_opts() {
        // §GAP-1 M1: a tool-calling model request round-trips through the codec — conversation turns with
        // MIXED content blocks (text + tool-call + tool-result, the loop-closing shape), tool defs (name +
        // opaque schema bytes), and the optional max-tokens.
        let req = ModelRequest {
            model: "anthropic.claude".to_string(),
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: vec![ContentBlock::Text("you are a build agent".to_string())],
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: vec![ContentBlock::Text("run the tests".to_string())],
                },
                // An assistant turn carrying text AND a tool-call (the id must round-trip).
                ChatMessage {
                    role: "assistant".to_string(),
                    content: vec![
                        ContentBlock::Text("running".to_string()),
                        ContentBlock::ToolCall {
                            id: "call-1".to_string(),
                            name: "shell".to_string(),
                            input: br#"{"cmd":"cargo test"}"#.to_vec(),
                        },
                    ],
                },
                // A tool turn carrying the RESULT, correlated by the same id.
                ChatMessage {
                    role: "tool".to_string(),
                    content: vec![ContentBlock::ToolResult {
                        id: "call-1".to_string(),
                        result: b"\x00\x01ok: 274 passed".to_vec(),
                    }],
                },
            ],
            tools: vec![
                ToolDef {
                    name: "shell".to_string(),
                    schema: br#"{"type":"object"}"#.to_vec(),
                },
                ToolDef {
                    name: "fs_read".to_string(),
                    schema: b"\x00\x01binary-schema".to_vec(),
                },
            ],
            max_tokens: Some(4096),
        };
        let decoded = decode_model_request(&encode_model_request(&req)).expect("round-trips");
        assert_eq!(decoded, req);
        // The tool-use id round-trips verbatim through the assistant call AND the tool result (the
        // correlation the loop depends on — v-agent-harness-host transport constraint).
        assert!(matches!(
            &decoded.messages[2].content[1],
            ContentBlock::ToolCall { id, .. } if id == "call-1"
        ));
        assert!(matches!(
            &decoded.messages[3].content[0],
            ContentBlock::ToolResult { id, .. } if id == "call-1"
        ));
    }

    #[test]
    fn model_request_handles_empty_tools_and_absent_max_tokens() {
        // The degenerate single-shot (no tools) + no opts case: additive-compatible with the opaque model
        // effect. Empty messages/tools and max_tokens=None round-trip cleanly.
        let req = ModelRequest {
            model: "m".to_string(),
            messages: vec![],
            tools: vec![],
            max_tokens: None,
        };
        let decoded = decode_model_request(&encode_model_request(&req)).expect("round-trips");
        assert_eq!(decoded, req);
        assert!(decoded.tools.is_empty());
        assert_eq!(decoded.max_tokens, None);
    }

    #[test]
    fn model_request_decode_is_total_codec_vs_shape() {
        // Distinguishes the two EventAstError classes, never panics: non-decodable bytes → Codec; a
        // well-formed frame with the wrong head (a members frame) → Shape.
        assert!(
            matches!(
                decode_model_request(b"not a model-request frame"),
                Err(EventAstError::Codec(_))
            ),
            "non-decodable bytes are a Codec error"
        );
        assert!(
            matches!(
                decode_model_request(&encode_members([&Hash::of(b"x")])),
                Err(EventAstError::Shape(_))
            ),
            "a members frame is a Shape error (wrong head), not a model-request"
        );
    }

    #[test]
    fn model_response_round_trips_tool_use_and_end_turn() {
        // §GAP-1 M2: a tool_use response (text + tool-calls the reducer dispatches) and a plain end_turn
        // response both round-trip. stop_reason is a raw string the reducer matches on.
        let tool_use = ModelResponse {
            stop_reason: "tool_use".to_string(),
            content: vec![
                ContentBlock::Text("I'll run the tests".to_string()),
                ContentBlock::ToolCall {
                    id: "call-1".to_string(),
                    name: "shell".to_string(),
                    input: br#"{"cmd":"cargo test"}"#.to_vec(),
                },
            ],
        };
        assert_eq!(
            decode_model_response(&encode_model_response(&tool_use)).expect("round-trips"),
            tool_use
        );
        let done = ModelResponse {
            stop_reason: "end_turn".to_string(),
            content: vec![ContentBlock::Text("all green".to_string())],
        };
        assert_eq!(
            decode_model_response(&encode_model_response(&done)).expect("round-trips"),
            done
        );
    }

    #[test]
    fn model_response_stop_reason_is_a_raw_string_not_a_narrowed_enum() {
        // The transport-constraint: a NOVEL/unknown stop-reason (a future Bedrock value) decodes fine — the
        // kernel carries the raw string, the reducer folds it as not-tool_use=done. No enum narrowing.
        let novel = ModelResponse {
            stop_reason: "some_future_aws_reason".to_string(),
            content: vec![ContentBlock::Text("x".to_string())],
        };
        let decoded = decode_model_response(&encode_model_response(&novel)).expect("round-trips");
        assert_eq!(decoded.stop_reason, "some_future_aws_reason");
    }

    #[test]
    fn model_response_decode_is_total_codec_vs_shape() {
        // Two EventAstError classes, never panics: non-decodable → Codec; wrong-head frame → Shape.
        assert!(
            matches!(
                decode_model_response(b"not a model-response frame"),
                Err(EventAstError::Codec(_))
            ),
            "non-decodable bytes are a Codec error"
        );
        assert!(
            matches!(
                decode_model_response(&encode_members([&Hash::of(b"x")])),
                Err(EventAstError::Shape(_))
            ),
            "a members frame is a Shape error (wrong head), not a model-response"
        );
    }

    #[test]
    fn metric_publish_round_trips_value_kind_and_labels() {
        // §operator-Q3: a metric sample round-trips — name, raw kind string, f64 value (exact via IEEE-754
        // LE bytes), and ordered (key,val) labels.
        let m = MetricSample {
            name: "agent.tool_calls".to_string(),
            kind: "counter".to_string(),
            value: 3.0,
            unit: "count".to_string(),
            labels: vec![
                ("tool".to_string(), "shell".to_string()),
                ("session".to_string(), "abc".to_string()),
            ],
        };
        assert_eq!(
            decode_metric_publish(&encode_metric_publish(&m)).expect("round-trips"),
            m
        );
        // A fractional value round-trips EXACTLY (the point of the IEEE-754-bytes encoding, no parse rounding).
        let gauge = MetricSample {
            name: "latency.p99".to_string(),
            kind: "gauge".to_string(),
            value: 12.3456789,
            unit: String::new(),
            labels: vec![],
        };
        let back = decode_metric_publish(&encode_metric_publish(&gauge)).expect("round-trips");
        assert_eq!(back.value, 12.3456789);
        assert!(back.labels.is_empty());
    }

    #[test]
    fn metric_publish_decode_is_total_codec_vs_shape() {
        // Two EventAstError classes, never panics: non-decodable → Codec; wrong-head frame → Shape.
        assert!(
            matches!(
                decode_metric_publish(b"not a metric-publish frame"),
                Err(EventAstError::Codec(_))
            ),
            "non-decodable bytes are a Codec error"
        );
        assert!(
            matches!(
                decode_metric_publish(&encode_members([&Hash::of(b"x")])),
                Err(EventAstError::Shape(_))
            ),
            "a members frame is a Shape error (wrong head), not a metric-publish"
        );
    }

    #[test]
    fn metrics_publish_batch_round_trips_and_handles_empty() {
        // operator #2545 review: publish >1 metric per effect. A batch of samples round-trips in order; each
        // element carries the same fields as a single metric-publish (kind/value/labels).
        let batch = vec![
            MetricSample {
                name: "agent.tool_calls".to_string(),
                kind: "counter".to_string(),
                value: 2.0,
                unit: "count".to_string(),
                labels: vec![("tool".to_string(), "shell".to_string())],
            },
            MetricSample {
                name: "agent.latency".to_string(),
                kind: "gauge".to_string(),
                value: 42.5,
                unit: "milliseconds".to_string(),
                labels: vec![],
            },
        ];
        assert_eq!(
            decode_metrics_publish(&encode_metrics_publish(&batch)).expect("round-trips"),
            batch
        );
        // An empty batch decodes to an empty Vec (a live-but-empty batch, not a decode Err).
        assert!(decode_metrics_publish(&encode_metrics_publish(&[]))
            .expect("empty batch round-trips")
            .is_empty());
        // Total Codec-vs-Shape (same discipline as the single decoder).
        assert!(matches!(
            decode_metrics_publish(b"not a metrics-publish frame"),
            Err(EventAstError::Codec(_))
        ));
        assert!(
            matches!(
                decode_metrics_publish(&encode_members([&Hash::of(b"x")])),
                Err(EventAstError::Shape(_))
            ),
            "a members frame is a Shape error (wrong head), not a metrics-publish batch"
        );
        // The single-sample codec still works (back-compat — #2545 landed it) + a single and a 1-batch carry
        // the same sample.
        let one = &batch[0];
        assert_eq!(
            decode_metric_publish(&encode_metric_publish(one)).unwrap(),
            *one
        );
    }

    #[test]
    fn metric_unit_round_trips_and_empty_is_byte_identical_to_the_landed_no_unit_form() {
        // operator Q3 round-3: the metric UNIT round-trips (single + batch), AND a unit-less sample encodes
        // BYTE-IDENTICALLY to the landed #2545 4-field form (no wire break — the unit is an optional appended
        // sub-form, absent = "").
        let with_unit = MetricSample {
            name: "req.size".to_string(),
            kind: "gauge".to_string(),
            value: 2048.0,
            unit: "bytes".to_string(),
            labels: vec![("route".to_string(), "/x".to_string())],
        };
        let mut no_unit = with_unit.clone();
        no_unit.unit = String::new();
        // Unit round-trips (single + batch).
        assert_eq!(
            decode_metric_publish(&encode_metric_publish(&with_unit)).unwrap(),
            with_unit
        );
        assert_eq!(
            decode_metrics_publish(&encode_metrics_publish(std::slice::from_ref(&with_unit)))
                .unwrap(),
            vec![with_unit.clone()]
        );
        // An empty unit decodes back to "" (the #2545 4-field shape's default) AND encodes to the SAME bytes
        // a pre-unit MetricSample would (unit appended only when non-empty → no wire break for unit-less).
        assert_eq!(
            decode_metric_publish(&encode_metric_publish(&no_unit))
                .unwrap()
                .unit,
            ""
        );
        // with_unit and no_unit differ ONLY by the appended unit form → their encodings differ (the unit is
        // actually carried, not dropped).
        assert_ne!(
            encode_metric_publish(&with_unit),
            encode_metric_publish(&no_unit),
            "a non-empty unit is carried on the wire; an empty one is omitted"
        );
    }

    #[test]
    fn shell_pipeline_round_trips_multi_stage_and_preserves_arg_order_and_spaces() {
        // Operator security directive: the shell effect MODELS pipes as a structured list of {program, args}
        // stages (each an authorized unit), not a whitespace-split of one target string. A multi-stage
        // pipeline round-trips with stage + arg ORDER preserved, and — unlike the bare-target v0 path — an
        // arg containing SPACES survives (it's a literal string, never re-split).
        let p = ShellPipeline {
            stages: vec![
                ShellStage {
                    program: "grep".to_string(),
                    args: vec!["-e".to_string(), "needle in haystack".to_string()],
                },
                ShellStage {
                    program: "sort".to_string(),
                    args: vec![],
                },
                ShellStage {
                    program: "head".to_string(),
                    args: vec!["-n".to_string(), "5".to_string()],
                },
            ],
        };
        let back = decode_shell_pipeline(&encode_shell_pipeline(&p)).expect("round-trips");
        assert_eq!(back, p);
        // The space-bearing arg survived as ONE arg (not split into two) — the point of the structured payload.
        assert_eq!(back.stages[0].args[1], "needle in haystack");
    }

    #[test]
    fn shell_pipeline_handles_single_stage_and_empty() {
        // A single-stage pipeline is well-formed (the structured successor to a bare target).
        let one = ShellPipeline {
            stages: vec![ShellStage {
                program: "echo".to_string(),
                args: vec!["ok".to_string()],
            }],
        };
        assert_eq!(
            decode_shell_pipeline(&encode_shell_pipeline(&one)).unwrap(),
            one
        );
        // An empty pipeline decodes to an empty stage list (a live-but-empty `(shell-pipeline)`, distinct from
        // a decode Err — the CALLER rejects an empty command; the codec is faithful).
        let empty = ShellPipeline { stages: vec![] };
        assert!(decode_shell_pipeline(&encode_shell_pipeline(&empty))
            .unwrap()
            .stages
            .is_empty());
    }

    #[test]
    fn shell_pipeline_decode_is_total_codec_vs_shape() {
        // Two EventAstError classes, never panics (the payload rides the durable effect log / a guest reducer):
        // non-decodable bytes → Codec; a well-formed frame with the WRONG head → Shape.
        assert!(
            matches!(
                decode_shell_pipeline(b"not a shell-pipeline frame"),
                Err(EventAstError::Codec(_))
            ),
            "non-decodable bytes are a Codec error"
        );
        assert!(
            matches!(
                decode_shell_pipeline(&encode_members([&Hash::of(b"x")])),
                Err(EventAstError::Shape(_))
            ),
            "a members frame is a Shape error (wrong head), not a shell-pipeline"
        );
    }

    #[test]
    fn shell_pipeline_outcome_round_trips_both_arms_incl_negative_exit_and_non_utf8() {
        // Option A (v-ah-host): the pipeline result rides as an Ok-payload tagged union. Both arms round-trip.
        // The ok arm carries the final stdout (as bytes — not guaranteed UTF-8).
        let ok = ShellPipelineOutcome::Ok {
            stdout: vec![0xff, 0xfe, b'h', b'i'], // deliberately non-UTF-8 bytes survive
        };
        assert_eq!(
            decode_shell_pipeline_outcome(&encode_shell_pipeline_outcome(&ok)).unwrap(),
            ok
        );
        // The failed arm carries the first failing stage's index + program + exit code + stderr. A NEGATIVE
        // exit code (e.g. -1 for a signal death / unknown) round-trips (i64, not u64).
        let failed = ShellPipelineOutcome::Failed {
            stage_index: 2,
            program: "head".to_string(),
            exit_code: -1,
            stderr: vec![b'b', b'o', b'o', b'm', 0x00],
        };
        assert_eq!(
            decode_shell_pipeline_outcome(&encode_shell_pipeline_outcome(&failed)).unwrap(),
            failed
        );
    }

    #[test]
    fn shell_pipeline_outcome_decode_is_total_codec_vs_shape() {
        // Never panics: non-decodable → Codec; a well-formed frame with the WRONG head → Shape.
        assert!(matches!(
            decode_shell_pipeline_outcome(b"not an outcome frame"),
            Err(EventAstError::Codec(_))
        ));
        assert!(
            matches!(
                decode_shell_pipeline_outcome(&encode_members([&Hash::of(b"x")])),
                Err(EventAstError::Shape(_))
            ),
            "a members frame is a Shape error (wrong head), not a shell-pipeline-outcome"
        );
    }

    // Build a `(component-signature (export (name)(params (ty <type-node>)…)(results (ty <type-node>)…))…)`
    // descriptor AST directly (the shape the HOST's component_signature_from_bytes emits) — for decode tests
    // WITHOUT wasmtime. Each `type_node` is a hand-built marker matching build_type's vocab ((u32)/(string)/…).
    #[cfg(test)]
    fn build_test_descriptor(exports: &[(&str, &[&str], &[&str])]) -> Vec<u8> {
        let mut b = Builder::new();
        let ty_form = |b: &mut Builder, marker: &str| {
            let h = b.name("ty");
            // build_type emits a PRIMITIVE type as a headed form `(u32)`/`(string)` (a list, head = the
            // marker), NOT a bare name atom — so wrap it as a list to match (and be head_name-walkable).
            let marker_head = b.name(marker);
            let node = b.list(vec![marker_head]);
            b.list(vec![h, node])
        };
        let head = b.name("component-signature");
        let mut items = vec![head];
        for (name, params, results) in exports {
            let export_head = b.name("export");
            let name_form = {
                let h = b.name("name");
                let v = str_leaf(&mut b, name);
                b.list(vec![h, v])
            };
            let params_form = {
                let h = b.name("params");
                let mut ps = vec![h];
                for m in *params {
                    ps.push(ty_form(&mut b, m));
                }
                b.list(ps)
            };
            let results_form = {
                let h = b.name("results");
                let mut rs = vec![h];
                for m in *results {
                    rs.push(ty_form(&mut b, m));
                }
                b.list(rs)
            };
            items.push(b.list(vec![export_head, name_form, params_form, results_form]));
        }
        let root = b.list(items);
        codec::encode(&b.finish(root))
    }

    #[test]
    fn component_signature_decodes_one_tree_with_inline_type_nodes_walked_in_place() {
        // Operator "one AST" shape: decode reads names + arities + INLINE type nodes (StructIds into the
        // owned arena) — a reducer WALKS each type in place (not per-type bytes). Multi-export incl. a 0-arg.
        let bytes = build_test_descriptor(&[
            ("parse", &["string"], &["u32"]),
            ("noop", &[], &[]),
            ("combine", &["u8", "u32"], &["bool"]),
        ]);
        let sig = decode_component_signature(&bytes).expect("decodes the one-tree descriptor");
        // Names + arities are readable for routing.
        assert_eq!(sig.exports.len(), 3);
        assert_eq!(sig.exports[0].name, "parse");
        assert_eq!(sig.exports[0].params.len(), 1);
        assert_eq!(sig.exports[1].params.len(), 0); // noop is 0-arg
        assert_eq!(sig.exports[1].results.len(), 0);
        assert_eq!(sig.exports[2].params.len(), 2);
        // The type is an INLINE node in sig.arenas — walk it IN PLACE (its head is the build_type marker),
        // NOT a re-decoded byte blob. This is the "walk one tree" the operator wants.
        assert_eq!(
            sig.arenas.head_name(sig.exports[0].params[0]),
            Some("string")
        );
        assert_eq!(sig.arenas.head_name(sig.exports[0].results[0]), Some("u32"));
        assert_eq!(sig.arenas.head_name(sig.exports[2].params[1]), Some("u32"));
        assert_eq!(
            sig.arenas.head_name(sig.exports[2].results[0]),
            Some("bool")
        );
    }

    #[test]
    fn component_signature_handles_empty_and_decode_is_total() {
        // An empty (component-signature) = a component with no exported funcs (live-but-empty, not a decode Err).
        let empty = build_test_descriptor(&[]);
        assert!(decode_component_signature(&empty)
            .unwrap()
            .exports
            .is_empty());
        // Total Codec-vs-Shape, never panics: non-decodable → Codec; wrong-head frame → Shape.
        assert!(matches!(
            decode_component_signature(b"not a component-signature frame"),
            Err(EventAstError::Codec(_))
        ));
        assert!(
            matches!(
                decode_component_signature(&encode_members([&Hash::of(b"x")])),
                Err(EventAstError::Shape(_))
            ),
            "a members frame is a Shape error (wrong head), not a component-signature"
        );
    }

    #[test]
    fn component_signature_descriptor_round_trips_byte_identically_through_the_codec() {
        // GATE (v-syntax AST-codec review, offer 1): the (component-signature …) descriptor is a FROZEN
        // wire the host emits + reducers decode, so it must survive the codec BIJECTION —
        // encode→decode→re-encode BYTE-IDENTICAL — or a future codec/AST change could silently reshape it
        // under a consumer. Pins the frozen-bijection invariant (same discipline as the doc-module/dict
        // round-trip gates). Covers a multi-export descriptor with the full head-kind split: a STR-head
        // compound-shaped marker is exercised elsewhere; here primitives (name-head) across arities incl. a
        // 0-arg export, which is the shape build_type emits for the common cases.
        let bytes = build_test_descriptor(&[
            ("parse", &["string"], &["u32"]),
            ("noop", &[], &[]),
            ("combine", &["u8", "u32"], &["bool"]),
        ]);
        // Decode the descriptor to its AST arena, then RE-ENCODE that arena: the bytes must be identical
        // (the codec is a bijection over a well-formed tree — no drift, no reordering, no leaf reshape).
        let a = codec::decode_detailed(&bytes).expect("descriptor decodes to an arena");
        let reencoded = codec::encode(&a);
        assert_eq!(
            reencoded, bytes,
            "the component-signature descriptor must re-encode byte-identically (frozen codec bijection)"
        );
        // And a structural re-decode through the descriptor surface yields the same exports/arities/type
        // heads — the sexpr-level read survives the round-trip, not just the raw bytes.
        let sig1 = decode_component_signature(&bytes).expect("first decode");
        let sig2 = decode_component_signature(&reencoded).expect("decode of the re-encoded bytes");
        assert_eq!(sig1.exports.len(), sig2.exports.len());
        for (e1, e2) in sig1.exports.iter().zip(sig2.exports.iter()) {
            assert_eq!(e1.name, e2.name);
            assert_eq!(e1.params.len(), e2.params.len());
            assert_eq!(e1.results.len(), e2.results.len());
            // The inline type-node HEADS match position-for-position (the walkable surface is stable).
            for (p1, p2) in e1.params.iter().zip(e2.params.iter()) {
                assert_eq!(sig1.arenas.head_name(*p1), sig2.arenas.head_name(*p2));
            }
            for (r1, r2) in e1.results.iter().zip(e2.results.iter()) {
                assert_eq!(sig1.arenas.head_name(*r1), sig2.arenas.head_name(*r2));
            }
        }
    }

    #[test]
    fn component_signature_decode_is_container_aware_rejecting_shared_heads_at_the_root() {
        // CROSS-VOCAB HEAD INVARIANT (v-syntax audit): the descriptor reuses the NAME heads `export` and
        // `name`, which are SHARED with the ML value-form (`export`) and doc-form (`name`) vocabularies and
        // disambiguated by CONTAINER only. decode_component_signature must therefore dispatch from the TYPED
        // ROOT (`component-signature`), never head-only — so a naked `(export …)` or `(name …)` frame
        // presented AS the root is a Shape reject, NOT mis-decoded as a signature. This test ENFORCES the
        // container-aware contract the doc comment documents: a future refactor that made decode head-only
        // (or accepted a bare export/name root) would flip these to a wrong-decode and fail here.
        let mut b = Builder::new();
        let h = b.name("export");
        let inner = str_leaf(&mut b, "inc");
        let root = b.list(vec![h, inner]);
        let bare_export = codec::encode(&b.finish(root));
        assert!(
            matches!(
                decode_component_signature(&bare_export),
                Err(EventAstError::Shape(_))
            ),
            "a bare (export …) at the root must be a Shape reject — decode keys on the component-signature \
             root, not the shared `export` head"
        );

        let mut b2 = Builder::new();
        let h2 = b2.name("name");
        let v2 = str_leaf(&mut b2, "inc");
        let root2 = b2.list(vec![h2, v2]);
        let bare_name = codec::encode(&b2.finish(root2));
        assert!(
            matches!(
                decode_component_signature(&bare_name),
                Err(EventAstError::Shape(_))
            ),
            "a bare (name <str>) at the root (shared with doc-form) must be a Shape reject, not conflated"
        );
    }

    #[test]
    fn decode_store_frame_routes_both_kinds_in_one_pass_and_rejects_unknown() {
        // The single-pass snapshot-restore discriminant (#2424 c2): one decode routes on the AST head,
        // no try-one-then-fall-back double-decode.
        let h = Hash::of(b"ptr");
        match decode_store_frame(&encode_name_set("system/x", &h)).expect("name-set routes") {
            StoreFrame::NameSet { name, hash } => {
                assert_eq!(name, "system/x");
                assert_eq!(hash, h);
            }
            other => panic!("expected NameSet, got {other:?}"),
        }
        let (m, o) = (Hash::of(b"member"), Hash::of(b"origin"));
        match decode_store_frame(&encode_member_op("session/g/room", true, &m, &(o, 3)))
            .expect("member-op routes")
        {
            StoreFrame::MemberOp {
                name,
                add,
                member,
                tag,
            } => {
                assert_eq!(name, "session/g/room");
                assert!(add);
                assert_eq!(member, m);
                assert_eq!(tag, (o, 3));
            }
            other => panic!("expected MemberOp, got {other:?}"),
        }
        // An unknown-head frame (a valid AST that's neither) is a clean Err, not a panic / mis-route.
        assert!(decode_store_frame(&encode(&all_variants()[0])).is_err());
        assert!(decode_store_frame(b"garbage").is_err());
    }

    #[test]
    fn malformed_member_op_frame_is_a_clean_error_not_a_panic() {
        // Totality: a truncated/garbage member-op frame is a clean Shape/Codec error (durable-log bytes).
        assert!(decode_member_op(b"not a valid frame at all").is_err());
        // A valid name-set frame is NOT a member-op (wrong head) → clean error, no cross-decode.
        let name_set = encode_name_set("session/room/lobby", &Hash::of(b"x"));
        assert!(
            decode_member_op(&name_set).is_err(),
            "a name-set frame must not decode as a member-op"
        );
    }

    #[test]
    fn closed_round_trips_both_close_outcome_arms_through_the_shared_codec() {
        // §6 supervision slice-1: the structured close outcome (Success-with-payload vs Failure-with-reason)
        // survives the durable shared codec — a supervisor reading the log recovers success-vs-failure + the
        // failure reason. (`every_variant_round_trips` covers the Success arm; this pins BOTH, esp. that the
        // Failure reason string round-trips — the bit a supervisor keys restart/escalate on.)
        let success = Event {
            seq: 1,
            cause: None,
            body: EventBody::Closed {
                outcome: CloseOutcome::Success(Payload::Inline(b"result-bytes".to_vec().into())),
            },
        };
        let failure = Event {
            seq: 2,
            cause: None,
            body: EventBody::Closed {
                outcome: CloseOutcome::Failure("goal unreachable: budget exhausted".to_string()),
            },
        };
        assert_eq!(decode(&encode(&success)).unwrap(), success);
        assert_eq!(decode(&encode(&failure)).unwrap(), failure);
        // The two arms are DISTINGUISHABLE after a round-trip (not collapsed to one opaque shape).
        assert_ne!(
            encode(&success),
            encode(&failure),
            "Success and Failure encode distinctly — a supervisor can tell them apart"
        );

        // FROZEN-WIRE COMPAT (fix-forward on #1938): a LEGACY `(closed <payload>)` sexpr (a bare payload form
        // under the `closed` head, as written before the CloseOutcome change) must decode as Success — Success
        // carries NO wrapper `success` head, it IS the bare payload. Build the legacy shape by hand + prove it.
        let legacy_bytes = {
            let mut b = Builder::new();
            let head = b.name("event");
            let seq = u64_leaf(&mut b, 3);
            let cause = {
                let n = b.name("none");
                b.list(vec![n])
            };
            let closed_head = b.name("closed");
            let pf = payload_form(&mut b, &Payload::Inline(b"legacy".to_vec().into()));
            let body = b.list(vec![closed_head, pf]);
            let root = b.list(vec![head, seq, cause, body]);
            codec::encode(&b.finish(root))
        };
        assert_eq!(
            decode(&legacy_bytes)
                .expect("legacy (closed <payload>) still decodes")
                .body,
            EventBody::Closed {
                outcome: CloseOutcome::Success(Payload::Inline(b"legacy".to_vec().into())),
            },
            "a legacy (closed (inline …)) decodes as Success (backward-compatible textual codec)"
        );

        // #1938-WINDOW shape (liaison pr1947): a `(closed (success <payload>))` sexpr — the wrapped form
        // #1938 briefly WROTE (and #1947's first cut dropped from the decoder) — must ALSO decode as Success,
        // else a Closed persisted in that window fails. Proves all 3 textual shapes are accepted on read.
        let window_bytes = {
            let mut b = Builder::new();
            let head = b.name("event");
            let seq = u64_leaf(&mut b, 4);
            let cause = {
                let n = b.name("none");
                b.list(vec![n])
            };
            let closed_head = b.name("closed");
            let success_head = b.name("success");
            let pf = payload_form(&mut b, &Payload::Inline(b"window".to_vec().into()));
            let success = b.list(vec![success_head, pf]);
            let body = b.list(vec![closed_head, success]);
            let root = b.list(vec![head, seq, cause, body]);
            codec::encode(&b.finish(root))
        };
        assert_eq!(
            decode(&window_bytes)
                .expect("#1938-window (closed (success …)) still decodes")
                .body,
            EventBody::Closed {
                outcome: CloseOutcome::Success(Payload::Inline(b"window".to_vec().into())),
            },
            "a #1938-window (closed (success …)) decodes as Success (all 3 shapes accepted)"
        );
    }

    #[test]
    fn effect_result_err_round_trips_retryability_and_is_legacy_backward_compatible() {
        // operator Q2: a typed EffectOutcome::Err retryability survives the durable shared codec both arms,
        // AND a legacy `(err <str>)` (pre-Q2, no retryability form) decodes as Permanent (fail-closed default).
        use crate::effect::EffectId;
        use crate::event::Retryability;
        let permanent = Event {
            seq: 1,
            cause: None,
            body: EventBody::EffectResult {
                id: EffectId(7),
                result: EffectOutcome::err("malformed request"),
                token: None,
            },
        };
        let retryable = Event {
            seq: 2,
            cause: None,
            body: EventBody::EffectResult {
                id: EffectId(8),
                result: EffectOutcome::err_retryable("bedrock throttled"),
                token: None,
            },
        };
        assert_eq!(decode(&encode(&permanent)).unwrap(), permanent);
        assert_eq!(decode(&encode(&retryable)).unwrap(), retryable);
        // The two retryabilities are DISTINGUISHABLE after a round-trip (a reducer folds on them structurally).
        assert_ne!(encode(&permanent), encode(&retryable));
        // Permanent encodes BYTE-IDENTICALLY to a legacy `(err <str>)` — no durable-wire break (the point of
        // the additive optional retryability form): build the legacy bare-`(err <str>)` body by hand + prove
        // it decodes as Err{Permanent}.
        let legacy_bytes = {
            let mut b = Builder::new();
            let head = b.name("event");
            let seq = u64_leaf(&mut b, 3);
            let cause = {
                let n = b.name("none");
                b.list(vec![n])
            };
            let er_head = b.name("effect-result");
            let idv = u64_leaf(&mut b, 9);
            let outcome = {
                let eh = b.name("err");
                let m = str_leaf(&mut b, "legacy permanent");
                b.list(vec![eh, m]) // BARE (err <str>) — the pre-Q2 shape
            };
            let tok = {
                let n = b.name("none");
                b.list(vec![n])
            };
            let body = b.list(vec![er_head, idv, outcome, tok]);
            let root = b.list(vec![head, seq, cause, body]);
            codec::encode(&b.finish(root))
        };
        match decode(&legacy_bytes)
            .expect("legacy (err <str>) still decodes")
            .body
        {
            EventBody::EffectResult {
                result:
                    EffectOutcome::Err {
                        message,
                        retryability,
                    },
                ..
            } => {
                assert_eq!(message, "legacy permanent");
                assert_eq!(
                    retryability,
                    Retryability::Permanent,
                    "a legacy un-annotated (err <str>) decodes as Permanent (safe default)"
                );
            }
            other => panic!("expected EffectResult Err, got {other:?}"),
        }
    }

    #[test]
    fn decode_name_set_is_total_on_garbage_and_wrong_shape() {
        // Non-conforming bytes are a clean Err, never a panic (durable-log / untrusted-store posture).
        assert!(decode_name_set(b"not a cadenza sexpr").is_err());
        // A valid AST of the WRONG shape (an http-response) is an Err, not a misread.
        let wrong = encode_http_response(200, &[], b"");
        assert!(decode_name_set(&wrong).is_err());
    }

    #[test]
    fn capability_manifest_encodes_to_the_agreed_sexpr_shape() {
        use crate::effect::{CapabilityEntry, CapabilityManifest, GrantState, ResourcePredicate};
        // A manifest with all three grant-states + a scoped predicate — encode, then DECODE the bytes as a
        // Cadenza AST and assert the agreed shape (host-capability-discovery I4 encode, design-locked):
        // (capabilities-manifest 1 (entries (entry <fam> <grant> <scope>) ...)).
        let manifest = CapabilityManifest {
            entries: vec![
                CapabilityEntry {
                    family: "http".into(),
                    grant: GrantState::Granted,
                    scope: Some(ResourcePredicate::HostIn(vec!["ok.host".into()])),
                },
                CapabilityEntry {
                    family: "model".into(),
                    grant: GrantState::Denied,
                    scope: None,
                },
                CapabilityEntry {
                    family: "shell".into(),
                    grant: GrantState::Absent,
                    scope: None,
                },
            ],
        };
        let bytes = encode_capability_manifest(&manifest);
        let a = codec::decode_detailed(&bytes).expect("manifest payload decodes as a Cadenza AST");

        // Root: (capabilities-manifest <version> <entries>)
        let root = a
            .as_form(a.root, "capabilities-manifest")
            .expect("root head");
        assert_eq!(
            root.len(),
            2,
            "capabilities-manifest has <version> + <entries>"
        );
        // version leaf == 1
        match a.get(root[0]) {
            Struct::Atom(l) => match a.leaf(*l) {
                Leaf::Int { value, .. } => assert_eq!(*value, BigInt::from(1u32)),
                other => panic!("version must be an Int leaf, got {other:?}"),
            },
            other => panic!("version must be an atom, got {other:?}"),
        }
        // (entries (entry ...) x3), in manifest order.
        let entries = a.as_form(root[1], "entries").expect("entries head");
        assert_eq!(entries.len(), 3, "one entry per family");
        // Entry 0: (entry "http" (granted) (some (host-in "ok.host")))
        let e0 = a.as_form(entries[0], "entry").expect("entry head");
        assert_eq!(a.as_str(e0[0]), Some("http"));
        assert!(a.as_form(e0[1], "granted").is_some(), "grant is (granted)");
        let some = a.as_form(e0[2], "some").expect("scope (some ..)");
        let host_in = a.as_form(some[0], "host-in").expect("(host-in ..)");
        assert_eq!(a.as_str(host_in[0]), Some("ok.host"));
        // Entry 1: (entry "model" (denied) (none))
        let e1 = a.as_form(entries[1], "entry").unwrap();
        assert_eq!(a.as_str(e1[0]), Some("model"));
        assert!(a.as_form(e1[1], "denied").is_some());
        assert!(a.as_form(e1[2], "none").is_some(), "denied → (none) scope");
        // Entry 2: (entry "shell" (absent) (none))
        let e2 = a.as_form(entries[2], "entry").unwrap();
        assert_eq!(a.as_str(e2[0]), Some("shell"));
        assert!(a.as_form(e2[1], "absent").is_some());
        assert!(a.as_form(e2[2], "none").is_some());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn projection_output_encodes_and_round_trips_the_real_grant_states() {
        // The I4 payload PIPELINE end-to-end: feed a manifest produced by `project_manifest` (not a
        // hand-built one) through `encode_capability_manifest`, decode the bytes, and assert the
        // projection's ACTUAL grant-states survive. The isolated tests cover each half — projection
        // grant-state logic (effect.rs) and encode shape from a literal manifest (above) — but nothing
        // pins the SEAM: that `GrantState`/`ResourcePredicate` as the projection emits them map onto the
        // encoder's `(granted)|(denied)|(absent)` arms. This is exactly the path I4b's inline-answer arm
        // wires (project_manifest → encode → EffectResult), so a drift on either side must fail here.
        use crate::authz::Authorizer;
        use crate::effect::{
            effect_ct, project_manifest, Capability, EffectKind, ResourcePredicate,
        };

        // Policy grants Http only; mechanism serves Http + Model but not Shell → one of each grant-state.
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Http,
            predicate: ResourcePredicate::Any,
        }]);
        let handles = |f: &str| f == effect_ct::HTTP || f == effect_ct::MODEL;
        let families = [effect_ct::HTTP, effect_ct::MODEL, effect_ct::SHELL];
        let manifest = project_manifest(&families, handles, &authz, |_| "probe://scope").await;

        let bytes = encode_capability_manifest(&manifest);
        let a =
            codec::decode_detailed(&bytes).expect("projected manifest payload decodes as an AST");

        let root = a
            .as_form(a.root, "capabilities-manifest")
            .expect("root head");
        let entries = a.as_form(root[1], "entries").expect("entries head");
        assert_eq!(
            entries.len(),
            3,
            "one entry per probed family, projection order"
        );

        // http → GRANTED (mechanism yes + policy allows); model → DENIED (mechanism yes + policy denies);
        // shell → ABSENT (mechanism no). project_manifest leaves scope None → every entry encodes (none).
        let expect = [
            (effect_ct::HTTP, "granted"),
            (effect_ct::MODEL, "denied"),
            (effect_ct::SHELL, "absent"),
        ];
        for (i, (fam, grant)) in expect.iter().enumerate() {
            let e = a.as_form(entries[i], "entry").expect("entry head");
            assert_eq!(a.as_str(e[0]), Some(*fam), "family at index {i}");
            assert!(
                a.as_form(e[1], grant).is_some(),
                "family {fam} must encode as ({grant})"
            );
            assert!(
                a.as_form(e[2], "none").is_some(),
                "projection emits no scope → (none)"
            );
        }
    }

    #[test]
    fn scope_predicate_encodes_every_resource_predicate_variant() {
        use crate::effect::{CapabilityEntry, CapabilityManifest, GrantState, ResourcePredicate};
        // The manifest payload is a DURABLE wire contract the guest decodes; the `<scope>` predicate has
        // SIX arms (Any/Exact/OneOf/HostIn/Prefix + I6 DescendantOf) but the shape test above exercises only
        // HostIn. A typo in any tag (`one-of`→`oneof`)
        // or a swapped payload arity would ship silently. Encode one entry per ResourcePredicate variant
        // and assert each `(some <predicate>)` head + payload decodes as designed. One entry per arm keeps
        // the family a don't-care label; grant is fixed Granted.
        let pred_entry = |family: &str, pred: ResourcePredicate| CapabilityEntry {
            family: family.into(),
            grant: GrantState::Granted,
            scope: Some(pred),
        };
        let manifest = CapabilityManifest {
            entries: vec![
                pred_entry("f-any", ResourcePredicate::Any),
                pred_entry("f-exact", ResourcePredicate::Exact("t".into())),
                pred_entry(
                    "f-one-of",
                    ResourcePredicate::OneOf(vec!["a".into(), "b".into()]),
                ),
                pred_entry(
                    "f-host-in",
                    ResourcePredicate::HostIn(vec!["h.host".into()]),
                ),
                pred_entry("f-prefix", ResourcePredicate::Prefix("pre/".into())),
                pred_entry(
                    "f-descendant-of",
                    ResourcePredicate::DescendantOf(crate::hash::Hash::of(b"controller")),
                ),
            ],
        };
        let bytes = encode_capability_manifest(&manifest);
        let a = codec::decode_detailed(&bytes).expect("manifest decodes as an AST");
        let root = a
            .as_form(a.root, "capabilities-manifest")
            .expect("root head");
        let entries = a.as_form(root[1], "entries").expect("entries head");
        assert_eq!(entries.len(), 6, "one entry per predicate arm");

        // Pull the `(some <predicate>)` predicate form for entry i, asserting the (some ..) wrapper carries
        // EXACTLY one child. Pinning the arity (not just the head) means an encoder that accidentally
        // appends a field to (some ...) fails here rather than passing silently — the point of a
        // wire-contract pin (PR #1653 review).
        let pred_of = |i: usize| {
            let e = a.as_form(entries[i], "entry").expect("entry head");
            let some = a.as_form(e[2], "some").expect("scope must be (some ..)");
            assert_eq!(some.len(), 1, "(some ..) wraps exactly one predicate");
            some[0]
        };

        // (any) — nullary head.
        assert!(a.as_form(pred_of(0), "any").is_some(), "(any)");
        // (exact <str>)
        let exact = a.as_form(pred_of(1), "exact").expect("(exact ..)");
        assert_eq!(a.as_str(exact[0]), Some("t"));
        // (one-of <str>…) — order-preserving list.
        let one_of = a.as_form(pred_of(2), "one-of").expect("(one-of ..)");
        assert_eq!(one_of.len(), 2, "one-of carries every element");
        assert_eq!(a.as_str(one_of[0]), Some("a"));
        assert_eq!(a.as_str(one_of[1]), Some("b"));
        // (host-in <str>…)
        let host_in = a.as_form(pred_of(3), "host-in").expect("(host-in ..)");
        assert_eq!(a.as_str(host_in[0]), Some("h.host"));
        // (prefix <str>)
        let prefix = a.as_form(pred_of(4), "prefix").expect("(prefix ..)");
        assert_eq!(a.as_str(prefix[0]), Some("pre/"));
        // (descendant-of <controller-hex>) — I6 supervision marker; the controller rides as its hex string.
        let desc = a
            .as_form(pred_of(5), "descendant-of")
            .expect("(descendant-of ..)");
        assert_eq!(
            a.as_str(desc[0]),
            Some(crate::hash::Hash::of(b"controller").to_hex().as_str())
        );
    }
}
