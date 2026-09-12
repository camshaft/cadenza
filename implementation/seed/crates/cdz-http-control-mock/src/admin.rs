//! The driver ↔ mock **admin protocol** — binary-AST Cadenza values, NOT JSON (operator: "i want cadenza
//! ast everywhere"; the standing binary-AST-is-THE-exchange-format rule). The driver sends an
//! [`AdminCommand`] value to inject state or query observations; the mock answers with an [`AdminReply`]
//! value. Both encode with the shared [`cdz_http_protocol::value`] toolkit — one codec across the harness.
//!
//! Composite sub-frames are carried as their already-encoded bytes (a `Bytes` leaf): an observed
//! [`ControlUp`](cdz_http_protocol::ControlUp) rides in an [`AdminReply::ControlUps`] as the bytes
//! `cdz_http_protocol::encode_control_up` produced, so the driver decodes each with `decode_control_up` —
//! no need to re-model those frames here.

use bytes::Bytes;
use cdz_http_protocol::value;
use cdz_str::Str;

/// A command the driver sends the mock to INJECT state or QUERY observations. A multi-constructor sum on the
/// wire: `(<Ctor> <record>)` (or `(Reset unit)` for the nullary variant), root-ascribed `AdminCommand`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdminCommand {
    /// Seed the program manifest: bind a program NAME to its `ProgramHash` bytes, so later commands can
    /// refer to it by name. The driver (which built the programs via nix) seeds these at scenario setup.
    SetProgram { name: Str, hash: Bytes },
    /// Set the root-router program the mock ships on connect, by manifest NAME (the mock resolves it).
    SetRootRouter { program: Str },
    /// Live-swap: push a new root-router program (by NAME) to connected gateway sessions.
    PushRootRouter { program: Str },
    /// Prime how the mock replies to a matching `control.send`: match on request `path` (a `None` matches
    /// any), reply with `reply` bytes (the `correlation`/`session` are echoed from the matched up).
    PrimeReply {
        match_path: Option<Str>,
        reply: Bytes,
    },
    /// Push an unsolicited `ControlDown` to a session (delivered as an `on_notification`).
    PushDown { session: Bytes, payload: Bytes },
    /// Clear injected config + captured observations (per-scenario isolation).
    Reset,
    /// Query: return the captured `ControlUp` envelopes.
    GetControlUps,
    /// Close every live gateway control-ws session (server-initiated), forcing the gateway to REDIAL — for
    /// testing the control-link reconnect/resilience path. On redial the mock ships the current `ControlConfig`
    /// again (the normal on-connect behavior), so a healthy gateway recovers.
    DropControl,
    /// Send an UNDECODABLE (malformed) control frame to every live gateway session — for testing the gateway's
    /// control-frame robustness: it must tolerate garbage on the control link (ignore/recover, never crash/hang).
    PushGarbageFrame,
}

/// The mock's answer to an [`AdminCommand`]. A multi-constructor sum, root-ascribed `AdminReply`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdminReply {
    /// An inject command succeeded.
    Ok,
    /// A command failed (e.g. an unresolvable program name); `message` is human-readable.
    Error { message: Str },
    /// The captured `ControlUp`s, each as the bytes `encode_control_up` produced (decode with
    /// `decode_control_up`), oldest first.
    ControlUps { ups: Vec<Bytes> },
}

// --- encode --------------------------------------------------------------------------------------------

