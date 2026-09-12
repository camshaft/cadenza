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
use cadenza_ast::ast::Builder;
use cdz_str::Str;
use serde::{Deserialize, Serialize};

/// The reusable binary-AST value-form toolkit (builders + readers). Exposed so downstream harness crates
/// (the mock control server's admin protocol, the driver) build binary-AST frames with ONE codec.
///
/// UNCHANGED by the serde migration: the five control frames now (de)serialize through
/// [`cadenza_ast_serde`], but this toolkit stays public + intact because the mock control server uses it
/// directly for its own admin protocol. lib.rs itself now only uses it for the [`FrameCodec`] envelope.
pub mod value;
use value::{bytes_leaf, finish_value, read_bytes, record, record_field, unascribe};

// --- the Rust mirrors of the control-plane frames --------------------------------------------------------

/// One HTTP header (`name`, `value`) — reused in the request context a [`ControlUp`] carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub name: Str,
    pub value: Str,
}

/// As much of the originating HTTP request as the gateway attaches to a [`ControlUp`], so the control
/// server can route a handler's `control.send` correctly without re-parsing the opaque payload. `method`
/// is the request method as a plain string (`"GET"`, `"POST"`, …); `path` the request path; `headers` the
/// request headers the gateway chose to forward.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlDown {
    /// The target session id the gateway routes this message to.
    pub session: Bytes,
    /// The correlation token: matches a pending `ControlUp` (⇒ a response) or is empty (⇒ a push).
    pub correlation: Bytes,
    /// The control server's opaque message payload (never inspected by the gateway).
    pub payload: Bytes,
}

// --- encode --------------------------------------------------------------------------------------------

// The three frames now (de)serialize through `cadenza-ast-serde`: the `#[derive(Serialize, Deserialize)]`
// on each struct maps it to/from the canonical binary-AST (a `("record" (= "field" v)…)` value), so the
// hand-written value-form builders are gone. The nested `RequestContext` / `Vec<Header>` are handled by
// their own derives (a record and a `("list" …)`). serde serialization of these plain data structs is
// infallible, so `encode_*` keep their `-> Bytes` signatures (the `.expect` can never fire).

/// Encode a [`ControlConfig`] boot frame to its canonical binary-AST value (via `cadenza-ast-serde`).
#[must_use]
pub fn encode_control_config(config: &ControlConfig) -> Bytes {
    Bytes::from(cadenza_ast_serde::to_bytes(config).expect("ControlConfig serializes"))
}

/// Encode a [`ControlUp`] envelope (handler → control) to its canonical binary-AST value.
#[must_use]
pub fn encode_control_up(msg: &ControlUp) -> Bytes {
    Bytes::from(cadenza_ast_serde::to_bytes(msg).expect("ControlUp serializes"))
}

/// Encode a [`ControlDown`] envelope (control → handler) to its canonical binary-AST value.
#[must_use]
pub fn encode_control_down(msg: &ControlDown) -> Bytes {
    Bytes::from(cadenza_ast_serde::to_bytes(msg).expect("ControlDown serializes"))
}

// --- decode --------------------------------------------------------------------------------------------

/// Decode a [`ControlConfig`], or `None` if malformed — the inverse of [`encode_control_config`].
#[must_use]
pub fn decode_control_config(bytes: &[u8]) -> Option<ControlConfig> {
    cadenza_ast_serde::from_bytes(bytes).ok()
}

/// Decode a [`ControlUp`], or `None` if malformed — the inverse of [`encode_control_up`].
#[must_use]
pub fn decode_control_up(bytes: &[u8]) -> Option<ControlUp> {
    cadenza_ast_serde::from_bytes(bytes).ok()
}

/// Decode a [`ControlDown`], or `None` if malformed — the inverse of [`encode_control_down`].
#[must_use]
pub fn decode_control_down(bytes: &[u8]) -> Option<ControlDown> {
    cadenza_ast_serde::from_bytes(bytes).ok()
}

