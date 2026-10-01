use super::*;
use crate::oracle::{Verdict, compile_catching};
// `bolero::generator` re-exports the `bolero_generator` crate (`pub use bolero_generator::self`),
// so the byte-slice test driver lives at this path.
use bolero::generator::bolero_generator::driver::{ByteSliceDriver, Options};

/// Generate a program by coercing a fixed byte string through the BOLERO driver (deterministic) —
/// exercises the `Driver`→`Choice` adapter + the shared grammar, matching the `cargo bolero` path.
fn gen_from(bytes: &[u8]) -> Program {
    let options = Options::default();
    let mut driver = ByteSliceDriver::new(bytes, &options);
    ProgramGen
        .generate(&mut driver)
        .expect("ProgramGen always produces a program")
}

/// Every SYMBOL-IN-COMPOUND shape (v-nix #7710, tag-20 value_codec) is a well-formed program the
/// compiler cleanly handles — cdz-smith emitted no symbols before, so this pins the new coverage
/// and the coercion invariant (never a crash / invalid wasm / parse error) over symbol programs.
#[test]
fn symbol_compound_shapes_are_cleanly_handled() {
    for seed in 0u8..40 {
        let bytes = [
            seed,
            seed.wrapping_mul(3),
            seed.wrapping_add(7),
            1,
            2,
            3,
            4,
            5,
        ];
        let mut c = ByteCursorChoice::new(&bytes);
        let body = gen_symbol_compound_body(&mut c);
        assert!(
            body.contains("#\""),
            "a symbol-compound shape must carry a symbol: {body}"
        );
        let prog = format!("(do (def (main) {body}) (export main))");
        match compile_catching(&prog) {
            Verdict::Compiled { .. } | Verdict::Declined { .. } => {}
            other => panic!("symbol-compound program not cleanly handled: {prog}\n{other:?}"),
        }
    }
}

/// The NFD/NFC symbol-normalization shape is REACHABLE (some seed emits it) and COMPILES — it pins
/// rcdzc #9245 (non-ASCII Symbol.of NFC-normalization, "FINDING #23") in the differential: the NFD
/// (decomposed) and NFC (precomposed) spellings of `café` are ONE symbol, so the equality is a valid
/// Bool program both backends must agree on.
#[test]
fn nfc_symbol_normalization_shape_is_reachable_and_compiles() {
    let mut saw_nfc = false;
    for seed in 0u8..60 {
        let bytes = [
            seed,
            seed.wrapping_mul(3),
            seed.wrapping_add(7),
            1,
            2,
            3,
            4,
            5,
        ];
        let mut c = ByteCursorChoice::new(&bytes);
        let body = gen_symbol_compound_body(&mut c);
        // The NFD form carries a combining acute accent (U+0301).
        if body.contains("cafe\u{301}") {
            saw_nfc = true;
            let prog = format!("(do (def (main) {body}) (export main))");
            assert!(
                matches!(compile_catching(&prog), Verdict::Compiled { .. }),
                "the NFC-normalization symbol shape must COMPILE: {prog}"
            );
        }
    }
    assert!(
        saw_nfc,
        "the NFD/NFC symbol shape was never generated across 60 seeds"
    );
}

/// Every NOMINAL-over-Symbol shape (v-nix #7714 — a nominal newtype wrapping a Symbol) is a
/// well-formed program the compiler cleanly handles, pinning the `(Symbol.of …)` value-form recovery.
#[test]
fn nominal_symbol_shapes_are_cleanly_handled() {
    for seed in 0u8..24 {
        let bytes = [seed, seed.wrapping_mul(5), seed.wrapping_add(3), 2, 4, 6];
        let mut c = ByteCursorChoice::new(&bytes);
        let (type_decl, body) = gen_nominal_symbol_program(&mut c);
        assert!(
            body.contains("Tag.T") && body.contains("#\""),
            "shape carries a nominal symbol: {body}"
        );
        let prog = format!("(do {type_decl} (def (main) {body}) (export main))");
        match compile_catching(&prog) {
            Verdict::Compiled { .. } | Verdict::Declined { .. } => {}
            other => panic!("nominal-symbol program not cleanly handled: {prog}\n{other:?}"),
        }
    }
}

/// `generate_coerced` actually REACHES the symbol special-program (variant slot 4) for some entropy —
/// so the Symbol-in-compound widening is live in the real coercion path, not merely callable directly.
#[test]
fn generate_coerced_reaches_a_symbol_program() {
    let hit = (0u64..4000).any(|s| {
        let bytes: Vec<u8> = (0..24)
            .map(|i| ((s.wrapping_mul(0x9E37_79B9).wrapping_add(i)) & 0xff) as u8)
            .collect();
        generate_coerced(&bytes).source.contains("#\"")
    });
    assert!(
        hit,
        "the symbol-in-compound special program (variant slot 4) must be reachable"
    );
}

/// The LARGE-VALUE grammar builds a tail-recursive `>64 KiB` heap-List program the compiler cleanly
/// handles (it compiles; the OOB it targets is a RUN-time value-escape, exercised by the differential,
/// not a compile fault). Pins the shape: a `build` recursive def + a `main` that returns/consumes it,
/// with a size ≥8500 (≥64 KiB at 8 B/Int64).
#[test]
fn large_value_grammar_builds_a_big_list_program() {
    for s in 0u64..24 {
        let bytes: Vec<u8> = (0..24)
            .map(|i| ((s.wrapping_mul(0x9E37_79B9).wrapping_add(i)) & 0xff) as u8)
            .collect();
        let src = generate_large_value(&bytes).source;
        assert!(
            src.contains("(build ") && src.contains("List.push"),
            "recursive builder: {src}"
        );
        assert!(src.contains("(export main)"), "exports main: {src}");
        match compile_catching(&src) {
            Verdict::Compiled { .. } | Verdict::Declined { .. } => {}
            other => panic!("large-value program not cleanly handled: {src}\n{other:?}"),
        }
    }
}

/// The NARROW type-fuzzing grammar (S194): every generated program parses + is cleanly handled by
/// the compiler (Compiled or a correct coded Declined — the ~20% ill-typed arm), never a crash /
/// invalid wasm / parse error. A well-formed in-fragment population for the false-reject hunt.
#[test]
fn typecheck_grammar_is_cleanly_handled() {
    let mut compiled = 0;
    let mut declined = 0;
    for s in 0u64..120 {
        let bytes: Vec<u8> = (0..64)
            .map(|i| ((s.wrapping_mul(0x9E37_79B9).wrapping_add(i)) & 0xff) as u8)
            .collect();
        let src = generate_typecheck(&bytes).source;
        match compile_catching(&src) {
            Verdict::Compiled { .. } => compiled += 1,
            Verdict::Declined { .. } => declined += 1,
            other => panic!("type-fuzz program not cleanly handled: {src}\n{other:?}"),
        }
    }
    // The 80/20 split means BOTH outcomes must actually occur (well-typed compiles + ill-typed
    // declines) — a witness that the grammar exercises both directions, not just one.
    assert!(
        compiled > 0,
        "the well-typed arm must produce compiled programs"
    );
    assert!(
        declined > 0,
        "the ill-typed arm must produce coded declines"
    );
}

/// The coercion invariant: ANY entropy → a valid, well-formed program the compiler CLEANLY handles
/// (compiles, or a correct decline like a const-folded overflow) — never a crash / invalid wasm /
/// parse error. Every input reaches the compiler.
#[test]
fn any_entropy_coerces_to_a_cleanly_handled_program() {
    let inputs: [&[u8]; 6] = [
        &[],
        &[0],
        &[1, 2, 3, 4, 5, 6, 7, 8],
        &[0xFF; 32],
        &[
            0x01, 0x00, 0x02, 0x01, 0x00, 0x00, 0x01, 0x02, 0x00, 0x00, 0x00, 0x00,
        ],
        &[
            0x9e, 0x37, 0x79, 0xb9, 0x7f, 0x4a, 0x7c, 0x15, 0x11, 0x22, 0x33, 0x44,
        ],
    ];
    for bytes in inputs {
        let program = gen_from(bytes);
        assert!(
            program.source.starts_with("(do ")
                    // `main` is either param-less `(def (main) …)` or a heap-param entry `(def (main (: v0 …
                    && program.source.contains("(def (main")
                    && program.source.ends_with("(export main))"),
            "shape: {}",
            program.source
        );
        let verdict = compile_catching(&program.source);
        assert!(
            matches!(verdict, Verdict::Compiled { .. } | Verdict::Declined { .. }),
            "coerced program must be cleanly handled (Compiled/Declined), got {verdict:?} for: {}",
            program.source
        );
    }
}

/// `generate_coerced` (the LIB byte-cursor path) coerces ANY entropy into a cleanly-handled program
/// too — the same invariant as the bolero path, exercised through `ByteCursorChoice`.
#[test]
fn generate_coerced_lib_path_is_cleanly_handled() {
    for seed in 0u64..64 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut bytes = Vec::new();
        for _ in 0..24 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let program = generate_coerced(&bytes);
        assert!(
            program.source.starts_with("(do ") && program.source.ends_with("(export main))"),
            "shape: {}",
            program.source
        );
        let verdict = compile_catching(&program.source);
        assert!(
            matches!(verdict, Verdict::Compiled { .. } | Verdict::Declined { .. }),
            "generate_coerced program must be cleanly handled, got {verdict:?} for: {}",
            program.source
        );
    }
    // Empty entropy still yields a valid program (all choices bottom out).
    assert!(generate_coerced(&[]).source.ends_with("(export main))"));
}

/// `gen_macro_body` REACHES both hygiene-safe shapes (pure-splice `(f …)` #8528 + hygiene-safe binder
/// `(g …)` #8531; v-lean-oracle-blessed S480) and every body COMPILES — the value-dim macro-expansion
/// surface, so the wasm-vs-rust differential confirms the compiler's expansion against the reduce oracle.
#[test]
fn gen_macro_body_reaches_all_forms_and_compiles() {
    let (mut saw_splice, mut saw_binder) = (false, false);
    for seed in 0u64..256 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(4127);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_macro_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        saw_splice |= body.contains("(def (f (quote x))");
        saw_binder |= body.contains("(def (g (quote e))");
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "macro body must COMPILE: {src}"
        );
    }
    assert!(saw_splice, "should reach the pure-splice macro shape");
    assert!(
        saw_binder,
        "should reach the hygiene-safe binder macro shape"
    );
}

/// REGRESSION GUARD for the bucket-1 emit miscompile (rcdzc #4961): an EXPORTED entry with a
/// heap/reference-typed param + a reachable RECURSIVE fn once emitted the recursive call's result at
/// the wrong wasm width (i32 vs i64) → InvalidWasm. Each `HEAP_PARAM_TYPES` entry, wired to the exact
/// minimal shape the fuzzer found + bisected, must COMPILE (not merely be cleanly handled) — so a
/// re-introduction of the def-index-shift bug fails here rather than silently in a campaign.
#[test]
fn heap_param_entry_over_a_recursive_fn_compiles() {
    for ty in HEAP_PARAM_TYPES {
        let src = format!(
            "(do (def (main (: v0 {ty})) (do (def (v1 v2) (if (<= v2 0) v2 (v1 (- v2 1)))) (v1 2))) (export main))"
        );
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "heap-param entry ({ty}) over a recursive fn must COMPILE (bucket-1 #4961 regression): {src}"
        );
    }
}

/// [`generate_large_value`] — the LargeValue generator behind `--large` (opt-invariance + wasm-vs-rust
/// differential) — must keep its two load-bearing invariants, or every `--large` sweep silently
/// degrades: (1) the builder count exceeds ONE 64 KiB linear-memory page (8 B × 8192 = 64 KiB), the
/// whole point being to stress the >64 KiB value-escape copy-out path where the #7793/#7800 OOBs lived
/// — a future edit lowering the `int_bounded(8500, …)` floor below 8192 would quietly stop reaching it;
/// (2) the program COMPILES cleanly (a tail-recursive builder + a param-less `main`). Pin BOTH across
/// varied entropy, and pin that BOTH main-body variants (return the list / `List.len` it) are reached.
#[test]
fn generate_large_value_exceeds_a_page_and_compiles() {
    // Extract the literal build count `n` from the main body: the sole `(build <digits> (list))` call
    // (the builder def uses `(build (- n 1) …)`, never a bare digit, so a digits-after-`(build ` match
    // is unambiguous).
    fn build_count(src: &str) -> u64 {
        let after = src
            .split("(build ")
            .find(|seg| seg.starts_with(|ch: char| ch.is_ascii_digit()))
            .expect("a `(build <n> (list))` call with a literal count");
        after
            .split(|ch: char| !ch.is_ascii_digit())
            .next()
            .and_then(|d| d.parse().ok())
            .expect("a parseable build count")
    }
    let (mut saw_return, mut saw_consume) = (false, false);
    for seed in 0u64..64 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(11);
        let mut bytes = Vec::new();
        // ≥17 bytes: the two `int_bounded(…)` calls consume 8 bytes EACH (16 total), so the
        // `variant(2)` that picks the main-body shape needs a live byte past them or it always
        // coerces to 0 (only the return variant) — 32 gives both variants real entropy.
        for _ in 0..32 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let src = generate_large_value(&bytes).source;
        // (1) exceeds one 64 KiB page (8 B/Int64 × 8192).
        let n = build_count(&src);
        assert!(
            n >= 8192,
            "large-value builder count {n} must exceed one 64 KiB page (>= 8192 Int64 elements) so it \
                 stresses the >64 KiB value-escape copy-out path: {src}"
        );
        // (2) compiles cleanly.
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "the large-value builder must COMPILE: {src}"
        );
        if src.contains("(def (main) (List.len (build ") {
            saw_consume = true;
        } else if src.contains("(def (main) (build ") {
            saw_return = true;
        }
    }
    assert!(
        saw_return && saw_consume,
        "both main-body variants must be reachable across seeds (return the list AND List.len it): \
             saw_return={saw_return} saw_consume={saw_consume}"
    );
}

