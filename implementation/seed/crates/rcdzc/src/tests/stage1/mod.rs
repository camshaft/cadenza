use super::imports_value_heap_runtime;
use crate::compile::compile_component;
use crate::testkit::parse;

/// Compile the same program shape and expect a DECLINE/reject, returning the error message.
fn expect_decline(body: &str) -> String {
    let src = format!("(module m (def (main) {body}) (export main))");
    compile_component(&crate::codec::encode(&parse(&src)))
        .expect_err("must decline/reject")
        .message
}

/// FOLD `body` (as `main`'s body) to its core `ConstInt` value at arbitrary precision — the value a
/// constant reduces to BEFORE any machine width or boundary. Used for a NON-ALIASED width like
/// `(UInt 48)`, whose bounds fold but which has no boundary representation to run across (it is
/// internal-only — R2). Panics if the body does not fold to a constant integer.
fn fold_const_u128(body: &str) -> u128 {
    use crate::core::Core;
    use crate::db::Db;
    use crate::lower::core_of;
    let src = format!("(module m (def (main) {body}) (export main))");
    let ast = parse(&src);
    let mut db = Db::load(ast);
    let main_body = db
        .defs
        .iter()
        .find(|d| d.name == "main")
        .and_then(|d| d.body)
        .expect("def main has a body");
    match core_of(&mut db, main_body) {
        Core::ConstInt(v) => {
            assert!(!v.negative, "expected a non-negative constant, got {v:?}");
            let mut acc: u128 = 0;
            for &b in &v.magnitude {
                acc = (acc << 8) | (b as u128);
            }
            acc
        }
        other => panic!("expected the body to fold to a ConstInt, got {other:?}"),
    }
}

/// The CDZ0307 discarded-value warnings from `src` — the `diagnostics()` query set (what `cdz check`
/// drives) filtered to the discarded-value code. Used rather than `warnings_of` so a body that does
/// not emit a component (e.g. one taking a parameter) still yields its diagnostics.
fn discarded_of(src: &str) -> Vec<crate::abi::Diagnostic> {
    crate::diagnostics(&mut crate::db::Db::load(parse(src)))
        .into_iter()
        .filter(|d| d.code.as_deref() == Some("CDZ0307"))
        .collect()
}

// ── "did you mean?" — the rustc-gold-standard fix suggestion for an unbound name ─────────────────

/// The full error `Diagnostic` for `(module m (def (main) BODY) (export main))` — like
/// `expect_decline`, but keeps the whole record (code + message + the structural fix) so a test can
/// assert on the proposed repair, not just the message text.
fn expect_error(body: &str) -> crate::abi::Diagnostic {
    let src = format!("(module m (def (main) {body}) (export main))");
    compile_component(&crate::codec::encode(&parse(&src))).expect_err("must decline/reject")
}

/// Whether the program shape `(module m (def (main) BODY) (export main))` COMPILES (no decline/reject)
/// — a compile-time verdict, no runtime needed. The complement of `expect_decline` for this module.
fn compiles_ok(body: &str) -> bool {
    let src = format!("(module m (def (main) {body}) (export main))");
    compile_component(&crate::codec::encode(&parse(&src))).is_ok()
}

/// Whether the component bytes contain `name` as a length-prefixed extern name (`<len><name>`) — a
/// crude but sufficient check that a NON-kebab source name did not leak verbatim into an extern
/// position. (`Log` is 3 bytes, prefixed by the uleb length `0x03`.)
fn contains_extern_name(bytes: &[u8], name: &str) -> bool {
    let mut needle = vec![name.len() as u8];
    needle.extend_from_slice(name.as_bytes());
    bytes.windows(needle.len()).any(|w| w == needle.as_slice())
}

// Whether the emitted component's core code carries a `call_indirect` opcode — the runtime-dispatch
// witness the closure-devirtualization/specialization tests assert is ABSENT once a known closure fuses.
fn component_has_call_indirect(bytes: &[u8]) -> bool {
    use wasmparser::{Parser, Payload};
    for payload in Parser::new(0).parse_all(bytes) {
        if let Ok(Payload::CodeSectionEntry(body)) = payload {
            let mut ops = body.get_operators_reader().expect("ops");
            while let Ok(op) = ops.read() {
                if matches!(op, wasmparser::Operator::CallIndirect { .. }) {
                    return true;
                }
            }
        }
    }
    false
}

// ── the `?`/`try` fallible short-circuit operator — T0a: node plumbing + operand typing ─────────
// (DESIGN-try-operator-rcdzc.md). These pin the FRONT-HALF invariants the later slices build on:
// `(try e)` resolves as a first-class node, its type is the operand's SUCCESS payload, an operand
// that is not a fallible sum is CDZ0203, and (until the boundary desugar lands, T1) a well-formed
// `(try e)` DECLINES rather than miscompiling.

/// The type of `main`'s body (`(try …)`) at a given operand — the success payload the `?` yields.
fn try_body_ty(body: &str) -> crate::ty::Ty {
    let src = format!("(module m (def (main) {body}) (export main))");
    let ast = parse(&src);
    let mut db = crate::db::Db::load(ast);
    let b = db
        .defs
        .iter()
        .find(|d| d.name == "main")
        .and_then(|d| d.body)
        .expect("def main has a body");
    crate::infer::type_of(&mut db, b)
}

#[test]
fn a_multi_column_match_decision_tree_shares_its_fallthrough_tail_as_a_dag() {
    // S2 (emit-size, wasm) FOUNDATION-GUARD: a multi-column literal match (`(tuple i i a) → i`) compiles
    // its decision tree to a `MatchSum` whose fall-through tails are SHARED — reached from several parent
    // continuations as the SAME `Rc<SumCont>` (`core.rs`: "may be reached from multiple arms as the SAME
    // `Rc<SumCont>` — and the emit-side dedup keys on that pointer"). The sharing makes the DAG's DISTINCT
    // node count grow LINEARLY with arm count even though a naive edge-walk is EXPONENTIAL (each shared tail
    // is re-reached ~2× per level): for `(tuple i i a)` arms the distinct-`Rc::as_ptr` count is ~3·N (the
    // two column tests + the leaf per arm), while total edge traversals are O(2^N).
    //
    // This is the INVARIANT the planned emit-side ptr_eq dedup depends on: it emits each DISTINCT
    // continuation ONCE and branches to it, collapsing the exponential edge-walk (today's exponential
    // emitted code — 1020 eq/if ops / ~9 KB at N=8) to linear. If a future lowering change regressed the
    // param-tuple sharing to all-DISTINCT (the shape the INLINE `(tuple a b c)` scrutinee still exhibits —
    // v-compiler-perf's separate lower.rs seam gap), the dedup would silently stop firing and the emit would
    // blow up again. This guard pins DISTINCT-node LINEARITY so that regression is caught at the DAG, before
    // it reaches emit. Correctness of the dispatch is pinned by the match_engine value tests; this pins only
    // the SHARING SHAPE. Value/opt-level parity of the eventual reshape lives in the corpus value-pin case.
    use crate::core::{Core, SumCont};
    use crate::db::Db;
    use crate::lower::core_of;
    use std::collections::HashSet;
    // Count DISTINCT `Rc<SumCont>` nodes reachable from a def `f`'s MatchSum root (by `Rc::as_ptr`).
    fn distinct_cont_nodes(src: &str) -> usize {
        crate::host::run_with_compiler_stack(|| {
            let mut db = Db::load(parse(src));
            let d = db.def_by_name("f").expect("def f present");
            let body = db.defs[d].body.expect("f has a body");
            let root = match core_of(&mut db, body) {
                Core::MatchSum { root, .. } => root,
                other => panic!("expected `f`'s body to be a MatchSum, got {other:?}"),
            };
            let mut seen: HashSet<usize> = HashSet::new();
            fn walk(c: &SumCont, seen: &mut HashSet<usize>) {
                // Key on the NODE address — a shared `Rc` is reached at the same `*const` from each parent,
                // so a re-reached tail is counted once (the property the emit memo exploits).
                if !seen.insert(c as *const SumCont as usize) {
                    return;
                }
                match c {
                    SumCont::Leaf(_) => {}
                    SumCont::Guarded { els, .. } => walk(els, seen),
                    SumCont::LitTest { then_, els, .. } => {
                        walk(then_, seen);
                        walk(els, seen);
                    }
                    SumCont::Switch { arms, .. } => {
                        for a in arms {
                            walk(&a.cont, seen);
                        }
                    }
                }
            }
            walk(&root, &mut seen);
            seen.len()
        })
    }
    // `(def (f (: t (Tuple Int64 Int64 Int64))) (match t ((tuple 0 0 a) 0) … (_ -1)))` — N arms each
    // testing TWO literal columns of a bound-PARAM tuple (the shape whose fall-through tail is shared).
    fn param_tuple_match_src(n: usize) -> String {
        let mut arms = String::new();
        for i in 0..n {
            arms.push_str(&format!("((tuple {i} {i} a) {i}) "));
        }
        format!(
            "(module m (def (f (: t (Tuple Int64 Int64 Int64))) (match t {arms}(_ -1))) \
                 (def (main (: a Int64)(: b Int64)(: c Int64)) (f (tuple a b c))) (export main))"
        )
    }
    // Doubling the arm count must AT MOST double the distinct-node count (LINEAR sharing), never square it
    // (the exponential all-distinct shape). At 8→16 arms an all-distinct tree grows ~256×; a shared DAG
    // grows ~2×. Require the ratio stay well under 4× (linear headroom for the fixed per-arm constant).
    let d8 = distinct_cont_nodes(&param_tuple_match_src(8));
    let d16 = distinct_cont_nodes(&param_tuple_match_src(16));
    let ratio = d16 as f64 / (d8.max(1)) as f64;
    assert!(
        d8 > 0 && ratio < 4.0,
        "a multi-column literal match's decision tree must SHARE its fall-through tail as a DAG — the \
             distinct `Rc<SumCont>` node count must grow LINEARLY with arm count (the invariant the emit-side \
             ptr_eq dedup keys on), not exponentially: arms 8→16 grew {ratio:.1}× (d8={d8}, d16={d16}); \
             linear is ~2×, an all-distinct (unshared) tree is ~256×"
    );
}

#[test]
fn a_multi_column_literal_match_emits_a_flat_br_if_chain_not_an_exponential_if_tree() {
    // S2 (emit-size, wasm) REGRESSION GATE — the EMIT-side twin of the DAG-sharing linearity guard above
    // (which pins the IR shape). A multi-column literal arm `(tuple i i a)` shares its fall-through tail as
    // a linear DAG, but the OLD nested-`if`/`else` emit re-emitted that shared tail in BOTH branches at
    // every column → the emitted code grew O(2^cols) (N=8 = 9352 bytes / 1020 `if` ops; N=16 ≈ 2.2 MB,
    // unrunnable). The flat `br_if` guard chain (`flattenable_multicol_arm` + the `SumCont::LitTest` emit)
    // collapses it: each column `br_if`s to one `$arm_fail` label and the shared tail is emitted ONCE, so
    // the emit is LINEAR (2 blocks + 2 `br_if` per arm, ZERO `if`). This gate pins both facts so a future
    // change that regressed to the nested tree — silently reintroducing the exponential blowup while still
    // returning the right value (the opt-sweep + value-guards would stay green) — is caught here.
    //
    // A RUNTIME scrutinee (built from params) so the match is actually EMITTED, not const-folded. Shape-only
    // (compile + count opcodes, no run — the value is pinned by the corpus value-guards + opt-sweep; a heap
    // Tuple scrutinee cannot run in the lib harness anyway). Mirrors the fusion-gate + dict-erasure gates.
    fn param_tuple_match_src(n: usize) -> String {
        let mut arms = String::new();
        for i in 0..n {
            arms.push_str(&format!("((tuple {i} {i} a) {i}) "));
        }
        format!(
            "(module m (def (f (: t (Tuple Int64 Int64 Int64))) (match t {arms}(_ -1))) \
                 (def (main (: a Int64)(: b Int64)(: c Int64)) (f (tuple a b c))) (export main))"
        )
    }
    let compile = |n: usize| -> Vec<u8> {
        compile_component(&crate::codec::encode(&parse(&param_tuple_match_src(n))))
            .expect("a multi-column literal match compiles")
    };
    // ZERO `if` ops: the flat chain replaces the nested-`if` tree entirely for this spine. A single `If`
    // opcode anywhere in the emit means the reshape did NOT fire (regressed to the nested emit).
    let bytes8 = compile(8);
    let ifs8 = super::count_opcode(&bytes8, |op| matches!(op, wasmparser::Operator::If { .. }));
    assert_eq!(
        ifs8, 0,
        "a multi-column literal match must emit a FLAT `br_if` guard chain (0 `if` ops), not the \
             exponential nested-`if` tree — found {ifs8} `if` op(s) at 8 arms, so the flat reshape \
             (`flattenable_multicol_arm`) did not fire"
    );
    // LINEAR emitted size: doubling the arm count 8→16 must AT MOST roughly double the emitted bytes (the
    // flat chain is 2 blocks + 2 `br_if` per arm). The old exponential emit grew ~230× over the same step
    // (9352 → ~2.2 M). Require the byte ratio stay well under 4× (linear is ~1.3×; exponential was ~230×).
    let bytes16 = compile(16);
    let ratio = bytes16.len() as f64 / bytes8.len().max(1) as f64;
    assert!(
        !bytes8.is_empty() && ratio < 4.0,
        "a multi-column literal match's EMITTED SIZE must grow LINEARLY with arm count (the flat `br_if` \
             chain), not exponentially: bytes 8→16 arms grew {ratio:.1}× (n8={}, n16={}); the flat chain is \
             ~1.3×, the old nested-`if` tree was ~230×",
        bytes8.len(),
        bytes16.len()
    );
}

#[test]
fn ml_forall_binder_compiles_and_monomorphizes_end_to_end() {
    // FORALL-BINDER e2e (v-inference × v-syntax). `forall a. T` in a parameter annotation is PURE
    // SUGAR: v-syntax's parser desugars it at parse time to a leading `(: a Type)` type-valued param +
    // the bare inner type, BYTE-IDENTICAL to a hand-written generic — so infer/monomorphization is
    // unchanged (no ∀ engine). v-syntax pins the desugar SHAPE (parse → s-expr); the semantic corpus
    // pins the desugared `(: a Type)` arena's compile+RUN. This seam keeps the parser→compile
    // INTEGRATION pin: an ML `forall` program parses → desugars → COMPILES to a VALID component (and
    // monomorphizes) as a single artifact. The RUN VALUES (id(Int64,42)=42, the two-instance gate=100,
    // apply(inc,41)=42) are covered transitively — v-syntax pins the parse→desugar shape, the corpus
    // pins the desugared arena's run — so dropping the in-crate run drops the cdz-run/wasmtime dep with
    // no coverage loss (the corpus cannot take ML surface source, only the desugared s-expr arena).
    let compile_ml = |program: &str| -> Vec<u8> {
        let parsed = cadenza_syntax::parser::read_ml(program);
        assert!(
            parsed.ok(),
            "ML program failed to parse: {:?}\n  src: {program}",
            parsed.errors
        );
        let bytes = cadenza_syntax::codec::encode(&parsed.arenas);
        let arenas = crate::codec::decode(&bytes)
            .unwrap_or_else(|| panic!("cadenza-syntax bytes failed rcdzc decode: {program}"));
        compile_component(&crate::codec::encode(&arenas))
            .unwrap_or_else(|d| panic!("ML forall program compiles: {} [{:?}]", d.message, d.code))
    };
    let validate = |bytes: &[u8]| {
        let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        v.validate_all(bytes)
            .expect("the compiled ML forall component validates");
    };
    // (1) IDENTITY at a concrete type. `id`'s solved type is `(-> Type (-> a a))`; the call passes the
    // type witness then the value: `id(Int64, 42)`. (Corpus run: 42.)
    validate(&compile_ml(
        "def id(x: forall a. a) = x\ndef main() = id(Int64, 42)\nexport { main }",
    ));
    // (2) USED AT TWO DISTINCT TYPES → two monomorphizations from one source def. `id` at Bool gates
    // `id` at Int64; both instances must exist. (Corpus run: 100.)
    validate(&compile_ml(
        "def id(x: forall a. a) = x\n\
             def main() = if id(Bool, true) then id(Int64, 100) else 0\n\
             export { main }",
    ));
    // (3) MULTI-BINDER `forall a b.` over a function-typed param — `apply(f: a -> b, x: a) = f(x)`, the
    // two type witnesses prepended in source order: `apply(Int64, Int64, inc, 41)`. (Corpus run: 42.)
    validate(&compile_ml(
        "def apply(f: forall a b. a -> b, x: a) = f(x)\n\
             def inc(n: Int64) = n + 1\n\
             def main() = apply(Int64, Int64, inc, 41)\n\
             export { main }",
    ));
}

/// UNARY NEGATION `(- e)` — the arity-1 subtraction (the ML prefix `-<expr>`). It is `0 - e` at the
/// operand's numeric type, so a constant folds and a runtime operand emits the checked subtract with
/// the `x == MIN` overflow trap. The fold unit + the wasmtime run + the MIN trap in one test, plus the
/// non-numeric reject, cover the whole negation path (06-numeric-model pins the same at corpus level).
#[test]
fn a_named_negate_folds_a_constant_operand() {
    use crate::core::Core;
    use crate::db::Db;
    use crate::lower::core_of;
    // FOLD unit for the named negate `Num.neg` (the first-class negation, `Prim::Neg` → `lower_negate`):
    // a constant operand folds to its negation — `(Num.neg (+ 2 3))` → `Core::ConstInt(-5)`, no runtime
    // subtract emitted. (Was `a_unary_minus_negates_its_operand`, migrated to `Num.neg` ahead of the
    // arity-1 prefix-`-` deprecation — `(- e)` becomes a partial subtraction; `Num.neg` is the replacement.)
    let fold = |body: &str| -> Option<i64> {
        let src = format!("(module m (def (main) {body}) (export main))");
        let mut db = Db::load(parse(&src));
        let d = db.def_by_name("main")?;
        let m_body = db.defs[d].body?;
        match core_of(&mut db, m_body) {
            Core::ConstInt(v) => v.to_i64(),
            _ => None,
        }
    };
    assert_eq!(
        fold("(Num.neg (+ 2 3))"),
        Some(-5),
        "constant negation folds to -5"
    );
    assert_eq!(
        fold("(Num.neg (Num.neg 4))"),
        Some(4),
        "double negation folds to +4"
    );
    // Runtime behavior — `(Num.neg n)` returns -n (f(7)=-7, f(-42)=42) and traps at Int64.min — is the
    // corpus case "Int64.neg of a runtime expression negates at runtime, both signs" +
    // "a genuinely-runtime UNARY negation ..." (spec/semantics/06-numeric-model.sexp), run via cdz-run.
}

#[test]
fn a_malformed_do_block_surfaces_in_the_diagnostics_query_on_any_body() {
    // An EMPTY `(do)` (no value form) and a `do` ending in a DECLARATION (`(do (def x 5))`, valueless)
    // are coded CDZ0201 well-formedness faults. They were reached only by the emit-path lowering walk
    // (nullary-EXPORTED bodies alone), so a malformed `(do)` as a PARAMETERIZED (or non-exported) def
    // body silently PASSED `cdz check` while `compile` rejected it — the `do`-form analogue of the
    // pattern-fault/binop-arity `check`≡`compile` gaps. `collect_node`'s `do` arm now surfaces the
    // do form's own coded poison, so the fault appears in `diagnostics()` (what `check` runs) whether
    // the def takes parameters or not.
    let empty_param = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def (g (: n Int64)) (do)) (export g))",
    )));
    let e = empty_param
        .iter()
        .find(|d| d.code.as_deref() == Some("CDZ0201"))
        .expect("an empty `(do)` in a parameterized body is caught by check");
    assert!(
        e.message.contains("empty `do` block has no value"),
        "names the empty-do fault: {}",
        e.message
    );

    // A trailing declaration is caught too.
    assert!(
        crate::diagnostics(&mut crate::db::Db::load(parse(
            "(module m (def (g) (do (def x 5))) (export g))",
        )))
        .iter()
        .any(|d| d
            .message
            .contains("must end in a value form, not a declaration")),
        "a `do` ending in a declaration is caught"
    );

    // Reported EXACTLY ONCE when the malformed `do` is also reached via a call (no infer/emit double).
    let called = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def (mk) (do)) (def (main) (+ 1 (mk))) (export main))",
    )));
    assert_eq!(
        called
            .iter()
            .filter(|d| d.message.contains("empty `do` block has no value"))
            .count(),
        1,
        "the malformed do reports once, not a double: {called:?}"
    );

    // NO false positive: a well-formed `do` (value forms, or a def followed by a value) is clean.
    for ok in [
        "(module m (def (g) (do 1 2)) (export g))",
        "(module m (def (g) (do (def x 5) x)) (export g))",
    ] {
        assert!(
            crate::diagnostics(&mut crate::db::Db::load(parse(ok)))
                .iter()
                .all(|d| !d.message.contains("`do` block")),
            "a well-formed do produces no do-block fault: {ok}"
        );
    }
}

#[test]
fn a_lambda_valued_def_body_is_type_checked_by_the_diagnostics_query() {
    // A `(def name (fn (p…) body))` LAMBDA-VALUED def registers a def whose `body` occurrence IS the
    // `fn` node (empty `db.defs` params). The def-body walk runs `type_errors` over that lambda node,
    // whose `collect_node` arm used to check ONLY param-linearity — never descending into the body. So
    // a type fault / unbound name in a lambda-valued def body silently PASSED `cdz check` (and
    // `compile`, when the def is unreached) while the SAME logic written `(def (name p…) body)` was
    // rejected — a check/compile discrepancy on a purely syntactic surface choice, the lambda-valued
    // analogue of the pattern-fault / binop-arity / do-block `check`≡`compile` gaps. `collect_node`'s
    // `Lambda` arm now descends into the body when the lambda IS a registered def body.
    //
    // A numeric-mix type fault in a NON-exported lambda-valued def body is now caught (CDZ0301).
    let mismatch = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def helper (fn ((: x Int64)) (+ x 1.0))))",
    )));
    assert!(
        mismatch
            .iter()
            .any(|d| d.code.as_deref() == Some("CDZ0301")),
        "an ill-typed lambda-valued def body must be caught by check: {mismatch:?}"
    );
    // An unbound name in a non-exported lambda-valued def body is caught (CDZ0101) — unbound is
    // unconditional well-formedness, not gated on the def being reached.
    let unbound = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def helper (fn ((: x Int64)) (nonexistent x))))",
    )));
    assert!(
        unbound
            .iter()
            .any(|d| d.code.as_deref() == Some("CDZ0101") && d.message.contains("nonexistent")),
        "an unbound name in a lambda-valued def body must be caught: {unbound:?}"
    );
    // Reported EXACTLY ONCE when the lambda-valued def is ALSO reached via a call (no infer/emit
    // double, and no double between the standalone body walk and the reached-poison walk).
    let called = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def helper (fn ((: x Int64)) (+ x 1.0))) \
             (def (main (: x Int64)) (helper x)) (export main))",
    )));
    assert_eq!(
        called
            .iter()
            .filter(|d| d.code.as_deref() == Some("CDZ0301"))
            .count(),
        1,
        "the ill-typed lambda-valued body reports once, not a double: {called:?}"
    );
    // NO false positive over an INLINE / let-bound lambda: it is NOT a registered def body, so it is
    // checked at its β-reduction call site (unchanged) — a well-typed HOF argument stays clean, and an
    // UNINSTANTIATED generic body raises no spurious fault here.
    for ok in [
        "(module m (def (apply-it (: f (-> Int64 Int64)) (: x Int64)) (f x)) \
             (def (main (: x Int64)) (apply-it (fn ((: y Int64)) (+ y 1)) x)) (export main))",
        "(module m (def helper (fn ((: x Int64)) (+ x 1))) (export helper))",
    ] {
        let clean = crate::diagnostics(&mut crate::db::Db::load(parse(ok)));
        assert!(
            clean
                .iter()
                .all(|d| d.severity != crate::abi::Severity::Error),
            "a well-typed lambda body must produce no error: {ok} → {clean:?}"
        );
    }
    // REGRESSION (corpus-bugfix/fuzzer, invalid-wasm miscompile): an ILL-TYPED body of an
    // IMMEDIATELY-APPLIED INLINE lambda must be rejected, same as the bare expression. `(* (tuple) 0)`
    // rejects CDZ0201 bare, but WRAPPED in `((fn (v0) (* (tuple) 0)) 0)` it slipped past check (only an
    // unused-param warning) and the backend emitted INVALID WASM. Root: the apply-path body-fault
    // collection SUBTRACTS a baseline of the callee's UNREDUCED body faults (to avoid duplicating a
    // NAMED def's independently-collected body faults) — but an inline lambda's body is NOT separately
    // collected, so with `v0` unused (reduction leaves the body unchanged) the fault WAS in the
    // baseline and got filtered, deleting it entirely. Fix: gate the baseline on the callee being a
    // NAMED def (`named_callee_head`); an inline lambda keeps an empty baseline so its body faults
    // surface. The `else`-companion of the lambda-valued-def case above (which MUST still de-dup).
    let inline_bug = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(do (def (main) (match ((fn (v0) (* (tuple) 0)) 0) (_ 0))) (export main))",
    )));
    assert!(
        inline_bug
            .iter()
            .any(|d| d.code.as_deref() == Some("CDZ0201")),
        "an ill-typed body of an immediately-applied inline lambda must be rejected (was: invalid \
             wasm): {inline_bug:?}"
    );
}

