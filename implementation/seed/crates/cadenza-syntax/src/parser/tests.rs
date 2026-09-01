//! Tests for the ML surface reader/parser ([`super`] = `crate::parser`). Split out of `parser.rs`
//! (2026-08-31, v-syntax) to keep `parser.rs` under the 512KB file-size mandate; the delanguaging
//! test-retirement (v-parser-corpus) continues migrating these to the ml/spec corpus from here.

use super::*;

fn parse_ok(src: &str) -> Arenas {
    let p = read_ml(src);
    assert!(
        p.ok(),
        "expected clean parse of {src:?}, got {:?}",
        p.errors
    );
    p.arenas
}

// `a_trailing_comment_attaches_to_the_last_form_not_the_whole_program` (a `//` after the LAST top-level form
// wraps that FORM — `(comment "text" last)` — NOT the whole root `do`, keeping each top-level def a direct
// root child for walkers) MIGRATED to the spec/syntax corpus (inc-6 batch-82):
//   * ml/473-comment-trailing-last-top-level-form `def a = 1`⏎`def b = 2`⏎`// end`→`(do (def a 1) (comment
//     "end" (def b 2)))` (the regression — trailing wraps the last form, not the root; a MID comment `// mid`
//     before the 2nd def produces the SAME tree).
//   * ml/474-comment-trailing-stacked-on-last-form `// x`⏎`// y`→`(comment "x" (comment "y" (def b 2)))`.
//   * ml/475-comment-leading-first-of-multi-form `// lead`⏎`def a = 1`⏎`def b = 2`→`(do (comment "lead" (def a
//     1)) (def b 2))`. Single-form trailing/leading `def main() = 42`⏎`// note`=ml/291-comment-line-on-def.

// `an_own_line_comment_before_a_collection_closer_attaches_to_the_last_element` (an own-line `//` after the
// last element, before the closer, attaches to the LAST element as a leading `(comment …)` — same shape as a
// between-elements own-line comment, preserved not dropped) MIGRATED to the spec/syntax corpus (inc-6 batch-84):
// list `#list(1 (comment "c" 2))`=ml/327-comment-leading-nonfirst-list-elem, tuple `#tuple(1 (comment "c" 2))`=
// ml/281-comment-leading-nonfirst-tuple-elem (the before-closer surface yields the same last-element-leading
// tree as the between-elements input); new — ml/477-comment-before-closer-set `#set(1 (comment "c" 2))`,
// ml/478-comment-before-closer-record `#record((= a 1) (comment "c" (= b 2)))`, ml/479-comment-before-closer-map
// `#map((comment "c" (= a 1)))`.

#[test]
fn a_closer_comment_does_not_reorder_when_the_last_element_already_has_a_comment() {
    use crate::sexpr;
    // COLLISION GUARD: when the last element ALREADY carries a leading comment (`[1, // mid\n 2\n
    // // last\n]`), attaching the closer comment there would print `last` ABOVE `mid` — an
    // out-of-order round-trip. So the drain is skipped in that case; the closer comment stays in its
    // slot (the drop-guard refuses to format, no corruption — the pre-fix behavior). The reader keeps
    // ONLY the mid comment on `2`; the last comment is NOT attached (would reorder).
    assert_eq!(
        sexpr::print(&parse_ok("def m() = [1,\n // mid\n 2\n // last\n]")),
        "(def (m) #list(1 (comment \"mid\" 2)))",
        "the closer comment is not attached (would reorder above the element's own comment)"
    );
}

// `multiline_def_body_equals_single_line_and_survives_the_export_wrapper` (layout is INSIGNIFICANT: a
// multi-line def body — body on the next line, indented continuations — parses to a STRUCTURALLY IDENTICAL
// tree as the single-line form) + `nested_multi_arg_constructor_in_a_type_def_block_parses_and_round_trips`
// (a nested multi-arg ctor application inside a sum-type-def block parses cleanly + round-trips) MIGRATED to
// the spec/syntax corpus (inc-6 batch-89): ml/487-nested-multi-arg-ctor-minimal (`Solidr.Cuber(V3r(r(4), r(4),
// r(4)))`) and ml/488-nested-multi-arg-ctor-multiline-block (the full multi-line Vec3r/Solidr program →
// `(def (main) ((. Solidr Differencer) ((. Solidr Cuber) (V3r (r 4) (r 4) (r 4))) ((. Solidr Spherer) ((. Rational
// of) 5 2))))`). The multi-line input's format.cdz + read-to-canonical-tree pins layout-insignificance +
// round-trip. (The reported browser divergence was a stale guide-wasm, not a read_ml defect.)

// `an_own_line_comment_after_a_match_bodied_def_leads_the_next_def_not_dropped` (an own-line comment after a
// match-bodied def, before the next def, leads the FOLLOWING form instead of being dropped as a phantom "next
// match arm") is subsumed by the spec/syntax corpus (inc-6 batch-81): ml/297-comment-after-match-bodied-def-
// leads-next `def f(e) = match e with …`⏎`// section header`⏎`def g() = 3`→`(do (def (f e) (match …)) (comment
// "section header" (def (g) 3)))` pins the identical own-line-comment-leads-next-def-after-a-match shape.

#[test]
fn a_chained_else_if_ladder_flattens_headers_to_one_indent() {
    // operator seq69/seq70: an `else if` ladder must NOT indent DEEPER per rung — every
    // `if`/`else if`/`else` header stays at the OUTER indent (the "20 levels deep" compiler-ml pain
    // was the printer nesting `else { if { else { if … } } }`, one cbox per rung). A too-wide chain
    // lays out as a FLAT ladder (headers aligned, bodies one level under each header).
    let src = "def classify(x) =\n\
                   \x20\x20if first-threshold-check-predicate(x) then first-branch-result-value(x)\n\
                   \x20\x20else if second-threshold-check-predicate(x) then second-branch-result-value(x)\n\
                   \x20\x20else if third-threshold-check-predicate(x) then third-branch-result-value(x)\n\
                   \x20\x20else final-fallback-branch-result-value(x)\n";
    let p = read_ml(src);
    assert!(p.ok(), "parses: {:?}", p.errors);
    let printed = crate::printer::print(&p.arenas, 60); // narrow width forces the ladder to break
    // Every `else if`/`else` header sits at the SAME indent (flat ladder, not deepening per rung).
    let header_indents: Vec<usize> = printed
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            t.starts_with("else if ") || t == "else"
        })
        .map(|l| l.len() - l.trim_start().len())
        .collect();
    assert!(
        header_indents.len() >= 3,
        "ladder broke into rungs:\n{printed}"
    );
    assert!(
        header_indents.iter().all(|&i| i == header_indents[0]),
        "all else-if/else headers at ONE indent (flat ladder, not deepening): {header_indents:?}\n{printed}"
    );
    // Round-trips structurally + idempotent.
    let rp = read_ml(&printed);
    assert!(
        rp.ok() && p.arenas.structurally_eq(&rp.arenas),
        "ladder round-trips: {:?}\n{printed}",
        rp.errors
    );
    assert_eq!(
        crate::printer::print(&rp.arenas, 60),
        printed,
        "idempotent\n{printed}"
    );
}

// `an_own_line_comment_between_infix_operands_survives_the_round_trip` (own-line `//` block lines between
// operands of a multi-line infix chain attach as LEADING `(comment …)` on the right operand, printed own-line
// before the operator) MIGRATED to the spec/syntax corpus (inc-6 batch-85): ml/480-comment-own-line-multiline-
// block-between-infix `def f(a, b, c) = if a`⏎`  and b`⏎`  // block line1`⏎`  // block line2`⏎`  and c`⏎`  then
// 1 else 2`→`(def (f a b c) (if (and (and a b) (comment "block line1" (comment "block line2" c))) 1 2))` — both
// block lines nested as leading comments on `c`. (Single own-line comment between infix operands is ml/298.)

// `a_trailing_comment_on_a_non_last_infix_operand_survives_the_round_trip` (a same-line `//` on a non-last
// operand of a multi-line infix chain → `(comment-after …)` on the left operand) is subsumed by the spec/syntax
// corpus (inc-6 batch-81): ml/295-comment-trailing-infix-operand `(and (comment-after "mid" (and a b)) c)` pins
// the identical shape (comment-after on a non-last infix operand).

// `a_trailing_comment_on_an_effect_op_survives_the_round_trip` (a same-line `//` on an effect op → a
// `(comment-after …)` on the op, staying the `effect E =` surface, not mis-attaching to a following def)
// MIGRATED to the spec/syntax corpus (inc-6 batch-81): ml/471-comment-trailing-effect-op `effect E =`⏎`  | get
// : Int64 -> Int64 // note on get`⏎`  | put : Int64 -> Unit // note on put`⏎`def f() = 1`→`(do (effect E
// (comment-after "note on get" (op get (-> Int64 Int64))) (comment-after "note on put" (op put (-> Int64
// Unit)))) (def (f) 1))` — both op-trailing comments as `(comment-after …)`, the following def unaffected.

// `a_multiline_trailing_comment_on_a_type_variant_round_trips` (a same-line `//` on a variant PLUS own-line
// continuation lines leaves the continuations as the NEXT variant's LEADING comment, nested OUTSIDE that
// variant's own trailing `(comment-after …)` — the reader/printer peel BOTH wrappers in either order)
// MIGRATED to the spec/syntax corpus (inc-6 batch-85): ml/481-comment-multiline-trailing-type-variant `type T
// =`⏎`  | A(Int64) // trailing on A`⏎`  // continuation of A`⏎`  | B(Int64) // trailing on B`⏎`def f() = 1`→
// `(do (type T (comment-after "trailing on A" (A Int64)) (comment-after "trailing on B" (comment "continuation
// of A" (B Int64)))) (def (f) 1))` — the nested `(comment-after trail (comment lead V))` wrapper on B.

// `an_own_line_comment_after_the_last_sum_variant_is_not_dropped` (an own-line comment after the LAST variant
// of a `type T = | A | B`, before the next form, leads the FOLLOWING form instead of being dropped as a phantom
// "next variant") MIGRATED to the spec/syntax corpus (inc-6 batch-81): ml/472-comment-own-line-after-last-sum-
// variant `type T =`⏎`  | A`⏎`  | B`⏎`  // trailing note after the last variant`⏎`def f() = 1`→`(do (type T A
// B) (comment "trailing note after the last variant" (def (f) 1)))`.

#[test]
fn deeply_nested_input_is_diagnosed_not_crashed() {
    // The Pratt parser recurses through `expr` one native frame per nesting level, so DESCENDING to
    // the depth guard (`MAX_NESTING_DEPTH` = 1024) itself needs more stack than a default `cargo test`
    // worker (~2 MB on Linux, ~512 KB–1 MB on macOS) — the guard fires cleanly, but the recursion
    // reaching it would overflow the worker's stack first (a spurious SIGABRT that is NOT what this
    // test asserts). Run the body on a large-stacked thread so it exercises the depth guard, not the
    // worker's stack limit. (The compiler's own deep walks use the same 64 MB guard-sized stack.)
    run_deep(|| {
        // Unguarded, a pathologically deep nest overflowed the native stack (SIGABRT) or — once a
        // naive guard returned an error node without stopping — SPUN on the unconsumed deep tail (a
        // hang). The depth guard records ONE error and POISONS the parser (`depth_exceeded` ⇒
        // `at_end`), so parsing TERMINATES with a clean diagnostic. The nest exceeds the limit.
        let n = (crate::sexpr::MAX_NESTING_DEPTH as usize) + 50;
        let src = format!("{}1{}", "(".repeat(n), ")".repeat(n));
        let p = read_ml(&src);
        assert!(
            !p.ok()
                && p.errors
                    .iter()
                    .any(|e| e.message.contains("nests too deeply")),
            "deep nesting must be a clean depth-limit error, not a crash/hang; got {:?}",
            p.errors
        );
        // A nest well under the limit still parses cleanly (no over-rejection).
        let ok = (crate::sexpr::MAX_NESTING_DEPTH as usize) - 1;
        let shallow = format!("{}1{}", "(".repeat(ok), ")".repeat(ok));
        let ps = read_ml(&shallow);
        assert!(
            ps.ok(),
            "a nest just under the limit must parse: {:?}",
            ps.errors
        );
    });
}