/// Encode an [`AdminCommand`] to its binary-AST bytes.
#[must_use]
pub fn encode_command(cmd: &AdminCommand) -> Bytes {
    let mut b = value::ValueBuilder::new();
    let node = match cmd {
        AdminCommand::SetProgram { name, hash } => {
            let n = value::str_leaf(&mut b, name);
            let h = value::bytes_leaf(&mut b, hash);
            let rec = value::record(&mut b, vec![("hash", h), ("name", n)]);
            value::bare_ctor(&mut b, "SetProgram", vec![rec])
        }
        AdminCommand::SetRootRouter { program } => {
            let p = value::str_leaf(&mut b, program);
            let rec = value::record(&mut b, vec![("program", p)]);
            value::bare_ctor(&mut b, "SetRootRouter", vec![rec])
        }
        AdminCommand::PushRootRouter { program } => {
            let p = value::str_leaf(&mut b, program);
            let rec = value::record(&mut b, vec![("program", p)]);
            value::bare_ctor(&mut b, "PushRootRouter", vec![rec])
        }
        AdminCommand::PrimeReply { match_path, reply } => {
            // An absent match is the empty string sentinel (matches any path); the decoder maps "" -> None.
            let path = value::str_leaf(&mut b, match_path.as_deref().unwrap_or(""));
            let reply = value::bytes_leaf(&mut b, reply);
            let rec = value::record(&mut b, vec![("match-path", path), ("reply", reply)]);
            value::bare_ctor(&mut b, "PrimeReply", vec![rec])
        }
        AdminCommand::PushDown { session, payload } => {
            let s = value::bytes_leaf(&mut b, session);
            let p = value::bytes_leaf(&mut b, payload);
            let rec = value::record(&mut b, vec![("payload", p), ("session", s)]);
            value::bare_ctor(&mut b, "PushDown", vec![rec])
        }
        AdminCommand::Reset => {
            let u = value::unit(&mut b);
            value::bare_ctor(&mut b, "Reset", vec![u])
        }
        AdminCommand::GetControlUps => {
            let u = value::unit(&mut b);
            value::bare_ctor(&mut b, "GetControlUps", vec![u])
        }
        AdminCommand::DropControl => {
            let u = value::unit(&mut b);
            value::bare_ctor(&mut b, "DropControl", vec![u])
        }
        AdminCommand::PushGarbageFrame => {
            let u = value::unit(&mut b);
            value::bare_ctor(&mut b, "PushGarbageFrame", vec![u])
        }
    };
    value::finish_value(b, node)
}

/// Encode an [`AdminReply`] to its binary-AST bytes.
#[must_use]
pub fn encode_reply(reply: &AdminReply) -> Bytes {
    let mut b = value::ValueBuilder::new();
    let node = match reply {
        AdminReply::Ok => {
            let u = value::unit(&mut b);
            value::bare_ctor(&mut b, "Ok", vec![u])
        }
        AdminReply::Error { message } => {
            let m = value::str_leaf(&mut b, message);
            let rec = value::record(&mut b, vec![("message", m)]);
            value::bare_ctor(&mut b, "Error", vec![rec])
        }
        AdminReply::ControlUps { ups } => {
            let elems: Vec<_> = ups.iter().map(|u| value::bytes_leaf(&mut b, u)).collect();
            let list = value::list_value(&mut b, elems);
            let rec = value::record(&mut b, vec![("ups", list)]);
            value::bare_ctor(&mut b, "ControlUps", vec![rec])
        }
    };
    value::finish_value(b, node)
}

// --- decode --------------------------------------------------------------------------------------------

/// Decode an [`AdminCommand`], or `None` if malformed.
#[must_use]
pub fn decode_command(bytes: &[u8]) -> Option<AdminCommand> {
    let arenas = value::decode(bytes)?;
    let root = arenas.root;
    let ctor = value::read_ctor(&arenas, root)?;
    let rec = *value::ctor_payload(&arenas, root)?.first()?;
    Some(match ctor {
        "SetProgram" => AdminCommand::SetProgram {
            name: read_str(&arenas, value::record_field(&arenas, rec, "name")?)?,
            hash: value::read_bytes(&arenas, value::record_field(&arenas, rec, "hash")?)?,
        },
        "SetRootRouter" => AdminCommand::SetRootRouter {
            program: read_str(&arenas, value::record_field(&arenas, rec, "program")?)?,
        },
        "PushRootRouter" => AdminCommand::PushRootRouter {
            program: read_str(&arenas, value::record_field(&arenas, rec, "program")?)?,
        },
        "PrimeReply" => {
            let path = read_str(&arenas, value::record_field(&arenas, rec, "match-path")?)?;
            AdminCommand::PrimeReply {
                match_path: (!path.is_empty()).then_some(path),
                reply: value::read_bytes(&arenas, value::record_field(&arenas, rec, "reply")?)?,
            }
        }
        "PushDown" => AdminCommand::PushDown {
            session: value::read_bytes(&arenas, value::record_field(&arenas, rec, "session")?)?,
            payload: value::read_bytes(&arenas, value::record_field(&arenas, rec, "payload")?)?,
        },
        "Reset" => AdminCommand::Reset,
        "GetControlUps" => AdminCommand::GetControlUps,
        "DropControl" => AdminCommand::DropControl,
        "PushGarbageFrame" => AdminCommand::PushGarbageFrame,
        _ => return None,
    })
}

