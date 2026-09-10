//! The http-outpost **control-plane wire contract** (`DESIGN-http-outpost-conformance-harness.md` §4).
//!
//! Rust mirrors + binary-AST codec for the three frames the control server and the HTTP gateway exchange
//! over their persistent bidirectional link:
//!
//! - [`ControlConfig`] — shipped control → gateway on connect: the CAS url + credential + the root-router
//!   program hash. The gateway holds nothing else; it learns everything here (and on later pushes).
//! - [`ControlUp`] — a handler's `control.send` on its way UP. The gateway wraps the handler's OPAQUE
//!   `payload` with **provenance + routing context** so the control server can route and respond: which
//!   handler (`program`), the session, a `correlation` token unique to this send, and as much of the
//!   originating HTTP request as the gateway can attach (`request`: method + path + headers). The gateway
//!   never inspects the payload — a pure opaque router — but it DOES stamp the routing context.
//! - [`ControlDown`] — control → handler, addressed by `session`. A `correlation` matching a pending
//!   `ControlUp` is the RESPONSE to that `control.send` (folded back into the awaiting handler call); an
//!   empty/unmatched `correlation` is an unsolicited push delivered as an `on_notification`.
//!
//! Each frame is a binary-AST value (the standing "binary-AST is THE data-exchange format" directive),
//! encoded with the canonical value-form primitives — the same forms the compiler's own `Value.encode`/
//! `Value.decode` produce. The frames are exchanged Rust↔Rust (gateway ↔ control server); the handler's
//! opaque `payload` is the only part a Cadenza guest sees, so the envelopes use the platform-boundary form
//! (root ascription only) and the decoders are ascription-tolerant. The value-form primitives are
//! replicated faithfully from `cdz-platform`'s private `src/contract_value.rs` (shared with
//! `cdz-http-gateway`'s `codec.rs`), extended with the `String`/`List` support the platform set lacks.

use bytes::Bytes;
use cadenza_ast::ast::{Arenas, Builder, CompoundCtor, IntValue, Leaf, Radix, Struct, StructId};
use cdz_str::Str;
use std::sync::Arc;

// --- the Rust mirrors of the control-plane frames --------------------------------------------------------

/// One HTTP header (`name`, `value`) — reused in the request context a [`ControlUp`] carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub name: Str,
    pub value: Str,
}

/// As much of the originating HTTP request as the gateway attaches to a [`ControlUp`], so the control
/// server can route a handler's `control.send` correctly without re-parsing the opaque payload. `method`
/// is the request method as a plain string (`"GET"`, `"POST"`, …); `path` the request path; `headers` the
/// request headers the gateway chose to forward.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RequestContext {
    pub method: Str,
    pub path: Str,
    pub headers: Vec<Header>,
}

/// The gateway's ENTIRE boot configuration, shipped by the control server on connect: `cas_url` +
/// `cas_credential` are how it reaches the content-addressed store (`GET {cas_url}/{hash}` with
/// `Authorization: Bearer {cas_credential}`); `root_router` is the `ProgramHash` bytes (33) of the root
/// router program the gateway fetches from that CAS and calls per request. A later push re-sends
/// `root_router` (a new router, applied live) or a fresh config (credential rotation). Single-ctor record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlConfig {
    /// The base URL of the content-addressed store the gateway fetches programs (+ their deps) from.
    pub cas_url: Str,
    /// The credential the gateway presents to the CAS (`Authorization: Bearer …`; empty ⇒ no auth).
    pub cas_credential: Bytes,
    /// The `ProgramHash` (its 33 raw bytes) of the root-router program the gateway calls per request.
    pub root_router: Bytes,
}