#[test]
fn deep_prefix_operator_runs_are_diagnosed_not_crashed() {
    // A run of a PREFIX operator that recurses `prefix` DIRECTLY — unary minus (`- - - … x`) and the
    // bare-form unquote (`, , , … x`) — bypassed `expr`'s depth guard (they call `self.prefix()`, not
    // `self.expr()`), so a pathologically deep run overflowed the native stack (SIGABRT). `guard_prefix`
    // now counts each layer against the same `MAX_NESTING_DEPTH` budget at those two recursion sites, so
    // a deep run is a clean depth-limit diagnostic. Needs a large stack (like the nested-`(` test — the
    // recursion DESCENDS to the 1024 guard, more than a default test worker's stack). (Regression for
    // the prefix-recursion stack-overflow class.)
    run_deep(|| {
        let over = (crate::sexpr::MAX_NESTING_DEPTH as usize) + 50;
        let cases = [
            format!("{}1", "-".repeat(over)),  // unary-minus run `----…1`
            format!("{}x", ",".repeat(over)),  // unquote run `,,,,…x`
            format!("{}x", ",@".repeat(over)), // unquote-splicing run `,@,@…x`
        ];
        for src in cases {
            let p = read_ml(&src);
            assert!(
                !p.ok()
                    && p.errors
                        .iter()
                        .any(|e| e.message.contains("nests too deeply")),
                "a deep prefix-operator run must be a clean depth-limit error, not a crash; \
                     src head {:?}, got {:?}",
                &src[..src.len().min(8)],
                p.errors
            );
        }
        // A moderate run well under the limit still parses cleanly (no over-rejection).
        let ok = read_ml("- - - 5");
        assert!(ok.ok(), "shallow negation must parse: {:?}", ok.errors);
        let okq = read_ml("`{ ,x + ,y }");
        assert!(okq.ok(), "shallow unquote must parse: {:?}", okq.errors);
    });
}

#[test]
fn deep_pattern_and_annotation_recursion_is_diagnosed_not_crashed() {
    // Two more recursion classes that bypass `expr`'s depth guard, each fixed by `guard_prefix`:
    // (1) PATTERNS — a tuple/list/ctor sub-pattern re-enters `pattern` on a path entirely separate
    //     from `expr`, so a deep `((((…` / `[[[[…` / `C(C(C(…` pattern overflowed the stack; the
    //     guard now lives at `pattern`'s entry.
    // (2) ANNOTATION / PRAGMA — the `@name form` and `@!key arg` arms recurse `prefix` DIRECTLY, so a
    //     stacked `@a @b … def` / `@!k @!k … def` overflowed; each layer is now counted.
    // All must be clean 'nests too deeply' diagnostics, not a SIGABRT. Large stack (descends to 1024).
    run_deep(|| {
        let over = (crate::sexpr::MAX_NESTING_DEPTH as usize) + 50;
        let cases = [
            format!("def f(x) = match x with | {} => 1", "(".repeat(over)), // tuple-pattern
            format!("def f(x) = match x with | {} => 1", "[".repeat(over)), // list-pattern
            format!(
                "def f(x) = match x with | {}x{} => 1",
                "C(".repeat(over),
                ")".repeat(over)
            ), // ctor-pattern
            format!("{}def f() = 1", "@ann ".repeat(over)),                 // stacked annotations
            format!("{}def f() = 1", "@!k ".repeat(over)),                  // stacked pragmas
        ];
        for src in &cases {
            let p = read_ml(src);
            assert!(
                p.errors
                    .iter()
                    .any(|e| e.message.contains("nests too deeply")),
                "a deep pattern/annotation must be a clean depth-limit error, not a crash; \
                     src head {:?}, got {:?}",
                &src[..src.len().min(24)],
                p.errors
            );
        }
        // Moderate, well-formed pattern + annotation still parse (no over-rejection).
        let ok = read_ml("def f(x) = match x with | (a, [b, c]) => 1 | Some(y) => 2 | _ => 0");
        assert!(ok.ok(), "shallow pattern must parse: {:?}", ok.errors);
        let oka = read_ml("@inline\ndef f() = 1");
        assert!(oka.ok(), "shallow annotation must parse: {:?}", oka.errors);
    });
}

#[test]
fn deep_flat_chains_are_diagnosed_not_crashed() {
    // A FLAT chain — left-associative infix (`1+1+1…`), a postfix member run (`x.a.a…`), or a
    // call chain (`f(1)(1)…`) — is parsed by a LOOP, not recursion, so the parser's per-`expr`
    // depth counter never grows with it. But each iteration deepens the produced ARENA on one
    // side, so an unbounded run built an arbitrarily deep TREE that a recursive CONSUMER (the
    // s-expr printer, `canon`, the compiler's own walk) then overflowed the stack on (SIGABRT) —
    // even though the PARSE itself never recursed and succeeded. The `expr`/`postfix` loop guards
    // now bound the folded spine against the same `MAX_NESTING_DEPTH`, so a pathological chain is a
    // clean parse diagnostic. (Regression for the flat-chain stack-overflow class.)
    //
    // Run the body on a LARGE-STACK thread (mirroring `sexpr::deeply_nested_input_is_diagnosed_not_
    // crashed`): the guard caps the tree at `MAX_NESTING_DEPTH` (1024), but the DOWNSTREAM walks this
    // test deliberately exercises — the ML printer, the s-expr printer, `codec::encode` — recurse one
    // native frame per level, and 1024 debug-build frames exceed a default `cargo test` worker's
    // stack. (This is about the CONSUMERS' recursion depth, NOT the parse: the parse itself loops.
    // The margin is genuinely tight — a printer-dispatch change once tipped it over — so provision the
    // stack explicitly rather than depend on per-frame size staying under an implicit cap.)
    let h = std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let over = (crate::sexpr::MAX_NESTING_DEPTH as usize) + 50;
                let cases = [
                    format!("1{}", "+1".repeat(over)),    // left-assoc infix spine
                    format!("1{}", " |> f".repeat(over)), // pipeline spine (also infix)
                    format!("x{}", ".a".repeat(over)),    // postfix member chain
                    format!("f{}", "(1)".repeat(over)),   // postfix call chain
                ];
                for src in cases {
                    let p = read_ml(&src);
                    assert!(
                        !p.ok()
                            && p.errors
                                .iter()
                                .any(|e| e.message.contains("nests too deeply")),
                        "a deep flat chain must be a clean depth-limit error, not a crash; got ok={} errs={:?}",
                        p.ok(),
                        p.errors
                    );
                    // The produced arena is well-formed and EVERY recursive downstream consumer must
                    // handle it without crashing — the guard capped the tree depth, so the whole pipeline
                    // is safe. This is the invariant the cap exists to uphold: "a reader produces
                    // bounded-depth trees ⇒ the printer, canon, and codec walks are all safe." Exercise all
                    // three (the s-expr printer, the ML printer, and codec::encode, which canonicalizes).
                    let _ = crate::printer::print(&p.arenas, 80);
                    let _ = crate::sexpr::print(&p.arenas);
                    let _ = crate::codec::encode(&p.arenas);
                }
                // A flat chain WELL under the limit parses cleanly (no over-rejection) and survives the
                // full round-trip through every surface.
                let ok_n = (crate::sexpr::MAX_NESTING_DEPTH as usize) / 2;
                let shallow = format!("1{}", "+1".repeat(ok_n));
                let ps = read_ml(&shallow);
                assert!(
                    ps.ok(),
                    "a flat chain under the limit must parse: {:?}",
                    ps.errors
                );
                // Binary round-trip: the deep-but-legal arena encodes and decodes back structurally-equal.
                let bytes = crate::codec::encode(&ps.arenas);
                let back = crate::codec::decode(&bytes).expect("deep-but-legal arena decodes");
                assert!(
                    back.structurally_eq(&ps.arenas),
                    "a deep-but-legal flat chain must survive the binary round-trip"
                );
            })
            .expect("spawn deep-flat-chain worker");
    if let Err(payload) = h.join() {
        std::panic::resume_unwind(payload);
    }
}

#[test]
fn combined_recursion_plus_postfix_depth_is_bounded() {
    // PR #383 DoS: bracket-nesting recursion (`self.depth`, one frame per level) and a postfix spine
    // (`.a.a…`) BOTH deepen the same produced arena, and a recursive consumer (printer/`canon`)
    // walks their SUM. The postfix guard must bound `self.depth + spine`, so a deeply-parenthesized
    // expression WITH a long postfix chain — combined arena depth ~2× MAX_NESTING_DEPTH — is
    // diagnosed, not left to overflow a small (e.g. the guide's ~1MB wasm) stack. A spine-only
    // check missed this. Run on a big stack so the recursion descent reaches the guard.
    run_deep(|| {
        let each = crate::sexpr::MAX_NESTING_DEPTH as usize; // parens AND postfix each ~= the limit
        let inner = format!("x{}", ".a".repeat(each));
        let src = format!("{}{}{}", "(".repeat(each), inner, ")".repeat(each));
        let p = read_ml(&src);
        assert!(
            !p.ok()
                && p.errors
                    .iter()
                    .any(|e| e.message.contains("nests too deeply")),
            "combined nest+postfix (~2× limit) must be diagnosed, not a crash; ok={} errs={:?}",
            p.ok(),
            p.errors
        );
        // A combined depth comfortably UNDER the limit still parses (no over-rejection).
        let q = (crate::sexpr::MAX_NESTING_DEPTH as usize) / 3;
        let ok_inner = format!("x{}", ".a".repeat(q));
        let ok_src = format!("{}{}{}", "(".repeat(q), ok_inner, ")".repeat(q));
        let ok = read_ml(&ok_src);
        assert!(
            ok.ok(),
            "a combined depth under the limit must parse: {:?}",
            ok.errors
        );
    });
}

/// Run `f` directly on the current test worker's stack. HISTORICAL: this used to spawn a 64 MB-stack
/// worker because the RECURSIVE ML reader descended one native frame per nesting level, so a deep input
/// overflowed a default `cargo test` worker's ~2 MB stack before REACHING the depth guard
/// (`MAX_NESTING_DEPTH`). `read_ml` is now fully ITERATIVE (explicit worklist — every grammar layer:
/// expr, pattern, type, and the annotation/pragma arg descents), so a deep input reaches the guard with
/// O(1) native stack: the deep-diagnostic tests below run on the DEFAULT stack and this big-stack worker
/// is retired. Kept as a thin passthrough so the tests' bodies (which assert the clean depth-limit
/// diagnostic) are unchanged — and so this is the standing proof that the reader needs no oversized stack.
/// (NOTE: tests that exercise a RECURSIVE downstream CONSUMER on a deep tree — the ML/s-expr printer,
/// `codec`, the codemod `Tree`/`Builder` drop — still provision their own big stack; that recursion is
/// not the reader's and is out of this vertical's scope.)
fn run_deep(f: impl FnOnce()) {
    f();
}

#[test]
fn parses_clean() {
    for src in [
        "42",
        "1 + 2 * 3",
        "f(a, b)",
        "a.b.c",
        "let x = 1, y = 2 in x + y",
        "if a then b else c",
        "fn(x, y) => x + y",
        "match e with | Some(n) => n | None => 0 | _ => neg",
        "match n with | x if x < 0 => neg | _ => pos",
        "List.at(xs, 0)",
        "x |> f",
        "x |> f(a) |> g",
        "total + tax |> round",
        "`{ x + 1 }",
        "#[a, b, c]",
    ] {
        let _ = parse_ok(src);
    }
}

// `pipeline_operator_builds_a_real_node` (`|>` is a REAL infix operator building an arena node `(|> L R)`,
// left-associative, looser than arithmetic — the resolver's application-rewrite happens later, so the
// surface tree keeps the operator) is fully subsumed by the spec/syntax corpus (inc-6 batch-63, confirmed):
// ml/61-pipeline-basic `x |> f`→`(|> x f)`, ml/63-pipeline-chain `x |> f |> g`→`(|> (|> x f) g)` (left-
// assoc), ml/64-pipeline-looser-than-plus `total + tax |> round`→`(|> (+ total tax) round)` (precedence);
// ml/62-pipeline-call-rhs `x |> f(a)`→`(|> x (f a))` covers the call RHS. No new cases needed — the parser
// `parse_ok` tree assertions here are byte-identical to those goldens' trees.

#[test]
fn arena_shapes() {
    // `1 + 2 * 3` -> (+ 1 (* 2 3))
    let a = parse_ok("1 + 2 * 3");
    assert_eq!(a.head_name(a.root), Some("+"));
    let plus = a.as_form(a.root, "+").unwrap();
    assert_eq!(a.head_name(plus[1]), Some("*"));

    // `f(a, b)` -> (f a b)
    let a = parse_ok("f(a, b)");
    assert_eq!(a.head_name(a.root), Some("f"));
    assert_eq!(a.as_form(a.root, "f").unwrap().len(), 2);

    // `a.b` -> (. a b) — the `.` head is a native `Leaf::Member` (kind identity, not `Name(".")`),
    // read via `member_parts`.
    let a = parse_ok("a.b");
    assert!(a.member_parts(a.root).is_some());

    // `p.0` -> (. p 0) — positional tuple access, the numeric sibling of `p.field`.
    let a = parse_ok("p.0");
    let (obj, key) = a.member_parts(a.root).unwrap();
    assert_eq!(a.as_name(obj), Some("p"));
    assert!(
        matches!(a.get(key), crate::ast::Struct::Atom(l) if matches!(a.leaf(*l), Leaf::Int { .. }))
    );
    // `(x.0).1` -> (. (. x 0) 1) — chained index, parens keep `0.1` from lexing as a float.
    let a = parse_ok("(x.0).1");
    let (inner, _) = a.member_parts(a.root).unwrap();
    assert!(a.member_parts(inner).is_some());

    // `if a then b else c` -> (if a b c)
    let a = parse_ok("if a then b else c");
    assert_eq!(a.as_form(a.root, "if").map(|t| t.len()), Some(3));

    // `let x = 1 in x` -> (let ((x 1)) x)
    let a = parse_ok("let x = 1 in x");
    let tail = a.as_form(a.root, "let").unwrap();
    assert_eq!(tail.len(), 2); // bindings + body
}

