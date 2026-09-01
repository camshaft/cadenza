use crate::compile::compile_component;
use crate::testkit::parse;

#[test]
fn a_row_op_over_a_record_with_a_unit_field_does_not_spuriously_dup_the_unit_projection() {
    // PR#914 Copilot invariant fix (breaker #45 follow-up): `collect_row_op_field_dups` gated the
    // heap-field dup on `get_op(field) == Ok(None)` to mean "heap handle" — but `get_op` ALSO returns
    // `Ok(None)` for `Ty::Unit` (the inline `IMM_UNIT` sentinel, `Drop`'d inline by the `Core::Proj`
    // emitter, never dup'd). So a Unit field-proj wrongly entered `dup_sites` → `collect_used_ops`
    // imported `dup` with no matching emit → broke "import exactly the ops we call" (a spurious import).
    // Fixed by excluding `Ty::Unit` from the gate (a Unit field owns no reference — `r`'s drop can't
    // dangle it). A row op over a record with a Unit field must emit a VALID module (a spurious dup
    // import, or a dup on the inline-unit sentinel, would otherwise perturb op resolution / be invalid).
    let compile = |src: &str| {
        compile_component(&crate::codec::encode(&parse(src)))
            .expect("a row op over a record with a Unit field compiles")
    };
    // `without` over a map-borne (owned) record that HAS a Unit field: the kept heap field (name via
    // qty-drop is scalar here, so keep a String) exercises the heap-dup, while the Unit field must be
    // skipped. Use a record with both a Unit and a String field, drop the qty, keep name+u.
    let bytes = compile(
        "(module m (def (main (: k Int64)) (do \
               (def inv (Map.insert Map.empty 1 (record (= name \"w\") (= u unit) (= qty k)))) \
               (def r (Option.expect (Map.lookup inv 1) \"s\")) \
               (String.byte-len (. (Record.without r (qty)) name)))) (export main))",
    );
    wasmparser::validate(&bytes).expect(
        "a row op over a record with a Unit field emits a valid module (Unit proj not dup-marked)",
    );
}

#[test]
fn a_lowercase_named_type_is_referenceable_in_a_field() {
    // A type is a VALUE, referenceable by name regardless of case. A lowercase-named sum
    // `(type mylist (Nil) (Cons Int64 mylist))` SELF-references `mylist` in its field, and a lowercase
    // `(type wrap (W num))` cross-references a declared `(type num …)`. The implicit-type-parameter scan
    // (`db::scan_top_level`) captures a free lowercase payload name as a tyvar (`a` in `(type Box (W a))`),
    // but a name that names a DECLARED type is dropped from the params after the full type-name gather —
    // so it resolves to the type (step 3), not a fresh variable. Before this the reference re-lexed as a
    // tyvar, the sum silently became generic, and its variants failed to resolve (a confusing CDZ0203).
    let ok = |src: &str| {
        assert!(
            compile_component(&crate::codec::encode(&parse(src))).is_ok(),
            "must compile: {src}"
        )
    };
    // Self-referential lowercase recursive sum — the field `mylist` resolves to the type, so its
    // variants (`Nil`/`Cons`) resolve and the match type-checks + emits (was CDZ0203). A recursive
    // `len` fold over it (self-referential recursion through the field) also compiles.
    ok(
        "(module m (type mylist (Nil) (Cons Int64 mylist)) (def (f (: l mylist)) (match l ((Nil) 0) ((Cons h t) 1))) (def (main) (f (mylist.Nil))) (export main))",
    );
    ok(
        "(module m (type mylist (Nil) (Cons Int64 mylist)) (def (len (: l mylist)) (match l ((Nil) 0) ((Cons h t) (+ 1 (len t))))) (def (main) (len (mylist.Nil))) (export main))",
    );
    // Cross-referencing lowercase types (`wrap` references declared `num`).
    ok(
        "(module m (type num (Z)) (type wrap (W num)) (def (g (: w wrap)) (match w ((W n) 1))) (def (main) (g (wrap.W (num.Z)))) (export main))",
    );
    // NO REGRESSION: a genuine tyvar (`a`, matching no declared type) stays a real type variable — the
    // generic sum still compiles + instantiates.
    ok(
        "(module m (type Box (W a) (E)) (def (main) (match (Box.W 5) ((W n) n) ((E) 0))) (export main))",
    );
    // The Capitalized spelling is unaffected.
    ok(
        "(module m (type Mylist (Nil) (Cons Int64 Mylist)) (def (f (: l Mylist)) (match l ((Nil) 0) ((Cons h t) 1))) (def (main) (f (Mylist.Nil))) (export main))",
    );
    // (The end-to-end RUN — a recursive `len` over a lowercase `mylist` → 3 — is covered by the corpus
    // gate, which composes the value-heap runtime this compile-only check does not.)
}

