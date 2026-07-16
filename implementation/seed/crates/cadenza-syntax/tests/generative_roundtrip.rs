//! Generative (property) round-trip: build RANDOM but well-formed programs from a grammar and assert
//! the ML surface round-trips each one — `read_ml(print(sexpr_program)).structurally_eq(program)` and
//! `print(reparse) == print(program)` (idempotence). This complements `corpus_roundtrip.rs`, whose
//! inputs are the FIXED corpus: a generator explores construct SHAPES and NESTINGS the corpus never
//! contains (deep infix chains under a match arm, a record whose field is an `if`, a call whose
//! argument is a tuple of lets, …), so a printer/parser asymmetry that no hand-written case happens to
//! hit still gets caught.
//!
//! Method (matching the crate's "plain" house style — no proptest/arbitrary dependency): a deterministic
//! SplitMix64 PRNG drives a recursive grammar that emits an s-expr STRING. The s-expr reader is the
//! independent ORACLE (a different code path from the ML reader/printer), and it only ever produces
//! VALID arenas — so every generated program is well-formed by construction, and the property under
//! test is purely "does the ML print→parse round-trip preserve the tree". Seeds are fixed, so a failure
//! reproduces exactly; the failing s-expr is printed for triage.

use cadenza_syntax::{codec, parser, printer, sexpr};

const WIDTH: usize = 100;

/// Deterministic SplitMix64 — reproducible generation without a dependency (mirrors the unit-test PRNGs
/// in `codec.rs`/`lexer.rs`).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
}

/// Generate a random s-expr EXPRESSION string. `depth` bounds recursion: at depth 0 only leaves are
/// produced, so generation always terminates. Every form emitted is one the ML surface has a spelling
/// for and round-trips (mirroring the constructs `corpus_roundtrip.rs` exercises); the s-expr reader
/// turns the string into a valid arena regardless.
fn gen_expr(rng: &mut Rng, depth: usize) -> String {
    // Leaves — always available; the only choice at depth 0.
    let names = ["a", "b", "x", "y", "f", "g", "foo", "bar"];
    let leaf = |rng: &mut Rng| -> String {
        match rng.below(5) {
            0 => rng.pick(&names).to_string(),
            1 => rng.below(1000).to_string(),   // int
            2 => format!("{}", rng.below(100)), // int (small)
            3 => "true".to_string(),
            _ => "false".to_string(),
        }
    };
    if depth == 0 {
        return leaf(rng);
    }
    // A recursive sub-expression at one less depth.
    let sub = |rng: &mut Rng| gen_expr(rng, depth - 1);
    match rng.below(12) {
        // leaf (bias toward leaves so trees stay finite-ish)
        0..=2 => leaf(rng),
        // infix arithmetic / comparison — a bare glyph head the ML surface prints infix
        3 => {
            let op = rng.pick(&["+", "-", "*", "<", "==", "|>"]);
            format!("({} {} {})", op, sub(rng), sub(rng))
        }
        // call: (f arg…) — a name head applied to 1–3 args
        4 => {
            let f = rng.pick(&names);
            let n = 1 + rng.below(3);
            let args: Vec<String> = (0..n).map(|_| sub(rng)).collect();
            format!("({} {})", f, args.join(" "))
        }
        // if
        5 => format!("(if {} {} {})", sub(rng), sub(rng), sub(rng)),
        // let: (let ((n v)…) body)
        6 => {
            let n = 1 + rng.below(2);
            let binds: Vec<String> = (0..n)
                .map(|i| format!("({} {})", ["p", "q", "r"][i], sub(rng)))
                .collect();
            format!("(let ({}) {})", binds.join(" "), sub(rng))
        }
        // list literal
        7 => {
            let n = rng.below(4);
            let elems: Vec<String> = (0..n).map(|_| sub(rng)).collect();
            format!("(\"list\" {})", elems.join(" "))
        }
        // tuple literal (≥2 elements — a 1-tuple is a grouping)
        8 => {
            let n = 2 + rng.below(2);
            let elems: Vec<String> = (0..n).map(|_| sub(rng)).collect();
            format!("(\"tuple\" {})", elems.join(" "))
        }
        // record literal: ("record" (field value)…)
        9 => {
            let n = 1 + rng.below(3);
            let fields: Vec<String> = (0..n)
                .map(|i| format!("({} {})", ["m", "n", "o"][i], sub(rng)))
                .collect();
            format!("(\"record\" {})", fields.join(" "))
        }
        // match: (match scrut (pat body)…) — patterns are simple names/literals/wildcards
        10 => {
            let n = 1 + rng.below(2);
            let arms: Vec<String> = (0..n)
                .map(|i| {
                    let pat = match i {
                        0 => rng.below(10).to_string(),
                        _ => "_".to_string(),
                    };
                    format!("({} {})", pat, sub(rng))
                })
                .collect();
            format!("(match {} {})", sub(rng), arms.join(" "))
        }
        // member access: (. obj field)
        _ => format!("(. {} {})", sub(rng), rng.pick(&names)),
    }
}