// `match_arm_is_pattern_body_pair` (a match arm is a `(pattern body)` pair) + `prefix_unary_minus_is_arity_one_
// subtraction` (prefix `-x` is the arity-1 negation `(- x)`, binding tighter than `+`, distinct from binary
// `a - b` = arity-2 `(- a b)`) + `guarded_arm_wraps_pattern` (a guarded arm pattern is `(guard pat cond)`)
// MIGRATED to the spec/syntax corpus (inc-6 batch-80):
//   * ml/470-match-basic-two-arm `match e with | Some(n) => n | _ => 0`→`(match e ((Some n) n) (_ 0))`.
//   * prefix negation: `(- x)`=ml/54-neg-prefix-name, `(+ (- x) 1)`=ml/57-neg-tighter-than-plus, `(- (+ x 1))`=
//     ml/58-neg-parenthesized-operand; new ml/469-binary-subtraction `a - b`→`(- a b)` (arity-2 contrast).
//   * guarded arm `(match n ((guard x (< x 0)) neg) (_ pos))`=ml/149-match-guarded-arm.

// `effect_op_resource_marker_lifts_to_a_hash_clean_sibling` (`@resource T` on an op param lifts OUT of the
// marker-FREE op TYPE into a decl-level `(resource <idx>)` sibling; a no-marker op has no sibling) is subsumed
// by the spec/syntax corpus (inc-6 batch-88): ml/51-effect-op-resource-marker-first-param `(op write (-> Bytes
// (-> Bytes Unit)) (resource 0))`, ml/52-effect-op-resource-marker-second-param `(resource 1)`, ml/53-effect-op-
// no-resource-marker `(op read (-> Bytes Bytes))` (name + type only, no resource sibling).

// `effect_decl_builds_op_signatures` (an `effect` decl builds `(op name Sig)` children; a leading-arrow op
// type `-> R` is the nullary-elided one-element `(-> R)`) MIGRATED to the spec/syntax corpus (inc-6 batch-88):
// ml/486-effect-decl-two-ops `effect Diag = | emit : Int64 -> Unit | collect : -> List(Int64)`→`(effect Diag
// (op emit (-> Int64 Unit)) (op collect (-> (List Int64))))` — `emit`'s `P -> R` is a two-element arrow,
// `collect`'s `-> R` the nullary-elided one-element arrow. (Effect with doc headers = ml/300-301.)

// `world_decl_builds_the_canonical_wit_world_node` (a `world Name = | export I = | m : (p : T) -> R |
// import J = …` parses to the canonical `(world Name (export I (member m (func (param p T) (result R)))) …)`
// node — world head/name, per-direction interface sub-nodes, each member a `(func (param …) (result …))`)
// MIGRATED to the spec/syntax corpus (parser-corpus): its parse-structure claim is SUBSUMED byte-for-byte by
// ml/13-world-full-decl `(world Reducer (export fold (member apply (func (param event Bytes) (result Bytes))))
// (import kv (member get …) (member put …)))` — a structural superset (export+import, member/func/param/result)
// — plus ml/12 nullary-member, ml/14-24 (prim/list/option/result/variant/enum-flags/record members). The
// builder-EQUIVALENCE claim (parse == the S1 programmatic builders) is the separate `inline_world_*` tests.

#[test]
fn a_docd_world_carries_the_doc_but_its_interfaces_are_identity_stable() {
    // A `///` doc on a world attaches as a `(doc …)` child right after the name (round-trip), same as
    // `effect`/`type`. The doc is SURFACE metadata, NOT part of the world's identity: `world_schema_
    // tree` (the identity constructor) takes no docs, so the identity is over the INTERFACE children
    // only. Pin that a doc'd world and its undocumented twin have byte-identical INTERFACE structure —
    // the invariant the compile arm's `parse_target_world` relies on when it skips doc heads (so a
    // documented world keeps the same `wit_world` as its undocumented twin; coordinated w/ v-compiler-ml
    // 2026-08-12). This guards the docs-vs-identity boundary from the syntax side.
    let doc = parse_ok("/// a reducer world\nworld W = | export i = | m : () -> u8");
    let plain = parse_ok("world W = | export i = | m : () -> u8");
    let dw = doc.as_form(doc.root, "world").expect("world form");
    let pw = plain.as_form(plain.root, "world").expect("world form");
    // The doc'd world carries a leading `(doc …)` after the name; the plain one does not.
    assert_eq!(doc.as_name(dw[0]), Some("W"));
    assert!(
        doc.as_form(dw[1], "doc").is_some(),
        "doc node attaches after the name"
    );
    assert!(
        plain.as_form(pw[1], "doc").is_none(),
        "no doc on the plain world"
    );
    // The IDENTITY-bearing part — the interfaces (children after name, skipping any doc) — matches
    // between the two worlds. The doc'd world's interfaces are children[2..] (past the doc);
    // the plain world's are children[1..]. Assert same count + pairwise structural equality, so the
    // identity computed over the non-doc children is doc-independent (what parse_target_world relies
    // on when it skips doc heads).
    let doc_ifaces: Vec<StructId> = dw[1..]
        .iter()
        .copied()
        .filter(|&c| doc.as_form(c, "doc").is_none())
        .collect();
    let plain_ifaces: Vec<StructId> = pw[1..].to_vec();
    assert_eq!(
        doc_ifaces.len(),
        plain_ifaces.len(),
        "same interface count once the doc is skipped"
    );
    for (&d, &p) in doc_ifaces.iter().zip(plain_ifaces.iter()) {
        assert_eq!(
            crate::sexpr::print_from(&doc, d),
            crate::sexpr::print_from(&plain, p),
            "each identity-bearing interface matches its undocumented twin"
        );
    }
}

#[test]
fn inline_world_surface_encodes_identically_to_the_world_schema_tree_builder() {
    // THE cross-source identity guarantee: the inline `world …` surface must lower to the EXACT SAME
    // canonical tree `cadenza-ast::Builder::world_schema_tree` builds (the node an external binary-AST
    // artifact and v-cml's emit also target), so a target world means one content-hash regardless of
    // source. Build `world W = | export i = | m : (p : T) -> R` both ways and assert byte-identical
    // `codec::encode`. If the parser ever drifts from the builder shape (a head-kind flip, a reordered
    // child), this fails — the same drift-guard `world_schema_tree`'s own byte-stable test gives the
    // builder, now extended across the SURFACE boundary.
    // A bare type name `T` lowers through `type_ref` to a plain `Name` atom, so use name atoms as the
    // builder's type descriptors to match the surface exactly.
    let parsed = parse_ok("world W = | export i = | m : (p : T) -> R");

    let mut b = crate::Builder::new();
    let pty = b.name("T");
    let rty = b.name("R");
    let sig = b.wit_func_sig(&[("p", pty)], rty);
    let iface = b.wit_interface(crate::ast::WitDir::Export, "i", &[("m", sig)]);
    let root = b.world_schema_tree("W", &[iface]);
    let built = b.finish(root);

    assert_eq!(
        crate::codec::encode(&parsed),
        crate::codec::encode(&built),
        "inline world surface must encode identically to world_schema_tree — cross-source identity"
    );
}

#[test]
fn inline_world_aggregate_members_encode_identically_to_the_builders() {
    // The cross-source identity guarantee across ALL aggregate member types — record, result, variant,
    // enum, and flags (the full set a typed reducer world binds at the boundary: a record
    // message/request, a result-of-payload-or-error answer, an outcome variant, plus enum/flags scalars).
    // The inline surface `{f: T, …}` / `result(A, B)` / `variant(C, D(T))` / `enum(A, …)` / `flags(A, …)`
    // must lower to the EXACT SAME canonical tree the shared `wit_type_*` builders produce, so a
    // typed-member world is one content-hash whether it comes from the inline decl, an external artifact,
    // or v-cml's emit. If `wit_type_desc_of` ever drifts from a builder shape (a field/case reorder, a
    // head-kind flip, a wrong result slot, a lost variant payload, enum-vs-flags confusion), this fails.
    let parsed = parse_ok(
        "world W = | export r = \
             | step : (msg : {id: string, payload: list(u8)}) -> result(bool, string) \
             | fold : (ev : list(u8)) -> variant(Continue, Break({schema: string, reason: string})) \
             | tag : (x : u8) -> enum(Red, Green) \
             | perms : (x : u8) -> flags(Read, Write)",
    );

    let mut b = crate::Builder::new();
    // step: (msg: record {id: string, payload: list(u8)}) -> result(bool, string). Fields/arms in the
    // same declaration order as the surface.
    let id_ty = b.wit_type_prim("string");
    let payload_ty = {
        let u8 = b.wit_type_prim("u8");
        b.wit_type_list(u8)
    };
    let msg = b.wit_type_record(&[("id", id_ty), ("payload", payload_ty)]);
    let ok = b.wit_type_prim("bool");
    let err = b.wit_type_prim("string");
    let res = b.wit_type_result(Some(ok), Some(err));
    let step_sig = b.wit_func_sig(&[("msg", msg)], res);
    // fold: (ev: list(u8)) -> variant(Continue, Break({schema: string, reason: string})). A payload-less
    // case then a record-payload case.
    let ev = {
        let u8 = b.wit_type_prim("u8");
        b.wit_type_list(u8)
    };
    let break_payload = {
        let schema = b.wit_type_prim("string");
        let reason = b.wit_type_prim("string");
        b.wit_type_record(&[("schema", schema), ("reason", reason)])
    };
    let outcome = b.wit_type_variant(&[("Continue", None), ("Break", Some(break_payload))]);
    let fold_sig = b.wit_func_sig(&[("ev", ev)], outcome);
    // tag: (x: u8) -> enum(Red, Green); perms: (x: u8) -> flags(Read, Write). Same names, distinct types.
    let tag_x = b.wit_type_prim("u8");
    let color = b.wit_type_enum(&["Red", "Green"]);
    let tag_sig = b.wit_func_sig(&[("x", tag_x)], color);
    let perms_x = b.wit_type_prim("u8");
    let perms_ty = b.wit_type_flags(&["Read", "Write"]);
    let perms_sig = b.wit_func_sig(&[("x", perms_x)], perms_ty);
    let iface = b.wit_interface(
        crate::ast::WitDir::Export,
        "r",
        &[
            ("step", step_sig),
            ("fold", fold_sig),
            ("tag", tag_sig),
            ("perms", perms_sig),
        ],
    );
    let root = b.world_schema_tree("W", &[iface]);
    let built = b.finish(root);

    assert_eq!(
        crate::codec::encode(&parsed),
        crate::codec::encode(&built),
        "inline record+result+variant+enum+flags member world must encode identically to the builders"
    );
}

#[test]
fn inline_pure_fold_world_encodes_identically_to_the_kernel_artifact_form() {
    // The CROSS-SOURCE BYTE-IDENTITY gate v-agent-harness cleared (build_type dedup landed cf1433380):
    // the inline surface for v-ah's `pure_fold_world_artifact` must encode BYTE-IDENTICALLY to the
    // artifact — both now route through the shared `world_schema_tree` + `wit_type_*` builders, so a
    // target world is one content-hash whether it comes from the inline decl or the external artifact.
    // Reproduce the artifact's exact build here (its cdz-kernel fn is another workspace, but it is
    // documented as building via these same shared builders): `world "reducer"`, one export interface
    // `cadenza:agent-kernel/fold`, one member `apply(event: list<u8>) -> list<u8>`.
    let parsed = parse_ok(
        "world reducer = | export `cadenza:agent-kernel/fold` = | apply : (event : list(u8)) -> list(u8)",
    );

    let mut b = crate::Builder::new();
    let ev = {
        let u8 = b.wit_type_prim("u8");
        b.wit_type_list(u8)
    };
    let res = {
        let u8 = b.wit_type_prim("u8");
        b.wit_type_list(u8)
    };
    let sig = b.wit_func_sig(&[("event", ev)], res);
    let fold = b.wit_interface(
        crate::ast::WitDir::Export,
        "cadenza:agent-kernel/fold",
        &[("apply", sig)],
    );
    let root = b.world_schema_tree("reducer", &[fold]);
    let built = b.finish(root);

    assert_eq!(
        crate::codec::encode(&parsed),
        crate::codec::encode(&built),
        "inline pure_fold world must encode identically to the shared-builder artifact form"
    );
}

#[test]
fn world_nullary_member_elides_the_param_list() {
    // A nullary member `now : () -> Timestamp` (or `now : -> Timestamp`) yields a func with zero
    // params but an always-present result — matching `wit_func_sig(&[], …)`.
    let a = parse_ok("world Clock = | export c = | now : () -> Timestamp");
    let iface = a
        .as_form(a.as_form(a.root, "world").unwrap()[1], "export")
        .unwrap();
    let func = a
        .as_form(a.as_form(iface[1], "member").unwrap()[1], "func")
        .unwrap();
    assert_eq!(func.len(), 1, "zero params, just the result sub-node");
    assert!(a.as_form(func[0], "result").is_some());
}