/// [`generate_reclaim_shapes`] — the ReclaimShapes generator behind `--reclaim` (the value/opt/
/// determinism oracles' counterpart to the corpus `(live-objects N)` leak pins) — must keep its two
/// load-bearing invariants or every `--reclaim` sweep silently degrades: (1) EVERY generated program
/// COMPILES cleanly (a declined shape stresses no reclaim path and contributes no value check); and
/// (2) ALL TEN owned-aggregate shapes stay reachable across varied entropy (matchsum-len, loop-accum
/// rebind, scalar-project-drop-heap-sibling, nested sum-in-sum, in-arm push rebind, dup-used-twice,
/// depth-3 recursive-descent, escaping-heap-child, param-scrutinee bare-payload-reuse, self-recursive
/// sum-fold) — a generator edit that drops a shape would quietly stop exercising that reclaim class.
#[test]
fn generate_reclaim_shapes_reaches_all_forms_and_compiles() {
    // Distinctive, mutually-exclusive markers for the twenty-seven shapes (see `generate_reclaim_shapes`).
    let mut reached = [false; 31];
    for seed in 0u64..1395 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(3);
        let mut bytes = Vec::new();
        // variant(5) reads 1 byte then four int_bounded reads consume 8 each (33 total); 40 keeps the
        // shape selector AND every literal on live entropy.
        for _ in 0..40 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let src = generate_reclaim_shapes(&bytes).source;
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "every reclaim shape must COMPILE (a decline stresses no reclaim path): {src}"
        );
        // Check the UNIQUE structural markers first; shape 0's plain matchsum-len marker is generic
        // (shape 6's innermost projection `((Mk xs) (List.len xs))` also contains it), so it goes LAST.
        if src.contains("(def (build (: k Int64)") {
            reached[1] = true;
        } else if src.contains("(type Pair (Mk Int64") {
            reached[2] = true;
        } else if src.contains("(type Inner (I (List Int64))") {
            reached[3] = true;
        } else if src.contains("(type C (W B))") {
            reached[6] = true;
        } else if src.contains("(List.len (List.push xs") {
            reached[4] = true;
        } else if src.contains("(+ (List.len xs) (List.len xs))") {
            reached[5] = true;
        } else if src.contains("((Mk xs) xs))") {
            reached[7] = true;
        } else if src.contains("(type Tree (Leaf Int64)") {
            reached[8] = true;
        } else if src.contains("(type NL (Nil)") {
            reached[9] = true;
        } else if src.contains("(def (times (: f (-> Int64 Int64))") {
            reached[10] = true;
        } else if src.contains("(def (drive (: g (-> Int64 Int64))") {
            reached[11] = true;
        } else if src.contains("(def (mpow (: base BigInt)") {
            reached[12] = true;
        } else if src.contains("(def (rep (: b Bytes)") {
            reached[13] = true;
        } else if src.contains("(def (take (: it Lst)") {
            reached[14] = true;
        } else if src.contains("(def (g (: s (Set Int64))") {
            reached[15] = true;
        } else if src.contains("(def (f (: xs (List Int64)) (: ys (List Int64)))") {
            reached[16] = true;
        } else if src.contains("(def m (Record.merge ") {
            reached[17] = true;
        } else if src.contains("(Value.decode (Value.encode ") {
            reached[18] = true;
        } else if src.contains("(type Cell (C (List Int64) Int64))") {
            reached[20] = true;
        } else if src.contains("(type P (Mk Int64 Int64))") {
            reached[21] = true;
        } else if src.contains("(def (caller (: xs (List Int64))) (go xs 2))") {
            reached[27] = true; // shape 27 = mutual-group caller-drop ADMIT (no-reuse caller; checked BEFORE
        // shape 22 since both contain the `(def (go …` marker — shape 22 REUSES xs)
        } else if src.contains("(def (go (: xs (List Int64)) (: d Int64))") {
            reached[22] = true;
        } else if src.contains("(type Req (Req (Record (: node Node)") {
            reached[23] = true;
        } else if src.contains("(type SC (SC (Record (: n Int64)))") {
            reached[24] = true;
        } else if src.contains("(def (walk (: xs (List Int64)) (: d Int64))") {
            reached[25] = true; // shape 25 = single-self-loop fresh-owned-arg caller-drop tripwire
        } else if src.contains("(def (inner (: s String) (: i Int64) (: acc Int64))") {
            reached[26] = true; // shape 26 = nested-tail-loop String.at Some-shell accumulate tripwire
        } else if src.contains("(String.slice \"abcdef\" 1 4)") {
            reached[28] = true; // shape 28 = String.at-on-a-slice-view source-reclaim tripwire (c6469 value side)
        } else if src.contains("(def (f (: mode Int64))") {
            reached[29] = true; // shape 29 = divergent-arm-consume live-after tripwire (c2236 value side)
        } else if src.contains("((list (list p q) (list x y) .. r)") {
            reached[30] = true; // shape 30 = N-per-arm refutable nested-list desugar tripwire (#8430)
        } else if src.contains("(type L (Nil) (Cons (List Int64) L))") {
            reached[19] = true;
        } else if src.contains("((Mk xs) (List.len xs)))") {
            reached[0] = true;
        }
    }
    assert!(
        reached.iter().all(|&r| r),
        "all thirty-one reclaim shapes must be reachable across seeds: reached={reached:?}"
    );
}

/// [`generate_effect`] — the Effect generator behind `--effect` (value-observable coverage of the
/// effects lowering) — must keep its two load-bearing invariants: (1) EVERY generated program COMPILES
/// (an effect body that declines exercises no lowering); and (2) ALL EIGHT forms stay reachable across
/// varied entropy (single-handler, nested-handler, effect+collection, multi-op, mapstate, cfjoin,
/// discarded-call, splat-in-handler) — a wiring edit that drops a form would silently stop fuzzing that slice.
#[test]
fn generate_effect_reaches_all_forms_and_compiles() {
    // Distinctive, mutually-exclusive markers (see `generate_effect`): nested = two effects E1/E2;
    // multiop = one effect E with two ops o1/o2; collection = a `List`; mapstate = effect T / op bump
    // threading a Map handler-state; splat = the `(.. #tuple((T.tick)))` splat (checked BEFORE cfjoin,
    // which form 7 ALSO matches via `(op tick `); cfjoin = op tick (control-flow-join nested-tuple);
    // discarded-call = a top-level `(def (bump (: x Int64)) …)` performing helper (checked BEFORE the
    // single-handler marker, which form 6 ALSO contains); single = the plain one-op form.
    let mut reached = [false; 8];
    for seed in 0u64..448 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(97);
        let mut bytes = Vec::new();
        for _ in 0..24 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let src = generate_effect(&bytes).source;
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "every effect program must COMPILE: {src}"
        );
        if src.contains("(def (bump (: x Int64)) (E.o x))") {
            reached[6] = true; // discarded-call state-threading (contains `(effect E (op o ` too — check first)
        } else if src.contains("(.. #tuple((T.tick)))") {
            reached[7] = true; // splat-in-handler (contains `(op tick ` too — check BEFORE cfjoin)
        } else if src.contains("(effect E1 ") {
            reached[1] = true; // nested-handler
        } else if src.contains("(effect E (op o1 ") {
            reached[3] = true; // multi-op
        } else if src.contains("(effect T (op bump ") {
            reached[4] = true; // mapstate (heap handler-state threading)
        } else if src.contains("(op tick ") {
            reached[5] = true; // cfjoin (control-flow-join nested-tuple new-state)
        } else if src.contains("List") {
            reached[2] = true; // effect + collection
        } else if src.contains("(effect E (op o ") {
            reached[0] = true; // single-handler
        }
    }
    assert!(
        reached.iter().all(|&r| r),
        "all eight effect forms must be reachable across seeds: reached={reached:?}"
    );
}

