//! The `http-request` / `http-response` value-form codec.
//!
//! Rust mirrors of the two userspace contracts
//! (`cdz-platform/contracts/userspace/http-{request,response}.cdz`) and the encode/decode between them and
//! the canonical `cadenza-ast` value form — the same form the compiler's own `Value.encode`/`Value.decode`
//! produce, so a request the gateway EMITS is decodable by a Cadenza handler guest, and the guest's
//! `http-response` decodes back here (the "one canonical codec across the Rust↔Cadenza boundary", design §5).
//!
//! The value-form primitives below are replicated faithfully from `cdz-platform`'s private
//! `src/contract_value.rs` (they are not `pub`), extended with the `String`/`List` support the platform's
//! set lacks (its contracts use only `Bytes`/`UInt`). The exact shapes were pinned EMPIRICALLY against the
//! compiler — `cdz compile <contract> && cdz run --format binary-ast | cdz convert --to sexpr` — not from
//! the doc alone (which was wrong about the nullary-variant form). The pinned forms, and the compiler-
//! produced byte fixtures the tests decode, are the cross-boundary guarantee:
//!
//! - a constructor `T.C(payload)` → the BARE-name form `(C <payload>…)`; a **nullary** variant carries the
//!   `unit` atom as its payload, so `Method.Get` is `(Get unit)` (NOT `(Get)`); a **single-constructor** sum
//!   ELIDES its constructor entirely — `Request.Request(rec)` / `Response.Response(rec)` / `Header.Header(rec)`
//!   are the record directly, with no wrapper.
//! - a record → the native `#record((= <field> <value>)…)` compound, fields in ascending NAME order.
//! - a `List(T)` value → the native `#list(<elem>…)` compound, elements in order (NOT sorted).
//! - `String` → a `Str` leaf; `Bytes` → a `Bytes` leaf; `Int64` → an `Int` leaf (decimal).
//! - the whole payload is wrapped at the encode boundary in a root ascription `(: <value> <Type>)`.

use bytes::Bytes;
use cadenza_ast::ast::{Arenas, Builder, CompoundCtor, IntValue, Leaf, Radix, Struct, StructId};
use std::sync::Arc;

// --- the Rust mirrors of the two contracts ---------------------------------------------------------------

/// The HTTP method — the `http-request` `Method` sum (each variant nullary).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
    Patch,
    Head,
    Options,
}

impl Method {
    /// The Cadenza constructor name for this method (the head of its `(Ctor unit)` value form).
    fn ctor(self) -> &'static str {
        match self {
            Method::Get => "Get",
            Method::Post => "Post",
            Method::Put => "Put",
            Method::Delete => "Delete",
            Method::Patch => "Patch",
            Method::Head => "Head",
            Method::Options => "Options",
        }
    }

    /// The method for a constructor name, or `None` for an unknown constructor.
    fn from_ctor(name: &str) -> Option<Method> {
        Some(match name {
            "Get" => Method::Get,
            "Post" => Method::Post,
            "Put" => Method::Put,
            "Delete" => Method::Delete,
            "Patch" => Method::Patch,
            "Head" => Method::Head,
            "Options" => Method::Options,
            _ => return None,
        })
    }
}

/// One HTTP header — the `Header` record (`name`, `value`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub name: String,
    pub value: String,
}

/// An inbound HTTP request — the `http-request` `Request` record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: Method,
    pub path: String,
    pub query: String,
    pub headers: Vec<Header>,
    pub body: Bytes,
}

/// An outbound HTTP response — the `http-response` `Response` record. `status` is the `Int64` status code
/// (always a small non-negative HTTP status, so a `u16` in Rust).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<Header>,
    pub body: Bytes,
}

// --- encode ----------------------------------------------------------------------------------------------

/// Encode an [`HttpRequest`] into the canonical binary-AST payload a handler's `on_message` receives.
#[must_use]
pub fn encode_request(req: &HttpRequest) -> Bytes {
    let mut b = Builder::new();
    let method = {
        let u = unit(&mut b);
        bare_ctor(&mut b, req.method.ctor(), vec![u])
    };
    let path = str_leaf(&mut b, &req.path);
    let query = str_leaf(&mut b, &req.query);
    let header_vals: Vec<StructId> = req
        .headers
        .iter()
        .map(|h| encode_header(&mut b, h))
        .collect();
    let headers = list_value(&mut b, header_vals);
    let body = bytes_leaf(&mut b, &req.body);
    let rec = record(
        &mut b,
        vec![
            ("method", method),
            ("path", path),
            ("query", query),
            ("headers", headers),
            ("body", body),
        ],
    );
    let root = ascribe(&mut b, rec, "Request");
    let arenas = b.finish(root);
    Bytes::from(cadenza_ast::codec::encode(&arenas))
}

