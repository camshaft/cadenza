//! The admin control interface's WIRE CODEC — how an [`AdminCommand`]/[`AdminResponse`] crosses a control
//! frame between an admin client and the running daemon.
//!
//! The domain types ([`AdminCommand`]/[`AdminResponse`]/[`InstallSpec`] in [`crate::admin`]) are kept
//! serde-free on purpose: they hold a [`SessionId`] (a genesis [`Hash`](struct@Hash), which is not
//! `Serialize`) and a reducer [`Hash`](struct@Hash) (likewise). Rather than push serde requirements onto the
//! domain types (and onto the kernel's `Hash`), the wire layer mirrors them with small serde-native DTOs —
//! ids as their canonical hex `String`, the reducer hash as its canonical hex — and converts. That decoupling
//! makes the WIRE CONTRACT explicit and versionable (it's what an external admin client encodes against), and
//! keeps the domain types transport-agnostic.
//!
//! The frame codec is length-prefixed JSON: a 4-byte big-endian `u32` length followed by that many bytes
//! of the JSON body. That framing is what a stream transport (the Unix-domain-socket listener, the next
//! slice) needs to know where one command/response ends — a bare JSON stream has no message boundary. This
//! module is transport-free: it encodes/decodes to/from `Vec<u8>` + a byte reader, so it's hermetically
//! testable (the socket that carries the frames is a following slice, and the codec is exercised by
//! `cargo test` in the default build regardless).

use crate::admin::{AdminCommand, AdminResponse, InstallSpec};
use crate::host::SessionId;
use cdz_kernel::hash::Hash;
use serde::{Deserialize, Serialize};

/// The wire form of an [`InstallSpec`]: a plain-`String` id + the reducer hash as canonical hex + the
/// optional goal. Decoupled from the domain [`InstallSpec`] so the domain type needs no serde derive and
/// the kernel's non-`Serialize` [`Hash`](struct@Hash) rides as hex.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallSpecWire {
    pub id: String,
    /// The reducer component's content hash as 64 lowercase hex chars ([`Hash::to_hex`]).
    pub reducer_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
}

/// The wire form of an [`AdminCommand`] — an internally-tagged JSON object (`{"cmd":"install-session",…}`)
/// so the shape is self-describing + stable for an external client to author against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum AdminCommandWire {
    InstallSession(InstallSpecWire),
    ListSessions,
    SessionStatus { id: String },
    StopSession { id: String },
}

/// The wire form of an [`AdminResponse`] — internally tagged on `result` (`{"result":"installed","id":…}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "kebab-case")]
pub enum AdminResponseWire {
    Installed {
        id: String,
    },
    Sessions {
        ids: Vec<String>,
    },
    /// The status body is the [`crate::status::session_status_json`] object, carried inline as parsed JSON
    /// (not a re-escaped string) so the whole response is one clean JSON document.
    Status {
        status: serde_json::Value,
    },
    Stopped {
        id: String,
    },
    Error {
        message: String,
    },
}

/// A wire-codec error: an unparseable frame, a bad JSON body, or a hash that isn't canonical hex. Stringly
/// typed — the transport layer surfaces it (a malformed admin frame is answered with an error response /
/// dropped, never a panic).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireError(pub String);

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "admin wire error: {}", self.0)
    }
}
impl std::error::Error for WireError {}

/// The frame length ceiling — a defensive bound so a malformed/hostile length prefix can't make the reader
/// attempt a huge allocation. An admin command/response is small (a hash, an id, a status object); 1 MiB is
/// far above any legitimate frame.
pub const MAX_FRAME_LEN: usize = 1024 * 1024;

// ── domain ⇄ wire conversions ────────────────────────────────────────────────────────────────────────

/// Parse a wire session-id HEX back to a [`SessionId`] (the genesis [`Hash`]). A non-canonical-hex id is a
/// [`WireError`] — a malformed admin frame is rejected, never silently coerced. The hex↔Hash conversion lives
/// ONLY here at the wire edge; the domain [`SessionId`] is the raw `Hash`.
fn session_id_from_hex(id: &str) -> Result<SessionId, WireError> {
    Hash::from_hex(id).map(SessionId::new).ok_or_else(|| {
        WireError(format!(
            "session id is not canonical hex (64 lowercase): {id:?}"
        ))
    })
}

impl From<&InstallSpec> for InstallSpecWire {
    fn from(spec: &InstallSpec) -> Self {
        InstallSpecWire {
            id: spec.id.to_hex(),
            reducer_hash: spec.reducer_hash.to_hex(),
            goal: spec.goal.clone(),
        }
    }
}