// a_recursive_newtype_traversal_recurses_on_its_projected_field migrated to corpus 05-compound-types
// "a recursive NEWTYPE-wrapped linked list folds to a scalar" — the SAME program (type Lst (Mk (Option
// (Tuple Int64 Lst))), sm recursing on the projected field (. p 1)) RUNS to sm[10,20,30]=60 on wasm,
// which requires it to type-check clean (proving the CDZ0203 over-rejection stays fixed). rcdzc
// compile-validity pin deleted (the corpus value-run subsumes it).

#[test]
fn a_self_referential_value_definition_names_the_cycle_not_a_resource_limit() {
    // `(def (g) g)` — a nullary VALUE defined in terms of itself with no base case — names nothing
    // (`g = g`). It used to reduce until the depth guard fired, mislabeled "expression nests too deeply
    // (a recursion/resource limit)" (reads as a compiler resource problem). Now it is rejected CDZ0201
    // naming the real cause — a value cannot reference itself — with the fix route. (Contrast the mutual
    // FUNCTIONS above, which run: a function's self-reference is legitimate recursion.)
    // Through the host-stack guard the bin uses (`host.rs`): a self-referential value cycle reduces
    // up to `DESCENT_DEPTH_LIMIT` (1024 levels — bounded, and the CDZ0201 IS emitted) before the guard
    // stops it, but that depth needs the guard-sized 64 MB stack, not a default `cargo test` worker's
    // ≈2 MB (which SIGABRTs, EXIT=101, 0 FAILED). Deep-but-finite, not a loop — the CLI already runs
    // this through the guard, so the test must too.
    let single = crate::host::run_with_compiler_stack(|| {
        crate::diagnostics(&mut crate::db::Db::load(parse(
            "(module m (def (g) g) (export g))",
        )))
    });
    let d = single
        .iter()
        .find(|d| d.code.as_deref() == Some("CDZ0201"))
        .expect("a self-referential value def is rejected");
    assert!(
        d.message
            .contains("defined in terms of itself with no base value"),
        "names the cycle, not a resource limit: {}",
        d.message
    );
    assert!(
        single
            .iter()
            .all(|d| !d.message.contains("nests too deeply")),
        "the misleading resource-limit decline is suppressed: {single:?}"
    );
    // A MUTUAL value cycle (`a = b`, `b = a`) is caught too — each cyclic def reports once, and the
    // "nests too deeply" decline is deduped away (exactly 2 errors, both the clear cycle message).
    let ds = crate::host::run_with_compiler_stack(|| {
        crate::diagnostics(&mut crate::db::Db::load(parse(
            "(module m (def (a) b) (def (b) a) (export a))",
        )))
    });
    assert_eq!(
        ds.iter()
            .filter(|d| d.code.as_deref() == Some("CDZ0201"))
            .count(),
        2,
        "each of the two mutually-cyclic values is named: {ds:?}"
    );
    assert!(
        ds.iter().all(|d| !d.message.contains("nests too deeply")),
        "no leftover resource-limit decline in the mutual cycle: {ds:?}"
    );
    // NO false positive: a def referring to ANOTHER def with a base value is fine; a recursive FUNCTION
    // is fine (it has params → a lambda, not a bare Ref cycle).
    assert!(
        crate::diagnostics(&mut crate::db::Db::load(parse(
            "(module m (def (a) 5) (def (b) a) (export b))",
        )))
        .iter()
        .all(|d| d.severity != crate::abi::Severity::Error),
        "a def referencing another def with a concrete value is valid"
    );
    assert!(
            crate::diagnostics(&mut crate::db::Db::load(parse(
                "(module m (def (f (: n Int64)) (if (< n 1) 0 (f (- n 1)))) (def (main) (f 5)) (export main))",
            )))
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
            "a recursive function is legitimate, not a value cycle"
        );
    // NO false positive on a def that merely REFERENCES a self-cyclic value without being IN the cycle.
    // `(def x x)` is self-cyclic; a `(def (main) x)` naming it must NOT ALSO be reported "`main` is
    // defined in terms of itself" — `main` is not in its own cycle, it points into `x`'s. The cycle
    // detector keyed a `seen`-set revisit of ANY node as a cycle, mis-attributing `x`'s downstream cycle
    // to `main`; keying the closure on the START node fixes it. Exactly ONE CDZ0201 (naming `x`), and it
    // is `x`, never `main`.
    let referrer = crate::host::run_with_compiler_stack(|| {
        crate::diagnostics(&mut crate::db::Db::load(parse(
            "(module m (def x x) (def (main) x) (export main))",
        )))
    });
    let cycle_defs: Vec<_> = referrer
        .iter()
        .filter(|d| {
            d.code.as_deref() == Some("CDZ0201") && d.message.contains("defined in terms of itself")
        })
        .collect();
    assert_eq!(
        cycle_defs.len(),
        1,
        "only the actually-cyclic `x` is named, not the referrer `main`: {referrer:?}"
    );
    assert!(
        cycle_defs[0].message.contains("`x`") && !cycle_defs[0].message.contains("`main`"),
        "the cycle is attributed to `x`, not the referrer `main`: {}",
        cycle_defs[0].message
    );
    // A CHAIN into a cycle that does not include the head — `(def (a) b) (def (b) b)` — reports the
    // cycle at `b` (which IS self-cyclic) and does NOT report `a` as self-referential (`a` merely
    // points into `b`'s cycle). Exactly one "defined in terms of itself", naming `b`.
    let chain = crate::host::run_with_compiler_stack(|| {
        crate::diagnostics(&mut crate::db::Db::load(parse(
            "(module m (def (a) b) (def (b) b) (export a))",
        )))
    });
    let chain_cycles: Vec<_> = chain
        .iter()
        .filter(|d| {
            d.code.as_deref() == Some("CDZ0201") && d.message.contains("defined in terms of itself")
        })
        .collect();
    assert_eq!(
        chain_cycles.len(),
        1,
        "only the self-cyclic `b` is named, not the referrer `a`: {chain:?}"
    );
    assert!(
        chain_cycles[0].message.contains("`b`"),
        "the chain's cycle is attributed to `b`: {}",
        chain_cycles[0].message
    );
}