#[test]
fn generate_export_param_reaches_all_forms_and_compiles() {
    // Distinctive, mutually-exclusive markers for the sixteen export-param shapes (see
    // `generate_export_param`): double/add/idn/u(UInt64)/f(3-arg)/sgn + three (List Int64) entry-param
    // shapes lhd/top(helper)/suml(recursive-walk) + the #9586 record-Option-newtype `run` + the #9684
    // const-list-of-Option-field sibling + the #9687 payload-variant field + the #9689 consuming-slice `cat`
    // + the #9694 String-consume `catlen` + the #9699 scalar-fielded Record `addpt` + the #9701 rpp3
    // heap-carrying Record `rsum` + the #9707 wfp1 >16-flat-scalar `big` + the #9716 els1 list<String>
    // byte-leaf `slen` + the #9714 rpp8/rpp9 nested-Tuple `f` + the #9718/#9742 eos1 option<String>
    // sum-entry-param `f` + the #9747 rpp21/22 result<Int64,String> two-payload-sum-entry-param `f` + the
    // #9746/#9753 eob1 option<Bytes> bytes-byte-leaf-sum-entry-param `f` + the lpt1 list<tuple<Int64,Int64>>
    // compound-list-element-entry-param `f` + the eot1 option<tuple<Int64,Int64>> sum-holding-a-compound
    // entry-param `f` + the lpr1 list<record<x,y>> record-list-element-entry-param `f` + the eor1
    // option<record<x,y>> sum-holding-a-record-entry-param `f` + the ell1 list<list<Int64>> nested-heap
    // (heap-list-of-heap-lists) entry-param `f` + the rrf1 record-with-a-record-field nested-product
    // entry-param `f` + the tol1 tuple<Int64,list<Int64>> value-holding-a-heap entry-param `f` + the chr1
    // Char scalar-entry-param `f` + the big1 BigInt heap-bignum scalar-entry-param `f` + the ssa1
    // String.scalar-at char-extraction entry-param `f` + the eop3 option<list<string>>
    // sum-holding-a-byte-leaf-list entry-param `f` + the rob1 record-of-bools bool-leaf entry-param `f` +
    // the tdd1 runtime-`?` do-def entry-param `main` + the trr1 expression-position `?` entry-param `main` + the trl1 multi-`?` compound-ctor entry-param `main` + the trn1 nested-compound-ctor `?` entry-param `main` + the trc1 call-argument `?` entry-param `main` + the trsc1 CHAMP-collection-in-a-try-Ok-arm entry-param `main` + the trml1 Map.lookup-in-a-try-Ok-arm entry-param `main` + the chdo1 Set.remove-threaded-dead-at-base entry-param `main` + the trae1 bare-returned `?`-bound heap-Result entry-param `main` + the srm2 nested set-rest re-match entry-param `main` + the trnt1 chained double-`?` do-def entry-param `main` + the trnt1c compact nested-`?` entry-param `main` + the chdo2 Map.remove-threaded-dead-at-base entry-param `main` + the trss1 String.slice-in-a-try-Ok-arm entry-param `main` + the byp2 Bytes-entry-param bin-match destructure `main` + the stll1 invariant-Set-param Set.to-list-in-a-self-loop `main` + the sci1 canonicalizing list-element double-used at Set.insert+Set.contains `main` + the mci1 canonicalizing list-key double-used at Map.insert+Map.lookup `main` + the mtll1 invariant-Map-param Map.to-list-in-a-self-loop `main` + the mvg1 narrow-width tuple MAP VALUE literal grounded to declared field width `main` + the trbs1 `?`-bound Bytes.slice-in-a-try-Ok-arm reclaiming the try-shell + slice view `main` + the trbs2 slice-of-slice over a `?`-bound Bytes reclaiming the try-shell + both view leaves `main`.
    let mut reached = [false; 60];
    for seed in 0u64..3600 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(51);
        let mut bytes = Vec::new();
        // variant(53) reads 1 byte then SEVEN int_bounded reads consume 8 each (57 total); 64 keeps the
        // shape selector AND every arg literal on live entropy. (shapes 19-52 reuse e0/e1/e2/s0/u/a — no new read.)
        for _ in 0..64 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let ep = generate_export_param(&bytes);
        assert!(
            matches!(compile_catching(&ep.source), Verdict::Compiled { .. }),
            "every export-param shape must COMPILE (a decline exercises no marshal): {}",
            ep.source
        );
        assert!(
            !ep.args.is_empty(),
            "every export-param shape must carry at least one call arg: {}",
            ep.source
        );
        if ep.source.contains("(def (double ") {
            reached[0] = true;
        } else if ep.source.contains("(def (add ") {
            reached[1] = true;
        } else if ep.source.contains("(def (idn ") {
            reached[2] = true;
        } else if ep.source.contains("(def (u (: x UInt64))") {
            reached[3] = true;
        } else if ep
            .source
            .contains("(def (f (: p Int64) (: q Int64) (: r Int64))")
        {
            reached[4] = true;
        } else if ep.source.contains("(def (sgn ") {
            reached[5] = true;
        } else if ep.source.contains("(def (lhd ") {
            reached[6] = true;
        } else if ep.source.contains("(def (peek ") {
            reached[7] = true; // shape 7 = helper `peek` + `top`
        } else if ep.source.contains("(def (walk ") {
            reached[8] = true; // shape 8 = recursive index-walk `suml`
        } else if ep.source.contains("(def (run (: nid UInt64))") {
            reached[9] = true; // shape 9 = #9586 record-Option-newtype `run`
        } else if ep.source.contains("(= tags (list") {
            reached[10] = true; // shape 10 = #9684 const-list-of-Option-field sibling
        } else if ep.source.contains("(: res (Result Int64 String))") {
            reached[11] = true; // shape 11 = #9687 payload-variant const-sum-field
        } else if ep.source.contains("(List.concat xs") {
            reached[12] = true; // shape 12 = #9689 consuming-slice `cat`
        } else if ep.source.contains("(String.concat a b)") {
            reached[13] = true; // shape 13 = #9694 String-consume `catlen`
        } else if ep.source.contains("(def (addpt ") {
            reached[14] = true; // shape 14 = #9699 scalar-fielded Record `addpt`
        } else if ep.source.contains("(def (rsum ") {
            reached[15] = true; // shape 15 = #9701 rpp3 heap-carrying Record `rsum`
        } else if ep.source.contains("(def (big ") {
            reached[16] = true; // shape 16 = #9707 wfp1 >16-flat-scalar `big`
        } else if ep.source.contains("(def (slen ") {
            reached[17] = true; // shape 17 = #9716 els1 list<String> byte-leaf `slen`
        } else if ep.source.contains("#tuple(Int64 #tuple(Int64 Int64))") {
            reached[18] = true; // shape 18 = #9714 rpp8/rpp9 nested-Tuple `f`
        } else if ep.source.contains("(: o (Option String))") {
            reached[19] = true; // shape 19 = #9718/#9742 eos1 option<String> sum-entry-param `f`
        } else if ep
            .source
            .contains("(def (step (: r (Result Int64 String)))")
        {
            reached[35] = true; // shape 35 = trr1 expression-position `?` entry-param `main` (#9859) —
        // checked BEFORE shape 20 since its `step` source CONTAINS shape 20's
        // `(: r (Result Int64 String))` marker (shape 20 is `(def (f (: r …`)
        } else if ep.source.contains("(Ok #list((try r) (try s)))") {
            reached[36] = true; // shape 36 = trl1 multi-`?` compound-ctor (flat #list) entry-param `main`
        // (#9869) — checked BEFORE shape 20 (its `mk` source CONTAINS shape 20's
        // marker); keyed on the FLAT-#list body to distinguish from shape 37's `mk`
        } else if ep.source.contains("(Ok #tuple(#record((= a (try r)))") {
            reached[37] = true; // shape 37 = trn1 NESTED-compound-ctor `?` entry-param `main` — checked
        // BEFORE shape 20 too; keyed on the nested tuple/record body (shape 37's
        // `mk` shares shape 36's `(def (mk (: r …` prefix, so key on the BODY)
        } else if ep.source.contains("(: r (Result Int64 String))") {
            reached[20] = true; // shape 20 = #9747 rpp21/22 result<Int64,String> two-payload-sum-entry-param `f`
        } else if ep.source.contains("(: o (Option Bytes))") {
            reached[21] = true; // shape 21 = #9746/#9753 eob1 option<Bytes> bytes-byte-leaf-sum-entry-param `f`
        } else if ep.source.contains("(: xs (List #tuple(Int64 Int64)))") {
            reached[22] = true; // shape 22 = lpt1 list<tuple<Int64,Int64>> compound-list-element-entry-param `f`
        } else if ep.source.contains("(: o (Option #tuple(Int64 Int64)))") {
            reached[23] = true; // shape 23 = eot1 option<tuple<Int64,Int64>> sum-holding-a-compound-entry-param `f`
        } else if ep
            .source
            .contains("(: xs (List (Record (: x Int64) (: y Int64))))")
        {
            reached[24] = true; // shape 24 = lpr1 list<record<x,y>> record-list-element-entry-param `f`
        } else if ep
            .source
            .contains("(: o (Option (Record (: x Int64) (: y Int64))))")
        {
            reached[25] = true; // shape 25 = eor1 option<record<x,y>> sum-holding-a-record-entry-param `f`
        } else if ep.source.contains("(: xs (List (List Int64)))") {
            reached[26] = true; // shape 26 = ell1 list<list<Int64>> nested-heap entry-param `f`
        } else if ep
            .source
            .contains("(: inner (Record (: x Int64) (: y Int64)))")
        {
            reached[27] = true; // shape 27 = rrf1 record-with-a-record-field nested-product entry-param `f`
        } else if ep.source.contains("(: t #tuple(Int64 (List Int64)))") {
            reached[28] = true; // shape 28 = tol1 tuple<Int64,list<Int64>> value-holding-a-heap entry-param `f`
        } else if ep.source.contains("(: c Char)") {
            reached[29] = true; // shape 29 = chr1 Char scalar-entry-param `f`
        } else if ep.source.contains("(: x BigInt)") {
            reached[30] = true; // shape 30 = big1 BigInt heap-bignum scalar-entry-param `f`
        } else if ep.source.contains("(String.scalar-at s 0)") {
            reached[31] = true; // shape 31 = ssa1 String.scalar-at char-extraction entry-param `f`
        } else if ep.source.contains("(: xs (Option (List String)))") {
            reached[32] = true; // shape 32 = eop3 option<list<string>> sum-holding-a-byte-leaf-list entry-param `f`
        } else if ep.source.contains("(: read Bool)") {
            reached[33] = true; // shape 33 = rob1 record-of-bools bool-leaf entry-param `f`
        } else if ep.source.contains("(def (quarter (: k Int64))") {
            reached[34] = true; // shape 34 = tdd1 runtime-`?` do-def entry-param `main` (#9840)
        } else if ep.source.contains("(Ok (sum2 (try r) (try s)))") {
            reached[38] = true; // shape 38 = trc1 call-argument `?` entry-param `main` (Int64-Err, no
        // collision with shape 20's `(Result Int64 String)` marker)
        } else if ep.source.contains("(Ok #set(x))") {
            reached[39] = true; // shape 39 = trsc1 CHAMP-collection-in-a-try-Ok-arm entry-param `main` (GAP-4)
        } else if ep.source.contains("(Ok #map((= x x)))") {
            reached[40] = true; // shape 40 = trml1 Map.lookup-in-a-try-Ok-arm entry-param `main` (node#6-nonlen #240b64090d)
        } else if ep.source.contains("(Set.remove s n)") {
            reached[41] = true; // shape 41 = chdo1 Set.remove-threaded-dead-at-base entry-param `main` (CHAMP reclaim-on-edge #e2f72191e0)
        } else if ep.source.contains("(def ir (try rr))") {
            reached[42] = true; // shape 42 = trae1 bare-returned `?`-bound heap-Result entry-param `main` ((a) alias-husk equalize #297538a0df)
        } else if ep.source.contains("(#set(10 (.. r))") {
            reached[43] = true; // shape 43 = srm2 nested set-rest re-match entry-param `main` (materialized set-rest residual #9962)
        } else if ep.source.contains("(Ok (try a))") {
            reached[44] = true; // shape 44 = trnt1 chained double-`?` do-def entry-param `main` (unified chained-? shell reclaim #a1e26895c3)
        } else if ep.source.contains("(Ok (try (try rr)))") {
            reached[45] = true; // shape 45 = trnt1c compact nested-`?` entry-param `main` (inner-first-hoist desugar #9978)
        } else if ep.source.contains("(Map.remove m n)") {
            reached[46] = true; // shape 46 = chdo2 Map.remove-threaded-dead-at-base entry-param `main` (CHAMP reclaim-on-edge #e2f72191e0, Map twin of 41)
        } else if ep.source.contains("(String.slice s 0 1)") {
            reached[47] = true; // shape 47 = trss1 String.slice-in-a-try-Ok-arm entry-param `main` (node#6-nonlen STRING-VIEW arm #240b64090d)
        } else if ep.source.contains("(bin (u8 x) (u8 y))") {
            reached[48] = true; // shape 48 = byp2 Bytes-entry-param bin-match destructure `main` (borrow-lift #17bccbd01a)
        } else if ep.source.contains("(Set.to-list s)") {
            reached[49] = true; // shape 49 = stll1 invariant-Set-param Set.to-list'd in a self-loop `main` (borrow-gate reclaim #b50180e899)
        } else if ep.source.contains("#set(#list(9 9))") {
            reached[50] = true; // shape 50 = sci1 canonicalizing list-element double-used at Set.insert+Set.contains `main` (borrow-when-canonicalizing #a0f501dde5)
        } else if ep.source.contains("(Map.insert Map.empty xs 7)") {
            reached[51] = true; // shape 51 = mci1 canonicalizing list-key double-used at Map.insert+Map.lookup `main` (borrow-when-canonicalizing #a0f501dde5, Map twin of 50)
        } else if ep.source.contains("(Map.to-list mp)") {
            reached[52] = true; // shape 52 = mtll1 invariant-Map-param Map.to-list'd in a self-loop `main` (borrow-gate reclaim #b50180e899, Map twin of 49)
        } else if ep.source.contains("(match (Bytes.slice b 1 2)") {
            reached[53] = true; // shape 53 = byp3 consumed Bytes.slice entry param crosses `main` (dup-aware borrow-lift reclaim / CDZ0904 decline-lift #f8382ff506) — `match` form (eab3 shape 55 uses Option.expect over the SAME slice, so this marker MUST be the match-specific prefix)
        } else if ep.source.contains("(Symbol.of s)") {
            reached[54] = true; // shape 54 = ckr2 consumed String entry param crosses `main` (dup-aware borrow-lift reclaim, byp3's non-Bytes sibling #f8382ff506)
        } else if ep.source.contains("Option.expect (Bytes.slice") {
            reached[55] = true; // shape 55 = eab3 consumed sliced Bytes entry param crosses through Option.expect `main` (consume-sink whitelist / CDZ0904 decline-lift #e05f838d9a)
        } else if ep.source.contains("(List (Tuple Int8))") {
            reached[56] = true; // shape 56 = nle1 narrow-width tuple LIST element literal grounded to declared field width `main` (rust-backend compound-list-element grounding #d107107d9c / #10105). Marker is the LIST-specific `(List (Tuple Int8))` — shape 57 (mvg1) ALSO contains `#tuple(100)`, so the mutually-exclusive markers are `(List (Tuple Int8))` vs `(Map Int64 (Tuple Int8))` (per the S652/S653 substring-collision discipline)
        } else if ep.source.contains("(Map Int64 (Tuple Int8))") {
            reached[57] = true; // shape 57 = mvg1 narrow-width tuple MAP VALUE literal grounded to declared field width `main` (rust-backend compound-map-value grounding #3421821f4f / #10098, Map twin of 56)
        } else if ep.source.contains("(Bytes.slice bs 0 2)") {
            reached[58] = true; // shape 58 = trbs1 `?`-bound Bytes.slice-in-a-try-Ok-arm reclaims the try-shell + slice view `main` (core-opt G5 rope-owned-builder relax #4726be48e / #10147, Bytes.slice sibling of shape 47's trss1). Marker `(Bytes.slice bs 0 2)` (bs/0/2) is unique — distinct from shape 53's `(match (Bytes.slice b 1 2)` and shape 55's `Option.expect (Bytes.slice` (both `b 1 2`, ENTRY-param slices)
        } else if ep.source.contains("(Bytes.slice outer 1 3)") {
            reached[59] = true; // shape 59 = trbs2 SLICE-OF-SLICE over a `?`-bound Bytes in a try-Ok-arm reclaims the try-shell + BOTH view leaves `main` (core-opt #4726be48e / #10147, view-of-view sibling of shape 58's trbs1). Marker `(Bytes.slice outer 1 3)` — unique (`outer` used by no other shape; trbs2's outer slice `(Bytes.slice bs 1 4)` also distinct from shape 58's `bs 0 2`)
        }
    }
    assert!(
        reached.iter().all(|&r| r),
        "all sixty export-param shapes must be reachable across seeds: reached={reached:?}"
    );
}

/// The generator REACHES the heap-param-entry shape (the #4961 regression-guard path) across varied
/// entropy — so the coercing fuzzer actually exercises the exported-entry heap-param ABI lowering, not
/// only param-less `main`.
#[test]
fn generator_reaches_a_heap_param_entry() {
    let mut saw = false;
    for seed in 0u64..128 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(5);
        let mut bytes = Vec::new();
        for _ in 0..24 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        if generate_coerced(&bytes)
            .source
            .contains("(def (main (: v0 ")
        {
            saw = true;
            break;
        }
    }
    assert!(
        saw,
        "the coercing generator should reach a heap-param entry `(def (main (: v0 …)) …)`"
    );
}

/// The generator REACHES a runtime `(: n Int64)` entry that actually REFERENCES `n` — a
/// runtime-dependent program (not const-foldable) that keeps `if`/`match` joins live (no dead-branch
/// elim). Guards that the runtime-`n` branch produces `n`-using bodies, not just an unused param.
#[test]
fn generator_reaches_a_runtime_n_entry() {
    let mut saw = false;
    for seed in 0u64..256 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(61);
        let mut bytes = Vec::new();
        for _ in 0..24 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let src = generate_coerced(&bytes).source;
        // A runtime-n entry that USES n: after the `(: n Int64))` param, the body has a standalone
        // `n` token (a var reference) — tokenize on non-identifier chars so `main` doesn't false-match.
        if let Some((_, after)) = src.split_once("(def (main (: n Int64)) ") {
            let body = after.strip_suffix(") (export main))").unwrap_or(after);
            if body
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .any(|tok| tok == "n")
            {
                saw = true;
                break;
            }
        }
    }
    assert!(
        saw,
        "the coercing generator should reach a runtime `(: n Int64)` entry that references `n`"
    );
}