impl InstallSpecWire {
    /// Convert back to the domain [`InstallSpec`], parsing the hex hash. `Err` if the hash isn't canonical
    /// (64 lowercase hex chars) — a malformed install frame is rejected, not silently coerced.
    pub fn to_domain(&self) -> Result<InstallSpec, WireError> {
        let reducer_hash = Hash::from_hex(&self.reducer_hash).ok_or_else(|| {
            WireError(format!(
                "reducer_hash is not canonical hex (64 lowercase): {:?}",
                self.reducer_hash
            ))
        })?;
        Ok(InstallSpec {
            // The wire `id` is the session-id HEX (a SessionId is the genesis Hash; hex only at this edge).
            id: session_id_from_hex(&self.id)?,
            reducer_hash,
            goal: self.goal.clone(),
        })
    }
}

impl From<&AdminCommand> for AdminCommandWire {
    fn from(cmd: &AdminCommand) -> Self {
        match cmd {
            AdminCommand::InstallSession(spec) => AdminCommandWire::InstallSession(spec.into()),
            AdminCommand::ListSessions => AdminCommandWire::ListSessions,
            AdminCommand::SessionStatus { id } => {
                AdminCommandWire::SessionStatus { id: id.to_hex() }
            }
            AdminCommand::StopSession { id } => AdminCommandWire::StopSession { id: id.to_hex() },
        }
    }
}

impl AdminCommandWire {
    /// Convert to the domain [`AdminCommand`], parsing any embedded hash. `Err` on a malformed install spec.
    pub fn to_domain(&self) -> Result<AdminCommand, WireError> {
        Ok(match self {
            AdminCommandWire::InstallSession(spec) => {
                AdminCommand::InstallSession(spec.to_domain()?)
            }
            AdminCommandWire::ListSessions => AdminCommand::ListSessions,
            AdminCommandWire::SessionStatus { id } => AdminCommand::SessionStatus {
                id: session_id_from_hex(id)?,
            },
            AdminCommandWire::StopSession { id } => AdminCommand::StopSession {
                id: session_id_from_hex(id)?,
            },
        })
    }
}

impl AdminResponseWire {
    /// Build a wire response from a domain [`AdminResponse`]. The `Status` variant's JSON string is PARSED
    /// into a `serde_json::Value` so the wire response is one clean document (not a JSON string nested
    /// inside JSON); a status body that somehow isn't valid JSON degrades to an `Error` rather than
    /// producing a malformed frame.
    pub fn from_domain(resp: &AdminResponse) -> Self {
        match resp {
            // Domain ids are SessionId (a genesis Hash); the wire carries them as canonical hex String — the
            // hex↔SessionId conversion lives here at the transport boundary, not in the domain type.
            AdminResponse::Installed { id } => AdminResponseWire::Installed { id: id.to_hex() },
            AdminResponse::Sessions { ids } => AdminResponseWire::Sessions {
                ids: ids.iter().map(|s| s.to_hex()).collect(),
            },
            AdminResponse::Status { json } => match serde_json::from_str(json) {
                Ok(status) => AdminResponseWire::Status { status },
                Err(e) => AdminResponseWire::Error {
                    message: format!("status body was not valid JSON: {e}"),
                },
            },
            AdminResponse::Stopped { id } => AdminResponseWire::Stopped { id: id.to_hex() },
            AdminResponse::Error { message } => AdminResponseWire::Error {
                message: message.clone(),
            },
        }
    }
}

// ── length-prefixed JSON frame codec ─────────────────────────────────────────────────────────────────