#[test]
fn a_recursive_def_with_no_base_case_names_the_missing_base_case_not_the_parameter() {
    // A recursive function whose EVERY path recurses (`(def (loop n) (loop (+ n 1)))`) declines: its
    // RESULT type is undetermined (the body never yields a concrete value). `def_scheme` returns `None`
    // — but so does an undetermined-PARAMETER decline, and the call site used to blame the PARAMETER for
    // both, telling the author to "add an explicit annotation `(: p Int64)`". That is WRONG here: with
    // `(: n Int64)` the parameter is already determined, yet the old message still demanded the very
    // annotation already present. The fix distinguishes the two via `recursive_def_params_all_determined`
    // — when the params ARE determined, the fault is the missing BASE CASE, and the message says so.
    let base_case_diag = |src: &str| {
        crate::diagnostics(&mut crate::db::Db::load(parse(src)))
            .into_iter()
            .find(|d| d.severity == crate::abi::Severity::Error)
            .unwrap_or_else(|| panic!("a no-base-case recursive def must decline: {src}"))
    };
    // Unannotated AND annotated — BOTH now name the missing base case (the annotated one is the bug:
    // the parameter is fine, so "annotate the parameter" was a dead end).
    for src in [
        "(module m (def (loop n) (loop (+ n 1))) (def (main) (loop 0)) (export main))",
        "(module m (def (loop (: n Int64)) (loop (+ n 1))) (def (main) (loop 0)) (export main))",
    ] {
        let d = base_case_diag(src);
        let m = &d.message;
        // Coded CDZ0204 (`NonProductiveRecursion`, the 02xx well-formedness REJECTION band — an invalid
        // user program with no base case, not a capability decline; distinct from CDZ0999 RecursionBound
        // and CDZ0203 TypeMismatch). Pins the code, not just the message (seq-286 coded-decline).
        assert_eq!(
            d.code.as_deref(),
            Some("CDZ0204"),
            "a no-base-case recursion is coded CDZ0204 (NonProductiveRecursion): {m}"
        );
        assert!(
            m.contains("never returns") && m.contains("BASE CASE"),
            "names the missing base case, not the parameter: {m}"
        );
        assert!(
            !m.contains("add an explicit annotation"),
            "must NOT send the author to annotate an already-fine parameter: {m}"
        );
    }
    // (A def WITH a base case that stops the recursion types + runs to its result — the positive
    // base-case control — is corpus-covered by the recursion cases in 09-functions; the wasmtime run
    // is dropped here, this test keeps only the diagnostic-QUALITY assertion the corpus cannot express:
    // the fault names the missing BASE CASE, NOT "add an explicit annotation".)
    // A GENUINE undetermined-PARAMETER decline (a pure pass-through with NO base case is dominated by
    // the missing base case, so construct the param case that survives a base case: a param never
    // width-fixed AND never seeded concretely). The recursive-param / monomorphization-tie guidance
    // still fires for that shape — the base-case branch does NOT swallow it.
    let ambiguous = "(module m (def (id x) x) (def (loop p) (if true (id p) (loop p))) \
                          (def (main) (loop (id 0))) (export main))";
    // (This particular one resolves via the seeded `(id 0)` → Int64; the assertion we care about is the
    // POSITIVE base-case naming above and the no-annotation-dead-end guarantee. Kept as a compile smoke.)
    let _ = crate::diagnostics(&mut crate::db::Db::load(parse(ambiguous)));
}