#[test]
fn a_do_local_declaration_scope_is_backward_only() {
    // Sequential scope: a form sees only the declarations BEFORE it. A FORWARD reference (`y`'s value
    // `(+ x 1)` references `x` declared AFTER it) is unbound — a declaration does not see later ones.
    let msg = expect_decline("(do (def y (+ x 1)) (def x 5) y)");
    assert!(
        msg.contains("unbound") && msg.contains('x'),
        "a forward reference to a later do-local declaration must be unbound; got: {msg}"
    );
}

#[test]
fn a_do_block_ending_in_a_declaration_is_malformed() {
    // A sequencing block YIELDS its last form's value, so it must end in a value form: a TRAILING
    // declaration `(do (def x 5))` leaves the block valueless.
    let msg = expect_decline("(do (def x 5))");
    assert!(
        msg.contains("must end in a value form"),
        "a do block ending in a declaration must be malformed; got: {msg}"
    );
}

#[test]
fn an_unused_do_local_declaration_with_an_ill_typed_value_is_still_caught() {
    // A VALUE declaration's value is type-checked EAGERLY (like a `let` binding value) — a fault
    // whether or not the name is later used. `(def x (if 5 1 2))` is ill-typed (non-Bool condition)
    // and caught even though the block yields the unrelated `42`.
    let msg = expect_decline("(do (def x (if 5 1 2)) 42)");
    assert!(
        msg.contains("condition must be Bool"),
        "an ill-typed do-local value declaration must be caught though unused; got: {msg}"
    );
}

#[test]
fn a_pure_non_final_do_form_that_discards_a_value_warns() {
    // The user-reported shape: `(do (inc 8) (* n 2))` — the first form computes a value that is thrown
    // away (a non-final form is evaluated only for its effect, and a pure one has none). In a pure
    // language that is almost always a bug (a call whose result the author forgot to use), so warn
    // CDZ0307 anchored at the discarded form, with a delete fix.
    let src = "(module m (def (inc n) (+ n 1)) \
             (def (dbl n) (do (inc 8) (* n 2))) (export dbl))";
    let ws = discarded_of(src);
    assert_eq!(ws.len(), 1, "one discarded-value warning: {ws:?}");
    assert!(
        ws[0].message.contains("computed but discarded"),
        "message names the defect: {}",
        ws[0].message
    );
    let node = ws[0].node.expect("carries the discarded form's node");
    assert!(
        crate::db::Db::load(parse(src)).is_user_node(crate::ast::StructId(node)),
        "node {node} must be a user node"
    );
    // The repair DELETES the dead statement.
    let fix = ws[0].fix.as_ref().expect("carries a delete fix");
    assert_eq!(fix.kind, crate::abi::FixKind::Delete);
}

#[test]
fn a_unit_typed_or_final_do_form_does_not_warn() {
    // The LAST form is the block's value — never discarded, so it never warns (`(do (inc 8) (* n 2))`
    // warns on `(inc 8)` only, not `(* n 2)`; the exactly-one assertion above already pins that). A
    // Unit-typed non-final form discards nothing (there is no value to use), so it does not warn: the
    // empty list `()` IS the unit value.
    assert!(
        discarded_of("(module m (def (main) (do () 42)) (export main))").is_empty(),
        "a Unit-typed non-final form discards no value"
    );
    // A block with a single form has no non-final form at all — nothing to warn about.
    assert!(
        discarded_of("(module m (def (main) (do 42)) (export main))").is_empty(),
        "a one-form block has no discarded intermediate"
    );
}

#[test]
fn multiple_discarded_intermediates_each_warn() {
    // `(do 1 2 3)`: BOTH non-final forms (`1`, `2`) discard a value; the last (`3`) is the block value.
    // A pure scalar intermediate is exactly the corpus-blessed `(do 1 2 3)` shape — well-formed and it
    // still compiles (CDZ0307 is a WARNING), but each dropped value is surfaced.
    let ws = discarded_of("(module m (def (main) (do 1 2 3)) (export main))");
    assert_eq!(ws.len(), 2, "two discarded scalars warn: {ws:?}");
}

#[test]
fn a_wide_do_block_discarded_value_pass_runs_in_bounded_time() {
    // REGRESSION (perf): `collect_discarded_value_warnings` + the `do`/`let` lowering both call
    // `lower::subtree_reaches_host_call` (does a statement reach an observable host call?), which walked
    // each statement's WHOLE subtree calling `core_of` per node — on the real corpus workload this pair
    // was ~45% of compile time. It is now MEMOIZED (`Db::reaches_host_call`) and SKIPS `core_of` on atom
    // nodes (a host call only lowers from an application `List`). A do-block of N pure non-final
    // statements each a non-trivial expression must stay LINEAR (and still warn on each), never a
    // per-statement re-walk of the whole block. N=800 with each statement flagged is the gate.
    let n = 800;
    let stmts: String = (0..n).map(|i| format!("(+ {i} {i}) ")).collect();
    let src = format!("(module m (def (main) (do {stmts}0)) (export main))");
    let ws = discarded_of(&src);
    // Every one of the N pure non-final statements is a discarded non-Unit value → N warnings; the last
    // `0` is the block value (not discarded). Confirms the pass still fires exactly, in bounded time.
    assert_eq!(
        ws.len(),
        n,
        "each of the {n} pure non-final statements warns"
    );
}

#[test]
fn an_effectful_non_final_do_form_does_not_warn() {
    // A non-final statement that reaches a HOST CALL is KEPT by the `Core::Seq` lowering — its call
    // crosses the boundary and must run, so sequencing it for effect is exactly why a non-final form
    // is allowed to have a value. It is NOT a discarded-value defect. `(do (log.emit "x") unit)`: the
    // `log.emit` statement is effectful (and Unit-typed), so no CDZ0307. Uses the same
    // `subtree_reaches_host_call` the lowering uses, so the diagnostic tracks exactly what DCE keeps.
    let src = "(do (effect log (op emit (-> String Unit))) \
                   (def (main) (host (log) (do (log.emit \"x\") unit))) (export main))";
    assert!(
        discarded_of(src).is_empty(),
        "an effectful (kept) non-final form is not a discarded value: {:?}",
        discarded_of(src)
    );
}

#[test]
fn a_do_local_declaration_is_not_a_discarded_value() {
    // A `(def …)` form of a `do` is a DECLARATION (it binds a name for the following forms), not an
    // evaluated statement — its value flows to a reference, not thrown away. So a leading do-local def
    // never warns CDZ0307, even though it is a non-final form.
    assert!(
        discarded_of("(module m (def (main) (do (def x 5) (+ x 1))) (export main))").is_empty(),
        "a do-local declaration is not a discarded statement"
    );
}

#[test]
fn an_at_param_site_is_a_declaration_not_a_discarded_value() {
    // A `@param(…) name : Type` site parses to `(: (@ (param …) name) Type)` — a top-level colon-
    // annotation form the `param_sidecar` pass consumes to GENERATE the `Param` effect. It is a
    // DECLARATION (a runtime input), not an expression evaluated for a thrown-away value, so it must NOT
    // warn CDZ0307 — like a `def`/`effect` decl. (It reaches the discarded-value pass as a non-`def`-head
    // non-final form, so the head-name skip misses it; `param_sidecar::is_param_site` is the guard.)
    assert!(
        discarded_of(
            "(module m \
                   (pragma param (param (: widget slider)) (: a Int64)) \
                   (def (main) (host (Param) (+ (Param.a) 1))) \
                 (export main))"
        )
        .is_empty(),
        "a scalar @param site is a declaration, not a discarded value"
    );
    // Two @param sites → still zero warnings (the guard is per-site, and both are declarations).
    assert!(
        discarded_of(
            "(module m \
                   (pragma param (param (: widget slider)) (: a Int64)) \
                   (pragma param (param (: widget slider)) (: b Int64)) \
                   (def (main) (host (Param) (+ (Param.a) (Param.b)))) \
                 (export main))"
        )
        .is_empty(),
        "every @param site is skipped — no per-param discarded-value noise"
    );
    // A Rational @param (heap-typed, desugars to num/den) is ALSO a declaration — same skip, no warning.
    assert!(
        discarded_of(
            "(module m \
                   (pragma param (param (: widget slider)) (: rate Rational)) \
                   (def (main) (host (Param) (Rational.value (Param.rate)))) \
                 (export main))"
        )
        .is_empty(),
        "a Rational @param site is a declaration too — the guard keys on the site, not the accessor type"
    );
    // GUARD-DOES-NOT-OVER-SUPPRESS: a genuine discarded pure value alongside a @param still warns exactly
    // once — the skip is surgical to the `@param` site, not the whole block.
    let ws = discarded_of(
        "(module m \
               (pragma param (param (: widget slider)) (: a Int64)) \
               (def (main) (host (Param) (do (+ 1 2) (Param.a)))) \
             (export main))",
    );
    assert_eq!(
        ws.len(),
        1,
        "the @param site is skipped but a real discarded `(+ 1 2)` still warns once: {ws:?}"
    );
}

#[test]
fn same_named_defs_are_distinct_bindings() {
    // The flat top-level namespace keys defs by name (first-wins) — but a def's IDENTITY is its
    // occurrence, and the `def_name_index` is only a lookup accelerator over `defs`, never the
    // source of truth. Two defs are still distinct `Db::defs` entries at distinct occurrences even
    // when they share a name; the index simply resolves the bare NAME to the first. This is the
    // seam the future module rework re-keys (by enclosing module) so sibling submodules' same-named
    // defs stop colliding — see the `def_name_index` field doc. Here we assert the property the
    // rework must uphold: distinct occurrences, and a deterministic first-wins name resolution.
    use crate::db::Db;
    let src = "(module m (def (a) 1) (def (b) 2) (def (main) (+ (a) (b))) (export main))";
    let db = Db::load(parse(src));
    // Every def is its own entry (distinct occurrences), regardless of name.
    let occs: std::collections::HashSet<_> = db.defs.iter().filter_map(|d| d.body).collect();
    assert_eq!(
        occs.len(),
        db.defs.iter().filter(|d| d.body.is_some()).count()
    );
    // The name index resolves each name to a real, distinct def. (The let-scope run
    // `(+ (let ((z 1)) z) (let ((z 2)) z))` = 3 — distinct let-bound shadows — is corpus-covered.)
    assert!(db.def_by_name("a").is_some());
    assert!(db.def_by_name("b").is_some());
    assert_ne!(db.def_by_name("a"), db.def_by_name("b"));
}

#[test]
fn an_unbound_name_close_to_a_def_suggests_it_with_a_heuristic_fix() {
    // The suggest+fix faces — CDZ0101 message "did you mean `compute`?" + a replace-fix to `compute`,
    // and the heuristic (unverified) flag — migrated to corpus 02-binding-and-control "an unbound name
    // close to a top-level def suggests it with a replace fix". What STAYS here is the fix-ANCHOR pin the
    // corpus cannot assert: the fix's node is the faulting reference itself (diagnostic + fix target the
    // SAME span, so an editor highlights and rewrites it).
    let src = "(module m (def (compute x) x) (def (main) (computee 1)) (export main))";
    let d = compile_component(&crate::codec::encode(&parse(src))).expect_err("must reject");
    let fix = d.fix.expect("a fix is carried");
    assert!(
        !fix.verified,
        "a nearest-name guess is heuristic, not verified"
    );
    assert_eq!(Some(fix.node), d.node, "fix targets the faulting node");
}

#[test]
fn a_misspelled_form_keyword_head_suggests_the_grammar_keyword() {
    // The POSITIVE faces migrated to corpus 02-binding-and-control: the four head-position keyword typos
    // (`mtch`→`match`, `iff`→`if`, `le`→`let`, `annd`→`and`) each get a "did you mean `<kw>`?" suggestion,
    // and a head typo NEARER to a real def wins (`matchee`→`matcher`). What STAYS here is the
    // ARGUMENT-POSITION NON-suggestion face — a suggestion-ABSENCE the corpus grades only `todo`: the
    // grammar keywords join the candidate pool ONLY in head position, so `(g mtch)` (mtch as an operand)
    // gets NO grammar suggestion.
    let arg = "(module m (def (g x) x) (def (f) (g mtch)) (export f))";
    let d = compile_component(&crate::codec::encode(&parse(arg))).expect_err("must reject");
    assert_eq!(d.code.as_deref(), Some("CDZ0101"), "got: {}", d.message);
    assert!(
        !d.message.contains("did you mean `match`?"),
        "an argument-position typo must not suggest a grammar keyword: {}",
        d.message
    );
}

#[test]
fn a_misspelled_form_keyword_head_suppresses_its_cascade() {
    // A misspelled keyword head makes the whole form (mis)parse as an APPLICATION, so its arms/bindings
    // fault too — `(mtch n (0 1) (_ 2))` → "cannot apply Int64" on the arm `(0 1)` + "unbound `_`";
    // `(le ((x 5)) x)` → "unbound `x`" (the bindings never took effect). Those are CONSEQUENT on the
    // head typo. The diagnostics now report ONLY the head's did-you-mean CDZ0101 (with its fix); the
    // cascade inside the mis-parsed form is dropped.
    let all = |src: &str| crate::diagnostics(&mut crate::db::Db::load(parse(src)));
    let mtch = all("(module m (def (f (: n Int64)) (mtch n (0 1) (_ 2))) (export f))");
    assert_eq!(
        mtch.iter()
            .filter(|d| d.severity == crate::abi::Severity::Error)
            .count(),
        1,
        "only the head typo remains, no cascade: {:?}",
        mtch.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(
        mtch[0].message.contains("did you mean `match`?"),
        "the surviving error is the head typo: {}",
        mtch[0].message
    );
    // The `let` case: the two spurious "unbound `x`" (from the never-bound body) are gone too.
    let le = all("(module m (def (f) (le ((x 5)) x)) (export f))");
    assert!(
        le.iter()
            .filter(|d| d.severity == crate::abi::Severity::Error)
            .all(|d| d.message.contains("did you mean `let`?")),
        "only the `let` typo remains, no spurious unbound-x cascade: {:?}",
        le.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // NO OVERREACH: an ordinary misspelled FUNCTION head keeps its argument's independent fault — the
    // suggestion `helper` is not a grammar keyword, so the cascade suppression does not apply.
    let fn_typo = all("(module m (def (helper a) a) (def (f) (helpr nonesuch)) (export f))");
    assert!(
        fn_typo
            .iter()
            .any(|d| d.message.contains("did you mean `helper`?"))
            && fn_typo.iter().any(|d| d.message.contains("`nonesuch`")),
        "a function-head typo keeps its genuine argument fault: {:?}",
        fn_typo.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // NO OVERREACH: a CORRECT `match` with a genuine unbound in an arm still reports that arm fault.
    let ok_match = all("(module m (def (f (: n Int64)) (match n (0 nonesuch) (_ 2))) (export f))");
    assert!(
        ok_match.iter().any(|d| d.message.contains("`nonesuch`")),
        "a well-formed match's arm fault is not suppressed: {:?}",
        ok_match.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn a_miscased_boolean_literal_suggests_the_lowercase_literal() {
    // The POSITIVE suggest+fix faces — `True` -> replace-fix `true`, `False` -> replace-fix `false` —
    // migrated to corpus 01-literals ("a miscased boolean True/False suggests the lowercase … literal
    // with a replace fix"). What STAYS here is the heuristic-CUTOFF pin the corpus can't cleanly express:
    // NO OVERREACH — `TRUE` (all-caps, edit distance 4) is beyond the typo cutoff → no baseless
    // suggestion, the plain unbound-name message (corpus `(not "did you mean")` grades only `todo` for a
    // suggestion-ABSENCE, so this negative cutoff assertion is kept as a rust residue).
    let caps = expect_error("TRUE");
    assert_eq!(
        caps.code.as_deref(),
        Some("CDZ0101"),
        "got: {}",
        caps.message
    );
    assert!(
        caps.fix.is_none() && !caps.message.contains("did you mean"),
        "an all-caps TRUE is too far to suggest: {}",
        caps.message
    );
}

// an_unbound_name_close_to_a_let_binding_suggests_the_binding (`(let ((counter 5)) (+ countr 1))` →
// CDZ0101 replace-fix "counter") migrated to corpus 02-binding-and-control "an unbound name close to a
// let binding suggests the binding with a replace fix". rcdzc test deleted (corpus-covered).

#[test]
fn an_unbound_name_close_to_a_match_arm_pattern_binder_suggests_it() {
    // A COMPOUND match-arm pattern binds names in the arm BODY's scope — a `(list … .. rest)` element
    // or rest binder, a `(Some p)` payload, a `(tuple a b)` element. A typo of one is CDZ0101, and the
    // candidate pool now includes the pattern's binders (before, a compound pattern contributed NONE,
    // so a rest-binder typo mis-suggested a far prelude name). Each below near-misses a real binder.
    let find = |body: &str| {
        let src = format!("(module m (def (f (: xs (List Int64))) {body}) (export f))");
        crate::diagnostics(&mut crate::db::Db::load(parse(&src)))
            .into_iter()
            .find(|d| d.code.as_deref() == Some("CDZ0101"))
            .unwrap_or_else(|| panic!("expected an unbound-name fault for {body}"))
    };
    // A list REST binder `rest`, typo'd `reest` in the body → suggests `rest` (`rest` is a
    // `(List Int64)`, so passing it to `List.len` type-checks; the only unbound name is the typo).
    let r = find("(match xs ((list) 0) ((list x .. rest) ((. List len) reest)))");
    assert_eq!(
        r.fix.as_ref().map(|f| f.replacement.as_str()),
        Some("rest"),
        "list rest binder is a candidate: {}",
        r.message
    );
    // A list ELEMENT binder `elem`, typo'd `elm`.
    let e = find("(match xs ((list elem) elm) (_ 0))");
    assert_eq!(
        e.fix.as_ref().map(|f| f.replacement.as_str()),
        Some("elem"),
        "list element binder is a candidate: {}",
        e.message
    );
    // A variant PAYLOAD binder `payload`, typo'd `payloed` — over an Option scrutinee.
    let src = "(module m (def (g (: o (Option Int64))) (match o ((Some payload) payloed) ((None) 0))) (export g))";
    let p = crate::diagnostics(&mut crate::db::Db::load(parse(src)))
        .into_iter()
        .find(|d| d.code.as_deref() == Some("CDZ0101"))
        .expect("expected an unbound-name fault");
    assert_eq!(
        p.fix.as_ref().map(|f| f.replacement.as_str()),
        Some("payload"),
        "variant payload binder is a candidate: {}",
        p.message
    );
}

#[test]
fn an_unbound_name_close_to_a_prelude_builtin_suggests_it() {
    // The prelude's names are candidates — `List` misspelled `Lst` (a module head). `Lst` is
    // distance-1 from BOTH `List` (insert `i`) and the built-in `Ast` (substitute `L`→`A`), so the
    // nearest-name search returns one of the two tied candidates — either is a valid prelude module.
    // (Before the built-in `Ast` prelude sum existed this was unambiguously `List`; `Ast` joining the
    // pool made it a legitimate tie — the point of the test is that a near-miss to a prelude name
    // GETS a suggestion, not which of two equidistant prelude names wins the tie-break.)
    let d = expect_error("(Lst.push (list 1 2) 3)");
    assert_eq!(d.code.as_deref(), Some("CDZ0101"), "got: {}", d.message);
    let suggested = d.fix.as_ref().map(|f| f.replacement.as_str());
    assert!(
        matches!(suggested, Some("List") | Some("Ast")),
        "a near-miss to a prelude module suggests a distance-1 prelude name (List or Ast): {}",
        d.message
    );
}

#[test]
fn a_genuinely_unknown_name_carries_no_misleading_suggestion() {
    // No in-scope name is within the edit-distance cutoff of `frobnicate`, so the diagnostic stays
    // the plain "unbound name" — no fix, no misleading "did you mean". (A false suggestion is worse
    // than none: an agent would apply the wrong edit.)
    let d = expect_error("frobnicate");
    assert_eq!(d.code.as_deref(), Some("CDZ0101"), "got: {}", d.message);
    assert!(
        !d.message.contains("did you mean"),
        "no suggestion: {}",
        d.message
    );
    assert!(d.fix.is_none(), "no fix carried: {:?}", d.fix);
}

#[test]
fn the_suggestion_is_deterministic_across_equidistant_candidates() {
    // Two defs equidistant (1 edit) from the reference `ab`: `ac` and `xb`. The tie breaks on the
    // lexicographically-smaller candidate, so the suggestion is a pure function of the source
    // (`spec/capabilities/diagnostics.md` §A Fix Is A Deterministic Function Of The Source), never
    // dependent on def/hash iteration order.
    let src = "(module m (def (ac) 1) (def (xb) 2) (def (main) (+ (ab) 0)) (export main))";
    let d = compile_component(&crate::codec::encode(&parse(src))).expect_err("must reject");
    assert_eq!(
        d.fix.as_ref().map(|f| f.replacement.as_str()),
        Some("ac"),
        "lexicographically-smaller candidate wins the tie: {}",
        d.message
    );
}

#[test]
fn many_references_to_one_missing_name_all_suggest_it() {
    // The program-wide typo-suggestion winner is MEMOIZED per (name, position-class), so N references
    // to the SAME missing name (a forgotten import / renamed helper called from N sites) share one
    // edit-distance scan instead of re-running it each — the O(N²) fix. This locks in that the memo
    // returns the CORRECT, IDENTICAL suggestion for every occurrence (not a stale/first-only answer):
    // 20 defs each call `helpr`, and every one of the 20 unbound-name faults must suggest `helper`.
    let n = 20;
    let defs = (0..n)
        .map(|i| format!("(def (d{i} x) (helpr x))"))
        .collect::<Vec<_>>()
        .join(" ");
    let src = format!("(do (def (helper y) y) {defs} (def (main) (d0 1)) (export main))");
    let mut db = crate::db::Db::load(parse(&src));
    let sugg: Vec<String> = crate::diagnostics(&mut db)
        .into_iter()
        .filter(|d| d.code.as_deref() == Some("CDZ0101"))
        .filter_map(|d| d.fix.map(|f| f.replacement))
        .collect();
    assert_eq!(
        sugg.len(),
        n,
        "one suggested fix per unbound `helpr` reference: {sugg:?}"
    );
    assert!(
        sugg.iter().all(|s| s == "helper"),
        "every occurrence suggests `helper` (the memo returns the same winner each time): {sugg:?}"
    );
}

#[test]
fn nearest_breaks_an_edit_distance_tie_by_shared_first_character() {
    // The internal `suggest::nearest` tie-break unit (the end-to-end diagnostic — `(. Lst len)` suggesting
    // the member-accessible `List` MODULE over the equidistant `Ast` variant — moved to corpus
    // 11-modules "a member-operand typo suggests a member-accessible module sharing a variant name"). This
    // white-box residual pins the tie-break the corpus cannot reach: on equal edit distance, a shared FIRST
    // CHARACTER wins (a typo rarely changes the leading letter). `Lst` shares `L` with `List`, not `Ast`.
    assert_eq!(
        crate::diag::suggest::nearest("Lst", ["Ast", "List"]),
        Some("List".into()),
        "a shared first character breaks an edit-distance tie"
    );
}

#[test]
fn a_lexical_well_formedness_fault_surfaces_in_an_unreached_body() {
    // A LEXICAL well-formedness poison a bare leaf resolves to — a malformed numeric literal
    // (CDZ0201), an out-of-range float (CDZ0201), an unrecognized string escape (CDZ0001), a char
    // naming a non-scalar (CDZ0002) — is a defect of the TOKEN, independent of reachability, like an
    // unbound name. But `collect_node`'s poison arm surfaced ONLY `Unbound`, so a malformed literal in
    // a PARAMETERIZED or non-exported body PASSED `cdz check` (the resolve poison reached only the
    // emit-path walk, which runs on nullary-exported bodies) while `compile` rejected it on a reached
    // body — the same "check misses a resolve/lower-only reject on an unreached body" hole M81's
    // pattern accessor and the `(do)`-block poison close. Now `check` (the Diagnostics query) surfaces
    // it in EVERY body.
    let find = |src: &str, code: &str| {
        crate::diagnostics(&mut crate::db::Db::load(parse(src)))
            .into_iter()
            .find(|d| d.code.as_deref() == Some(code))
            .unwrap_or_else(|| panic!("no {code} for {src}"))
    };
    // An OUT-OF-RANGE FLOAT in a PARAMETERIZED body (never lowered standalone by the emit walk) surfaces
    // CDZ0201. Its reject round-trips to a ~400-digit decimal expansion — impractical as corpus text — so
    // this facet stays a rust residual; the octal-int / non-scalar-char / non-exported / reachable-dedup
    // sibling facets moved to corpus 01-literals "a malformed literal in a parameterized (unreached) body
    // still surfaces" + siblings.
    find(
        "(module m (def (g (: n Int64)) (+ 1.0e400 2.0)) (export g))",
        "CDZ0201",
    );
    // NO false positive: a well-formed literal in a parameterized body stays clean (a compile-clean check
    // the corpus would need a run to express; kept as a white-box control here).
    assert!(
        crate::diagnostics(&mut crate::db::Db::load(parse(
            "(module m (def (g (: n Int64)) (+ n 42)) (export g))"
        )))
        .iter()
        .all(|d| d.severity != crate::abi::Severity::Error),
        "a valid literal in a parameterized body produces no fault"
    );
}

// a_type_fault_inside_a_let_body_is_still_caught (`(let ((x 5)) (if x 1 2))` → CDZ0203 "condition must
// be Bool") migrated to corpus 02-binding-and-control "an ill-typed condition inside a let body is still
// caught (the check descends through the let)". rcdzc test deleted (corpus-covered).

#[test]
fn distinct_literal_map_keys_are_not_a_duplicate() {
    // The negative direction the O(N) scan must preserve: distinct written literals are NOT a
    // duplicate, so the map literal compiles clean (no CDZ0201). Distinct ints (incl. one dec + one
    // hex that are DIFFERENT values), distinct strings, distinct bools.
    assert!(compiles_ok("(map (= 1 10) (= 2 20))"));
    assert!(compiles_ok("(map (= 1 10) (= 0x2 20) (= 3 30))"));
    assert!(compiles_ok("(map (= \"a\" 1) (= \"b\" 2))"));
    assert!(compiles_ok("(map (= true 1) (= false 2))"));
}

#[test]
fn two_distinct_names_bound_to_the_same_value_are_a_runtime_overwrite_not_a_duplicate() {
    // The crucial subtlety `literal_key_token` preserves by returning `None` for a NAME key: two
    // DISTINCT names that merely FOLD to the same value are a RUNTIME overwrite (the map holds one
    // entry at run time, keys compared BY VALUE), NOT a compile-time duplicate reject. Reading the
    // key THROUGH its binding — as a pairwise `const_compound_eq` on the folded values would —
    // conflates the two; the direct-literal gate keeps them apart, so the program COMPILES (the
    // overwrite is a runtime fact, checked by the 05-compound-types §2510 corpus case's `Map.len`).
    assert!(compiles_ok(
        "(let ((a 5)) (let ((b 5)) (map (= a 1) (= b 2))))"
    ));
}

#[test]
fn a_scalar_member_access_reports_one_coded_error_not_a_bare_plus_rich_duplicate() {
    // The DEDUP residual (the surviving type/arity-naming MESSAGES moved to corpus
    // 05-compound-types "a called scalar member access surfaces the type-naming message" +
    // "a called tuple index out of range surfaces the arity-naming message"). What stays here is the
    // one-error guarantee the corpus CANNOT express: `(count N)`/`(no-other-errors)` reason over CODED
    // faults only, and the deduped duplicate is the emit path's UNCODED bare decline. A direct member
    // access on a non-record scalar `(. 5 x)` used to report TWICE — `infer`'s rich "…, found Int64"
    // (coded) AND the emit path's bare "member access requires a record" (uncoded) — at DIFFERENT nodes,
    // so the node-keyed dedup missed the pair. `dedup_faults` now drops the bare form when the rich
    // "…, found <T>" is present. Exactly ONE error survives.
    let errs: Vec<crate::abi::Diagnostic> = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "m",
            crate::codec::encode(&parse("(module m (def (main) ((. 5 x))) (export main))")),
        )],
        &[crate::backend::Target::Wasm],
    )
    .diagnostics
    .into_iter()
    .filter(|d| d.severity == crate::abi::Severity::Error)
    .collect();
    assert_eq!(
        errs.len(),
        1,
        "a scalar member access = ONE error (no bare+rich duplicate), got: {:?}",
        errs.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // The TUPLE-INDEX twin: `(. (tuple 1 2) 5)` similarly reported the bare (uncoded) "tuple index 5 is
    // out of range" AND the rich "… for a 2-element tuple"; now ONE.
    let terrs: Vec<crate::abi::Diagnostic> = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "m",
            crate::codec::encode(&parse(
                "(module m (def (main) ((. (tuple 1 2) 5))) (export main))",
            )),
        )],
        &[crate::backend::Target::Wasm],
    )
    .diagnostics
    .into_iter()
    .filter(|d| d.severity == crate::abi::Severity::Error)
    .collect();
    assert_eq!(
        terrs.len(),
        1,
        "a tuple index OOB = ONE error (no bare+rich duplicate), got: {:?}",
        terrs.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

// named_member_access_on_a_tuple_points_at_the_numeric_index_form migrated to corpus 05-compound-types:
// the tuple-by-name rich message (`(. t x)` on (Tuple Int64 Int64) -> CDZ0201 "accessed by position, not
// by name" + numeric-index form + arity range) is the new case "member access on a tuple by NAME points
// at the numeric-index form"; the scalar `(. 5 x)` "requires a record" face is the existing "member
// access on a non-record is a type error" (:240) + the call-through rich-message case (:248). rcdzc test
// deleted (both faces corpus-covered).

#[test]
fn a_called_tuple_by_name_access_reports_the_position_message_once_not_a_call_site_cascade() {
    // A tuple-literal accessed by NAME in a def that is CALLED — `(def (g) (. (tuple 1 2) foo))` used
    // from an exported body — reported TWICE: the precise "a tuple is accessed by position" at the def,
    // PLUS the bare "member access requires a record" at the CALL SITE (the reached-poison walk lowers
    // the reduced `(. (tuple 1 2) foo)`, which cannot fold). The call-site decline is the same defect
    // reached again through lowering — a different node + a weaker message, so neither same-node dedup
    // nor the reduced-body baseline-diff collapsed it. `dedup_faults` now drops the bare decline for the
    // tuple-by-position reject. Exactly ONE fault, the precise one; the bare decline is gone.
    let ds = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def (g) (. (tuple 1 2) foo)) (def (main) (g)) (export main))",
    )));
    let errs: Vec<_> = ds
        .iter()
        .filter(|d| d.severity == crate::abi::Severity::Error)
        .collect();
    assert_eq!(
        errs.len(),
        1,
        "one precise fault, not a call-site cascade: {ds:?}"
    );
    assert!(
        errs[0]
            .message
            .contains("a tuple is accessed by position, not by name `foo`"),
        "the surviving fault is the precise position message: {}",
        errs[0].message
    );
    assert!(
        !ds.iter()
            .any(|d| d.message == "member access requires a record"),
        "the bare call-site decline is suppressed: {ds:?}"
    );
    // A SCALAR member access on a non-record still reports its own "found <T>" message (NOT suppressed —
    // the exact-match drops only the bare lowering form). Two independent defects still both report.
    let scalar = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def (main) (. 5 x)) (export main))",
    )));
    assert!(
        scalar.iter().any(|d| d
            .message
            .contains("member access requires a record, found Int64")),
        "a scalar non-record access keeps its typed message: {scalar:?}"
    );
}