// --- frame dispatch (contract-hash-tagged envelope) ------------------------------------------------------
// The control link is a single bidirectional message stream carrying ALL three frame types. Rather than
// disambiguate by record SHAPE (fragile — two frames could share a shape), each frame travels inside an
// envelope tagged with its contract-id, and the receiver dispatches by that tag (operator directive: "the
// control-link protocol should use contract hashes for dispatch so the receiver dispatches by content type").
//
// The tag values ARE the canonical COMPUTED contract-ids — the same ones a Cadenza guest derives from the
// contract's `descriptor().id`, and that the userspace-contract codegen (§5, #8673) emits for Rust as
// `contracts::{control_config,control_up,control_down}::contract().id()`. This crate stays dep-minimal
// (cadenza-ast + bytes + cdz-str — NOT the heavy cdz-platform runtime; see the Cargo.toml header), so the
// ids are INJECTED by the consumer that already reaches them (the mock control server + the gateway both dep
// cdz-platform). The codec is therefore id-agnostic + fully testable in isolation, and forward-compatible
// with a later light `cdz-platform-contracts` extraction: only the id SOURCE moves, never this envelope.

/// One control-link message: exactly one of the three frames. Encoded/decoded through a [`FrameCodec`],
/// which tags it with its contract-id so the peer dispatches by content type, not by guessing at shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlFrame {
    /// A boot / reconfigure config (control → gateway).
    Config(ControlConfig),
    /// A handler's `control.send` on its way up (gateway → control).
    Up(ControlUp),
    /// A response or unsolicited push (control → gateway).
    Down(ControlDown),
}

/// The contract-hash-tagged frame codec. Holds the three canonical computed contract-ids (the frame tags),
/// injected by the consumer from `contracts::{control_config,control_up,control_down}::contract().id()`
/// (the crate stays dep-minimal + id-agnostic — see the section note). Cheap to `Clone` (three `Bytes`).
#[derive(Debug, Clone)]
pub struct FrameCodec {
    config_id: Bytes,
    up_id: Bytes,
    down_id: Bytes,
}

impl FrameCodec {
    /// A codec tagging frames with the given canonical contract-ids. Each id is the 33-byte `ContractId` of
    /// the corresponding frame contract (`cdz-platform.control.{config,up,down}`) — supply
    /// `contracts::control_config::contract().id()` (etc.) as `Bytes`.
    #[must_use]
    pub fn new(config_id: Bytes, up_id: Bytes, down_id: Bytes) -> Self {
        Self {
            config_id,
            up_id,
            down_id,
        }
    }

    /// Encode a [`ControlFrame`] into a tagged envelope: a record `{ contract, payload }` (fields
    /// name-sorted, structural — no type ascription, per the ascription-free codec) where `contract` is the
    /// frame's contract-id and `payload` is the frame's own encoded value (from `encode_control_*`). One
    /// nesting; no re-encode. Demux is by the `contract` TAG, never by shape or a type name.
    #[must_use]
    pub fn encode(&self, frame: &ControlFrame) -> Bytes {
        let (id, payload) = match frame {
            ControlFrame::Config(c) => (&self.config_id, encode_control_config(c)),
            ControlFrame::Up(u) => (&self.up_id, encode_control_up(u)),
            ControlFrame::Down(d) => (&self.down_id, encode_control_down(d)),
        };
        let mut b = Builder::new();
        let contract = bytes_leaf(&mut b, id);
        let payload = bytes_leaf(&mut b, &payload);
        let rec = record(&mut b, vec![("contract", contract), ("payload", payload)]);
        finish_value(b, rec)
    }