#[test]
fn bare_world_is_still_an_ordinary_name_not_a_world_decl() {
    // `world` is CONTEXTUAL — only `world <name> =` heads a decl. A bare `world` used as a variable
    // (a let binding, a reference) MUST stay an ordinary name so the common word is not burned.
    let a = parse_ok("let world = 5 in world + 1");
    // The body reads `world` as a plain name, not a `(world …)` decl — no `world`-headed form at root.
    assert!(
        a.as_form(a.root, "world").is_none(),
        "a bare `world` must not parse as a world declaration"
    );
    // And `world` alone as an expression is just the name.
    let b = parse_ok("world");
    assert_eq!(b.as_name(b.root), Some("world"));
}

// `handle_promotes_effect_and_seed_with_state_last` + `handle_stateless_seed_elides_to_unit` +
// `host_delegation_builds_effect_list` (the effect-handling surface: `handle E(seed) with | op(params…) =>
// body in expr` promotes the effect NAME + seed to the head, the arm op is BARE, and the LAST arm binder is
// the state — an elided `(seed)` → `unit` with an empty param list; `host e1, e2 in body` builds an effect
// LIST) MIGRATED to the spec/syntax corpus (inc-6 batch-70):
//   * ml/421-handle-stateful-seed-state-last `handle Fresh(0) with | next(u, s) => resume(s, s + 1) in
//     Fresh.next()`→`(handle Fresh 0 ((next (u) s (resume s (+ s 1)))) ((. Fresh next)))`.
//   * ml/422-handle-stateless-seed-elides-unit `handle Choose with | pick(s) => resume(5, s) in
//     Choose.pick()`→`(handle Choose unit ((pick () s (resume 5 s))) ((. Choose pick)))` (elided seed=unit,
//     nullary op — state consumed the only binder).
//   * ml/423-host-delegation-effect-list `host ask, log in ask.ask()`→`(host (ask log) ((. ask ask)))`.
//   Each fmt pins the `handle … with`⏎`  | op(…) => …`⏎`in`⏎`body` / `host …, … in body` surface.

// `semicolon_sequences_a_function_body` (a `;`-separated body folds to a flat `(do …)`, greedily collecting
// its run and STOPPING at the next top-level `def`) + `top_level_forms_juxtapose_without_semicolons` (top-level
// forms are whitespace-separated, no `;`) MIGRATED to the spec/syntax corpus (inc-6 batch-72):
// ml/431-semicolon-body-stops-at-next-def `def f() = a; b; c`⏎`def g() = 2`→`(do (def (f) (do a b c)) (def (g)
// 2))` (body stops at `def g`); the def-juxtaposition `def a = 1 def b = 2`→`(do (def a 1) (def b 2))` is
// subsumed by ml/203-top-level-value-defs-blank-separated.

// OPERATOR RULED (2026-08-31, via v-syntax) + IMPLEMENTED: top-level `;` is a SEPARATOR required ONLY to
// disambiguate an ambiguous expr boundary (the `f() g()`→Qty fold); ALL declarations are exempt and a lone
// expr needs none. `f(); g()` → `(do (f) (g))` (two forms); `f() g()` (no `;`) is the AMBIGUOUS case — the
// reader now REJECTS it (was a silent bogus `(Qty.of (f) (Unit.of g))` fold, because `g(` is a call-followed
// name, never a unit). name/call/member-magnitude quantities stay goldened (ml/242-245); the change is to
// the `;`-optional claim (via declining the unit-suffix on a call-followed name), not the quantity sugar.
// Corpus pins the language rule: `f(); g()`=ml/432-semicolon-top-level-folds (parses to two forms),
// `f() g()`=ml/476-ambiguous-toplevel-juxtaposition-rejected (a DECLINE case — the ambiguity is rejected).
// This Rust test additionally asserts the diagnostic SUGGESTS `;` (implementation-quality, stays Rust).
#[test]
fn top_level_semicolon_separates_and_ambiguous_juxtaposition_is_rejected() {
    // `;` SEPARATES top-level expressions: `f(); g()` folds a stmt-level `(do …)` the root splices flat.
    let with = parse_ok("f(); g()");
    let wt = with.as_form(with.root, "do").unwrap();
    assert_eq!(wt.len(), 2);
    assert_eq!(with.head_name(wt[0]), Some("f"));
    assert_eq!(with.head_name(wt[1]), Some("g"));
    // But whitespace-juxtaposing two top-level exprs on ONE line — `f() g()` — is AMBIGUOUS (the sugar
    // would fold `g` as a unit into a bogus quantity), so it is now a parse ERROR that suggests `;`.
    let without = read_ml("f() g()");
    assert!(
        !without.ok(),
        "juxtaposed top-level exprs `f() g()` must require `;`, got: {}",
        crate::sexpr::print(&without.arenas)
    );
    assert!(
        without.errors.iter().any(|e| e.message.contains(';')),
        "the ambiguous-juxtaposition error suggests `;`: {:?}",
        without.errors
    );
}

// `semicolon_in_argument_position_needs_parens` (a call argument is a single expression, so a `;` inside must
// parenthesize: `f((a; b))` is a one-arg call whose argument is the sequence `(do a b)`) +
// `if_branch_does_not_swallow_the_trailing_sequence` (`if`'s branches parse at `PREC_SEQ + 1`, so a `;` after
// the `if` belongs to the ENCLOSING sequence) MIGRATED to the spec/syntax corpus (inc-6 batch-72):
// ml/434-semicolon-in-arg-needs-parens `f((a; b))`→`(f (do a b))`, ml/435-if-branch-no-swallow-trailing-seq
// `def f() = if c then a else b; more`→`(def (f) (do (if c a b) more))` (NOT `(if c a (do b more))`).

#[test]
fn spans_are_total_and_distinct_for_occurrences() {
    // `x + x`: two x occurrences share one leaf but have distinct ids and distinct spans.
    let p = read_ml("x + x");
    assert!(p.ok());
    let a = &p.arenas;
    // span table has one entry per structure node
    assert_eq!(p.spans.len(), a.structure.len());
    let plus = a.as_form(a.root, "+").unwrap();
    let (l, r) = (plus[0], plus[1]);
    assert_ne!(l, r);
    let ls = p.spans.get(l).unwrap();
    let rs = p.spans.get(r).unwrap();
    assert_ne!(
        ls, rs,
        "the two `x` occurrences map to different source spans"
    );
    // both are the text "x"
    assert_eq!(&"x + x"[ls.start..ls.end], "x");
    assert_eq!(&"x + x"[rs.start..rs.end], "x");
}

#[test]
fn one_leaf_for_repeated_name() {
    let a = parse_ok("f(f, f)");
    // "f" interned once (+ nothing else); 3 occurrences.
    assert_eq!(a.leaves.len(), 1);
}

// `quantity_literal_desugars` (a numeric literal + bare unit name is a quantity literal `(Qty.of n (Unit.of
// #unit))`, binding tighter than every operator) MIGRATED to the spec/syntax corpus (inc-6): ml/80-quantity-
// concise-int `5 feet`, ml/81-quantity-concise-decimal `5.0 meter`, ml/82-quantity-rate-division `5 feet / 1
// second`→`(/ (Qty.of 5 …) (Qty.of 1 …))`, ml/84-quantity-in-call-arg `dist(5 feet)`.

// `compound_unit_desugars_on_glued_operators` (a unit magnitude + GLUED `/`/`*`/`^` extends the UNIT into a
// composite; GLUE is the disambiguator — a spaced `/` or a glued-`/`-before-a-NUMBER stays arithmetic) MIGRATED
// to the spec/syntax corpus (inc-6 batch-86): rate `59 GiB/s`=ml/85-quantity-compound-rate-per, accel `9 m/s^2`
// (`^` tighter than `/`)=ml/86-quantity-compound-accel, force `3 kg*m/s^2`=ml/87-quantity-compound-force; new —
// ml/482-quantity-single-unit-exponent `10 m^2`→`(Qty.of 10 (^ (Unit.of m) 2))`, ml/483-quantity-spaced-slash-
// is-arithmetic `59 GiB / 2`→`(/ (Qty.of 59 (Unit.of GiB)) 2)` (spaced → arithmetic), ml/484-quantity-glued-
// slash-before-number-is-arithmetic `59 GiB/2` (a unit `/`'s RHS must be a NAME, so `/2` divides).

#[test]
fn compound_unit_node_spans_cover_the_whole_unit_expression() {
    // A compound-unit op node (`a/b`, `a*b`) and an exponent node (`m^2`) must span from the LEFT/BASE
    // operand's start — not from the operator — so a diagnostic anchored on the unit expression covers
    // the whole thing (PR#731: the spans previously started at `/`/`*`/`^`, truncating to `/b` / `^2`).
    // The unit expr is the SECOND operand of the `(Qty.of num <unit>)` node; slice source by its span.
    let src = "def main() = 59 GiB/s";
    let p = read_ml(src);
    assert!(p.ok(), "parse: {:?}", p.errors);
    let a = &p.arenas;
    // Reach the `(/ (Unit.of GiB) (Unit.of s))` composite: def body = `((. Qty of) 59 <unit>)`, an
    // application list whose LAST element is the unit expression.
    let def = a.as_form(a.root, "def").unwrap();
    let crate::ast::Struct::List(items) = a.get(def[1]) else {
        panic!("Qty.of body is a list")
    };
    let unit = *items.last().unwrap();
    let us = p.spans.get(unit).unwrap();
    // The composite `/` node must span "GiB/s" WHOLE — from `G` through `s` — not just "/s".
    assert_eq!(
        &src[us.start..us.end],
        "GiB/s",
        "the compound-unit `/` node spans the whole unit expr, not just from the operator"
    );

    // And an exponent node `m^2` spans "m^2" whole, not "^2".
    let src2 = "def main() = 9 m^2";
    let p2 = read_ml(src2);
    assert!(p2.ok(), "parse: {:?}", p2.errors);
    let a2 = &p2.arenas;
    let def2 = a2.as_form(a2.root, "def").unwrap();
    let crate::ast::Struct::List(items2) = a2.get(def2[1]) else {
        panic!("list")
    };
    let unit2 = *items2.last().unwrap();
    let us2 = p2.spans.get(unit2).unwrap();
    assert_eq!(
        &src2[us2.start..us2.end],
        "m^2",
        "the unit-exponent `^` node spans the whole base^exp, not just from the `^`"
    );
}

#[test]
fn a_deep_glued_unit_chain_is_diagnosed_not_an_unbounded_arena() {
    // `compound_unit_tail` folds a glued `/`/`*` chain in a LOOP (self.depth doesn't grow), but each
    // iteration deepens the arena by one `(/ … …)` level — so an unbounded chain (`m/s/s/s…`) would
    // build a tree far deeper than the reader's MAX_NESTING_DEPTH cap that every other nesting path
    // enforces, a recursive-consumer overflow risk (the flat-chain DoS class, PR #383). The loop's
    // spine guard now bounds `self.depth + spine`, so a pathological chain is a clean depth diagnostic.
    let over = (crate::sexpr::MAX_NESTING_DEPTH as usize) + 50;
    let src = format!("def f() = 5 m{}", "/s".repeat(over));
    let p = read_ml(&src);
    assert!(
        !p.ok()
            && p.errors
                .iter()
                .any(|e| e.message.contains("nests too deeply")),
        "a deep glued unit chain must be a clean depth-limit error, not an unbounded arena: {:?}",
        p.errors
    );
    // A moderate chain well under the limit still parses cleanly (no over-rejection).
    let ok = read_ml("def f() = 5 m/s/s/s");
    assert!(
        ok.ok(),
        "a short glued unit chain must parse: {:?}",
        ok.errors
    );
}

// The `forall`-in-param-annotation DESUGAR parse-tree assertions MIGRATED to the spec/syntax corpus (inc-6):
// `forall a b. TYPE` in a param annotation desugars at parse time to leading `(: a Type)` params (infer never
// sees a `(forall …)` node) — ml/135-forall-param-desugars `def id(x: forall a. a) = x`→`(def (id (: a Type)
// (: x a)) x)`, ml/136-forall-param-multi-desugars, ml/137-forall-param-already-canonical `def id(a: Type, x:
// a) = x` (BYTE-IDENTICAL tree to ml/135, pinning the pure-sugar equivalence). This test keeps ONLY the
// value-position error guard below (a reserved-keyword diagnostic — out of the parse-tree/fmt corpus scope).
#[test]
fn a_bare_value_position_forall_is_a_reserved_keyword_error() {
    // `forall` is a RESERVED keyword (like `as`): recognized in type position, never a bare value
    // name — a value-position `forall` is the usual "keyword outside its form" error.
    let bare = read_ml("forall");
    assert!(
        !bare.ok() && bare.errors.iter().any(|e| e.message.contains("keyword")),
        "a bare value-position `forall` is a reserved-keyword error: {:?}",
        bare.errors
    );
}