/// Decode an [`AdminReply`], or `None` if malformed.
#[must_use]
pub fn decode_reply(bytes: &[u8]) -> Option<AdminReply> {
    let arenas = value::decode(bytes)?;
    let root = arenas.root;
    let ctor = value::read_ctor(&arenas, root)?;
    Some(match ctor {
        "Ok" => AdminReply::Ok,
        "Error" => {
            let rec = *value::ctor_payload(&arenas, root)?.first()?;
            AdminReply::Error {
                message: read_str(&arenas, value::record_field(&arenas, rec, "message")?)?,
            }
        }
        "ControlUps" => {
            let rec = *value::ctor_payload(&arenas, root)?.first()?;
            let list = value::record_field(&arenas, rec, "ups")?;
            let ups = value::read_list(&arenas, list)?
                .iter()
                .map(|&e| value::read_bytes(&arenas, e))
                .collect::<Option<Vec<_>>>()?;
            AdminReply::ControlUps { ups }
        }
        _ => return None,
    })
}

/// A `Str` from a string leaf (the value toolkit's `read_str` yields `String`; the admin frames want `Str`).
fn read_str(arenas: &value::Arenas, id: value::ValueId) -> Option<Str> {
    value::read_str(arenas, id).map(Str::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_round_trips() {
        let cmds = [
            AdminCommand::SetProgram {
                name: Str::from("router-hello"),
                hash: Bytes::from_static(b"a-33-byte-program-hash-goes-here."),
            },
            AdminCommand::SetRootRouter {
                program: Str::from("router-hello"),
            },
            AdminCommand::PushRootRouter {
                program: Str::from("router-echo"),
            },
            AdminCommand::PrimeReply {
                match_path: Some(Str::from("/emit")),
                reply: Bytes::from_static(b"PONG"),
            },
            AdminCommand::PrimeReply {
                match_path: None,
                reply: Bytes::from_static(b"any"),
            },
            AdminCommand::PushDown {
                session: Bytes::from_static(b"sess-1"),
                payload: Bytes::from_static(b"push"),
            },
            AdminCommand::Reset,
            AdminCommand::GetControlUps,
        ];
        for cmd in cmds {
            let decoded = decode_command(&encode_command(&cmd)).expect("command decodes");
            assert_eq!(decoded, cmd, "command did not round-trip: {cmd:?}");
        }
    }

    #[test]
    fn every_reply_round_trips() {
        let replies = [
            AdminReply::Ok,
            AdminReply::Error {
                message: Str::from("unresolvable program 'nope'"),
            },
            AdminReply::ControlUps {
                ups: vec![
                    Bytes::from_static(b"encoded-up-1"),
                    Bytes::from_static(b"encoded-up-2"),
                ],
            },
            AdminReply::ControlUps { ups: vec![] },
        ];
        for reply in replies {
            let decoded = decode_reply(&encode_reply(&reply)).expect("reply decodes");
            assert_eq!(decoded, reply, "reply did not round-trip: {reply:?}");
        }
    }

    #[test]
    fn a_captured_control_up_survives_the_admin_envelope() {
        // The ControlUps reply carries each up as the bytes encode_control_up produced, so the driver
        // recovers the full frame (handler id + correlation + request context) after the round-trip.
        let up = cdz_http_protocol::ControlUp {
            program: Bytes::from_static(b"handler-emitter"),
            session: Bytes::from_static(b"sess-1"),
            correlation: Bytes::from_static(b"corr-1"),
            payload: Bytes::from_static(b"ping"),
            request: cdz_http_protocol::RequestContext {
                method: Str::from("POST"),
                path: Str::from("/emit"),
                headers: vec![],
            },
        };
        let reply = AdminReply::ControlUps {
            ups: vec![cdz_http_protocol::encode_control_up(&up)],
        };
        let AdminReply::ControlUps { ups } = decode_reply(&encode_reply(&reply)).unwrap() else {
            panic!("expected ControlUps");
        };
        let recovered = cdz_http_protocol::decode_control_up(&ups[0]).expect("inner up decodes");
        assert_eq!(recovered, up);
    }

    #[test]
    fn malformed_admin_frames_are_none() {
        assert!(decode_command(b"garbage").is_none());
        assert!(decode_reply(&[]).is_none());
        // A reply is not a command (wrong ctor set).
        assert!(decode_command(&encode_reply(&AdminReply::Ok)).is_none());
    }
}
