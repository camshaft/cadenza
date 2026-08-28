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

use crate::{codec, parser, printer, sexpr};

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
    match rng.below(17) {
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
        // list literal — native `#list(…)` ctor head (M2 native-compound-data)
        7 => {
            let n = rng.below(4);
            let elems: Vec<String> = (0..n).map(|_| sub(rng)).collect();
            format!("#list({})", elems.join(" "))
        }
        // tuple literal (≥2 elements — a 1-tuple is a grouping) — native `#tuple(…)`
        8 => {
            let n = 2 + rng.below(2);
            let elems: Vec<String> = (0..n).map(|_| sub(rng)).collect();
            format!("#tuple({})", elems.join(" "))
        }
        // record literal: #record((= field value)…) — native ctor head + `(= name value)` FieldPair fields
        9 => {
            let n = 1 + rng.below(3);
            let fields: Vec<String> = (0..n)
                .map(|i| format!("(= {} {})", ["m", "n", "o"][i], sub(rng)))
                .collect();
            format!("#record({})", fields.join(" "))
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
        11 => format!("(. {} {})", sub(rng), rng.pick(&names)),
        // lambda: (fn (param…) body) — 0–2 params; prints `fn(p, …) => body`
        12 => {
            let n = rng.below(3);
            let params: Vec<&str> = (0..n).map(|i| ["a", "b", "c"][i]).collect();
            format!("(fn ({}) {})", params.join(" "), sub(rng))
        }
        // type ascription: (: expr Type) — prints `expr : Type` (parenthesized where an infix binds
        // tighter). A bare-name/application type keeps the reparse a plain `:` annotation (a nested
        // ascription is a real form, distinct from a def-body return type).
        13 => {
            let ty_depth = rng.below(2);
            let ty = gen_type_expr(rng, ty_depth);
            format!("(: {} {})", sub(rng), ty)
        }
        // map literal: #map((= key value)…) — native ctor head + `(= k v)` FieldPair entries (unified with
        // a record field per M2); prints `#{ k = v, … }` (distinct from a `record`).
        14 => {
            let n = 1 + rng.below(3);
            let entries: Vec<String> = (0..n)
                .map(|i| format!("(= {} {})", ["m", "n", "o"][i], sub(rng)))
                .collect();
            format!("#map({})", entries.join(" "))
        }
        // annotation `(@ name form)` -> `@name form` (bare) / `(@ (tag "s") form)` -> `@tag("s") form`
        // (parameterized). The annotation prints on its own line above the wrapped form; in a STATEMENT
        // position it stays bare, but in an OPERAND position (infix/ascription operand, match scrutinee)
        // the printer PARENTHESIZES the whole `(@ …)` so a trailing operator binds to the annotated whole,
        // and the wrapped compound form is itself parenthesized — exercising both annotation round-trip fixes.
        15 => format!("(@ {} {})", rng.pick(&["inline", "pure", "test"]), sub(rng)),
        _ => format!("(@ (tag \"s\") {})", sub(rng)),
    }
}

/// Generate a random s-expr TYPE-EXPRESSION string — used as a sum-variant payload or an ascription RHS.
/// `depth` bounds recursion (at 0, only a bare name). The forms: a bare type name (`Int64`, `a`, `T`), an
/// application `(List T)` / `(Tuple A B)` (prints `List(T)` / `Tuple(A, B)`), an arrow `(-> A B)`
/// (prints `A -> B`), or a generic `(forall (a) T)` (prints `forall a. T`). The application args and the
/// forall/arrow forms exercise the landed forall-in-nested-type-position parse (`type_postfix`/
/// `type_arg_exprs`): a `forall`/arrow argument to a type application (`Tuple(forall a. a)`) parses as a
/// type, not a value.
fn gen_type_expr(rng: &mut Rng, depth: usize) -> String {
    let atoms = ["Int64", "Bool", "a", "b", "T", "L"];
    if depth == 0 {
        return rng.pick(&atoms).to_string();
    }
    match rng.below(6) {
        0 | 1 => rng.pick(&atoms).to_string(),
        2 | 3 => {
            // application `(List T)` / `(Tuple A B)` — a type-position application, so its args parse via
            // `type_ref` (the landed `type_postfix`/`type_arg_exprs` path). Nesting a `forall`/arrow arg
            // here exercises that fix.
            let head = rng.pick(&["List", "Tuple", "Option"]);
            let n = 1 + rng.below(2);
            let args: Vec<String> = (0..n).map(|_| gen_type_expr(rng, depth - 1)).collect();
            format!("({} {})", head, args.join(" "))
        }
        // arrow type `(-> A B)` -> `A -> B` (right-associative on the surface)
        4 => format!(
            "(-> {} {})",
            gen_type_expr(rng, depth - 1),
            gen_type_expr(rng, depth - 1)
        ),
        // generic `(forall (a) T)` -> `forall a. T` — the contextual keyword whose parse in a nested type
        // position (ascription RHS, type-application argument) the landed forall fixes cover.
        _ => {
            let binder = rng.pick(&["a", "b"]);
            format!("(forall ({}) {})", binder, gen_type_expr(rng, depth - 1))
        }
    }
}