/// Encode an [`HttpResponse`] into the canonical binary-AST bytes a handler returns on its closing `Break`.
#[must_use]
pub fn encode_response(resp: &HttpResponse) -> Bytes {
    let mut b = Builder::new();
    let status = uint_leaf(&mut b, u64::from(resp.status));
    let header_vals: Vec<StructId> = resp
        .headers
        .iter()
        .map(|h| encode_header(&mut b, h))
        .collect();
    let headers = list_value(&mut b, header_vals);
    let body = bytes_leaf(&mut b, &resp.body);
    let rec = record(
        &mut b,
        vec![("status", status), ("headers", headers), ("body", body)],
    );
    let root = ascribe(&mut b, rec, "Response");
    let arenas = b.finish(root);
    Bytes::from(cadenza_ast::codec::encode(&arenas))
}

/// A `Header` value — a single-constructor sum, so its value is the record directly (the `Header` ctor is
/// elided).
fn encode_header(b: &mut Builder, h: &Header) -> StructId {
    let name = str_leaf(b, &h.name);
    let value = str_leaf(b, &h.value);
    record(b, vec![("name", name), ("value", value)])
}

// --- decode ----------------------------------------------------------------------------------------------

/// Decode a canonical binary-AST payload into an [`HttpRequest`], or `None` if it is malformed / not a
/// `Request` shape. Total (never panics) — a malformed request is a rejected value the caller answers `400`.
#[must_use]
pub fn decode_request(bytes: &[u8]) -> Option<HttpRequest> {
    let arenas = cadenza_ast::codec::decode(bytes)?;
    let rec = as_ascribed(&arenas, arenas.root)?; // `Request` ctor is elided → the record directly
    let method = read_method(&arenas, record_field(&arenas, rec, "method")?)?;
    let path = read_str(&arenas, record_field(&arenas, rec, "path")?)?;
    let query = read_str(&arenas, record_field(&arenas, rec, "query")?)?;
    let headers = read_headers(&arenas, record_field(&arenas, rec, "headers")?)?;
    let body = read_bytes(&arenas, record_field(&arenas, rec, "body")?)?;
    Some(HttpRequest {
        method,
        path,
        query,
        headers,
        body,
    })
}

/// Decode a handler's closing-`Break` reason bytes into an [`HttpResponse`], or `None` if malformed.
#[must_use]
pub fn decode_response(bytes: &[u8]) -> Option<HttpResponse> {
    let arenas = cadenza_ast::codec::decode(bytes)?;
    let rec = as_ascribed(&arenas, arenas.root)?; // `Response` ctor is elided → the record directly
    let status = u16::try_from(read_uint(&arenas, record_field(&arenas, rec, "status")?)?).ok()?;
    let headers = read_headers(&arenas, record_field(&arenas, rec, "headers")?)?;
    let body = read_bytes(&arenas, record_field(&arenas, rec, "body")?)?;
    Some(HttpResponse {
        status,
        headers,
        body,
    })
}

/// Read a `List(Header)` value into a `Vec<Header>`, or `None` if not a list / a member is malformed.
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

/// A constructor application `(<name> <payload>…)` — the ctor name as the head, then its payload.
fn bare_ctor(b: &mut Builder, name: &str, payload: Vec<StructId>) -> StructId {
    let head = b.name(name);
    b.list(std::iter::once(head).chain(payload).collect())
}

/// The `unit` atom — the payload of a nullary variant (`Method.Get` → `(Get unit)`).
fn unit(b: &mut Builder) -> StructId {
    b.name("unit")
}

/// A root ascription `(: <value> <ty>)` — the top-level wrapper the decoder requires at the payload boundary.
fn ascribe(b: &mut Builder, value: StructId, ty: &str) -> StructId {
    let colon = b.name(":");
    let ty = b.name(ty);
    b.list(vec![colon, value, ty])
}

/// A record value — the native `#record((= <field> <value>)…)` compound, fields emitted in ascending NAME
/// order (the compiler's canonical order; the decoder reads records name-ordered).
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

/// A `String` leaf (`cadenza-ast` `Leaf::Str` — a text value, distinct from a `Leaf::Name` bare identifier).
fn str_leaf(b: &mut Builder, s: &str) -> StructId {
    b.atom_leaf(Leaf::Str(Arc::from(s)))
}

/// An integer leaf carrying `value`, written in decimal.
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

