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

/// One entry of an `http-route-table` frame (`http-route-table.cdz` `Route`): a `(method, path)` route
/// served by the handler addressed by `handler` (a `ProgramHash`'s bytes), folding the contract `contract`
/// (a `ContractId`'s bytes). `handler`/`contract` stay raw `Bytes` here — the codec layer is
/// `cdz-platform`-free; the router (`gateway`) maps them to the typed ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteFrame {
    pub method: Method,
    pub path: String,
    pub handler: Bytes,
    pub contract: Bytes,
}

/// An inbound WebSocket event (`ws-event.cdz` `Event`) the edge surfaces to a per-connection session:
/// `Connect` on upgrade, a `Frame` per inbound frame, `Disconnect` on close. `conn` is the connection id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsEvent {
    Connect { conn: Bytes },
    Frame { conn: Bytes, data: Bytes },
    Disconnect { conn: Bytes },
}

/// An outbound WebSocket frame (`ws-send.cdz` `Send`) a session emits to push `data` to connection `conn`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsSend {
    pub conn: Bytes,
    pub data: Bytes,
}

/// The router governing program's routing decision (`guests/router/reducer.cdz` `Decision`): the handler
/// component to spawn (a `ProgramHash`'s bytes) + the contract-id it folds (a `ContractId`'s bytes). A
/// SINGLE-constructor sum → a plain record value; an EMPTY `handler` is the no-match sentinel (the router's
/// stand-in for "no route" — the gateway answers its 404 floor), since the guest compiler cannot
/// `Value.encode` a multi-ctor `Route | NotFound` sum. `handler`/`contract` stay raw `Bytes` here (the codec
/// layer is `cdz-platform`-free; the gateway maps them to the typed ids).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteDecision {
    pub handler: Bytes,
    pub contract: Bytes,
}

impl RouteDecision {
    /// Whether the router matched a route: a non-empty `handler`. An empty `handler` is the no-match
    /// sentinel (→ the gateway's 404 floor).
    #[must_use]
    pub fn is_match(&self) -> bool {
        !self.handler.is_empty()
    }
}

/// A `dispatch` effect a router program emits to hand a request off to a subprogram (`DESIGN-http-outpost-
/// drive-contract.md` §2): `subprogram` is the handler's `ProgramHash` bytes (the gateway fetches it from the
/// CAS + drives it), `input` is the bytes delivered to it as its first message (typically the encoded
/// `http-request`). The subprogram's terminal `Break` reason folds back as the dispatch effect's answer. A
/// single-constructor record on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchEffect {
    /// The subprogram's `ProgramHash` bytes.
    pub subprogram: Bytes,
    /// The bytes delivered to the subprogram as its first message payload.
    pub input: Bytes,
}

/// A handler-to-control message on its way UP the control link (`DESIGN-http-outpost-drive-contract.md` §3,
/// the bidirectional-messaging directive): a looping program emits a `cdz.control.send` effect with an OPAQUE
/// payload, and the gateway wraps it in this envelope, stamping PROVENANCE (`program` = the emitting
/// program's `ProgramHash` bytes, `session` = the connection/session id it ran for) so the control server
/// knows where it came from. The gateway never inspects `payload` — it is a pure opaque router. A single-
/// constructor record on the wire (binary-AST, self-describing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlUp {
    /// The emitting program's `ProgramHash` bytes.
    pub program: Bytes,
    /// The connection/session id the program ran for.
    pub session: Bytes,
    /// The program's opaque message payload (never inspected by the gateway).
    pub payload: Bytes,
}

/// A control-to-handler message pushed DOWN the control link (`DESIGN-http-outpost-drive-contract.md` §3):
/// the control server addresses a message to a specific handler session; the gateway ROUTES it by
/// `session` to that reducer instance and folds it as an `on_notification` (the handler decides what to do).
/// The gateway reads only `session` (the addressing), never `payload`. A single-constructor record on the
/// wire (binary-AST).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlDown {
    /// The target session id the gateway routes this message to.
    pub session: Bytes,
    /// The control server's opaque message payload (never inspected by the gateway).
    pub payload: Bytes,
}

