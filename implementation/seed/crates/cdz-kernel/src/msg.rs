//! Messaging over the log (agent-runtime L2a) — a message is a typed, durable, addressed EVENT.
//!
//! Vision §9: everything the fleet does is messaging (merge-request → reject → assign → ask → answer → …),
//! and it is a first-class *proven* pillar, not a nice-to-have. A [`Message`] is an addressed, typed,
//! durable event `{from, to, kind, subject, refs[], body}`; an [`Ack`] marks a source message processed.
//! Both ride the L1 [`crate::Log`] as an [`crate::Event`] whose `kind` is `"message"`/`"ack"` and whose
//! `payload` is the encoding below — so the inbox (L2b) is a fold over these events, not a queue.
//!
//! L2a is the type + its ENCODING: a pure, dependency-free, binary-safe codec to/from the event payload
//! bytes (the same length-prefixed discipline as `file_log`'s records + `dynamo_log`'s marshalling), so it
//! is unit-tested with no log/network. L2b folds these into an inbox projection; L2c does reply-then-ack.

use crate::Seq;
use anyhow::{anyhow, Result};

/// The event `kind` tag for a message event (a [`Message`] in its payload).
pub const MESSAGE: &str = "message";
/// The event `kind` tag for an ack event (an [`Ack`] in its payload).
pub const ACK: &str = "ack";

/// An addressed, typed, durable message — the fleet's coordination primitive as a log event (vision §9).
/// `kind` is the fleet's earned vocabulary (`merge-request`, `reject`, `assign`, `ask`, `answer`, `note`,
/// …) kept as a string (open, matching the fleet + the L1 `Event.kind`); `refs` are the shas/branches a
/// message is about; `body` is free-form (binary-safe — a completion or diff may contain any bytes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub from: String,
    pub to: String,
    pub kind: String,
    pub subject: String,
    pub refs: Vec<String>,
    pub body: Vec<u8>,
}

/// An acknowledgement that the message at `message_seq` was processed — the "mark processed" of the
/// inbox-as-projection (vision §9). A separate event so reply-then-ack is crash-safe (L2c): the reply
/// message is appended before this ack; a crash between leaves the source un-acked (re-driven).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ack {
    pub message_seq: Seq,
}

impl Message {
    /// Encode to the [`crate::Event`] payload bytes: `from, to, kind, subject` as length-prefixed strings,
    /// then `refs` (a u32 count + each length-prefixed), then `body` (length-prefixed). Binary-safe +
    /// dependency-free — the inverse of [`Message::decode`].
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_str(&mut out, &self.from);
        put_str(&mut out, &self.to);
        put_str(&mut out, &self.kind);
        put_str(&mut out, &self.subject);
        put_u32(&mut out, self.refs.len() as u32);
        for r in &self.refs {
            put_str(&mut out, r);
        }
        put_bytes(&mut out, &self.body);
        out
    }

    /// Decode a [`Message`] from an event payload produced by [`Message::encode`]. Errors on a truncated /
    /// malformed payload (the log is the source of truth — a bad decode must be loud, not a silent empty).
    pub fn decode(bytes: &[u8]) -> Result<Message> {
        let mut i = 0usize;
        let from = get_str(bytes, &mut i)?;
        let to = get_str(bytes, &mut i)?;
        let kind = get_str(bytes, &mut i)?;
        let subject = get_str(bytes, &mut i)?;
        let n = get_u32(bytes, &mut i)? as usize;
        let mut refs = Vec::with_capacity(n);
        for _ in 0..n {
            refs.push(get_str(bytes, &mut i)?);
        }
        let body = get_bytes(bytes, &mut i)?;
        if i != bytes.len() {
            return Err(anyhow!(
                "message payload has {} trailing bytes after decode",
                bytes.len() - i
            ));
        }
        Ok(Message {
            from,
            to,
            kind,
            subject,
            refs,
            body,
        })
    }
}

impl Ack {
    /// Encode to the event payload: the source message's `seq` as a little-endian u64.
    pub fn encode(&self) -> Vec<u8> {
        self.message_seq.to_le_bytes().to_vec()
    }