/// Encode a serializable wire value as a length-prefixed JSON frame: a 4-byte big-endian `u32` length
/// followed by the JSON body. The inverse of [`decode_frame`]. `Err` only if serialization itself fails
/// (which, for these fixed DTOs, doesn't happen in practice) or the body exceeds [`MAX_FRAME_LEN`].
pub fn encode_frame<T: Serialize>(value: &T) -> Result<Vec<u8>, WireError> {
    let body = serde_json::to_vec(value).map_err(|e| WireError(format!("encode failed: {e}")))?;
    if body.len() > MAX_FRAME_LEN {
        return Err(WireError(format!(
            "frame body {} exceeds MAX_FRAME_LEN {MAX_FRAME_LEN}",
            body.len()
        )));
    }
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// Decode one length-prefixed JSON frame from the front of `buf`, returning the decoded value and the
/// number of bytes consumed (so a caller draining a stream buffer can advance past it). `Err` if the
/// declared length exceeds [`MAX_FRAME_LEN`] or if the body isn't valid JSON for `T`.
///
/// Returns `Ok(None)` when `buf` doesn't yet hold a full frame (fewer than 4 header bytes, or fewer than
/// the declared body length) — the "need more bytes" signal a streaming reader loops on (NOT an `Err`: a
/// short buffer is an incomplete read to retry, not a malformed frame). Only an oversized declared length
/// or a full-but-unparseable body is `Err`.
pub fn decode_frame<T: for<'de> Deserialize<'de>>(
    buf: &[u8],
) -> Result<Option<(T, usize)>, WireError> {
    if buf.len() < 4 {
        return Ok(None); // not even the length header yet
    }
    let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if len > MAX_FRAME_LEN {
        return Err(WireError(format!(
            "declared frame length {len} exceeds MAX_FRAME_LEN {MAX_FRAME_LEN}"
        )));
    }
    let end = 4 + len;
    if buf.len() < end {
        return Ok(None); // header present, body not fully arrived yet
    }
    let value = serde_json::from_slice(&buf[4..end])
        .map_err(|e| WireError(format!("frame body was not valid JSON: {e}")))?;
    Ok(Some((value, end)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: &str) -> InstallSpec {
        InstallSpec {
            id: SessionId::new(Hash::of(id.as_bytes())),
            reducer_hash: Hash::of(id.as_bytes()),
            goal: Some("do the thing".into()),
        }
    }

    #[test]
    fn command_domain_wire_round_trips_through_json() {
        // Every AdminCommand → wire → JSON bytes → wire → domain is identity (the hash survives via hex).
        for cmd in [
            AdminCommand::InstallSession(spec("worker")),
            AdminCommand::ListSessions,
            AdminCommand::SessionStatus {
                id: SessionId::new(Hash::of(b"s1")),
            },
            AdminCommand::StopSession {
                id: SessionId::new(Hash::of(b"victim")),
            },
        ] {
            let wire = AdminCommandWire::from(&cmd);
            let json = serde_json::to_vec(&wire).unwrap();
            let back: AdminCommandWire = serde_json::from_slice(&json).unwrap();
            assert_eq!(back, wire, "wire JSON round-trips");
            assert_eq!(back.to_domain().unwrap(), cmd, "wire → domain is identity");
        }
    }

    #[test]
    fn install_command_json_has_the_expected_shape() {
        // Pin the wire contract an external admin client authors against.
        let wire = AdminCommandWire::from(&AdminCommand::InstallSession(spec("w")));
        let json: serde_json::Value = serde_json::to_value(&wire).unwrap();
        assert_eq!(json["cmd"], "install-session");
        assert_eq!(json["id"], SessionId::new(Hash::of(b"w")).to_hex());
        assert_eq!(json["reducer_hash"], Hash::of(b"w").to_hex());
        assert_eq!(json["goal"], "do the thing");
    }

    #[test]
    fn list_command_is_a_bare_tagged_object() {
        let wire = AdminCommandWire::from(&AdminCommand::ListSessions);
        let json: serde_json::Value = serde_json::to_value(&wire).unwrap();
        assert_eq!(json, serde_json::json!({"cmd": "list-sessions"}));
    }

    #[test]
    fn remaining_command_tags_are_pinned_to_their_wire_literals() {
        // Golden-pin the two command tags the shape tests above DON'T cover (install-session + list-sessions
        // are pinned; session-status + stop-session were only exercised symbolically by the round-trip test,
        // which moves both sides together — a `rename_all`/variant rename would silently drift the tag an
        // external admin client authors against and the round-trip would still pass). Pin the literal `cmd`
        // string (+ the id field) so any wire-tag drift is a loud failure.
        let status = serde_json::to_value(AdminCommandWire::from(&AdminCommand::SessionStatus {
            id: SessionId::new(Hash::of(b"s1")),
        }))
        .unwrap();
        assert_eq!(
            status,
            serde_json::json!({"cmd": "session-status", "id": SessionId::new(Hash::of(b"s1")).to_hex()})
        );

        let stop = serde_json::to_value(AdminCommandWire::from(&AdminCommand::StopSession {
            id: SessionId::new(Hash::of(b"victim")),
        }))
        .unwrap();
        assert_eq!(
            stop,
            serde_json::json!({"cmd": "stop-session", "id": SessionId::new(Hash::of(b"victim")).to_hex()})
        );
    }

    #[test]
    fn response_tags_are_pinned_to_their_wire_literals() {
        // Golden-pin the response `result` tags (only `status` was pinned above). These are the wire contract
        // an external admin client PARSES; a rename would silently break clients with the round-trip green.
        let installed =
            serde_json::to_value(AdminResponseWire::from_domain(&AdminResponse::Installed {
                id: SessionId::new(Hash::of(b"w")),
            }))
            .unwrap();
        assert_eq!(
            installed,
            serde_json::json!({"result": "installed", "id": SessionId::new(Hash::of(b"w")).to_hex()})
        );

        let sessions =
            serde_json::to_value(AdminResponseWire::from_domain(&AdminResponse::Sessions {
                ids: vec![
                    SessionId::new(Hash::of(b"a")),
                    SessionId::new(Hash::of(b"b")),
                ],
            }))
            .unwrap();
        assert_eq!(
            sessions,
            serde_json::json!({"result": "sessions", "ids": [
                SessionId::new(Hash::of(b"a")).to_hex(),
                SessionId::new(Hash::of(b"b")).to_hex(),
            ]})
        );

        let stopped =
            serde_json::to_value(AdminResponseWire::from_domain(&AdminResponse::Stopped {
                id: SessionId::new(Hash::of(b"gone")),
            }))
            .unwrap();
        assert_eq!(
            stopped,
            serde_json::json!({"result": "stopped", "id": SessionId::new(Hash::of(b"gone")).to_hex()})
        );

        let error = serde_json::to_value(AdminResponseWire::from_domain(&AdminResponse::Error {
            message: "boom".into(),
        }))
        .unwrap();
        assert_eq!(
            error,
            serde_json::json!({"result": "error", "message": "boom"})
        );
    }

    #[test]
    fn a_goalless_install_omits_the_goal_field() {
        let wire = AdminCommandWire::from(&AdminCommand::InstallSession(InstallSpec {
            id: SessionId::new(Hash::of(b"w")),
            reducer_hash: Hash::of(b"w"),
            goal: None,
        }));
        let json: serde_json::Value = serde_json::to_value(&wire).unwrap();
        assert!(json.get("goal").is_none(), "goal:None is skipped: {json}");
    }

    #[test]
    fn a_noncanonical_reducer_hash_is_rejected() {
        let wire = AdminCommandWire::InstallSession(InstallSpecWire {
            id: "w".into(),
            reducer_hash: "not-hex".into(),
            goal: None,
        });
        let err = wire.to_domain().unwrap_err();
        assert!(err.0.contains("canonical hex"), "{err}");
    }

    #[test]
    fn a_noncanonical_session_id_is_rejected_on_every_id_carrying_command() {
        // Fail-closed wire id parsing (the sessionid-hash sweep): a command's session `id` is the peer's
        // genesis-hash HEX, parsed back via session_id_from_hex. A non-canonical id (vanity label, non-hex,
        // wrong length) is a WireError — a malformed admin frame is REJECTED at the transport edge, never
        // silently coerced into a bogus SessionId. Pins it on all three id-carrying commands (install spec id,
        // session-status, stop-session). The reducer_hash reject above covers the OTHER hash on install; the
        // valid reducer_hash here isolates the FAILURE to the id so install's id path is genuinely exercised
        // (reducer_hash is parsed first, so a bad-id install needs a good reducer_hash to reach the id check).
        let good_reducer = Hash::of(b"reducer").to_hex();
        for bad in ["victim", "not-hex", "deadbeef" /* too short */, ""] {
            let install = AdminCommandWire::InstallSession(InstallSpecWire {
                id: bad.into(),
                reducer_hash: good_reducer.clone(),
                goal: None,
            });
            assert!(
                matches!(install.to_domain(), Err(WireError(m)) if m.contains("canonical hex")),
                "install with a non-hex id {bad:?} is a WireError"
            );
            assert!(
                matches!(AdminCommandWire::SessionStatus { id: bad.into() }.to_domain(),
                    Err(WireError(m)) if m.contains("canonical hex")),
                "session-status with a non-hex id {bad:?} is a WireError"
            );
            assert!(
                matches!(AdminCommandWire::StopSession { id: bad.into() }.to_domain(),
                    Err(WireError(m)) if m.contains("canonical hex")),
                "stop-session with a non-hex id {bad:?} is a WireError"
            );
        }
    }

    #[test]
    fn response_status_is_inlined_as_parsed_json_not_a_nested_string() {
        // A Status response carries the session_status_json object inline as JSON, so the whole frame is one
        // clean document (the status is an object, not a re-escaped string).
        let resp = AdminResponse::Status {
            json: r#"{"session_id":"s1","state":"Quiescent","errored":false}"#.into(),
        };
        let wire = AdminResponseWire::from_domain(&resp);
        let json: serde_json::Value = serde_json::to_value(&wire).unwrap();
        assert_eq!(json["result"], "status");
        assert_eq!(json["status"]["session_id"], "s1");
        assert_eq!(json["status"]["state"], "Quiescent");
        assert_eq!(json["status"]["errored"], false);
    }

    #[test]
    fn response_with_an_invalid_status_body_degrades_to_error_not_a_bad_frame() {
        let resp = AdminResponse::Status {
            json: "this is not json".into(),
        };
        let wire = AdminResponseWire::from_domain(&resp);
        assert!(
            matches!(wire, AdminResponseWire::Error { message } if message.contains("not valid JSON"))
        );
    }

    #[test]
    fn frame_encode_decode_round_trips() {
        let wire = AdminCommandWire::from(&AdminCommand::InstallSession(spec("w")));
        let frame = encode_frame(&wire).unwrap();
        // The header is the big-endian body length.
        let declared = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
        assert_eq!(declared, frame.len() - 4);
        let (back, consumed): (AdminCommandWire, usize) =
            decode_frame(&frame).unwrap().expect("a full frame decodes");
        assert_eq!(back, wire);
        assert_eq!(consumed, frame.len());
    }

    #[test]
    fn decode_returns_none_until_the_full_frame_has_arrived() {
        // The streaming "need more bytes" signal: a partial header, then a partial body, both yield None;
        // the completed buffer yields the value.
        let wire = AdminCommandWire::from(&AdminCommand::ListSessions);
        let frame = encode_frame(&wire).unwrap();
        assert_eq!(
            decode_frame::<AdminCommandWire>(&frame[..2]).unwrap(),
            None,
            "partial header → None"
        );
        assert_eq!(
            decode_frame::<AdminCommandWire>(&frame[..frame.len() - 1]).unwrap(),
            None,
            "header but truncated body → None"
        );
        assert!(
            decode_frame::<AdminCommandWire>(&frame).unwrap().is_some(),
            "full frame → Some"
        );
    }

    #[test]
    fn decode_advances_past_one_frame_leaving_the_next() {
        // Two frames concatenated (a stream): decoding consumes exactly the first, and the returned offset
        // lets the caller decode the second from the remainder.
        let f1 = encode_frame(&AdminCommandWire::from(&AdminCommand::ListSessions)).unwrap();
        let f2 = encode_frame(&AdminCommandWire::from(&AdminCommand::StopSession {
            id: SessionId::new(Hash::of(b"x")),
        }))
        .unwrap();
        let mut buf = f1.clone();
        buf.extend_from_slice(&f2);

        let (first, consumed): (AdminCommandWire, usize) = decode_frame(&buf).unwrap().unwrap();
        assert_eq!(first, AdminCommandWire::ListSessions);
        assert_eq!(consumed, f1.len());

        let (second, _): (AdminCommandWire, usize) =
            decode_frame(&buf[consumed..]).unwrap().unwrap();
        assert_eq!(
            second,
            AdminCommandWire::StopSession {
                id: SessionId::new(Hash::of(b"x")).to_hex()
            }
        );
    }

    #[test]
    fn an_oversized_declared_length_is_rejected() {
        // A hostile/garbage length prefix above MAX_FRAME_LEN is an Err, not a huge allocation attempt.
        let mut buf = ((MAX_FRAME_LEN as u32) + 1).to_be_bytes().to_vec();
        buf.extend_from_slice(b"whatever");
        let err = decode_frame::<AdminCommandWire>(&buf).unwrap_err();
        assert!(err.0.contains("exceeds MAX_FRAME_LEN"), "{err}");
    }

    #[test]
    fn a_full_frame_with_garbage_body_is_an_error() {
        // Header declares a body length that's present, but the body isn't valid JSON for the type → Err
        // (distinct from the None "need more bytes" case).
        let body = b"not json at all";
        let mut buf = (body.len() as u32).to_be_bytes().to_vec();
        buf.extend_from_slice(body);
        let err = decode_frame::<AdminCommandWire>(&buf).unwrap_err();
        assert!(err.0.contains("not valid JSON"), "{err}");
    }
}