/// The gateway's ENTIRE boot configuration, shipped by the control server on connect (`DESIGN-http-outpost.md`
/// §2/§3 + the dumb-gateway redirect): the gateway holds no table and no local files — it learns everything
/// from the control server. `cas_url` + `cas_credential` are how it reaches the content-addressed store
/// (`GET {cas_url}/{hash}` with `Authorization: Bearer {cas_credential}`); `root_router` is the `ProgramHash`
/// of the ROOT ROUTER program the gateway fetches from that CAS and calls per request (routing lives inside
/// that program, not the gateway). A later control push re-sends `root_router` (a new router — applied live)
/// or a fresh `ControlConfig` (credential rotation). A single-constructor record value on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlConfig {
    /// The base URL of the content-addressed store the gateway fetches programs (+ their deps) from by hash.
    pub cas_url: String,
    /// The credential the gateway presents to the CAS (`Authorization: Bearer …`).
    pub cas_credential: Bytes,
    /// The `ProgramHash` (its 33 raw bytes) of the root router program the gateway calls per request.
    pub root_router: Bytes,
}

// --- encode ----------------------------------------------------------------------------------------------

/// Encode an [`HttpRequest`] into the canonical binary-AST payload a handler's `on_message` receives.
#[must_use]
pub fn encode_request(req: &HttpRequest) -> Bytes {
    let mut b = Builder::new();
    let method = encode_method(&mut b, req.method);
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

/// A `Method` value — the BARE `(<Ctor> unit)` form. `Value.encode` does NOT ascribe a variant value
/// (empirically pinned: it renders `Method.Post` as `(Post unit)`, not `(: (Post unit) Method)`), and
/// `Value.decode` reads it type-directed — so encoding it bare is what a request-reading guest decodes.
fn encode_method(b: &mut Builder, method: Method) -> StructId {
    let u = unit(b);
    bare_ctor(b, method.ctor(), vec![u])
}

/// A `Header` value — a single-constructor sum (ctor elided → the record), ascribed with its nominal type
/// name. `Value.encode` ascribes a RECORD value (`(: #record… Header)`) but NOT a variant/list/scalar; a
/// request-reading guest's `Value.decode` rejects a bare-record header (the forward-path e2e proved the
/// ascription is required here while the method's is not).
fn encode_header(b: &mut Builder, h: &Header) -> StructId {
    let name = str_leaf(b, &h.name);
    let value = str_leaf(b, &h.value);
    let rec = record(b, vec![("name", name), ("value", value)]);
    ascribe(b, rec, "Header")
}

/// Encode a route table into the canonical binary-AST frame the control server ships (`http-route-table`
/// `RouteTable`). `RouteTable`/`Route` are single-constructor sums, so both elide their ctors — the value
/// is the `#list` of `Route` records directly, under the root ascription.
#[must_use]
pub fn encode_route_table(routes: &[RouteFrame]) -> Bytes {
    let mut b = Builder::new();
    let entries: Vec<StructId> = routes
        .iter()
        .map(|r| {
            let method = encode_method(&mut b, r.method);
            let path = str_leaf(&mut b, &r.path);
            let handler = bytes_leaf(&mut b, &r.handler);
            let contract = bytes_leaf(&mut b, &r.contract);
            let rec = record(
                &mut b,
                vec![
                    ("method", method),
                    ("path", path),
                    ("handler", handler),
                    ("contract", contract),
                ],
            );
            ascribe(&mut b, rec, "Route")
        })
        .collect();
    let list = list_value(&mut b, entries);
    let root = ascribe(&mut b, list, "RouteTable");
    let arenas = b.finish(root);
    Bytes::from(cadenza_ast::codec::encode(&arenas))
}

/// Encode a `RouteQuery` — the per-request envelope the gateway delivers to the DYNAMIC (stateless) router
/// guest (`guests/router-dynamic`): the encoded `http-request` bytes + the current route-table frame bytes.
/// The router is a pure function of `(request, table)` (the table rides in the message, not KV state — the
/// P3b pivot), so the gateway holds the live table and passes it per request. `RouteQuery` is a
/// single-constructor sum → the record directly (fields name-sorted), under the root ascription.
#[must_use]
pub fn encode_route_query(request: &[u8], table: &[u8]) -> Bytes {
    let mut b = Builder::new();
    let request = bytes_leaf(&mut b, request);
    let table = bytes_leaf(&mut b, table);
    let rec = record(&mut b, vec![("request", request), ("table", table)]);
    let root = ascribe(&mut b, rec, "RouteQuery");
    let arenas = b.finish(root);
    Bytes::from(cadenza_ast::codec::encode(&arenas))
}

/// Encode a [`DispatchEffect`] — single-ctor record (fields name-sorted), root-ascribed.
#[must_use]
pub fn encode_dispatch(effect: &DispatchEffect) -> Bytes {
    let mut b = Builder::new();
    let input = bytes_leaf(&mut b, &effect.input);
    let subprogram = bytes_leaf(&mut b, &effect.subprogram);
    let rec = record(&mut b, vec![("input", input), ("subprogram", subprogram)]);
    let root = ascribe(&mut b, rec, "DispatchEffect");
    let arenas = b.finish(root);
    Bytes::from(cadenza_ast::codec::encode(&arenas))
}

/// Decode a [`DispatchEffect`], or `None` if malformed — the inverse of [`encode_dispatch`].
#[must_use]
pub fn decode_dispatch(bytes: &[u8]) -> Option<DispatchEffect> {
    let arenas = cadenza_ast::codec::decode(bytes)?;
    let rec = unascribe(&arenas, arenas.root);
    Some(DispatchEffect {
        subprogram: read_bytes(&arenas, record_field(&arenas, rec, "subprogram")?)?,
        input: read_bytes(&arenas, record_field(&arenas, rec, "input")?)?,
    })
}

/// Encode a [`ControlUp`] envelope (handler → control) — single-ctor record, fields name-sorted, root-ascribed.
#[must_use]
pub fn encode_control_up(msg: &ControlUp) -> Bytes {
    let mut b = Builder::new();
    let payload = bytes_leaf(&mut b, &msg.payload);
    let program = bytes_leaf(&mut b, &msg.program);
    let session = bytes_leaf(&mut b, &msg.session);
    let rec = record(
        &mut b,
        vec![
            ("payload", payload),
            ("program", program),
            ("session", session),
        ],
    );
    let root = ascribe(&mut b, rec, "ControlUp");
    let arenas = b.finish(root);
    Bytes::from(cadenza_ast::codec::encode(&arenas))
}

/// Encode a [`ControlDown`] envelope (control → handler) — single-ctor record, fields name-sorted, ascribed.
#[must_use]
pub fn encode_control_down(msg: &ControlDown) -> Bytes {
    let mut b = Builder::new();
    let payload = bytes_leaf(&mut b, &msg.payload);
    let session = bytes_leaf(&mut b, &msg.session);
    let rec = record(&mut b, vec![("payload", payload), ("session", session)]);
    let root = ascribe(&mut b, rec, "ControlDown");
    let arenas = b.finish(root);
    Bytes::from(cadenza_ast::codec::encode(&arenas))
}

/// Encode a [`ControlConfig`] — the boot-config frame the control server ships the gateway on connect. A
/// single-constructor sum → the record directly (fields name-sorted), under the root ascription.
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
    let root = ascribe(&mut b, rec, "ControlConfig");
    let arenas = b.finish(root);
    Bytes::from(cadenza_ast::codec::encode(&arenas))
}