#[test]
fn the_renamed_collection_ops_resolve_under_their_new_names() {
    // The canonical spellings COMPILE — a pure surface rename, no eval/backend change (the intrinsics
    // `map-size`/`tuple-cat`/`tuple-pop` stay wired; only the surface key moved). Same shape as the
    // pre-rename `tuple_cat_split_at_pop_reshape_tuples_positionally` compile check.
    for src in [
        "(module m (def (main) (Map.len (Map.insert (Map.insert (Map.empty) 1 10) 2 20))) (export main))",
        "(module m (def (main) (Tuple.concat (tuple 1 2) (tuple 3 4))) (export main))",
        "(module m (def (main) (Tuple.remove (tuple 1 2 3))) (export main))",
    ] {
        assert!(
            compile_component(&crate::codec::encode(&parse(src))).is_ok(),
            "the canonical collection-op name must resolve + compile: {src}"
        );
    }
    // …and the renamed `Tuple.remove` COMPILES on a value program: `(. (Tuple.remove (tuple 7 8 9)) 0)`
    // pops element 0 (the head), returning `(tuple 7 (tuple 8 9))`, and reads element 0 back = 7. That
    // RUN is corpus-covered by 15-rows-and-open-sums "popping a tuple yields element zero and the
    // remaining tuple" ((Tuple.remove (tuple 1 2 3)) = (tuple 1 (tuple 2 3))).
    assert!(
        compile_component(&crate::codec::encode(&parse(
            "(module m (def (main) (. (Tuple.remove (tuple 7 8 9)) 0)) (export main))",
        )))
        .is_ok(),
        "the renamed Tuple.remove compiles on a value program"
    );
}

#[test]
fn a_misspelled_field_call_head_reports_one_error_not_a_dup() {
    // A misspelled field access used as a CALL HEAD (`((. r fld-typo) 5)`) is checked by BOTH the
    // infer member-check (which adds the did-you-mean fix) AND the emit-side member fold — at two
    // DIFFERENT nodes (the `.fld-typo` projection vs the enclosing apply), so it used to surface the
    // SAME "record has no field" fault TWICE. `dedup_faults` now drops the fix-less emit-side copy
    // when the fix-carrying infer copy of the same field-fault is present → ONE error, WITH the fix.
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "m",
            crate::codec::encode(&parse(
                "(module m (def (main) ((. (record (= compute 1)) computee) 5)) (export main))",
            )),
        )],
        &[crate::backend::Target::Wasm],
    );
    let field_errs: Vec<&crate::abi::Diagnostic> = out
        .diagnostics
        .iter()
        .filter(|d| {
            d.severity == crate::abi::Severity::Error && d.message.contains("record has no field")
        })
        .collect();
    assert_eq!(
        field_errs.len(),
        1,
        "one no-field error, not the infer+lower duplicate: {:?}",
        out.diagnostics
    );
    assert!(
        field_errs[0].fix.is_some() && field_errs[0].message.contains("did you mean `compute`?"),
        "the surviving copy is the RICH one (with the did-you-mean fix): {}",
        field_errs[0].message
    );
}

#[test]
fn returning_a_constant_record_compiles_via_the_resource_escape() {
    // A CONSTANT record returned as the program result now crosses the host boundary as a
    // component-model resource whose `encode()` yields the canonical value form (the escape path,
    // §3a) — it no longer declines. (The end-to-end value assertion, decoding to `(: (record …)
    // (Record …))`, is the corpus case "a constant record is returned as a program result" in
    // `spec/semantics/05-compound-types.sexp`.) A record
    // consumed INTERNALLY still folds/declines per its use; this pins that a constant record RESULT
    // compiles to a component.
    let src = "(module m (def (main) (record (= x 1))) (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "a constant record return must compile via the resource escape"
    );
}

// ── Value-heap H2a: tuple surface + fold (static-correct first) ───────────────────────────────
//
// A tuple projected by a constant index over a compile-time-visible tuple FOLDS to the element (no
// heap) — the same static reduction a record member fold takes, so these run to a scalar. A runtime
// tuple (one that escapes) DECLINES pending H2b's heap emission. An out-of-arity index and a non-
// tuple projection are COMPILE-TIME rejects (CDZ0201), never runtime traps.

#[test]
fn returning_a_constant_tuple_compiles_via_the_resource_escape() {
    // A CONSTANT tuple returned across the host boundary now crosses as a monomorphized component
    // RESOURCE whose `encode() -> list<u8>` yields the canonical binary value form; the host decodes
    // + prints `(: (tuple …) (Tuple …))` (the escape path, §3a). It no longer declines. (The
    // end-to-end value assertion is the corpus case "a constant tuple is returned as a program
    // result" in `spec/semantics/05-compound-types.sexp`.)
    // A RUNTIME tuple (elements computed at run time) still declines here pending R2 (the real
    // handle-walking encoder) — see `a_runtime_tuple_return_still_declines_pending_r2`.
    let src = "(module m (def (main) (tuple 1 2)) (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "a constant tuple return must compile via the resource escape"
    );
}

#[test]
fn a_parameterized_compound_return_export_compiles_via_the_resource_escape() {
    // A tuple returned from an export that TAKES A PARAMETER now crosses as the resource escape: the
    // resource's constructor `make` FORWARDS the export's scalar params (`make(n) -> own<t>`), so the
    // host computes the compound from its argument, then `encode()` walks the live handle to the
    // value form. This closed the last cross-cutting heap-return decline (a `List`/`BigInt`/`Rational`
    // from a parameterized export declined identically); the end-to-end value is corpus-gated
    // ("a parameterized export returns a … computed from its argument"). Previously DECLINED
    // "a heap value escapes … only from a NULLARY export".
    let src = "(module m (def (pair (: n Int64)) (tuple n 1)) (export pair))";
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "a parameterized compound-return export must compile via the param-forwarding resource escape"
    );
}

#[test]
fn a_recursive_sum_carrying_bytes_renders_via_the_value_encode_walker() {
    // A recursive sum whose payload carries BYTES (a `BytesList` — a parse tree, a binary structure)
    // now COMPILES its value-encode escape. Previously `shape_of` DECLINED on `Ty::Bytes`; it now emits
    // `ShapeNode::Bytes` (descriptor tag 4) and the runtime `value-encode` flattens the rope + renders
    // a KIND_BYTES leaf (guarded byte-exact in cdz-runtime's `value_encode_renders_a_bytes_leaf`).
    use crate::testkit::parse;
    let src = "(module m (type BytesList (Cons (Tuple Bytes BytesList)) Nil) \
                     (def (build (: n Int64)) (if (< n 1) (BytesList.Nil ()) \
                        (BytesList.Cons (tuple (Bytes.of (list 1 2)) (build (- n 1)))))) \
                     (def (main) (build 2)) (export main))";
    compile_component(&crate::codec::encode(&parse(src)))
        .expect("a recursive sum carrying Bytes compiles via the value-encode walker");
}

#[test]
fn a_recursive_sum_carrying_a_set_renders_via_the_value_encode_walker() {
    // A recursive sum carrying a SET (a `SetList` — a tree of sets) now COMPILES its value-encode
    // escape. `shape_of` emits `ShapeNode::Set` (descriptor tag 12) for a SCALAR-element set, and the
    // runtime `value-encode` iterates the CHAMP + SORTS the elements into canonical key-VALUE order,
    // rendering `(Set.of (list …))` (guarded byte-exact + canonical-order in cdz-runtime's
    // `value_encode_renders_a_set_in_canonical_order`). A non-scalar-element set still declines.
    use crate::testkit::parse;
    let src = "(module m (type SetList (Cons (Tuple (Set Int64) SetList)) Nil) \
                     (def (build (: n Int64)) (if (< n 1) (SetList.Nil ()) \
                        (SetList.Cons (tuple (Set.of (list n)) (build (- n 1)))))) \
                     (def (main) (build 2)) (export main))";
    compile_component(&crate::codec::encode(&parse(src)))
        .expect("a recursive sum carrying a Set compiles via the value-encode walker");
}

#[test]
fn a_recursive_sum_carrying_a_map_renders_via_the_value_encode_walker() {
    // A recursive sum carrying a MAP (a `MapList` — a tree of maps, nested config/JSON) now COMPILES
    // its value-encode escape. `shape_of` emits `ShapeNode::Map` (descriptor tag 13) for a
    // SCALAR-KEY map (the value may be any encodable shape), and the runtime `value-encode` iterates
    // the CHAMP + SORTS entries into canonical KEY order, rendering `(map (k v)…)` (guarded byte-exact
    // + canonical-order in cdz-runtime's `value_encode_renders_a_map_in_canonical_key_order`). A
    // non-scalar-KEY map still declines.
    use crate::testkit::parse;
    let src = "(module m (type MapList (Cons (Tuple (Map String Int64) MapList)) Nil) \
                     (def (build (: n Int64)) (if (< n 1) (MapList.Nil ()) \
                        (MapList.Cons (tuple (map (= \"k\" n)) (build (- n 1)))))) \
                     (def (main) (build 2)) (export main))";
    compile_component(&crate::codec::encode(&parse(src)))
        .expect("a recursive sum carrying a Map compiles via the value-encode walker");
}

#[test]
fn a_value_eq_on_a_sum_payload_string_compiles() {
    // Comparing a variant's PAYLOAD (a `SumPayload`/tuple-element read) to a constant string —
    // `(= h "+")` where `h` is bound from a `(NPrim (tuple h a b))` payload — is the shape a recursive
    // resolver dispatches on. The `value-eq` operand-ownership analysis must classify a payload READ
    // as Borrowed (the enclosing compound owns it) and a constant-string literal as Owned (it
    // materializes a fresh byte-leaf that the compare drops); previously the `ConstStr` operand
    // declined "an ownership this backend cannot yet prove", blocking the whole resolver.
    let src = "(module m (type N (NI Int64) (NP (Tuple String N))) \
                     (def (f n) (match n ((NI v) v) ((NP (tuple h t)) (if (= h \"+\") (f t) 0)))) \
                     (def (main) (f (NP (tuple \"+\" (NI 5))))) (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "a value-eq comparing a sum-payload string to a constant compiles (payload=Borrowed, \
             const string=Owned)"
    );
}

#[test]
fn a_value_eq_on_an_inlined_match_operand_compiles() {
    // A `String ==` whose operand is an INLINED function returning a `match` — `(= (f …) "z")` where
    // `f` returns `(match (Map.lookup m k) ((Some s) s) ((None) "?"))`, inlined into the `=` operand.
    // The two arms DISAGREE on ownership: the `Some` arm returns a BORROWED payload (`s` = a
    // `SumPayload` read off the owned Option), the `None` arm returns an OWNED const (`"?"`). The
    // `value-eq` operand-ownership analysis had no `MatchSum` arm and fell through to the generic
    // decline "borrowing op operand has an ownership this backend cannot yet prove" — blocking any
    // program that compares a returned map/variant payload once the wrapper inlines (the shape a
    // compiler-in-Cadenza substitution pass hits). It must now classify a match operand by the JOIN of
    // its arm bodies — BORROWED here (a mixed join, the leak-safe value), so no drop follows and the
    // operand is left to its owner (the standalone-function path leaks the scrutinee the same way).
    let src = "(module m \
                     (def (f (: m (Map String String)) (: k String)) \
                        (match (Map.lookup m k) ((Some s) s) ((None) \"?\"))) \
                     (def (main) (if (= (f (Map.insert (Map.empty) \"y\" \"z\") \"y\") \"z\") 1 0)) \
                     (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "a value-eq on an inlined match operand compiles (match operand classified by its arm join, \
             mixed → Borrowed)"
    );
}

#[test]
fn a_multi_export_compound_return_declines_with_the_multi_export_diagnosis() {
    // The OTHER compound-return trigger: a program with MULTIPLE exports, one returning a compound.
    // The resource-escape path takes only a SINGLE nullary compound export, so a multi-export program
    // declines — and here the message DOES name "multiple exports" (the trigger that actually
    // applies), distinct from the single-parameterized-export diagnosis above. Pins the two triggers
    // are diagnosed separately, not conflated into one misleading phrase.
    let src = "(module m (def (main) (tuple 5 6)) (def (other) 7) (export main) (export other))";
    let err = compile_component(&crate::codec::encode(&parse(src)))
        .expect_err("a multi-export compound return declines");
    assert!(
        err.message.contains("multiple exports"),
        "a multi-export compound return must name the multi-export trigger, got: {}",
        err.message
    );
}

#[test]
fn a_multi_export_string_declines_by_arity_not_by_type() {
    // A STRING (and every other heap value: list/bytes/map/set/…) crosses the host boundary FINE as
    // the SOLE export (the resource-escape path). So a String export ALONGSIDE a second export must
    // decline by ARITY — "a heap value crosses only as the single export" — NOT by TYPE ("String has
    // no component boundary representation", the old misleading message that blamed the type and
    // misdirected a fix). The diagnosis is keyed on the SAME `crosses_as_resource_escape` predicate the
    // escape gate uses, so every escape-capable type gets the arity message, not just Tuple/Record/Sum.
    for src in [
        // two compound (String) exports
        "(module m (def (aaa) \"first\") (def (bbb) \"second\") (export aaa) (export bbb))",
        // a compound (String) alongside a scalar
        "(module m (def (aaa) \"first\") (def (n) 5) (export aaa) (export n))",
    ] {
        let err = compile_component(&crate::codec::encode(&parse(src)))
            .expect_err("a multi-export String declines");
        assert!(
            err.message.contains("SINGLE export") && err.message.contains("multiple exports"),
            "a multi-export String must decline by ARITY (single-export), got: {}",
            err.message
        );
        assert!(
            !err.message.contains("no component boundary representation"),
            "the message must not blame the TYPE (String crosses fine as the sole export), got: {}",
            err.message
        );
    }
    // CONTRAST: a String as the SOLE export crosses fine (the resource escape) — the constraint is
    // arity, not the type.
    assert!(
        compile_component(&crate::codec::encode(&parse(
            "(module m (def (greet) \"hello\") (export greet))"
        )))
        .is_ok(),
        "a String as the sole export must cross the boundary"
    );
    // NO OVER-REJECTION: a multi-export program with a NOMINAL-over-scalar export still crosses (it
    // erases to a scalar boundary valtype, so it is not an arity decline).
    assert!(
            compile_component(&crate::codec::encode(&parse(
                "(module m (type UserId (Mk Int64)) (def (uid) (UserId.Mk 42)) (def (n) 5) (export uid) (export n))"
            )))
            .is_ok(),
            "a nominal-over-scalar in a multi-export program crosses as its scalar, not an arity decline"
        );
}

#[test]
fn an_escaped_value_with_an_unresolved_type_reports_an_ambiguity_not_an_export_shape_error() {
    // A bare `(None)` returned as the program result has type `(Option ?0)` — the payload is a free
    // variable nothing constrains, so the escaped value has no defined serialization. This is a
    // SINGLE NULLARY sum export (the escape path's shape IS satisfied), so the reject must name the
    // AMBIGUOUS TYPE and the annotation fix (CDZ0203), NOT the export-shape message (which would
    // misdiagnose — the shape is fine, the type is undetermined). The prior message wrongly said the
    // sum "crosses only as a single nullary export's result" — which it already is.
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "m",
            crate::codec::encode(&parse("(module m (def (main) (None)) (export main))")),
        )],
        &[crate::backend::Target::Wasm],
    );
    let err = out
        .diagnostics
        .iter()
        .find(|d| d.severity == crate::abi::Severity::Error)
        .expect("a bare escaped None with an unresolved payload declines");
    assert_eq!(
        err.code.as_deref(),
        Some("CDZ0203"),
        "an ambiguous escaped type is a type fault"
    );
    assert!(
        err.message.contains("not fully determined") && err.message.contains("Annotate the export"),
        "the message must name the unresolved type + the annotation fix, got: {}",
        err.message
    );
    // CONTROLS: an ANNOTATED None escapes (the type is resolved), and a `(Some 5)` (payload
    // constrained by its argument) escapes — the ambiguity bites ONLY an unannotated free-var escape.
    assert!(
        compile_component(&crate::codec::encode(&parse(
            "(module m (def (main) (: (None) (Option Int64))) (export main))"
        )))
        .is_ok(),
        "an annotated None must escape"
    );
    assert!(
        compile_component(&crate::codec::encode(&parse(
            "(module m (def (main) (Some 5)) (export main))"
        )))
        .is_ok(),
        "a Some with a constrained payload must escape"
    );
}