/// Every form the sized-int body arm emits — an ascribed literal `(: n T)`, a checked conversion
/// `(T.of n)`, and a width-safe binary op `(<op> (: a T) (: b T))` — must COMPILE (not merely be
/// cleanly handled) for EVERY `T` in `SIZED_INT_TYPES`: the arm is deliberately kept on the compile
/// path (small 0..=9 operands + no-overflow ops) so the coverage actually reaches narrow-width emit.
/// Guards `SIZED_INT_TYPES`/`SIZED_INT_OPS` (a bad type/op name would decline/parse-error here).
#[test]
fn gen_sized_int_body_reaches_nested_and_compiles() {
    let mut saw_nested = false;
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(2671);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_sized_int_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        // A NESTED sized expr = a sized op whose operand is itself an op (recursion working): >= 2 of
        // the sized ops present.
        let ops = ["(+ ", "(* ", "(& ", "(| ", "(^ "];
        let n: usize = ops.iter().map(|o| body.matches(o).count()).sum();
        saw_nested |= n >= 2;
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "recursive sized-int body must COMPILE: {src}"
        );
    }
    assert!(
        saw_nested,
        "recursive sized-int body should reach a NESTED (>=2-op) expression"
    );
}

#[test]
fn every_sized_int_body_form_compiles() {
    for t in SIZED_INT_TYPES {
        for src in [
            format!("(do (def (main) (: 5 {t})) (export main))"),
            format!("(do (def (main) ({t}.of 9)) (export main))"),
            format!("(do (def (main) (+ (: 9 {t}) (: 9 {t}))) (export main))"),
            format!("(do (def (main) (* (: 9 {t}) (: 9 {t}))) (export main))"),
            format!("(do (def (main) (& (: 9 {t}) (: 3 {t}))) (export main))"),
            format!("(do (def (main) (| (: 5 {t}) (: 2 {t}))) (export main))"),
            format!("(do (def (main) (^ (: 6 {t}) (: 3 {t}))) (export main))"),
        ] {
            assert!(
                matches!(compile_catching(&src), Verdict::Compiled { .. }),
                "sized-int body form must COMPILE: {src}"
            );
        }
    }
}

/// The generator REACHES the sized-int body arm across varied entropy — so the coverage (narrow-width
/// value/arith/conversion emit) is actually exercised by the coercing fuzzer, not dead.
#[test]
fn generator_reaches_a_sized_int_body() {
    let mut saw = false;
    for seed in 0u64..256 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(9);
        let mut bytes = Vec::new();
        for _ in 0..24 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let src = generate_coerced(&bytes).source;
        if SIZED_INT_TYPES
            .iter()
            .any(|t| src.contains(&format!(": {t})")) || src.contains(&format!("{t}.of ")))
        {
            saw = true;
            break;
        }
    }
    assert!(saw, "the coercing generator should reach a sized-int body");
}

/// Every `gen_float` expression (both widths, across depths/arms) is CLEANLY HANDLED — it COMPILES or
/// cleanly DECLINES (e.g. a const-folded non-finite `/ 0.0` or an overflow-to-`inf` `*`), never a
/// crash / invalid wasm / parse error. It is uniform-width by construction (Float32 leaves ascribed,
/// if/match arms same width) so no join mixes widths (which would hit v-rb's open match-emit-widen
/// bug) — a width leak would surface as InvalidWasm here. Also asserts the arm REACHES the compile
/// path (some body COMPILES), so the float value/arith/emit coverage is real, not all const-declines.
#[test]
fn gen_float_body_is_cleanly_handled_and_reaches_emit() {
    let mut saw_compiled = false;
    for seed in 0u64..256 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(13);
        let mut bytes = Vec::new();
        for _ in 0..32 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut fresh = 0usize;
        for is_f32 in [false, true] {
            let mut src = String::from("(do (def (main) ");
            gen_float(
                &mut ByteCursorChoice::new(&bytes),
                is_f32,
                MAX_DEPTH,
                &mut fresh,
                &mut src,
            );
            src.push_str(") (export main))");
            let v = compile_catching(&src);
            assert!(
                matches!(v, Verdict::Compiled { .. } | Verdict::Declined { .. }),
                "uniform-width float body must be cleanly handled (is_f32={is_f32}), got {v:?}: {src}"
            );
            saw_compiled |= matches!(v, Verdict::Compiled { .. });
        }
    }
    assert!(
        saw_compiled,
        "the float body arm should REACH the compile path (float value/arith emit), not only const-declines"
    );
}

/// The generator REACHES a float-typed body across varied entropy — so float value/arith/if-join/let
/// lowering is actually exercised by the coercing fuzzer, not dead.
#[test]
fn generator_reaches_a_float_body() {
    let mut saw = false;
    for seed in 0u64..256 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(21);
        let mut bytes = Vec::new();
        for _ in 0..24 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let src = generate_coerced(&bytes).source;
        // A float body shows up as a bare `N.0` (Float64) or an ascribed `Float32` literal.
        if src.contains(".0)") || src.contains(".0 ") || src.contains("Float32)") {
            saw = true;
            break;
        }
    }
    assert!(
        saw,
        "the coercing generator should reach a float-typed body"
    );
}

/// Every `gen_typed_compound` — a heterogeneous tuple (independently-typed leaves) or a non-Int64
/// homogeneous list — is CLEANLY HANDLED (leaf elements are type-correct by construction, so these
/// COMPILE); guards `gen_scalar_leaf`/`pick_scalar_ty` (a bad type name / ill-typed leaf would surface
/// as decline/InvalidWasm here). Also asserts a non-Int64-element compound is REACHED (real coverage
/// past the Int64-only `gen_compound`).
#[test]
fn gen_typed_compound_is_cleanly_handled_and_diverse() {
    let mut saw_non_int64 = false;
    for seed in 0u64..256 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(37);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut src = String::from("(do (def (main) ");
        gen_typed_compound(
            &mut ByteCursorChoice::new(&bytes),
            COMPOUND_DEPTH,
            &mut 0,
            &mut src,
        );
        src.push_str(") (export main))");
        assert!(
            matches!(
                compile_catching(&src),
                Verdict::Compiled { .. } | Verdict::Declined { .. }
            ),
            "type-diverse compound must be cleanly handled: {src}"
        );
        // Reached a non-Int64 element (float / bool / sized) → genuinely past Int64-only compounds.
        if src.contains(".0") || src.contains("true") || src.contains("false") || src.contains(": ")
        {
            saw_non_int64 = true;
        }
    }
    assert!(
        saw_non_int64,
        "type-diverse compounds should reach non-Int64 element types"
    );
}

/// Every `gen_typed_fn_call_body` (a typed local `(def (g (: x T)) …)` + `(g <T-leaf>)`) is CLEANLY
/// HANDLED across scalar param/return types, and REACHES a non-Int64 param type — so typed function
/// param/return/call ABI is genuinely exercised, not just the Int64 helpers.
#[test]
fn gen_typed_fn_call_body_is_cleanly_handled_and_diverse() {
    let mut saw_non_int64_param = false;
    for seed in 0u64..256 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(41);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut src = String::from("(do (def (main) ");
        gen_typed_fn_call_body(&mut ByteCursorChoice::new(&bytes), &mut 0, &mut src);
        src.push_str(") (export main))");
        assert!(
            matches!(
                compile_catching(&src),
                Verdict::Compiled { .. } | Verdict::Declined { .. }
            ),
            "typed fn def+call must be cleanly handled: {src}"
        );
        // A non-Int64 param shows up as `(: x Float…/Int8/…/Bool)` — i.e. `(: x ` not followed by Int64.
        if src.contains("(: x ") && !src.contains("(: x Int64)") {
            saw_non_int64_param = true;
        }
    }
    assert!(
        saw_non_int64_param,
        "typed fn bodies should reach a non-Int64 param type"
    );
}

/// Every `gen_compound_consume` (tuple/record projection, `List.len`, Option `match`, `Result` `match`)
/// COMPILES — the build+consume shapes are type-correct by construction and stay on the compile path
/// (no overflow / div0), so this exercises consumption emit and guards `gen_compound_consume` (a
/// malformed projection / match would surface as decline/InvalidWasm here). The 256 seeds hit every arm
/// across all scalar payload types (incl the `Result` Ok/Err ctors added in S97).
#[test]
fn gen_compound_consume_compiles() {
    for seed in 0u64..256 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(53);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut src = String::from("(do (def (main) ");
        gen_compound_consume(&mut ByteCursorChoice::new(&bytes), &mut src);
        src.push_str(") (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "compound-consume body must COMPILE: {src}"
        );
    }
}

/// `gen_compound_consume` REACHES a `Result` match with BOTH the `Ok` and the `Err` scrutinee ctor
/// (added S97), and every such body COMPILES — guards the sum-match consumption arm against a
/// regression that would stop generating it (silently shrinking differential reach into sum-match emit).
#[test]
fn result_match_consume_reaches_both_ctors_and_compiles() {
    let (mut saw_ok, mut saw_err) = (false, false);
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(97);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_compound_consume(&mut ByteCursorChoice::new(&bytes), &mut body);
        if !body.contains("(Result ") {
            continue; // a non-Result arm this seed
        }
        saw_ok |= body.contains("(Ok ");
        saw_err |= body.contains("(Err ");
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "Result-match consume body must COMPILE: {src}"
        );
    }
    assert!(saw_ok, "Result-match should reach the Ok scrutinee ctor");
    assert!(saw_err, "Result-match should reach the Err scrutinee ctor");
}

/// `gen_compound_consume` REACHES a sum-match over a COMPOUND payload (S102: `(Some (tuple/record/list …))`
/// consumed in-arm) covering all three payload shapes, and every such body COMPILES — guards the
/// compound-payload consumption arm (binds a native compound from a match arm + projects/List.len in-arm),
/// the fresh M2 native ctor-leaf codegen the scalar-payload matches never reach.
#[test]
fn compound_payload_match_reaches_all_shapes_and_compiles() {
    let (mut saw_tuple, mut saw_record, mut saw_list) = (false, false, false);
    for seed in 0u64..1024 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(131);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_compound_consume(&mut ByteCursorChoice::new(&bytes), &mut body);
        // The compound-payload arm is the only one that pairs `(Some (` with a native compound head.
        if !(body.contains("(Some (tuple ")
            || body.contains("(Some (record ")
            || body.contains("(Some (list "))
        {
            continue;
        }
        saw_tuple |= body.contains("(Some (tuple ");
        saw_record |= body.contains("(Some (record ");
        saw_list |= body.contains("(Some (list ");
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "compound-payload match body must COMPILE: {src}"
        );
    }
    assert!(
        saw_tuple,
        "compound-payload match should reach a tuple payload"
    );
    assert!(
        saw_record,
        "compound-payload match should reach a record payload"
    );
    assert!(
        saw_list,
        "compound-payload match should reach a list payload"
    );
}

/// `gen_compound_consume` REACHES native `#set`/`#map` literals (S110: fills the Set/Map codegen gap),
/// and every such body COMPILES — guards the set/map arm (native leaf kinds 23/24: `#set(…)`, its
/// `Set.len`, `#map((= k v) …)`, its `Map.len`). A malformed literal / removed op surfaces here.
#[test]
fn set_map_literals_are_reached_and_compile() {
    let (mut saw_set, mut saw_map) = (false, false);
    for seed in 0u64..1024 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(211);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_compound_consume(&mut ByteCursorChoice::new(&bytes), &mut body);
        if !(body.contains("#set(") || body.contains("#map(")) {
            continue;
        }
        saw_set |= body.contains("#set(");
        saw_map |= body.contains("#map(");
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "set/map literal body must COMPILE: {src}"
        );
    }
    assert!(saw_set, "should reach a #set literal");
    assert!(saw_map, "should reach a #map literal");
}

/// `build_program` REACHES the USER-SUM shape (S140) — a top-level `(type …)` + a construct/match main
/// — for BOTH the multi-variant tagged sum (`type Shape`) and the single-variant newtype (`type Pt`),
/// and every such program COMPILES. Guards the user-sum arm (a malformed decl/ctor/pattern would
/// decline here). Top-level `(type …)` is required to GRADE (a local one SKIPs) — this pins it top-level.
#[test]
fn build_program_reaches_user_sum_shapes_and_compiles() {
    let (mut saw_multi, mut saw_newtype, mut saw_nullary) = (false, false, false);
    for seed in 0u64..1024 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(719);
        let mut bytes = Vec::new();
        for _ in 0..24 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let src = build_program(&mut ByteCursorChoice::new(&bytes)).source;
        if !src.contains("(type ") {
            continue; // not the user-sum shape this seed
        }
        // The top-level type decl must precede `(def (main)` (pins it top-level, not in-body).
        assert!(
            src.find("(type ").unwrap() < src.find("(def (main)").unwrap(),
            "the `(type …)` must be a TOP-LEVEL decl (before main), else it SKIPs: {src}"
        );
        saw_multi |= src.contains("(type Shape ");
        saw_newtype |= src.contains("(type Pt ");
        saw_nullary |= src.contains("(type Color ");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "user-sum program must COMPILE: {src}"
        );
    }
    assert!(
        saw_multi,
        "should reach a multi-variant tagged sum (type Shape)"
    );
    assert!(
        saw_newtype,
        "should reach a single-variant newtype (type Pt)"
    );
    assert!(saw_nullary, "should reach a nullary-ctor enum (type Color)");
}