/// Encode a [`WsEvent`] into the canonical binary-AST payload a per-connection WS session's `on_message`
/// receives. `Event` is a multi-constructor sum, so the value is the bare-name `(<Ctor> #record…)` form
/// (not elided), under the root ascription.
#[must_use]
pub fn encode_ws_event(event: &WsEvent) -> Bytes {
    let mut b = Builder::new();
    let value = match event {
        WsEvent::Connect { conn } => {
            let c = bytes_leaf(&mut b, conn);
            let rec = record(&mut b, vec![("conn", c)]);
            bare_ctor(&mut b, "Connect", vec![rec])
        }
        WsEvent::Frame { conn, data } => {
            let c = bytes_leaf(&mut b, conn);
            let d = bytes_leaf(&mut b, data);
            let rec = record(&mut b, vec![("conn", c), ("data", d)]);
            bare_ctor(&mut b, "Frame", vec![rec])
        }
        WsEvent::Disconnect { conn } => {
            let c = bytes_leaf(&mut b, conn);
            let rec = record(&mut b, vec![("conn", c)]);
            bare_ctor(&mut b, "Disconnect", vec![rec])
        }
    };
    let root = ascribe(&mut b, value, "Event");
    let arenas = b.finish(root);
    Bytes::from(cadenza_ast::codec::encode(&arenas))
}

