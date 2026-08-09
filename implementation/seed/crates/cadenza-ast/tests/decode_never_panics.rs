//! `codec::decode` is a TOTAL function on arbitrary bytes: it must return `None`/`Err` on any
//! malformed, truncated, or hostile input — never panic, never overflow the stack, never loop.
//!
//! `decode` parses UNTRUSTED transport bytes (a component's embedded AST, a peer's schema payload), so
//! "no input crashes the decoder" is a real security/robustness invariant, distinct from the codec's
//! hand-targeted per-`DecodeError`-variant tests (those pin that a SPECIFIC corruption maps to a
//! SPECIFIC error; this pins that the WHOLE input space is panic-free). A deterministic sweep — no
//! RNG (unavailable/non-reproducible in this harness) — over structured adversarial byte families plus
//! every truncation-prefix and single-byte-flip of a valid encoding.

use cadenza_ast::ast::{Builder, StructId};
use cadenza_ast::codec::{decode, decode_detailed, encode};

/// The canonical schema header — the format's fixed magic (`cdzast\x00\x01`). Kept literal here (the
/// codec's own `SCHEMA_HEADER` const is private) so a hostile "valid header, garbage body" family can
/// be built.
const HEADER: [u8; 8] = *b"cdzast\x00\x01";

/// A small, valid arena: `(f a 1)` — two names, one int leaf, one list. Its `encode` is the seed for
/// the truncation and bit-flip families.
fn sample_encoding() -> Vec<u8> {
    let mut b = Builder::new();
    let f = b.name("f");
    let a = b.name("a");
    let one = b.name("1"); // a name leaf is enough; the point is a multi-node arena
    let root = b.list(vec![f, a, one]);
    let arenas = b.finish(root);
    encode(&arenas)
}

/// Decoding must not panic; return whether it produced a value (for the callers that assert success).
fn decode_is_total(bytes: &[u8]) -> bool {
    // Both entry points must be total. `decode` delegates to `decode_detailed().ok()`, but pin both so
    // a future refactor that adds a panic to either surface is caught.
    let a = decode(bytes);
    let b = decode_detailed(bytes);
    a.is_some() == b.is_ok()
}

#[test]
fn decode_is_total_on_structured_adversarial_families() {
    let mut inputs: Vec<Vec<u8>> = Vec::new();

    // Degenerate lengths.
    inputs.push(vec![]);
    inputs.push(vec![0x00]);
    inputs.push(HEADER.to_vec()); // header only, no body

    // Valid header + a hostile/garbage body of varied shapes.
    for tail in [
        vec![0xff; 4],
        vec![0x80; 8], // a run of continuation bytes → overlong/never-terminating varint
        vec![0x7f, 0x7f, 0x7f], // huge-count-then-nothing
        vec![0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01], // a max-ish varint (claims a giant count)
        vec![0x00, 0x00], // 0 leaves, 0 structures → root read fails
    ] {
        let mut b = HEADER.to_vec();
        b.extend_from_slice(&tail);
        inputs.push(b);
    }

    // Wrong headers (right length, wrong magic) + near-miss headers.
    inputs.push(b"cdzast\x00\x02".to_vec()); // a future format version
    inputs.push(b"CDZAST\x00\x01".to_vec()); // wrong case
    inputs.push(vec![0xde, 0xad, 0xbe, 0xef, 0xde, 0xad, 0xbe, 0xef]);

    // A long incompressible-looking run.
    inputs.push((0u8..=255).cycle().take(1024).collect());

    for (i, inp) in inputs.iter().enumerate() {
        assert!(
            decode_is_total(inp),
            "decode disagreed with decode_detailed on adversarial input #{i} ({} bytes)",
            inp.len()
        );
        // The stronger claim: neither entry point panicked (reaching here proves it).
    }
}

#[test]
fn decode_is_total_on_every_truncation_prefix_of_a_valid_encoding() {
    // A truncated-mid-stream artifact (a partial download, a clipped payload) must decode to a clean
    // error at every cut point, never a panic. Sweep every prefix length 0..=len.
    let good = sample_encoding();
    assert!(decode(&good).is_some(), "the seed encoding decodes");
    for cut in 0..=good.len() {
        let prefix = &good[..cut];
        assert!(
            decode_is_total(prefix),
            "decode panicked/inconsistent on the {cut}-byte prefix"
        );
    }
    // Only the full-length prefix is a valid decode; every shorter one is an error (a prefix of a
    // canonical encoding is never itself canonical).
    for cut in 0..good.len() {
        assert!(
            decode(&good[..cut]).is_none(),
            "a {cut}-byte prefix of a valid encoding must not decode"
        );
    }
}

#[test]
fn decode_is_total_on_every_single_byte_flip_of_a_valid_encoding() {
    // A single corrupted byte anywhere in a valid artifact must yield a clean error or a still-valid
    // (different) arena — never a panic, stack overflow, or hang. Flip the high bit of each byte in
    // turn (a deterministic, structure-agnostic mutation that hits headers, tags, counts, and ids).
    let good = sample_encoding();
    for i in 0..good.len() {
        let mut m = good.clone();
        m[i] ^= 0x80;
        assert!(
            decode_is_total(&m),
            "decode panicked/inconsistent on a high-bit flip at byte {i}"
        );
    }
    // Also flip the low bit (catches tag/id boundary corruption the high bit can miss).
    for i in 0..good.len() {
        let mut m = good.clone();
        m[i] ^= 0x01;
        assert!(
            decode_is_total(&m),
            "decode panicked/inconsistent on a low-bit flip at byte {i}"
        );
    }
}

#[test]
fn decode_is_total_on_deeply_nested_and_wide_valid_encodings() {
    // A deeply-nested or very-wide valid arena must decode without overflowing the stack (the codec's
    // reachability/tree check is iterative for exactly this reason) — pin it as a round-trip.
    // Deep: a left-spine of nested single-child lists.
    let mut b = Builder::new();
    let leaf = b.name("x");
    let mut node: StructId = leaf;
    for _ in 0..2000 {
        node = b.list(vec![node]);
    }
    let deep = b.finish(node);
    let bytes = encode(&deep);
    assert!(
        decode(&bytes).is_some(),
        "a 2000-deep arena round-trips without overflow"
    );

    // Wide: one list with many children.
    let mut b2 = Builder::new();
    let kids: Vec<StructId> = (0..5000).map(|k| b2.name(&format!("n{k}"))).collect();
    let wide_root = b2.list(kids);
    let wide = b2.finish(wide_root);
    let wbytes = encode(&wide);
    assert!(decode(&wbytes).is_some(), "a 5000-wide arena round-trips");
}