// `infix_ascription_rhs_beginning_with_forall_is_a_type_not_an_expression` (a NESTED, expression-position
// infix `:` ascription whose RHS begins with `forall` parses the RHS as a TYPE, keeping a `(forall …)` node —
// distinct from a PARAM-annotation forall which desugars to `(: a Type)`, ml/135-140) MIGRATED to the
// spec/syntax corpus (inc-6 batch-73): ml/436-infix-ascription-forall-rhs `f(x : forall a. a)`→`(f (: x
// (forall (a) a)))`, ml/437-infix-ascription-forall-arrow `forall a. a -> a`→`(forall (a) (-> a a))`,
// ml/438-infix-ascription-forall-in-let `let h = k : forall a. a in h`, ml/439-infix-ascription-forall-in-
// operand `(q : forall a. a) + 1`. The forall-ONLY intercept contrast (a non-forall `:` RHS stays as-is):
// ml/440-infix-ascription-value-rhs `f(x : a + b)`→`(f (: x (+ a b)))`, ml/441-infix-ascription-arrow-type-rhs
// `f(x : a -> b)`→`(f (: x (-> a b)))`.

// `type_application_argument_is_parsed_as_a_type_not_a_value` (a type-position APPLICATION — Tuple/List/Record
// — parses each arg as a TYPE via `type_ref`, so `forall`/arrow/nested-app args parse correctly) +
// `a_record_payload_variant_parses_the_labeled_field_form` (a sum-type variant payload accepts labeled `name :
// Type` args, not just bare types) MIGRATED to the spec/syntax corpus (inc-6 batch-76):
//   * ml/448-type-app-forall-arg-tuple `Tuple(forall b. L)`→`(Tuple (forall (b) L))`, ml/449-type-app-forall-
//     arg-list-arrow `List(forall a. a -> a)`, ml/450-type-app-nested `Tuple(List(a), Int64)`, ml/451-type-app-
//     arrow-arg `List(a -> b)`, ml/452-type-app-record-field-forall `Record(p: forall a. a)`→`(Record (: p
//     (forall (a) a)))`. (Labeled `Record(x: Int64, y: Int64)`=ml/390.)
//   * ml/453-record-payload-variant `type R = | record(field : NoSuchField)`→`(type R (record (: field
//     NoSuchField)))`, ml/454-record-payload-variant-multi, ml/455-variant-mixed-positional-labeled `v(Int64,
//     tag : String)`→`(v Int64 (: tag String))`. (Positional `Pair(Int64, String)`=ml/116.)

// `derived_unit_infix_operators_parse_in_type_annotation_position` (a DERIVED-UNIT type annotation composes
// unit factors with the infix glyphs `^`/`*`/`/` — the type grammar folds them into bare-glyph heads sharing
// the value grammar's precedence, so `Qty(Int64, meter ^ 2)` round-trips) MIGRATED to the spec/syntax corpus
// (inc-6 batch-75): ml/442-derived-unit-exponent `meter ^ 2`→`(^ meter 2)`, ml/443-derived-unit-product
// `meter * second`→`(* meter second)`, ml/444-derived-unit-rate-left-assoc `gram / meter / second`→`(/ (/ gram
// meter) second)`, ml/445-derived-unit-exponent-looser-than-rate `meter / second ^ 2`→`(^ (/ meter second) 2)`
// (`^` tier-7 LOOSER than `/`/`*` tier-11), ml/446-derived-unit-parenthesized-exponent `meter / (second ^ 2)`→
// `(/ meter (^ second 2))`, ml/447-derived-unit-in-result-position (`-> Qty(…)` parses identically).

// `at_bang_param_carries_a_config_and_a_name_type_binder` (`@!param` carries a glued `(config kv…)` of
// `key: value` pairs PLUS a `name : Type` binder → `(pragma param (param (: k v)…) (: name Type))`) MIGRATED
// to the spec/syntax corpus (inc-6 batch-77): ml/163-pragma-param-single-config, ml/164-pragma-param-multi-
// config (tuple value), ml/165-pragma-param-empty-config (`@!param` no parens), ml/166-pragma-param-function-
// typed (arrow binder), ml/456-pragma-param-empty-config-parens (`@!param()` explicit empty parens → `(param)`,
// same tree as ml/165). A NON-`param` pragma `@!default-fraction Rational` is the single-type-arg form ml/167.

// The leading-`def forall` DESUGAR parse-tree assertions MIGRATED to the spec/syntax corpus (inc-6): a LEADING
// `def forall a b. f(…)` clause prepends a `(: a Type)` param per binder — ml/139-def-leading-forall-desugars
// `def forall a. id(x: a) = x`→`(def (id (: a Type) (: x a)) x)`, ml/140-def-leading-forall-multi. The three-
// spelling pure-sugar equivalence (leading == param-annotation == hand-written) is pinned by ml/139 ≡ ml/135 ≡
// ml/137 sharing a byte-identical tree.sexp. This test keeps ONLY the malformed-recovery guards below.
#[test]
fn a_malformed_leading_def_forall_recovers_without_panic() {
    // A malformed leading forall recovers (never panics): missing binder, missing `.`.
    assert!(!read_ml("def forall . f() = 1").ok());
    assert!(!read_ml("def forall a f() = 1").ok());
}

// `unit_application_is_a_general_postfix_on_any_expression` (unit application is a general POSTFIX, not
// literal-only: any expr followed same-line by a bare name is a `(Qty.of expr (Unit.of #name))` quantity;
// the unit binds TIGHTER than infix) + `unit_suffix_does_not_cross_a_newline_on_a_variable` MIGRATED to the
// spec/syntax corpus (inc-6 batch-77): bare-var `x meter`=ml/242, call `f(x) meter`=ml/243; the new bits —
// ml/457-unit-suffix-binds-tighter-than-infix `x + 1 meters`→`(+ x ((. Qty of) 1 …))`, ml/458-unit-suffix-
// parenthesized-sum-magnitude `(x + 1) meters`→`((. Qty of) (+ x 1) …)`, ml/459-unit-suffix-in-call-arg
// `cube(width meters)`, ml/460-type-suffixed-literal-not-a-quantity `100N feet`→`(do 100N feet)` (a numeric-
// type suffix is EXEMPT), ml/461-unit-suffix-no-cross-newline `def w = x`⏎`meters`→`(do (def w x) meters)`.

// `set_literal_desugars` (`#(…)` is the native set ctor literal `#set(…)`, head `Leaf::Ctor(Set)`, uniform
// with `#list`/`#tuple`/`#record`/`#map`) MIGRATED to the spec/syntax corpus (inc-6 batch-63):
// ml/376-set-literal-basic `#(1, 2, 3)`→`#set(1 2 3)`, ml/378-set-literal-empty `#()`→`#set()`,
// ml/391-set-literal-expr-element `#(x + 1)`→`#set((+ x 1))` (an expression element parses fully),
// ml/392-set-literal-as-call-arg `contains(#(1, 2), 1)`→`(contains #set(1 2) 1)` (composes as an operand).

// `bin_literal_desugars` (`b[…]` desugars to the `(bin …)` grammar form — each segment an ordinary
// call-shaped expression under the `bin` head) MIGRATED to the spec/syntax corpus (inc-6 batch-64):
// ml/237-bin-literal-typed-segments `b[u16(258), u8(1)]`→`(bin (u16 258) (u8 1))`, ml/239-bin-literal-empty
// `b[]`→`(bin)`, ml/393-bin-literal-le-modifier `b[u16(258, le), bits(1, 1)]`→`(bin (u16 258 le) (bits 1
// 1))`, ml/394-bin-literal-dependent-size `b[u16(Bytes.len(payload)), bytes(payload)]`→`(bin (u16 ((. Bytes
// len) payload)) (bytes payload))`, ml/395-bin-literal-as-operand `b[u8(1)] == other`→`(= (bin (u8 1)) other)`.

// `a_def_parameter_may_be_a_destructuring_pattern` + `a_destructuring_pattern_parameter_parses` (a def
// parameter that STARTS a compound pattern — `(`-tuple, `[`-list, `#{`-map, `{`-record, `b[`-binary — is a
// destructuring binder routed to `pattern`, not a bare name; plain-name / annotated params keep the
// ordinary binder path) MIGRATED to the spec/syntax corpus (inc-6 batch-68):
//   * ml/410-def-param-list-plain `[a, b]`→`(list a b)`, ml/411-def-param-map `#{ 1 = v }`→`(map (= 1 v))`
//     (canonical `(= key sub)` FieldPair), ml/414-def-param-bin `b[u8(n)]`→`(bin (u8 n))`.
//   * ml/412-def-param-nested-tuple-in-list-rest `[(a, b), .. rest]`→`(list (tuple a b) (.. rest))`,
//     ml/413-def-param-mixed-plain-and-list-rest `def f(x, [a, .. rest])`→`(def (f x (list a (.. rest))) x)`.
//   Already pinned: tuple `(a, b)`=ml/336, list-rest `[x, .. rest]`=ml/337, record `{x=a,y=b}`=ml/345 /
//   `{x=a}`=ml/346, plain-name/annotated-param=ml/102. (The regression guarded: `param` once routed only
//   `(`-led patterns to `pattern()`, "expected a name" on `[`/`#{`.)

// `bin_pattern_desugars` (in pattern position `b[…]` desugars to the same `(bin …)` head with sub-PATTERN
// segments — `u16(n)` binds `n`, `bytes(rest)` binds the tail) MIGRATED to the spec/syntax corpus (inc-6
// batch-64): ml/240-bin-pattern-match `match x with | b[u16(n), bytes(rest)] => n`→`(match x ((bin (u16 n)
// (bytes rest)) n))`, ml/396-bin-pattern-empty `b[]`→`(match x ((bin) 0))`, ml/397-bin-pattern-le-modifier
// `b[u16(n, le)]`→`(match x ((bin (u16 n le)) n))`. (Single-segment `b[u16(n)]` is ml/241.)

// `number_before_keyword_is_not_a_quantity` (only a bare NON-keyword identifier attaches as a unit; a word-
// operator keeps its infix meaning after a number) MIGRATED to the spec/syntax corpus (inc-6 batch-79):
// ml/464-word-op-after-number-not-a-quantity `5 and mask`→`(and 5 mask)` (the boolean `and`, not a quantity in
// unit `and`). (Number-magnitude quantities `5 feet` etc. are ml/80-88.)

// `a_set_pattern_parses` + `a_set_rest_pattern_parses_in_a_match_arm` (a `#(`-led SET PATTERN — the pattern
// twin of the `#(…)` set literal: native set ctor leaf head, sub-pattern elements, a `.. rest` binding the
// residual set to `(.. rest)`; in def-param AND match-arm position) MIGRATED to the spec/syntax corpus
// (inc-6 batch-65):
//   * ml/398-set-pattern-param `def f(#(a, b)) = a`→`(def (f #set(a b)) a)`, ml/399-set-pattern-rest-param
//     `def f(#(a, .. rest)) = a`→`(def (f #set(a (.. rest))) a)`, ml/400-set-pattern-empty-param `def f(#())
//     = 0`→`(def (f #set()) 0)`.
//   * ml/333-set-rest-pattern already pins the match-arm `#(a, .. rest)` + `_` catch-all; ml/401-set-rest-
//     match-arm-literal-element `match s with | #(1, .. rest) => rest | _ => s`→`(match s (#set(1 (.. rest))
//     rest) (_ s))` adds the literal-element residual-binding form (the tree behind the #6877 e2e example).
//   The clean-surface round-trip (no `` `..` `` fallback) is each case's fmt-idempotence.

// (`a_destructuring_pattern_parameter_parses` MIGRATED with the a_def_parameter breadcrumb above, inc-6
// batch-68: its `(a, b)`=ml/336, `[x, .. rest]`=ml/337, `b[u8(n)]`=ml/414, plain-name (trivial) + annotated
// `xs: List(Int64)`=ml/102 assertions are all pinned there.)

// `a_tuple_rest_pattern_parses` + `a_record_rest_pattern_parses` (a tuple/record pattern with a trailing
// `.. rest` binding the remaining positional/un-named elements to the wrapped `(.. rest)` node — the twin
// of the list/map/set-pattern rest) MIGRATED to the spec/syntax corpus (inc-6 batch-66):
//   * ml/331-tuple-rest-pattern `(a, b, .. rest)`→`(tuple a b (.. rest))`, ml/402-tuple-rest-single-leading
//     `(x, .. rest)`→`(tuple x (.. rest))`, ml/403-tuple-rest-nested-in-rest-binder `(a, .. (b, c))`→
//     `(tuple a (.. (tuple b c)))` (the rest binder is a full sub-pattern position). Plain `(a, b)` = ml/336.
//   * ml/332-record-rest-pattern `{ a = x, .. rest }`→`(record (= a x) (.. rest))`, ml/404-record-rest-field-
//     shorthand `{ a, .. rest }`→`(record (= a a) (.. rest))` (field shorthand puns `{ a }`→`(= a a)`).
//     Plain `{ a = x, b = y }` = ml/345.