/// Generate a random well-formed sum-`type` DECLARATION string — `(type Name variant…)` with 1–4
/// variants. Each variant is one of the three surface shapes the printer must round-trip: a BARE-ATOM
/// nullary `A` (prints `A`), a 1-ELEMENT-LIST nullary `(A)` (prints `A()` — a DISTINCT arena from the
/// bare atom, the shape the `69694e100` printer fix pins), or a payload `(Ctor T …)` (prints
/// `Ctor(T, …)`). Constructor names are unique per declaration so the s-expr reader accepts them.
fn gen_type_decl(rng: &mut Rng) -> String {
    let type_names = ["Color", "Shape", "Tree", "Expr", "Val"];
    let ctors = [
        "Mk", "Node", "Leaf", "Red", "Green", "Blue", "S", "Z", "Cons", "Nil",
    ];
    let name = rng.pick(&type_names);
    let n = 1 + rng.below(4);
    let variants: Vec<String> = (0..n)
        .map(|i| {
            let ctor = ctors[i];
            match rng.below(3) {
                // bare-atom nullary — prints `Ctor`
                0 => ctor.to_string(),
                // 1-element-list nullary — prints `Ctor()`, a DISTINCT arena the printer must preserve
                1 => format!("({})", ctor),
                // payload variant — prints `Ctor(T, …)`
                _ => {
                    let np = 1 + rng.below(2);
                    let payloads: Vec<String> = (0..np)
                        .map(|_| {
                            let d = 1 + rng.below(2);
                            gen_type_expr(rng, d)
                        })
                        .collect();
                    format!("({} {})", ctor, payloads.join(" "))
                }
            }
        })
        .collect();
    format!("(type {} {})", name, variants.join(" "))
}