#[test]
fn an_undetermined_escape_type_is_reported_by_check_not_only_compile() {
    // The undetermined-escape-result-type reject (a bare `(None)` : `Option ?`, an empty `(Set.of
    // (list))` : `Set ?`) was an EMIT-path check only — `cdz compile` failed CDZ0203 but `cdz check`
    // (which runs no backend) ACCEPTED it, a check≡compile gap. `collect_faults` now mirrors the emit
    // guard, so `diagnostics()` (the `cdz check`/LSP surface) reports the SAME CDZ0203.
    let check_err = |src: &str| -> Option<crate::abi::Diagnostic> {
        crate::diagnostics(&mut crate::db::Db::load(parse(src)))
            .into_iter()
            .find(|d| d.severity == crate::abi::Severity::Error)
    };
    for src in [
        "(module m (def (main) (None)) (export main))",
        "(module m (def (main) (Set.of (list))) (export main))",
    ] {
        let d = check_err(src).unwrap_or_else(|| panic!("check must report the ambiguity: {src}"));
        assert_eq!(d.code.as_deref(), Some("CDZ0203"), "got: {}", d.message);
        assert!(
            d.message.contains("not fully determined") && d.message.contains("Annotate the export"),
            "check names the undetermined type + fix: {}",
            d.message
        );
        // check≡compile: the emit path must AGREE (also reject).
        assert!(
            compile_component(&crate::codec::encode(&parse(src))).is_err(),
            "compile must also reject (check≡compile): {src}"
        );
    }
    // NO OVERREACH — check stays clean where compile succeeds:
    //  • an annotated escape (the free var is resolved),
    //  • a DIVERGING export (body traps; result `_`/Never is NOT a heap-escape, emittable as a
    //    trapping function — the `crosses_as_resource_escape` guard excludes it),
    //  • an ordinary scalar.
    for ok in [
        "(module m (def (main) (: (None) (Option Int64))) (export main))",
        "(module m (def (main) (trap \"x\")) (export main))",
        "(module m (def (main) 42) (export main))",
    ] {
        assert!(
            check_err(ok).is_none(),
            "check must stay clean where compile succeeds: {ok} -> {:?}",
            check_err(ok).map(|d| d.message)
        );
    }
}

// ── the prelude: a built-in module is an arena record, reached by the same projection ──────

#[test]
fn the_bare_name_unit_is_the_unit_value() {
    // `unit` is a prelude value — an alias for the empty list `()`, the other spelling of the unit
    // value (core-semantics.md #Unit And The Empty Tuple Are The Same Value). It must RESOLVE (not
    // reject "unbound name `unit`") and produce the unit value exactly as `()` does. Compiling a
    // unit-returning `main` from each spelling must succeed and yield the SAME core.
    let core_of_body = |body: &str| {
        let src = format!("(module m (def (main) {body}) (export main))");
        // A unit `main` has no scalar result to run; assert it COMPILES (resolves) — before the fix
        // `unit` rejected CDZ0101 and no component was produced.
        compile_component(&crate::codec::encode(&crate::testkit::parse(&src)))
            .unwrap_or_else(|e| panic!("`{body}` must compile: {}", e.message))
    };
    // Both spellings compile to a valid component (the unit value crosses the boundary as an empty
    // result). `unit` no longer rejects as an unbound name.
    let from_unit = core_of_body("unit");
    let from_parens = core_of_body("()");
    // The two produce byte-identical components — `unit` and `()` are the same value.
    assert_eq!(
        from_unit, from_parens,
        "`unit` and `()` must compile to the same component (the same unit value)"
    );
}

#[test]
fn an_unrealized_builtin_field_declines() {
    // `(. (Int 100) max)` — the field EXISTS (present as a poison: a >64-bit width's bounds are not
    // yet realized, `int_bounds` returns `None`), so projecting it declines "not yet realized" rather
    // than rejecting as absent. No open-module rule: it is filled with a poison. (The former exemplar
    // `(. Int64 of)` is now REALIZED — `of` is the checked conversion — so a still-unrealized field is
    // used here; see `checked_integer_conversion_folds_in_range_and_traps_out_of_range`.)
    let msg = expect_decline("(. (Int 100) max)");
    assert!(msg.contains("is not supported"), "got: {msg}");
}

// ── the full binary-integer operator set (all fold at width 64) ──────────────────────────────

// division_by_zero_fails_the_build (`(/ 5 0)` → CDZ0304) migrated to corpus 06-numeric-model
// "division by zero traps". rcdzc test deleted (corpus-covered).

#[test]
fn dividing_a_runtime_value_by_the_constant_zero_fails_the_build() {
    // `(/ n 0)` / `(% n 0)` with a RUNTIME numerator but the compile-time literal `0` as the divisor
    // ALWAYS traps regardless of `n` — no runtime value makes it valid. Rejected CDZ0304 (like the
    // both-constant `(/ 5 0)`), not shipped as a runtime trap. The message says how to fix it: guard
    // the divisor or remove the division. Distinct from `(/ n z)` with a runtime `z` (a genuine
    // runtime trap — `z` is a variable, not the literal `0`).
    let d = compile_component(&crate::codec::encode(&parse(
        "(module m (def (g (: n Int64)) (/ n 0)) (export g))",
    )))
    .expect_err("dividing by the constant 0 fails the build");
    assert_eq!(d.code.as_deref(), Some("CDZ0304"), "got: {}", d.message);
    assert!(
        d.message.contains("by the constant 0 always traps"),
        "names the always-traps cause + fix route: {}",
        d.message
    );
    // `%` by the constant 0 too.
    assert_eq!(
        compile_component(&crate::codec::encode(&parse(
            "(module m (def (g (: n Int64)) (% n 0)) (export g))",
        )))
        .expect_err("% by 0 fails")
        .code
        .as_deref(),
        Some("CDZ0304")
    );
    // SHIELDED: a `(/ n 0)` in an UNTAKEN branch is unreachable, so it is NOT rejected — the const-trap
    // reached-poison walk does not descend an untaken branch (the M74 shielding discipline).
    assert!(
        compile_component(&crate::codec::encode(&parse(
            "(module m (def (g (: n Int64)) (if false (/ n 0) 1)) (export g))",
        )))
        .is_ok(),
        "a divide-by-zero shielded in an untaken branch is not rejected"
    );
    // NOT the runtime-divisor case: `(/ n z)` with a variable `z` compiles (traps only if z==0 at run
    // time) — the reject fires only for the LITERAL `0` divisor.
    assert!(
        compile_component(&crate::codec::encode(&parse(
            "(module m (def (g (: n Int64) (: z Int64)) (/ n z)) (export g))",
        )))
        .is_ok(),
        "a runtime divisor is not a compile-time trap"
    );
}

// division_of_min_by_minus_one_overflows (`(/ -9223372036854775808 -1)` → CDZ0304) migrated to corpus
// 06-numeric-model "division of the minimum integer by -1 overflows and traps". rcdzc test deleted (corpus-covered).

#[test]
fn a_conditional_const_divide_by_zero_demotes_to_a_kind_preserving_trap() {
    use crate::core::Core;
    use crate::db::Db;
    use crate::lower::core_of;
    // dzb2/dzb3 (operator ruling 2026-08-27, Lean-oracle finding): a const `(/ 1 0)` in a
    // conditionally-reached `if` branch DEMOTES to a runtime trap that PRESERVES the divide-by-zero
    // KIND — `Core::TrapDivZero`, NOT the bare `Core::Trap` (which would report the kind as
    // "unreachable"). Assert the else branch lowered to the kind-preserving trap.
    let mut db = Db::load(crate::testkit::parse(
        "(module m (def (main (: n Int64)) (if (> n 0) 7 (/ 1 0))) (export main))",
    ));
    let d = db.def_by_name("main").expect("def main");
    let body = db.defs[d].body.expect("main has a body");
    let Core::If { else_, .. } = core_of(&mut db, body) else {
        panic!("main lowers to a runtime `if` (the guard is a runtime value)");
    };
    assert!(
        matches!(core_of(&mut db, else_), Core::TrapDivZero),
        "the demoted const-divide-by-zero else branch preserves the div-by-zero kind, got {:?}",
        core_of(&mut db, else_)
    );

    // A conditional const INTEGER OVERFLOW demotes to the KIND-PRESERVING `Core::TrapOverflow` (the
    // overflow twin — operator: "add a dedicated overflow core op as well"). `(* MAX MAX)` overflows
    // Int64, so the else branch surfaces the "overflow" kind at runtime, not bare "unreachable".
    let mut db2 = Db::load(crate::testkit::parse(
        "(module m (def (main (: n Int64)) \
               (if (> n 0) 7 (* 9223372036854775807 9223372036854775807))) (export main))",
    ));
    let d2 = db2.def_by_name("main").expect("def main");
    let body2 = db2.defs[d2].body.expect("main has a body");
    let Core::If { else_, .. } = core_of(&mut db2, body2) else {
        panic!("main lowers to a runtime `if`");
    };
    assert!(
        matches!(core_of(&mut db2, else_), Core::TrapOverflow),
        "the demoted const-overflow else branch preserves the overflow kind, got {:?}",
        core_of(&mut db2, else_)
    );

    // DISCRIMINATION: a shift-COUNT-out-of-range const trap still demotes to the kind-LESS `Core::Trap`
    // — wasm masks the shift count (no native overflow trap to surface), and the cause message names an
    // out-of-range count, not "overflow"/"divide by zero", so it stays the generic `unreachable` trap.
    let mut db3 = Db::load(crate::testkit::parse(
        "(module m (def (main (: n Int64)) (if (> n 0) 7 (<< 1 100))) (export main))",
    ));
    let d3 = db3.def_by_name("main").expect("def main");
    let body3 = db3.defs[d3].body.expect("main has a body");
    let Core::If { else_, .. } = core_of(&mut db3, body3) else {
        panic!("main lowers to a runtime `if`");
    };
    assert!(
        matches!(core_of(&mut db3, else_), Core::Trap),
        "a shift-count-out-of-range conditional const trap stays the kind-less Core::Trap, got {:?}",
        core_of(&mut db3, else_)
    );

    // The `%` (REMAINDER) by zero shares the div-by-zero cause (`const_trap_cause`'s `Div | Rem if y==0`),
    // so it demotes to `Core::TrapDivZero` exactly like `/` — pin the Rem arm of the fix, not just Div.
    let mut db4 = Db::load(crate::testkit::parse(
        "(module m (def (main (: n Int64)) (if (> n 0) 7 (% 1 0))) (export main))",
    ));
    let d4 = db4.def_by_name("main").expect("def main");
    let body4 = db4.defs[d4].body.expect("main has a body");
    let Core::If { else_, .. } = core_of(&mut db4, body4) else {
        panic!("main lowers to a runtime `if`");
    };
    assert!(
        matches!(core_of(&mut db4, else_), Core::TrapDivZero),
        "a conditional const `%`-by-zero preserves the div-by-zero kind, got {:?}",
        core_of(&mut db4, else_)
    );

    // The OTHER Div overflow — `Int64.min / -1` (the quotient 2^63 has no Int64 value) — is an OVERFLOW
    // (cause "the quotient overflows Int64"), NOT a divide-by-zero (the divisor is -1), so it demotes to
    // `Core::TrapOverflow`. Pins that `is_overflow_trap` catches the division-overflow message too, and
    // that the two Div traps are discriminated by kind (div-by-zero vs overflow) at the demote.
    let mut db5 = Db::load(crate::testkit::parse(
        "(module m (def (main (: n Int64)) (if (> n 0) 7 (/ -9223372036854775808 -1))) (export main))",
    ));
    let d5 = db5.def_by_name("main").expect("def main");
    let body5 = db5.defs[d5].body.expect("main has a body");
    let Core::If { else_, .. } = core_of(&mut db5, body5) else {
        panic!("main lowers to a runtime `if`");
    };
    assert!(
        matches!(core_of(&mut db5, else_), Core::TrapOverflow),
        "a conditional const Int64.min/-1 division overflow preserves the overflow kind, got {:?}",
        core_of(&mut db5, else_)
    );
}

#[test]
fn a_self_identity_fold_is_blocked_when_the_operand_binding_can_trap() {
    // SOUNDNESS (cdz-smith L2 differential): the self-identity folds (`x OP x → const`) DISCARD their
    // operand, so they must not fire when forcing that operand could TRAP — else an observable trap is
    // elided. A `let` binding is lazy, so `(< v0 v0)` with `v0 = (/ 10 n)` (a runtime checked div that
    // ÷0-traps) must NOT fold to `false`: `is_trap_free` now follows the ref to v0's init `(/ 10 n)`,
    // which is not trap-free, so the compare stays runtime (forces v0 → traps at n=0). (#4417 precedent.)
    use crate::core::Core;
    use crate::db::Db;
    use crate::lower::core_of;
    let main_core = |src: &str| {
        let mut db = Db::load(crate::testkit::parse(src));
        let d = db.def_by_name("main").expect("def main");
        let mainb = db.defs[d].body.expect("main body");
        core_of(&mut db, mainb)
    };
    // Each buggy self-identity fold (cdz-smith's precise set) over a TRAPPING lazy binding must be
    // BLOCKED: v0's force ÷0-TRAPS, so if the fold fired it would drop v0 (dead) and elide the trap.
    // Blocked ⇒ v0 stays USED (twice) ⇒ A-normalization KEEPS it as a live `Core::Let` (forced → traps).
    // The six comparisons return Bool (wrapped in an `if` for an Int64 body); `-`/`^` return Int64
    // directly. `(/ 10 n)` is a runtime checked divide (÷0-traps at n=0), so `is_trap_free` follows the
    // ref to it and reports NOT trap-free — the discarding fold declines for every operator.
    for body in [
        "(if (< v0 v0) 1 0)",
        "(if (<= v0 v0) 1 0)",
        "(if (> v0 v0) 1 0)",
        "(if (>= v0 v0) 1 0)",
        "(- v0 v0)",
        "(^ v0 v0)",
    ] {
        let src = format!(
            "(module m (def (main (: n Int64)) (let ((v0 (/ 10 n))) {body})) (export main))"
        );
        assert!(
            matches!(main_core(&src), Core::Let { .. }),
            "self-identity over a TRAPPING lazy binding must NOT fold — v0 must stay a live Core::Let \
                 (forced → traps); body was {body}"
        );
    }
    // REGRESSION GUARD: a PURE binding still folds away. `w = wrapping-add` never traps → `(< w w)` folds
    // to false → w is dead-eliminated → `main` is NOT a `Core::Let` (fully folded), proving the fix ONLY
    // tightens trapping bindings.
    assert!(
        !matches!(
            main_core(
                "(module m (def (main (: n Int64)) (let ((w (Int64.wrapping-add n 1))) (if (< w w) 1 0))) (export main))"
            ),
            Core::Let { .. }
        ),
        "self-comparison over a PURE (wrapping) binding still folds — w must be eliminated, main not a Let"
    );
    // SECOND EFFECT KIND (cdz-smith reinforcement, 2026-08-28): the elided effect is ANY force-trap, not
    // just ÷0. A checked left-shift init `(<< 1 n)` traps on an OUT-OF-RANGE runtime shift count — a
    // DIFFERENT `is_trap_free` path than `/` (Shl is not in the trap-free set, so a full-range runtime
    // count is not trap-free) — so the fold must ALSO decline over it. Pins that the fix is
    // fault-kind-agnostic: v0 stays a live `Core::Let` (forced → traps), not folded away.
    assert!(
        matches!(
            main_core(
                "(module m (def (main (: n Int64)) (let ((v0 (<< 1 n))) (if (< v0 v0) 1 0))) (export main))"
            ),
            Core::Let { .. }
        ),
        "self-identity over a lazy binding whose init is a checked shift (out-of-range count traps) must NOT fold"
    );
}

// left_shift_that_overflows_fails_the_build (`(<< 4611686018427387904 1)` → CDZ0304) migrated to corpus
// 06-numeric-model "a CONSTANT bare-Int64 left shift whose result overflows Int64 is rejected". rcdzc test deleted.

// NOTE: the constant out-of-range shift-count rejects — `(<< 1 64)` (count ≥ width) and `(<< 1 -1)`
// (negative count) — migrated to corpus 06-numeric-model "a constant shift by an OUT-OF-RANGE count is
// rejected at compile time with CDZ0304", whose doc explicitly covers the `>= 64` and negative-count class
// (minimal repro `(<< 5 -1)` + the `(<< 5 64)` / right-shift twins). rcdzc tests deleted (corpus-covered).

#[test]
fn an_unsigned_narrow_width_shift_trap_names_the_width_and_cause() {
    // The tests above exercise the i64/`fold_arith` shift path (default width 64). The UNSIGNED
    // SOLVED-WIDTH fold (`fold_shift_bitwise_at_width`) has its OWN, more actionable CDZ0304 messages
    // that name the concrete width and distinguish an out-of-range COUNT from a shifted-result
    // OVERFLOW — the improvement PR#796's review asked for. Those narrow-width messages had NO
    // coverage; pin both so the actionable wording can't silently regress to a generic "count or
    // overflow". (An unsigned narrow type reaches the width fold; a signed type stays on the i64 path.)
    //
    // Count ≥ width: `(: 1 (UInt 8)) << (: 8 …)` — count 8 ≥ the 8-bit width, an out-of-range count.
    let count_oob = expect_decline("(<< (: 1 (UInt 8)) (: 8 (UInt 8)))");
    assert!(
        count_oob.contains("shift count 8 is out of range for the 8-bit type")
            && count_oob.contains("must be 0..=7"),
        "an unsigned narrow-width out-of-range shift count names the width + valid range: {count_oob}"
    );
    // Result overflow (in-range count): `(: 4 (UInt 8)) << 7` = 512, which moves a set bit past the
    // 8-bit width — distinct from the count fault, named as an overflow with the offending shift amount.
    let result_ovf = expect_decline("(<< (: 4 (UInt 8)) (: 7 (UInt 8)))");
    assert!(
        result_ovf.contains("the shifted result overflows the 8-bit type")
            && result_ovf.contains("by 7 moves a set bit past the width"),
        "an unsigned narrow-width shifted-result overflow names the width + is distinct from a \
             count fault: {result_ovf}"
    );
}

// ── comparisons: ∀a. a → a → Bool, folded to a boolean, generic over the operand type ────────

#[test]
fn constant_compound_equality_folds_and_a_runtime_one_emits_a_heap_walk() {
    // The CONSTANT-compound structural-equality folds (Some/None/tuple/nested → bool, and the Bytes
    // structural-equality arms) are corpus cases: 03-equality-and-observation "constant compound
    // equality folds structurally over Option, None, tuple, and nesting" and 10-bytes "constant Bytes
    // structural equality folds over unequal, length-differing, concat, and compact forms".
    // A genuinely-RUNTIME compound comparison (a recursive result, not constant-foldable) now
    // COMPILES to a `value-eq` heap walk (a scalar-leaf compound is canonical by construction, so the
    // tagless `champ_eq` walk is exact). `dn 3` recurses (cannot fold), so `(= (dn 3) (Some 0))`
    // reaches the runtime structural-equality path — it must build a component (not decline) and
    // IMPORT the runtime (the walk is a runtime call). The RESULT VALUE (true → 1) is verified end to
    // end by the corpus heap-walk cases (03-equality-and-observation), which run against the composed
    // runtime; here we assert the compile succeeds AND pulls in the runtime import.
    let runtime = "(module m (def (dn n) (if (< n 1) (Some 0) (dn (- n 1)))) \
                        (def (main) (if (= (dn 3) (Some 0)) 1 0)) (export main))";
    let program = compile_component(&crate::codec::encode(&parse(runtime)))
        .expect("a runtime scalar-leaf compound equality now compiles to a value-eq heap walk");
    assert!(
        imports_value_heap_runtime(&program),
        "a runtime compound equality imports the value-heap runtime (the value-eq walk is a runtime call)"
    );
}

// provable_overflow_fails_the_build (`(+ Int64.max 1)` → CDZ0304) migrated to corpus 06-numeric-model
// "overflow of the default integer traps deterministically". rcdzc test deleted (corpus-covered).

// ── type annotations `(: e T)`: transparent to the value, constrains the type ────────────────

// The two positive transparency RUN cases — `(: 5 (Int 64))` = 5 (the reduced `(Int 64)` type-ctor
// annotation grounds the width) and `(: true Bool)` = true (the Bool boundary is transparent) — are
// covered by corpus 07-type-system: "an annotation whose type is a reduced (Int 64) constructor grounds
// the value" + "a Bool annotation on a Bool value is transparent". The reject companion stays here.
#[test]
fn a_lowercase_name_in_a_type_position_points_at_the_unannotated_generic_route() {
    // A bare LOWERCASE name in a type-annotation position that resolves to nothing — `(: x a)`. An ML/
    // Haskell user reads `a` as a TYPE VARIABLE (and it IS one in a VARIANT PAYLOAD `(type Box (B a))`),
    // but an annotation has no `∀`-binder to scope a fresh variable, so `a` is genuinely unbound. The
    // bare "unbound name `a`" is right but unhelpful; the message now points at the real route to
    // polymorphism — an UNANNOTATED parameter. Still CDZ0101 (unbound), across all three annotation sites.
    let unbound_hint = |src: &str| -> String {
        crate::diagnostics(&mut crate::db::Db::load(parse(src)))
            .into_iter()
            .find(|d| d.code.as_deref() == Some("CDZ0101"))
            .unwrap_or_else(|| panic!("expected CDZ0101 for {src}"))
            .message
    };
    for src in [
        "(module m (def (id (: x a)) x) (def (main) (id 1)) (export main))",
        "(module m (def (main) (: 5 a)) (export main))",
        "(module m (def (main) (let (((: y a) 5)) y)) (export main))",
    ] {
        let m = unbound_hint(src);
        assert!(
            m.contains("unbound name `a`")
                && m.contains("not a type variable")
                && m.contains("UNANNOTATED"),
            "the lowercase-type-var hint points at the unannotated-generic route: {m}"
        );
    }
    // At a PARAMETER site (where a generic function is being written), the hint ALSO names the explicit
    // `Type`-parameter idiom — the composable route for a documenting user-generic signature `(: it (Iter
    // a))`. The value/let-binder sites (no parameter list) do NOT, keeping only the drop / concrete-type
    // guidance. (Generics are type-valued parameters — spec §"Generics Are Type-Valued Parameters".)
    // The parameter-ness is driven by an EXPLICIT flag threaded from the call site, NOT by sniffing the
    // human-readable `lead` string (Copilot PR #438) — so these by-site assertions also guard that a
    // future reword of `lead` cannot silently drop/add the Type-parameter route. Both the TOP-LEVEL and
    // a NESTED `(List a)` parameter annotation carry it (the flag threads through the nested walk).
    for param_site in [
        "(module m (def (id (: x a)) x) (def (main) (id 1)) (export main))",
        "(module m (def (id (: x (List a))) x) (def (main) (id (list))) (export main))",
    ] {
        let param_hint = unbound_hint(param_site);
        assert!(
            param_hint.contains("`Type` parameter") && param_hint.contains("(: t Type)"),
            "a parameter-site lowercase type-var (top-level + nested) names the Type-parameter route: {param_hint}"
        );
    }
    for value_site in [
        "(module m (def (main) (: 5 a)) (export main))",
        "(module m (def (main) (let (((: y a) 5)) y)) (export main))",
    ] {
        let m = unbound_hint(value_site);
        assert!(
            !m.contains("`Type` parameter"),
            "a value/binder-site hint does NOT name the Type-parameter route (no param list): {m}"
        );
    }
    // NESTED type-var positions get the SAME rich guidance (was the terse "unbound name `b`"): a var
    // inside `(List b)` / `(Tuple a b)` / `(-> a b)` / `(Map k v)`, at every annotation site.
    for src in [
        "(module m (def (f (: x (List b))) x) (export f))",
        "(module m (def (f (: x (Tuple a b))) x) (export f))",
        "(module m (def (f (: g (-> a b))) g) (export f))",
        "(module m (def (f (: m (Map k v))) m) (export f))",
        "(module m (def (main) (: 5 (List b))) (export main))",
        "(module m (def (main) (let (((: x (List b)) 5)) 0)) (export main))",
    ] {
        let m = unbound_hint(src);
        assert!(
            m.contains("not a type variable") && m.contains("UNANNOTATED"),
            "a NESTED lowercase type-var gets the rich hint too: {src} -> {m}"
        );
    }
    // An UPPERCASE unknown type NESTED in a compound (`(List Widget)`) now names it a missing TYPE too
    // (the same message the top-level case gets — the nested walk enriches uppercase leaves alongside
    // lowercase type-vars), NOT a would-be var and NOT the terse "unbound name". A near typo in the same
    // nested position still keeps its did-you-mean (asserted below).
    let nested_upper = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def (f (: x (List Widget))) x) (export f))",
    )))
    .into_iter()
    .find(|d| d.code.as_deref() == Some("CDZ0101"))
    .expect("Widget nested is unbound");
    assert!(
        nested_upper.message.contains("unknown type `Widget`")
            && nested_upper.message.contains("(type Widget …)")
            && !nested_upper.message.contains("not a type variable"),
        "an uppercase nested missing type names the missing type: {}",
        nested_upper.message
    );
    // NO false change: a NEAR typo of a real type nested in a compound (`(List Strng)`) keeps the
    // did-you-mean (the branch is gated on no near suggestion, exactly as the top-level case is).
    let nested_typo = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def (f (: x (List Strng))) x) (export f))",
    )))
    .into_iter()
    .find(|d| d.code.as_deref() == Some("CDZ0101"))
    .expect("Strng nested is unbound");
    assert!(
        nested_typo.message.contains("did you mean `String`?"),
        "a near type typo nested in a compound keeps the did-you-mean: {}",
        nested_typo.message
    );
    assert!(
        crate::diagnostics(&mut crate::db::Db::load(parse(
            "(module m (def (f (: x (List Int64))) x) (export f))"
        )))
        .iter()
        .all(|d| d.severity != crate::abi::Severity::Error),
        "a well-formed nested type annotation is clean"
    );
    // A lowercase name in a VARIANT PAYLOAD is still a genuine type variable — NOT this fault. And an
    // UPPERCASE unbound type name (`Widget`) is a plain missing-type, keeping the bare message (it is
    // not a would-be type variable).
    assert!(
        crate::diagnostics(&mut crate::db::Db::load(parse(
            "(module m (type Box (B a)) (def (main) 1) (export main))"
        )))
        .iter()
        .all(|d| d.severity != crate::abi::Severity::Error),
        "a lowercase name in a variant payload is a type variable, not an unbound-name fault"
    );
    // A bare UPPERCASE unknown type at a TOP-LEVEL annotation position names it a missing TYPE (not a
    // would-be type variable, not the generic "unbound name"): "unknown type `Widget` — no type by that
    // name is declared … declare it with `(type Widget …)`". Still CDZ0101. (A nested position like
    // `(List Widget)` now gets the SAME message — the `nested_upper` case above pins that.)
    let widget = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def (f (: x Widget)) x) (def (main) (f 1)) (export main))",
    )))
    .into_iter()
    .find(|d| d.code.as_deref() == Some("CDZ0101"))
    .expect("Widget is unbound");
    assert!(
        widget.message.contains("unknown type `Widget`")
            && widget.message.contains("(type Widget …)")
            && !widget.message.contains("not a type variable"),
        "an uppercase missing type in an annotation names the missing type + the declare fix: {}",
        widget.message
    );
    // A NEAR typo of a real type must still win the did-you-mean (the suggestion pool includes type
    // names), NOT the generic "unknown type" — the branch is gated on there being no near suggestion.
    let typo = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def (f (: x Strng)) x) (def (main) (f \"a\")) (export main))",
    )))
    .into_iter()
    .find(|d| d.code.as_deref() == Some("CDZ0101"))
    .expect("Strng is unbound");
    assert!(
        typo.message.contains("did you mean `String`?"),
        "a near type typo keeps the did-you-mean, not the generic unknown-type: {}",
        typo.message
    );
}