/// `build_program` REACHES the RECURSIVE-PERFORM effect shape — a top-level recursive `loop` that
/// PERFORMS the op deep inside itself, discharged by `main`'s enclosing `handle` — and every such
/// program COMPILES. The perform is cross-function (inside `loop`, not lexically in the handle body):
/// both `(effect …)` and `(def (loop …))` must be TOP-LEVEL (a locally-nested perform has no home =
/// CDZ0401, and a local def SKIPs in the oracle) — this pins the shape top-level. Also asserts the
/// resume-value spread reaches both `s` (identity) and `(+ s p)` (fold).
#[test]
fn build_program_reaches_recursive_perform_effect_and_compiles() {
    let (mut saw, mut saw_ident, mut saw_fold) = (false, false, false);
    for seed in 0u64..1024 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(929);
        let mut bytes = Vec::new();
        for _ in 0..24 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let src = build_program(&mut ByteCursorChoice::new(&bytes)).source;
        if !src.contains("(def (loop ") {
            continue; // not the recursive-perform shape this seed
        }
        saw = true;
        // The effect decl + loop def must be TOP-LEVEL (before main), else the perform has no home.
        assert!(
            src.find("(effect E").unwrap() < src.find("(def (main)").unwrap(),
            "the `(effect …)` + `loop` must be TOP-LEVEL (before main): {src}"
        );
        saw_ident |= src.contains("(resume s ");
        saw_fold |= src.contains("(resume (+ s p)");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "recursive-perform effect program must COMPILE: {src}"
        );
    }
    assert!(saw, "should reach the recursive-perform effect shape");
    assert!(
        saw_ident,
        "should reach the identity resume value (resume s …)"
    );
    assert!(
        saw_fold,
        "should reach the folding resume value (resume (+ s p) …)"
    );
}

/// `build_program` REACHES the CROSS-MODULE shape — a top-level inline `(module M …)` exporting a
/// function that `main` calls across the boundary — and every such program COMPILES. The module MUST
/// be top-level (before main), pinning it a whole-program shape. Asserts all three forms (scalar,
/// two-arg, compound-result) are reached.
#[test]
fn build_program_reaches_cross_module_shape_and_compiles() {
    // Each cross-module form crosses the boundary as a distinct type; every one must compile.
    let markers = [
        "(M.f ", "(M.g ", "(M.mk ", "(M.tup ", "(M.opt ", "(M.rec ", "(M.lt ", "(M.big ",
    ];
    let mut seen = [false; 8];
    let mut saw = false;
    for seed in 0u64..2048 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1013);
        let mut bytes = Vec::new();
        for _ in 0..24 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let src = build_program(&mut ByteCursorChoice::new(&bytes)).source;
        if !src.contains("(module M ") {
            continue; // not the cross-module shape this seed
        }
        saw = true;
        assert!(
            src.find("(module M ").unwrap() < src.find("(def (main)").unwrap(),
            "the `(module …)` must be a TOP-LEVEL decl (before main): {src}"
        );
        for (i, m) in markers.iter().enumerate() {
            seen[i] |= src.contains(m);
        }
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "cross-module program must COMPILE: {src}"
        );
    }
    assert!(saw, "should reach the cross-module shape");
    for (i, m) in markers.iter().enumerate() {
        assert!(seen[i], "should reach the cross-module form {m}");
    }
}

/// `build_program` REACHES the RECURSIVE COLLECTION-BUILDER shape — a top-level recursive `def` that
/// grows a List/Map across its calls, consumed by `main` — and every such program COMPILES. The
/// builder def must be TOP-LEVEL (a local recursive def SKIPs in the oracle). Asserts both the List
/// (`build`) and Map (`bm`) builders are reached.
#[test]
fn build_program_reaches_recursive_collection_builder_and_compiles() {
    let (mut saw, mut saw_list, mut saw_map) = (false, false, false);
    for seed in 0u64..2048 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(2027);
        let mut bytes = Vec::new();
        for _ in 0..24 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let src = build_program(&mut ByteCursorChoice::new(&bytes)).source;
        if !src.contains("(def (build ") && !src.contains("(def (bm ") {
            continue; // not the recursive-collection-builder shape this seed
        }
        saw = true;
        let builder = if src.contains("(def (build ") {
            "(def (build "
        } else {
            "(def (bm "
        };
        assert!(
            src.find(builder).unwrap() < src.find("(def (main)").unwrap(),
            "the recursive builder def must be TOP-LEVEL (before main): {src}"
        );
        saw_list |= src.contains("(def (build ");
        saw_map |= src.contains("(def (bm ");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "recursive collection-builder program must COMPILE: {src}"
        );
    }
    assert!(saw, "should reach the recursive collection-builder shape");
    assert!(saw_list, "should reach the List builder (build)");
    assert!(saw_map, "should reach the Map builder (bm)");
}

/// `gen_bignum_body` REACHES both BigInt (`N`) and Rational (`R`) forms and every body COMPILES (S132:
/// fills the BigInt/Rational numeric-family gap). BigInt `+`/`-`/`*` never overflow; Rational `/` uses a
/// nonzero denominator — so all stay on the compile path (they SKIP in the value oracle for now).
#[test]
fn gen_bignum_body_reaches_bigint_and_rational_and_compiles() {
    let (mut saw_n, mut saw_r, mut saw_cmp) = (false, false, false);
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(613);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_bignum_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        saw_n |= body.contains('N');
        saw_r |= body.contains('R');
        // A comparison body begins with a comparison op head (arith ops are `+ - * /`).
        saw_cmp |= body.starts_with("(=") || body.starts_with("(<") || body.starts_with("(>");
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "bignum body must COMPILE: {src}"
        );
    }
    assert!(saw_n, "should reach a BigInt (N) form");
    assert!(saw_r, "should reach a Rational (R) form");
    assert!(saw_cmp, "should reach a BigInt/Rational comparison");
}

/// `gen_qty_body` REACHES both the bare `Qty.of` literal form and the same-unit arithmetic form,
/// exercises a PARENTHESIZED magnitude (the #7227 regression guard), and every body COMPILES —
/// filling the Qty numeric-family gap (Qty was absent from the coercing/value-comparable grammar).
#[test]
fn gen_qty_body_reaches_all_forms_and_compiles() {
    let (mut saw_lit, mut saw_arith, mut saw_grouped) = (false, false, false);
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(937);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_qty_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        // A bare-literal body is `(Qty.value (Qty.of …))`; an arithmetic body wraps an op.
        saw_arith |= body.starts_with("(Qty.value (+")
            || body.starts_with("(Qty.value (-")
            || body.starts_with("(Qty.value (*");
        saw_lit |= body.starts_with("(Qty.value (Qty.of");
        // A parenthesized magnitude renders `(Qty.of (n) …)` (grouped literal — #7227 guard).
        saw_grouped |= body.contains("(Qty.of (");
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "qty body must COMPILE: {src}"
        );
    }
    assert!(saw_lit, "should reach a bare Qty.of literal");
    assert!(
        saw_arith,
        "should reach a Qty same-unit arithmetic combination"
    );
    assert!(
        saw_grouped,
        "should reach a parenthesized (grouped) magnitude"
    );
}

/// `gen_map_lookup_body` REACHES both a PRESENT-key lookup (the `Some` arm, key `0..=9`) and an
/// ABSENT-key lookup (the `None` arm, key `99`), and every body COMPILES — filling the Map.lookup
/// (keyed read → `Option V`) gap the coercing grammar's Map.len-only coverage never reached.
#[test]
fn gen_map_lookup_body_reaches_present_and_absent_and_compiles() {
    let (mut saw_present, mut saw_absent) = (false, false);
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1049);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_map_lookup_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        // The absent lookup uses the sentinel key ` 99)`; a present lookup uses a `0..=9` key.
        saw_absent |= body.contains(" 99) ((Some");
        saw_present |= !body.contains(" 99) ((Some") && body.contains("(Map.lookup");
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "map-lookup body must COMPILE: {src}"
        );
    }
    assert!(saw_present, "should reach a present-key lookup (Some arm)");
    assert!(saw_absent, "should reach an absent-key lookup (None arm)");
}

/// `gen_collection_op_body` REACHES all six op forms (Set.union, Set.remove, Set.contains,
/// Map.remove, Set.intersection, Set.difference) and every body COMPILES — filling the set-merge /
/// element-removal / membership / set-algebra gap the coercing grammar's Set.len/insert +
/// Map.len/lookup coverage never reached.
#[test]
fn gen_collection_op_body_reaches_all_forms_and_compiles() {
    let (
        mut saw_union,
        mut saw_sremove,
        mut saw_contains,
        mut saw_mremove,
        mut saw_intersection,
        mut saw_difference,
        mut saw_merge,
    ) = (false, false, false, false, false, false, false);
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1163);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_collection_op_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        saw_union |= body.contains("Set.union");
        saw_sremove |= body.contains("Set.remove");
        saw_contains |= body.contains("Set.contains");
        saw_mremove |= body.contains("Map.remove");
        saw_intersection |= body.contains("Set.intersection");
        saw_difference |= body.contains("Set.difference");
        saw_merge |= body.contains("Map.merge");
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "collection-op body must COMPILE: {src}"
        );
    }
    assert!(saw_union, "should reach Set.union");
    assert!(saw_sremove, "should reach Set.remove");
    assert!(saw_contains, "should reach Set.contains");
    assert!(saw_mremove, "should reach Map.remove");
    assert!(saw_intersection, "should reach Set.intersection");
    assert!(saw_difference, "should reach Set.difference");
    assert!(saw_merge, "should reach Map.merge");
}

/// `gen_effect_body` emits a well-formed value-comparable EFFECT program (effect decl + stateful
/// handler + tail-resume + twice-performed op) and every body COMPILES — the effect-SEMANTICS
/// value-coverage the coercing grammar never reached (effects were crash-checked only). Also asserts
/// the resume-value form spread reaches both the state-folding `(+ s p)` and a bare param/literal.
#[test]
fn gen_effect_body_is_well_formed_and_compiles() {
    let (mut saw_handle, mut saw_resume, mut saw_statefold, mut saw_bare, mut saw_abort) =
        (false, false, false, false, false);
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1481);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_effect_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        saw_handle |= body.contains("(handle E");
        saw_resume |= body.contains("(resume ");
        saw_statefold |= body.contains("(resume (+ s p)");
        saw_bare |= body.contains("(resume s ") || body.contains("(resume p ");
        // An ABORT body is a (handle …) with NO (resume …) — the arm returns a value directly.
        saw_abort |= body.contains("(handle E") && !body.contains("(resume ");
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "effect body must COMPILE: {src}"
        );
    }
    assert!(saw_handle, "effect body should emit a (handle E …)");
    assert!(saw_resume, "effect body should emit a (resume …)");
    assert!(
        saw_statefold,
        "effect body should reach the state-folding resume value (+ s p)"
    );
    assert!(
        saw_bare,
        "effect body should reach a bare param resume value"
    );
    assert!(
        saw_abort,
        "effect body should reach the ABORT (non-resumptive) form"
    );
}

/// `gen_effect_multiop_body` emits a well-formed TWO-op effect program (one effect declaring `o1`+`o2`,
/// a per-op handler arm, a body performing BOTH) and every body COMPILES — the op-dispatch value
/// coverage the single-op handler never reached. Asserts both ops + both arms are present.
#[test]
fn gen_effect_multiop_body_is_well_formed_and_compiles() {
    let (mut saw_two_ops, mut saw_both_performs) = (false, false);
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1601);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_effect_multiop_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        saw_two_ops |= body.contains("(op o1 ") && body.contains("(op o2 ");
        saw_both_performs |= body.contains("(E.o1 ") && body.contains("(E.o2 ");
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "multi-op effect body must COMPILE: {src}"
        );
    }
    assert!(saw_two_ops, "should declare two ops (o1 + o2)");
    assert!(saw_both_performs, "should perform both ops (E.o1 + E.o2)");
}

/// `gen_effect_mapstate_body` emits a well-formed HEAP-STATE handler-threading program (a `bump` op
/// threading a `Map` handler-state through `resume`, seeded with the immortal `Map.empty`, with an
/// immortal-unchanged-seed guard arm) and every body COMPILES — the handler-state-threading reclaim
/// family (#9522/#9525) that the scalar-Int64-state effect forms never reach. Asserts the Map-state
/// structure is present, and confirms the KNOWN value 21 (state threaded correctly across two bumps).
#[test]
fn gen_effect_mapstate_body_is_well_formed_and_compiles() {
    let mut saw_mapstate = false;
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(2609);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_effect_mapstate_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        saw_mapstate |= body.contains("(handle T Map.empty")
            && body.contains("(Map.insert s key")
            && body.contains("(resume -1 s)");
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "map-state effect body must COMPILE: {src}"
        );
    }
    assert!(
        saw_mapstate,
        "should thread a Map handler-state (handle Map.empty + Map.insert + immortal-seed guard arm)"
    );
}

/// `gen_effect_cfjoin_body` emits a well-formed CONTROL-FLOW-JOIN handler-state program (a `tick` op
/// whose resume NEW-STATE is an `(if …)` producing a nested tuple — the #9532→#9533 SumPayload-reclaim-
/// off-a-join-scrutinee shape) and every body COMPILES to a VALID component (a regression relaxing the
/// join-scrutinee reclaim fence re-emits the resume continuation → u32::MAX → CDZ0910 invalid-wasm, which
/// a bare compile-check would FAIL here). Asserts the nested-tuple state + the control-flow-join new-state.
#[test]
fn gen_effect_cfjoin_body_is_well_formed_and_compiles() {
    let mut saw_cfjoin = false;
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(3313);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_effect_cfjoin_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        saw_cfjoin |= body.contains("(op tick ")
            && body.contains("#tuple(#tuple(")
            && body.contains("(if (= (% c 2) 0)");
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "control-flow-join effect body must COMPILE to a valid component: {src}"
        );
    }
    assert!(
        saw_cfjoin,
        "should build a nested-tuple state machine with a control-flow-join (if) resume new-state"
    );
}