#[test]
fn set_to_list_over_a_float_containing_compound_declines_not_silently_empties() {
    // WARNING: SILENT-DATA-LOSS regression (routed corpus-bugfix/breaker 2026-07-28): `Set.to-list` over a
    // FLOAT-LEAF TUPLE element ran to an EMPTY list on wasm (Set.len correct but to-list []), silently
    // dropping every element — worse than a decline (a fold over the enumeration processes NOTHING).
    // Root: `orderable_leaf_or_compound` propagated its `float_ok` (the bare-float-root to-list mode)
    // INTO the compound arms, so a float-in-tuple passed the Set/Map shape descriptor guard — but the
    // runtime's compound `value_cmp_shaped` returns None for a Float leaf mid-walk (a bare float orders
    // by to_bits; a float INSIDE a compound does not), so `op_set_to_list`'s sort yielded []. Per 03:626
    // / §319 a float-containing compound has NO total order → the CORRECT answer is a uniform DECLINE
    // (same as `<`, same as the Set<Bytes> ruling). Fixed by recursing the compound arms with
    // `float_ok = false`, so `Set.to-list` over a float-containing compound DECLINES at compile time.
    // (breaker #34: 5 faces — tuple/record/list/nested/map-key — one shared fix.)
    let compiles = |src: &str| compile_component(&crate::codec::encode(&parse(src))).is_ok();
    // The float-leaf tuple + a sibling face (Map key) must DECLINE (compile fails cleanly).
    assert!(
        !compiles(
            "(module m (def (main) (Set.to-list (Set.of (list (tuple 1.5 1) (tuple 2.5 2) (tuple -1.0 3))))) (export main))"
        ),
        "Set.to-list over a float-leaf tuple must DECLINE (float compound has no order, 03:626), not silently []"
    );
    assert!(
        !compiles(
            "(module m (def (main) (Map.to-list (Map.insert (Map.empty) (tuple 1.5 1) 9))) (export main))"
        ),
        "Map.to-list with a float-leaf tuple KEY must DECLINE"
    );
    // CONTROL — a BARE float set STILL enumerates (float root orders by canonical bytes, unregressed).
    assert!(
        compiles(
            "(module m (def (main) (List.len (Set.to-list (Set.of (list 1.5 2.5 3.5))))) (export main))"
        ),
        "a bare-float Set.to-list must STILL enumerate (float root canonical-byte order, not regressed)"
    );
    // CONTROL — an INT-leaf tuple set STILL enumerates (the compound arm didn't over-decline).
    assert!(
        compiles(
            "(module m (def (main) (List.len (Set.to-list (Set.of (list (tuple 1 1) (tuple 2 2)))))) (export main))"
        ),
        "an int-leaf tuple Set.to-list must STILL enumerate (only FLOAT leaves make a compound unorderable)"
    );
}