/// The value of a record's field named `name`, via the native-record reader.
fn record_field(arenas: &Arenas, id: StructId, name: &str) -> Option<StructId> {
    let fields = arenas.compound_form_of(id, CompoundCtor::Record)?;
    fields.iter().find_map(|&f| {
        let kv = arenas.as_form(f, "=")?;
        (kv.len() == 2 && arenas.as_name(kv[0]) == Some(name)).then_some(kv[1])
    })
}

/// The members of a native `#list(…)` value, or `None` if `id` is not a list.
fn read_list(arenas: &Arenas, id: StructId) -> Option<&[StructId]> {
    arenas.compound_form_of(id, CompoundCtor::List)
}

/// The [`Method`] of a `(Ctor unit)` value — the head constructor name, mapped to a method.
fn read_method(arenas: &Arenas, id: StructId) -> Option<Method> {
    match arenas.get(id) {
        Struct::List(items) => Method::from_ctor(arenas.as_name(*items.first()?)?),
        Struct::Atom(_) => None,
    }
}

/// A `String` leaf's text.
fn read_str(arenas: &Arenas, id: StructId) -> Option<String> {
    arenas.as_str(id).map(str::to_string)
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

/// An integer leaf's value as a `u64`, or `None` if not an integer / negative / too large.
fn read_uint(arenas: &Arenas, id: StructId) -> Option<u64> {
    match arenas.get(id) {
        Struct::Atom(leaf) => match arenas.leaf(*leaf) {
            Leaf::Int { value, .. } => value.to_u128().and_then(|u| u64::try_from(u).ok()),
            _ => None,
        },
        Struct::List(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> HttpRequest {
        HttpRequest {
            method: Method::Post,
            path: "/".to_string(),
            query: "q=1".to_string(),
            headers: vec![
                Header {
                    name: "accept".to_string(),
                    value: "*/*".to_string(),
                },
                Header {
                    name: "x-n".to_string(),
                    value: "v".to_string(),
                },
            ],
            body: Bytes::from_static(b"hi"),
        }
    }

    fn sample_response() -> HttpResponse {
        HttpResponse {
            status: 200,
            headers: vec![],
            body: Bytes::from_static(b"hello"),
        }
    }

    #[test]
    fn request_round_trips() {
        let req = sample_request();
        let decoded = decode_request(&encode_request(&req)).expect("request decodes");
        assert_eq!(decoded, req);
    }

    #[test]
    fn response_round_trips() {
        let resp = sample_response();
        let decoded = decode_response(&encode_response(&resp)).expect("response decodes");
        assert_eq!(decoded, resp);
    }

    #[test]
    fn every_method_round_trips() {
        for m in [
            Method::Get,
            Method::Post,
            Method::Put,
            Method::Delete,
            Method::Patch,
            Method::Head,
            Method::Options,
        ] {
            let mut req = sample_request();
            req.method = m;
            let decoded = decode_request(&encode_request(&req)).expect("decodes");
            assert_eq!(decoded.method, m, "method {m:?} did not round-trip");
        }
    }

    #[test]
    fn empty_headers_and_body_round_trip() {
        let req = HttpRequest {
            method: Method::Get,
            path: "/health".to_string(),
            query: String::new(),
            headers: vec![],
            body: Bytes::new(),
        };
        assert_eq!(decode_request(&encode_request(&req)).unwrap(), req);
        let resp = HttpResponse {
            status: 404,
            headers: vec![Header {
                name: "content-type".to_string(),
                value: "text/plain".to_string(),
            }],
            body: Bytes::new(),
        };
        assert_eq!(decode_response(&encode_response(&resp)).unwrap(), resp);
    }

    /// A malformed / wrong-shape payload decodes to `None`, never panics.
    #[test]
    fn malformed_is_none_not_panic() {
        assert!(decode_request(b"not a binary ast").is_none());
        assert!(decode_response(&[]).is_none());
        // A response's bytes are not a valid request (missing method/path) → None.
        assert!(decode_request(&encode_response(&sample_response())).is_none());
    }

    /// CROSS-COMPILER PIN: decode bytes the ACTUAL compiler produced (`cdz run --format binary-ast` over a
    /// literal `Request`/`Response` value) and assert our reader recovers the exact value. This proves the
    /// codec matches the compiler's `Value.encode`, not merely that it is self-consistent. Regenerate the
    /// fixtures if the canonical binary-AST layout ever changes.
    #[test]
    fn decodes_compiler_produced_fixtures() {
        let req = decode_request(include_bytes!("../tests/fixtures/request.bin"))
            .expect("compiler-produced request decodes");
        assert_eq!(req, sample_request());

        let resp = decode_response(include_bytes!("../tests/fixtures/response.bin"))
            .expect("compiler-produced response decodes");
        assert_eq!(resp, sample_response());
    }
}