/// `gen_effect_discarded_call_program` emits a well-formed DISCARDED-PERFORM effect program (a discarded
/// non-tail call to a top-level `bump` helper that performs `E.o`, whose effect threads the handler state
/// read by the KEPT tail perform). Emits the FULL program (a local performing `def` declines CDZ0401), so
/// it is NOT main-body-wrapped. Every program COMPILES; asserts the discarded-call structure + confirms
/// the value `s0 + a` (a>=1 so the discarded effect always shifts the value). NOTE: this is NOT a #9606
/// DCE tripwire — the perform is KEPT by subtree_reaches_effect_perform (see the generator doc).
#[test]
fn gen_effect_discarded_call_program_is_well_formed_and_compiles() {
    let mut saw_discarded = false;
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(4099);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut src = String::new();
        gen_effect_discarded_call_program(&mut ByteCursorChoice::new(&bytes), &mut src);
        saw_discarded |=
            src.contains("(def (bump (: x Int64)) (E.o x))") && src.contains("(do (bump ");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "discarded-call DCE program must COMPILE: {src}"
        );
    }
    assert!(
        saw_discarded,
        "should emit a top-level performing `bump` helper called in discarded non-tail position"
    );
}

/// `gen_effect_splat_handler_program` emits a well-formed #9642 splat-in-handler-body program (a handler
/// body applies a top-level `one` to a splat `(.. #tuple((T.tick)))` of a 1-tuple containing a perform;
/// the splat must expand to perform exactly once and thread the state). Emits the FULL program (a
/// top-level `one`), so it is NOT main-body-wrapped. Every program COMPILES (the expander lifts the
/// statically-expandable splat); asserts the splat + perform structure + confirms the value `s0 * b`
/// (s0>=1, b>=2 so the multiply is load-bearing).
#[test]
fn gen_effect_splat_handler_program_is_well_formed_and_compiles() {
    let mut saw_splat = false;
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(5501);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut src = String::new();
        gen_effect_splat_handler_program(&mut ByteCursorChoice::new(&bytes), &mut src);
        saw_splat |= src.contains("(one (.. #tuple((T.tick))))") && src.contains("(op tick ");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "splat-in-handler effect program must COMPILE (the expander lifts the splat): {src}"
        );
    }
    assert!(
        saw_splat,
        "should emit an effectful-operand splat `(.. #tuple((T.tick)))` applied in a handler body"
    );
}

/// `gen_effect_collection_body` emits a well-formed EFFECT × COLLECTION program (the handled body
/// builds a `(list …)` of PERFORM results, consumed by `List.len`/`List.at`) and every body COMPILES —
/// the effect-value × collection-marshal interaction the single-shape arms never combine. Asserts both
/// forms (List.len / List.at) are reached.
#[test]
fn gen_effect_collection_body_is_well_formed_and_compiles() {
    let (mut saw_len, mut saw_at) = (false, false);
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1867);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_effect_collection_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        // Every one builds a list of performs inside a handle.
        assert!(
            body.contains("(handle E") && body.contains("(list (E.o "),
            "effect-collection body must build a list of performs: {body}"
        );
        saw_len |= body.contains("(List.len (list (E.o ");
        saw_at |= body.contains("(List.at (list (E.o ");
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "effect-collection body must COMPILE: {src}"
        );
    }
    assert!(saw_len, "should reach the List.len form");
    assert!(saw_at, "should reach the List.at form");
}

/// `gen_effect_nested_body` emits a well-formed NESTED-HANDLER effect program (two effects, the E2
/// handle nested inside the E1 handle, both performed) and every body COMPILES — the multi-frame
/// handler-stack resolution the single-handler shapes never reached. Asserts both effects + a nested
/// (two-`handle`) structure are present.
#[test]
fn gen_effect_nested_body_is_well_formed_and_compiles() {
    let (mut saw_two_effects, mut saw_nested) = (false, false);
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1733);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_effect_nested_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        saw_two_effects |= body.contains("(effect E1 ") && body.contains("(effect E2 ");
        saw_nested |= body.matches("(handle ").count() >= 2;
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "nested-handler effect body must COMPILE: {src}"
        );
    }
    assert!(saw_two_effects, "should declare two effects (E1 + E2)");
    assert!(saw_nested, "should nest two (handle …) frames");
}

/// `gen_list_producing_op_body` REACHES all forms (List.push, List.prepend, Set.to-list, Map.to-list)
/// and every body COMPILES — filling the list-BUILDING collection ops the coercing grammar never
/// reached.
#[test]
fn gen_list_producing_op_body_reaches_all_forms_and_compiles() {
    let (mut saw_push, mut saw_settolist, mut saw_maptolist, mut saw_prepend) =
        (false, false, false, false);
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1277);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_list_producing_op_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        saw_push |= body.contains("List.push");
        saw_settolist |= body.contains("Set.to-list");
        saw_maptolist |= body.contains("Map.to-list");
        saw_prepend |= body.contains("List.prepend");
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "list-producing-op body must COMPILE: {src}"
        );
    }
    assert!(saw_push, "should reach List.push");
    assert!(saw_prepend, "should reach List.prepend");
    assert!(saw_settolist, "should reach Set.to-list");
    assert!(saw_maptolist, "should reach Map.to-list");
}

/// `gen_partial_application_body` REACHES all three currying forms (2-ary `let`-partial, 3-ary chained,
/// 3-ary 2-arg `let`-partial) and every body COMPILES (S143: fills the partial-application gap that
/// #5488 now grades — a local def under-applied → a closure over the remaining params, later completed).
#[test]
fn gen_partial_application_body_reaches_all_forms_and_compiles() {
    let (mut saw_2ary, mut saw_chain, mut saw_2arg) = (false, false, false);
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(827);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_partial_application_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        saw_2ary |= body.contains("(def (pa a b)");
        saw_chain |= body.contains("(((pa3 ");
        saw_2arg |= body.contains("((g (pa3 ");
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "partial-application body must COMPILE: {src}"
        );
    }
    assert!(saw_2ary, "should reach a 2-ary `let`-partial form");
    assert!(saw_chain, "should reach a 3-ary chained-currying form");
    assert!(saw_2arg, "should reach a 3-ary 2-arg `let`-partial form");
}

/// `gen_higher_order_body` REACHES all three higher-order forms (lambda-applied-once, lambda-applied-
/// twice, named-def-as-value) and every body COMPILES (S146: a fn value passed as an argument and
/// applied inside another def — the applyClosure-over-a-closure-valued-param path).
#[test]
fn gen_higher_order_body_reaches_all_forms_and_compiles() {
    let (mut saw_once, mut saw_twice, mut saw_named) = (false, false, false);
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(941);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_higher_order_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        saw_once |= body.contains("(def (apply f x) (f x)) (apply (fn ");
        saw_twice |= body.contains("(def (twice g n)");
        saw_named |= body.contains("(apply inc ");
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "higher-order body must COMPILE: {src}"
        );
    }
    assert!(saw_once, "should reach a lambda-applied-once form");
    assert!(saw_twice, "should reach a lambda-applied-twice form");
    assert!(saw_named, "should reach a named-def-as-value form");
}

/// `gen_discard_body` REACHES all four discarded-value kinds (scalar, tuple, list, bool) and every body
/// COMPILES (S148: a non-def leading do-statement is computed then discarded — the sequencing/dead-value
/// drop lowering #5507 grades).
#[test]
fn gen_discard_body_reaches_all_kinds_and_compiles() {
    let (mut saw_scalar, mut saw_tuple, mut saw_list, mut saw_bool) = (false, false, false, false);
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1187);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_discard_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        saw_tuple |= body.contains("(do (tuple ");
        saw_list |= body.contains("(do (list ");
        saw_bool |= body.contains("(do (< ");
        saw_scalar |= body.starts_with("(do ")
            && !body.contains("(do (tuple ")
            && !body.contains("(do (list ")
            && !body.contains("(do (< ");
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "discard body must COMPILE: {src}"
        );
    }
    assert!(saw_scalar, "should reach a discarded-scalar form");
    assert!(saw_tuple, "should reach a discarded-tuple form");
    assert!(saw_list, "should reach a discarded-list form");
    assert!(saw_bool, "should reach a discarded-bool form");
}

/// `gen_float_ordering_body` REACHES both widths (Float64, Float32) and all four ordering relations
/// (`< > <= >=`), and every body COMPILES (S149: float ordering as the returned Bool value — #5519).
#[test]
fn gen_float_ordering_body_reaches_both_widths_and_all_rels_and_compiles() {
    let (mut saw_f64, mut saw_f32, mut saw_nan, mut saw_inf) = (false, false, false, false);
    let mut rels_seen = std::collections::BTreeSet::new();
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1291);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_float_ordering_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        saw_nan |= body.contains("Float64.nan");
        saw_inf |= body.contains("Float64.Infinity");
        if body.contains("Float32") {
            saw_f32 = true;
        } else {
            saw_f64 = true;
        }
        for rel in ["<=", ">=", "<", ">"] {
            if body.starts_with(&format!("({rel} ")) {
                rels_seen.insert(rel);
                break;
            }
        }
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "float-ordering body must COMPILE: {src}"
        );
    }
    assert!(saw_f64, "should reach a Float64 ordering");
    assert!(saw_f32, "should reach a Float32 ordering");
    assert!(saw_nan, "should reach a NaN-operand (Float64.nan) ordering");
    assert!(
        saw_inf,
        "should reach an Infinity-operand (Float64.Infinity) ordering"
    );
    assert_eq!(
        rels_seen.len(),
        4,
        "should reach all four ordering relations"
    );
}

/// `gen_compound_keyed_collection_body` REACHES all three forms (compound-keyed set, `Set.insert`,
/// compound-keyed map) and every body COMPILES (S154: sets/maps keyed by `(tuple …)` compounds — the
/// structural total order over compound values #5540 grades).
#[test]
fn gen_compound_keyed_collection_body_reaches_all_forms_and_compiles() {
    let (mut saw_set, mut saw_insert, mut saw_map) = (false, false, false);
    let (mut saw_tuple_key, mut saw_record_key, mut saw_nested_key, mut saw_list_key) =
        (false, false, false, false);
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1409);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_compound_keyed_collection_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        saw_insert |= body.contains("Set.insert");
        saw_set |= body.starts_with("(Set.len #set(");
        saw_map |= body.contains("Map.len");
        saw_record_key |= body.contains("(record ");
        saw_nested_key |= body.contains("(tuple (tuple ");
        saw_list_key |= body.contains("(list ");
        saw_tuple_key |= body.contains("(tuple ") && !body.contains("(tuple (tuple ");
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "compound-keyed collection body must COMPILE: {src}"
        );
    }
    assert!(saw_set, "should reach a compound-keyed set (Set.len #set)");
    assert!(saw_insert, "should reach a Set.insert form");
    assert!(saw_map, "should reach a compound-keyed map (Map.len)");
    assert!(saw_tuple_key, "should reach a flat-tuple-keyed form");
    assert!(saw_record_key, "should reach a record-keyed form");
    assert!(saw_nested_key, "should reach a nested-tuple-keyed form");
    assert!(saw_list_key, "should reach a list-keyed form");
}

/// `gen_float_keyed_collection_body` REACHES all four forms (Float64 set, Float64 map, NaN-key set,
/// Float32 set) and every body COMPILES (S157: float-carrying set/map keys — canonical-bit order +
/// canonical key equality + NaN keys, #5556).
#[test]
fn gen_float_keyed_collection_body_reaches_all_forms_and_compiles() {
    let (mut saw_f64_set, mut saw_f64_map, mut saw_f32, mut saw_nan) = (false, false, false, false);
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1523);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_float_keyed_collection_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        saw_f32 |= body.contains("Float32");
        saw_nan |= body.contains("Float64.nan");
        saw_f64_map |= body.starts_with("(Map.len");
        saw_f64_set |= body.starts_with("(Set.len #set(")
            && !body.contains("Float32")
            && !body.contains("Float64.nan");
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "float-keyed collection body must COMPILE: {src}"
        );
    }
    assert!(saw_f64_set, "should reach a Float64-keyed set");
    assert!(saw_f64_map, "should reach a Float64-keyed map");
    assert!(saw_f32, "should reach a Float32-keyed set");
    assert!(saw_nan, "should reach a NaN-key set");
}

/// `gen_string_body` REACHES all five String-op forms (byte-len, scalar-at, concat, slice, bare literal)
/// and every body COMPILES (S166: a String-op family the Int64/float/compound grammar never reached).
#[test]
fn gen_string_body_reaches_all_forms_and_compiles() {
    let (
        mut saw_len,
        mut saw_at,
        mut saw_concat,
        mut saw_slice,
        mut saw_lit,
        mut saw_cmp,
        mut saw_char_cmp,
    ) = (false, false, false, false, false, false, false);
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1657);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_string_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        saw_len |= body.contains("String.byte-len");
        saw_at |= body.contains("String.scalar-at");
        saw_concat |= body.contains("String.concat");
        saw_slice |= body.contains("String.slice");
        // A comparison body begins with an op head; a CHAR comparison compares two
        // `String.scalar-at` results (contains scalar-at), a STRING comparison two literals.
        let is_cmp = body.starts_with("(=") || body.starts_with("(<") || body.starts_with("(>");
        saw_char_cmp |= is_cmp && body.contains("String.scalar-at");
        saw_cmp |= is_cmp && !body.contains("String.scalar-at");
        saw_lit |= body.starts_with('"');
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "string-op body must COMPILE: {src}"
        );
    }
    assert!(saw_len, "should reach String.byte-len");
    assert!(saw_at, "should reach String.scalar-at");
    assert!(saw_concat, "should reach String.concat");
    assert!(saw_slice, "should reach String.slice");
    assert!(saw_lit, "should reach a bare string literal");
    assert!(saw_cmp, "should reach a string comparison");
    assert!(saw_char_cmp, "should reach a char (scalar-at) comparison");
}