/// A handler-to-control message on its way UP the control link. The gateway wraps a looping program's
/// `control.send` OPAQUE `payload` in this envelope, stamping `program` (the emitting handler's
/// `ProgramHash` bytes — WHICH handler), `session` (the connection/session it ran for), `correlation` (a
/// token unique to this send, echoed on the [`ControlDown`] response so it reaches the exact awaiting
/// call), and `request` (the originating HTTP request context, so control can route). Single-ctor record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlUp {
    /// The emitting handler's `ProgramHash` bytes — which handler is sending.
    pub program: Bytes,
    /// The connection/session id the handler ran for.
    pub session: Bytes,
    /// A token unique to this `control.send`, echoed back on the response to correlate it.
    pub correlation: Bytes,
    /// The handler's opaque message payload (never inspected by the gateway).
    pub payload: Bytes,
    /// The originating HTTP request context, so control can route without re-parsing the payload.
    pub request: RequestContext,
}

/// A control-to-handler message pushed DOWN the control link, addressed by `session`. When `correlation`
/// matches a pending [`ControlUp`], this is the RESPONSE to that `control.send` (folded back into the
/// awaiting handler call); an empty / unmatched `correlation` is an unsolicited push the gateway delivers
/// to the session as an `on_notification`. The gateway reads `session` + `correlation` (addressing), never
/// `payload`. Single-ctor record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlDown {
    /// The target session id the gateway routes this message to.
    pub session: Bytes,
    /// The correlation token: matches a pending `ControlUp` (⇒ a response) or is empty (⇒ a push).
    pub correlation: Bytes,
    /// The control server's opaque message payload (never inspected by the gateway).
    pub payload: Bytes,
}

// --- encode --------------------------------------------------------------------------------------------

/// Encode a [`ControlConfig`] boot frame. Single-ctor record (fields name-sorted), root-ascribed.
#[must_use]
pub fn encode_control_config(config: &ControlConfig) -> Bytes {
    let mut b = Builder::new();
    let cas_url = str_leaf(&mut b, &config.cas_url);
    let cas_credential = bytes_leaf(&mut b, &config.cas_credential);
    let root_router = bytes_leaf(&mut b, &config.root_router);
    let rec = record(
        &mut b,
        vec![
            ("cas-credential", cas_credential),
            ("cas-url", cas_url),
            ("root-router", root_router),
        ],
    );
    finish(b, rec, "ControlConfig")
}

/// Encode a [`ControlUp`] envelope (handler → control). Single-ctor record (fields name-sorted, `request`
/// a nested `RequestContext` record), root-ascribed.
#[must_use]
pub fn encode_control_up(msg: &ControlUp) -> Bytes {
    let mut b = Builder::new();
    let correlation = bytes_leaf(&mut b, &msg.correlation);
    let payload = bytes_leaf(&mut b, &msg.payload);
    let program = bytes_leaf(&mut b, &msg.program);
    let request = encode_request_context(&mut b, &msg.request);
    let session = bytes_leaf(&mut b, &msg.session);
    let rec = record(
        &mut b,
        vec![
            ("correlation", correlation),
            ("payload", payload),
            ("program", program),
            ("request", request),
            ("session", session),
        ],
    );
    finish(b, rec, "ControlUp")
}

/// Encode a [`ControlDown`] envelope (control → handler). Single-ctor record (fields name-sorted), ascribed.
#[must_use]
pub fn encode_control_down(msg: &ControlDown) -> Bytes {
    let mut b = Builder::new();
    let correlation = bytes_leaf(&mut b, &msg.correlation);
    let payload = bytes_leaf(&mut b, &msg.payload);
    let session = bytes_leaf(&mut b, &msg.session);
    let rec = record(
        &mut b,
        vec![
            ("correlation", correlation),
            ("payload", payload),
            ("session", session),
        ],
    );
    finish(b, rec, "ControlDown")
}

/// A `RequestContext` value — a record `{ headers, method, path }` (fields name-sorted); `headers` a
/// `List(Header)` of `{ name, value }` records. Returned as an inner value to nest in a [`ControlUp`].
fn encode_request_context(b: &mut Builder, ctx: &RequestContext) -> StructId {
    let header_vals: Vec<StructId> = ctx.headers.iter().map(|h| encode_header(b, h)).collect();
    let headers = list_value(b, header_vals);
    let method = str_leaf(b, &ctx.method);
    let path = str_leaf(b, &ctx.path);
    record(
        b,
        vec![("headers", headers), ("method", method), ("path", path)],
    )
}