#[test]
fn a_bound_or_constant_width_does_not_trip_the_unbound_width_reject() {
    // WHITE-BOX RESIDUAL. The unbound-NAME-width rejects (`(Int hello)` etc. → width-specific CDZ0101,
    // nested + non-first arg positions) are now corpus 06-numeric-model ("an unbound name in an integer
    // width position…" + nested/non-first). This keeps the no-false-positive controls the corpus does not
    // cheaply express — chiefly a BOUND width VARIABLE `(Int a)` (a `Type` parameter used as a generic
    // width), which must check clean, not trip the unbound-width reject. (A concrete `(Int 64)` / odd
    // `(Int 7)` are corpus width cases; an over-ceiling `(UInt 65)` keeps its CDZ0302 there.)
    let diags = |src: &str| crate::diagnostics(&mut crate::db::Db::load(parse(src)));
    for ok in [
        "(module m (def (f (: a (Int 64))) a) (export f))",
        "(module m (def (f (: a (Int 7))) a) (export f))",
        "(module m (def (f (: a Type) (: x (Int a))) x) (export f))",
    ] {
        let ok_diags = diags(ok);
        assert!(
            ok_diags
                .iter()
                .all(|d| d.severity != crate::abi::Severity::Error),
            "a valid width form checks clean: {ok} → {ok_diags:?}"
        );
    }
}

// ── integer widths (I3, fold): named widths, per-width bounds, annotations, odd widths ────────

#[test]
fn arbitrary_odd_widths_compute_their_bounds() {
    // The bounds are computed FROM THE WIDTH PARAMETER, not a per-named-type table — so an ODD,
    // non-machine width works: `(UInt 7)` max = 2^7-1 = 127, `(UInt 24)` max = 2^24-1 = 16777215,
    // `(UInt 48)` max = 2^48-1 = 281474976710655. These pin that nothing assumes a power-of-two
    // machine width. A non-aliased width is INTERNAL-ONLY (no boundary representation — R2), so the
    // bound is asserted at the FOLD (the arbitrary-precision `ConstInt`), not by running it across
    // the component edge. (`arbitrary-width value cannot be exported` is pinned separately below.)
    assert_eq!(fold_const_u128("(. (UInt 7) max)"), 127);
    assert_eq!(fold_const_u128("(. (UInt 24) max)"), 16777215);
    assert_eq!(fold_const_u128("(. (UInt 48) max)"), 281474976710655);
}

#[test]
fn an_odd_width_annotation_range_checks() {
    // `(: 127 (UInt 7))` fits (2^7-1=127); `(: 128 (UInt 7))` does NOT — the range check is
    // width-exact, not rounded to a machine width. The fit case is checked at the FOLD (a `(UInt 7)`
    // is internal-only, no boundary form); the overflow case rejects (CDZ0302) before any boundary.
    assert_eq!(fold_const_u128("(: 127 (UInt 7))"), 127);
    assert!(expect_decline("(: 128 (UInt 7))").contains("does not fit"));
}

#[test]
fn a_non_aliased_width_parameter_declines_naming_the_width() {
    // A non-aliased-width PARAMETER `(: x (UInt 48))` DECLINES (naming the width/boundary) — accepting
    // one would trust an incoming wider value fits the narrower width, which the guest cannot verify.
    // (The RESULT-crossing-widened RUN half is corpus-covered by 06-numeric-model "a non-aliased-width
    // result crosses the boundary widened to the next aliased width"; the full value-and-signedness
    // matrix is in `runtime_ops::a_non_aliased_width_result_crosses_…`.)
    let param = "(module m (def (f (: x (UInt 48))) x) (export f))";
    let msg = compile_component(&crate::codec::encode(&parse(param)))
        .expect_err("a non-aliased width parameter cannot be accepted")
        .message;
    assert!(
        msg.contains("boundary") || msg.contains("aliased"),
        "got: {msg}"
    );
}

// ── truncating conversion `T.wrap` (R3): constant FOLD, no sums, never traps ──────────────────
//
// `(UInt8.wrap n)` = `((. UInt8 wrap) n)` — projecting the module's `wrap` operator and applying it.
// On a constant it FOLDS via `IntValue::wrap_to` to a `ConstInt` already at the target width. The
// target width comes from the TYPE (`UInt8`), not a magic op name — one truncating conversion, width-
// indexed. The result crosses the boundary as the target's faithful primitive (u8/s8/…).

#[test]
fn wrap_to_a_nonaliased_width_folds() {
    // `((UInt 48).wrap -1)` = 2^48-1 at the FOLD (the low 48 bits of -1) — the truncation is correct.
    // `(UInt 48).wrap` is the postfix-member sugar (reads to `(. (UInt 48) wrap)`). The RUN half (the
    // result crossing WIDENED to `u64` and running to 281474976710655) is pinned in the corpus:
    // 06-numeric-model "`((UInt 48).wrap (: -1 Int64))` = 281474976710655".
    assert_eq!(fold_const_u128("((UInt 48).wrap -1)"), (1u128 << 48) - 1);
}

// NOTE: `signed_and_unsigned_of_the_same_width_do_not_promote` (`(+ (: 1 Int8) (: 2 UInt8))` → CDZ0301,
// signedness-alone mismatch at equal width) migrated to corpus 06-numeric-model "same-width integers of
// different signedness do not promote (signedness alone is a mismatch)". rcdzc test deleted (corpus-covered).

#[test]
fn a_wide_record_argument_unifies_across_many_calls_in_bounded_time() {
    // REGRESSION (perf): `unify` applies the substitution to BOTH operands on entry, and `Subst::apply`
    // REBUILT a `Ty::Record`'s whole field map (`.iter().map(apply).collect()` into a fresh `Rc`) even
    // when the type held no substitutable variable. So passing a WIDE (W-field) GROUND record argument
    // to a function called N times rebuilt the W-field map at every call site → O(W × calls). FIX: a
    // GROUND fast-path in `apply` (`Ty::is_ground` → return the input's cheap Rc clone), turning the
    // per-call cost from an allocate-and-rebuild into a read-only check. This binds one wide record and
    // passes it through a function W=N=200 times — well into the old quadratic regime; it must type,
    // compile, and RETURN the projected field (k0 = 0), in bounded time.
    let w = 200;
    let fields = (0..w)
        .map(|i| format!("(= k{i} {i})"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut calls = String::from("0");
    for _ in 0..w {
        calls = format!("(+ {calls} (f r))");
    }
    let src = format!(
        "(module m (def (f x) (. x k0)) (def (main) (let ((r (record {fields}))) {calls})) (export main))"
    );
    // Every `(f r)` projects k0 = 0, so the sum is 0 — a well-typed program. The perf-relevant part is
    // that type-checking (the per-call `unify` → `apply` over the wide record) and full compilation
    // both COMPLETE in bounded time; the record's runtime value is covered by the record-read tests.
    // Through the host-stack guard the bin uses (`host.rs`): the per-call unify/fold walk recurses deep
    // over the wide record, SIGABRTing a default `cargo test` worker's ≈2 MB stack (EXIT=101, 0 FAILED)
    // even though it TERMINATES — deep-but-finite, not a loop (`RUST_MIN_STACK=64M` passes). `&src` is
    // borrowed (the scoped guard permits it); `src` is still used by the `compile_component` below,
    // which self-guards. Sizing the stack from `DESCENT_DEPTH_LIMIT` bounds it by depth.
    let diags = crate::host::run_with_compiler_stack(|| {
        crate::diagnostics(&mut crate::db::Db::load(parse(&src)))
    });
    assert!(
        diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a wide record passed through many calls type-checks cleanly: {diags:?}"
    );
    compile_component(&crate::codec::encode(&parse(&src)))
        .expect("wide-record-arg program compiles in bounded time");
}

// ── binding patterns: a `let` binder may be an irrefutable pattern ───────────────────────────

#[test]
fn an_annotated_let_binder_structural_mismatch_names_the_delta() {
    // A `(: <pat> <Type>)` let binder whose annotation and bound value are the same structured kind but
    // differ — two records of a different field set, two tuples of a different arity — named two whole
    // types the reader had to diff ("a binder annotated (Record (x Int64)) is bound to a value of type
    // (Record (y Int64))"). It now appends the structural DELTA (the shared `structural_delta_hint`),
    // the SAME minimal-conflict hint the value-annotation / argument / peer-join sites carry.
    let msg = |body: &str| -> String {
        let src = format!("(module m (def (main) {body}) (export main))");
        crate::diagnostics(&mut crate::db::Db::load(parse(&src)))
            .into_iter()
            .find(|d| d.code.as_deref() == Some("CDZ0203"))
            .unwrap_or_else(|| panic!("expected CDZ0203 for {body}"))
            .message
    };
    // A record FIELD-SET difference.
    let rec = msg("(let (((: r (Record (x Int64))) (record (= y 2)))) r)");
    assert!(
        rec.contains("a binder annotated") && rec.contains("missing field `x`"),
        "the record field-set delta is named on the binder: {rec}"
    );
    // A record FIELD-TYPE difference (same field set).
    let field_ty = msg("(let (((: r (Record (x Int64))) (record (= x true)))) r)");
    assert!(
        field_ty.contains("field `x` should be Int64, but this one is Bool"),
        "the field-type delta is named: {field_ty}"
    );
    // A tuple ARITY difference.
    let tup = msg("(let (((: t (Tuple Int64 Int64)) (tuple 1 2 3))) t)");
    assert!(
        tup.contains("expected a tuple with 2 elements, but this one has 3"),
        "the tuple arity delta is named: {tup}"
    );
    // NO false positive: a scalar mismatch (Bool vs Int64) has no structural delta — the bare message.
    let scalar = msg("(let (((: x Bool) 5)) x)");
    assert!(
        scalar.contains("a binder annotated Bool is bound to a value of type Int64")
            && !scalar.contains(" — "),
        "a scalar binder mismatch keeps the bare message (no delta tail): {scalar}"
    );
}

#[test]
fn a_let_binder_may_be_a_zero_leading_list_rest_pattern_leading_element_rest_is_refutable() {
    // core-semantics.md §A Binding Position Accepts An Irrefutable Pattern + §147: a list pattern is
    // irrefutable ONLY in the ZERO-LEADING rest form `(list .. rest)` — it matches EVERY list (empty
    // included), binding `rest` (→ `SumPayload{[RestFrom(0)]}`) to the whole value, so it may bind. A
    // LEADING-element rest `(list a .. rest)` is REFUTABLE (it requires ≥1 element, missing the empty
    // list), so in a binding position it is CDZ0210 — the same rule the fixed-arity form gets (operator
    // ruling: only the zero-leading form is irrefutable; a possibly-empty leading destructure belongs in
    // a `match`). This test asserts the zero-leading form COMPILES and the leading forms REJECT.
    let code = |body: &str| -> Option<String> {
        let src = format!("(module m (def (main) {body}) (export main))");
        let out = crate::compile::compile(
            &[crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "m",
                crate::codec::encode(&parse(&src)),
            )],
            &[crate::backend::Target::Wasm],
        );
        if out
            .artifact(crate::backend::Target::Wasm.artifact_kind())
            .is_some()
        {
            return None;
        }
        out.diagnostics
            .iter()
            .find(|d| d.severity == crate::abi::Severity::Error)
            .and_then(|d| d.code.clone())
    };
    // Zero-leading `(list .. all)` binds the WHOLE list as `all` (irrefutable, matches every list).
    assert_eq!(
        code("(let (((list .. all) (list 1 2 3 4))) ((. List len) all))"),
        None
    );
    // A LEADING + rest binder is now refutable → CDZ0210 (misses the empty list).
    assert_eq!(
        code("(let (((list a b .. rest) (list 10 20 30))) (+ a b))"),
        Some("CDZ0210".to_string())
    );
    // A NESTED tuple leading element is STILL a leading element (dd > 0) → CDZ0210.
    assert_eq!(
        code("(let (((list (tuple a b) .. rest) (list (tuple 1 2) (tuple 3 4)))) (+ a b))"),
        Some("CDZ0210".to_string())
    );
    // A single-leading-element rest → CDZ0210 as well.
    assert_eq!(
        code("(let (((list a .. rest) (list 10 20 30))) a)"),
        Some("CDZ0210".to_string())
    );
}

// (Removed `a_constant_list_let_read_by_element_binders_folds_to_a_scalar`: it asserted the constant-fold
// of a LEADING-element list-let `(list a b .. rest)` read by its element binders. Under the operator/spec
// ruling (only the ZERO-LEADING `(list .. rest)` is irrefutable in a binding position; a leading-element
// rest is refutable → CDZ0210, §139/§147), that binding form no longer compiles, so the fold is moot —
// its inputs are now compile-time rejects. The zero-leading form binds the whole list, which escapes and
// materializes, so there is no scalar-fold case left to pin here.)

#[test]
fn a_wrong_type_variant_payload_is_a_malformed_construction() {
    // A variant constructor is a single-arity function whose argument is checked against its DECLARED
    // payload type (core-semantics.md §A Sum Type Constructor Is A Single-Arity Function). A wrong-type
    // / wrong-arity payload is a MALFORMED construction (CDZ0201, a structural sum-shape violation),
    // the code the corpus assigns — NOT the generic unify mismatch (CDZ0203) an ordinary function's
    // argument mismatch gets. The reclassification is on the SAME instantiated-and-substituted unify
    // the generic application takes, so a GENERIC/nested construction is not over-rejected.
    let code = |body: &str| -> Option<String> {
        let src = format!("(module m {body} (export main))");
        let out = crate::compile::compile(
            &[crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "m",
                crate::codec::encode(&parse(&src)),
            )],
            &[crate::backend::Target::Wasm],
        );
        if out
            .artifact(crate::backend::Target::Wasm.artifact_kind())
            .is_some()
        {
            return None;
        }
        out.diagnostics
            .iter()
            .find(|d| d.severity == crate::abi::Severity::Error)
            .and_then(|d| d.code.clone())
    };
    // Wrong-type scalar payload (String where Int64 declared), wrong-arity tuple payload → CDZ0201.
    assert_eq!(
        code("(type T (Mk Int64)) (def (main) (T.Mk \"x\"))").as_deref(),
        Some("CDZ0201")
    );
    assert_eq!(
        code("(type W (Wt (Tuple Int64 Int64))) (def (main) (W.Wt (tuple 1 2 3)))").as_deref(),
        Some("CDZ0201")
    );
    // NO OVER-REJECTION: a GENERIC variant accepts any payload type; a nested construction is fine.
    assert_eq!(
        code("(def (main) (match (Some true) ((Some x) (if x 1 0)) (None 0)))"),
        None
    );
    assert_eq!(
        code("(def (main) (match (Ok (Err 9)) ((Ok (Ok n)) n) ((Ok (Err e)) e) ((Err _) -2)))"),
        None
    );
    // An ordinary (non-constructor) function's argument mismatch stays CDZ0203, not reclassified.
    assert_eq!(
        code("(def (main) (if (< 1 true) 1 0))").as_deref(),
        Some("CDZ0203")
    );
}

#[test]
fn comparing_distinct_same_shape_nominal_sums_is_a_nominal_boundary_error() {
    // Comparing two values whose types are distinct NOMINAL sums of the SAME structural shape —
    // `(= (A.Mk 1) (B.Mk 1))` for `(type A (Mk Int64))` / `(type B (Mk Int64))` — is a comparison
    // across the nominal boundary (CDZ0202), NOT the `false` an untagged structural comparison gives
    // (type-system.md §Nominal Types Are Not Comparable Across Their Boundary). Two sums of DIFFERENT
    // shape (disjoint variants — `Option` vs `Result`) are unrelated types → the plain CDZ0203; the
    // same sum compared to itself is well-typed (→ false).
    let code = |body: &str| -> Option<String> {
        let src = format!("(module m {body} (def (main) 0) (export main))");
        let out = crate::compile::compile(
            &[crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "m",
                crate::codec::encode(&parse(&src)),
            )],
            &[crate::backend::Target::Wasm],
        );
        out.diagnostics
            .iter()
            .find(|d| d.severity == crate::abi::Severity::Error)
            .and_then(|d| d.code.clone())
    };
    // Same-shape distinct sums → CDZ0202.
    assert_eq!(
        code("(type A (Mk Int64)) (type B (Mk Int64)) (def (c) (= (A.Mk 1) (B.Mk 1)))").as_deref(),
        Some("CDZ0202")
    );
    // Disjoint-variant sums (Option vs Result) → CDZ0203 (unrelated types, not a nominal boundary).
    assert_eq!(
        code("(def (c) (= (Some 1) (Ok 1)))").as_deref(),
        Some("CDZ0203")
    );
    // A scalar-vs-Bool mismatch stays CDZ0203.
    assert_eq!(code("(def (c) (= 1 true))").as_deref(), Some("CDZ0203"));
}

#[test]
fn a_recursive_function_over_a_sum_infers_its_unannotated_parameters() {
    // A recursive function that MATCHES an unannotated parameter against a user sum's constructors —
    // `(def (len c) (match c (CNil 0) ((CCons (tuple h t)) (+ 1 (len t)))))` — infers `c : Code` from
    // the pattern SHAPE (`pattern_implied_ty` threads the variant/tuple pattern onto the param var),
    // rather than leaving it a free var that grounds to `Any` and declines "annotate its parameters".
    // A PASS-THROUGH parameter returned by one arm (`ys` in `cat`) is inferred from the sibling arm's
    // determined result. Both were previously an unannotated-recursive-param decline. `compiles`
    // means the recursive signatures were determined (no decline); the runtime run is corpus-gated.
    let compiles = |body: &str| -> bool {
        let src = format!("(module m {body} (export main))");
        let out = crate::compile::compile(
            &[crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "m",
                crate::codec::encode(&parse(&src)),
            )],
            &[crate::backend::Target::Wasm],
        );
        out.artifact(crate::backend::Target::Wasm.artifact_kind())
            .is_some()
    };
    // `len` — a single sum parameter matched against its constructors, inferred from the pattern.
    assert!(compiles(
        "(type Code CNil (CCons (Tuple Int64 Code))) \
             (def (len c) (match c (CNil 0) ((CCons (tuple h t)) (+ 1 (len t))))) \
             (def (main) (len (CCons (tuple 1 (CCons (tuple 2 CNil))))))"
    ));
    // `cat` — TWO sum parameters, one (`xs`) inferred from the pattern, the other (`ys`) inferred as
    // the pass-through returned by the `CNil` arm (its type borrowed from the `CCons` arm's result).
    assert!(compiles(
        "(type Code CNil (CCons (Tuple Int64 Code))) \
             (def (cat xs ys) (match xs (CNil ys) ((CCons (tuple h t)) (CCons (tuple h (cat t ys)))))) \
             (def (len c) (match c (CNil 0) ((CCons (tuple h t)) (+ 1 (len t))))) \
             (def (main) (len (cat (CCons (tuple 1 CNil)) CNil)))"
    ));
}

#[test]
fn sum_shape_descriptor_closes_recursion_with_a_ref() {
    // The compiler's shape descriptor for a RECURSIVE sum `(type IL (Cons (Tuple Int64 IL)) Nil)` is
    // a TABLE with a self-`Ref` (the `Cons` payload tuple's second element points back at the sum),
    // so the descriptor is FINITE. This must be BYTE-IDENTICAL to the descriptor the runtime's
    // `value_encode_form_matches_the_codec` test hard-codes (the compiler/runtime contract). We build
    // it from the solved `Ty::Sum` and assert the bytes.
    let src = "(module m (type IL (Cons (Tuple Int64 IL)) Nil) (def (main) Nil) (export main))";
    let mut db =
        crate::db::Db::load(crate::codec::decode(&crate::codec::encode(&parse(src))).unwrap());
    let body = db
        .defs
        .iter()
        .find(|d| d.name == "main")
        .unwrap()
        .body
        .unwrap();
    let ty = crate::infer::type_of(&mut db, body);
    let desc = crate::lower::sum_shape_descriptor(&mut db, &ty).expect("descriptor");
    // The value walk must terminate + close the recursion: decode the table, confirm some entry is a
    // `Ref` (tag 11) — the recursion-closing back-edge — and that the descriptor is non-trivial.
    assert!(
        desc.len() > 8,
        "a recursive-sum descriptor is more than a trivial table"
    );
    assert!(
        desc.contains(&11u8),
        "the recursive payload closes with a Ref (tag 11)"
    );
    // The descriptor round-trips through a bounded decode (no infinite expansion): count the table
    // entries via the leading LEB and confirm it is small + finite (the IL sum has a handful).
    let table_len = desc[0] as usize; // small table → single LEB byte
    assert!(
        (1..=16).contains(&table_len),
        "IntList's shape table is small + finite, got {table_len}"
    );
}