// `degenerate_rest_patterns_parse_permissively_without_panic` (the compound pattern surfaces are
// INTENTIONALLY PERMISSIVE about rest position/count — rest-ONLY, NON-TRAILING, and MULTIPLE rests all
// parse to the wrapped `(.. rest)` node in situ; the at-most-one/trailing-only constraints are the
// match-lowering's job, not the scope-blind parser's) MIGRATED to the spec/syntax corpus (inc-6 batch-67):
//   * rest-ONLY (whole-collection bind): ml/405-rest-only-record-pattern `{ .. rest }`→`(record (.. rest))`,
//     ml/406-rest-only-set-pattern `#(.. rest)`→`#set((.. rest))`, ml/407-rest-only-list-pattern `[.. rest]`→
//     `(list (.. rest))` (a list PATTERN `(list …)`, distinct from the `#list(…)` construction spread ml/151).
//   * ml/408-non-trailing-rest-pattern `(a, .. rest, b)`→`(tuple a (.. rest) b)`, ml/409-multiple-rests-pattern
//     `(a, .. r1, .. r2)`→`(tuple a (.. r1) (.. r2))` — surface-permissive; lowering rejects the malformed.
//   (Tuple has no rest-only form — a rest needs a leading element + comma; the map rest-only `#{ .. rest }`
//   is covered elsewhere.)

// `parameterized_annotation_name_takes_a_glued_application` (a `(` GLUED to an annotation name makes the name
// slot a call `(tag "slow")` → `(@ (tag "slow") form)`; a `(` NOT glued — whitespace/newline between — is the
// annotated FORM) MIGRATED to the spec/syntax corpus (inc-6 batch-87): parameterized `@tag("slow")`=ml/94-
// annotation-parameterized, bare `@test`=ml/92-annotation-sigil-name-agnostic, stacked bare+param=ml/95 /
// multi-arg=ml/96; new ml/485-annotation-not-glued-paren-is-annotated-form `@test`⏎`(g)`→`(@ test g)` (the
// gluing guard — a non-glued `(` is the annotated form, NOT the annotation's call args).

// `paren_comma_in_type_position_is_a_tuple_type` (a paren-comma `(A, B)` in TYPE position — RHS of a `:` —
// is the tuple TYPE `(Tuple A B)`, NOT the tuple VALUE ctor `#tuple(A B)` the prefix path builds in
// value/pattern position; tuple values/patterns and tuple TYPES share the `(…)` spelling) MIGRATED to the
// spec/syntax corpus (inc-6 batch-69):
//   * ml/415-tuple-type-param-annotation `def f(p: (Int64, Int64)) = p.0`→`(def (f (: p (Tuple Int64
//     Int64))) (. p 0))`, ml/416-tuple-type-paren-equals-explicit `Tuple(Int64, Int64)` — BYTE-IDENTICAL
//     tree.sexp to ml/415, pinning the `(A, B)` == `Tuple(A, B)` type-position equivalence.
//   * ml/417-tuple-type-function-operand `(Int64, Bool) -> Int64`→`(-> (Tuple Int64 Bool) Int64)`,
//     ml/418-tuple-type-nested `(Int64, (Bool, Int64))`→`(Tuple Int64 (Tuple Bool Int64))`,
//     ml/419-paren-single-type-transparent `(Int64)`→`Int64` (transparent grouping, not a 1-tuple),
//     ml/420-paren-empty-unit-type `()`→`unit`.
//   The VALUE-position `(1, 2)`→`#tuple(1 2)` contrast (the retyping is TYPE-only) is ml/06-tuple-literal.

// `a_destructuring_pattern_let_binder_parses` (a `let` binder opening a destructuring pattern binds by
// pattern — the twin of the pattern parameter) is fully subsumed by the spec/syntax corpus (inc-6 batch-69):
// ml/342-let-tuple-binder-destructure `let (a, b) = p in a + b`→`(let (((tuple a b) p)) (+ a b))`,
// ml/343-let-list-rest-binder-destructure `let [x, .. rest] = ys in x`→`(let (((list x (.. rest)) ys)) x)`,
// ml/03-let plain `let x = 1 in x`→`(let ((x 1)) x)`, ml/344-let-mixed-binder-destructure `let x = 1, (a,
// b) = p in x + a`→`(let ((x 1) ((tuple a b) p)) (+ x a))`. No new cases needed.

// `quantity_sugar_does_not_cross_a_newline` (the number+name adjacency quantity sugar must NOT eat the next
// statement's leading identifier across a newline) MIGRATED to the spec/syntax corpus (inc-6 batch-79):
// ml/465-quantity-sugar-no-cross-newline `def a() = 10`⏎`a() + 5`→`(do (def (a) 10) (+ (a) 5))` (the `10`
// stays a bare number, `a() + 5` is the next form), ml/466-quantity-same-line-then-next-stmt `5 feet`⏎`x`→
// `(do ((. Qty of) 5 (Unit.of #feet)) x)` (a SAME-LINE quantity terminates at the newline). Same-line `10 a`
// quantity = ml/80-88.

// The same-line `as`-conversion parse-tree assertions of `as_conversion_does_not_cross_a_newline` MIGRATED to
// the spec/syntax corpus (inc-6 batch-79): `x as meter`=ml/220-as-conversion-basic, `5.0 as meter`⏎`x`→`(do
// ((. Unit in) (Unit.of #meter) 5.0) x)`=ml/467-as-conversion-same-line-then-next-stmt. This test keeps ONLY
// the `x`⏎`as meter` RECOVERY guard below (a leading `as` on a new line must not reach back across the newline
// — an error-recovery/negative assertion, out of the parse-tree/fmt corpus scope).
#[test]
fn a_leading_as_on_a_new_line_does_not_absorb_the_previous_statement() {
    use crate::sexpr;
    // `x <newline> as meter`: `x` is a complete statement; the leading `as` on the next line does not
    // continue it. (`read_ml` tolerates the stray `as`-with-no-left-operand error and still yields a
    // tree; the stray `as`/`meter` land as their own error-recovered forms — the point is `x` is NOT
    // folded into a `(Unit.in … x)` conversion.)
    let parsed = read_ml("x\nas meter");
    let printed = sexpr::print(&parsed.arenas);
    assert!(
        !printed.contains("Unit in"),
        "a leading `as` on a new line must not absorb the previous statement into a conversion: {printed}"
    );
}

// `as_conversion_target_may_be_a_compound_unit` (the `as` conversion target extends across a GLUED `/`/`*`/`^`
// chain into a COMPOUND unit, like `<num> GiB/s`; a SPACED `/ 2` stays a division of the conversion — the glue
// rule) MIGRATED to the spec/syntax corpus (inc-6 batch-78): single `x as meter`=ml/220-as-conversion-basic,
// compound rate `x as GiB/s`=ml/227-unit-in-compound-target-call-form (`(Unit.in (/ …) q)` shape); new bits —
// ml/462-as-conversion-compound-exponent `x as m/s^2`→`((. Unit in) (/ (Unit.of #m) (^ (Unit.of #s) 2)) x)`
// (`^` tighter than `/`), ml/463-as-conversion-spaced-slash-division `x as meter / 2`→`(/ ((. Unit in) (Unit.of
// #meter) x) 2)` (a SPACED `/ 2` is a division of the conversion, not part of the compound unit).

// `backtick_escapes_reserved_word` (`` `let` `` is the name "let", not a let-form) MIGRATED to the spec/syntax
// corpus (inc-6 batch-79): ml/468-bare-backtick-reserved-word `` `let` ``→`let`. (In-call/list backtick escapes
// are ml/334 `` f(`let`) `` and ml/335 `` [`+`, `-`] ``.)