/// A `Header` value — a record `{ name, value }` (fields name-sorted).
fn encode_header(b: &mut Builder, h: &Header) -> StructId {
    let name = str_leaf(b, &h.name);
    let value = str_leaf(b, &h.value);
    record(b, vec![("name", name), ("value", value)])
}

// --- decode --------------------------------------------------------------------------------------------

/// Decode a [`ControlConfig`], or `None` if malformed — the inverse of [`encode_control_config`].
#[must_use]
pub fn decode_control_config(bytes: &[u8]) -> Option<ControlConfig> {
    let arenas = cadenza_ast::codec::decode(bytes)?;
    let rec = unascribe(&arenas, arenas.root);
    Some(ControlConfig {
        cas_url: read_str(&arenas, record_field(&arenas, rec, "cas-url")?)?,
        cas_credential: read_bytes(&arenas, record_field(&arenas, rec, "cas-credential")?)?,
        root_router: read_bytes(&arenas, record_field(&arenas, rec, "root-router")?)?,
    })
}

/// Decode a [`ControlUp`], or `None` if malformed — the inverse of [`encode_control_up`].
#[must_use]
pub fn decode_control_up(bytes: &[u8]) -> Option<ControlUp> {
    let arenas = cadenza_ast::codec::decode(bytes)?;
    let rec = unascribe(&arenas, arenas.root);
    Some(ControlUp {
        program: read_bytes(&arenas, record_field(&arenas, rec, "program")?)?,
        session: read_bytes(&arenas, record_field(&arenas, rec, "session")?)?,
        correlation: read_bytes(&arenas, record_field(&arenas, rec, "correlation")?)?,
        payload: read_bytes(&arenas, record_field(&arenas, rec, "payload")?)?,
        request: read_request_context(&arenas, record_field(&arenas, rec, "request")?)?,
    })
}

/// Decode a [`ControlDown`], or `None` if malformed — the inverse of [`encode_control_down`].
#[must_use]
pub fn decode_control_down(bytes: &[u8]) -> Option<ControlDown> {
    let arenas = cadenza_ast::codec::decode(bytes)?;
    let rec = unascribe(&arenas, arenas.root);
    Some(ControlDown {
        session: read_bytes(&arenas, record_field(&arenas, rec, "session")?)?,
        correlation: read_bytes(&arenas, record_field(&arenas, rec, "correlation")?)?,
        payload: read_bytes(&arenas, record_field(&arenas, rec, "payload")?)?,
    })
}

/// Read a `RequestContext` record, or `None` if malformed / not a record.
fn read_request_context(arenas: &Arenas, id: StructId) -> Option<RequestContext> {
    Some(RequestContext {
        method: read_str(arenas, record_field(arenas, id, "method")?)?,
        path: read_str(arenas, record_field(arenas, id, "path")?)?,
        headers: read_headers(arenas, record_field(arenas, id, "headers")?)?,
    })
}

/// Read a `List(Header)` into a `Vec<Header>`, or `None` if not a list / a member is malformed.
fn read_headers(arenas: &Arenas, id: StructId) -> Option<Vec<Header>> {
    read_list(arenas, id)?
        .iter()
        .map(|&h| {
            Some(Header {
                name: read_str(arenas, record_field(arenas, h, "name")?)?,
                value: read_str(arenas, record_field(arenas, h, "value")?)?,
            })
        })
        .collect()
}

// --- the canonical value-form primitives (replicated from cdz-platform/src/contract_value.rs) ------------

/// Wrap `value` in the root ascription `(: value ty)`, finish the AST, and encode it to binary-AST bytes.
fn finish(mut b: Builder, value: StructId, ty: &str) -> Bytes {
    let root = ascribe(&mut b, value, ty);
    let arenas = b.finish(root);
    Bytes::from(cadenza_ast::codec::encode(&arenas))
}