#[test]
fn ml_surface_round_trips_generated_programs() {
    // Sweep many independently-seeded programs across a range of depths. For each: read the generated
    // s-expr to the oracle arena, print it as ML, re-read the ML, and require the reparse (a) succeeds,
    // (b) is structurally equal to the oracle arena, and (c) is idempotent (printing the reparse is
    // byte-identical). A failure prints the generating s-expr + the ML for triage; fixed seeds mean it
    // reproduces exactly.
    let seeds: [u64; 4] = [
        0x0bad_c0de_dead_beef,
        0x5eed_1234_5678_9abc,
        0xfeed_face_cafe_babe,
        0x1357_9bdf_2468_ace0,
    ];
    let mut total = 0usize;
    for &seed in &seeds {
        let mut rng = Rng(seed);
        for _ in 0..1500 {
            let depth = 1 + rng.below(5); // depth 1..=5
            let src = gen_expr(&mut rng, depth);
            // Wrap as a definition body so the generated expression sits in a real program position
            // (a bare top-level expression is also valid, but a def exercises the statement path too).
            let program = format!("(def (main) {src})");
            let oracle = match sexpr::read(&program) {
                Ok(a) => a,
                // The generator only emits valid s-exprs; a read error is a generator bug, not a
                // round-trip failure — surface it.
                Err(e) => panic!(
                    "generator produced an unreadable s-expr: {program}\n  {}",
                    e.0
                ),
            };
            let ml = printer::print(&oracle, WIDTH);
            let reparsed = parser::read_ml(&ml);
            assert!(
                reparsed.ok(),
                "ML reparse FAILED\n  s-expr: {program}\n  ml:     {ml}\n  errs:   {:?}",
                reparsed.errors
            );
            assert!(
                reparsed.arenas.structurally_eq(&oracle),
                "ML round-trip changed the tree\n  s-expr: {program}\n  ml:     {ml}\n  reparsed: {}",
                sexpr::print(&reparsed.arenas)
            );
            assert_eq!(
                printer::print(&reparsed.arenas, WIDTH),
                ml,
                "ML print is not idempotent\n  s-expr: {program}\n  ml: {ml}"
            );
            total += 1;
        }
    }
    assert!(total >= 6000, "swept a meaningful space, got {total}");
}

#[test]
fn binary_and_all_surface_round_trip_generated_programs() {
    // The same generated programs, through the BINARY codec and the CROSS-surface paths. For each:
    //   * codec::decode(encode(oracle)) is structurally equal to the oracle (the bijection), and encode
    //     is a canonical fixed point (encode∘decode∘encode == encode);
    //   * ml→binary→ml is lossless (print ML, read it, encode, decode, print ML again — byte-identical,
    //     and structurally equal to the oracle);
    //   * sexpr→binary→sexpr reproduces the canonical s-expr text.
    // This complements `corpus_roundtrip.rs`'s binary/all-surface guards (fixed corpus) by exercising
    // the codec + conversion seams over generated shapes/nestings the corpus never contains. Distinct
    // seeds from the ML test so the two explore different programs.
    let seeds: [u64; 4] = [
        0x2468_ace0_1357_9bdf,
        0xdead_beef_0bad_c0de,
        0xcafe_babe_feed_face,
        0x9abc_5678_1234_5eed,
    ];
    let mut total = 0usize;
    for &seed in &seeds {
        let mut rng = Rng(seed);
        for _ in 0..1500 {
            let depth = 1 + rng.below(5);
            let src = gen_expr(&mut rng, depth);
            let program = format!("(def (main) {src})");
            let oracle = match sexpr::read(&program) {
                Ok(a) => a,
                Err(e) => panic!(
                    "generator produced an unreadable s-expr: {program}\n  {}",
                    e.0
                ),
            };

            // Binary: decode(encode) is structurally equal + encode is a canonical fixed point.
            let bytes = codec::encode(&oracle);
            let back = codec::decode(&bytes).expect("generated program's encoding decodes");
            assert!(
                back.structurally_eq(&oracle),
                "binary round-trip changed the tree\n  s-expr: {program}",
            );
            assert_eq!(
                codec::encode(&back),
                bytes,
                "encode is not a canonical fixed point\n  s-expr: {program}",
            );

            // sexpr → binary → sexpr reproduces the canonical s-expr text.
            let sx = sexpr::print(&oracle);
            let sx_back = codec::decode(&codec::encode(&oracle)).expect("decode");
            assert_eq!(
                sexpr::print(&sx_back),
                sx,
                "sexpr→binary→sexpr changed the text\n  s-expr: {program}",
            );

            // ml → binary → ml is lossless (and structurally equal to the oracle).
            let ml = printer::print(&oracle, WIDTH);
            let via_bin = codec::decode(&codec::encode(&parser::read_ml(&ml).arenas))
                .expect("ml→binary decodes");
            assert_eq!(
                printer::print(&via_bin, WIDTH),
                ml,
                "ml→binary→ml changed the ML\n  s-expr: {program}\n  ml: {ml}",
            );
            assert!(
                via_bin.structurally_eq(&oracle),
                "ml→binary→ml changed the tree\n  s-expr: {program}\n  ml: {ml}",
            );
            total += 1;
        }
    }
    assert!(total >= 6000, "swept a meaningful space, got {total}");
}