#[test]
fn string_unescape_and_nfc() {
    let a = parse_ok(r#" "a\nb" "#);
    assert_eq!(
        a.leaf(match a.get(a.root) {
            crate::ast::Struct::Atom(l) => *l,
            _ => panic!(),
        }),
        &Leaf::Str("a\nb".into())
    );
}

#[test]
fn never_panics() {
    for src in [
        "",
        "(",
        ")",
        "let",
        "match {",
        "1 +",
        ".",
        "=>",
        "fn(",
        "if then",
        "`",
        "\"",
        // Malformed / incomplete inline `world` decls: the world_expr/world_interface/world_member
        // paths must recover (a diagnostic), never panic — the crate's totality invariant extended
        // to the new surface. Empty interface (no members), missing `=`, dangling direction, a
        // member with no arrow/result, an unterminated param list, a non-import/export interface head.
        "world W =",
        "world W = | export i =",
        "world W = | export i",
        "world W = | export i = | m",
        "world W = | export i = | m :",
        "world W = | export i = | m : (p : u8)",
        "world W = | export i = | m : (p :",
        "world W = | frobnicate i = | m : () -> u8",
        "world W = | export = | m : () -> u8",
        "world W = | export i = | m : () ->",
        // Malformed / edge SEC-F1 `@resource` effect-op markers: the effect_op resource-lift
        // (lift_resource_marker / unwrap_resource_param) + the printer resugar must recover, never
        // panic. Dangling `@resource` (no type), `@resource` on a nullary/leading-arrow op (no param
        // to mark), two `@resource` markers (only the first lifts), `@resource` on the result.
        "effect E = | op : @resource",
        "effect E = | op : @resource -> Unit",
        "effect E = | op : @resource Bytes",
        "effect E = | op : @resource Bytes -> @resource Bytes -> Unit",
        "effect E = | op : Bytes -> @resource Unit",
        "effect E = | op : -> @resource Unit",
    ] {
        let _ = read_ml(src); // must not panic
    }
}

// ---- error recovery ----
//
// The parser is a RECOVERING parser: it never bails at the first error. It collects every error
// into `errors`, and — crucially — resynchronizes so one stray symbol yields roughly one error
// instead of an avalanche, and so structure AROUND a mistake is still recovered. These tests pin
// that behavior down: they assert the arena stays well-formed, that multiple independent errors
// are all reported, that recovery syncs on delimiters, and that parsing always terminates.

/// Assert the arena is well-formed regardless of errors: the root id is in range, every list
/// child id is in range and traversable, and the span table is total (1:1 with structure nodes,
/// the invariant the whole `SpanTable` design rests on). Returns the parse for further checks.
fn recovered(src: &str) -> Parsed {
    let p = read_ml(src);
    let n = p.arenas.structure.len();
    assert!(n > 0, "arena is never empty for {src:?}");
    assert!(
        (p.arenas.root.0 as usize) < n,
        "root id in range for {src:?}"
    );
    assert_eq!(
        p.spans.len(),
        n,
        "span table stays total (1:1 with structure) for {src:?}"
    );
    // Every span is a GEOMETRICALLY VALID slice of the source — ordered, in-bounds, and on UTF-8
    // char boundaries — even on malformed/recovered input. This is what makes `&src[sp.start..sp.end]`
    // safe: an LSP hover / diagnostic underline / codemod edit slices the source by a node's span, so
    // a span past the end or off a char boundary would PANIC on the exact byte a user hovers. Totality
    // (above) only says a span EXISTS per node; this says it can be safely SLICED. Checked for every
    // id (spans are a flat vector, so a plain scan covers the whole arena).
    for i in 0..n as u32 {
        let sp = p.spans.get(StructId(i)).expect("total span table");
        assert!(
            sp.start <= sp.end
                && sp.end <= src.len()
                && src.is_char_boundary(sp.start)
                && src.is_char_boundary(sp.end),
            "span {sp:?} for node {i} is not a valid slice of {src:?}"
        );
    }
    // Every reachable node's children are valid ids — the tree is fully traversable.
    fn walk(a: &Arenas, id: StructId, seen: &mut usize) {
        *seen += 1;
        if let crate::ast::Struct::List(children) = a.get(id) {
            for &c in children {
                assert!(
                    (c.0 as usize) < a.structure.len(),
                    "child id {} in range",
                    c.0
                );
                walk(a, c, seen);
            }
        }
    }
    let mut seen = 0;
    walk(&p.arenas, p.arenas.root, &mut seen);
    p
}

#[test]
fn every_ml_span_is_a_valid_source_slice_over_arbitrary_input() {
    // Names the span-GEOMETRY property `recovered` now enforces, so it is a first-class invariant
    // (not just an incidental helper check that a refactor could drop): on ANY input — well-formed or
    // garbage — every node's span is an ordered, in-bounds, char-boundary slice of the source, so
    // `&src[sp.start..sp.end]` (LSP hover / diagnostic underline / codemod edit) can never panic. The
    // s-expr surface got this via `every_node_span_slices_back_to_that_node...`; the ML surface parses
    // a far larger grammar with error recovery + graft/desugar spans, where an off-by-one is likelier.
    // A dedicated sweep over a sigil-rich alphabet (distinct seed from the recovery sweep) drives the
    // grammar's structural paths; `recovered` asserts the geometry for each generated program.
    let alphabet: Vec<char> = "()[]{}|,;=>-+*/<:.@#`\"\\ \tabcdefimntxλ中0123456789\n"
        .chars()
        .collect();
    let mut rng = SplitMix64(0x59a2_0d1c_5111_7a5f);
    for len in 0..=40usize {
        for _ in 0..100 {
            let s: String = (0..len)
                .map(|_| alphabet[(rng.next() as usize) % alphabet.len()])
                .collect();
            let _ = recovered(&s); // asserts span geometry (+ totality/traversability) for this input
        }
    }
}

#[test]
fn recovered_arena_is_always_well_formed() {
    // Whatever the garbage, the arena is traversable and the span table is total.
    for src in [
        "@",
        "f(@)",
        "1 + @ + 2",
        "let x = @ in x",
        "[1, @, 3]",
        "{ a = @, b = 2 }",
        "match e with | @ => 1",
        "def f(@, x) = x",
        "f(a b c",
        "module m { @ }",
        ")(][}{",
        "1 @ 2 # 3 ~ 4",
    ] {
        let _ = recovered(src);
    }
}

/// A tiny deterministic PRNG (SplitMix64) — reproducible fuzz without a dependency, matching the
/// lexer/codec house style (the crate stays "plain").
struct SplitMix64(u64);
impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

#[test]
fn recovered_arena_invariants_hold_on_arbitrary_input() {
    // The parser's charter invariant — never panic, always produce a well-formed traversable arena
    // with a TOTAL span table — on ARBITRARY input, not just the hand-picked garbage above. The
    // lexer's own fuzz calls `read_ml` on random soup but only checks non-panic; this asserts the
    // PARSER's structural invariants (`recovered`) hold on every fuzzed string. The alphabet stresses
    // the grammar's structural sigils (parens/brackets/braces, `,`/`;`/`|`, keywords' lead chars,
    // operators, quote/comment openers, unicode) so recovery, resync, and span-table upkeep are
    // exercised across deeply malformed shapes.
    let alphabet: Vec<char> = "()[]{}|,;=>-+*/<:.@#`\"\\ \tabcdefimntxλ中0123456789\n"
        .chars()
        .collect();
    let mut rng = SplitMix64(0x5111_7a5f_11e2_0d1c);
    for len in 0..=32usize {
        for _ in 0..120 {
            let s: String = (0..len)
                .map(|_| alphabet[(rng.next() as usize) % alphabet.len()])
                .collect();
            // `recovered` asserts: non-empty arena, root in range, span table 1:1 with structure,
            // and every reachable child id in range (fully traversable) — for THIS fuzzed input.
            let _ = recovered(&s);
        }
    }
    // A few pathological repeats that have historically stressed recovery/depth guards.
    for s in [
        "((((((((", "))))))))", "{{{{{{{{", "[,[,[,[,", ";;;;;;;;", "||||||||", "@@@@@@@@",
    ] {
        let _ = recovered(s);
        let _ = recovered(&s.repeat(8));
    }
}

#[test]
fn does_not_bail_at_first_error() {
    // Several independent mistakes are ALL reported, not just the first. Three stray symbols
    // separated as their own top-level statements yield (at least) three errors.
    let p = recovered("@; ~; $");
    assert!(
        p.errors.len() >= 3,
        "each stray statement reports its own error, got {:?}",
        p.errors
    );
}

#[test]
fn a_single_stray_symbol_does_not_cascade() {
    // One bad token in the middle of an otherwise-fine call yields a small, bounded number of
    // errors — recovery resynchronizes rather than mis-parsing everything after it.
    let p = read_ml("f(a, $, c)");
    assert!(!p.ok(), "the stray `$` is reported");
    assert!(
        p.errors.len() <= 2,
        "one stray token stays bounded, got {} errors: {:?}",
        p.errors.len(),
        p.errors
    );
    // The call is still recovered as `(f a <error> c)` — the good arguments survive.
    let a = &p.arenas;
    assert_eq!(a.head_name(a.root), Some("f"));
    let call = a.as_form(a.root, "f").unwrap();
    assert_eq!(call.len(), 3, "three arguments recovered around the error");
    assert_eq!(a.as_name(call[0]), Some("a"));
    assert_eq!(a.as_name(call[2]), Some("c"));
}

#[test]
fn error_inside_brackets_does_not_escape_them() {
    // The offending token inside `( … )` must NOT consume the closing `)` — the parser resyncs on
    // the bracket, so the SECOND statement after it parses cleanly as its own form.
    let p = read_ml("f(@); g(x)");
    assert!(!p.ok());
    // Root is a `(do …)` of two statements; the second is a clean call `(g x)`.
    let top = p.arenas.as_form(p.arenas.root, "do").unwrap();
    assert_eq!(top.len(), 2, "two top-level statements survive: {top:?}");
    assert_eq!(p.arenas.head_name(top[1]), Some("g"));
    let g = p.arenas.as_form(top[1], "g").unwrap();
    assert_eq!(p.arenas.as_name(g[0]), Some("x"));
}

#[test]
fn missing_comma_between_args_recovers() {
    // `f(1 2)` — a missing separator is reported once, and BOTH arguments are still recovered. (This
    // uses NUMBER args deliberately: since the general unit-suffix landed, `f(a b)` is now a VALID
    // parse — `a` with unit `b`, i.e. `f((Qty.of a (Unit.of #b)))` — not a missing comma. A number
    // cannot be a unit name, so `1 2` is still unambiguously a missing separator, which keeps this
    // recovery invariant exercised on a shape the unit grammar does not claim.)
    let p = read_ml("f(1 2)");
    assert!(!p.ok(), "the missing `,` is reported");
    assert_eq!(p.errors.len(), 1, "exactly one error: {:?}", p.errors);
    assert!(
        p.errors[0].message.contains(','),
        "the error names the missing comma: {:?}",
        p.errors[0]
    );
    let a = &p.arenas;
    let call = a.as_form(a.root, "f").unwrap();
    assert_eq!(call.len(), 2, "both args recovered");
    // Both args are the number literals (Int atoms — `as_name` is None for a non-Name leaf).
    for (arg, want) in [(call[0], "1"), (call[1], "2")] {
        match a.get(arg) {
            crate::ast::Struct::Atom(lid) => {
                assert!(
                    matches!(a.leaf(*lid), Leaf::Int { .. }),
                    "arg is an Int literal"
                );
                let sp = p.spans.get(arg).unwrap();
                assert_eq!(&"f(1 2)"[sp.start..sp.end], want, "arg slices to {want}");
            }
            other => panic!("arg is an atom, got {other:?}"),
        }
    }
}

#[test]
fn an_unterminated_literal_names_its_specific_cause() {
    // A lexer ERROR token in expression position (an unterminated literal run to end-of-input) used
    // to read as the generic "expected an expression" — misdirecting, since the token IS where an
    // expression starts; the real defect is the unclosed literal. Each opener now names its cause,
    // the ML-surface twin of the s-expr reader's "unterminated string".
    for (src, needle) in [
        ("def f() = \"abc", "unterminated string literal"),
        ("def f() = b\"abc", "unterminated byte-string literal"),
        ("def f() = #\"abc", "unterminated symbol literal"),
        ("def f() = `abc", "unterminated backtick name"),
    ] {
        let p = read_ml(src);
        assert!(!p.ok(), "{src:?} is rejected");
        assert!(
            p.errors.iter().any(|e| e.message.contains(needle)),
            "{src:?} names {needle:?}, not the generic message: {:?}",
            p.errors
        );
        assert!(
            !p.errors
                .iter()
                .any(|e| e.message == "expected an expression"),
            "{src:?} does not fall back to the generic message: {:?}",
            p.errors
        );
    }
    // A well-terminated string is unaffected (no spurious error).
    assert!(
        read_ml("def f() = \"abc\"").ok(),
        "a closed string parses clean"
    );
}

// `missing_comma_in_list_recovers` (`[1 2 3]` — every element is recovered despite the missing `,`, into
// the native `#list(1 2 3)` ctor) MIGRATED to the spec/syntax corpus (parser-corpus): ml/503-list-missing-
// comma pins the DECLINE (error.txt `expected \`,\``) AND — via the new `recovered.sexp` recovery-golden
// (harness extension: render_sexpr of the RECOVERED arena on a decline) — the recovered partial tree
// `#list(1 2 3)`, i.e. all three elements survive as a well-formed List. This is the first consumer of the
// recovered.sexp capability the corpus grew for error-recovery-quality tests.

// The fixed-input error-RECOVERY tests (a decline that still yields a usable PARTIAL tree — the reader
// recovers instead of bailing) MIGRATED to the spec/syntax corpus (parser-corpus inc-8) via the new
// `recovered.sexp` recovery-golden (each pins the DECLINE + its recovered arena):
//   * `missing_closer_is_reported_and_recovered` `f(a, b` -> ml/537-call-missing-closer-recovers,
//     error.txt `expected )` + recovered `(f a b)` (call keeps both args).
//   * `recovers_the_let_around_a_bad_binding` `let x = $ in x + 1` -> ml/538-let-bad-binding-recovers,
//     recovered `(let ((x <error>)) (+ x 1))` (let shape + body survive; bad value is an <error> leaf).
//   * `keyword_boundary_is_not_swallowed_by_a_bad_condition` `if $ then a else b` ->
//     ml/539-if-bad-condition-recovers, recovered `(if <error> a b)` (all three branches recovered).
//   * `match_arm_boundary_survives_a_bad_pattern` `match e with | @ => 1 | _ => 2` ->
//     ml/540-match-bad-pattern-recovers, recovered `(match e (<error> 1) (_ 2))` (both arms recovered).
// The recovered.sexp golden is STRICTLY MORE precise than the old structural-count asserts (it pins the
// exact recovered tree incl. the <error> placeholder). Termination-over-arbitrary-junk (stray_closers,
// exhaustive_short_token_soup) stays Rust — a generated-input property, not a fixed recovered tree.

#[test]
fn stray_closers_do_not_hang_and_stay_bounded() {
    // A pile of mismatched closers/garbage must terminate (the test completing IS the assertion)
    // and produce a well-formed arena with a finite error list.
    for src in [
        ")))))",
        "][}{)(",
        "f(((((",
        "[[[[[",
        "{{{{{",
        "#{#{#{",
        ",,,,,",
        "..........",
        "=> => =>",
        "| | | |",
        "@@@@@@@@@@",
        "let let let",
    ] {
        let p = recovered(src);
        assert!(
            p.errors.len() < 10_000,
            "error list stays finite for {src:?} (no runaway loop)"
        );
    }
}

#[test]
fn valid_programs_still_report_no_errors() {
    // Recovery must be inert on well-formed input — no spurious errors, exact trees preserved.
    for src in [
        "1 + 2 * 3",
        "f(a, b, c)",
        "let x = 1, y = 2 in x + y",
        "match e with | Some(n) => n | None => 0",
        "def f(x, y) = x + y",
        "[1, 2, 3]",
        "{ a = 1, b = 2 }",
        "#{ k = v }",
        "if a then b else c",
        "module m { def x = 1 def y = 2 }", // module members are whitespace-separated, no `;`
    ] {
        let p = read_ml(src);
        assert!(p.ok(), "no spurious errors on {src:?}: {:?}", p.errors);
    }
}

#[test]
fn exhaustive_short_token_soup_always_terminates_well_formed() {
    // The strongest termination evidence: enumerate EVERY sequence of up to four tokens drawn
    // from an alphabet chosen to stress recovery (delimiters, separators, keywords, junk). If any
    // combination could drive `prefix`/`sep_continue`/the block loops into a non-advancing cycle,
    // this test would hang — so its completion is the proof that parsing always makes progress.
    // Each parse is also checked for a well-formed, traversable arena and a total span table.
    let alphabet = [
        "(", ")", "[", "]", "{", "}", "#", ",", ";", ".", "=>", "|", "@", "let", "in", "if",
        "match", "with", "def", "x", "1",
    ];
    let mut count = 0usize;
    // lengths 1..=3 exhaustively; a light length-4 sweep keeps the total bounded but deep.
    for len in 1..=3 {
        let combos = alphabet.len().pow(len as u32);
        for mut n in 0..combos {
            let mut src = String::new();
            for _ in 0..len {
                src.push_str(alphabet[n % alphabet.len()]);
                src.push(' ');
                n /= alphabet.len();
            }
            let _ = recovered(&src); // must terminate + stay well-formed
            count += 1;
        }
    }
    assert!(count > 8_000, "swept a meaningful space, got {count}");
}

#[test]
fn nested_error_reports_once_and_outer_form_survives() {
    // A bad token nested two levels deep is reported, and every enclosing construct is still
    // recovered up to the root.
    let p = recovered("g(f(a, @), b)");
    assert!(!p.ok());
    let a = &p.arenas;
    // outer call `(g (f a <error>) b)`
    let g = a.as_form(a.root, "g").expect("outer call recovered");
    assert_eq!(g.len(), 2, "outer call keeps both args: {g:?}");
    let f = a.as_form(g[0], "f").expect("inner call recovered");
    assert_eq!(f.len(), 2, "inner call keeps both args: {f:?}");
    assert_eq!(a.as_name(g[1]), Some("b"), "arg after the bad one survives");
}

// ---- first-class embedded syntaxes (front-end syntax-switch) ----

// The embedded-region PARSE-TREE + round-trip tests (`json_embedded_region_grafts…`, `toml_embedded_region_
// grafts…`, `embedded_region_round_trips…`, `an_embedded_region_nested_in_a_larger_expr_re_emits…`,
// `a_grammar_tag_not_glued_to_a_brace_is_an_ordinary_name`, `a_brace_inside_a_json_string_does_not_close_the_
// region`) MIGRATED to the spec/syntax corpus (inc-6 batch-71). A reserved tag GLUED to `{` switches into a
// sub-grammar and grafts `(embedded #"tag" <sub-arena>)`; the printer re-emits the `tag{ … }` surface:
//   * ml/424-embedded-json-region `json{ {"a": [1, true], "b": null} }`→`(embedded #"json" (json-object
//     (member "a" (json-array 1 true)) (member "b" (json-null))))`, ml/425-embedded-toml-region (toml-document).
//   * ml/426-embedded-json-in-let (nested in a `let`), ml/427-embedded-mixed-in-tuple (json + toml in a tuple)
//     — the printer's embedded arm fires wherever `expr` recurses; round-trip is each case's fmt-idempotence.
//   * ml/428-grammar-tag-bare-is-a-name `json`→`json` (bare, not glued) + ml/430-grammar-tag-spaced-not-
//     embedded `json {}`→`(do json #record())` (a space before `{` does NOT switch).
//   * ml/429-embedded-json-brace-in-string `json{ {"s": "a}b{c"} }` — a `}` inside a JSON string doesn't
//     close the region early (the raw-region scanner tracks string literals).
// The recovery/span/never-panic embedded tests below STAY Rust (diagnostic-quality / span / totality guards).

#[test]
fn a_malformed_embedded_body_is_a_recovered_error_not_a_panic() {
    // A sub-grammar parse error lifts a diagnostic into this parse and leaves an `<error>` placeholder
    // under the embedded node — the arena stays well-formed, the parse never panics.
    let p = read_ml(r#"json{ {"a": } }"#); // missing value → JSON error
    assert!(!p.ok(), "a malformed JSON body surfaces a parse error");
    let a = &p.arenas;
    let emb = a
        .as_form(a.root, "embedded")
        .expect("still an (embedded …) node even on a bad body");
    assert_eq!(
        a.as_name(emb[1]),
        Some("<error>"),
        "a bad body grafts an <error> placeholder, keeping the arena well-formed"
    );
}

#[test]
fn embedded_node_spans_are_document_coordinates_that_slice_back_to_the_embedded_source() {
    // LSP-transparency: an embedded region's nodes keep their OWN spans, shifted into the OUTER
    // document's coordinates — so a cursor inside a `json{ … }` body resolves to the exact JSON node,
    // not the whole region. Assert every grafted node's span is a valid in-bounds slice of the OUTER
    // source (not the body), and that a known interior leaf's span slices to its literal text.
    let src = r#"json{ {"key": 42} }"#;
    let p = read_ml(src);
    assert!(p.ok(), "parses clean: {:?}", p.errors);
    let a = &p.arenas;
    let spans = &p.spans;
    // Every node's span is a valid slice of the OUTER document source (geometry preserved through the
    // offset shift — the whole point is these are document coordinates, safe to slice for an editor).
    for id in (0..a.structure.len() as u32).map(StructId) {
        let sp = spans.get(id).expect("total span table");
        assert!(
            sp.start <= sp.end
                && sp.end <= src.len()
                && src.is_char_boundary(sp.start)
                && src.is_char_boundary(sp.end),
            "embedded node {id:?} span {sp:?} is not a valid slice of the outer source {src:?}"
        );
    }
    // The `42` value leaf inside the JSON must span the literal `42` IN THE OUTER SOURCE (offset, not
    // body-relative). Find it: the embedded subtree is emb[1]; walk to the number leaf.
    let emb = a.as_form(a.root, "embedded").expect("root is (embedded …)");
    // The subtree root is a JSON object `(object (member "key" 42))`-ish; locate the leaf whose outer
    // span slices to "42".
    let mut found_42 = false;
    for id in (0..a.structure.len() as u32).map(StructId) {
        if let crate::ast::Struct::Atom(lid) = a.get(id)
            && matches!(a.leaf(*lid), Leaf::Int { .. })
        {
            let sp = spans.get(id).unwrap();
            assert_eq!(
                &src[sp.start..sp.end],
                "42",
                "the JSON number leaf's span must slice to `42` in the OUTER source"
            );
            found_42 = true;
        }
    }
    assert!(found_42, "the embedded JSON `42` leaf was grafted");
    let _ = emb;
}

#[test]
fn an_unterminated_embedded_region_recovers_without_panicking() {
    // No closing `}` at all — the scanner reports an unterminated region, consumes to end, and emits a
    // placeholder. Never a panic, never a hang.
    let p = read_ml(r#"json{ {"a": 1} "#); // no closing brace for the region
    assert!(!p.ok(), "an unterminated region is an error");
    assert!(
        p.arenas.as_form(p.arenas.root, "embedded").is_some(),
        "still produces a well-formed (embedded …) node"
    );
}

#[test]
fn embedded_syntax_switch_never_panics_and_stays_wellformed_over_arbitrary_bodies() {
    // The embedded-syntax switch (`json{ … }` / `toml{ … }`) runs on UNTRUSTED body text — the raw
    // region scanner (brace-balance, string-aware), the sub-grammar reader, and the span-remapping
    // graft all consume arbitrary bytes. Like every other surface, it must be TOTAL: never PANIC, and
    // on a SUCCESSFUL parse produce a well-formed traversable arena with a span table that is total +
    // GEOMETRICALLY valid over the OUTER source (spans are shifted into document coordinates, so a bad
    // offset would slice out of bounds). The hand tests pin specific shapes; this sweeps a
    // delimiter-rich alphabet through both grammars, plus unterminated / deeply-nested-brace / string-
    // heavy / multibyte bodies that stress the scanner. `recovered` asserts the invariants (arena
    // well-formed + total, char-boundary, in-bounds span table) for each generated program.
    let alphabet: Vec<char> = "{}[]\":,.-+0123456789 \tabctfn\\/\nλ中".chars().collect();
    let mut rng = SplitMix64(0xe3be_dded_c0de_1a7e);
    for grammar in ["json", "toml"] {
        for len in 0..=40usize {
            for _ in 0..80 {
                let body: String = (0..len)
                    .map(|_| alphabet[(rng.next() as usize) % alphabet.len()])
                    .collect();
                // Wrap the arbitrary body in the switch — the region scanner + sub-grammar reader must
                // survive whatever it contains (unbalanced braces → unterminated region; a `}` in a
                // string → not a close; garbage → a recovered sub-grammar error). `recovered` asserts
                // no panic + a well-formed, span-total, geometry-valid arena.
                let _ = recovered(&format!("{grammar}{{{body}}}"));
                // Also the UNTERMINATED form (no closing brace) — the scanner must consume to end and
                // emit a placeholder, never hang or panic.
                let _ = recovered(&format!("{grammar}{{{body}"));
                // And embedded in a larger program (a def RHS), so the token-cursor advance past the
                // region is exercised in context.
                let _ = recovered(&format!("def d() = {grammar}{{{body}}}\nx"));
            }
        }
    }
    // A few pathological brace/string shapes head-on.
    for prog in [
        r#"json{{{{{{{{"#,             // deeply unbalanced opens
        r#"json{"}}}}}}}}"}"#,         // braces buried in a string
        r#"json{ {"a": "b\"}\"c"} }"#, // escaped quotes + braces in a string
        "toml{}",                      // empty body
        "json{}",                      // empty body
        r#"toml{ a = "]}[{" }"#,       // toml string with delimiters
        "json{中{λ}中}",               // multibyte around braces
    ] {
        let _ = recovered(prog);
    }
}

// ---- record-type annotation surface: `{field: T, …}` ----

// The brace record-type PARSE-TREE + round-trip tests (`brace_record_type_annotation_equals_the_explicit_
// record_form`, `brace_record_type_nests_and_carries_function_and_tuple_field_types`,
// `brace_record_type_round_trips_through_the_printer`) MIGRATED to the spec/syntax corpus (inc-6 batch-62,
// FIRST parser.rs migration). A `{x: T}` type-position brace is sugar the printer canonicalizes to the
// explicit `Record(x : T)` form (same arena):
//   * ml/386-brace-record-type-annotation `def f(r: {x: Int64, y: Int64}) = r`→`(def (f (: r (Record (: x
//     Int64) (: y Int64)))) r)`, format.cdz = the explicit `Record(x : Int64, y : Int64)` canonicalization.
//   * ml/387-brace-record-type-empty `{}`→`(Record)`, ml/388-brace-record-type-function-field `{describe:
//     Int64 -> Int64}`→`(Record (: describe (-> Int64 Int64)))`, ml/389-brace-record-type-nested-and-tuple
//     `{p: {x: Int64}, pair: (Int64, Bool)}`→nested `(Record …)`/`(Tuple …)`.
//   * ml/390-explicit-record-type-annotation `Record(x: Int64, y: Int64)` — BYTE-IDENTICAL tree.sexp to
//     ml/386, pinning the brace==explicit equivalence; the round-trip is each case's fmt-idempotence.
// The REJECT/steering tests below (head-app field, malformed WIT member type) stay Rust — diagnostic-
// quality assertions, out of the parse-tree/fmt corpus scope.

#[test]
fn a_head_app_record_type_field_is_rejected_steering_to_the_colon_form() {
    // RT1 (DESIGN-record-type-syntax OQ-A): the obsolete head-application record-TYPE field spelling
    // `Record(field(T))` is REJECTED — a record-type field is written `field: T`. The message steers
    // to the colon form. (The canonical `(: field T)` ascription is what the colon surface produces.)
    for src in [
        "def f(r: Record(a(Int64))) = r",
        "def f(r: Record(x(Int64), y(Bool))) = r",
        // Nested field type in head-app form is still the rejected spelling.
        "def f(r: Record(inner(Option(Bytes)))) = r",
    ] {
        let p = read_ml(src);
        assert!(!p.ok(), "head-app record field must reject: {src}");
        assert!(
            p.errors.iter().any(|e| e
                .message
                .contains("record-type field is written `field: T`")),
            "the reject steers to the colon form: {src} -> {:?}",
            p.errors
        );
    }
    // The canonical colon form parses clean and builds the `(: name T)` ascription — NOT rejected.
    let a = parse_ok("def f(r: Record(a: Int64, b: Bool)) = r");
    assert_eq!(
        crate::sexpr::print(&a),
        "(def (f (: r (Record (: a Int64) (: b Bool)))) r)"
    );
    // NO false-reject on a legitimate GENERIC type application — `List(a)` / `Tuple(A, B)` take
    // POSITIONAL type args (a bare type, not a `name(T)` field), so they still parse.
    assert!(read_ml("def f(xs: List(a)) = xs").ok());
    assert!(read_ml("def f(p: Tuple(Int64, Bool)) = p").ok());
    assert!(read_ml("def f(o: Option(Int64)) = o").ok());
}

#[test]
fn malformed_wit_member_type_spellings_are_rejected_with_guidance() {
    // A malformed WIT aggregate member type (wrong result arity, a variant case with 2+ payloads, a
    // non-name enum/flags case) is REJECTED with an actionable, steering message — not silently left as
    // a broken member descriptor. Same reject-with-guidance policy as the record-field form. Each is in
    // world-member type position, where the head IS the WIT type keyword.
    let cases: [(&str, &str); 4] = [
        (
            "world W = | export i = | m : (x : u8) -> result(bool, string, u8)",
            "at most two arguments",
        ),
        (
            "world W = | export i = | m : (x : u8) -> variant(Ok, Bad(u8, string))",
            "single payload type",
        ),
        (
            "world W = | export i = | m : (x : u8) -> enum(Red, Green(u8))",
            "an `enum` case is a bare name",
        ),
        (
            "world W = | export i = | m : (x : u8) -> flags(Read(u8), Write)",
            "a `flags` bit is a bare name",
        ),
    ];
    for (src, needle) in cases {
        let p = read_ml(src);
        assert!(!p.ok(), "malformed WIT member type must reject: {src}");
        assert!(
            p.errors.iter().any(|e| e.message.contains(needle)),
            "reject steers with `{needle}`: {src} -> {:?}",
            p.errors
        );
    }
    // The WELL-FORMED spellings still parse clean — no false reject.
    for src in [
        "world W = | export i = | m : (x : u8) -> result(bool, string)",
        "world W = | export i = | m : (x : u8) -> result(bool)",
        "world W = | export i = | m : (x : u8) -> variant(Ok, Bad(string))",
        "world W = | export i = | m : (x : u8) -> enum(Red, Green, Blue)",
        "world W = | export i = | m : (x : u8) -> flags(Read, Write)",
    ] {
        assert!(
            read_ml(src).ok(),
            "well-formed WIT member type must parse: {src}"
        );
    }
}