#[test]
fn a_loop_invariant_length_is_hoisted_out_of_the_loop() {
    // LOOP-INVARIANT CODE MOTION: the classic index loop `(if (< i (List.len xs)) …)` recomputed
    // `(List.len xs)` — a `vec-len` runtime import CALL — every iteration, though `xs` is a
    // pass-through param (threaded unchanged on every back-edge). LICM now computes it ONCE before the
    // loop into a slot and reads the slot inside. Pins the `vec-len` (`Lir::CallImport(OP_VEC_LEN)`)
    // OUT of the loop at the Lir level (exactly one such call, and it PRECEDES the `Lir::Loop`) + value
    // parity. NOTE the inner `(List.at xs i)` has its OWN internal bounds `vec-len` (a different node,
    // over the varying `i`), so total `vec-len` count is 2 — but the LOOP-GUARD one is hoisted.
    use crate::backend::wasm::lir::Lir;
    use crate::backend::wasm::select::select_function_of;
    let src = "(module m \
                     (def (sumidx (: xs (List Int64)) (: i Int64) (: acc Int64)) \
                       (if (< i ((. List len) xs)) \
                           (sumidx xs (+ i 1) (+ acc (match ((. List at) xs i) ((Some x) x) ((None _) 0)))) \
                           acc)) \
                     (def (f (: xs (List Int64))) (sumidx xs 0 0)) (export f))";
    let mut db = crate::db::Db::load(crate::testkit::parse(src));
    let layout = crate::layout::compute(&mut db).expect("layout");
    let d = db.def_by_name("sumidx").expect("sumidx");
    let ps: Vec<_> = db.defs[d]
        .params
        .clone()
        .into_iter()
        .map(|p| {
            let b = db
                .ast
                .as_form(p, ":")
                .and_then(|t| t.first().copied())
                .unwrap_or(p);
            (b, crate::infer::type_of(&mut db, b))
        })
        .collect();
    let body = db.defs[d].body.expect("body");
    let code = select_function_of(&mut db, body, &ps, &layout, Some(d))
        .expect("select")
        .code;
    // Find the loop; the loop-guard `vec-len` must be emitted BEFORE it (hoisted), not only inside.
    let loop_ix = code
        .iter()
        .position(|i| matches!(i, Lir::Loop(_)))
        .expect("sumidx compiles to a loop");
    let vec_len_before = code[..loop_ix]
        .iter()
        .filter(|i| matches!(i, Lir::CallImport("vec-len")))
        .count();
    assert_eq!(
        vec_len_before, 1,
        "the loop-invariant `(List.len xs)` is hoisted to a single `vec-len` BEFORE the loop, got \
             {vec_len_before}: {code:?}"
    );

    // The value parity of an index loop that runs correctly with the hoisted loop-invariant length is
    // covered by the corpus loop-invariant-hoist family (02-binding-and-control "a loop whose bound is
    // an invariant computation runs correctly (the bound is hoisted)" + the match-scrutinee-invariant
    // case); only the Lir hoist witness (the single `vec-len` before the loop) stays here.
}

#[test]
fn a_loop_invariant_bitwise_op_is_hoisted() {
    // A trap-free, loop-invariant SCALAR op — `(& k 255)` over the pass-through param `k` — is hoisted
    // out of the loop (computed once) instead of recomputed each iteration. Pins the `i64.and` count
    // BEFORE the loop and value parity. (The in-loop `i64.and` of the `(+ acc …)` overflow guard is a
    // DIFFERENT computation, so it stays; we assert the invariant `& 255` moved out.)
    use crate::backend::wasm::lir::Lir;
    use crate::backend::wasm::select::select_function_of;
    let src = "(module m \
                     (def (go (: n Int64) (: k Int64) (: acc Int64)) \
                       (if (= n 0) acc (go (- n 1) k (+ acc (& k 255))))) \
                     (def (f (: k Int64)) (go 10 k 0)) (export f))";
    let mut db = crate::db::Db::load(crate::testkit::parse(src));
    let layout = crate::layout::compute(&mut db).expect("layout");
    let d = db.def_by_name("go").expect("go");
    let ps: Vec<_> = db.defs[d]
        .params
        .clone()
        .into_iter()
        .map(|p| {
            let b = db
                .ast
                .as_form(p, ":")
                .and_then(|t| t.first().copied())
                .unwrap_or(p);
            (b, crate::infer::type_of(&mut db, b))
        })
        .collect();
    let body = db.defs[d].body.expect("body");
    let code = select_function_of(&mut db, body, &ps, &layout, Some(d))
        .expect("select")
        .code;
    let loop_ix = code
        .iter()
        .position(|i| matches!(i, Lir::Loop(_)))
        .expect("go compiles to a loop");
    let and_before = code[..loop_ix]
        .iter()
        .filter(|i| matches!(i, Lir::I64And))
        .count();
    assert_eq!(
        and_before, 1,
        "the invariant `(& k 255)` is hoisted to a single `i64.and` before the loop: {code:?}"
    );
    // Value parity — that the hoisted `(& k 255)` still yields the right accumulated result — is the
    // corpus case "a loop-invariant bitwise op hoisted out of a tail loop preserves the value"
    // (spec/semantics/09-functions.sexp): f(999) = 10 * (999 & 255) = 2310, run via cdz-run.
}