/// Encode a [`WsSend`] into the canonical binary-AST bytes. `Send` is a single-constructor sum → the ctor
/// elides to the record directly, under the root ascription.
#[must_use]
pub fn encode_ws_send(send: &WsSend) -> Bytes {
    let mut b = Builder::new();
    let conn = bytes_leaf(&mut b, &send.conn);
    let data = bytes_leaf(&mut b, &send.data);
    let rec = record(&mut b, vec![("conn", conn), ("data", data)]);
    let root = ascribe(&mut b, rec, "Send");
    let arenas = b.finish(root);
    Bytes::from(cadenza_ast::codec::encode(&arenas))
}

/// Encode a [`RouteDecision`] into canonical binary-AST bytes — the form the router guest's
/// `Value.encode(Decision.Decision({ handler, contract }))` produces: `Decision` is single-ctor → the record
/// directly, under the root ascription.
#[must_use]
pub fn encode_decision(decision: &RouteDecision) -> Bytes {
    let mut b = Builder::new();
    let handler = bytes_leaf(&mut b, &decision.handler);
    let contract = bytes_leaf(&mut b, &decision.contract);
    let rec = record(&mut b, vec![("handler", handler), ("contract", contract)]);
    let root = ascribe(&mut b, rec, "Decision");
    let arenas = b.finish(root);
    Bytes::from(cadenza_ast::codec::encode(&arenas))
}

// --- decode ----------------------------------------------------------------------------------------------

/// Decode a canonical binary-AST payload into an [`HttpRequest`], or `None` if it is malformed / not a
/// `Request` shape. Total (never panics) — a malformed request is a rejected value the caller answers `400`.
#[must_use]
pub fn decode_request(bytes: &[u8]) -> Option<HttpRequest> {
    let arenas = cadenza_ast::codec::decode(bytes)?;
    let rec = unascribe(&arenas, arenas.root); // `Request` ctor is elided → the (possibly ascribed) record
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
    let rec = unascribe(&arenas, arenas.root); // `Response` ctor is elided → the (possibly ascribed) record
    let status = u16::try_from(read_uint(&arenas, record_field(&arenas, rec, "status")?)?).ok()?;
    let headers = read_headers(&arenas, record_field(&arenas, rec, "headers")?)?;
    let body = read_bytes(&arenas, record_field(&arenas, rec, "body")?)?;
    Some(HttpResponse {
        status,
        headers,
        body,
    })
}

/// Decode an `http-route-table` frame into its [`RouteFrame`]s, or `None` if malformed. `RouteTable` is a
/// single-constructor sum wrapping `List(Route)`, so (after the optional root ascription) the value is the
/// `#list` of `Route` records directly; each `Route` is likewise a single-ctor record. Ascription-tolerant
/// (via `record_field`/`read_method`), so it reads both the platform-boundary form (root ascribed only) and
/// the guest `Value.encode` form (every node ascribed).
#[must_use]
pub fn decode_route_table(bytes: &[u8]) -> Option<Vec<RouteFrame>> {
    let arenas = cadenza_ast::codec::decode(bytes)?;
    let list = unascribe(&arenas, arenas.root); // `RouteTable` ctor elided → the `#list` of routes
    read_list(&arenas, list)?
        .iter()
        .map(|&r| {
            Some(RouteFrame {
                method: read_method(&arenas, record_field(&arenas, r, "method")?)?,
                path: read_str(&arenas, record_field(&arenas, r, "path")?)?,
                handler: read_bytes(&arenas, record_field(&arenas, r, "handler")?)?,
                contract: read_bytes(&arenas, record_field(&arenas, r, "contract")?)?,
            })
        })
        .collect()
}

/// Decode a canonical binary-AST payload into a [`WsEvent`], or `None` if malformed. `Event` is a
/// multi-ctor sum → `(<Ctor> #record…)` (after the optional root ascription); ascription-tolerant per field.
#[must_use]
pub fn decode_ws_event(bytes: &[u8]) -> Option<WsEvent> {
    let arenas = cadenza_ast::codec::decode(bytes)?;
    let ev = unascribe(&arenas, arenas.root); // `(<Ctor> #record)` — a multi-ctor variant is not elided
    let Struct::List(items) = arenas.get(ev) else {
        return None;
    };
    let ctor = arenas.as_name(*items.first()?)?;
    let rec = *items.get(1)?;
    let conn = read_bytes(&arenas, record_field(&arenas, rec, "conn")?)?;
    match ctor {
        "Connect" => Some(WsEvent::Connect { conn }),
        "Disconnect" => Some(WsEvent::Disconnect { conn }),
        "Frame" => {
            let data = read_bytes(&arenas, record_field(&arenas, rec, "data")?)?;
            Some(WsEvent::Frame { conn, data })
        }
        _ => None,
    }
}