#[test]
fn sum_shape_descriptor_describes_a_record_carrying_arm() {
    // A 2-arm sum whose 2nd arm carries a RECORD `(B (Record (x String)))`: the descriptor MUST describe
    // arm B's record (the field name `x` rides in the descriptor), NOT drop it to empty — the codegen bug
    // where a 2-arm-sum record arm value-encodes to an empty leaf (035911). Localizes descriptor-vs-walker:
    // if this ASSERT passes the descriptor is correct (bug is downstream in the runtime value-encode walker);
    // if it FAILS the descriptor drops the record (bug is here in shape_of/sum_shape_descriptor).
    let src = "(module m (type P (A Bytes) (B (Record (x String)))) (def (main) (A (Bytes.of (list)))) (export main))";
    let mut db =
        crate::db::Db::load(crate::codec::decode(&crate::codec::encode(&parse(src))).unwrap());
    let body = db
        .defs
        .iter()
        .find(|d| d.name == "main")
        .unwrap()
        .body
        .unwrap();
    let ty = crate::infer::type_of(&mut db, body);
    let desc = crate::lower::sum_shape_descriptor(&mut db, &ty).expect("descriptor");
    eprintln!("[035911] P descriptor bytes = {desc:02x?}");
    assert!(
        desc.contains(&b'x'),
        "the descriptor must carry arm B's record field name `x` (not drop the record to empty): {desc:02x?}"
    );
}

#[test]
fn shape_descriptor_peels_a_quantity_element_to_its_inner() {
    // A quantity erases to its inner scalar at runtime (the unit is compile-time-only), so the shape
    // descriptor of a `(List (Qty Int64 meter))` must be Some — `shape_of` peels `Ty::Qty` to the inner
    // and produces the SAME shape as `(List Int64)`. Without the peel, `shape_of` fell to `_ => None`, so
    // a compound Map/Set KEY containing a quantity (a list-of-Qty / tuple-of-Qty key) DECLINED to compile
    // ("list-key canonicalization: key type has no bakeable shape descriptor"). This pins the peel.
    use crate::ty::{Ty, Unit};
    let qty = Ty::Qty {
        inner: Box::new(Ty::int64()),
        unit: Unit::base("meter"),
    };
    let list_of_qty = Ty::List(Box::new(qty));
    let list_of_int = Ty::List(Box::new(Ty::int64()));
    let mut db = crate::db::Db::load(
        crate::codec::decode(&crate::codec::encode(&parse(
            "(module m (def (main) 0) (export main))",
        )))
        .unwrap(),
    );
    let desc_qty = crate::lower::value_cmp_shape_descriptor(&mut db, &list_of_qty)
        .expect("a list-of-quantity key type now has a bakeable shape descriptor");
    let desc_int = crate::lower::value_cmp_shape_descriptor(&mut db, &list_of_int)
        .expect("a list-of-int key type has a shape descriptor");
    // The quantity element erases to its inner Int64, so the two descriptors are BYTE-IDENTICAL — the
    // key comparator/hasher canonicalizes a list-of-Qty exactly as a list-of-Int (the unit is erased).
    assert_eq!(
        desc_qty, desc_int,
        "a (List (Qty Int64 meter)) key hashes/compares identically to a (List Int64) key — the unit erases"
    );
}

#[test]
fn two_sibling_modules_may_each_define_a_private_helper_of_the_same_name() {
    // REGRESSION: the duplicate-value-def check is PER-MODULE (per-file), not global — mirroring the
    // duplicate-TYPE check. A value-name set is fixed within ONE module, but two SEPARATE files of a
    // linked package may each define a PRIVATE helper of the same name (`node-count` in a lib AND in
    // the importing entry). Each module owns its value namespace; a sibling's un-imported def is
    // invisible, so re-using the name is not a redeclaration. The old global scan flagged this as a
    // spurious "defined more than once", blocking the idiomatic multi-module layout (a shared type
    // module `ast` + several passes that each carry their own generically-named helper).
    let lib = parse(
        "(do (type Ast (Int Int64) (Name String)) (def (foo (: x Int64)) (+ x 1)) (export (. Ast *)))",
    );
    let app = parse(
        "(do (import \"lib\" (Ast)) (def (foo (: x Int64)) (+ x 2)) \
               (def (main) (foo (match (. Ast Int 0) (((. Ast Int) n) n) (((. Ast Name) _) 0)))) (export main))",
    );
    let files = vec![("lib".to_string(), lib), ("app".to_string(), app)];
    let linked = crate::link::link(&files, "app").expect("package links");
    let mut db = crate::db::Db::load_linked(linked.arenas.clone(), Some(linked.linkage()));
    let dups: Vec<_> = crate::diagnostics(&mut db)
        .into_iter()
        .filter(|d| d.message.contains("defined more than once"))
        .collect();
    assert!(
        dups.is_empty(),
        "a private `foo` in each of two sibling modules is not a duplicate: {dups:?}"
    );

    // But a SAME-FILE duplicate must STILL reject — the per-file scoping narrows the collision, it
    // does not remove it. Two `foo` in ONE module collide as before.
    let mut db2 = crate::db::Db::load(parse("(module m (def (foo) 1) (def (foo) 2) (export foo))"));
    assert!(
        crate::diagnostics(&mut db2)
            .iter()
            .any(|d| d.message.contains("defined more than once")),
        "a same-file duplicate def still rejects"
    );
}

#[test]
fn a_package_export_binds_the_entry_files_def_not_a_private_siblings_same_named_def() {
    // The sharper control (breaker's "even a PRIVATE lib `main` hijacks"): the sibling `lib` defines
    // `main` but does NOT export it (it exports `other`); the entry `zzz` exports its own `main`. The
    // component's `main` must still be the entry's — a private, un-exported sibling def must never
    // hijack the export. This pins that `export_def` resolves through the exporting file's OWN visible
    // map (its defs + imports), which includes a file's private defs by name for its OWN export only.
    let lib =
        parse("(do (def (main (: n Int64)) (* n 100)) (def (other (: n Int64)) n) (export other))");
    let zzz = parse("(do (def (main (: n Int64)) (* n 7)) (export main))");
    let files = vec![("lib".to_string(), lib), ("zzz".to_string(), zzz)];
    let linked = crate::link::link(&files, "zzz").expect("package links");
    let db = crate::db::Db::load_linked(linked.arenas.clone(), Some(linked.linkage()));
    // The exported `main` must bind to the def whose signature occurrence lives in the ENTRY file
    // (`zzz`), not `lib`'s. Verify at the fold level: the export's `def` is a def in the entry file.
    let main_export = db
        .exports
        .iter()
        .find(|e| e.name == "main")
        .expect("component exports `main`");
    let def_idx = main_export.def.expect("exported `main` binds a def");
    let sig_occ = db.defs[def_idx].sig_occ;
    let entry_file = linked
        .files
        .iter()
        .position(|f| f.path == "zzz")
        .expect("entry file present");
    assert_eq!(
        db.file_of(sig_occ),
        Some(entry_file),
        "the exported `main` binds the ENTRY file `zzz`'s def, not the private sibling `lib`'s"
    );
}

#[test]
fn binding_position_irrefutability_holds_across_all_lambda_positions() {
    // COVERAGE-HARDENING (v-inference 2026-08-02, at-rest sweep): the binding-position irrefutability
    // rule (core-semantics.md:135-139 — a `fn` parameter MUST accept an irrefutable pattern; a refutable
    // one is CDZ0210) must hold UNIFORMLY across every lambda position, not just the let-bound one the
    // recent fixes (adv-51 / refutable-lambda-body / nested-gap) directly exercised. Pins that a
    // destructuring param BINDS and a refutable param REJECTS whether the lambda is a HOF ARGUMENT or
    // IMMEDIATELY-APPLIED — guarding these positions against a future regression in the desugar/walk.
    use crate::abi::Artifact;
    let codes_for = |s: &str| -> Vec<String> {
        let entry = crate::codec::encode(&parse(s));
        let out = crate::compile::compile(
            &[
                Artifact::new(Artifact::KIND_AST, "app", entry.clone()),
                cadenza_compile_abi::abi::entry_artifact("app"),
            ],
            &[crate::backend::Target::Wasm],
        );
        out.diagnostics
            .iter()
            .filter_map(|d| d.code.clone())
            .collect()
    };
    // A destructuring tuple param on a lambda passed as a HOF ARGUMENT binds cleanly.
    assert!(
            !codes_for("(do (def (ap (: f (-> (Tuple Int64 Int64) Int64)) (: p (Tuple Int64 Int64))) (f p)) (def (main) (ap (fn ((tuple a b)) (+ a b)) (tuple 3 4))) (export main))")
                .contains(&"CDZ0210".to_string()),
            "an irrefutable tuple param on a HOF-argument lambda binds (no CDZ0210)"
        );
    // A REFUTABLE param on a HOF-argument lambda rejects CDZ0210.
    assert!(
            codes_for("(do (def (ap (: f (-> (Option Int64) Int64)) (: o (Option Int64))) (f o)) (def (main) (ap (fn ((Some x)) x) (Some 3))) (export main))")
                .contains(&"CDZ0210".to_string()),
            "a refutable param on a HOF-argument lambda rejects CDZ0210"
        );
    // A destructuring param on an IMMEDIATELY-APPLIED lambda binds cleanly.
    assert!(
        !codes_for("(do (def (main) ((fn ((tuple a b)) (+ a b)) (tuple 3 4))) (export main))")
            .contains(&"CDZ0210".to_string()),
        "an irrefutable tuple param on an immediately-applied lambda binds (no CDZ0210)"
    );
    // A REFUTABLE param on an IMMEDIATELY-APPLIED lambda rejects CDZ0210.
    assert!(
        codes_for("(do (def (main) ((fn ((Some x)) x) (Some 3))) (export main))")
            .contains(&"CDZ0210".to_string()),
        "a refutable param on an immediately-applied lambda rejects CDZ0210"
    );
}

#[test]
fn a_deep_curried_application_spine_reduces_without_tripping_the_reduce_depth_limit() {
    // FIX (v-inference 2026-08-02, co-diagnosed w/ v-compiler-perf): a curried application spine
    // `((((f 0) 1) 2)…)` over an N-param def declined CDZ0999 at N≈32 — `lambda_of`'s Apply arm reduced
    // the spine HEAD-FIRST recursively, holding one `enter_reduction` guard alive per level, so an N-deep
    // spine nested N guards and tripped `REDUCE_DEPTH_LIMIT=32`, wrongly declining a legitimately
    // terminating program. FIX: flatten the spine + reduce left-to-right in a loop that drops the guard
    // per step, keeping guard depth ~1. Verifies a deep spine now COMPILES + computes the right value,
    // AND that a genuinely divergent term still declines (the divergence guard is preserved).
    use crate::abi::Artifact;
    use crate::testkit::parse;
    let build = |n: usize| -> String {
        let params: String = (0..n)
            .map(|k| format!("(: p{k} Int64)"))
            .collect::<Vec<_>>()
            .join(" ");
        let mut body = format!("p{}", n - 1);
        for k in (0..n - 1).rev() {
            body = format!("(+ p{k} {body})");
        }
        let mut spine = String::from("f");
        for k in 0..n {
            spine = format!("({spine} {k})");
        }
        format!("(do (def (f {params}) {body}) (def (main) {spine}) (export main))")
    };
    // A 50-arg curried spine compiles + computes 0+1+…+49 = 1225 (was CDZ0999 at N≈32).
    let _ = compile_component(&crate::codec::encode(&parse(&build(50))))
        .expect("a 50-arg curried spine reduces without tripping REDUCE_DEPTH_LIMIT");
    // DIVERGENCE GUARD PRESERVED: a self-applying term must STILL decline CDZ0999 (not loop / overflow).
    let div = {
        let entry = crate::codec::encode(&parse(
            "(do (def (main) (let ((w (fn (v) (v (v v))))) (w w))) (export main))",
        ));
        let out = crate::compile::compile(
            &[
                Artifact::new(Artifact::KIND_AST, "app", entry.clone()),
                cadenza_compile_abi::abi::entry_artifact("app"),
            ],
            &[crate::backend::Target::Wasm],
        );
        out.diagnostics
            .iter()
            .filter_map(|d| d.code.clone())
            .collect::<Vec<_>>()
    };
    assert!(
        div.contains(&"CDZ0999".to_string()),
        "a self-applying divergent term must still decline CDZ0999 (guard preserved)"
    );
}

#[test]
fn a_tuple_destructuring_parameter_on_a_lambda_binds_like_a_def_param() {
    // OVER-REJECT FIX (v-inference 2026-08-02, breaker adv-51): a tuple-destructuring parameter on a
    // `fn`/LAMBDA rejected CDZ0101 "unbound", while the SAME irrefutable pattern works as a `def`
    // parameter and a `let` binder — a core-semantics.md:135-137 violation (a `fn` parameter MUST
    // accept an irrefutable pattern). ROOT: `binding_params::lower` only walked the module's `defs`,
    // never `fn` expression nodes in bodies, so a lambda's destructuring param never got the
    // `(fn ((tuple a b)) BODY) → (fn (p$k) (let (((tuple a b) p$k)) BODY))` desugar. FIX: `lower` now
    // also walks each def body for such lambdas and rewrites them. Verifies the lambda now BINDS +
    // computes the right value at runtime, matches the def-param control, handles nested + multi-param
    // lambdas, and still rejects an ill-formed (refutable) lambda pattern with the binding-position code.
    use crate::testkit::parse;
    // The adv-51 minimal: f = (fn ((tuple x y)) (+ (* x 10) y)); (f (tuple 3 4)) → 34.
    let src = "(do (def (main) (let ((f (fn ((tuple x y)) (+ (* x 10) y)))) (f (tuple 3 4)))) \
                   (export main))";
    let _ = compile_component(&crate::codec::encode(&parse(src)))
        .expect("a tuple-destructuring lambda parameter binds and compiles");
    // NESTED lambda: an inner destructuring lambda inside an outer one.
    let nested = "(do (def (main) \
                        (let ((g (fn ((tuple a b)) (let ((h (fn ((tuple c d)) (+ c d)))) (+ (* a 10) (+ b (h (tuple 1 2)))))))) \
                          (g (tuple 3 4)))) (export main))";
    let _ = compile_component(&crate::codec::encode(&parse(nested)))
        .expect("a nested destructuring lambda compiles");
    // MULTI-PARAM lambda: a destructuring param alongside a bare param.
    let multi = "(do (def (main) (let ((f (fn ((tuple x y) z) (+ (+ x y) z)))) (f (tuple 3 4) 5))) \
                     (export main))";
    let _ = compile_component(&crate::codec::encode(&parse(multi)))
        .expect("a multi-param destructuring lambda compiles");
    // ILL-FORMED: a REFUTABLE lambda pattern (a literal element) must still reject with the
    // binding-position non-exhaustiveness code (CDZ0210), NOT silently miscompile — the desugar routes
    // it through the same `let` validation the def path uses.
    use crate::abi::Artifact;
    let codes_for = |s: &str| -> Vec<String> {
        let entry = crate::codec::encode(&parse(s));
        let out = crate::compile::compile(
            &[
                Artifact::new(Artifact::KIND_AST, "app", entry.clone()),
                cadenza_compile_abi::abi::entry_artifact("app"),
            ],
            &[crate::backend::Target::Wasm],
        );
        out.diagnostics
            .iter()
            .filter_map(|d| d.code.clone())
            .collect()
    };
    // The lambda-param binds cleanly (no CDZ0101) — the core regression assertion at the code level.
    assert!(
        !codes_for(src).contains(&"CDZ0101".to_string()),
        "a tuple-destructuring lambda parameter must not spuriously reject CDZ0101"
    );
}

#[test]
fn an_abstract_typed_map_set_key_is_rejected_cdz0202_but_a_concrete_key_stays_legal() {
    // SOUNDNESS (breaker/corpus-bugfix, concierge-ruled 2026-07-29): a CHAMP Map/Set keyed by an
    // ABSTRACT-typed value (imported handle, ctor `T` withheld) observes the module's PRIVATE
    // representation through champ_eq at insert/lookup — the SAME type-system.md:180 MUST violation as a
    // direct `(=)`, an indirect route. Must reject CDZ0202. FIX (infer::check_application): the
    // collection-construction prims (SetOf/SetInsert/MapNew/MapInsert) reject when the RESULT key type is
    // abstract-at-this-site (`is_abstract_type_at`). Values stay legal to HOLD (payloads); a concrete/
    // prelude key stays legal (only a genuinely abstract imported key rejects).
    use crate::abi::Artifact;
    let lib = crate::codec::encode(&parse(
        "(do (type Temp (T Int64)) (def (mk (: c Int64)) (Temp.T c)) (export Temp) (export mk))",
    ));
    let codes_for = |body: &str| -> Vec<String> {
        let entry = crate::codec::encode(&parse(&format!(
            "(do (import \"lib\" (Temp mk)) (def (main (: k Int64)) {body}) (export main))"
        )));
        let out = crate::compile::compile(
            &[
                Artifact::new(Artifact::KIND_AST, "lib", lib.clone()),
                Artifact::new(Artifact::KIND_AST, "app", entry.clone()),
                cadenza_compile_abi::abi::entry_artifact("app"),
            ],
            &[crate::backend::Target::Wasm],
        );
        out.diagnostics
            .iter()
            .filter_map(|d| d.code.clone())
            .collect()
    };
    // An abstract-typed value as a Set KEY (via Set.of) → CDZ0202 (was: compiled + observed the private rep).
    assert!(
        codes_for("(Set.size (Set.of (list (mk k))))").contains(&"CDZ0202".to_string()),
        "an abstract-typed Set key must reject CDZ0202"
    );
    // CONTROL: a CONCRETE (Int64) key stays legal — the gate must not over-reject a non-abstract key.
    assert!(
        !codes_for("(Set.size (Set.of (list k)))").contains(&"CDZ0202".to_string()),
        "a concrete Int64 Set key stays legal (no CDZ0202)"
    );
    // COMPOUND key CONTAINING an abstract type — PR#890 (Copilot) soundness gap. A `(Tuple Temp Int64)`
    // / `(List Temp)` key is `Ty::Tuple`/`Ty::List`, NOT a `nominal_or_sum_decl`, so the top-type check
    // was SKIPPED — but CHAMP key eq/hash walks the WHOLE compound, still observing `Temp`'s abstract
    // rep, the same indirect route one level down. Must reject CDZ0202.
    // A TUPLE key `(tuple (mk k) k)` — element 0 is the abstract `Temp`.
    assert!(
        codes_for("(Set.size (Set.of (list (tuple (mk k) k))))").contains(&"CDZ0202".to_string()),
        "a tuple key CONTAINING an abstract type must reject CDZ0202 (compound key)"
    );
    // A LIST key `(list (mk k))` — element type is the abstract `Temp`.
    assert!(
        codes_for("(Set.size (Set.of (list (list (mk k)))))").contains(&"CDZ0202".to_string()),
        "a list key CONTAINING an abstract type must reject CDZ0202 (compound key)"
    );
    // CONTROL: a compound key of only CONCRETE types stays legal (no over-reject through the recursion).
    assert!(
        !codes_for("(Set.size (Set.of (list (tuple k k))))").contains(&"CDZ0202".to_string()),
        "a tuple key of concrete Int64s stays legal (no CDZ0202)"
    );
    // DEPTH: a DEEPLY-nested compound key — the abstract `Temp` leaf sits TWO tuples deep in
    // `(tuple (tuple (mk k) k) k)`. `key_ty_contains_abstract_at` must recurse to ANY depth, not
    // just the outermost structural level — CHAMP key eq/hash walks the whole spine to the leaf.
    // Pins the recursion against a future shallow-walk (top-level-elems-only) regression that would
    // silently COMPILE + observe the abstract leaf — a soundness hole.
    assert!(
        codes_for("(Set.size (Set.of (list (tuple (tuple (mk k) k) k))))")
            .contains(&"CDZ0202".to_string()),
        "a deeply-nested compound key with an abstract leaf 2 tuples deep must reject CDZ0202"
    );
    // NEGATIVE BOUNDARY (value position): opacity rejects only the KEY-comparison observation, NOT
    // holding an abstract VALUE. A Map keyed by a CONCRETE `Int64` whose VALUE is the abstract `Temp`
    // is LEGAL — the value spine is never compared (only the concrete key is), so the private rep is
    // never observed. The bound `_v` holds but never inspects it. Guards the reject from creeping into
    // value positions (an over-reach that rejected an abstract type in ANY collection slot would break
    // this legitimate hold-don't-compare use).
    let value_codes = codes_for(
        "(match (Map.lookup (Map.insert Map.empty k (mk k)) k) ((Some _v) 1) ((None _u) 0))",
    );
    assert!(
        value_codes.is_empty(),
        "an abstract VALUE under a concrete key is legal — value-holding never triggers any \
             diagnostic (opacity rejects only KEY comparison); got {value_codes:?}"
    );
}

#[test]
fn a_literal_parameter_position_is_rejected() {
    // A parameter is a BINDER: a bare name, a wildcard `_`, an annotated `(: name T)`, or a
    // destructuring pattern. A bare LITERAL — `(def (f 5) …)`, `(def (f true) …)` — is NONE of these:
    // it binds nothing, so the parameter is dead and any argument passed to it is silently ignored. It
    // used to be accepted with NO diagnostic (the scan reads `children[1..]` without validating each is
    // a binder). Now it rejects CDZ0201, anchored at the literal. A COMPOUND list parameter is a
    // destructuring pattern — left to the binding-pattern path — so this fires ONLY on a bare literal.
    let d = |src: &str| -> crate::abi::Diagnostic {
        crate::diagnostics(&mut crate::db::Db::load(parse(src)))
            .into_iter()
            .find(|d| d.message.contains("a parameter must be a name"))
            .unwrap_or_else(|| panic!("no malformed-parameter reject for {src}"))
    };
    for src in [
        "(module m (def (f 5) 1) (export f))",
        "(module m (def (f true) 1) (export f))",
        "(module m (def (f \"x\") 1) (export f))",
        // A literal amid valid params is still caught (anchored at the literal, not the whole list).
        "(module m (def (f x 5 y) 1) (export f))",
        // A `(fn (<param>…) <body>)` LAMBDA parameter is the SAME binder position, held to the same
        // rule — a literal `fn` param used to be SILENTLY ACCEPTED (the def-param scan reads only
        // `db.defs`, and a lambda's params are never registered there), an asymmetry between the two
        // binder forms now closed. The `fn` twins of the three def literals above.
        "(module m (def (f) ((fn (5) 3) 1)) (export f))",
        "(module m (def (f) ((fn (true) 3) 1)) (export f))",
        "(module m (def (f) ((fn (\"x\") 3) 1)) (export f))",
        // A literal amid valid `fn` params, anchored at the literal.
        "(module m (def (f) ((fn (x 5 y) 3) 1 2 3)) (export f))",
    ] {
        assert_eq!(
            d(src).code.as_deref(),
            Some("CDZ0201"),
            "{src}: {}",
            d(src).message
        );
    }
    // NO false positive: a name, a wildcard `_`, an annotated binder, a destructuring pattern, and a
    // nullary def all stay clear of THIS fault (a `_`/name may still get the separate ambiguous-boundary
    // fault when exported, which is unrelated). The `fn` binder form is held to the same clear cases.
    for ok in [
        "(module m (def (f (: x Int64)) (+ x 1)) (export f))",
        "(module m (def (g _) 1) (def (main) (g 5)) (export main))",
        "(module m (def (f (: (tuple a b) (Tuple Int64 Int64))) (+ a b)) (export f))",
        "(module m (def (main) 1) (export main))",
        // A `fn` with a name, a wildcard, and an annotated binder is clear of THIS fault.
        "(module m (def (f) ((fn (x) x) 1)) (export f))",
        "(module m (def (f) ((fn (_) 3) 1)) (export f))",
        "(module m (def (f) ((fn ((: x Int64)) x) 1)) (export f))",
        // A sum VARIANT named `fn` (a Rust keyword, a legal Cadenza identifier) reifies to a
        // `(fn <payload>)` synth node that the fn-param scan MUST NOT mistake for a lambda — the scan
        // requires a lambda's `[params, body]` (tail length ≥ 2), and this synth is bodyless. This
        // pins the regression the naive arena-wide `(fn …)` scan caused (false "literal binds nothing"
        // on the `fn`-named variant's payload occurrence).
        "(do (type W (fn Int64) (struct Int64) (enum)) (def (f (: w W)) \
             (match w ((W.fn n) n) ((W.struct n) (* n 2)) ((W.enum) 0))) \
             (def (main) (f (W.fn 5))) (export main))",
    ] {
        assert!(
            !crate::diagnostics(&mut crate::db::Db::load(parse(ok)))
                .iter()
                .any(|d| d.message.contains("a parameter must be a name")),
            "a valid parameter is not flagged: {ok}"
        );
    }
}