    /// Decode a tagged envelope, dispatching by the `contract` tag to the matching frame decoder. `None` if
    /// the envelope is malformed, the tag matches none of the three known frame contract-ids (an unknown /
    /// future frame type), or the tagged payload fails to decode as that frame. Dispatch is by TAG, never by
    /// the payload's shape.
    #[must_use]
    pub fn decode(&self, bytes: &[u8]) -> Option<ControlFrame> {
        let arenas = value::decode(bytes)?;
        let rec = unascribe(&arenas, arenas.root);
        let contract = read_bytes(&arenas, record_field(&arenas, rec, "contract")?)?;
        let payload = read_bytes(&arenas, record_field(&arenas, rec, "payload")?)?;
        if contract == self.config_id {
            decode_control_config(&payload).map(ControlFrame::Config)
        } else if contract == self.up_id {
            decode_control_up(&payload).map(ControlFrame::Up)
        } else if contract == self.down_id {
            decode_control_down(&payload).map(ControlFrame::Down)
        } else {
            None
        }
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

    // --- frame dispatch ---
    // Three distinct 33-byte test tags (real ids are `contracts::control_*::contract().id()`; the codec is
    // id-agnostic, so synthetic tags exercise it fully).
    const CONFIG_TAG: &[u8; 33] = b"test.control.config.tag..........";
    const UP_TAG: &[u8; 33] = b"test.control.up.tag..............";
    const DOWN_TAG: &[u8; 33] = b"test.control.down.tag............";

    fn test_codec() -> FrameCodec {
        FrameCodec::new(
            Bytes::from_static(CONFIG_TAG),
            Bytes::from_static(UP_TAG),
            Bytes::from_static(DOWN_TAG),
        )
    }

    #[test]
    fn every_frame_round_trips_through_the_tagged_envelope() {
        let codec = test_codec();
        let frames = [
            ControlFrame::Config(ControlConfig {
                cas_url: Str::from("http://cas.local:9000"),
                cas_credential: Bytes::from_static(b"tok"),
                root_router: Bytes::from_static(b"cdz-router.root................."),
            }),
            ControlFrame::Up(ControlUp {
                program: Bytes::from_static(b"h"),
                session: Bytes::from_static(b"s"),
                correlation: Bytes::from_static(b"c"),
                payload: Bytes::from_static(b"opaque"),
                request: sample_request_context(),
            }),
            ControlFrame::Down(ControlDown {
                session: Bytes::from_static(b"s"),
                correlation: Bytes::from_static(b"c"),
                payload: Bytes::from_static(b"PONG"),
            }),
        ];
        for frame in frames {
            assert_eq!(codec.decode(&codec.encode(&frame)), Some(frame));
        }
    }

    #[test]
    fn dispatch_is_by_tag_not_by_shape() {
        // A codec that does not know the DOWN tag cannot decode a Down frame the first codec produced — even
        // though the payload is a perfectly well-formed ControlDown. Dispatch keys on the tag, so an unknown
        // tag is `None` (a future / foreign frame type), never a mis-decode.
        let codec = test_codec();
        let down = ControlFrame::Down(ControlDown {
            session: Bytes::from_static(b"s"),
            correlation: Bytes::new(),
            payload: Bytes::from_static(b"push"),
        });
        let wire = codec.encode(&down);
        let stranger = FrameCodec::new(
            Bytes::from_static(CONFIG_TAG),
            Bytes::from_static(UP_TAG),
            Bytes::from_static(b"a.completely.different.down.tag.."),
        );
        assert_eq!(stranger.decode(&wire), None);
        // The original codec still round-trips it.
        assert_eq!(codec.decode(&wire), Some(down));
    }

    #[test]
    fn a_malformed_envelope_is_none() {
        let codec = test_codec();
        assert!(codec.decode(b"not an envelope").is_none());
        // A bare frame value (no envelope) has no `contract` field → None.
        let bare = encode_control_down(&ControlDown {
            session: Bytes::from_static(b"s"),
            correlation: Bytes::new(),
            payload: Bytes::new(),
        });
        assert!(codec.decode(&bare).is_none());
    }
}