/// Generate a random `@!param` module directive as its s-expr arena
/// `(pragma param (param <kv>…) (: name Type))` — the operator's module-level `@param`. The config kvs
/// group under a `(param <kv>…)` sub-node (0–3 `key: value` pairs, each a `(: key value)` ascription with
/// a NON-keyword value), and the REQUIRED `(: name Type)` binder gives the param name + declared type. It
/// prints as `@!param(k: v, …) name : Type` (empty config -> `@!param name : Type`, no parens). Exercises
/// the `param_pragma_payload` parse + `print_param_pragma` render round-trip over a broad space.
fn gen_param_pragma(rng: &mut Rng) -> String {
    let keys = ["widget", "range", "default", "step"];
    let values = ["slider", "number", "stepper", "toggle", "1", "42", "true"];
    let names = ["width", "base", "ratio", "w", "h", "thickness"];
    let nkv = rng.below(4); // 0..=3 config kvs (distinct keys, in order)
    let mut kvs: Vec<String> = Vec::new();
    for &key in keys.iter().take(nkv) {
        let value = rng.pick(&values);
        kvs.push(format!("(: {} {})", key, value));
    }
    let config = if kvs.is_empty() {
        "(param)".to_string()
    } else {
        format!("(param {})", kvs.join(" "))
    };
    let name = *rng.pick(&names);
    let ty_depth = rng.below(2);
    let ty = gen_type_expr(rng, ty_depth);
    format!("(pragma param {} (: {} {}))", config, name, ty)
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

#[test]
fn ml_surface_round_trips_generated_type_declarations() {
    // Sweep random sum-`type` declarations wrapped in a `(do <type> (def (main) …))` module. `gen_expr`
    // never emits a `type`, so the type-declaration printer path — `print_type`/`print_variant`/
    // `is_type_shape` — went unexercised by the expression sweep above and was pinned only by a handful
    // of hand-written cases. In particular the two DISTINCT nullary-variant arenas (bare atom `A` -> `A`
    // vs 1-element list `(A)` -> `A()`) are the shapes the `69694e100` fix disambiguates: rendering `(A)`
    // as bare `A` would CANONICALIZE the arena and silently break a corpus round-trip whose reference
    // uses the `(A)` spelling. Assert the ML reparse (a) succeeds, (b) is structurally equal to the
    // oracle (no canonicalization of either nullary spelling, no payload/type-application drift), and
    // (c) is idempotent. Also route each generated type through the BINARY codec so the encode/decode
    // bijection covers the type-declaration node set, not just expressions. Fixed seeds -> reproducible;
    // the failing s-expr + ML print for triage.
    let seeds: [u64; 4] = [
        0x739a_11c4_0f2e_dd01,
        0x2c6e_88ab_5510_37f9,
        0xa0f1_3d72_9e4b_6c85,
        0x4b8d_f206_71ca_9e3d,
    ];
    let mut total = 0usize;
    for &seed in &seeds {
        let mut rng = Rng(seed);
        for _ in 0..1500 {
            let ty = gen_type_decl(&mut rng);
            let body_depth = 1 + rng.below(3);
            let body = gen_expr(&mut rng, body_depth);
            let program = format!("(do {ty} (def (main) {body}))");
            let oracle = match sexpr::read(&program) {
                Ok(a) => a,
                Err(e) => panic!(
                    "generator produced an unreadable s-expr: {program}\n  {}",
                    e.0
                ),
            };
            let ml = printer::print(&oracle, WIDTH);
            let reparsed = parser::read_ml(&ml);
            assert!(
                reparsed.ok(),
                "ML reparse FAILED\n  s-expr: {program}\n  ml:\n{ml}\n  errs:   {:?}",
                reparsed.errors
            );
            assert!(
                reparsed.arenas.structurally_eq(&oracle),
                "ML round-trip changed the tree\n  s-expr: {program}\n  ml:\n{ml}\n  reparsed: {}",
                sexpr::print(&reparsed.arenas)
            );
            assert_eq!(
                printer::print(&reparsed.arenas, WIDTH),
                ml,
                "ML print is not idempotent\n  s-expr: {program}\n  ml:\n{ml}"
            );
            // Binary codec: decode(encode) is structurally equal to the oracle (bijection over the
            // type-declaration node set).
            let back =
                codec::decode(&codec::encode(&oracle)).expect("type decl's encoding decodes");
            assert!(
                back.structurally_eq(&oracle),
                "binary round-trip changed the tree\n  s-expr: {program}",
            );
            total += 1;
        }
    }
    assert!(total >= 6000, "swept a meaningful space, got {total}");
}

#[test]
fn ml_surface_round_trips_generated_param_pragmas() {
    // Sweep random `@!param` module directives — `(pragma param (param <kv>…) (: name Type))` — wrapped in
    // a `(do <pragma> (def (main) …))` module. The `@!param` grammar (parser `param_pragma_payload` +
    // printer `print_param_pragma`) is the operator's module-level `@param`; its parse special-cases the
    // `param` pragma key to carry a grouped config sub-node + a `name : Type` binder (so the trailing name
    // is not mis-eaten as a unit-suffix), and its printer renders back the `@!param(k: v, …) name : Type`
    // surface (empty config -> no parens, which the parser re-accepts). Sweep the parse+print round-trip
    // over a broad space of config-kv counts, values, names, and payload types: assert the ML reparse
    // succeeds, is structurally equal to the s-expr oracle, is idempotent, and survives the binary codec.
    // (Round-trip-ONLY — never compiles the pragma, so independent of the rcdzc pragma-registry `param`
    // arm v-metaprogramming owns; the compile-corpus fixture stays deferred until that lands.) Fixed
    // seeds -> reproducible; the failing s-expr + ML print for triage.
    let seeds: [u64; 4] = [
        0x9e21_74cc_0b3f_1a05,
        0x51fd_6a30_c8e2_9b47,
        0x0c7a_e519_44d1_6f8b,
        0xb3d2_8f61_2ea0_57c9,
    ];
    let mut total = 0usize;
    for &seed in &seeds {
        let mut rng = Rng(seed);
        for _ in 0..1500 {
            let pragma = gen_param_pragma(&mut rng);
            let body_depth = 1 + rng.below(3);
            let body = gen_expr(&mut rng, body_depth);
            let program = format!("(do {pragma} (def (main) {body}))");
            let oracle = match sexpr::read(&program) {
                Ok(a) => a,
                Err(e) => panic!(
                    "generator produced an unreadable s-expr: {program}\n  {}",
                    e.0
                ),
            };
            let ml = printer::print(&oracle, WIDTH);
            let reparsed = parser::read_ml(&ml);
            assert!(
                reparsed.ok(),
                "ML reparse FAILED\n  s-expr: {program}\n  ml:\n{ml}\n  errs:   {:?}",
                reparsed.errors
            );
            assert!(
                reparsed.arenas.structurally_eq(&oracle),
                "ML round-trip changed the tree\n  s-expr: {program}\n  ml:\n{ml}\n  reparsed: {}",
                sexpr::print(&reparsed.arenas)
            );
            assert_eq!(
                printer::print(&reparsed.arenas, WIDTH),
                ml,
                "ML print is not idempotent\n  s-expr: {program}\n  ml:\n{ml}"
            );
            let back =
                codec::decode(&codec::encode(&oracle)).expect("param pragma's encoding decodes");
            assert!(
                back.structurally_eq(&oracle),
                "binary round-trip changed the tree\n  s-expr: {program}",
            );
            total += 1;
        }
    }
    assert!(total >= 6000, "swept a meaningful space, got {total}");
}