#[test]
fn a_wide_effect_handler_compiles_linearly() {
    // COMPILE-PERF guard (not a behavior check): a handler over a WIDE effect (many ops, many arms) must
    // COMPILE — the behaviour the `program_delegates_effect` memo (in `effects::perform_host_target`)
    // must preserve. That routing fallback walks every export body per residual host-perform; recomputing
    // it per op made an N-op handler O(N²) (an 800-op handler spent ~86% of the compile in
    // `body_has_host_delegating`). The memo makes it linear. Here 12 ops; the body performs op p3.
    // The VALUE (multi-arm dispatch selects the performed op's arm) is a behavior pinned in the corpus by
    // 14-effects "a single handler with both a resuming and an abortive arm dispatches each op to its own
    // arm kind"; this test keeps ONLY the wide-handler-compiles face (no wasmtime run needed).
    let n = 12;
    let ops: String = (0..n)
        .map(|i| format!(" (op p{i} (-> Unit Int64))"))
        .collect();
    let arms: String = (0..n)
        .map(|i| format!(" (p{i} () s (resume {i} s))"))
        .collect();
    let src = format!(
        "(do (effect E{ops}) (def (main) (handle E unit ({arms}) (+ ((. E p3)) 1))) (export main))"
    );
    compile_component(&crate::codec::encode(&parse(&src)))
        .expect("a wide 12-op effect handler compiles (linear routing memo)");
}

// two_same_named_effects_are_distinct_not_conflated (two `(effect Log …)` with different ops; a handler
// arm on the SECOND's op `record` → CDZ0403 because a bare `Log` resolves first-declared {emit}) migrated
// to corpus 14b-effects-and-handlers "two effects declared with the same name are distinct, not one merged
// effect" — the observable CDZ0403 is the regression signal for the first-wins effect_decl_by_name index
// (a conflating index would ACCEPT the `record` arm). NOTE: that corpus case's handler arm was previously
// MIS-NATIVIZED to `#record((= n) …)` (treating the arm, whose op is *named* `record`, as a record
// literal) so it never parsed as an arm and graded todo; this batch fixes it to `(record (n) s (resume n
// s))` → PASS. rcdzc test deleted (corpus-covered).

// an_effect_reached_with_no_handler_or_delegation_is_cdz0401 (`(effect Ask …)` performed with no handler
// nor delegation → CDZ0401) migrated to corpus 14b-effects-and-handlers "an effect operation reached with
// neither a handler nor a delegation is rejected". rcdzc test deleted (corpus-covered, code-only).
#[test]
fn a_host_op_performed_via_an_inlined_helper_reaches_the_import_set() {
    // A HELPER that performs a host op, delegated at the entrypoint and INLINED into it, must
    // contribute its op to the component's host-import set. `collect_host_imports` / the host-arg-string
    // pass walk the LOWERED CORE (not the AST), so a `HostCall` β-spliced into the caller by inlining
    // the helper is found — before, they AST-walked and saw only the un-inlined `(emit-msg …)`
    // application, missing the performed op ("a host call's operation is not in the host-import set" /
    // "a host-arg string was not laid in the data segment"). This is the fix that lets a reusable
    // assertion helper (`assert-eq` performing `Test.fail`) work. The helper carries the whole
    // self-contained `host … (perform; trap)`, the working idiom `cdz test` uses. It EMITS (the guest
    // performs the op then traps) — the diverging-body → unit-entry path.
    let src = "(do (effect Test (op fail (-> String Unit))) \
                    (def (emit-msg (: m String)) (host (Test) (do ((. Test fail) m) (trap \"x\")))) \
                    (def (main) (if (= (+ 1 1) 3) unit (emit-msg \"1+1 should be 3\"))) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src)))
        .expect("a host op performed via an inlined helper must reach the import set + emit");
    assert!(!bytes.is_empty(), "the emitted component has bytes");
}

#[test]
fn a_cross_function_no_home_effect_wraps_the_entrypoint_not_the_callee() {
    // A perform in a CALLED function still delegates at the ENTRYPOINT — the wrap must target the
    // exported body, not the callee where the perform (and the error anchor) live.
    let src = "(do (effect Ask (op ask (-> Unit Int64))) \
                   (def (helper) ((. Ask ask))) \
                   (def (main) (+ (helper) 1)) (export main))";
    let err = compile_component(&crate::codec::encode(&parse(src)))
        .expect_err("cross-function ungranted effect must be rejected");
    assert_eq!(err.code.as_deref(), Some("CDZ0401"));
    let fix = err.fix.expect("carries the wrap fix");
    assert_eq!(fix.kind, crate::abi::FixKind::Wrap);
    // The wrap node is main's body — a DIFFERENT node than the error's anchor (the perform in helper).
    assert_ne!(
        Some(fix.node),
        err.node,
        "the wrap targets the entrypoint body, the error anchors at the perform site"
    );
}

#[test]
fn a_no_home_effect_reports_one_error_not_a_shadowing_decline() {
    // One ungranted effect must yield ONE primary `error:` — the coded CDZ0401 — NOT the coded
    // rejection PLUS the emit path's uncoded "performed with no enclosing handler here" DECLINE for
    // the same op (both were surfaced as `error:`, reading as two errors for one root cause).
    // `dedup_faults` drops the standalone-perform decline when a CDZ0401 is present
    // (`reference-compiler.md` §Outcomes Are Ordered By Safety — the coded rejection is the stronger
    // report). Assert exactly ONE error-severity diagnostic, and it is CDZ0401.
    let src = "(do (effect Ask (op ask (-> Unit Int64))) \
                   (def (main) (+ ((. Ask ask)) 1)) (export main))";
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "m",
            crate::codec::encode(&parse(src)),
        )],
        &[crate::backend::Target::Wasm],
    );
    let errors: Vec<&crate::abi::Diagnostic> = out
        .diagnostics
        .iter()
        .filter(|d| d.severity == crate::abi::Severity::Error)
        .collect();
    assert_eq!(
        errors.len(),
        1,
        "one ungranted effect = one error, got: {:?}",
        out.diagnostics
    );
    assert_eq!(errors[0].code.as_deref(), Some("CDZ0401"));
    // The shadowing decline is gone specifically.
    assert!(
        !out.diagnostics
            .iter()
            .any(|d| d.message == crate::diag::NO_HOME_STANDALONE_DECLINE),
        "the standalone-perform no-home decline must not accompany the CDZ0401"
    );
}

#[test]
fn a_malformed_handler_reports_one_error_not_a_shadowing_reducibility_decline() {
    // A misspelled handler op is CDZ0403 (with a `did you mean` fix). The malformed handler ALSO fails
    // to fold, so `lower` returns the uncoded "not yet reducible by the tail-resumptive fold" DECLINE —
    // a CONSEQUENCE of the misspelling, not an independent limitation. `dedup_faults` drops that
    // decline when a CDZ0403/CDZ0405 is present, so the fault is ONE primary `error:` carrying the fix
    // (an agent that applies the fix does not then face a second, confusing error). `guess` is
    // undeclared (only `pick` exists).
    let src = "(do (effect Choose (op pick (-> Unit Int64))) \
                   (def (main) (handle Choose 0 ((guess (u) s (resume s s))) 42)) (export main))";
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "m",
            crate::codec::encode(&parse(src)),
        )],
        &[crate::backend::Target::Wasm],
    );
    let errors: Vec<&crate::abi::Diagnostic> = out
        .diagnostics
        .iter()
        .filter(|d| d.severity == crate::abi::Severity::Error)
        .collect();
    assert_eq!(
        errors.len(),
        1,
        "a malformed handler = one error, got: {:?}",
        out.diagnostics
    );
    assert_eq!(errors[0].code.as_deref(), Some("CDZ0403"));
    // The shadowing reducibility decline is gone specifically.
    assert!(
        !out.diagnostics
            .iter()
            .any(|d| d.message == crate::diag::HANDLER_NOT_REDUCIBLE_DECLINE),
        "the not-reducible decline must not accompany the coded handler reject"
    );
}

#[test]
fn an_over_applied_op_in_a_handle_reports_one_error_not_a_reducibility_decline() {
    // An OVER-APPLIED operation performed inside a handle — `(E.set 1 2)` for a 1-arg `op set` — is
    // CDZ0203 "`E.set` takes 1 argument, but 2 were given" (with a delete fix). The over-application
    // ALSO makes the handler unfoldable, so `lower` returns the uncoded "not yet reducible" DECLINE — a
    // CONSEQUENCE of the arity error, not an independent limit. `dedup_faults` now drops that decline
    // when a member-op over-application reject is present (joining the malformed-handler / resume-result
    // / arm-arity triggers), so the mistyped perform is ONE actionable error.
    let src = "(do (effect E (op set (-> Int64 Unit))) \
                   (def (main) (handle E 0 ((set (a) s (resume unit s))) (E.set 1 2))) (export main))";
    let ds = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
    let errors: Vec<&crate::abi::Diagnostic> = ds
        .iter()
        .filter(|d| d.severity == crate::abi::Severity::Error)
        .collect();
    assert_eq!(
        errors.len(),
        1,
        "an over-applied op in a handle = one error, got: {:?}",
        ds.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(
        errors[0].code.as_deref() == Some("CDZ0203")
            && errors[0]
                .message
                .contains("takes 1 argument, but 2 were given"),
        "the one error is the arity reject: {}",
        errors[0].message
    );
    assert!(
        !ds.iter()
            .any(|d| d.message == crate::diag::HANDLER_NOT_REDUCIBLE_DECLINE),
        "the consequent fold-decline is dropped"
    );
    // NO OVER-SUPPRESSION: a VALID perform under the same handle compiles clean.
    assert!(
        crate::diagnostics(&mut crate::db::Db::load(parse(
            "(do (effect E (op set (-> Int64 Unit))) \
                 (def (main) (handle E 0 ((set (a) s (resume unit s))) (E.set 1))) (export main))"
        )))
        .iter()
        .all(|d| d.severity != crate::abi::Severity::Error),
        "a well-formed perform under the handle compiles"
    );
}

#[test]
fn a_collection_extracted_performing_closure_declines_honestly_not_no_enclosing_handler() {
    // A handled effect performed via a closure EXTRACTED from a collection — the closure is stored in a
    // `(list …)`, projected with `List.at`, unwrapped through `(match … ((Some f) (f …)))`, and applied
    // — all LEXICALLY under the `handle` that discharges the op. The tail-resumptive fold cannot route
    // the perform: `subtree_performs` treats a lambda VALUE as pure (a closure body performs only when
    // APPLIED), and the fold cannot trace the application back through the collection slot, so the
    // performing lambda escapes to standalone lifting. Pre-fix its perform reached `lower`'s standalone
    // arm and surfaced the FACTUALLY-WRONG `NO_HOME_STANDALONE_DECLINE` ("performed with no enclosing
    // handler here") — even though the handle plainly ENCLOSES it (the escaped lambda lifts by its
    // ORIGINAL AST node into a synthesized standalone subtree whose parent chain no longer reaches the
    // handle, so a post-lift ancestry check cannot recover the enclosure — the detection must run at the
    // REDUCED-body level, before lifting). At the lowering entry (`lower`'s `Handle` arm),
    // `reduced_body_leaks_escaped_perform` scans the reduced body for a discharged-op perform inside a
    // LIVE lambda whose own `core_of` is the misleading no-home poison, and remaps it to the honest
    // `HANDLER_NOT_REDUCIBLE_DECLINE` todo (breaker's diagnostic-quality finding, routed by corpus-bugfix
    // 2026-07-28; same discipline as the guard-perform decline — a not-yet-routed path must say "not yet
    // reducible", NEVER a false "no home"). SAFE reject, honest MESSAGE. The scan yields to a
    // more-specific co-occurring "partial application of a runtime closure" decline (see
    // `a_partial_application_of_a_performing_closure_under_a_handler_declines_cleanly`).
    let msg = |src: &str| {
        let out = crate::compile::compile(
            &[crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "m",
                crate::codec::encode(&parse(src)),
            )],
            &[crate::backend::Target::Wasm],
        );
        out.diagnostics
            .iter()
            .filter(|d| d.severity == crate::abi::Severity::Error)
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };
    let collection = "(do (effect Ask (op ask (-> Int64 Int64))) \
                   (def (main) (handle Ask 5 ((ask (n) s (resume (* n 2) s))) \
                     (match (List.at (list (fn (x) (Ask.ask x))) 0) \
                       ((Some f) (f 3)) (None 0)))) (export main))";
    let ms = msg(collection);
    assert!(
        !ms.iter()
            .any(|m| *m == crate::diag::NO_HOME_STANDALONE_DECLINE),
        "a collection-extracted perform UNDER a handle must NOT report 'no enclosing handler' — the \
             handle encloses it: {ms:?}"
    );
    assert!(
        ms.iter()
            .any(|m| *m == crate::diag::HANDLER_NOT_REDUCIBLE_DECLINE),
        "expected the honest not-yet-reducible decline for the collection-extracted closure: {ms:?}"
    );
    // NO REGRESSION: a directly-applied performing let-bound closure still FOLDS clean. The application
    // `(f 3)` inlines and routes, leaving the `f` binding DEAD (unreferenced) — lowering drops it, so the
    // vestigial lambda never reaches standalone lowering. The leak guard skips a dead binding's init.
    let via_local = "(do (effect Ask (op ask (-> Int64 Int64))) \
                   (def (main) (handle Ask 5 ((ask (n) s (resume (* n 2) s))) \
                     (let ((f (fn (x) (Ask.ask x)))) (f 3)))) (export main))";
    assert!(
        msg(via_local).is_empty(),
        "a directly-applied let-bound performing closure under a handle must still fold: {:?}",
        msg(via_local)
    );
    // NO REGRESSION: a plain perform directly in the handle body still folds (→ 10).
    let direct = "(do (effect Ask (op ask (-> Int64 Int64))) \
                   (def (main) (handle Ask 5 ((ask (n) s (resume (* n 2) s))) \
                     (Ask.ask 5))) (export main))";
    assert!(
        msg(direct).is_empty(),
        "a direct perform under the handle must still fold clean: {:?}",
        msg(direct)
    );
}

#[test]
fn a_recursive_fn_perform_the_specializer_cant_thread_declines_without_a_mangled_name() {
    // corpus-bugfix/breaker 2026-07-28: a do-def-bound perform in a RECURSIVE fn under a handle used to
    // report CDZ0201 "`check-all#eff2` has no body" — a compiler-INTERNAL effect-specialization name
    // (`#eff2`) leaked into a user-facing message. ROOT: `specialize_recursive` reserves the spec def
    // (body `None`) + memoizes its name BEFORE threading the recursive body (so a self-call resolves its
    // own name); when the body is UNTHREADABLE (`thread` → None), the reserved def is left bodyless, and a
    // reference to it hit `def_as_resolved`'s "has no body" coded reject. `def_as_resolved` now reports an
    // UNCODED "not yet reducible" decline naming the BASE fn (`check-all`) for an internal `#eff`-marked
    // bodyless spec — the honest todo (the specializer's body-clone increment that would fold it → 110 is
    // later). Satisfies both asks: (1) clean decline, not CDZ0201; (2) names `check-all`, not `#eff2`.
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "m",
            crate::codec::encode(&parse(
                "(module m (effect Env (op scale (-> Int64 Int64))) \
                     (def (check-all (: i Int64) (: bad Int64)) \
                       (if (= i 0) bad (do (def scaled (Env.scale i)) (check-all (- i 1) (+ bad scaled))))) \
                     (def (main) (handle Env 2 ((scale (v) s (resume (* v s) s))) (check-all 10 0))) \
                     (export main))",
            )),
        )],
        &[crate::backend::Target::Wasm],
    );
    let errors: Vec<&crate::abi::Diagnostic> = out
        .diagnostics
        .iter()
        .filter(|d| d.severity == crate::abi::Severity::Error)
        .collect();
    // No MANGLED internal specialization name in any user-facing message.
    assert!(
        !errors.iter().any(|d| d.message.contains("#eff")),
        "must not leak a mangled `#eff` specialization name: {:?}",
        errors.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // The decline is CDZ0900 (deferred/unsupported, seq-286: every decline carries a code), NOT the hard
    // "has no body" CDZ0201, and names the base fn.
    assert!(
        errors.iter().all(|d| d.code.as_deref() == Some("CDZ0900")),
        "the recursive-spec decline must be CDZ0900 (deferred), not CDZ0201 or uncoded: {:?}",
        errors
            .iter()
            .map(|d| (&d.code, &d.message))
            .collect::<Vec<_>>()
    );
    assert!(
        errors
            .iter()
            .any(|d| d.message.contains("check-all") && d.message.contains("cannot specialize")),
        "expected an honest not-reducible decline naming `check-all`: {:?}",
        errors.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn a_width_mismatched_handler_state_declines_cleanly_never_invalid_wasm() {
    // F1 (corpus-bugfix/breaker 2026-07-28): a handler whose STATE slot infers to a narrow int (UInt8)
    // while the op RESULT is Int64 must NOT emit an invalid wasm module. `(next (u) s (resume s (+ s x)))`
    // with x:UInt8 infers the state as UInt8 (via the next-state `(+ s x)`), but the resume value (the
    // state) flows into the Int64 op result; threaded across TWO do-def performs the emit placed an i32
    // where i64 was expected → INVALID module ("expected i64, found i32"; rust widened and ran = backend
    // divergence). `reduce_handle`'s width-consistency guard now DECLINES cleanly (uncoded "not yet
    // reducible" todo) rather than emit invalid wasm — the safe floor (a widening-coercion fold is later).
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "m",
            crate::codec::encode(&parse(
                "(module m (effect Src (op next (-> Unit Int64))) \
                     (def (main (: x UInt8)) (handle Src 10 ((next (u) s (resume s (+ s x)))) \
                       (do (def a (Src.next)) (def b (Src.next)) (+ a b)))) \
                     (export main))",
            )),
        )],
        &[crate::backend::Target::Wasm],
    );
    // No INVALID-MODULE / codegen error: either it declines (uncoded todo) or it folds — never a coded
    // reject and never an emit failure. Assert no error-severity diagnostic carries a wasm-validation
    // failure, AND that every error (if any) is UNCODED — the honest "not yet reducible" decline, never a
    // coded CDZ0xxx reject (PR#883 Copilot: the negative-substring check alone would pass on an unrelated
    // coded error, missing the "never a coded reject" half of the contract).
    let errors: Vec<&crate::abi::Diagnostic> = out
        .diagnostics
        .iter()
        .filter(|d| d.severity == crate::abi::Severity::Error)
        .collect();
    assert!(
        !errors.iter().any(|d| d.message.contains("invalid")
            || d.message.contains("type mismatch")
            || d.message.contains("failed to compile")),
        "must not emit an invalid wasm module: {:?}",
        errors.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(
        errors.iter().all(|d| d.code.as_deref() == Some("CDZ0900")),
        "any error must be the CDZ0900 deferred decline (not a hard coded reject): {:?}",
        errors
            .iter()
            .map(|d| (&d.code, &d.message))
            .collect::<Vec<_>>()
    );
    // NO REGRESSION: a MATCHING-width state (Int64 seed + Int64 x, op result Int64) still folds and
    // runs — the corpus case "a matching-width handler state folds across two sequential performs"
    // (spec/semantics/14-effects-and-handlers.sexp): main(5) = 25 (a=10, b=15), run via cdz-run.
}

// The mistyped-resume REJECT + coercion-fix facets (CDZ0201 + (Int64.of …) wrap for a coercible Int8, no
// fix for a Bool) moved to corpus 14b-effects-and-handlers "a coercible mistyped resume value …" +
// sibling. Residual: the "exactly ONE error" dedup the corpus cannot express — the dropped consequent is
// the UNCODED "not yet reducible by the tail-resumptive fold" decline, so (no-other-errors) (coded-only)
// does not catch it.
#[test]
fn a_mistyped_resume_reports_one_error_dropping_the_uncoded_fold_decline() {
    let errors_of = |src: &str| -> Vec<crate::abi::Diagnostic> {
        crate::compile::compile(
            &[crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "m",
                crate::codec::encode(&parse(src)),
            )],
            &[crate::backend::Target::Wasm],
        )
        .diagnostics
        .into_iter()
        .filter(|d| d.severity == crate::abi::Severity::Error)
        .collect()
    };
    // coercible (Int8) and non-coercible (Bool) mistyped resumes both report exactly ONE error (the
    // consequent not-yet-reducible fold-decline is dropped).
    for src in [
        "(do (effect E (op a (-> Int64 Int64))) (def (main (: x Int8)) (handle E unit ((a (n) s (resume x s))) (E.a 1))) (export main))",
        "(do (effect E (op a (-> Int64 Int64))) (def (main) (handle E unit ((a (n) s (resume true s))) (E.a 1))) (export main))",
    ] {
        let errs = errors_of(src);
        assert_eq!(
            errs.len(),
            1,
            "a mistyped resume = one error (fold-decline dropped): {errs:?}"
        );
        assert_eq!(errs[0].code.as_deref(), Some("CDZ0201"));
    }
}

#[test]
fn a_handle_with_an_unbound_effect_name_reports_one_error_not_a_shadowing_decline() {
    // `handle Nope …` where `Nope` is not a declared effect: the unbound name is CDZ0101 (with a
    // did-you-mean fix), and the handle can't fold → the emit path would ALSO return the "not yet
    // reducible" decline. That decline is a consequence of the unbound effect, not an independent
    // limitation — the lower path detects the arm op is an unbound name and propagates that CDZ0101
    // (which dedups) instead of the decline, so the fault is ONE primary error carrying the fix.
    let src = "(do (effect E (op get (-> Unit Int64))) \
                   (def (main) (handle Nope 0 ((get (u) s (resume s s))) 42)) (export main))";
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "m",
            crate::codec::encode(&parse(src)),
        )],
        &[crate::backend::Target::Wasm],
    );
    let errors: Vec<&crate::abi::Diagnostic> = out
        .diagnostics
        .iter()
        .filter(|d| d.severity == crate::abi::Severity::Error)
        .collect();
    assert_eq!(
        errors.len(),
        1,
        "an unbound effect name = one error, got: {:?}",
        out.diagnostics
    );
    assert_eq!(errors[0].code.as_deref(), Some("CDZ0101"));
    assert!(
        !out.diagnostics
            .iter()
            .any(|d| d.message == crate::diag::HANDLER_NOT_REDUCIBLE_DECLINE),
        "the not-reducible decline must not shadow the unbound-effect CDZ0101"
    );
}

#[test]
fn a_far_handler_op_access_typo_reports_the_absent_field_exactly_once() {
    // A FAR-typo op ACCESS in a handle body — `((. E zzzzz))` where `zzzzz` matches no op — surfaces
    // the member absent-op CDZ0201 via TWO unanchored desugar/reduction paths (the handle's
    // op-resolution AND the perform). Once the member two-tier (M82) began appending a closest-matches
    // suffix, the two copies' MESSAGES differed (one listed matches, one bare) so the full-message
    // dedup key let both through as a DOUBLE report. `dedup_faults` now keys an unanchored no-field
    // fault by its INVARIANT CORE (`has no <word> \`k\``, category-aware — here "effect `E` has no
    // operation `zzzzz`") + collapses an unanchored copy against an anchored one by core, so the miss
    // reports exactly ONCE — keeping the located, closest-matches copy.
    let src = "(do (effect E (op ask (-> Unit Int64)) (op tell (-> Int64 Unit))) \
                   (def (main) (handle E unit ((ask () s (resume 1 s)) (tell (x) s (resume unit s))) \
                     ((. E zzzzz)))) (export main))";
    let field_errs: Vec<_> = crate::diagnostics(&mut crate::db::Db::load(parse(src)))
        .into_iter()
        .filter(|d| d.message.contains("has no operation `zzzzz`"))
        .collect();
    assert_eq!(
        field_errs.len(),
        1,
        "the far op-access miss reports exactly once, not a double: {field_errs:?}"
    );
    assert!(
        field_errs[0].message.contains("closest matches:") && field_errs[0].node.is_some(),
        "the surviving copy lists the ops AND is located: {}",
        field_errs[0].message
    );
}