#[test]
fn a_trapping_loop_invariant_in_the_condition_is_hoisted() {
    // A loop-invariant CHECKED op — `(* n 2)`, a checked multiply (NOT trap-free) — sits in the loop
    // CONDITION `(< i (* n 2))`, an ALWAYS-EVALUATED position (the exit check runs even for a 0-
    // iteration loop). LICM hoists it out of the loop even though it can trap, because doing so is
    // trap-EQUIVALENT: the condition evaluates `(* n 2)` on entry either way. The `(* n 2)`
    // strength-reduces to `i64.shl` (cycle-21) + its overflow round-trip guard; the whole thing must
    // appear BEFORE the loop, not inside. (A trapping invariant BURIED IN A BRANCH would stay put —
    // the frontier restriction — but here it is in the always-run condition.)
    use crate::backend::wasm::lir::Lir;
    use crate::backend::wasm::select::select_function_of;
    let src = "(module m \
                     (def (go (: i Int64) (: n Int64) (: acc Int64)) \
                       (if (< i (* n 2)) (go (+ i 1) n (+ acc i)) acc)) \
                     (def (f (: x Int64)) (go 0 x 0)) (export f))";
    let mut db = crate::db::Db::load(crate::testkit::parse(src));
    let layout = crate::layout::compute(&mut db).expect("layout");
    let d = db.def_by_name("go").expect("go");
    let ps: Vec<_> = db.defs[d]
        .params
        .clone()
        .into_iter()
        .map(|p| {
            let b = db
                .ast
                .as_form(p, ":")
                .and_then(|t| t.first().copied())
                .unwrap_or(p);
            (b, crate::infer::type_of(&mut db, b))
        })
        .collect();
    let body = db.defs[d].body.expect("body");
    let code = select_function_of(&mut db, body, &ps, &layout, Some(d))
        .expect("select")
        .code;
    let loop_ix = code
        .iter()
        .position(|i| matches!(i, Lir::Loop(_)))
        .expect("go compiles to a loop");
    // The invariant `(* n 2)` → `i64.shl` is hoisted BEFORE the loop; none remains inside.
    let shl_before = code[..loop_ix]
        .iter()
        .filter(|i| matches!(i, Lir::I64Shl))
        .count();
    let shl_inside = code[loop_ix..]
        .iter()
        .filter(|i| matches!(i, Lir::I64Shl))
        .count();
    assert_eq!(
        shl_before, 1,
        "the invariant `(* n 2)` is hoisted (one `i64.shl` before the loop): {code:?}"
    );
    assert_eq!(
        shl_inside, 0,
        "no `(* n 2)` shift remains inside the loop body: {code:?}"
    );
}