    /// Decode an [`Ack`] from an event payload produced by [`Ack::encode`].
    pub fn decode(bytes: &[u8]) -> Result<Ack> {
        let arr: [u8; 8] = bytes.try_into().map_err(|_| {
            anyhow!(
                "ack payload is not an 8-byte seq (got {} bytes)",
                bytes.len()
            )
        })?;
        Ok(Ack {
            message_seq: Seq::from_le_bytes(arr),
        })
    }
}

// ── length-prefixed codec helpers (u32-LE lengths; binary-safe; no deps) ────────────────────────────────

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    put_u32(out, b.len() as u32);
    out.extend_from_slice(b);
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    put_bytes(out, s.as_bytes());
}

fn get_u32(bytes: &[u8], i: &mut usize) -> Result<u32> {
    let end = *i + 4;
    let slice = bytes
        .get(*i..end)
        .ok_or_else(|| anyhow!("truncated message: expected a 4-byte length at offset {i}"))?;
    let v = u32::from_le_bytes(slice.try_into().expect("slice is exactly 4 bytes"));
    *i = end;
    Ok(v)
}

fn get_bytes(bytes: &[u8], i: &mut usize) -> Result<Vec<u8>> {
    let len = get_u32(bytes, i)? as usize;
    let end = *i + len;
    let slice = bytes
        .get(*i..end)
        .ok_or_else(|| anyhow!("truncated message: expected {len} bytes at offset {i}"))?;
    *i = end;
    Ok(slice.to_vec())
}

fn get_str(bytes: &[u8], i: &mut usize) -> Result<String> {
    let b = get_bytes(bytes, i)?;
    String::from_utf8(b).map_err(|e| anyhow!("message field not valid UTF-8: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_round_trips_all_fields() {
        // A full fleet-shaped message (the earned vocabulary + a ref) round-trips exactly.
        let m = Message {
            from: "v-agent-harness".into(),
            to: "pr-sync".into(),
            kind: "merge-request".into(),
            subject: "L2a: messaging types".into(),
            refs: vec!["52622a761".into()],
            body: b"one commit on cdz-kernel".to_vec(),
        };
        assert_eq!(
            Message::decode(&m.encode()).unwrap(),
            m,
            "message encode/decode is identity"
        );
    }

    #[test]
    fn message_round_trips_multiple_refs_and_empty_and_binary() {
        // Multiple refs, empty fields, and a binary body (NUL/high bytes/newlines) all survive the codec.
        let m = Message {
            from: "".into(),
            to: "b".into(),
            kind: "note".into(),
            subject: "".into(),
            refs: vec!["a".into(), "b".into(), "c".into()],
            body: vec![0u8, 255, b'"', b'\n', 200],
        };
        assert_eq!(Message::decode(&m.encode()).unwrap(), m);
        // No-refs case too.
        let m0 = Message {
            from: "x".into(),
            to: "y".into(),
            kind: "ack".into(),
            subject: "s".into(),
            refs: vec![],
            body: vec![],
        };
        assert_eq!(Message::decode(&m0.encode()).unwrap(), m0);
    }

    #[test]
    fn ack_round_trips() {
        let a = Ack { message_seq: 42 };
        assert_eq!(Ack::decode(&a.encode()).unwrap(), a);
        assert_eq!(
            Ack::decode(&Ack { message_seq: 0 }.encode())
                .unwrap()
                .message_seq,
            0
        );
    }

    #[test]
    fn decode_rejects_a_truncated_payload() {
        // A truncated message payload is a loud error, not a silent partial — the log is the source of truth.
        let full = Message {
            from: "a".into(),
            to: "b".into(),
            kind: "note".into(),
            subject: "s".into(),
            refs: vec![],
            body: b"hi".to_vec(),
        }
        .encode();
        assert!(
            Message::decode(&full[..full.len() - 3]).is_err(),
            "a truncated message must not decode"
        );
        assert!(
            Ack::decode(&[0u8, 1, 2]).is_err(),
            "a non-8-byte ack payload must not decode"
        );
    }

    #[test]
    fn message_kind_tags_are_stable() {
        assert_eq!((MESSAGE, ACK), ("message", "ack"));
    }
}