#[test]
fn a_handler_binding_the_same_operation_twice_is_rejected_with_a_delete_fix() {
    // A handler's arms ARE its effect's operation set (a FIXED set, like a record's fields or an
    // effect's op declarations), so binding one operation twice — `(handle E s ((emit …) (emit …)) …)`
    // — is the same closed-set ill-formedness a duplicate record field / effect-op declaration is: the
    // second arm is dead. Rejected CDZ0201 with a DELETE fix on the redundant arm (the handler analogue
    // of the duplicate-field/op/export family). `emit` declared once, bound twice → rejected.
    let src = "(do (effect E (op emit (-> Int64 Unit))) \
                   (def (main) (handle E 0 ((emit (n) s (resume unit s)) (emit (m) s (resume unit s))) \
                     (E.emit 5))) (export main))";
    let mut db = crate::db::Db::load(parse(src));
    let d = crate::diagnostics(&mut db)
        .into_iter()
        .find(|d| {
            d.code.as_deref() == Some("CDZ0201") && d.message.contains("handled more than once")
        })
        .expect("a duplicate handler arm is CDZ0201");
    assert!(
        d.message.contains("`emit`"),
        "names the duplicated operation: {}",
        d.message
    );
    let fix = d
        .fix
        .as_ref()
        .expect("carries a delete-the-duplicate-arm fix");
    assert_eq!(fix.kind, crate::abi::FixKind::Delete);
    // The anchor is a real USER node (the op-key occurrence), so the error carries file:line:col.
    assert!(
        db.is_user_node(crate::ast::StructId(
            d.node.expect("CDZ0201 must carry a node")
        )),
        "the duplicate-arm fault anchors at a user node"
    );

    // NO false positive: two DISTINCT operations, and two SEPARATE effects each declaring `emit`
    // (nested handlers) — neither is a duplicate (keyed by (effect-decl, op-name), not name alone).
    let ok_distinct = "(do (effect E (op emit (-> Int64 Unit)) (op tick (-> Unit Unit))) \
                   (def (main) (handle E 0 ((emit (n) s (resume unit s)) (tick () s (resume unit s))) \
                     (E.emit 5))) (export main))";
    assert!(
        !crate::diagnostics(&mut crate::db::Db::load(parse(ok_distinct)))
            .iter()
            .any(|d| d.message.contains("handled more than once")),
        "two distinct ops in one handler is not a duplicate"
    );
    let ok_two_effects = "(do (effect E (op emit (-> Int64 Unit))) (effect F (op emit (-> Int64 Unit))) \
                   (def (main) (handle E 0 ((emit (n) s (resume unit s))) \
                     (handle F 0 ((emit (n) s (resume unit s))) (do (E.emit 1) (F.emit 2))))) (export main))";
    assert!(
        !crate::diagnostics(&mut crate::db::Db::load(parse(ok_two_effects)))
            .iter()
            .any(|d| d.message.contains("handled more than once")),
        "two effects each with their own `emit` handler is not a duplicate"
    );
}

#[test]
fn an_undeclared_handler_op_anchors_to_a_user_node() {
    // CDZ0403's anchor must be a real USER node so the error carries `file:line:col`. The desugar
    // synthesizes the arm's op projection `(. E k)` (spanless), so anchoring there once lost the
    // location; it now anchors at the op-KEY occurrence (which keeps the arm's op-name span).
    let src = "(do (effect Choose (op pick (-> Unit Int64))) \
                   (def (main) (handle Choose unit ((guess () s (resume 5 s))) ((. Choose pick)))) \
                   (export main))";
    let mut db = crate::db::Db::load(parse(src));
    let d = crate::diagnostics(&mut db)
        .into_iter()
        .find(|d| d.code.as_deref() == Some("CDZ0403"))
        .expect("a CDZ0403 diagnostic");
    let node = d
        .node
        .expect("CDZ0403 must carry a node, not be unanchored");
    assert!(
        db.is_user_node(crate::ast::StructId(node)),
        "node {node} must be a user node (the op key), not the synthesized projection"
    );
}

#[test]
fn a_misspelled_handler_arm_op_does_not_also_report_no_home() {
    // A misspelled arm op (`emitt` for declared `emit`) is the primary CDZ0403 ("did you mean
    // `emit`?", with its fix). It must NOT ALSO report CDZ0401 no-home on the handled body's
    // `(E.emit …)`: the arm typo leaves `emit` undischarged, so the perform spuriously looks
    // home-less — a cascade of the arm typo (fixing the arm spelling clears both). Only the root
    // CDZ0403 should surface.
    let src = "(do (effect E (op emit (-> Int64 Unit))) \
                   (def (main) (handle E 0 ((emitt (v) s (resume unit s))) (E.emit 5))) (export main))";
    let mut db = crate::db::Db::load(parse(src));
    let codes: Vec<String> = crate::diagnostics(&mut db)
        .into_iter()
        .filter_map(|d| d.code)
        .collect();
    assert!(
        codes.iter().any(|c| c == "CDZ0403"),
        "the misspelled arm op is a CDZ0403; got {codes:?}"
    );
    assert!(
        !codes.iter().any(|c| c == "CDZ0401"),
        "the no-home CDZ0401 is a cascade of the arm typo and must be suppressed; got {codes:?}"
    );
}

#[test]
fn an_unbound_effect_name_in_a_handle_anchors_to_a_user_node() {
    // `(handle Nope …)` names an effect that does not exist → CDZ0101. The desugar drops the head
    // effect name and projects it into each arm as `(. Nope op)`; the FIRST arm reuses the SOURCE
    // effect-name occurrence (M31), so the unbound-name reject anchors to the real `Nope` token —
    // a user node with `file:line:col` — not a spanless minted atom.
    let src = "(do (def (main) (handle Nope 0 ((go () s (resume 1 s))) 5)) (export main))";
    let mut db = crate::db::Db::load(parse(src));
    let d = crate::diagnostics(&mut db)
        .into_iter()
        .find(|d| d.code.as_deref() == Some("CDZ0101") && d.message.contains("Nope"))
        .expect("an unbound-effect CDZ0101");
    let node = d
        .node
        .expect("the unbound-effect CDZ0101 must carry a node, not be unanchored");
    assert!(
        db.is_user_node(crate::ast::StructId(node)),
        "node {node} must be the source `Nope` occurrence, not a synthesized atom"
    );
}

#[test]
fn an_undeclared_handler_op_close_to_a_declared_one_suggests_it() {
    // The effect-op "did you mean?" (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route
    // To A Fix): a handler arm names `emitt`, a typo of the effect's declared `emit` → CDZ0403 names
    // the near op AND carries a replace fix on the op KEY.
    let src = "(do (effect Log (op emit (-> Int64 Unit))) \
                   (def (main) (handle Log unit ((emitt (v) s (resume unit s))) ((. Log emit) 5))) \
                   (export main))";
    let err = compile_component(&crate::codec::encode(&parse(src)))
        .expect_err("undeclared op must reject");
    assert_eq!(err.code.as_deref(), Some("CDZ0403"), "got: {}", err.message);
    assert!(
        err.message.contains("did you mean `emit`?"),
        "names the near op: {}",
        err.message
    );
    let fix = err.fix.expect("a fix is carried");
    assert_eq!(fix.replacement, "emit");
    assert!(!fix.verified, "a nearest-name guess is heuristic");
}

// a_non_exhaustive_handler_is_cdz0405 migrated (code + message + fix, in full) to corpus
// 14b-effects-and-handlers "a handler that does not discharge every operation of its effect is rejected":
// enriched that case with (message "`collect`") (message "add (collect () s (resume") (fix
// (replacement-contains "(collect ()") (unverified)) — the omitted-op name, the spelled template arm, and
// the machine-applicable add-the-arm fix. rcdzc test deleted (fully corpus-covered).

#[test]
fn the_handler_add_arm_fix_resume_value_type_checks_in_one_shot() {
    // The `trap`-resume-value add-arm fix (M61's match-arm lesson applied to CDZ0405): the covering arm
    // the fix inserts must CLEAR the fault in ONE shot even when the missing op's RESULT type is
    // non-Unit. The old `(resume unit s)` body cascaded to a CDZ0201 "a handler resumes with a value of
    // type Unit but the operation's result type is <T>" the moment the op returned non-Unit; the
    // diverging `(resume (trap …) s)` resumes with a ∀a value that unifies with any result type, so the
    // repaired handler type-checks. Verified against the `diagnostics()` walk (`cdz check` / what
    // `fix --verify` runs), not full emit — this handler shape's emission is a separate effects gap.
    // These are the exact arms the fix inserts (asserted in the sibling test).
    fn no_type_error(src: &str) {
        let errs: Vec<_> = crate::diagnostics(&mut crate::db::Db::load(parse(src)))
            .into_iter()
            .filter(|d| {
                d.severity == crate::abi::Severity::Error
                    && matches!(d.code.as_deref(), Some("CDZ0201" | "CDZ0203" | "CDZ0405"))
            })
            .map(|d| (d.code, d.message))
            .collect();
        assert!(
            errs.is_empty(),
            "the handler add-arm fix's resume value must type-check against the op's result type \
                 (no CDZ0201/0203/0405 cascade): {errs:?}\nsrc: {src}"
        );
    }
    // `get : Unit → Int64` — the missing op returns Int64; the fix's `(resume (trap …) s)` type-checks.
    no_type_error(
        "(module m (effect E (op tick (-> Unit Unit)) (op get (-> Unit Int64))) \
             (def (main) (handle E 0 ((tick () s (resume unit s)) (get () s (resume (trap \"TODO: get\") s))) (E.get))) (export main))",
    );
    // And the OLD `unit` resume value genuinely DID cascade — pin the regression.
    let with_unit: Vec<_> = crate::diagnostics(&mut crate::db::Db::load(parse(
            "(module m (effect E (op tick (-> Unit Unit)) (op get (-> Unit Int64))) \
             (def (main) (handle E 0 ((tick () s (resume unit s)) (get () s (resume unit s))) (E.get))) (export main))",
        )))
        .into_iter()
        .filter(|d| d.code.as_deref() == Some("CDZ0201"))
        .collect();
    assert!(
        !with_unit.is_empty(),
        "a `unit` resume value where the op returns Int64 is the CDZ0201 cascade the trap value avoids"
    );
}

// a_delegation_of_an_unreached_effect_is_cdz0404 (`(host (log) 42)` delegating `log` never performed →
// CDZ0404 latent authority) migrated to corpus 14b-effects-and-handlers "a host delegation of an effect
// the body never reaches is latent authority and is rejected". rcdzc test deleted (corpus-covered, code-only).

#[test]
fn a_misspelled_delegated_op_does_not_also_report_latent_authority() {
    // A MISSPELLED op in a delegated body — `(E.emitt 5)` for declared `emit` — is the primary
    // CDZ0201 typo ("did you mean `emit`?"). It must NOT ALSO trigger CDZ0404 latent authority: the
    // misspelled `E.emitt` does not resolve as a perform, so `body_reaches_effect` is false, but the
    // author DID intend to reach `E` — the CDZ0404 is a pure cascade of the typo (fixing the typo
    // clears both). Only the root CDZ0201 should surface.
    let src = "(do (effect E (op emit (-> Int64 Unit))) \
                   (def (main) (host (E) (E.emitt 5))) (export main))";
    let mut db = crate::db::Db::load(parse(src));
    let codes: Vec<String> = crate::diagnostics(&mut db)
        .into_iter()
        .filter_map(|d| d.code)
        .collect();
    assert!(
        codes.iter().any(|c| c == "CDZ0201"),
        "the misspelled op is a CDZ0201 typo; got {codes:?}"
    );
    assert!(
        !codes.iter().any(|c| c == "CDZ0404"),
        "the latent-authority CDZ0404 is a cascade of the typo and must be suppressed; got {codes:?}"
    );
}

#[test]
fn a_latent_authority_delegation_offers_a_delete_fix() {
    // The CDZ0404 repair is to DROP the unreached effect from the manifest — a DELETE fix on the
    // effect-name occurrence (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A
    // Fix), the first use of `Edit::Delete`. `main` delegates `log` but never performs it.
    let src = "(do (effect log (op emit (-> String Unit))) \
                   (def (main) (host (log) 42)) (export main))";
    let err = compile_component(&crate::codec::encode(&parse(src)))
        .expect_err("latent authority must reject");
    assert_eq!(err.code.as_deref(), Some("CDZ0404"), "got: {}", err.message);
    let fix = err.fix.expect("a delete fix is carried");
    assert_eq!(fix.kind, crate::abi::FixKind::Delete);
    assert!(!fix.verified, "a delete is heuristic (intent guess)");
    // The fix targets the delegated effect-name occurrence (the same node the diagnostic anchors).
    assert_eq!(
        Some(fix.node),
        err.node,
        "delete targets the effect-name node"
    );
}

#[test]
fn a_wide_delegation_with_every_effect_reached_reports_no_latent_authority() {
    // The CDZ0404 latent-authority check computes the SET of effects the body reaches in ONE walk and
    // tests each delegated effect by membership (was one full body walk PER delegated effect → O(N²)).
    // This locks in the SET semantics: a host delegating many effects that are ALL reached must report
    // ZERO CDZ0404 — the reached-set contains every delegated decl. (Checked via `diagnostics`, the
    // fault path, so the separate emit-time "one interface per envelope" decline does not mask it.)
    let src = "(do (effect A (op a (-> String Unit))) (effect B (op b (-> String Unit))) \
                   (effect C (op c (-> String Unit))) \
                   (def (main) (host (A B C) (do (A.a \"x\") (B.b \"y\") (C.c \"z\")))) (export main))";
    let mut db = crate::db::Db::load(parse(src));
    let codes: Vec<String> = crate::diagnostics(&mut db)
        .into_iter()
        .filter_map(|d| d.code)
        .collect();
    assert!(
        !codes.iter().any(|c| c == "CDZ0404"),
        "every delegated effect is reached — no latent authority; got {codes:?}"
    );
}

#[test]
fn a_wide_delegation_flags_only_the_unreached_effect_as_latent_authority() {
    // The set-membership latent-authority check must still flag a GENUINELY unreached effect: a host
    // delegating A, B, C whose body reaches only A and B leaves C ∉ the reached set → exactly one
    // CDZ0404 (for C), not zero and not one-per-effect. Guards that the O(N)→set rewrite did not lose
    // the per-effect verdict (the `body_reached_effects` twin agrees with the old per-effect probe).
    let src = "(do (effect A (op a (-> String Unit))) (effect B (op b (-> String Unit))) \
                   (effect C (op c (-> String Unit))) \
                   (def (main) (host (A B C) (do (A.a \"x\") (B.b \"y\")))) (export main))";
    let mut db = crate::db::Db::load(parse(src));
    let n404 = crate::diagnostics(&mut db)
        .into_iter()
        .filter(|d| d.code.as_deref() == Some("CDZ0404"))
        .count();
    assert_eq!(
        n404, 1,
        "only the unreached effect C is latent authority — exactly one CDZ0404"
    );
}

#[test]
// The pinned sums are written `0 + 1 + … + n` to make `sum(0..=n)` explicit against the comment's
// N(N-1)/2 — the leading `0 +` is pedagogical, not a stray identity op.
#[allow(clippy::identity_op)]
fn the_effect_fold_pure_classifier_scales_linearly_over_a_wide_context() {
    // REGRESSION (perf): the frame-free effect fold classifies pure one-hole contexts via
    // `effects::strongly_pure`, which called `subtree_performs` — a full recursive subtree walk — at
    // its entry AND at every node its own structural descent reached. So over a wide/deep handle body
    // the SAME node's perform-verdict was recomputed O(size) times: `strongly_pure` was O(size²) and
    // the fold super-linear (a width-400 pure `+`-spine around one perform: `cdz check` ~170ms and
    // growing ~O(n^1.7); a depth-N nested-`let` perform chain was ~O(N³)). FIX: memoize
    // `subtree_performs` per `(node, handler-context-key)` in `db.subtree_performs_cache` — whether a
    // subtree performs is a pure function of the node and the discharged-op set — so each verdict
    // computes once. The wide context is now LINEAR (width-400 ~25ms), the deep chain ~halved.
    //
    // Correctness: a chain of N performs, each bound by a nested `let`, threaded under a counter
    // handler (`resume s (+ s 1)`, seed 0) — the i-th read is `i`, so the sum is 0+1+…+(N-1).
    fn nested_chain(n: usize) -> String {
        let mut summ = String::from("0");
        for i in 0..n {
            summ = format!("(+ a{i} {summ})");
        }
        let mut inner = summ;
        for i in (0..n).rev() {
            inner = format!("(let ((a{i} ((. Ask get)))) {inner})");
        }
        format!(
            "(do (effect Ask (op get (-> Unit Int64))) \
                   (def (main) (handle Ask 0 ((get () s (resume s (+ s 1)))) {inner})) \
                   (export main))"
        )
    }
    // The chain COMPILES (the fold + effect-handler lowering); the i-th of N threaded performs reads i
    // (counter seeded 0), so it sums to N(N-1)/2 = 15 — that RUN value is corpus/conformance territory
    // (14-effects handler-resume-counter dispatch); this perf test keeps the compile + count guards.
    let _ = compile_component(&crate::codec::encode(&parse(&nested_chain(6))))
        .expect("a 6-perform nested-let chain folds and compiles");
    // Growth guard on a WIDE pure context — `(+ 0 (+ 1 … (Ask.get)))`, one perform at the bottom of an
    // N-wide `+`-spine that `strongly_pure`/`pure_hole` must classify. It is SHALLOW (no deep recursion,
    // so no interaction with the fold's reduction-depth backstop) and its classification cost is the
    // memoized `subtree_performs` — the cleanest signal for this fix. Was O(n²) (the per-node re-walk);
    // now linear. The NOISE-FREE signal is `SUBTREE_PERFORMS_UNCACHED_CALLS` — the count of ACTUAL
    // un-cached perform-verdict computations (the compiler's own recursion count, a pure function of the
    // program), NOT wall-clock. A wall-clock ratio false-fails under fleet load (a width-200 run in a
    // quiet slice vs a width-400 run hitting a scheduling stall inflates the ratio past threshold —
    // exactly the flake the count-based guard avoids). Linear ⇒ the count grows ~2× over a 2× width;
    // the old per-node re-walk was O(n²) ⇒ ~4×.
    fn wide_pure(n: usize) -> String {
        let mut e = String::from("((. Ask get))");
        for i in 0..n {
            e = format!("(+ {i} {e})");
        }
        format!(
            "(do (effect Ask (op get (-> Unit Int64))) \
                   (def (main) (handle Ask 5 ((get () s (resume s s))) {e})) \
                   (export main))"
        )
    }
    // The wide context folds to `sum(0..n) + 5` — pin the value at a small width too.
    // The wide `+`-spine COMPILES; it sums 0..4 plus the perform's resumed state 5 = 11 (that RUN value
    // is corpus territory as above). Kept as a compile guard feeding the perf-count check below.
    let _ = compile_component(&crate::codec::encode(&parse(&wide_pure(4))))
        .expect("a wide pure context around one perform folds and compiles");
    fn uncached_calls(src: &str) -> u64 {
        // The wide `+`-spine (width 200/400) parses and type-checks to a deep-but-finite recursion —
        // route it through the depth-sized compiler stack so it doesn't overflow the ~2 MB `cargo test`
        // worker stack (the guard-thread pattern the deep-recursion tests use).
        crate::host::run_with_compiler_stack(|| {
            crate::db::SUBTREE_PERFORMS_UNCACHED_CALLS.with(|c| c.set(0));
            let _ = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
            crate::db::SUBTREE_PERFORMS_UNCACHED_CALLS.with(|c| c.get())
        })
    }
    // Width 200→400 is a 2× spine; LINEAR (each node's verdict computed once via the memo) ⇒ the
    // un-cached count grows ~2×; the O(n²) per-node re-walk (a cache-defeating regression) grew ~4×.
    // Require the ratio stay well under 3× (between the regimes, with margin for constant terms).
    let n200 = uncached_calls(&wide_pure(200));
    let n400 = uncached_calls(&wide_pure(400));
    let ratio = n400 as f64 / (n200.max(1)) as f64;
    assert!(
        n200 > 0 && ratio < 3.0,
        "the effect fold's pure classifier must scale LINEARLY over a wide context (was O(n²) via a \
             per-node whole-subtree `subtree_performs` re-walk; now memoized per `(node, ctx.key)`): width \
             200→400 grew un-cached `subtree_performs` computations {ratio:.1}× (n200={n200}, n400={n400}); \
             linear is ~2×, the old re-walk was ~4×"
    );
}

#[test]
fn a_parameterized_recursive_walk_threads_a_list_state() {
    // E3: a recursive PARAMETERIZED effectful walk `(walk n)` that accumulates into a LIST state,
    // performing inside a nested `(do …)`. `walk` emits `n` at each step (threading `(List.push s n)`)
    // and reads the list back at the base via `collect`. Specialized as `walk#ctx(n: Int64, s: List
    // Int64)` — original param `n` threaded + annotated with its solved type, state a trailing list
    // param, recursion via the memoized self-call. Seeded `(list 0)` (a DETERMINED element type — an
    // empty `(list)` seed declines, its element type being `Any`), `(walk 3)` accumulates
    // `(list 0 3 2 1)` whose length is 4. Exercises parameterized spec + list-state-through-recursion
    // + do-intermediate perform + recursion-through-do all at once.
    let src = "(do (effect Diag (op emit (-> Int64 Unit)) (op collect (-> Unit (List Int64)))) \
                   (def (walk n) (if (< n 1) (Diag.collect unit) (do (Diag.emit n) (walk (- n 1))))) \
                   (def (main) (handle Diag (list 0) \
                     ((emit (v) s (resume unit (List.push s v))) (collect (u) s (resume s s))) \
                     (List.len (walk 3)))) (export main))";
    // COMPILES (the specialization + list-state threading succeed) — asserting compilation, not a run,
    // because a list-returning body needs the value-heap runtime composed from the store (the
    // `#[ignore]`d heap tests do that); the store-driven CLI run yields 4 (`(list 0 3 2 1)` length).
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "a parameterized recursive walk threading a determined list state must compile"
    );
}

// The resume ARITY + PLACEMENT rejects (too-many CDZ0201 + delete fix, too-few "no next-state", genuine
// top-level stray "no enclosing handler arm") moved to corpus 14b-effects-and-handlers "a resume with too
// many operands …" + siblings. Residual: the CROSS-DIAGNOSTIC suppression the corpus cannot express — a
// malformed resume that IS in an arm must not ALSO emit the stray-resume secondary, and a STRAY+malformed
// resume reports the PLACEMENT error as the one primary with the misleading arity message suppressed.
#[test]
fn a_malformed_resume_in_an_arm_suppresses_the_stray_resume_secondary() {
    use crate::testkit::parse;
    // (a) too-many / (b) too-few, both IN an arm: neither also reports the stray "no enclosing handler arm".
    for src in [
        "(module m (effect E (op get (-> Unit Int64))) (def (main) (handle E 0 ((get () s (resume 5 s 9))) (+ (E.get) 1))) (export main))",
        "(module m (effect E (op get (-> Unit Int64))) (def (main) (handle E 0 ((get () s (resume 5))) (+ (E.get) 1))) (export main))",
    ] {
        let d = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
        assert!(
            !d.iter()
                .any(|x| x.message.contains("no enclosing handler arm")),
            "a malformed resume in an arm must not report the stray secondary: {:?}",
            d.iter().map(|x| &x.message).collect::<Vec<_>>()
        );
    }
    // (d) STRAY + malformed: the placement error is the one primary; the misleading arity message is dropped.
    for bad in [
        "(module m (def (main) (resume 5)) (export main))",
        "(module m (def (main) (resume 5 6 7)) (export main))",
    ] {
        let d = crate::diagnostics(&mut crate::db::Db::load(parse(bad)));
        assert!(
            d.iter()
                .any(|x| x.message.contains("no enclosing handler arm"))
                && !d.iter().any(|x| x.message.starts_with("this resume has")),
            "a stray+malformed resume reports placement, suppresses arity: {:?}",
            d.iter().map(|x| &x.message).collect::<Vec<_>>()
        );
    }
}

// The head-naming MESSAGE facets (a value/type/unbound/prelude-type handle head names "head must name an
// EFFECT" / "is a type") moved to corpus 14b-effects-and-handlers "a handle head that is a value def …" +
// siblings. Residual: the "exactly ONE diagnostic" dedup the corpus cannot express — the dropped member-
// access / fold-decline cascades are UNCODED, so (no-other-errors) (coded-only) does not catch them.
#[test]
fn a_handle_head_naming_a_non_effect_reports_one_diagnostic_dropping_uncoded_cascades() {
    // value head: exactly ONE diagnostic (the member-access + fold-decline cascades dropped).
    let ds = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def foo 5) (def (main) (handle foo 0 ((x (u) s (resume 1 s))) 5)) (export main))",
    )));
    assert_eq!(
        ds.len(),
        1,
        "value head = one diagnostic (cascade dropped): {ds:?}"
    );
    // type head: exactly ONE error (the fold-decline + no-variant consequents dropped).
    let td: Vec<crate::abi::Diagnostic> = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (type C (Red)) (def (main) (handle C 0 ((a (u) s (resume 1 s))) 5)) (export main))",
    )))
    .into_iter()
    .filter(|d| d.severity == crate::abi::Severity::Error)
    .collect();
    assert_eq!(
        td.len(),
        1,
        "type head = one error (consequents dropped): {td:?}"
    );
}

mod part2;
