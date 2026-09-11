//! Building the control-link [`FrameCodec`](cdz_http_protocol::FrameCodec) for the mock from the 3 canonical
//! frame contract-ids. The gateway speaks TAGGED `ControlFrame` envelopes (operator tagged-frame directive), so
//! the mock must too — it tags/reads config/up/down by their `cdz-platform.control.{config,up,down}` ids.
//!
//! The mock is a LIGHT crate (no `cdz-platform` to compute the ids), so it takes them as env vars the harness
//! rig sets from the authoritative `contract-declarations` manifest (base62, the one text form). We decode
//! base62 → the raw 33 hash bytes here (the same fixed-width algorithm `cdz-contract`'s `Hash` uses — copied,
//! std-only, to keep the mock dep-minimal), so the mock's tags match the gateway's by construction.

use cdz_http_protocol::FrameCodec;

/// Env vars naming the 3 canonical frame contract-ids (base62), set by the harness rig from the
/// `contract-declarations` manifest.
pub const CONFIG_ID_VAR: &str = "CDZ_CONTROL_FRAME_CONFIG_ID";
pub const UP_ID_VAR: &str = "CDZ_CONTROL_FRAME_UP_ID";
pub const DOWN_ID_VAR: &str = "CDZ_CONTROL_FRAME_DOWN_ID";

/// A 33-byte tagged hash rendered as exactly 45 base62 chars (`cdz-contract`'s canonical text form).
const BYTES: usize = 33;
const CHARS: usize = 45;
const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// Decode a canonical base62 hash text (exactly [`CHARS`] alphabet chars) to its 33 raw bytes — the inverse of
/// `cdz-contract`'s `base62::encode` (treat as a big-endian 264-bit integer: `n = n*62 + digit`, big-endian
/// multiply-accumulate across 33 bytes). `None` on a bad length / char / a value that overflows 33 bytes.
#[must_use]
pub fn decode_base62(s: &str) -> Option<[u8; BYTES]> {
    let s = s.as_bytes();
    if s.len() != CHARS {
        return None;
    }
    let mut n = [0u8; BYTES];
    for &c in s {
        let digit = ALPHABET.iter().position(|&a| a == c)? as u32;
        // n = n * 62 + digit, big-endian with carry from the least-significant byte up.
        let mut carry = digit;
        for b in n.iter_mut().rev() {
            let acc = u32::from(*b) * 62 + carry;
            *b = (acc & 0xFF) as u8;
            carry = acc >> 8;
        }
        if carry != 0 {
            return None; // overflows 33 bytes — not a canonical id
        }
    }
    Some(n)
}

/// Build the [`FrameCodec`] from the 3 env-provided base62 frame ids. `None` if any var is unset or not a
/// valid base62 hash — the caller (the mock binary) treats that as a fatal misconfiguration.
#[must_use]
pub fn from_env() -> Option<FrameCodec> {
    let one = |var: &str| {
        let s = std::env::var(var).ok()?;
        decode_base62(&s).map(|b| bytes::Bytes::copy_from_slice(&b))
    };
    Some(FrameCodec::new(
        one(CONFIG_ID_VAR)?,
        one(UP_ID_VAR)?,
        one(DOWN_ID_VAR)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_http_protocol::{ControlDown, ControlFrame};

    #[test]
    fn decode_base62_rejects_bad_length_and_chars() {
        assert!(decode_base62("too-short").is_none());
        // 45 chars but a non-alphabet char ('!') → None.
        let bad = format!("{:!<45}", "");
        assert!(decode_base62(&bad).is_none());
    }

    #[test]
    fn decodes_the_canonical_control_frame_ids_to_33_bytes_and_round_trips_through_the_codec() {
        // The 3 canonical ids (from the contract-declarations manifest) decode to 33-byte tags, and a codec
        // built from them tags+reads a frame (proving the decoded bytes are a usable ContractId tag).
        let config = "01VmmnosWcpFWSdzmltJAzHbQGyLy3ZZBcnMz7O9EAGxM";
        let up = "01VDIqMGsAaORRtt2egN0DaA7gsEqp56JHavtf0ZLh86C";
        let down = "01Hpg8KHcUMQ6cZSgL63bzLBmwUHDp7sLqhBWix03hDyq";
        for id in [config, up, down] {
            assert_eq!(
                decode_base62(id).map(|b| b.len()),
                Some(33),
                "id {id} decodes to 33 bytes"
            );
        }
        let codec = FrameCodec::new(
            bytes::Bytes::copy_from_slice(&decode_base62(config).unwrap()),
            bytes::Bytes::copy_from_slice(&decode_base62(up).unwrap()),
            bytes::Bytes::copy_from_slice(&decode_base62(down).unwrap()),
        );
        let down_frame = ControlFrame::Down(ControlDown {
            session: bytes::Bytes::from_static(b"s"),
            correlation: bytes::Bytes::from_static(b"c"),
            payload: bytes::Bytes::from_static(b"PONG"),
        });
        let encoded = codec.encode(&down_frame);
        assert_eq!(
            codec.decode(&encoded),
            Some(down_frame),
            "tagged Down round-trips"
        );
    }
}