#[test]
fn a_repeated_trapping_node_inside_one_if_arm_is_not_hoisted_past_the_branch() {
    // The CSE DUAL of the LICM frontier restriction: a repeated possibly-trapping node that lives ONLY
    // inside a conditional arm must NOT be computed up-front. `(if c (+ (/ a b) (/ a b)) 5)` uses the
    // checked `(/ a b)` TWICE — but only on the THEN path. The straight-line CSE is gated to a
    // body with NO `if`/`match` (so every shared node is unconditionally reached), so a body WITH an
    // `if` is ineligible: `(/ a b)` stays inside the then-arm, NOT hoisted to the function top. If it
    // were speculatively hoisted, `c=false, b=0` would trap on a division the taken `5` arm never runs.
    // Pins the correctness gate end-to-end: the trap fires IFF the then-arm is taken.
    use crate::backend::wasm::lir::Lir;
    use crate::backend::wasm::select::select_function_of;
    let src = "(module m \
                     (def (f (: c Bool) (: a Int64) (: b Int64)) (if c (+ (/ a b) (/ a b)) 5)) \
                     (export f))";
    let mut db = crate::db::Db::load(crate::testkit::parse(src));
    let layout = crate::layout::compute(&mut db).expect("layout");
    let d = db.def_by_name("f").expect("f");
    let ps: Vec<_> = db.defs[d]
        .params
        .clone()
        .into_iter()
        .map(|p| {
            let bb = db
                .ast
                .as_form(p, ":")
                .and_then(|t| t.first().copied())
                .unwrap_or(p);
            (bb, crate::infer::type_of(&mut db, bb))
        })
        .collect();
    let body = db.defs[d].body.expect("body");
    let code = select_function_of(&mut db, body, &ps, &layout, Some(d))
        .expect("select")
        .code;
    // The `(/ a b)` divide stays INSIDE the `if` — it must appear AT OR AFTER the branch, never before
    // it. (An `if` over a non-select-ifiable trapping body keeps a real `Lir::If`.)
    let if_ix = code
        .iter()
        .position(|i| matches!(i, Lir::If(_)))
        .expect("the trapping-arm body keeps a real if");
    let div_before = code[..if_ix]
        .iter()
        .filter(|i| matches!(i, Lir::I64DivS))
        .count();
    assert_eq!(
        div_before, 0,
        "no `(/ a b)` may be hoisted before the branch (would speculate its trap): {code:?}"
    );
    // Value + trap parity — c=false → 5 (the untaken `(/ a 0)` is NOT speculated, no trap), c=true
    // with a safe divisor → (a/b)+(a/b), c=true with b=0 → traps on the taken divide — is the corpus
    // case "a repeated trapping divide inside one if-arm is not speculated past the branch"
    // (spec/semantics/02-binding-and-control.sexp), run via cdz-run.
}

#[test]
fn a_loop_invariant_in_both_the_condition_and_the_body_is_hoisted_once() {
    // The SAME loop-invariant `(* n 2)` appears in BOTH the condition `(< i (* n 2))` AND the body
    // `(+ acc (* n 2))` — two distinct StructIds, but `core_eq`. LICM value-numbers the hoist: it
    // computes `(* n 2)` ONCE before the loop and points BOTH occurrences at that slot, so exactly
    // ONE `i64.shl` is emitted (the strength-reduced `* 2`), not one hoisted + one per-iteration copy.
    use crate::backend::wasm::lir::Lir;
    use crate::backend::wasm::select::select_function_of;
    let src = "(module m \
                     (def (go (: i Int64) (: n Int64) (: acc Int64)) \
                       (if (< i (* n 2)) (go (+ i 1) n (+ acc (* n 2))) acc)) \
                     (def (f (: x Int64)) (go 0 x 0)) (export f))";
    let mut db = crate::db::Db::load(crate::testkit::parse(src));
    let layout = crate::layout::compute(&mut db).expect("layout");
    let d = db.def_by_name("go").expect("go");
    let ps: Vec<_> = db.defs[d]
        .params
        .clone()
        .into_iter()
        .map(|p| {
            let b = db
                .ast
                .as_form(p, ":")
                .and_then(|t| t.first().copied())
                .unwrap_or(p);
            (b, crate::infer::type_of(&mut db, b))
        })
        .collect();
    let body = db.defs[d].body.expect("body");
    let code = select_function_of(&mut db, body, &ps, &layout, Some(d))
        .expect("select")
        .code;
    assert_eq!(
        code.iter().filter(|i| matches!(i, Lir::I64Shl)).count(),
        1,
        "the invariant `(* n 2)` in both the condition and the body is hoisted ONCE (one i64.shl): \
             {code:?}"
    );
    let loop_ix = code
        .iter()
        .position(|i| matches!(i, Lir::Loop(_)))
        .expect("go compiles to a loop");
    assert_eq!(
        code[loop_ix..]
            .iter()
            .filter(|i| matches!(i, Lir::I64Shl))
            .count(),
        0,
        "no `(* n 2)` remains inside the loop body (the body copy reads the hoisted slot): {code:?}"
    );
}
