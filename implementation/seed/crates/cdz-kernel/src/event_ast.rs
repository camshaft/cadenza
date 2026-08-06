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
///     `(one-of <str>…)` | `(host-in <str>…)` | `(prefix <str>)` — mirrors [`crate::effect::ResourcePredicate`].
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
            let t = str_leaf(b, target);
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
                    target: read_str(a, *t)?.into(),
                    idempotency_key: read_hash(a, *idem)?,
                    deadline_ms: read_opt_ms(a, *dl)?,
                    token: read_opt_bytes(a, *tok)?,
                },
                [idv, k, t, idem, dl, tok] => EventBody::Dispatched {
                    id: EffectId(read_u64(a, *idv)?),
                    kind: read_kind(a, *k)?,
                    family: read_kind(a, *k)?.family().into(),
                    target: read_str(a, *t)?.into(),
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
                    target: "self".into(),
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
                    result: EffectOutcome::Err("boom".into()),
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
        // five arms but the shape test above exercises only HostIn. A typo in any tag (`one-of`→`oneof`)
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
            ],
        };
        let bytes = encode_capability_manifest(&manifest);
        let a = codec::decode_detailed(&bytes).expect("manifest decodes as an AST");
        let root = a
            .as_form(a.root, "capabilities-manifest")
            .expect("root head");
        let entries = a.as_form(root[1], "entries").expect("entries head");
        assert_eq!(entries.len(), 5, "one entry per predicate arm");

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
    }
}