/// Decode a session's `ws-send` bytes into a [`WsSend`], or `None` if malformed. `Send` is single-ctor →
/// the record directly (after the optional ascription).
#[must_use]
pub fn decode_ws_send(bytes: &[u8]) -> Option<WsSend> {
    let arenas = cadenza_ast::codec::decode(bytes)?;
    let rec = unascribe(&arenas, arenas.root);
    Some(WsSend {
        conn: read_bytes(&arenas, record_field(&arenas, rec, "conn")?)?,
        data: read_bytes(&arenas, record_field(&arenas, rec, "data")?)?,
    })
}

/// Decode a `RouteQuery` envelope into its `(request, table)` byte payloads, or `None` if malformed — the
/// inverse of [`encode_route_query`]. `RouteQuery` is single-ctor → the record directly (after the optional
/// ascription).
#[must_use]
pub fn decode_route_query(bytes: &[u8]) -> Option<(Bytes, Bytes)> {
    let arenas = cadenza_ast::codec::decode(bytes)?;
    let rec = unascribe(&arenas, arenas.root);
    let request = read_bytes(&arenas, record_field(&arenas, rec, "request")?)?;
    let table = read_bytes(&arenas, record_field(&arenas, rec, "table")?)?;
    Some((request, table))
}

/// Decode a [`ControlUp`] envelope, or `None` if malformed — the inverse of [`encode_control_up`].
#[must_use]
pub fn decode_control_up(bytes: &[u8]) -> Option<ControlUp> {
    let arenas = cadenza_ast::codec::decode(bytes)?;
    let rec = unascribe(&arenas, arenas.root);
    Some(ControlUp {
        program: read_bytes(&arenas, record_field(&arenas, rec, "program")?)?,
        session: read_bytes(&arenas, record_field(&arenas, rec, "session")?)?,
        payload: read_bytes(&arenas, record_field(&arenas, rec, "payload")?)?,
    })
}

/// Decode a [`ControlDown`] envelope, or `None` if malformed — the inverse of [`encode_control_down`].
#[must_use]
pub fn decode_control_down(bytes: &[u8]) -> Option<ControlDown> {
    let arenas = cadenza_ast::codec::decode(bytes)?;
    let rec = unascribe(&arenas, arenas.root);
    Some(ControlDown {
        session: read_bytes(&arenas, record_field(&arenas, rec, "session")?)?,
        payload: read_bytes(&arenas, record_field(&arenas, rec, "payload")?)?,
    })
}

/// Decode a [`ControlConfig`] boot-config frame, or `None` if malformed — the inverse of
/// [`encode_control_config`]. Single-ctor → the record directly (after the optional ascription).
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