/// `gen_bytes_body` REACHES all five Bytes-op forms (len, at, literal, of-list, concat) and every body
/// COMPILES (S167: the Bytes construct family — distinct from String and numeric/compound grammar).
#[test]
fn gen_bytes_body_reaches_all_forms_and_compiles() {
    let (mut saw_len, mut saw_at, mut saw_lit, mut saw_of, mut saw_concat, mut saw_cmp) =
        (false, false, false, false, false, false);
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1789);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_bytes_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        saw_len |= body.contains("Bytes.len");
        saw_at |= body.contains("Bytes.at");
        saw_of |= body.contains("Bytes.of");
        saw_concat |= body.contains("Bytes.concat");
        saw_lit |= body.starts_with("b\"");
        // A bytes COMPARISON body begins with an op head over two b"…" byte values.
        saw_cmp |= body.starts_with("(=") || body.starts_with("(<") || body.starts_with("(>");
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "bytes-op body must COMPILE: {src}"
        );
    }
    assert!(saw_len, "should reach Bytes.len");
    assert!(saw_at, "should reach Bytes.at");
    assert!(saw_lit, "should reach a b\"…\" literal");
    assert!(saw_of, "should reach Bytes.of");
    assert!(saw_concat, "should reach Bytes.concat");
    assert!(saw_cmp, "should reach a bytes comparison");
}

/// `gen_nested_compound_body` REACHES all five forms (List.at, List.concat, tuple-of-lists,
/// list-of-tuples, record-of-compounds) and every body COMPILES (S168: deeper structural shapes than
/// the flat single-level compound arms).
#[test]
fn gen_nested_compound_body_reaches_all_forms_and_compiles() {
    let (mut saw_at, mut saw_concat, mut saw_tol, mut saw_lot, mut saw_rec) =
        (false, false, false, false, false);
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1913);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_nested_compound_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        saw_at |= body.contains("List.at");
        saw_concat |= body.contains("List.concat");
        saw_tol |= body.starts_with("(tuple (list ");
        saw_lot |= body.starts_with("(list (tuple ");
        saw_rec |= body.starts_with("(record (= a (tuple ");
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "nested-compound body must COMPILE: {src}"
        );
    }
    assert!(saw_at, "should reach List.at");
    assert!(saw_concat, "should reach List.concat");
    assert!(saw_tol, "should reach a tuple-of-lists");
    assert!(saw_lot, "should reach a list-of-tuples");
    assert!(saw_rec, "should reach a record-of-compounds");
}

/// `gen_nested_sum_body` REACHES all four forms (Option-of-Option, Result-of-Option, Option-of-tuple,
/// Option-of-list) and every body COMPILES (S169: deeper sum-wrapping than the flat Some/Ok/Err arms).
#[test]
fn gen_nested_sum_body_reaches_all_forms_and_compiles() {
    let (mut saw_oo, mut saw_ro, mut saw_ot, mut saw_ol) = (false, false, false, false);
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(2039);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_nested_sum_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        saw_oo |= body.contains("(Some (Some ");
        saw_ro |= body.contains("(Ok (Some ");
        saw_ot |= body.contains("(Some (tuple ");
        saw_ol |= body.contains("(Some (list ");
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "nested-sum body must COMPILE: {src}"
        );
    }
    assert!(saw_oo, "should reach Option-of-Option");
    assert!(saw_ro, "should reach Result-of-Option");
    assert!(saw_ot, "should reach Option-of-tuple");
    assert!(saw_ol, "should reach Option-of-list");
}

/// `gen_int_conversion_body` reaches a BREADTH of Source/Target int-type pairs (≥4 distinct targets +
/// ≥4 distinct sources) and every `(<Target>.of (: <v> <Source>))` body COMPILES (S170: int cross-width
/// conversion codegen — widen/narrow/cross-sign).
#[test]
fn gen_int_conversion_body_reaches_breadth_and_compiles() {
    let mut targets = std::collections::BTreeSet::new();
    let mut sources = std::collections::BTreeSet::new();
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(2161);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_int_conversion_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        for t in SIZED_INT_TYPES {
            if body.starts_with(&format!("({t}.of ")) {
                targets.insert(*t);
            }
            if body.contains(&format!(" {t}))")) {
                sources.insert(*t);
            }
        }
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "int-conversion body must COMPILE: {src}"
        );
    }
    assert!(targets.len() >= 4, "should reach >=4 distinct target types");
    assert!(sources.len() >= 4, "should reach >=4 distinct source types");
}

/// `gen_wide_compound_body` REACHES all five wider-arity forms (3-tuple, 4-tuple, 3-record, 3-tuple
/// projection, 3-record projection) and every body COMPILES (S171: wider construction/projection).
#[test]
fn gen_wide_compound_body_reaches_all_forms_and_compiles() {
    let (mut saw_t3, mut saw_t4, mut saw_r3, mut saw_pt, mut saw_pr) =
        (false, false, false, false, false);
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(2293);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_wide_compound_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        saw_pt |= body.starts_with("(. (tuple ");
        saw_pr |= body.starts_with("(. (record ");
        saw_t4 |= body.starts_with("(tuple ") && body.matches(' ').count() >= 4;
        saw_t3 |= body.starts_with("(tuple ") && body.matches(' ').count() == 3;
        saw_r3 |= body.starts_with("(record ");
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "wide-compound body must COMPILE: {src}"
        );
    }
    assert!(saw_t3, "should reach a 3-tuple");
    assert!(saw_t4, "should reach a 4-tuple");
    assert!(saw_r3, "should reach a 3-field record");
    assert!(saw_pt, "should reach a tuple projection");
    assert!(saw_pr, "should reach a record projection");
}

/// `gen_bool_logic_body` REACHES all four forms (and, or, not, nested) and every body COMPILES
/// (S174: short-circuit boolean combinators over comparisons).
#[test]
fn gen_bool_logic_body_reaches_all_forms_and_compiles() {
    let (mut saw_and, mut saw_or, mut saw_not, mut saw_nested) = (false, false, false, false);
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(2417);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_bool_logic_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        saw_and |= body.contains("(and ");
        saw_or |= body.contains("(or ");
        saw_not |= body.contains("(not ");
        // A DEEP shape: >= 2 boolean combinators = a bool op nested inside another (recursion working).
        let combinators = body.matches("(and ").count()
            + body.matches("(or ").count()
            + body.matches("(not ").count();
        saw_nested |= combinators >= 2;
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "bool-logic body must COMPILE: {src}"
        );
    }
    assert!(saw_and, "should reach an `and`");
    assert!(saw_or, "should reach an `or`");
    assert!(saw_not, "should reach a `not`");
    assert!(
        saw_nested,
        "should reach a DEEP (recursively-nested) bool shape"
    );
}

/// `gen_sized_shift_body` REACHES all three forms (shift-left, shift-right, nested shift+and) over a
/// breadth of sized-int types, and every body COMPILES (S175: narrow-width shift codegen).
#[test]
fn gen_sized_shift_body_reaches_all_forms_and_compiles() {
    let (mut saw_shl, mut saw_shr, mut saw_nested) = (false, false, false);
    let mut types = std::collections::BTreeSet::new();
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(2549);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_sized_shift_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        saw_nested |= body.starts_with("(& (<< ");
        saw_shl |= body.starts_with("(<< ");
        saw_shr |= body.starts_with("(>> ");
        for t in SIZED_INT_TYPES {
            if body.contains(&format!(" {t})")) {
                types.insert(*t);
            }
        }
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "sized-shift body must COMPILE: {src}"
        );
    }
    assert!(saw_shl, "should reach a shift-left");
    assert!(saw_shr, "should reach a shift-right");
    assert!(saw_nested, "should reach a nested shift+and");
    assert!(
        types.len() >= 4,
        "should reach >=4 distinct sized-int types"
    );
}

/// `gen_mutual_recursion_body` REACHES both forms (even/odd Bool parity, ping/pong Int accumulator), the
/// defs are TOP-LEVEL (assembled before `main`, so they GRADE — a local recursive def SKIPs), and every
/// program COMPILES (S147: two top-level defs calling each other — a mutual call graph no single
/// self-recursive helper reaches).
#[test]
fn gen_mutual_recursion_body_reaches_both_forms_and_compiles() {
    let (mut saw_parity, mut saw_pingpong) = (false, false);
    for seed in 0u64..512 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1063);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let (defs, body) = gen_mutual_recursion_body(&mut ByteCursorChoice::new(&bytes));
        saw_parity |= defs.contains("(def (ev n)");
        saw_pingpong |= defs.contains("(def (pinga n acc)");
        let src = format!("(do {defs} (def (main) {body}) (export main))");
        // The mutually-recursive defs must precede `(def (main)` (top-level, so they resolve + GRADE).
        assert!(
            src.find("(def (main)").unwrap()
                > src
                    .find("(def (ev n)")
                    .or_else(|| src.find("(def (pinga n acc)"))
                    .unwrap(),
            "mutual-recursion defs must be TOP-LEVEL (before main): {src}"
        );
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "mutual-recursion program must COMPILE: {src}"
        );
    }
    assert!(saw_parity, "should reach the even/odd parity form");
    assert!(saw_pingpong, "should reach the ping/pong accumulator form");
}

/// `gen_try_body` REACHES all four `?`/`try` forms (Ok/Err success+short-circuit for Result, Some/None
/// for Option) and every body COMPILES (S118: fills the `?`/try codegen gap #5249 unlocked). Guards the
/// try arm — a malformed ascription/boundary would decline/CDZ0230 here.
#[test]
fn gen_try_body_reaches_all_forms_and_compiles() {
    let (mut saw_ok, mut saw_err, mut saw_some, mut saw_none) = (false, false, false, false);
    for seed in 0u64..1024 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(307);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_try_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        saw_ok |= body.contains("(try (Ok ");
        saw_err |= body.contains("(try (Err ");
        saw_some |= body.contains("(try (Some ");
        saw_none |= body.contains("(try None)");
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "try body must COMPILE: {src}"
        );
    }
    assert!(saw_ok, "should reach a Result Ok-success try");
    assert!(saw_err, "should reach a Result Err-short-circuit try");
    assert!(saw_some, "should reach an Option Some-success try");
    assert!(saw_none, "should reach an Option None-short-circuit try");
}

/// `gen_pattern_match_body` REACHES all destructuring-pattern forms (tuple-2 / tuple-3 / record /
/// nested Some-tuple) and every body COMPILES (S119: fills the compound-PATTERN gap #5257 round-trips).
/// A malformed pattern (or an unsupported list pattern → CDZ0210) would surface here.
#[test]
fn gen_pattern_match_body_reaches_all_forms_and_compiles() {
    let (mut saw_t2, mut saw_t3, mut saw_rec, mut saw_nested) = (false, false, false, false);
    for seed in 0u64..1024 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(409);
        let mut bytes = Vec::new();
        for _ in 0..16 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let mut body = String::new();
        gen_pattern_match_body(&mut ByteCursorChoice::new(&bytes), &mut body);
        saw_t2 |= body.contains("((tuple x y) x)");
        saw_t3 |= body.contains("((tuple x y z) y)");
        saw_rec |= body.contains("((record (= a x) (= b y)) y)");
        saw_nested |= body.contains("((Some (tuple x y)) x)");
        let src = format!("(do (def (main) {body}) (export main))");
        assert!(
            matches!(compile_catching(&src), Verdict::Compiled { .. }),
            "pattern-match body must COMPILE: {src}"
        );
    }
    assert!(saw_t2, "should reach a 2-tuple destructure pattern");
    assert!(saw_t3, "should reach a 3-tuple destructure pattern");
    assert!(saw_rec, "should reach a record destructure pattern");
    assert!(
        saw_nested,
        "should reach a nested Some-tuple destructure pattern"
    );
}

/// Every operator the generator can emit is a valid Int64→Int64→Int64 op the compiler CLEANLY
/// handles (guards the `OPS` list: a bogus/removed op would surface here rather than as silent
/// declines in the fuzzer). With small operands (6, 3) there is no overflow / div-by-zero, so each
/// compiles.
#[test]
fn every_operator_is_a_cleanly_handled_int64_op() {
    for op in OPS {
        let source = format!("(do (def (main) ({op} 6 3)) (export main))");
        assert!(
            matches!(compile_catching(&source), Verdict::Compiled { .. }),
            "operator `{op}` should compile as an Int64 op: {source}"
        );
    }
}

/// The helper + call shape the generator can emit compiles: a non-recursive `(def (f a b) …)` plus a
/// `(f <e> <e>)` call from main. Pins that function-def + multi-arg call lowering is valid Cadenza.
#[test]
fn helper_and_call_shape_compiles() {
    let src = "(do (def (f a b) (+ a b)) (def (main) (f 3 4)) (export main))";
    assert!(
        matches!(compile_catching(src), Verdict::Compiled { .. }),
        "helper + call must compile: {src}"
    );
}

/// The boolean-connective condition shapes the generator can emit compile: `and`/`or`/`not` over
/// relations, as an `if` condition. Pins that boolean-connective lowering is valid Cadenza.
#[test]
fn boolean_connective_condition_compiles() {
    let src = "(do (def (main) (if (and (< 1 2) (or (not (> 3 4)) (<= 5 6))) 1 0)) (export main))";
    assert!(
        matches!(compile_catching(src), Verdict::Compiled { .. }),
        "boolean-connective condition must compile: {src}"
    );
}

/// The compound `main`-body shapes the generator can emit compile: a `(tuple …)` and a `(list …)`
/// of Int64 elements. Pins that product/collection construction from the coercing generator is valid.
#[test]
fn compound_main_body_shapes_compile() {
    for src in [
        "(do (def (main) (tuple 1 2)) (export main))",
        "(do (def (main) (list 1 2 3)) (export main))",
    ] {
        assert!(
            matches!(compile_catching(src), Verdict::Compiled { .. }),
            "compound main body must compile: {src}"
        );
    }
}