/// A root ascription `(: <value> <ty>)` — the top-level wrapper the decoder tolerates at the payload boundary.
fn ascribe(b: &mut Builder, value: StructId, ty: &str) -> StructId {
    let colon = b.name(":");
    let ty = b.name(ty);
    b.list(vec![colon, value, ty])
}

/// A record value — the native `#record((= <field> <value>)…)` compound, fields in ascending NAME order
/// (the compiler's canonical order; the decoder reads records name-ordered).
fn record(b: &mut Builder, fields: Vec<(&str, StructId)>) -> StructId {
    let mut fields = fields;
    fields.sort_by_key(|&(name, _)| name);
    let pairs: Vec<StructId> = fields
        .into_iter()
        .map(|(name, value)| {
            let key = b.name(name);
            b.field_pair(key, value)
        })
        .collect();
    b.compound(CompoundCtor::Record, &pairs)
}

/// A `List(T)` value — the native `#list(<elem>…)` compound, elements in order (NOT sorted).
fn list_value(b: &mut Builder, elems: Vec<StructId>) -> StructId {
    b.compound(CompoundCtor::List, &elems)
}

/// A `Bytes` leaf.
fn bytes_leaf(b: &mut Builder, bytes: &[u8]) -> StructId {
    b.atom_leaf(Leaf::Bytes(Arc::from(bytes)))
}

/// A `String` leaf (`cadenza-ast` `Leaf::Str` — text, distinct from a `Leaf::Name` bare identifier).
fn str_leaf(b: &mut Builder, s: &str) -> StructId {
    b.atom_leaf(Leaf::Str(Arc::from(s)))
}

/// An integer leaf carrying `value`, written in decimal. (Unused by the current frames; kept alongside the
/// other primitives so the shared set stays complete for the frames a later slice adds.)
#[allow(dead_code)]
fn uint_leaf(b: &mut Builder, value: u64) -> StructId {
    b.atom_leaf(Leaf::Int {
        value: IntValue::from_u128(u128::from(value)),
        radix: Radix::Dec,
    })
}

// --- the readers (exact inverses; total) -----------------------------------------------------------------

/// The value inside a root ascription `(: <value> <ty>)`, ignoring the type token.
fn as_ascribed(arenas: &Arenas, id: StructId) -> Option<StructId> {
    let inner = arenas.as_form(id, ":")?;
    (inner.len() == 2).then_some(inner[0])
}

/// Strip an optional ascription `(: <value> <ty>)`, returning the inner value (or `id` unchanged if not
/// ascribed) — tolerant of an ascription anywhere a value/record is expected.
fn unascribe(arenas: &Arenas, id: StructId) -> StructId {
    as_ascribed(arenas, id).unwrap_or(id)
}

/// The value of a record's field named `name`, via the native-record reader. Strips an optional ascription
/// on `id` first, so it reads both the bare and the ascribed record form.
fn record_field(arenas: &Arenas, id: StructId, name: &str) -> Option<StructId> {
    let fields = arenas.compound_form_of(unascribe(arenas, id), CompoundCtor::Record)?;
    fields.iter().find_map(|&f| {
        let kv = arenas.as_form(f, "=")?;
        (kv.len() == 2 && arenas.as_name(kv[0]) == Some(name)).then_some(kv[1])
    })
}

/// The members of a native `#list(…)` value, or `None` if `id` is not a list.
fn read_list(arenas: &Arenas, id: StructId) -> Option<&[StructId]> {
    arenas.compound_form_of(id, CompoundCtor::List)
}

/// A `String` leaf's text, as a [`Str`] (O(1)-clone, shares the wire bytes).
fn read_str(arenas: &Arenas, id: StructId) -> Option<Str> {
    arenas.as_str(id).map(Str::from)
}