/// Decode the router governing program's closing-`Break` reason bytes into a [`RouteDecision`], or `None` if
/// malformed. `Decision` is single-ctor → the record directly (after the optional ascription); an empty
/// `handler` (the no-match sentinel) decodes to a `RouteDecision` whose [`is_match`](RouteDecision::is_match)
/// is `false`.
#[must_use]
pub fn decode_decision(bytes: &[u8]) -> Option<RouteDecision> {
    let arenas = cadenza_ast::codec::decode(bytes)?;
    let rec = unascribe(&arenas, arenas.root);
    Some(RouteDecision {
        handler: read_bytes(&arenas, record_field(&arenas, rec, "handler")?)?,
        contract: read_bytes(&arenas, record_field(&arenas, rec, "contract")?)?,
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

/// Strip an optional ascription `(: <value> <ty>)`, returning the inner value (or `id` unchanged if not
/// ascribed). The guest `Value.encode` ascribes EVERY constructed node with its type — the top-level value
/// AND each nested one (e.g. every `Header` list element is `(: #record… Header)`) — whereas the platform's
/// payload-boundary `encode_ascribed` ascribes only the root. Reading either form means being tolerant of an
/// ascription anywhere a value/record is expected, so this is applied wherever a field/element is read.
fn unascribe(arenas: &Arenas, id: StructId) -> StructId {
    as_ascribed(arenas, id).unwrap_or(id)
}

/// The value of a record's field named `name`, via the native-record reader. Strips an optional ascription
/// on `id` first (a `Value.encode`d record node is `(: #record… Ty)`), so it reads both the bare and the
/// ascribed record form.
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

/// The [`Method`] of a `(Ctor unit)` value — the head constructor name, mapped to a method. Strips an
/// optional ascription (`Value.encode` renders the variant as `(: (Ctor unit) Method)`).
fn read_method(arenas: &Arenas, id: StructId) -> Option<Method> {
    match arenas.get(unascribe(arenas, id)) {
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

    fn sample_route_table() -> Vec<RouteFrame> {
        vec![
            RouteFrame {
                method: Method::Get,
                path: "/".to_string(),
                handler: Bytes::from_static(b"h1"),
                contract: Bytes::from_static(b"c1"),
            },
            RouteFrame {
                method: Method::Post,
                path: "/mcp".to_string(),
                handler: Bytes::from_static(b"h2"),
                contract: Bytes::from_static(b"c2"),
            },
        ]
    }

    #[test]
    fn route_table_round_trips() {
        let table = sample_route_table();
        let decoded = decode_route_table(&encode_route_table(&table)).expect("route table decodes");
        assert_eq!(decoded, table);
    }

    #[test]
    fn empty_route_table_round_trips() {
        let decoded = decode_route_table(&encode_route_table(&[])).expect("empty table decodes");
        assert!(decoded.is_empty());
    }

    /// CROSS-COMPILER PIN: decode a route-table frame the actual compiler produced (`cdz run` over a literal
    /// `RouteTable`), proving the codec matches the compiler's `Value.encode`.
    #[test]
    fn decodes_compiler_produced_route_table() {
        let table = decode_route_table(include_bytes!("../tests/fixtures/route-table.bin"))
            .expect("compiler-produced route table decodes");
        assert_eq!(table, sample_route_table());
    }

    #[test]
    fn malformed_route_table_is_none() {
        assert!(decode_route_table(b"garbage").is_none());
        // A response's bytes are not a route table (no per-route records) → None.
        assert!(decode_route_table(&encode_response(&sample_response())).is_none());
    }

    #[test]
    fn ws_event_round_trips() {
        for ev in [
            WsEvent::Connect {
                conn: Bytes::from_static(b"c1"),
            },
            WsEvent::Frame {
                conn: Bytes::from_static(b"c1"),
                data: Bytes::from_static(b"hello ws"),
            },
            WsEvent::Disconnect {
                conn: Bytes::from_static(b"c1"),
            },
        ] {
            let decoded = decode_ws_event(&encode_ws_event(&ev)).expect("ws-event decodes");
            assert_eq!(decoded, ev);
        }
    }

    #[test]
    fn ws_send_round_trips() {
        let send = WsSend {
            conn: Bytes::from_static(b"c1"),
            data: Bytes::from_static(b"pong"),
        };
        assert_eq!(decode_ws_send(&encode_ws_send(&send)).unwrap(), send);
    }

    #[test]
    fn route_query_round_trips() {
        let request = encode_request(&sample_request());
        let table = encode_route_table(&sample_route_table());
        let (r, t) = decode_route_query(&encode_route_query(&request, &table)).expect("decodes");
        assert_eq!(r, request);
        assert_eq!(t, table);
        // The embedded payloads still decode as their own forms after the round-trip through the envelope.
        assert!(decode_request(&r).is_some());
        assert_eq!(decode_route_table(&t).unwrap(), sample_route_table());
    }

    #[test]
    fn control_config_round_trips() {
        let config = ControlConfig {
            cas_url: "https://cas.example.internal:8443/blobs".to_string(),
            cas_credential: Bytes::from_static(b"bearer-token-abc123"),
            root_router: Bytes::from_static(b"cdz-router.root................."),
        };
        let decoded =
            decode_control_config(&encode_control_config(&config)).expect("config decodes");
        assert_eq!(decoded, config);
        // An empty credential (dev / no-auth CAS) still round-trips.
        let no_auth = ControlConfig {
            cas_url: "http://localhost:9000".to_string(),
            cas_credential: Bytes::new(),
            root_router: Bytes::from_static(b"cdz-router.root................."),
        };
        assert_eq!(
            decode_control_config(&encode_control_config(&no_auth)).unwrap(),
            no_auth
        );
    }

    #[test]
    fn dispatch_effect_round_trips() {
        let d = DispatchEffect {
            subprogram: Bytes::from_static(b"cdz-http.handler.echo............"),
            input: encode_request(&sample_request()),
        };
        let decoded = decode_dispatch(&encode_dispatch(&d)).expect("dispatch decodes");
        assert_eq!(decoded, d);
        assert!(
            decode_request(&decoded.input).is_some(),
            "the input still decodes"
        );
        assert!(decode_dispatch(b"garbage").is_none());
    }

    #[test]
    fn control_envelopes_round_trip() {
        let up = ControlUp {
            program: Bytes::from_static(b"cdz-router.root................."),
            session: Bytes::from_static(b"conn-42"),
            payload: Bytes::from_static(b"opaque handler->control bytes"),
        };
        assert_eq!(decode_control_up(&encode_control_up(&up)).unwrap(), up);
        let down = ControlDown {
            session: Bytes::from_static(b"conn-42"),
            payload: Bytes::from_static(b"opaque control->handler push"),
        };
        assert_eq!(
            decode_control_down(&encode_control_down(&down)).unwrap(),
            down
        );
        // Empty payload/session (edge cases) still round-trip.
        let empty = ControlDown {
            session: Bytes::new(),
            payload: Bytes::new(),
        };
        assert_eq!(
            decode_control_down(&encode_control_down(&empty)).unwrap(),
            empty
        );
    }

    #[test]
    fn a_malformed_control_envelope_is_none() {
        assert!(decode_control_up(b"garbage").is_none());
        assert!(decode_control_down(b"garbage").is_none());
        // A down envelope lacks `program`, so it is not a valid up envelope.
        let down = encode_control_down(&ControlDown {
            session: Bytes::from_static(b"s"),
            payload: Bytes::from_static(b"p"),
        });
        assert!(decode_control_up(&down).is_none());
    }

    #[test]
    fn a_malformed_control_config_is_none() {
        assert!(decode_control_config(b"not a config").is_none());
        // A route table is not a config (missing the config fields) → None.
        assert!(decode_control_config(&encode_route_table(&sample_route_table())).is_none());
    }

    #[test]
    fn route_decision_round_trips() {
        // A match: a non-empty handler + contract.
        let matched = RouteDecision {
            handler: Bytes::from_static(b"cdz-http.handler.root............"),
            contract: Bytes::from_static(b"cdz-platform.http.request........"),
        };
        let decoded = decode_decision(&encode_decision(&matched)).expect("decision decodes");
        assert_eq!(decoded, matched);
        assert!(decoded.is_match());

        // No match: the empty-handler sentinel decodes back and reads as not-a-match.
        let no_match = RouteDecision {
            handler: Bytes::new(),
            contract: Bytes::new(),
        };
        let decoded = decode_decision(&encode_decision(&no_match)).expect("sentinel decodes");
        assert_eq!(decoded, no_match);
        assert!(!decoded.is_match());
    }

    /// CROSS-COMPILER PIN: decode ws-event / ws-send frames the actual compiler produced (`cdz run` over
    /// literal `Event.Frame` / `Send.Send` values), proving the codec matches the compiler's value form.
    #[test]
    fn decodes_compiler_produced_ws_frames() {
        assert_eq!(
            decode_ws_event(include_bytes!("../tests/fixtures/ws-event.bin")).expect("ws-event"),
            WsEvent::Frame {
                conn: Bytes::from_static(b"c1"),
                data: Bytes::from_static(b"hello"),
            }
        );
        assert_eq!(
            decode_ws_send(include_bytes!("../tests/fixtures/ws-send.bin")).expect("ws-send"),
            WsSend {
                conn: Bytes::from_static(b"c1"),
                data: Bytes::from_static(b"pong"),
            }
        );
    }

    #[test]
    fn malformed_ws_is_none() {
        assert!(decode_ws_event(b"garbage").is_none());
        assert!(decode_ws_send(&[]).is_none());
        // A ws-send (a bare record) is not a ws-event (needs a `(Ctor record)` head).
        let send = encode_ws_send(&WsSend {
            conn: Bytes::from_static(b"c"),
            data: Bytes::from_static(b"d"),
        });
        assert!(decode_ws_event(&send).is_none());
    }
}