/// The bool-valued `main`-body shapes the generator can emit compile to VALID wasm: `main : Bool`
/// from a relation and from boolean connectives. Pins that bool RETURN-value lowering (bool-as-i32
/// result + the bool value codec) — distinct from bool-as-`if`-condition — is valid Cadenza.
#[test]
fn bool_main_body_shapes_compile() {
    for src in [
        "(do (def (main) (< 1 2)) (export main))",
        "(do (def (main) (and (< 1 2) (not (>= 3 4)))) (export main))",
    ] {
        assert!(
            matches!(compile_catching(src), Verdict::Compiled { .. }),
            "bool main body must compile to valid wasm: {src}"
        );
    }
}

/// The terminating recursive-helper shapes the generator can emit compile to VALID wasm: a
/// `(def (r n) (if (<= n 0) <base> (<op> n (r (- n 1)))))` called with a small fuel literal. Mirrors
/// the corpus §"a do-local function declaration is recursive" shape. Pins that SELF-recursive call
/// lowering is valid Cadenza (a surface no other generator arm reaches).
#[test]
fn recursive_helper_shape_compiles() {
    for src in [
        // + accumulation — a plain counted sum, cannot trap.
        "(do (def (r n) (if (<= n 0) 0 (+ n (r (- n 1))))) (def (main) (r 5)) (export main))",
        // The exact corpus `fac` shape (multiply), called with a small fuel.
        "(do (def (r n) (if (<= n 0) 1 (* n (r (- n 1))))) (def (main) (r 5)) (export main))",
        // Called with fuel 0 — hits the base case immediately.
        "(do (def (r n) (if (<= n 0) 7 (- n (r (- n 1))))) (def (main) (r 0)) (export main))",
    ] {
        assert!(
            matches!(
                compile_catching(src),
                Verdict::Compiled { .. } | Verdict::Declined { .. }
            ),
            "recursive helper shape must be cleanly handled: {src}"
        );
    }
}

/// Sweeping varied entropy: the recursive-helper arm (`(r <fuel>)`) is REACHABLE, and every coerced
/// program that emits it is cleanly handled (compiles / declines — never a crash / invalid wasm /
/// parse error, and never a non-terminating run). Guards the terminating-recursion widening.
#[test]
fn recursive_arm_is_reachable_and_cleanly_handled() {
    let mut saw_rec = false;
    for seed in 0u64..400 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut bytes = Vec::new();
        for _ in 0..24 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let program = gen_from(&bytes);
        if program.source.contains("(def (r n)") {
            saw_rec = true;
            let verdict = compile_catching(&program.source);
            assert!(
                matches!(verdict, Verdict::Compiled { .. } | Verdict::Declined { .. }),
                "recursive-helper program must be cleanly handled, got {verdict:?} for: {}",
                program.source
            );
        }
    }
    assert!(
        saw_rec,
        "the recursive-helper arm should be reachable across 400 varied entropy inputs"
    );
}

/// The TAIL-recursive accumulator helper shapes compile to VALID wasm: `(def (t n acc) (if (<= n 0)
/// acc (t (- n 1) (<op> acc n))))` called with a small fuel + seed. Pins that tail-position recursive
/// call lowering (the corpus "tail-recursive counted loop" surface) is valid Cadenza — distinct from
/// the non-tail `r`. In the else-branch `n >= 1`, so `/`/`%` never divide by zero (trap-free).
#[test]
fn tail_recursive_shape_compiles() {
    for src in [
        "(do (def (t n acc) (if (<= n 0) acc (t (- n 1) (+ acc n)))) (def (main) (t 5 0)) (export main))",
        "(do (def (t n acc) (if (<= n 0) acc (t (- n 1) (* acc n)))) (def (main) (t 5 1)) (export main))",
        // `/` in the accumulator — safe because n >= 1 in the recursive branch (no div-by-zero).
        "(do (def (t n acc) (if (<= n 0) acc (t (- n 1) (/ acc n)))) (def (main) (t 4 100)) (export main))",
        // fuel 0 → base case immediately, returns the seed.
        "(do (def (t n acc) (if (<= n 0) acc (t (- n 1) (- acc n)))) (def (main) (t 0 9)) (export main))",
    ] {
        assert!(
            matches!(compile_catching(src), Verdict::Compiled { .. }),
            "tail-recursive helper shape must compile to valid wasm: {src}"
        );
    }
}

/// Sweeping varied entropy: the tail-recursive arm (`(t <fuel> <seed>)`) is REACHABLE, and every
/// coerced program that emits it is cleanly handled (compiles / declines — never crash / invalid wasm
/// / parse error, and never a non-terminating run). Guards the tail-recursion widening.
#[test]
fn tail_recursive_arm_is_reachable_and_cleanly_handled() {
    let mut saw_tail = false;
    for seed in 0u64..400 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut bytes = Vec::new();
        for _ in 0..24 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let program = gen_from(&bytes);
        if program.source.contains("(def (t n acc)") {
            saw_tail = true;
            let verdict = compile_catching(&program.source);
            assert!(
                matches!(verdict, Verdict::Compiled { .. } | Verdict::Declined { .. }),
                "tail-recursive program must be cleanly handled, got {verdict:?} for: {}",
                program.source
            );
        }
    }
    assert!(
        saw_tail,
        "the tail-recursive arm should be reachable across 400 varied entropy inputs"
    );
}

/// Self-operations on a bound var — `(op v v)` / `(rel v v)`, the same in-scope name reused for both
/// operands — are REACHABLE across varied entropy, and every program that emits one is cleanly
/// handled. Guards the variable-reuse widening that stresses the const-fold-soundness surface.
#[test]
fn self_operations_on_bound_vars_are_reachable_and_cleanly_handled() {
    // A doubled var token `vK vK` (same name, space-separated) is the self-operation signature.
    fn has_self_op(src: &str) -> bool {
        let toks: Vec<&str> = src.split(['(', ')', ' ']).collect();
        toks.windows(2).any(|w| {
            let t = w[0];
            !t.is_empty()
                && t == w[1]
                && t.starts_with('v')
                && t[1..].chars().all(|c| c.is_ascii_digit())
                && t.len() > 1
        })
    }
    let mut saw_self_op = false;
    for seed in 0u64..400 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut bytes = Vec::new();
        for _ in 0..24 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let program = gen_from(&bytes);
        if has_self_op(&program.source) {
            saw_self_op = true;
            let verdict = compile_catching(&program.source);
            assert!(
                matches!(verdict, Verdict::Compiled { .. } | Verdict::Declined { .. }),
                "self-op program must be cleanly handled, got {verdict:?} for: {}",
                program.source
            );
        }
    }
    assert!(
        saw_self_op,
        "a self-operation `(op v v)`/`(rel v v)` should be reachable across 400 varied inputs"
    );
}

/// The identical-branch `if` shapes the generator can emit are cleanly handled — a plain `(if C a a)`
/// and one whose condition can TRAP at runtime (`(if (< (r 2) 5) a a)`, r divide-by-zero). Pins that
/// the identical-branch fold surface (which must preserve a trapping condition's effect) is valid
/// Cadenza the compiler handles (compiles or a correct trap-decline), never a crash / invalid wasm.
#[test]
fn identical_branch_if_shapes_are_cleanly_handled() {
    for src in [
        "(do (def (main) (if (< 1 2) 5 5)) (export main))",
        "(do (def (r n) (if (<= n 0) -9223372036854775808 (/ n (r (- n 1))))) (def (main) (if (< (r 2) 5) 7 7)) (export main))",
    ] {
        assert!(
            matches!(
                compile_catching(src),
                Verdict::Compiled { .. } | Verdict::Declined { .. }
            ),
            "identical-branch if must be cleanly handled: {src}"
        );
    }
}

/// Scalar `=` and STRUCTURAL compound `=` shapes the generator can emit compile: Int64 equality as an
/// `if` condition, and `(= (tuple …) (tuple …))` / `(= (list …) (list …))` structural equality. Pins
/// that equality (incl. the recursive/heap compound-equality lowering) is valid Cadenza the compiler
/// handles — `!=` is deliberately NOT generated (invalid form, CDZ0101).
#[test]
fn equality_and_compound_equality_shapes_compile() {
    for src in [
        "(do (def (main) (if (= 3 3) 1 0)) (export main))",
        "(do (def (main) (if (= (tuple 1 2) (tuple 3 4)) 1 0)) (export main))",
        "(do (def (main) (if (= (list 1 2 3) (list 1 2 3)) 1 0)) (export main))",
    ] {
        assert!(
            matches!(compile_catching(src), Verdict::Compiled { .. }),
            "equality shape must compile: {src}"
        );
    }
}

/// The base case (empty entropy) coerces to the simplest program — a single bounded literal main —
/// which COMPILES, proving the generator reaches the backend, not just the parser.
#[test]
fn base_case_entropy_compiles() {
    let program = gen_from(&[]);
    assert!(
        !program.source.contains("(+ ")
            && !program.source.contains("(- ")
            && !program.source.contains("(* "),
        "base case should be a bare literal main, got: {}",
        program.source
    );
    assert!(matches!(
        compile_catching(&program.source),
        Verdict::Compiled { .. }
    ));
}

/// Sweeping varied entropy: the `if` and `let` arms are REACHABLE, and every coerced program is
/// cleanly handled (never a crash / invalid wasm / parse error). Guards that the widened grammar
/// stays in-bounds.
#[test]
fn if_arm_is_reachable_and_every_coerced_program_is_cleanly_handled() {
    let mut saw_if = false;
    let mut saw_let = false;
    for seed in 0u64..200 {
        // A varied, well-mixed byte string per seed (SplitMix-ish), so the driver visits all arms.
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut bytes = Vec::new();
        for _ in 0..24 {
            x ^= x >> 30;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            bytes.push((x >> 24) as u8);
        }
        let program = gen_from(&bytes);
        saw_if |= program.source.contains("(if (");
        saw_let |= program.source.contains("(let ((");
        let verdict = compile_catching(&program.source);
        assert!(
            matches!(verdict, Verdict::Compiled { .. } | Verdict::Declined { .. }),
            "coerced program must be cleanly handled, got {verdict:?} for: {}",
            program.source
        );
    }
    assert!(
        saw_if,
        "the if arm should be reachable across 200 varied entropy inputs"
    );
    assert!(
        saw_let,
        "the let arm should be reachable across 200 varied entropy inputs"
    );
}

/// A recursive-entropy input coerces into a NESTED program (exercises the recursive arms), cleanly
/// handled.
#[test]
fn recursive_entropy_builds_a_nested_arith_program() {
    let program = gen_from(&[1u8; 24]);
    assert!(
        program.source.matches('(').count() >= 2,
        "expected a nested node: {}",
        program.source
    );
    assert!(matches!(
        compile_catching(&program.source),
        Verdict::Compiled { .. } | Verdict::Declined { .. }
    ));
}

/// The `ByteCursorChoice` ENTROPY-MODEL contract — the invariant EVERY fixed-seed reachability
/// test AND the persistent coverage corpus (a saved byte string must decode to the SAME program
/// across ticks) silently depend on. Pinned DIRECTLY so a refactor that changes byte consumption
/// (e.g. `variant` reading a variable width, or `int_bounded` reading 4 bytes) fails LOUDLY here,
/// instead of silently remapping every seed — rotting the on-disk corpus and reshuffling every
/// sibling reachability test — with no test noticing.
#[test]
fn byte_cursor_choice_entropy_model_is_stable() {
    // `variant(n)` consumes EXACTLY ONE byte and returns `byte % n`.
    let mut c = ByteCursorChoice::new(&[7, 200, 3]);
    assert_eq!(c.variant(5), 7 % 5);
    assert_eq!(c.pos, 1, "variant consumes exactly one byte");
    assert_eq!(c.variant(10), 200 % 10);
    assert_eq!(c.pos, 2);

    // `variant(0)` returns 0 and consumes NOTHING (an empty range reads no entropy).
    let mut c = ByteCursorChoice::new(&[9]);
    assert_eq!(c.variant(0), 0);
    assert_eq!(c.pos, 0, "variant(0) reads no byte");

    // `int_bounded` consumes EXACTLY EIGHT bytes (a big-endian u64 folded into the range).
    let mut c = ByteCursorChoice::new(&[0; 16]);
    let _ = c.int_bounded(0, 1_000_000);
    assert_eq!(c.pos, 8, "int_bounded consumes exactly eight bytes");
    // …and it IS a big-endian fold: 0x00..00_01 = 1, so `min + (1 % span)`.
    let mut c = ByteCursorChoice::new(&[0, 0, 0, 0, 0, 0, 0, 1]);
    // The `1 % 10` is written OUT (not folded to `1`) to document the `min + (value % span)` mapping.
    #[allow(clippy::identity_op)]
    {
        assert_eq!(c.int_bounded(0, 9), 1 % 10);
    }

    // A degenerate range (`min >= max`) reads NOTHING and returns `min`.
    let mut c = ByteCursorChoice::new(&[0; 8]);
    assert_eq!(c.int_bounded(42, 42), 42);
    assert_eq!(c.pos, 0, "a degenerate int_bounded range reads no entropy");

    // EXHAUSTION coerces to the LOW end: past the buffer `byte()` yields 0, so `variant` → 0 and
    // `int_bounded` → min. This is exactly what makes generation always TERMINATE at its base case.
    let mut c = ByteCursorChoice::new(&[]);
    assert_eq!(c.variant(4), 0, "exhausted variant coerces to 0");
    assert_eq!(
        c.int_bounded(-5, 5),
        -5,
        "exhausted int_bounded coerces to min"
    );
}