/// A `Bytes` leaf's bytes.
fn read_bytes(arenas: &Arenas, id: StructId) -> Option<Bytes> {
    match arenas.get(id) {
        Struct::Atom(leaf) => match arenas.leaf(*leaf) {
            Leaf::Bytes(bytes) => Some(Bytes::copy_from_slice(bytes)),
            _ => None,
        },
        Struct::List(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request_context() -> RequestContext {
        RequestContext {
            method: Str::from("POST"),
            path: Str::from("/emit"),
            headers: vec![
                Header {
                    name: Str::from("accept"),
                    value: Str::from("*/*"),
                },
                Header {
                    name: Str::from("x-trace"),
                    value: Str::from("abc"),
                },
            ],
        }
    }

    #[test]
    fn control_config_round_trips() {
        let config = ControlConfig {
            cas_url: Str::from("https://cas.example.internal:8443/blobs"),
            cas_credential: Bytes::from_static(b"bearer-token-abc123"),
            root_router: Bytes::from_static(b"cdz-router.root................."),
        };
        assert_eq!(
            decode_control_config(&encode_control_config(&config)).unwrap(),
            config
        );
        // An empty credential (dev / no-auth CAS) still round-trips.
        let no_auth = ControlConfig {
            cas_url: Str::from("http://localhost:9000"),
            cas_credential: Bytes::new(),
            root_router: Bytes::from_static(b"cdz-router.root................."),
        };
        assert_eq!(
            decode_control_config(&encode_control_config(&no_auth)).unwrap(),
            no_auth
        );
    }

    #[test]
    fn control_up_round_trips_with_provenance_and_request_context() {
        let up = ControlUp {
            program: Bytes::from_static(b"cdz-http.handler.emitter........"),
            session: Bytes::from_static(b"conn-42"),
            correlation: Bytes::from_static(b"corr-1"),
            payload: Bytes::from_static(b"opaque handler->control bytes"),
            request: sample_request_context(),
        };
        let decoded = decode_control_up(&encode_control_up(&up)).expect("control-up decodes");
        assert_eq!(decoded, up);
        // The handler id + request context + correlation survive the round-trip verbatim.
        assert_eq!(decoded.program, up.program);
        assert_eq!(decoded.correlation, up.correlation);
        assert_eq!(decoded.request.method, "POST");
        assert_eq!(decoded.request.path, "/emit");
        assert_eq!(decoded.request.headers.len(), 2);
    }

    #[test]
    fn control_up_with_empty_request_context_round_trips() {
        let up = ControlUp {
            program: Bytes::from_static(b"p"),
            session: Bytes::new(),
            correlation: Bytes::new(),
            payload: Bytes::new(),
            request: RequestContext::default(),
        };
        assert_eq!(decode_control_up(&encode_control_up(&up)).unwrap(), up);
    }

    #[test]
    fn control_down_round_trips_response_and_push() {
        // A correlated response.
        let response = ControlDown {
            session: Bytes::from_static(b"conn-42"),
            correlation: Bytes::from_static(b"corr-1"),
            payload: Bytes::from_static(b"PONG"),
        };
        assert_eq!(
            decode_control_down(&encode_control_down(&response)).unwrap(),
            response
        );
        // An unsolicited push (empty correlation).
        let push = ControlDown {
            session: Bytes::from_static(b"conn-42"),
            correlation: Bytes::new(),
            payload: Bytes::from_static(b"server push"),
        };
        assert_eq!(
            decode_control_down(&encode_control_down(&push)).unwrap(),
            push
        );
    }

    #[test]
    fn malformed_frames_are_none_not_panic() {
        assert!(decode_control_config(b"not a config").is_none());
        assert!(decode_control_up(b"garbage").is_none());
        assert!(decode_control_down(&[]).is_none());
        // A down envelope lacks `program`/`request`, so it is not a valid up envelope.
        let down = encode_control_down(&ControlDown {
            session: Bytes::from_static(b"s"),
            correlation: Bytes::from_static(b"c"),
            payload: Bytes::from_static(b"p"),
        });
        assert!(decode_control_up(&down).is_none());
        // A config is not a down envelope (no `session`).
        let config = encode_control_config(&ControlConfig {
            cas_url: Str::from("x"),
            cas_credential: Bytes::new(),
            root_router: Bytes::new(),
        });
        assert!(decode_control_down(&config).is_none());
    }
}
