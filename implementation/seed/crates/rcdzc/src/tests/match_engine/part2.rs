use crate::compile::compile_component;
use crate::testkit::parse;

use super::*;

// (if_and_match_int_literal_vs_float_offer_a_float_literal_retype_fix migrated to corpus 02-binding-and-control,
// the int-literal-vs-float branch-clash retype-fix block: an `if` clash → CDZ0201, a `match` arm clash →
// CDZ0203, each carrying a replace fix that retypes whichever branch holds the int literal up to a float
// (`(if b 1 2.0)`→"1.0", `(if b 1.0 2)`→"2.0", `(match x (0 1) (_ 2.0))`→"1.0", `(match x (0 1.0) (_ 2))`→"2.0");
// the cross-kind int-vs-bool `(if b 1 true)` carries no fix (CDZ0203 (no-fix)). All 5 PASS wasm.)

// (a_mixed_int_float_arithmetic_operand_is_cdz0301_with_a_conform_to_first_coercion_fix migrated to corpus
// 06-numeric-model, the "conform-to-first" repair block beside the (+ 2 2.0) no-promotion case: float-first
// retypes the trailing int literal up (replacement "1.0"), int-first drops the float fractional form
// (replacement "1"), a computed int second operand wraps (replacement-contains "Float64.of-int"), an int
// literal first against a non-literal float retypes up (replacement "5.0") in both operand orders, and the
// integer-only % rejects a float operand (CDZ0301, no fix). All 6 PASS wasm.)

// (arithmetic_on_a_non_numeric_operand_carries_no_phantom_int64_clash migrated to corpus 07-type-system,
// beside the same-type non-numeric arithmetic family: "same-typed String/List operands to +/% reject without
// a phantom Int64 clash" — CDZ0201 + (not "must be the same type here"), the message-absence now expressible
// via #6146's (not …) sub-form (the test pre-dated it, so it had stayed white-box). All 3 PASS wasm.)

// (a_compound_operand_against_a_scalar_names_the_kind_boundary migrated to corpus 07-type-system, the
// "COMPOUND/SUM/NOMINAL operand against a SCALAR names the KIND BOUNDARY" block: record/tuple/list vs int
// (compare/order → CDZ0201 "different types"+"kind boundary"), user-sum+int / sum=string / nominal+int /
// sum-vs-record (→ CDZ0201 "kind boundary", + (not "Int64 and Color") no-phantom on the sum+int face), plus
// controls "two DIFFERENT user sums keep the generic same-kind mismatch" (CDZ0203) and "a same-sum comparison
// is valid" (runs → false). The record=record-ok + different-record-shapes→CDZ0203 controls are covered by
// 07's existing equality field-set cases. All 9 PASS wasm.)

#[test]
fn arithmetic_or_comparison_with_a_type_value_operand_names_it_not_a_phantom_int64() {
    // Residual: the corpus-inexpressible facets of the type-value-operand reject (the reject-NAMING facets —
    // a Type-value vs a scalar → CDZ0201 "kind boundary" / no phantom `Int64 and Type`, and two Type operands
    // → "arithmetic is not defined on Type" — moved to corpus 07-type-system "an arithmetic op with a
    // user-type VALUE operand …" + siblings). What stays: (a) the NO-relabel control — a bare `(= Int64
    // Int64)` on two Type operands declines its OWN way (CDZ0900, type equality is `Type.eq`), NOT relabeled a
    // kind boundary (the cross-kind guard needs a positive `(message)` companion to grade, awkward for a
    // CDZ0900 decline), and (b) the DEDUP — an arithmetic op on a type value is ONE error, the spanless
    // UNCODED "type value has no runtime form" decline dropped (count-by-code cannot pin a codeless dup).
    // NO false change: a bare `(= Int64 Int64)` (two Types) is NOT relabeled — type equality is
    // `Type.eq`, and the bare `=` on two types keeps its own path (both share the Type kind tag, so the
    // cross-kind guard does not fire).
    let two_types_eq =
        reject_full("(module m (def (main) (if (= Int64 Int64) 1 0)) (export main))")
            .expect("bare = on two types still declines");
    assert!(
        !two_types_eq.message.contains("kind boundary"),
        "bare = on two identical types is not relabeled a kind boundary: {}",
        two_types_eq.message
    );
    // ONE primary error, no cascade: `(+ Color 1)` used to report the CDZ0201 kind-boundary AND a
    // SPANLESS uncoded "a type value has no runtime form" decline (lowering the type-valued operand) —
    // two `error:` lines for ONE root cause. `dedup_faults` now drops the spanless decline whenever the
    // Type-kind-boundary CDZ0201 is present, so an arithmetic op on a type value is a single error.
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "m",
            crate::codec::encode(&parse(
                "(module m (type Color (Red)) (def (main) (+ Color 1)) (export main))",
            )),
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
        "an arithmetic op on a type value = ONE error, not the CDZ0201 + a spanless \
             no-runtime-form decline: {:?}",
        out.diagnostics
    );
    assert_eq!(errors[0].code.as_deref(), Some("CDZ0201"));
    assert!(
        !out.diagnostics
            .iter()
            .any(|d| d.message.contains("type value has no runtime form")),
        "the spanless no-runtime-form decline is suppressed: {:?}",
        out.diagnostics
    );
}

// (a_bool_against_a_number_or_char_names_the_scalar_kind_boundary migrated to corpus 07-type-system, beside
// the `(= 1 true)` int-vs-bool case: "ordering a boolean against a number" / "adding a boolean to a number" /
// "comparing a boolean against a character" → CDZ0203 naming the scalar KIND BOUNDARY (message "between a
// boolean and a number/character") + (not "must be the same type here"); plus "a Char against a number keeps
// its total-conversion wrap fix" (CDZ0203 (fix (kind wrap))). The int-vs-float→CDZ0301 control is 07's
// "ordering an integer against a float" case. All PASS wasm.)

#[test]
fn a_list_match_is_well_formed_or_declines() {
    // A list is OPEN (any length): a match with only fixed-arity arms and no catch-all is
    // NON-EXHAUSTIVE (CDZ0210) — a finite set of lengths cannot cover every list.
    assert_eq!(
        reject_code("(module m (def (main) (match (list 1 2) ((list a b) a))) (export main))")
            .as_deref(),
        Some("CDZ0210")
    );
    // A malformed rest pattern — more than one binder after `..` — is CDZ0201 (a rest pattern is
    // `(list p… .. rest)`, exactly one tail binder).
    assert_eq!(
        reject_code(
            "(module m (def (main) (match (list 1 2 3) ((list x .. r s) x) (_ 0))) (export main))"
        )
        .as_deref(),
        Some("CDZ0201")
    );
    // The non-exhaustive list match now carries an ADD-ARM fix, like the scalar/sum case. FIXED-only
    // (no catch-all) → append a wildcard `(_ (trap "TODO"))` covering every remaining length.
    let find = |body: &str| {
        let src = format!("(module m (def (f (: xs (List Int64))) {body}) (export f))");
        crate::diagnostics(&mut crate::db::Db::load(parse(&src)))
            .into_iter()
            .find(|d| d.code.as_deref() == Some("CDZ0210"))
            .unwrap_or_else(|| panic!("expected CDZ0210 for {body}"))
    };
    let no_catchall = find("(match xs ((list) 0) ((list a) a))");
    assert_eq!(
        no_catchall.fix.as_ref().map(|f| f.replacement.as_str()),
        Some("(_ (trap \"TODO\"))"),
        "fixed-only list match adds a wildcard arm: {}",
        no_catchall.message
    );
    assert_eq!(
        no_catchall.fix.as_ref().map(|f| f.kind),
        Some(crate::abi::FixKind::InsertInto)
    );
    // A REST pattern that leaves the EMPTY list uncovered (`(list a .. r)` covers length ≥ 1) →
    // append the specific missing arm `((list) (trap "TODO"))`.
    let missing_empty = find("(match xs ((list a .. r) a))");
    assert_eq!(
        missing_empty.fix.as_ref().map(|f| f.replacement.as_str()),
        Some("((list) (trap \"TODO\"))"),
        "a rest pattern missing the empty case adds `((list) …)`: {}",
        missing_empty.message
    );
}

#[test]
fn a_list_pattern_must_be_linear_and_refutable_elements_decline() {
    // LINEARITY (`core-semantics.md §145`): a list pattern is a binder position and MUST be linear —
    // `(list a a)` binds `a` twice → CDZ0102, exactly as `(tuple a a)` does. This was previously a
    // SOUNDNESS GAP: the list matcher never ran the linearity check, so `(list a a)` compiled silently.
    assert_eq!(
        reject_code(
            "(module m (def (f xs) (match xs ((list a a) (+ a a)) (_ 0))) \
                         (def (main) (f (list 1 2))) (export main))"
        )
        .as_deref(),
        Some("CDZ0102"),
        "a repeated leading binder is non-linear"
    );
    // A repeat spanning a leading position AND the rest binder is the same CDZ0102 (`(list a b .. a)`).
    assert_eq!(
        reject_code(
            "(module m (def (f xs) (match xs ((list a b .. a) a) (_ 0))) \
                         (def (main) (f (list 1 2 3))) (export main))"
        )
        .as_deref(),
        Some("CDZ0102"),
        "a binder repeated across a leading position and the rest is non-linear"
    );
    // A repeat NESTED inside an element sub-pattern is still caught (`(list (tuple a a) .. r)`).
    assert_eq!(
        reject_code(
            "(module m (def (f xs) (match xs ((list (tuple a a) .. r) a) (_ 0))) \
                         (def (main) (f (list (tuple 1 2)))) (export main))"
        )
        .as_deref(),
        Some("CDZ0102"),
        "a binder repeated inside a nested tuple element is non-linear"
    );

    // A SHAPE-INCOMPATIBLE element (a wrong-arity tuple against a scalar-list element) is a hard reject,
    // NOT a decline: `(list Int64)` elements are scalars, so a `(tuple a b)` element cannot match.
    assert_eq!(
        reject_code(
            "(module m (def (f (: xs (List Int64))) (match xs ((list (tuple a b) .. r) a) (_ 0))) \
                         (export f))"
        )
        .as_deref(),
        Some("CDZ0201"),
        "a tuple element pattern against a scalar list element is a shape error"
    );

    // A refutable SCALAR/STRING LITERAL element NO LONGER declines — it now DISPATCHES by element value
    // (desugars to a fresh binder + a `(= binder <lit>)` guard; see
    // `a_refutable_literal_list_element_dispatches_by_element_value`). So `(list 0 .. r)` with a `_`
    // catch-all COMPILES (no code, no decline). A refutable MULTI-VARIANT CONSTRUCTOR element ALSO
    // now compiles (dispatches by discriminant; see
    // `a_refutable_ctor_list_element_dispatches_by_discriminant`).
    assert_eq!(
        reject_code(
            "(module m (def (f (: xs (List Int64))) (match xs ((list 0 .. r) 1) (_ 0))) \
                                    (def (main) (f (list 0 1))) (export main))"
        ),
        None,
        "a refutable scalar-literal list element now compiles (dispatches by value)"
    );
    assert_eq!(
        reject_code(
            "(module m (type C (A Int64) (B Int64)) \
                   (def (f (: xs (List C))) (match xs ((list (C.A n) .. r) n) (_ 0))) \
                   (def (main) (f (list (C.A 1)))) (export main))"
        ),
        None,
        "a refutable multi-variant-ctor list element now compiles (dispatches by discriminant)"
    );
    // MORE THAN ONE refutable-ctor element in a single arm now COMPILES too: each ctor element gets a
    // fresh binder, all their discriminant-tests are ANDed into the arm guard, and the body re-matches
    // are NESTED (innermost holds the original body, so every ctor payload is in scope). `[A n, B m ..r]`
    // extracts both payloads (`n + m`).
    assert!(
            compile_component(&crate::codec::encode(&parse(
                "(module m (type C (A Int64) (B Int64)) \
                   (def (f (: xs (List C))) (match xs ((list (C.A n) (C.B m) .. r) (+ n m)) (_ 0))) \
                   (def (main) (f (list (C.A 1) (C.B 2)))) (export main))"
            )))
            .is_ok(),
            "two refutable-ctor elements in one arm now compile (gate verifies value = 3)"
        );
}

// (a_list_pattern_on_a_non_list_value_names_the_type_not_the_payload migrated to corpus 05-compound-types: a
// list pattern on a record/tuple is a shape error CDZ0201 naming the type (not the internal "payload" term);
// the valid-list-pattern-no-fault control is covered by the existing 05 list-pattern match cases. PASS wasm.)

// (two_rest_markers_in_one_arm_are_linear migrated to corpus 05-compound-types: a reused rest BINDER across
// sibling rest-lists is non-linear (CDZ0102); the well-formed two-rest positive is the existing 05 "a tuple of
// two rest-lists binds each leading head" case. PASS wasm.)

// (applying_an_effect_name_names_the_category_not_the_leaked_record_type migrated to corpus 07-type-system:
// `(E 5)` → CDZ0201 "`E` is an effect, not a function" (no leaked Record/Any); the literal-head control
// is the existing applying-a-non-function case. PASS wasm.)

// (applying_a_nullary_function_says_it_takes_no_arguments migrated to corpus 09-functions, the
// applying-a-non-function family: "applying a nullary function names it and says it takes no arguments"
// (`(def (g) 5) (g 5)` → CDZ0201 message "takes no arguments"), "…with two surplus arguments pluralizes the
// count" (`(g 5 6)` → message "but 2 were applied"), and "applying a plain value def keeps the type-named
// message, not the nullary-function wording" (`(def v 5) (v 5)` → message "cannot apply a value of type
// Int64"). All three graded PASS on wasm.)

// (applying_a_type_name_names_it_a_type_and_points_at_annotation_position migrated to corpus 07-type-system,
// the applying-a-type-in-expression-position pair: "applying a non-generic prelude type to a value names it a
// type, not a function" (`(Int64 5)` → CDZ0203 message "is a type, not a function") and "applying a generic
// type constructor to a value names the type-argument position" (`(Option 5)` → CDZ0203 message "its type
// argument must be a type"). Both graded PASS on wasm.)

#[test]
fn a_value_juxtaposed_with_a_type_names_the_missing_colon_annotation() {
    // `(5 Int64)` — a value annotation written WITHOUT the colon (the correct form is `(: 5 Int64)`).
    // A plain value head applied to exactly one argument that resolves as a TYPE reads as applying a
    // non-function; previously the generic "cannot apply a value of type Int64 — it is not a function"
    // hid the real cause. It is the value-position twin of the parameter slice `(a Float64)` → `(: a
    // Float64)` and the argument-position counterpart of `(Int64 5)`'s "a type appears in an
    // annotation" message. Now it names the missing-colon repair and carries a HEURISTIC fix with the
    // exact `(: 5 Int64)` spelling (heuristic — the `:` is the certain structural repair, but the
    // annotation may itself not hold, e.g. `(5 Bool)` → a CDZ0203; a Verified fix must clear the
    // diagnostic by construction, which this does only when the value's type satisfies the annotation).
    let d = reject_full("(module m (def (main) (5 Int64)) (export main))")
        .expect("a colon-less value annotation is rejected");
    assert_eq!(d.code.as_deref(), Some("CDZ0201"), "got: {}", d.message);
    assert!(
        d.message.contains("`(: <value> <Type>)`")
            && d.message.contains("leading `:`")
            && d.message.contains("juxtaposed with a type"),
        "names the missing-colon repair: {}",
        d.message
    );
    let fix = d.fix.as_ref().expect("carries an add-`:` fix");
    assert!(
        !fix.verified,
        "heuristic — the annotation is not proven to hold"
    );
    assert_eq!(fix.replacement, "(: 5 Int64)", "the exact repair spelling");

    // A COMPOUND type argument (`(List Int64)`) has no single name atom to splice — the message still
    // names the shape but carries NO fix.
    let dc = reject_full("(module m (def (main) (5 (List Int64))) (export main))")
        .expect("a compound-type colon-less annotation is rejected");
    assert!(
        dc.message.contains("juxtaposed with a type"),
        "compound-type case still names the shape: {}",
        dc.message
    );
    assert!(
        dc.fix.is_none(),
        "no fix when the type is compound: {:?}",
        dc.fix
    );

    // NO false positive: a two-value application `(5 6)` — the argument is NOT a type — keeps the
    // generic "not a function" message (it is a genuine malformed application, not a missing colon).
    let d2 = reject_full("(module m (def (main) (5 6)) (export main))")
        .expect("applying a value to a value is rejected");
    assert!(
        d2.message.contains("cannot apply a value of type") && !d2.message.contains("juxtaposed"),
        "a non-type argument is not hijacked as a missing-colon annotation: {}",
        d2.message
    );

    // NO false positive: the type-in-HEAD form `(Int64 5)` keeps its own category message (a sibling
    // test pins it), and this slice must not shadow it.
    let d3 = reject_full("(module m (def (main) (Int64 5)) (export main))")
        .expect("applying a type name is rejected");
    assert!(
        d3.message.contains("`Int64` is a type, not a function"),
        "type-in-head is unaffected: {}",
        d3.message
    );
}

// (applying_a_monomorphic_sum_type_to_arguments_says_it_takes_no_type_parameters migrated to corpus
// 07-type-system: "annotating with a monomorphic sum applied to a type argument says it takes no type
// parameters" (`(: t (T Int64))` → CDZ0203 + replace-with-`T` fix) and "applying a monomorphic sum in value
// position says it takes no type parameters" (`(Color 5)` → CDZ0203 + replace-with-`Color` fix). Both PASS wasm.)

// (a_prelude_type_constructor_with_the_wrong_arity_names_its_expected_argument_count migrated to corpus
// 07-type-system, the "PRELUDE type-constructor wrong-arity" block: List over-applied, Map under-applied, Set
// over-applied, the value-annotation-site variant, the WIDTH-indexed Int/UInt (zero and two args), Qty (one and
// zero args), and the empty arrow `(->)` — each CDZ0203 naming the ctor + expected-vs-supplied arity. The
// correct-arity no-fault + genuine-non-type "requires a type" controls are covered by the working
// List/Int/Qty/arrow cases across the corpus. All 10 reject cases graded PASS on wasm.)

#[test]
fn a_bare_name_in_a_qty_unit_position_names_it_a_unit_not_a_type() {
    // Residual: the reject-NAMING facets (a bare lowercase/uppercase name in the Qty unit position →
    // CDZ0101 "`Qty`'s second argument is a UNIT" + the `(Unit.base #"…")` replace fix, at a param + a
    // value-annotation site) moved to corpus 18-units-of-measure "a bare lowercase name in the Qty unit
    // position …" + siblings. What stays: the CROSS-DIAGNOSTIC position-awareness — ONE program `(Qty widget
    // meter)` produces TWO distinct-position faults (inner=type guidance, outer=unit) — which a corpus
    // `(error …)` (single primary message) cannot pin; the valid `(Unit.base …)` no-false-change control is
    // covered by corpus 18's valid Qty cases.
    // The INNER (type) position still gets TYPE guidance — a bad inner type + a bad unit produce their
    // OWN distinct messages, not both-as-units.
    let both = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def (g (: q (Qty widget meter))) q) (export g))",
    )));
    assert!(
        both.iter()
            .any(|d| d.message.contains("not a type variable")),
        "the inner Qty position keeps type guidance: {:?}",
        both.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(
        both.iter().any(|d| d.message.contains("not a unit")),
        "the unit Qty position gets the unit message: {:?}",
        both.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // NO false change: a VALID `(Qty Float64 (Unit.base #"meter"))` raises no unit fault.
    assert!(
        !crate::diagnostics(&mut crate::db::Db::load(parse(
            "(module m (def (g (: q (Qty Float64 (Unit.base #\"meter\")))) q) (export g))"
        )))
        .iter()
        .any(|d| d.message.contains("not a unit")),
        "a well-formed Qty unit is not flagged"
    );
}

#[test]
fn a_non_unit_qty_of_arg_unbound_unit_is_not_a_double_report() {
    // The non-unit-second-arg rejects (CDZ0201 "`Qty.of`'s second argument must be a UNIT" for a bare
    // Int / String / tuple) + the valid Unit.base/Unit.one controls migrated to corpus 18-units-of-measure
    // (the Qty.of argument-validation reject cluster). What STAYS here is the corpus-inexpressible
    // no-DOUBLE contrast: a bare UNBOUND unit name (`(Qty.of 5 meter)`, meter undefined) surfaces its OWN
    // CDZ0101 (unbound), NOT ALSO the not-a-unit reject (the check is guarded on the arg being otherwise
    // fault-free) — a no-OTHER-message assertion the corpus (error …) surface cannot express.
    let unbound = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def (main) (Qty.of 5 meter)) (export main))",
    )));
    assert!(
        unbound
            .iter()
            .any(|d| d.message.contains("unbound name `meter`"))
            && !unbound
                .iter()
                .any(|d| d.message.contains("second argument must be a UNIT")),
        "a bare unbound unit gets only its own unbound-name error, not a double: {:?}",
        unbound.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn a_partial_builtin_operation_as_an_unconsumed_value_is_rejected_not_silently_shipped() {
    use crate::testkit::parse;
    // M227 (co-designed with v-inference): a BUILT-IN OPERATION applied at FEWER args than it takes —
    // a partial application `(String.slice s 0)` (slice takes 3), `(String.at s)` (takes 2) — as an
    // UNCONSUMED value (a dead/unexported def body, or an exported def returning the fn) reached
    // NEITHER `collect_reached_poisons` (nullary+exported only) NOR the lower reject, so it shipped
    // unflagged by BOTH `cdz check` and `cdz compile`. Now rejected in the all-bodies `type_errors`
    // walk (a built-in operation needs a runtime closure to be partial — not yet built). The
    // completion test is LOCAL (spine-top via the parent), so an inner partial an outer application
    // saturates is NOT flagged.
    let reject = |src: &str| {
        crate::diagnostics(&mut crate::db::Db::load(parse(src)))
            .into_iter()
            .find(|d| d.message.contains("applied at the wrong arity"))
    };
    for src in [
        "(module m (def (f (: s String)) (String.slice s 0)) (def (main) 0) (export main))",
        "(module m (def (f (: s String)) (String.at s)) (export f))",
        // NESTED-PARENS spine `((String.slice s) 0)` — the SAME application as the flat form (`(f a b)`
        // desugars to `((f a) b)`), so it must reject identically. The immediate head here is the inner
        // `Apply` `(String.slice s)` (whose `meta_apply_of` is `None`), which formerly SKIPPED the check
        // — the PR#491 hole (Copilot-flagged, v-inference-verified). The predicate now flattens the
        // spine to its bottom head first, so the nested and flat surfaces are treated identically.
        "(module m (def (f (: s String)) ((String.slice s) 0)) (def (main) 0) (export main))",
    ] {
        assert!(
            reject(src).is_some(),
            "a partial built-in operation as an unconsumed value must reject: {src}"
        );
    }
    // NO false positive — the tick-108 regression set + the sharp edges v-inference flagged, all must
    // stay clean: a FULL builtin application; a curried CONSTRUCTOR spine that completes; user-function
    // and module-member CURRYING (legitimate — a user fn is partially applicable); UNARY NEGATION
    // (`Sub` at arity 1, the prefix-neg `lower` builds as `0 - e`); ordinary arithmetic.
    for ok in [
        "(module m (def (f (: s String)) (String.slice s 0 1)) (def (main) (f \"hi\")) (export main))",
        "(module m (type P (Mk Int64 Int64)) (def (main) (match ((P.Mk 3) 4) ((P.Mk a b) (+ a b)))) (export main))",
        "(module m (def (g (: x Int64) (: y Int64)) (+ x y)) (def (h) (g 1)) (def (main) 0) (export main))",
        "(do (module lib (def (g (: x Int64) (: y Int64)) (+ x y)) (export g)) (def (h) ((. lib g) 1)) (def (main) 0) (export main))",
        "(module m (def (f (: x Int64)) (- x)) (def (main) (f 5)) (export main))",
        "(module m (def (main) (+ 1 2)) (export main))",
    ] {
        assert!(
            reject(ok).is_none(),
            "a completed / partial-applicable / unary-neg form must NOT be flagged: {ok}"
        );
    }
}

// (a_user_generic_sum_with_the_wrong_type_arg_count_names_its_expected_arity migrated to corpus 07-type-system,
// the "USER generic sum wrong-arity" block: `(Box Int64 Bool)` over-applied (takes 1, 2 supplied), `(Pair Int64)`
// under-applied (takes 2, 1 supplied), and the value-annotation-site `(: 5 (Box Int64 Bool))` — each CDZ0203
// naming the sum arity + fix. The correct-arity-clean + monomorphic-keeps-M108-message controls are covered by
// the working generic-sum cases and the monomorphic-sum cases above. All 3 PASS wasm.)

#[test]
fn applying_a_non_function_reports_one_error_not_a_shadowing_decline() {
    // Applying a non-function must be ONE primary `error:` — the coded `cannot apply a value of
    // type … — it is not a function` — NOT that reject PLUS the emit path's uncoded "value is not
    // applyable" decline for the same node (both surfaced as `error:`, reading as two errors).
    // `dedup_faults` drops the weaker decline when the coded not-a-function reject is present.
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "m",
            crate::codec::encode(&parse("(module m (def (main) (5 3)) (export main))")),
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
        "applying a non-function = one error, got: {:?}",
        out.diagnostics
    );
    assert!(
        errors[0].message.contains("it is not a function"),
        "the surviving error is the coded not-a-function reject: {}",
        errors[0].message
    );
    assert!(
        !out.diagnostics
            .iter()
            .any(|d| d.message == crate::diag::NOT_APPLYABLE_DECLINE),
        "the 'value is not applyable' decline must not accompany the coded reject"
    );
}

#[test]
fn an_ill_typed_try_operand_reports_one_error_not_a_shadowing_constant_decline() {
    // A `?` on a non-fallible operand (`(try 3.14)`) must be ONE primary `error:` — the coded CDZ0203
    // `?` operand must be a fallible `Result`/`Option`, found Float64 — NOT that reject PLUS the emit
    // path's uncoded "the ?/try operator lowers only a constant operand yet" decline. The ill-typed
    // operand's non-sum CONSTANT core misses the `Resolved::Try` `SumNew` fold arm in `lower`, so the
    // decline fired alongside the CDZ0203 — and misleadingly, since the operand IS constant (its problem
    // is the TYPE). `dedup_faults`'s `has_try_non_fallible_reject` gate drops the decline when the CDZ0203
    // is present. (A genuinely-RUNTIME fallible operand — no CDZ0203 — keeps its honest BRICK-3b decline;
    // that path is covered by the runtime-`?`-declines corpus/behavior, not suppressed here.)
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "m",
            crate::codec::encode(&parse("(module m (def (main) (try 3.14)) (export main))")),
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
        "an ill-typed `?` operand = one error, got: {:?}",
        out.diagnostics
    );
    assert_eq!(
        errors[0].code.as_deref(),
        Some("CDZ0203"),
        "the surviving error is the coded non-fallible-operand reject: {}",
        errors[0].message
    );
    assert!(
        !out.diagnostics.iter().any(|d| d
            .message
            .starts_with(crate::diag::TRY_RUNTIME_OPERAND_DECLINE_PREFIX)),
        "the misleading 'lowers only a constant operand yet' decline must not accompany the CDZ0203"
    );
}

#[test]
fn over_applying_a_function_reports_one_error_not_a_shadowing_decline() {
    // Over-application (`(f 1 2)` for a 1-param `f`) must be ONE primary `error:` — the coded CDZ0203
    // `applied 2 arguments to a function of arity 1 …` — NOT that reject PLUS the evaluator's uncoded
    // "applied more arguments than the function accepts" decline for the same node. `dedup_faults`
    // drops the weaker decline when the coded over-application reject is present.
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "m",
            crate::codec::encode(&parse(
                "(module m (def (f (: x Int64)) x) (def (main) (f 1 2)) (export main))",
            )),
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
        "over-application = one error, got: {:?}",
        out.diagnostics
    );
    assert_eq!(errors[0].code.as_deref(), Some("CDZ0203"));
    assert!(
        !out.diagnostics
            .iter()
            .any(|d| d.message == crate::diag::OVER_APPLICATION_DECLINE),
        "the 'applied more arguments' decline must not accompany the coded reject"
    );
}

#[test]
fn over_applying_a_builtin_operation_reports_one_error_with_the_delete_fix() {
    // A BUILT-IN operation over-applied (`(Map.len m x)` — size takes one operand) is the built-in
    // analogue of the user-function case above: `lower` emits the uncoded "`Map.len` is applied at
    // the wrong arity — a built-in operation must be applied to exactly its arguments" decline AND
    // `infer` the coded CDZ0203 over-application (with its delete-surplus fix). They are the same
    // defect — `dedup_faults` now drops the weaker decline WHEN the coded reject is present, so the
    // program reports ONE primary error carrying the fix. (An UNDER-application keeps the decline —
    // no coded sibling — tested separately below.)
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "m",
            crate::codec::encode(&parse(
                "(module m (def (main) ((. Map len) (map (1 2)) 99)) (export main))",
            )),
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
        "built-in over-application = one error, got: {:?}",
        out.diagnostics
    );
    assert_eq!(errors[0].code.as_deref(), Some("CDZ0203"));
    assert_eq!(
        errors[0].fix.as_ref().map(|f| f.kind),
        Some(crate::abi::FixKind::Delete),
        "the surviving over-application error carries the delete-surplus fix: {:?}",
        errors[0].fix
    );
    assert!(
        !out.diagnostics
            .iter()
            .any(|d| d.message.contains(crate::diag::BUILTIN_WRONG_ARITY_DECLINE)),
        "the built-in wrong-arity decline must not accompany the coded reject"
    );
    // An UNDER-application (`(List.at l)`, missing the index) has NO coded sibling — the wrong-arity
    // decline is KEPT (it is the only report; nothing to delete, so no fix).
    let under = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "m",
            crate::codec::encode(&parse(
                "(module m (def (main) ((. List at) (list 1))) (export main))",
            )),
        )],
        &[crate::backend::Target::Wasm],
    );
    assert!(
        under
            .diagnostics
            .iter()
            .any(|d| d.message.contains(crate::diag::BUILTIN_WRONG_ARITY_DECLINE)),
        "an under-application keeps the wrong-arity decline: {:?}",
        under.diagnostics
    );
}

#[test]
fn applying_an_applyable_head_is_not_flagged_as_a_non_function() {
    // The guard must NOT over-reject: a head that is applyable via a `(meta apply)` PRIMITIVE (the
    // `tuple`/`record`/`list` compound-value alias, a type ctor) has no type SCHEME but IS applyable,
    // so it must still COMPILE. `(tuple 1 2)` and `(list 1 2)` build their compounds (reject_code =
    // None = compiled); a record field read likewise. Pins that the non-function check excludes
    // meta-apply heads (the tuple/record/list regression the first cut of this check caused).
    assert_eq!(
        reject_code("(module m (def (main) (tuple 1 2)) (export main))"),
        None
    );
    assert_eq!(
        reject_code("(module m (def (main) (list 1 2)) (export main))"),
        None
    );
    assert_eq!(
        reject_code("(module m (def (main) (. (record (= x 1) (= y 2)) x)) (export main))"),
        None
    );
}

#[test]
fn a_runtime_list_literal_bulk_builds_via_vec_of_arr() {
    // A runtime list literal `(list …)` builds in ONE bulk call: a flat `arr` (`arr-alloc` + a boxed
    // `arr-set` per element) then a single `vec-of-arr` — NOT `vec-empty` + N× consuming `vec-push`.
    // Asserted at the Lir level: exactly one `vec-of-arr`, N `arr-set`s, and ZERO `vec-push`/
    // `vec-empty`. (The list has a runtime element so it is not folded/baked away.) Behavioral value
    // parity is covered by the composed `List.at`/`List.len` runtime tests.
    use crate::backend::wasm::lir::Lir;
    use crate::db::Db;
    let ast = crate::testkit::parse(
        "(module m (def (f (: a Int64) (: n Int64)) \
               (match (List.at (list a (+ a 1) (+ a 2)) n) ((Some x) x) (None -1))) \
               (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = crate::layout::compute(&mut db).expect("layout");
    let d = db.def_by_name("f").expect("def f");
    let sig = db.defs[d].params.clone();
    let params: Vec<_> = sig
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
    let code = crate::backend::wasm::select::select_function(&mut db, body, &params, &layout)
        .expect("select")
        .code;
    let count = |name| {
        code.iter()
            .filter(|i| matches!(i, Lir::CallImport(n) if *n == name))
            .count()
    };
    assert_eq!(count("vec-of-arr"), 1, "one bulk build, got: {code:?}");
    assert_eq!(count("arr-set"), 3, "three elements laid into the arr");
    assert_eq!(count("vec-push"), 0, "no per-element push chain");
    assert_eq!(count("vec-empty"), 0, "no vec-empty seed");
}

// (a_mixed_element_list_is_rejected was already covered by corpus 05-compound-types "a list mixing
// integer and boolean elements is a type error" (`#list(1 true)` → CDZ0201) — redundant, removed.)

#[test]
fn a_bin_value_out_of_range_for_its_segment_is_a_provable_trap() {
    // A constant LITERAL segment value that does not fit its width grounds to the segment's width type
    // and range-checks: it has no encoding → the build fails (CDZ0304), rather than truncating. `(u8
    // 256)` needs 9 bits; `(u8 -1)` is negative into unsigned. A bit-field value wider than its width
    // (`(bits 256 8)`) is the sub-byte companion, also a provable rejection. (A non-literal value that
    // does not fit is a type error, CDZ0203 — see the sibling test.)
    for src in [
        "(module m (def (main) (Bytes.len (bin (u8 256)))) (export main))",
        "(module m (def (main) (Bytes.len (bin (u8 -1)))) (export main))",
    ] {
        assert_eq!(reject_code(src).as_deref(), Some("CDZ0304"), "src: {src}");
    }
    // A byte-aligned bit-field whose value overflows its width is the CDZ0304 fit-trap (aligned, so
    // the well-formedness check passes and the value-fit check fires): `(bits 256 8)` needs 9 bits.
    assert_eq!(
        reject_code("(module m (def (main) (Bytes.len (bin (bits 256 8)))) (export main))")
            .as_deref(),
        Some("CDZ0304"),
    );
    // The message is ACTIONABLE, not the terse "binary value does not fit segment": it names the
    // offending VALUE, the segment's width TYPE, and the VALID RANGE (mirroring the annotation-position
    // CDZ0302), so a bin over-range reads as clearly as a `(: 300 UInt8)` annotation over-range.
    let d = reject_full("(module m (def (main) (Bytes.len (bin (u8 300)))) (export main))")
        .expect("`(u8 300)` over-range rejects");
    assert!(
        d.message.contains("300") && d.message.contains("UInt8") && d.message.contains("0..=255"),
        "the bin over-range message names the value, width type, and range: {}",
        d.message
    );
    // A NON-ALIASED bit-field width spells its type as the `(UInt k)` ctor form (a bare `UInt4` is
    // unbound), and names the k-bit range — `(bits 20 4)` → "the value 20 does not fit … 4-bit
    // (UInt 4) field (the valid range is 0..=15)".
    let bits = reject_full(
        "(module m (def (main) (Bytes.len (bin (bits 20 4) (bits 0 4)))) (export main))",
    )
    .expect("`(bits 20 4)` over-range rejects");
    assert!(
        bits.message.contains("20")
            && bits.message.contains("(UInt 4)")
            && bits.message.contains("0..=15"),
        "a non-aliased bit-field over-range names the `(UInt k)` type + range: {}",
        bits.message
    );
    // A SIGNED segment names the signed type + its (negative-inclusive) range — `(i8 200)` overflows
    // Int8's -128..=127.
    let signed = reject_full("(module m (def (main) (Bytes.len (bin (i8 200)))) (export main))")
        .expect("`(i8 200)` over-range rejects");
    assert!(
        signed.message.contains("Int8") && signed.message.contains("-128..=127"),
        "a signed segment names the signed type + range: {}",
        signed.message
    );
}

// (an_ill_formed_bin_form_is_rejected_cdz0220 migrated to corpus 16-binary-matching, the structural
// well-formedness reject block after the CDZ0304 value-fit case: bit-fields not closing to a whole byte
// → CDZ0220 (message "total 4 bits")(message "add 4 more bits to reach 1 byte"); a non-final unsized
// bytes segment → CDZ0220; a bit-field width that is negative / non-constant → CDZ0220 (message
// "bit-field width must be a compile-time constant natural"). --case grades codes + messages (all 4 PASS).)
#[test]
fn a_non_byte_aligned_int_bin_segment_names_the_supported_widths() {
    // A fixed-width integer bin segment is one of the byte-aligned widths — `u8/u16/u32/u64` or
    // `i8/i16/i32/i64`. A `uNN`/`iNN` head with any OTHER width (`u24`, `u7`, `u128`, `i0`) IS the
    // `uNN` SHAPE the generic "unrecognized kind (expected uNN/iNN/…)" message points at, so that
    // message misled — it told the author to write what they already wrote. Now such a head names the
    // real limit (the supported widths) and points a non-byte-aligned width at the `(bits v k)` segment.
    for bad in ["u24", "u17", "u7", "u128", "i24", "i0"] {
        let d = reject_full(&format!(
            "(module m (def (main) (bin ({bad} 1))) (export main))"
        ))
        .unwrap_or_else(|| panic!("`{bad}` must reject"));
        assert_eq!(d.code.as_deref(), Some("CDZ0201"), "{bad}: {}", d.message);
        assert!(
            d.message.contains("u8/u16/u32/u64")
                && d.message.contains("(bits v k)")
                && d.message.contains(bad),
            "`{bad}` names the supported widths + the bits alternative: {}",
            d.message
        );
    }
    // A GENUINELY unrecognized kind (not the `uNN`/`iNN` shape) keeps the generic message — no
    // over-reach onto a `u`/`i` with no digits or an arbitrary word.
    for generic in ["frob", "u", "i", "xyz"] {
        let d = reject_full(&format!(
            "(module m (def (main) (bin ({generic} 1))) (export main))"
        ))
        .unwrap_or_else(|| panic!("`{generic}` must reject"));
        assert!(
            d.message.contains("unrecognized bin segment kind"),
            "`{generic}` keeps the generic kind message: {}",
            d.message
        );
    }
    // The valid byte-aligned widths still parse (no false positive).
    for ok in ["u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64"] {
        assert!(
            reject_code(&format!(
                "(module m (def (main) (if (= (bin ({ok} 1)) (bin ({ok} 1))) 1 0)) (export main))"
            ))
            .is_none(),
            "`{ok}` is a valid integer segment width"
        );
    }
    // APPLYABLE FIX: a `uNN`/`iNN` width that is a CONFIDENT near-miss of a real byte-aligned kind
    // carries a rename fix on the kind head — `u166`→`u16`, `i17`→`i16` (same signedness). The message
    // still names `(bits v k)`, so an author who wanted a genuine non-aligned width sees that route too.
    for (typo, want) in [("u166", "u16"), ("i17", "i16")] {
        let d = reject_full(&format!(
            "(module m (def (main) (bin ({typo} 1))) (export main))"
        ))
        .unwrap_or_else(|| panic!("`{typo}` must reject"));
        let fix = d
            .fix
            .as_ref()
            .unwrap_or_else(|| panic!("`{typo}` carries a width-rename fix: {}", d.message));
        assert_eq!(fix.kind, crate::abi::FixKind::Replace);
        assert_eq!(fix.replacement, want, "`{typo}` renames to the near width");
        assert!(!fix.verified, "a width-typo guess is heuristic");
    }
    // A width TOO FAR from any byte-aligned kind keeps the guidance but NO misleading rename fix
    // (`u128`/`u9999` — the author likely wants `(bits v k)`, not a one-token width swap).
    for far in ["u128", "u9999"] {
        let d = reject_full(&format!(
            "(module m (def (main) (bin ({far} 1))) (export main))"
        ))
        .unwrap_or_else(|| panic!("`{far}` must reject"));
        assert!(
            d.fix.is_none(),
            "`{far}` is beyond the typo cutoff — no rename fix, just the `(bits v k)` guidance: {:?}",
            d.fix
        );
    }
}

// (a_bin_pattern_over_a_non_bytes_scrutinee_is_a_type_error migrated to corpus 16-binary-matching, in the
// bin-pattern MATCH section: a `(bin …)` pattern over a definite non-Bytes scrutinee (Int64/String/List)
// → CDZ0203 (message "`(bin …)` pattern matches a Bytes value")(message <scrutinee type>); + the
// no-false-reject control (bin over a Bytes param is not a type error — it declines only on the
// non-scalar-param boundary). --case grades the codes + messages + the bare declines.)
// (a_structural_pattern_over_a_mismatched_scrutinee_kind_is_a_type_error migrated to corpus
// 05-compound-types, next to "a map pattern over a non-map scrutinee is a type error": a list/map/tuple
// pattern over a definite scrutinee of a DIFFERENT kind → CDZ0203 (message "a List/Map/Tuple value")
// (message <scrutinee type>) — 6 rejects (list-over-Int/String/Map, map-over-Int, tuple-over-Int/Map) +
// 3 no-over-rejection controls that MATCH + RUN (list→1, map→10, tuple→7; the controls build the
// collection as a constant inside a nullary main, since a collection PARAMETER to an export declines).
// --case grades the codes + messages + run values.)
// (a_bytes_match_with_only_a_bin_arm_and_no_catch_all_is_non_exhaustive migrated to corpus 16-binary-matching: CDZ0210. PASS wasm.)

#[test]
fn a_runtime_bin_construction_builds_and_range_checks_under_wasmtime() {
    // A fixed-width `(bin …)` segment REQUIRES the width-matching typed value (`(u8 v)` takes UInt8,
    // `(bits v k)` takes `(UInt k)`), so a value that does not fit is a COMPILE-TIME TYPE error (CDZ0203),
    // never a runtime trap — the caller narrows with `UInt8.wrap`/`(UInt k).wrap`. This test keeps only
    // that compile-time segment-type guard (a diagnostic assertion the corpus cannot express). The runtime
    // CONSTRUCTION behaviors — u16 big/little-endian, multi-segment, signed two's-complement, UInt8.wrap
    // narrowing, a length-prefixed `(bytes …)` frame splice, and `(bits …)` bit-field packing — are
    // corpus-covered by 16-binary-matching ("a u16 segment encodes big-endian by default" / "the le
    // modifier encodes a u16 little-endian" / "a multi-segment bin concatenates mixed-width signed and
    // unsigned segments in order" / the i8 two's-complement case / "bit-field segments pack sub-byte
    // values into one byte" / "a length-prefixed frame is built from a size segment and a bytes segment" /
    // "a runtime bin construction result compares equal to the Bytes it builds").
    let rejects_0203 = |src: &str| {
        let ds = crate::diagnostics(&mut crate::db::Db::load(crate::testkit::parse(src)));
        assert!(
            ds.iter().any(
                |d| d.code.as_deref() == Some("CDZ0203") && d.message.contains("segment takes")
            ),
            "expected a CDZ0203 segment type error for {src}, got: {ds:?}"
        );
    };
    // A wider runtime value (`Int64`) into a fixed-width `u8` segment is a compile-time type error.
    rejects_0203("(module m (def (main (: n Int64)) ((. Bytes len) (bin (u8 n)))) (export main))");
    // Likewise a wider value into a `(bits _ 4)` bit-field segment.
    rejects_0203(
        "(module m (def (main (: n Int64)) ((. Bytes len) (bin (bits n 4) (bits 5 4)))) (export main))",
    );
}

#[test]
fn bytes_of_out_of_range_element_is_a_width_error() {
    // `Bytes.of : (List UInt8) → Bytes` — a byte IS a UInt8, so an element outside 0..=255 is not a
    // UInt8 and is rejected as an OUT-OF-RANGE WIDTH literal (CDZ0302), NOT a runtime trap: under the
    // UInt8 model the ill-typed byte cannot be constructed. 256 is too large, -1 negative. To truncate
    // a wider value into a byte, the program writes `(UInt8.wrap n)` explicitly (total, never traps).
    // (Used via `len` so `main` returns a scalar.)
    assert_eq!(
        reject_code(
            "(module m (def (main) ((. Bytes len) ((. Bytes of) (list 256)))) (export main))"
        )
        .as_deref(),
        Some("CDZ0302")
    );
    assert_eq!(
        reject_code(
            "(module m (def (main) ((. Bytes len) ((. Bytes of) (list -1)))) (export main))"
        )
        .as_deref(),
        Some("CDZ0302")
    );
    // The reject NAMES the truncation `UInt8.wrap` AND now offers it as a structural fix — wrap the
    // offending element in `(UInt8.wrap …)` (which truncates to the low 8 bits). Anchored at the
    // element, not the whole `Bytes.of` / list, and it VERIFIES: applying it recompiles clean.
    let d = reject_full(
        "(module m (def (main) ((. Bytes len) ((. Bytes of) (list 256)))) (export main))",
    )
    .expect("must reject");
    let fix = d.fix.as_ref().expect("a UInt8.wrap fix is carried");
    assert_eq!(fix.kind, crate::abi::FixKind::Wrap, "a wrap fix: {:?}", fix);
    assert!(
        fix.replacement.contains("UInt8") && fix.replacement.contains("wrap"),
        "the fix wraps in UInt8.wrap: {}",
        fix.replacement
    );
    // Applying the truncation makes the program compile (the byte is now in range by construction).
    assert!(
            compile_component(&crate::codec::encode(&parse(
                "(module m (def (main) ((. Bytes len) ((. Bytes of) (list ((. UInt8 wrap) 256))))) (export main))"
            )))
            .is_ok(),
            "the UInt8.wrap'd element compiles"
        );
}

#[test]
fn a_single_use_list_consume_stays_on_the_fbip_fast_path_no_dup() {
    // The FBIP fast path MUST be preserved: a list bound and consumed EXACTLY once (no later use) needs
    // NO retain — the single `List.push` spends the sole reference in place. `build 0 2` = `[0 1]`,
    // pushed to `[0 1 9]`, length 3. The emitted body must import NO `dup` (a single-use consume
    // produces no retain site; a spurious dup+drop pair would regress the allocation bench). Pins that
    // `collect_dup_sites` marks only a consume WITH a later use. The run value-correctness (a single owned
    // `List.push` then `List.len`) is corpus-covered by 05 "List.len over an owned-temporary List.push
    // result reclaims it"; this keeps only the dup-absence bench guard the corpus cannot express.
    let src = "(module m \
               (def (build i n out) (if (< i n) (build (+ i 1) n ((. List push) out i)) out)) \
               (def (main) (let ((e (build 0 2 (list)))) ((. List len) ((. List push) e 9)))) (export main))";
    assert!(
        !component_imports_op(&component(src), "dup"),
        "a single-use consume must not import `dup` (FBIP fast path — no retain site)"
    );
}

#[test]
fn a_float_element_into_an_empty_set_imports_box_float_not_box_int() {
    // MISCOMPILE (invalid wasm): `Set.insert (Set.of (list)) x` with `x : Float64` — a single float
    // insert into a runtime EMPTY set — imported `box-int` (the import collector's `box_op_ty(elem_ty)`
    // DEFAULTS an unresolved `Var` element type to `box-int`) while the emit's node-aware `box_op_for`
    // called `box-float` → `box-float` un-imported → `call u32::MAX` → invalid component. Fix: the
    // collector's Set/Map insert arms use `box_op_for` (node-aware). Assert the component imports
    // `box-float` (and not the wrong default) so it links. A CONSTANT float set folds (no insert), so
    // the empty runtime base is the trigger.
    let set_f64 = "(module m \
               (def (mk (: x Float64)) ((. Set insert) ((. Set of) (list)) x)) \
               (def (main (: d Float64)) ((. Set len) (mk d))) (export main))";
    assert!(
        component_imports_op(&component(set_f64), "box-float"),
        "a Float64 set element must import `box-float` (else the emit's box-float call is unresolved)"
    );
    // f32 → box-float32; and a float map VALUE into an empty map likewise.
    let set_f32 = "(module m \
               (def (mk (: x Float32)) ((. Set insert) ((. Set of) (list)) x)) \
               (def (main (: d Float32)) ((. Set len) (mk d))) (export main))";
    assert!(
        component_imports_op(&component(set_f32), "box-float32"),
        "a Float32 set element must import `box-float32`"
    );
    let map_f64 = "(module m \
               (def (mk (: x Float64)) ((. Map insert) (map) 0 x)) \
               (def (main (: d Float64)) ((. Map len) (mk d))) (export main))";
    assert!(
        component_imports_op(&component(map_f64), "box-float"),
        "a Float64 map value into an empty map must import `box-float`"
    );
}

#[test]
fn a_single_consume_threaded_accumulator_stays_dup_free() {
    // FBIP fast-path bench guard (backend-shape witness, NOT corpus-expressible): the
    // simultaneously-live-args RETAIN — a self-recursive call threading `base` UNCHANGED in one arg while a
    // sibling consumes it (`List.push base 99`) — must fire a `dup` (else it FBIP-mutates the shared `base`
    // and the threaded copy drifts). But a SINGLE-consume threaded accumulator, where `out` is consumed
    // EXACTLY ONCE in ONE arg with no sibling use (`build 0 n (List.push out i)`), must NOT dup, or the FBIP
    // fast path + alloc bench regress. A `dup` import here is invisible to a value or live-objects check.
    // The drift-CORRECTNESS run (the retain witness, value 3*m over m=1..4, plus the drift m=3→12-not-9)
    // lives in corpus 05 as "a heap arg threaded UNCHANGED to a self-recursive call while a sibling arg
    // consumes it is retained"; this keeps only the dup-absence guard the corpus cannot express.
    let accum = "(module m (def (build i n out) (if (< i n) (build (+ i 1) n ((. List push) out i)) out)) \
               (def (main) ((. List len) (build 0 5 (list)))) (export main))";
    assert!(
        !component_imports_op(&component(accum), "dup"),
        "a single-consume threaded accumulator must not import `dup` (FBIP fast path — bench guard)"
    );
}

#[test]
fn a_single_consume_sum_payload_with_a_dead_scrutinee_stays_dup_free() {
    // FBIP fast-path bench guard (backend-shape witness, NOT corpus-expressible): a sum-match payload
    // binder lowers to `Core::SumPayload` = a BORROW of the scrutinee's payload. When that payload is
    // consumed EXACTLY ONCE and the scrutinee is NOT otherwise live, the retain must NOT fire — the
    // module must stay `dup`-free (a `dup` import here is a perf regression, invisible to a value or
    // live-objects check since dup vs no-dup give the same result). The drift-CORRECTNESS run cases
    // (payload consumed while the scrutinee is still live → 5 / 12) live in corpus 05 as `spr1`/`spr2`;
    // this keeps the dup-absence guard the corpus cannot express. (`is_heap_type` counting a
    // `Ty::Nominal` — a single-variant newtype erasing to its heap inner — is the retain-candidate
    // rule the corpus run cases exercise.)
    let linear = "(module m (type Box (B (List Int64))) \
               (def (f bx) ((. List len) ((. List push) (match bx ((B xs) xs)) 9))) \
               (def (main) (f (B (list 1 2)))) (export main))";
    assert!(
        !component_imports_op(&component(linear), "dup"),
        "a single-consume payload with a dead scrutinee must not import `dup` (FBIP fast path)"
    );
}

#[test]
fn a_single_consume_option_expect_with_a_dead_scrutinee_stays_dup_free() {
    // FBIP fast-path bench guard (backend-shape witness, NOT corpus-expressible): `Option.expect s`
    // reads `sum-payload` (a BORROW). When that payload is consumed EXACTLY ONCE and the Option is NOT
    // otherwise live, the retain must NOT fire — the module must stay `dup`-free (a `dup` import here is
    // a perf regression, invisible to a value or live-objects check). The drift-CORRECTNESS run cases
    // (Option.expect payload consumed while `s` is threaded live → 12, single + chained) and the
    // Unit-payload sentinel-drop validity cases (→ 4) live in corpus 05 as `ope1`..`ope4`; this keeps
    // only the dup-absence guard the corpus cannot express.
    let linear = "(module m \
               (def (f (: s (Option (List Int64)))) ((. List len) ((. List push) ((. Option expect) s \"v\") 9))) \
               (def (main (: d Int64)) (f (Some ((. List push) (list) d)))) (export main))";
    assert!(
        !component_imports_op(&component(linear), "dup"),
        "a single-consume Option.expect with a dead scrutinee must not import `dup` (FBIP fast path)"
    );
}

#[test]
fn a_string_param_threaded_and_concatenated_in_a_selfrec_loop_is_retained_not_freed() {
    // FBIP fast-path bench guard (backend-shape, NOT corpus-expressible): a SINGLE-consume String.concat
    // (no threading) must NOT import `dup`. The RETAIN drift-CORRECTNESS run — a String PARAM threaded
    // UNCHANGED through a self-recursive loop AND consumed by `String.concat` each step, which was an OOB
    // memory-access TRAP at n≥4 before `is_heap_type` gained `String`/`Symbol` (a String is a heap rope
    // exactly as Bytes is) — lives in corpus 13 as "a String param threaded UNCHANGED to a self-call AND
    // consumed by String.concat each step is retained" (n=4/8 → 4/8); this keeps only the dup-absence
    // guard the corpus cannot express.
    let single = "(module m (def (f (: a String)) ((. String byte-len) ((. String concat) a \"y\"))) \
               (def (main) (f \"x\")) (export main))";
    assert!(
        !component_imports_op(&component(single), "dup"),
        "a single-consume String.concat must not import `dup` (FBIP fast path — bench guard)"
    );
}

// (a_list_push_type_mismatch_is_rejected was already covered by corpus 05-compound-types "pushing an
// element of a different type onto a list is a type error" ((List.push #list(1 2) true) → CDZ0201) —
// redundant, removed.)

#[test]
fn every_collection_receiver_op_takes_its_receiver_first() {
    // DRIFT GUARD for the operator's consistent-arg-order directive (concierge 9699/9758): every
    // collection / text / compound RECEIVER-op's scheme MUST carry the receiver (the List/Map/Set/
    // String/Bytes it operates on) as ARG-0, uniform with push/at/update/prepend and pipeline-friendly
    // (a data-first pipe passes the receiver as the first argument). A new prelude op that puts the
    // element/key/index before the receiver FAILS this test rather than shipping an inconsistent
    // surface. Constructors-from-scalars (`Rational.of`, `Char.from-int`) and unary conversions are
    // EXCLUDED (they have no receiver) — this asserts only the multi-arg receiver-ops.
    //
    // Mechanism: a def whose body is the bare member op `(. Module op)` infers to the op's arrow; the
    // OUTERMOST arrow's parameter is arg-0. We assert that parameter's `Ty` is the receiver kind. (A
    // bare-value member op reduces to its `(meta t)` scheme's arrow — the same arrow an application
    // unifies against.)
    use crate::testkit::parse;
    use crate::ty::Ty;
    // (module, op, receiver-kind predicate on the arg-0 Ty). Only multi-arg receiver-ops; a nullary
    // (`Map.empty`) or unary op (`List.len`) still has the receiver in arg-0 where it takes one.
    #[allow(clippy::type_complexity)]
    let cases: &[(&str, &str, fn(&Ty) -> bool, &str)] = &[
        ("List", "push", |t| matches!(t, Ty::List(_)), "List"),
        ("List", "prepend", |t| matches!(t, Ty::List(_)), "List"),
        ("List", "concat", |t| matches!(t, Ty::List(_)), "List"),
        ("List", "update", |t| matches!(t, Ty::List(_)), "List"),
        ("List", "at", |t| matches!(t, Ty::List(_)), "List"),
        ("List", "len", |t| matches!(t, Ty::List(_)), "List"),
        ("Map", "insert", |t| matches!(t, Ty::Map(..)), "Map"),
        ("Map", "lookup", |t| matches!(t, Ty::Map(..)), "Map"),
        ("Map", "remove", |t| matches!(t, Ty::Map(..)), "Map"),
        ("Map", "swap", |t| matches!(t, Ty::Map(..)), "Map"),
        ("Map", "take", |t| matches!(t, Ty::Map(..)), "Map"),
        ("Set", "contains", |t| matches!(t, Ty::Set(_)), "Set"),
        ("Set", "insert", |t| matches!(t, Ty::Set(_)), "Set"),
        ("Set", "remove", |t| matches!(t, Ty::Set(_)), "Set"),
        ("Set", "union", |t| matches!(t, Ty::Set(_)), "Set"),
        ("Set", "intersection", |t| matches!(t, Ty::Set(_)), "Set"),
        ("Set", "difference", |t| matches!(t, Ty::Set(_)), "Set"),
        ("Bytes", "at", |t| matches!(t, Ty::Bytes), "Bytes"),
        ("Bytes", "concat", |t| matches!(t, Ty::Bytes), "Bytes"),
        ("Bytes", "slice", |t| matches!(t, Ty::Bytes), "Bytes"),
        ("String", "at", |t| matches!(t, Ty::String), "String"),
        ("String", "scalar-at", |t| matches!(t, Ty::String), "String"),
        ("String", "concat", |t| matches!(t, Ty::String), "String"),
        ("String", "slice", |t| matches!(t, Ty::String), "String"),
    ];
    for (module, op, is_receiver, kind) in cases {
        let src =
            format!("(module m (def (probe) (. {module} {op})) (def (main) 0) (export main))");
        let mut db = crate::db::Db::load(parse(&src));
        let d = db.def_by_name("probe").expect("probe def");
        let body = db.defs[d].body.expect("probe body");
        let ty = crate::infer::type_of(&mut db, body);
        match &ty {
            Ty::Fn(arg0, _) => assert!(
                is_receiver(arg0),
                "prelude arg-order drift: `{module}.{op}` arg-0 is {} — expected the {kind} receiver \
                     (operator's consistent receiver-first directive, concierge 9699)",
                arg0.render_name(&db.name_ctx())
            ),
            other => panic!(
                "`{module}.{op}` did not infer to an arrow (got {}); the drift guard needs a \
                     receiver-op with an arrow scheme",
                other.render_name(&db.name_ctx())
            ),
        }
    }
}

// (a_list_prepend_type_mismatch_is_rejected migrated to corpus 05-compound-types "prepending an element
// of a different type onto a list is a type error" ((List.prepend #list(1 2) true) → CDZ0201) — the one
// uncovered list-op homogeneity face, added next to the push/update cases.)

// (a_list_update_type_mismatch_is_rejected was already covered by corpus 05-compound-types "updating a
// list slot with an element of a different type is a type error" ((List.update #list(1 2 3) 1 true) →
// CDZ0201) — redundant, removed.)

#[test]
fn a_multi_use_let_bound_list_is_built_once() {
    // A `let`-bound list used at MORE THAN ONE site is a runtime computation worth naming: it is
    // built ONCE (a flat `arr` + one `vec-of-arr`) and the handle reused, not rebuilt at every use.
    // Asserted at the Lir level: the emitted body contains exactly ONE `vec-of-arr` (the list-build
    // op) despite `xs` being read at two `List.at` sites. (Before the keep-binding fix a list binding
    // was not kept, so each use rebuilt it → two builds.)
    use crate::backend::wasm::lir::Lir;
    use crate::db::Db;
    let ast = crate::testkit::parse(
        "(module m (def (f (: i Int64) (: j Int64)) \
               (let ((xs (list 10 20 30))) \
                  (match (List.at xs i) \
                    ((Some x) (match (List.at xs j) ((Some y) (+ x y)) (None x))) \
                    (None -1)))) \
               (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = crate::layout::compute(&mut db).expect("layout");
    let d = db.def_by_name("f").expect("def f");
    let sig = db.defs[d].params.clone();
    let params: Vec<_> = sig
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
    let code = crate::backend::wasm::select::select_function(&mut db, body, &params, &layout)
        .expect("select")
        .code;
    let builds = code
        .iter()
        .filter(|i| matches!(i, Lir::CallImport("vec-of-arr")))
        .count();
    assert_eq!(
        builds, 1,
        "a multi-use let-bound list is built once (one vec-of-arr), got: {code:?}"
    );
}

#[test]
fn a_tail_recursive_list_fold_compiles_to_a_constant_stack_loop() {
    // A tail-recursive fold over a LIST — `(sa xs acc) = (match xs ((list) acc) ((list x .. rest) (sa
    // rest (+ acc x))))` — is a self-tail-call inside a `Core::MatchList` cons arm. The loop transform
    // now threads tail position into list-match arms (`emit_tail`/`body_has_member_tail_call`/
    // `tail_callees` handle `MatchList`), so it compiles to ONE `loop` (constant stack) instead of a
    // stack-growing recursive `call`. Pins the `loop` at the Lir level + value parity + that a large
    // list (which would overflow a recursive stack) folds fine.
    use crate::backend::wasm::lir::Lir;
    use crate::backend::wasm::select::select_function_of;
    let src = "(module m (def (sa (: xs (List Int64)) (: acc Int64)) \
                     (match xs ((list) acc) ((list x .. rest) (sa rest (+ acc x))))) \
                     (def (f (: a Int64) (: b Int64)) (sa (list a b) 0)) (export f))";
    let mut db = crate::db::Db::load(crate::testkit::parse(src));
    let layout = crate::layout::compute(&mut db).expect("layout");
    let d = db.def_by_name("sa").expect("sa");
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
    // `select_function_of` with `self_def = Some(d)` enables the self-recursion loop transform.
    let code = select_function_of(&mut db, body, &ps, &layout, Some(d))
        .expect("select")
        .code;
    assert!(
        code.iter().any(|i| matches!(i, Lir::Loop(_))),
        "the tail list fold compiles to a loop, got: {code:?}"
    );
    assert!(
        !code
            .iter()
            .any(|i| matches!(i, Lir::Call(_) | Lir::ReturnCall(_))),
        "no residual self-`call`/`return_call` — the recursion became a loop br, got: {code:?}"
    );
}

#[test]
fn a_non_tail_list_fold_is_accumulator_transformed_into_a_constant_stack_loop() {
    // THE USER'S EXACT PROGRAM: `(def (sum xs) (match xs ((list) 0) ((list x .. rest) (+ x (sum
    // rest)))))`. The recursive call `(sum rest)` sits in an OPERAND of `+`, so it is NOT a tail call
    // — it would compile to a stack-growing `call` (a long list overflows). `accum::introduce` now
    // recognizes the LIST-FOLD shape (empty arm = the `+` identity `0`, cons arm = the combine, self
    // call threading `rest` through the scrutinee position) and rewrites it to a TAIL accumulator
    // `(sum$acc xs acc) = (match xs ((list) acc) ((list x .. rest) (sum$acc rest (+ acc x))))` +
    // reseeds `sum` to `(sum$acc xs 0)`. The MatchList loop transform (Bug-1 fix) then compiles the
    // synthesized accumulator to a `loop` — so the user's natural non-tail sum runs in O(1) stack.
    //
    // First: the transform fired — a `sum$acc` def was synthesized and `sum` still takes one param.
    let db = crate::db::Db::load(crate::testkit::parse(
        "(module m (def (sum xs) \
               (match xs ((list) 0) ((list x .. rest) (+ x (sum rest))))) (export sum))",
    ));
    assert!(
        db.def_by_name("sum$acc").is_some(),
        "the non-tail list fold gained a synthesized accumulator def"
    );

    // The synthesized accumulator compiles to a LOOP (no residual self-`call`/`return_call`). Select
    // `sum$acc` with `self_def` set (as the pipeline does) and pin the `loop` at the Lir level.
    use crate::backend::wasm::lir::Lir;
    use crate::backend::wasm::select::select_function_of;
    let mut db = crate::db::Db::load(crate::testkit::parse(
        "(module m (def (sum (: xs (List Int64))) \
               (match xs ((list) 0) ((list x .. rest) (+ x (sum rest))))) \
             (def (f (: a Int64) (: b Int64)) (sum (list a b))) (export f))",
    ));
    let layout = crate::layout::compute(&mut db).expect("layout");
    let acc = db
        .def_by_name("sum$acc")
        .expect("accumulator synthesized for the annotated fold");
    let ps: Vec<_> = db.defs[acc]
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
    let body = db.defs[acc].body.expect("acc body");
    let code = select_function_of(&mut db, body, &ps, &layout, Some(acc))
        .expect("select")
        .code;
    assert!(
        code.iter().any(|i| matches!(i, Lir::Loop(_))),
        "the synthesized accumulator compiles to a loop, got: {code:?}"
    );
    assert!(
        !code
            .iter()
            .any(|i| matches!(i, Lir::Call(_) | Lir::ReturnCall(_))),
        "no residual self-`call`/`return_call` in the accumulator — it became a loop br, got: {code:?}"
    );
}

#[test]
fn a_runtime_string_match_without_a_wildcard_is_non_exhaustive() {
    // A String is OPEN (like Int) — no finite literal set exhausts it — so a string match MUST end in a
    // wildcard, else CDZ0210 (the same rule an open-Int match follows). This holds for a runtime string
    // too (the desugar-to-if-chain does not relax exhaustiveness).
    let code = |body: &str| -> Option<String> {
        let src = format!("(module m (def (op (: s String)) {body}) (export op))");
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
    assert_eq!(
        code("(match s (\"add\" 1) (\"sub\" 2))").as_deref(),
        Some("CDZ0210"),
        "a wildcard-less string match is non-exhaustive"
    );
    // With a wildcard it compiles.
    assert_eq!(code("(match s (\"add\" 1) (\"sub\" 2) (_ 0))"), None);
}

#[test]
fn a_guarded_list_arm_does_not_count_toward_exhaustiveness() {
    // A guarded list arm may fail its guard, so it covers NO length unconditionally — a match whose only
    // tail-covering arm is GUARDED is non-exhaustive (CDZ0210), exactly as a guarded scalar/sum tail is.
    // `(match xs ((guard (list .. all) (> (List.len all) 0)) 1))` — the guarded catch-all does not close
    // coverage, so the empty list (and a failing guard) are uncovered.
    let code = |body: &str| -> Option<String> {
        let src = format!("(module m (def (f (: xs (List Int64))) {body}) (export f))");
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
        // Skip the umbrella CDZ0900 not-yet decline (seq-286) — here the non-scalar `(List Int64)` entry
        // param hits the "non-scalar entry parameter" CDZ0900 on the export path (#6101) regardless of the
        // match under test; surface the exhaustiveness/pattern code (CDZ0210/…), not that boundary not-yet.
        out.diagnostics
            .iter()
            .filter(|d| d.severity == crate::abi::Severity::Error)
            .find(|d| d.code.as_deref() != Some("CDZ0900"))
            .and_then(|d| d.code.clone())
    };
    // A guarded rest-arm alone is non-exhaustive (its guard may fail).
    assert_eq!(
        code("(match xs ((guard (list x .. rest) (> x 0)) 1))").as_deref(),
        Some("CDZ0210"),
        "a lone guarded list arm does not cover every length"
    );
    // Adding an UNGUARDED catch-all makes it exhaustive again (no over-rejection).
    assert_eq!(
        code("(match xs ((guard (list x .. rest) (> x 0)) 1) (_ 0))"),
        None,
        "a guarded arm plus an unguarded catch-all is exhaustive"
    );
}

#[test]
fn a_refutable_literal_list_element_still_requires_a_catch_all() {
    // A literal element is a value TEST that may not match, so — like any guarded arm — it does NOT
    // count toward length-coverage exhaustiveness. `(match xs ((list 0 .. r) 1) ((list _ .. r) 2))`
    // leaves the empty list uncovered AND the `(list 0 .. r)` arm's non-zero-head case relies on the
    // second arm, but there is no arm covering length 0 → CDZ0210 (a `_`/`(list)` arm is required).
    assert_eq!(
        reject_code(
            "(module m (def (f (: xs (List Int64))) \
                   (match xs ((list 0 .. r) 1) ((list _ .. r) 2))) \
                 (def (main) (f (list 0))) (export main))"
        )
        .as_deref(),
        Some("CDZ0210"),
        "a refutable-literal-element match still needs a catch-all covering every length"
    );
}

// (a_bool_list_match_missing_a_lead_value_or_the_empty_arm_still_rejects migrated to corpus
// 05-compound-types, the saturation-soundness reject block after the bool/ctor-lead-saturating cases:
// a bool-lead match covering only one bool value → CDZ0210 (the other first-element value uncovered);
// no empty arm → CDZ0210 (length 0 uncovered). --case grades the reject codes.)
#[test]
fn a_sum_variants_list_payload_split_across_empty_and_rest_arms_is_exhaustive() {
    // A sum variant whose LIST PAYLOAD is refined by MULTIPLE arms that jointly cover every length —
    // `(Bx (list)) [len 0] + (Bx (list x .. r)) [len ≥ 1]` — is TOTAL for the `Bx` variant without a
    // `_`. The decision-tree twin of Inc-23's list-of-bools saturation: a `ListLen` lit-test is
    // normally refutable (excluded from coverage), but the else of the `== 0` test is exactly "len ≥ 1"
    // (`refine_listlen_else_rows`), which the second arm's `≥ 1` test covers → it becomes an
    // unconditional leaf. Before, this was a spurious CDZ0210 (a valid total match rejected).
    assert!(
        reject_code(
            "(module m (type Box (Bx (List Int64))) \
                   (def (f (: b Box)) (match b ((Bx (list)) 0) ((Bx (list x .. _r)) x))) \
                   (def (main) (f (Bx (list 7)))) (export main))"
        )
        .is_none(),
        "an empty + non-empty list-payload split covers the variant without a wildcard"
    );
    // Reordered (non-empty THEN empty) is equally total; a lone zero-lead rest `(Bx (list .. r))`
    // (vacuous length test — matches every length) covers the variant on its own.
    for src in [
        "(module m (type Box (Bx (List Int64))) \
               (def (f (: b Box)) (match b ((Bx (list x .. _r)) x) ((Bx (list)) 0))) \
               (def (main) (f (Bx (list 7)))) (export main))",
        "(module m (type Box (Bx (List Int64))) \
               (def (f (: b Box)) (match b ((Bx (list .. _r)) 0))) \
               (def (main) (f (Bx (list 7)))) (export main))",
    ] {
        assert!(
            reject_code(src).is_none(),
            "a reordered / vacuous list-payload cover is exhaustive: {src}"
        );
    }
    // MULTI-VARIANT: the same split under one variant, alongside a sibling variant, is total.
    assert!(
        reject_code(
            "(module m \
                   (def (f (: o (Option (List Int64)))) \
                     (match o ((Some (list)) 0) ((Some (list x .. _r)) x) ((None) -1))) \
                   (def (main) (f (Some (list 7)))) (export main))"
        )
        .is_none(),
        "a per-variant list-payload split composes with sibling variants"
    );
    // The RUNTIME dispatch — the guard-drop preserves first-match-wins and the moved length dispatch:
    // mk(0) → (Bx []) → the empty-list arm; mk(1) → (Bx [7]) → the non-empty arm binds x=7 — is
    // corpus-covered by 05-compound-types "a sum variant's list payload split across empty and rest arms
    // is exhaustive and dispatches" (and its erased-newtype vec-get twin); this test keeps the
    // COMPILE-time exhaustiveness pins above (a list-arm set covering empty + every non-empty is total).
}

// (a_sum_list_payload_with_an_uncovered_length_still_rejects + a_ctor_list_match_missing_a_variant_or_the_empty_arm_still_rejects
// migrated to corpus 05-compound-types, the saturation-soundness reject block: a sum-payload list match
// with only the non-empty arm / an exact-0 + at-least-2 gap / an Option covering only Some → CDZ0210;
// a ctor-lead match missing a variant / with no empty arm → CDZ0210. All pin that the saturation
// relaxation fires ONLY on JOINTLY-total coverage; any length/value/variant gap still rejects. --case
// grades the reject codes (all 7 across the block PASS).)

// (a_refutable_ctor_list_element_still_requires_a_catch_all migrated to corpus 05-compound-types: a
// ctor-element list arm is refutable so does not count toward length-coverage — two ctor arms still leave
// the empty list uncovered → CDZ0210. PASS wasm.)

#[test]
fn a_map_list_element_dispatches_by_key_presence() {
    // The MAP twin of the refutable-ctor list element: a list of key-value records matched by KEY in one
    // arm — `(match xs ((list (map (1 a)) .. rest) a) …)`. A `(map (k v)…)` element is REFUTABLE (matches
    // only a map containing the named keys) AND binds the values, so it desugars to a fresh binder + a
    // key-presence guard + a body re-match binding the values (`desugar_refutable_map_list_elements`,
    // reusing the direct map matcher). Before, it declined CDZ0201 "not a tuple, record, or constructor"
    // — the list-arm element check had tuple/sum/nested-list arms but no `map`.
    assert!(
        reject_code(
            "(module m (def (f (: xs (List (Map Int64 Int64)))) \
                   (match xs ((list (map (1 a)) .. rest) a) (_ (- 0 1)))) \
                 (def (main) (f (list (map (= 1 77))))) (export main))"
        )
        .is_none(),
        "a map list element now compiles (dispatches by key presence)"
    );
    // The RUNTIME dispatch — key-present binds the value (77), an absent key falls through (-1), and two
    // named keys bind both (105) — is corpus-covered by 05-compound-types "a map pattern as a list-arm
    // element binds its value binder" / "a list-arm map element whose key is absent falls through" / "a
    // list-arm map element binds both of two named keys"; this test keeps the compile (CDZ0201-gone) pin.
}

// (a_map_pattern_key_of_the_wrong_type_is_a_type_error migrated to corpus 05-compound-types, in the
// map-pattern section: a wrong-type key pattern on a typed map → CDZ0201 (message "map-pattern key is
// String")(message "the map's keys are Int64") + the symmetric Int-on-String reject + a RUNTIME-map
// wrong-key reject (separate desugar path) + a well-typed-key control that compiles + runs (→ 10). The
// diagnostic also anchors the squiggle at the offending key — a node-position refinement the corpus
// (error …) surface does not pin. --case grades the codes + messages + run value.)

#[test]
fn a_tail_recursive_sum_consumer_compiles_to_a_constant_stack_loop() {
    // A tail-recursive consumer of a SUM type — `(count n acc) = (match n ((Zero) acc) ((Succ m) (count
    // m (+ acc 1))))` over `(type Nat (Zero) (Succ Nat))` — is a self-tail-call inside a `Core::MatchSum`
    // arm (the decision tree's `(Succ m)` leaf). The loop transform now threads tail position into the
    // sum decision tree (`emit_tail`/`body_has_member_tail_call`/`tail_callees` handle `MatchSum` via
    // `emit_sum_cont`/`emit_sum_match_arms` + the `sum_cont_*` walkers), so it compiles to ONE `loop`
    // (constant stack) instead of a stack-growing recursive `call`. Pins the `loop` at the Lir level +
    // value parity + that a deeply nested nat (which would overflow a recursive stack) counts fine.
    use crate::backend::wasm::lir::Lir;
    use crate::backend::wasm::select::select_function_of;
    let src = "(module m (type Nat (Zero) (Succ Nat)) \
                     (def (count (: n Nat) (: acc Int64)) \
                       (match n ((Zero) acc) ((Succ m) (count m (+ acc 1))))) \
                     (def (f (: n Nat)) (count n 0)) (export f))";
    let mut db = crate::db::Db::load(crate::testkit::parse(src));
    let layout = crate::layout::compute(&mut db).expect("layout");
    let d = db.def_by_name("count").expect("count");
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
    // `select_function_of` with `self_def = Some(d)` enables the self-recursion loop transform.
    let code = select_function_of(&mut db, body, &ps, &layout, Some(d))
        .expect("select")
        .code;
    assert!(
        code.iter().any(|i| matches!(i, Lir::Loop(_))),
        "the tail sum consumer compiles to a loop, got: {code:?}"
    );
    assert!(
        !code
            .iter()
            .any(|i| matches!(i, Lir::Call(_) | Lir::ReturnCall(_))),
        "no residual self-`call`/`return_call` — the recursion became a loop br, got: {code:?}"
    );
    // (VALUE PARITY of the MatchSum-arm tail fold — `count` over `(type Nat (Zero) (Succ Nat))` folds to
    // the depth — is corpus 09 "a self-tail-recursive SUM consumer loops (tail call in a MatchSum arm)
    // and computes the fold"; the constant-STACK claim is exactly this Lir-loop witness (no residual
    // `call` → a `loop` → O(1) stack by construction). This keeps only that Lir witness.)
}

#[test]
fn an_inferred_width_cdz0302_names_the_range_but_offers_no_value_rewriting_fix() {
    // ADVICE-VALIDITY: an out-of-range literal whose width came from a SOLVED/INFERRED `Ty` (a nested
    // compound payload, OR — since #1766 — a sibling list element's annotation) must NOT carry a retype
    // fix. There is no written type-node on the literal to retype; the shared `int_out_of_range_reject`
    // would attach `replace <literal> with <TypeName>`, rewriting the VALUE `-41` into a TYPE name
    // (`(list (: 1 UInt64) Int8)` — a type in value position, and `Int8` for an UNSIGNED list): a
    // machine-applicable fix that CORRUPTS the source. The message still names the valid range (the
    // actionable fact); it just carries NO fix. A DIRECT annotation `(: v T)` DOES have a type-node and
    // keeps its retype fix (asserted below), so the value-position sites lose the fix WITHOUT regressing
    // the direct-annotation route.
    let fixless = |src: &str| {
        let diags = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
        let d = diags
            .iter()
            .find(|d| d.code.as_deref() == Some("CDZ0302"))
            .unwrap_or_else(|| panic!("expected CDZ0302 for: {src}"));
        assert!(
            d.message.contains("the valid range is"),
            "still names the actionable range: {}",
            d.message
        );
        assert!(
            d.fix.is_none(),
            "an inferred-width CDZ0302 must NOT carry a source-corrupting value→type fix, got: {:?}",
            d.fix
        );
    };
    // Sibling-inferred element width (the #1766 path) — negative-in-unsigned, both orders, and over-max.
    fixless("(module m (def (main) (list (: 1 UInt64) -41)) (export main))");
    fixless("(module m (def (main) (list -41 (: 1 UInt64))) (export main))");
    fixless("(module m (def (main) (list (: 1 UInt8) 300)) (export main))");
    // Nested Sum payload — the pre-#1766 path through the same reject.
    fixless("(module m (def (main) (: (Some -41) (Option UInt64))) (export main))");
    // CONTRAST: a DIRECT value annotation retains its retype fix, and it targets the TYPE spelling
    // (`UInt64`→`Int8` for the negative-in-unsigned sign-flip), NOT the value literal.
    let (arenas, span) =
        crate::testkit::parse_spanned("(module m (def (main) (: -41 UInt64)) (export main))");
    let diags = crate::diagnostics(&mut crate::db::Db::load(arenas));
    let d = diags
        .iter()
        .find(|d| d.code.as_deref() == Some("CDZ0302"))
        .expect("direct annotation still reports CDZ0302");
    let fix = d
        .fix
        .as_ref()
        .expect("a direct value annotation keeps its retype fix");
    assert_eq!(fix.kind, crate::abi::FixKind::Replace);
    assert_eq!(
        fix.replacement, "Int8",
        "sign-flips a negative into the smallest signed width"
    );
    let (start, len) = span.spans[fix.node as usize];
    assert_eq!(
        &span.source[start as usize..(start + len) as usize],
        "UInt64",
        "the retype fix targets the annotation TYPE node, not the value literal"
    );
}

#[test]
fn a_mutrec_map_and_set_accumulator_ground_the_partner_sum_child_and_emit_on_rust() {
    // Coverage-hardening for the #1816 mutual-recursion partner-result-retype fix — beyond the List
    // accumulator it landed with. The same shape with a MAP accumulator (value = the partner-tuple's
    // sum `child`) and a SET accumulator (element = `child`) also freezes on rust without the fix:
    // `child`'s type flows off the partner call `(dn b i)`, and the SumPayload-arm re-type feeds its
    // concrete `Ast` into whatever collection it lands in — via Map.insert's value arm / Set.insert's
    // element arm, distinct code paths from List.push. Both must EMIT on rust (the freeze was rust-only;
    // wasm heap-erases the element). This locks the fix's generality across the three heap collections.
    let emits_rust = |src: &str| {
        let mut dbr = crate::db::Db::load(parse(src));
        let lay = crate::layout::compute(&mut dbr).expect("mutrec collection accumulator lays out");
        crate::backend::emit(crate::backend::Target::Rust, &mut dbr, &lay, None, None)
            .expect("mutrec collection accumulator grounds the partner sum child and emits rust");
    };
    // MAP accumulator — the inserted VALUE is `child` (an Ast bound off the partner tuple).
    emits_rust(
        "(module m \
               (type Ast (AInt Int64) ALeaf (AList (List Ast))) \
               (def (dn b i) \
                 (if (= i 0) (tuple (AInt (Option.expect (List.at b 0) \"x\")) (+ i 1)) \
                             (tuple (AList (list (dac b i (- i 1) Map.empty))) (+ i 1)))) \
               (def (dac b i n acc) \
                 (if (< n 1) acc \
                     (match (dn b i) ((tuple child nx) (dac b nx (- n 1) (Map.insert acc n child)))))) \
               (def (main) 0) (export main))",
    );
    // SET accumulator — the inserted ELEMENT is `child`.
    emits_rust(
        "(module m \
               (type Ast (AInt Int64) ALeaf (AList (List Ast))) \
               (def (dn b i) \
                 (if (= i 0) (tuple (AInt (Option.expect (List.at b 0) \"x\")) (+ i 1)) \
                             (tuple (AList (list (dac b i (- i 1) Set.empty))) (+ i 1)))) \
               (def (dac b i n acc) \
                 (if (< n 1) acc \
                     (match (dn b i) ((tuple child nx) (dac b nx (- n 1) (Set.insert acc child)))))) \
               (def (main) 0) (export main))",
    );
}

#[test]
fn an_unannotated_mutrec_accumulator_grounds_its_sum_element_and_emits_on_rust() {
    // THE FIX for the mutual-recursion `(List Any)` freeze (v-rust-backend ask, breaker mx5). An
    // UNANNOTATED accumulator `acc` in a mutually-recursive decoder — `dac`'s `acc` seeds as an empty
    // `(list)` and is only ever `(List.push acc child)` where `child : Ast` is bound by `(match (dn b i)
    // ((tuple child nx) …))`, `dn`/`dac` a mutual SCC. Its element SHOULD ground to `Ast`. It DID on
    // wasm (heap-erased) but FROZE to `(List Any)` on the rust path: during the connected param-solve,
    // `child`'s type reads through `arg_ty_in_env`'s SumPayload arm off the partner call `(dn b i)`,
    // which types `Any` mid-solve (dn's scheme deferred under the in-flight SCC) → `child : Any` → froze
    // `acc = (List Any)` in `db.param_types` → rust "parameter type (List Any) has no native
    // representation". FIX (infer.rs SumPayload arm): when the partner-call result is an undetermined
    // `Any`, RE-TYPE it from the callee's BODY (whose constructors fix the concrete `(Tuple Ast Int64)`),
    // guarded by `db.scc_result_typing` against the mutual-edge recursion. `acc` now grounds to `(List
    // Ast)` and BOTH backends emit. Prior to the fix the unannotated form declined rust while the
    // annotated workaround (the pin above) was the only rust-buildable spelling.
    let src = "(module m \
          (type Ast (AInt Int64) ALeaf (AList (List Ast))) \
          (def (dn b i) \
            (if (= i 0) \
                (tuple (AInt (Option.expect (List.at b 0) \"in range\")) (+ i 1)) \
                (tuple (AList (dac b i (- i 1) (list))) (+ i 1)))) \
          (def (dac b i n acc) \
            (if (< n 1) acc \
                (match (dn b i) ((tuple child nx) (dac b nx (- n 1) (List.push acc child)))))) \
          (def (top b) (match (dn b 0) ((tuple ast pos) ast))) \
          (def (main) (match (top (list 42 7)) ((AInt n) n) (_ -1))) (export main))";
    // WASM (always emitted).
    let mut dbw = crate::db::Db::load(parse(src));
    let layw =
        crate::layout::compute(&mut dbw).expect("unannotated mutrec decoder lays out (wasm)");
    crate::backend::emit(crate::backend::Target::Wasm, &mut dbw, &layw, None, None)
        .expect("unannotated mutrec decoder must emit wasm");
    // RUST (the regressed side — must now EMIT, not decline on a frozen (List Any) accumulator).
    let mut dbr = crate::db::Db::load(parse(src));
    let layr =
        crate::layout::compute(&mut dbr).expect("unannotated mutrec decoder lays out (rust)");
    crate::backend::emit(crate::backend::Target::Rust, &mut dbr, &layr, None, None).expect(
            "unannotated mutrec accumulator grounds its element to Ast (not frozen (List Any)) and emits rust",
        );
}

#[test]
fn an_annotated_accumulator_lets_a_mutually_recursive_decoder_emit_on_both_backends() {
    // PERIMETER pin for the (List Any) mutual-recursion freeze (v-rust-backend ask + breaker mx2):
    // the UNANNOTATED accumulator `acc` of `dac` freezes to `(List Any)` and DECLINES on rust — its
    // element `Ast` can't flow through the mutual partner `dn`'s not-yet-settled tuple result during
    // the connected param-solve (a genuine `solving_params`↔`def_scheme` cycle; filed for a dedicated
    // slice). The DOCUMENTED WORKAROUND is to ANNOTATE the accumulator `(: acc (List Ast))`: the
    // annotation gives the param a fixed type (no inference through the cycle), so BOTH backends emit.
    // This pins the workaround (so it keeps working) AND guards the perimeter — when the freeze is
    // fixed the unannotated form joins this on rust; if annotated-param handling regresses, this
    // catches it. Both backends must EMIT (the freeze is rust-only; wasm always ran).
    let src = "(module m \
          (type Ast (AInt Int64) ALeaf (AList (List Ast))) \
          (def (dn b i) \
            (if (= i 0) \
                (tuple (AInt (Option.expect (List.at b 0) \"in range\")) (+ i 1)) \
                (tuple (AList (dac b i (- i 1) (list))) (+ i 1)))) \
          (def (dac b i n (: acc (List Ast))) \
            (if (< n 1) acc \
                (match (dn b i) ((tuple child nx) (dac b nx (- n 1) (List.push acc child)))))) \
          (def (top b) (match (dn b 0) ((tuple ast pos) ast))) \
          (def (main) (match (top (list 42 7)) ((AInt n) n) (_ -1))) (export main))";
    let mut dbw = crate::db::Db::load(parse(src));
    let layw = crate::layout::compute(&mut dbw).expect("annotated-acc decoder lays out (wasm)");
    crate::backend::emit(crate::backend::Target::Wasm, &mut dbw, &layw, None, None)
        .expect("annotated-acc mutrec decoder must emit wasm");
    let mut dbr = crate::db::Db::load(parse(src));
    let layr = crate::layout::compute(&mut dbr).expect("annotated-acc decoder lays out (rust)");
    crate::backend::emit(crate::backend::Target::Rust, &mut dbr, &layr, None, None).expect(
            "annotating the accumulator `(: acc (List Ast))` lets the mutrec decoder emit rust (the freeze workaround)",
        );
}

#[test]
fn many_pass_through_defs_each_infer_their_param_via_the_call_site_index() {
    // Call-site inference (`call_site_arg_types`) seeds an open param from a caller's argument — it
    // needs "every call of this def", which it now reads from a CALL-SITE INDEX built once, not by
    // re-scanning every def body per query (the O(defs × program) = O(N²) a decoder pipeline of N
    // pass-through pairs hit). This locks in that the index preserves the inference at width: 12
    // INDEPENDENT pass-through chains `a{i}(b) → c{i}(b) → (List.len b)` — each `a{i}`'s param `b` is
    // decided ONLY transitively through its own chain's call site — all compile to valid wasm (the
    // seed threaded each `b` to `(List Int64)`; a broken index would leave `b` `Any` and decline).
    let n = 12;
    let mut defs = String::new();
    for i in 0..n {
        defs.push_str(&format!(
            "(def (a{i} b) (c{i} b)) (def (c{i} b) (+ (List.len b) {i})) "
        ));
    }
    let src = format!("(module m {defs} (def (main) (a0 (list 1 2))) (export main))");
    // Compiles (every `a{i}`/`c{i}` param inferred `(List Int64)` via the call-site seed) — a broken
    // index (missing a chain's call site) would leave a param `Any` and decline.
    assert!(
        compile_component(&crate::codec::encode(&parse(&src))).is_ok(),
        "every pass-through chain's param is inferred via the call-site index"
    );
}

#[test]
fn a_body_that_traps_through_a_seq_emits_a_trapping_function() {
    // The divergence detection peers THROUGH a `Core::Seq` (an effect-statement run then a value) to
    // its trapping tail — the shape a unit-test FAILURE path takes: run a `report`/`log` host effect
    // FOR ITS EFFECT, then `(trap …)`. Before, `(host (log) (do (log.emit "m") (trap …)))` selected
    // the body's result type to the trap's `Never` (a fresh var, no machine rep) and DECLINED "function
    // return type has no machine representation" — the whole `do` block's value is the trap, but the
    // Seq wrapper hid it from the bare `Core::Trap`-exact guard. Now `body_diverges` recurses through
    // `Seq { tail }` (and `Let { body }`), so the body is recognized as diverging → emitted UNIT
    // (0-result), the host observes the effect THEN the trap. The dual sites — the core function
    // signature (`select_function`) and the component boundary (`wasm::mod`) — share `body_diverges`.
    let src = "(module m (effect log (op emit (-> String Unit))) \
                   (def (main) (host (log) (do ((. log emit) \"boom\") (trap \"failed\")))) \
                   (export main))";
    // The KEY assertion: it EMITS (no "no machine representation" decline). A component is produced.
    let bytes = component(src);
    assert!(
        !bytes.is_empty(),
        "an effect-then-trap body must emit, not decline as unrepresentable"
    );
}

// (a_cross_width_nan_comparison_is_a_type_error migrated to corpus 06-numeric-model, the nan-comparison
// type block in the NaN section: a cross-WIDTH nan comparison (Float32.nan vs Float64.nan / vs a Float64
// literal, either order) → CDZ0301, exactly as the finite cross-width comparison; a nan vs a non-float →
// cross-kind CDZ0301; and the SAME-width nan comparison compiles + folds TRUE (structural =, not IEEE
// arithmetic-identity false). --case grades the reject codes + the run values (all 7 PASS).)
#[test]
fn decimal_from_f64_round_trips_by_bits() {
    // The float-fold's result representation: `Decimal::from_f64(f)` builds a decimal whose
    // `to_f64_bits()` returns EXACTLY `f`'s bits (via shortest round-tripping formatting), so a
    // computed float crosses the boundary unchanged. Covers the non-exact 0.1+0.2, a large whole
    // float (no i64 saturation), a tiny value, and signed zero. A non-finite input yields `None`.
    use crate::ast::Decimal;
    for f in [
        0.1f64 + 0.2f64,
        42.0,
        1e19,
        1.0 / 3.0,
        -0.0,
        0.0,
        5e-324, // smallest subnormal
    ] {
        let d = Decimal::from_f64(f).expect("finite float has a decimal form");
        assert_eq!(
            d.to_f64_bits(),
            f.to_bits(),
            "Decimal::from_f64({f}) must round-trip by bits"
        );
    }
    assert!(Decimal::from_f64(f64::INFINITY).is_none());
    assert!(Decimal::from_f64(f64::NAN).is_none());
}

// (a_utf8_bin_match_with_no_catch_all_is_non_exhaustive migrated to corpus 16-binary-matching: CDZ0210. PASS wasm.)

// (a_string_annotation_checks_against_a_string_value migrated to corpus 13-strings: `(: "hi" Int64)` is a
// String-vs-scalar annotation mismatch → CDZ0203. PASS wasm.)

// (checked_integer_conversion_over_range_message_is_actionable migrated to corpus 06-numeric-model: the
// actionable-message pins now live on the checked-conversion reject cases — "an out-of-range checked integer
// conversion of a constant is rejected" (`(UInt8.of 256)` → message "0..=255"), "a checked conversion of a
// negative constant into an unsigned type is rejected" (`(UInt8.of -1)` → message ".wrap", the .of-vs-.wrap
// hint), and the added "an out-of-range signed checked conversion names its signed valid range"
// (`(Int8.of 200)` → message "-128..=127"). All three graded PASS on wasm.)

#[test]
fn a_provably_nonnegative_index_elides_the_list_at_lower_bound_check() {
    // BOUNDS-CHECK LOWER-HALF ELISION: `List.at`/`Bytes.at` test `(index >= 0) & (index < len)`. When
    // the index is provably NON-NEGATIVE (a masked value `(& i 3)`, a length, an unsigned type), the
    // `index >= 0` half is a compile-time `true` — drop it, test only `index < len`. Pins the elision
    // at the Lir level (a masked index emits NO `index >= 0` sub-check) AND that a PLAIN (possibly
    // negative) index keeps it. The runtime value parity (the upper bound still catches OOB over a
    // heap-built list at a masked/negative index) is corpus-covered by 05 lbe1/lbe2/lbe3; this keeps
    // only the Lir elision witness the corpus cannot express.
    use crate::backend::wasm::lir::Lir;
    use crate::db::Db;
    let select = |src: &str, name: &str| {
        let ast = crate::testkit::parse(src);
        let mut db = Db::load(ast);
        let layout = crate::layout::compute(&mut db).expect("layout");
        let d = db.def_by_name(name).expect("def");
        let body = db.defs[d].body.expect("body");
        let params: Vec<_> = db.defs[d]
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
        crate::backend::wasm::select::select_function(&mut db, body, &params, &layout)
            .expect("select")
            .code
    };
    // A `List.at` bounds check has ONE `i64.lt_s` (the upper `index < len`). The LOWER check adds a
    // `ConstI64(0)` immediately followed by `i64.ge_s`. A masked index `(& i 3)` (∈ [0,3]) drops it;
    // a plain param index keeps it.
    let has_lower_check = |c: &[Lir]| {
        c.windows(2)
            .any(|w| matches!(w, [Lir::ConstI64(0), Lir::I64GeS]))
    };
    let masked = select(
        "(module m (def (f (: xs (List Int64)) (: i Int64)) \
               (match (List.at xs (& i 3)) ((Some x) x) (None -1))) (def (main) 0) (export main))",
        "f",
    );
    assert!(
        !has_lower_check(&masked),
        "a masked (nonneg) index drops the `index >= 0` lower-bound check, got: {masked:?}"
    );
    let plain = select(
        "(module m (def (f (: xs (List Int64)) (: i Int64)) \
               (match (List.at xs i) ((Some x) x) (None -1))) (def (main) 0) (export main))",
        "f",
    );
    assert!(
        has_lower_check(&plain),
        "a plain (possibly-negative) index KEEPS the `index >= 0` check, got: {plain:?}"
    );
}

// NOTE: the self-hosting arg-walk idiom (corpus §'a list built by a recursive push-loop is then
// iterated by index') — `(def (sum-at xs i n) … (match (List.at xs i) …))` — is NOT yet compilable,
// but the block is the runtime `List.at` EMIT: it is an orthogonal INFERENCE gap. When the list is a
// FUNCTION PARAMETER (`xs`), its element type stays an unsolved `?0` (the concrete `List Int64` at
// the call site does not flow into the parameter's element), so the match binder `x` is `?0` and the
// add over it declines "projecting a tuple element of type ?0". The runtime read itself works when
// the list's element type is known (the two cases above index a locally-built `List Int64`); the
// parameter-element propagation is a separate increment (the "runtime list's element type flows
// through" gap the increment-2 note records).

#[test]
fn a_variant_name_colliding_with_a_prelude_name_does_not_shadow_it() {
    // A bare variant name resolves BEFORE the prelude (`resolve` step 3c precedes step 4), so a variant
    // whose name COLLIDES with a built-in prelude entry (`Int`/`List`/`Name` — a type constructor, a
    // collection module) must NOT shadow it — else that name breaks everywhere it is used as a
    // type/module. The `variant_ctor_index` build skips a prelude-colliding variant name; the variant
    // stays reachable QUALIFIED (`(. T Int)`) via the sum record's field. Regression: declaring `(type
    // T (Int Int64))` made bare `Int` the variant ctor, so an unrelated `(: x Int64)` failed to reduce
    // (`Int` was no longer the width constructor) — a global corruption from one declaration.
    // The unrelated `Int64` annotation still reduces even with a `(type T (Int Int64))` in scope.
    // (The construct-HEAD position DOES now shadow — a bare `(Int 42)` builds T's variant — but that is
    // scoped to a user node in head position and does NOT touch the width TYPE in annotation position,
    // the invariant this test guards. See `a_type_name_colliding_variant_constructs_as_the_local_variant`.)
    assert!(
            reject_code(
                "(module m (type T (Int Int64)) (def (g (: x Int64)) x) (def (main) (g 5)) (export main))"
            )
            .is_none(),
            "a variant named `Int` must not shadow the prelude `Int` type constructor"
        );
    // The colliding variant is still reachable QUALIFIED and checks its payload: `(. T Int)` applied
    // to a String is the wrong-payload CDZ0201, exactly as a non-colliding variant is.
    assert_eq!(
        reject_code("(module m (type T (Int Int64)) (def (main) ((. T Int) \"x\")) (export main))")
            .as_deref(),
        Some("CDZ0201"),
        "a qualified colliding-variant ctor still type-checks its payload"
    );
}

#[test]
fn a_bare_prelude_colliding_variant_matches_as_the_local_variant() {
    // In a MATCH the scrutinee's type is known, so a BARE variant-name pattern head that COLLIDES with
    // a prelude entry (`(Int n)` on `(type T (Int Int64))`, `(Some n)` on a user `(type … (Some …))`)
    // resolves against the SCRUTINEE sum's variant set FIRST — reaching the LOCAL variant, not the
    // prelude `Int`/`Some`. Without this, the bare head resolved (scope→def→prelude) to the prelude
    // entry and the ctor check rejected CDZ0203, so an AST sum with prelude-colliding variant names
    // could only be matched QUALIFIED. `pattern_constraints` remaps a bare head to the scrutinee decl's
    // cached ctor for that name. (The CONSTRUCT half now shadows too — see
    // `a_type_name_colliding_variant_constructs_as_the_local_variant` below.)
    let ok = |src: &str| assert!(reject_code(src).is_none(), "must compile: {src}");
    // Single-variant nominal newtype, bare `Int` pattern (qualified construct).
    ok(
        "(module m (type T (Int Int64)) (def (f (: t T)) (match t ((Int n) n))) (def (main) (f (T.Int 42))) (export main))",
    );
    // Multi-variant sum, bare `Int` pattern beside a nullary arm.
    ok(
        "(module m (type T (Int Int64) (Nil)) (def (f (: t T)) (match t ((Int n) n) ((Nil) 0))) (def (main) (f (T.Int 42))) (export main))",
    );
    // `Some`-colliding variant, bare pattern.
    ok(
        "(module m (type T (Some Int64) (Nada)) (def (f (: t T)) (match t ((Some n) n) ((Nada) 0))) (def (main) (f (T.Some 42))) (export main))",
    );
    // NO OVER-ACCEPTANCE: a bare variant of a DIFFERENT sum (`Bar` of `U`) over a `T` scrutinee still
    // rejects CDZ0203 (the remap only reaches T's OWN variants; a foreign name is left to the check).
    assert_eq!(
            reject_code("(module m (type T (Int Int64)) (type U (Bar Int64)) (def (f (: t T)) (match t ((Bar n) n))) (def (main) (f (T.Int 42))) (export main))").as_deref(),
            Some("CDZ0203"),
            "a foreign variant name in a pattern still rejects"
        );
}

#[test]
fn the_builtin_ast_sum_type_checks_its_variant_payloads() {
    // 12-metaprogramming "a built-in Ast constructor applied to a wrong-type payload is a type error":
    // the built-in `Ast` is an ordinary MONOMORPHIC prelude sum (Int:Int64, Name:String, List:(List
    // Ast)) — a variant per syntactic form (type-system.md §The Abstract Syntax Tree Type Is An
    // Ordinary Sum Type). Its variants are reached ONLY QUALIFIED (`Ast.Int`), their names colliding
    // with prelude `Int`/`List` so they don't bind bare. `Ast.Int`'s payload is Int64, so `(Ast.Int
    // "x")` applies it to a String — the wrong-payload CDZ0201, exactly as a user sum variant is.
    assert_eq!(
        reject_code("(module m (def (main) (Ast.Int \"x\")) (export main))").as_deref(),
        Some("CDZ0201"),
        "the built-in Ast.Int checks its Int64 payload"
    );
    // The correct payload types (no fault); a String payload to `Ast.Name` is likewise well-typed.
    assert!(
        reject_code("(module m (def (main) (Ast.Int 42)) (export main))").is_none(),
        "Ast.Int applied to Int64 is well-typed"
    );
    assert!(
        reject_code("(module m (def (main) (Ast.Name \"x\")) (export main))").is_none(),
        "Ast.Name applied to String is well-typed"
    );
}

#[test]
fn quote_reifies_to_the_ast_value_it_denotes() {
    // 12-metaprogramming §Quote Produces An AST Value: `(quote FORM)` evaluates to the `Ast` sum value
    // representing FORM's structure. `crate::quote::reify_quotes` rewrites each quote into the
    // constructor application that BUILDS that value, so a quote result and a hand-built `Ast.*` value
    // are ONE value — structural equality (`=`) between them holds. The reification maps by SHAPE:
    // integer -> `(Ast.Int n)`, bare name -> `(Ast.Name "n")`, compound `(a …)` -> `(Ast.List (list
    // …))`. Each equality below compiles clean (the two sides denote the same value); the gate checks
    // they actually run to `true`.
    assert!(
        reject_code("(module m (def (main) (= (quote 42) (Ast.Int 42))) (export main))").is_none(),
        "a quoted integer equals the same Ast.Int node"
    );
    assert!(
        reject_code("(module m (def (main) (= (quote foo) (Ast.Name \"foo\"))) (export main))")
            .is_none(),
        "a quoted name equals the same Ast.Name node"
    );
    assert!(
        reject_code(
            "(module m (def (main) \
                   (= (quote (+ 1 2)) \
                      (Ast.List (list (Ast.Name \"+\") (Ast.Int 1) (Ast.Int 2))))) \
                 (export main))"
        )
        .is_none(),
        "a quoted compound form equals the same Ast.List node"
    );
    // A quote is INERT: a stray `,x` (unquote NOT under a quasiquote) is a syntax error — the quote is
    // NOT reified around it; `resolve::resolve_unquote` fires CDZ0003 (metaprogramming.md §Quasiquote
    // Constructs AST With Selective Evaluation, a plain quote body is inert data, not a template).
    assert_eq!(
        reject_code("(module m (def (main) (quote (g (unquote x)))) (export main))").as_deref(),
        Some("CDZ0003"),
        "a stray unquote under a plain quote is CDZ0003, not silently reified"
    );
    // A BOOLEAN literal reifies to `(Ast.Bool b)` — the boolean is a syntactic form, so the `Ast` sum
    // carries it (type-system.md §The Abstract Syntax Tree Is An Ordinary Sum Type: "an integer, a
    // float, a string, a boolean, a name, and a list"). `(quote true)` equals the hand-built node.
    assert!(
        reject_code("(module m (def (main) (= (quote true) (Ast.Bool true))) (export main))")
            .is_none(),
        "a quoted boolean equals the same Ast.Bool node"
    );
    // A boolean nested in a compound reifies as an `Ast.Bool` element (structural, like `Ast.Int`).
    assert!(
        reject_code(
            "(module m (def (main) \
                   (= (quote (f false)) \
                      (Ast.List (list (Ast.Name \"f\") (Ast.Bool false))))) \
                 (export main))"
        )
        .is_none(),
        "a quoted compound with a boolean reifies with an Ast.Bool element"
    );
    // `Ast.Bool`'s payload is type-checked like any variant ctor: a non-Bool payload is CDZ0201.
    assert_eq!(
        reject_code("(module m (def (main) (Ast.Bool 5)) (export main))").as_deref(),
        Some("CDZ0201"),
        "Ast.Bool applied to a non-Bool payload is a type error"
    );
    // A STRING literal reifies to `(Ast.Str "…")` — a string is a syntactic form, DISTINCT from a
    // name. `(quote "foo")` is `(Ast.Str "foo")`, not `(Ast.Name "foo")`, so they compare unequal.
    assert!(
        reject_code("(module m (def (main) (= (quote \"hi\") (Ast.Str \"hi\"))) (export main))")
            .is_none(),
        "a quoted string equals the same Ast.Str node"
    );
    assert!(
        reject_code("(module m (def (main) (= (quote \"foo\") (Ast.Name \"foo\"))) (export main))")
            .is_none(),
        "a quoted string vs a quoted name is well-typed (both Ast) — the runtime value is false"
    );
    // A FLOAT literal reifies to `(Ast.Float d)` — a float is a syntactic form (the Ast sum now
    // realizes the COMPLETE spec set: Int/Float/Bool/Str/Name/List). `(quote 3.0)` is `(Ast.Float
    // 3.0)`, DISTINCT from `(Ast.Int 3)`, so they compare unequal.
    assert!(
        reject_code("(module m (def (main) (= (quote 1.5) (Ast.Float 1.5))) (export main))")
            .is_none(),
        "a quoted float equals the same Ast.Float node"
    );
    assert!(
        reject_code("(module m (def (main) (= (quote 3.0) (Ast.Int 3))) (export main))").is_none(),
        "a quoted float vs a quoted int is well-typed (both Ast) — the runtime value is false"
    );
    // A quote whose body mentions a leaf the `Ast` sum still can't carry (a CHAR literal — no
    // `Ast.Char` variant) is NOT reified: it DECLINES (a Todo), never a miscompile.
    assert_eq!(
        reject_code("(module m (def (main) (quote #\\a)) (export main))"),
        None,
        "an un-reifiable quote body (a char leaf) declines cleanly (no artifact, no coded rejection)"
    );
    // The other two un-reifiable value leaves — a SYMBOL (`#"…"`) and a BYTES (`b"…"`) literal —
    // have no `Ast` variant either, so they take the SAME `reify` catch-all bail as a char: DECLINE
    // (a Todo), never a coded rejection or a miscompile. Pins the full bail set (`Char/Sym/Bytes`)
    // the quote-leaf dispatch documents, so a future `Ast` variant addition that quietly starts
    // reifying one of them (or turns the honest decline into a hard error) trips a test.
    assert_eq!(
        reject_code("(module m (def (main) (quote #\"m\")) (export main))"),
        None,
        "an un-reifiable quote body (a symbol leaf) declines cleanly (no artifact, no coded rejection)"
    );
    assert_eq!(
        reject_code("(module m (def (main) (quote b\"x\")) (export main))"),
        None,
        "an un-reifiable quote body (a bytes leaf) declines cleanly (no artifact, no coded rejection)"
    );
}

#[test]
fn scan_manifest_reads_each_param_site_name_widget_range_and_type() {
    // @param sidecar WIDGET MANIFEST (DESIGN-runtime-parameter-host-effect.md 2nd output): param_sidecar::
    // scan_manifest walks every `(: (@ (param <kv>) name) Type)` site into a ParamRecord the host reads
    // (v-cdz-tooling plumbs the Query + `cdz param-manifest` CLI over these). This tests the SCAN half —
    // that each site's name + widget + range + type node are read off the arena correctly. Node-ids
    // (ty/range) are rendered by the query handler (Db type column + span table); here we assert the
    // NAME + widget string + that a range yields two element nodes + a type node is present.
    use crate::param_sidecar::scan_manifest;
    // NOTE the s-expr surface spells the range list `(list 0 100)` — the `[0 100]` bracket-sugar is an
    // ML-surface literal the s-expr reader does NOT parse as a list (it reads `[0`/`100]` as atoms). The
    // canonical arena node is `(list lo hi)` either way (bracket sugar desugars to it on the ML side).
    let ast = crate::testkit::parse(
        "(module m \
               (pragma param (param (: widget slider) (: range (list 0 100))) (: width Int64)) \
               (pragma param (param (: widget toggle)) (: mirror Bool)) \
               (def (main) 0) \
             (export main))",
    );
    let recs = scan_manifest(&ast);
    assert_eq!(recs.len(), 2, "two @param sites → two manifest records");
    let width = recs
        .iter()
        .find(|r| r.name == "width")
        .expect("width record");
    assert_eq!(
        width.widget.as_deref(),
        Some("slider"),
        "width's widget config reads as `slider`"
    );
    assert!(
        width.range.is_some(),
        "width's `range: [0 100]` yields two element nodes"
    );
    // The declared type node is present (rendered by the query handler); confirm it names Int64.
    assert_eq!(
        ast.as_name(width.ty),
        Some("Int64"),
        "width's declared type node is Int64"
    );
    let mirror = recs
        .iter()
        .find(|r| r.name == "mirror")
        .expect("mirror record");
    assert_eq!(
        mirror.widget.as_deref(),
        Some("toggle"),
        "mirror's widget config reads as `toggle`"
    );
    assert!(
        mirror.range.is_none(),
        "mirror has no range kv → range is None (a stable-schema null, not a crash)"
    );
    assert_eq!(
        ast.as_name(mirror.ty),
        Some("Bool"),
        "mirror's type node is Bool"
    );
    // OPTIONS + DEFAULT config (a dropdown param): scan_manifest reads the `(: options (list …))` list
    // node + the `(: default <val>)` value node. Both come back as node-ids the query handler renders
    // (options → a JSON array of the list's elements, default → the rendered value / JSON null when
    // absent). Here assert they are PRESENT (Some) for a param that declares them.
    let ast2 = crate::testkit::parse(
        "(module m \
               (pragma param (param (: widget dropdown) (: options (list \"m\" \"mm\" \"in\")) (: default \"mm\")) (: unit String)) \
               (def (main) 0) \
             (export main))",
    );
    let recs2 = scan_manifest(&ast2);
    let unit = recs2
        .iter()
        .find(|r| r.name == "unit")
        .expect("unit record");
    assert_eq!(
        unit.widget.as_deref(),
        Some("dropdown"),
        "unit's widget is dropdown"
    );
    assert!(
        unit.options.is_some(),
        "unit's `options: (list …)` yields the list node"
    );
    assert!(
        unit.default.is_some(),
        "unit's `default: \"mm\"` yields the default value node"
    );
    // The options node IS the (list …) — confirm it reads as a list form the handler can enumerate.
    assert!(
        ast2.as_form(unit.options.unwrap(), "list").is_some(),
        "the options node is a (list …) the query handler enumerates for the JSON array"
    );
}

#[test]
fn scan_manifest_reads_a_range_with_a_native_list_the_ml_surface_lowers_to() {
    // ML-vs-sexpr range shape (v-guide-infra): an ML `@param(range: [lo, hi])` lowers the `[lo, hi]`
    // bracket to a NATIVE ctor-leaf list `#list(lo hi)` (the parser's `ctor_head("list", …)` emits
    // `Leaf::Ctor(List)`), and the s-expr surface's `(list lo hi)` has a NAME head. `config_range` reads
    // both via `compound_form_of(_, List)` (ctor-leaf + name), so an ML-authored range and an s-expr one
    // both scan. Here `#list(2 20)` IS the node the ML `[2, 20]` lowers to; the name-head twin below is the
    // s-expr surface. (Historical: the ML surface once lowered brackets to a legacy STRING head `("list" …)`
    // and this test pinned that; the reader now emits the native ctor-leaf, so the fixture is native — this
    // also keeps the case flip-safe when the M3 reader-flip drops the legacy string-head recognizer.)
    use crate::param_sidecar::scan_manifest;
    let ast = crate::testkit::parse(
        "(module m \
               (pragma param (param (: widget slider) (: range #list(2 20))) (: thickness Int64)) \
               (def (main) 0) \
             (export main))",
    );
    let recs = scan_manifest(&ast);
    let thickness = recs
        .iter()
        .find(|r| r.name == "thickness")
        .expect("thickness record");
    let (lo, hi) = thickness
        .range
        .expect("a native #list(2 20) range — the ML-lowered shape — is scanned, not dropped");
    assert_eq!(
        (
            ast.as_int(lo).map(|v| v.to_decimal_string()).as_deref(),
            ast.as_int(hi).map(|v| v.to_decimal_string()).as_deref()
        ),
        (Some("2"), Some("20")),
        "the native #list range's two element nodes are the authored bounds 2 and 20"
    );
    // The NAME-head `(list …)` (s-expr surface) still works — the fix ADDED the string-head path, it
    // did not replace the name-head one.
    let ast_name = crate::testkit::parse(
        "(module m \
               (pragma param (param (: widget slider) (: range (list 2 20))) (: thickness Int64)) \
               (def (main) 0) \
             (export main))",
    );
    assert!(
        scan_manifest(&ast_name)
            .iter()
            .find(|r| r.name == "thickness")
            .expect("thickness")
            .range
            .is_some(),
        "the name-head (list …) range (s-expr surface) still scans — both head spellings accepted"
    );
}

#[test]
fn at_param_site_generates_a_param_accessor_the_guest_can_reference() {
    // @param sidecar (DESIGN-runtime-parameter-host-effect.md): a `@param(widget: …) name : Type` site
    // — parsed to `(: (@ (param …) name) Type)` — makes the sidecar GENERATE `(effect Param (op name
    // (-> Unit Type)))`, so a guest `(Param.name)` resolves to the generated accessor. This tests the
    // SCAN+GENERATE: WITH a @param site, `(Param.width)` resolves (no CDZ0101); WITHOUT it, `Param` is
    // unbound (proving the generation is what binds it). The run-with-a-host-value path is covered by
    // the 26-runtime-params corpus (host-response wiring lives in cdz-run, not this lib harness).
    assert!(
        reject_code(
            "(module m \
                   (pragma param (param (: widget slider)) (: width Int64)) \
                   (def (main) (host (Param) (Param.width))) \
                 (export main))"
        )
        .is_none(),
        "an @param site generates the Param effect so (Param.width) resolves"
    );
    // WITHOUT the @param site, Param is not generated → (Param.width) is unbound (CDZ0101). This is the
    // control proving the sidecar's generation is exactly what binds the accessor.
    assert_eq!(
        reject_code("(module m (def (main) (host (Param) (Param.width))) (export main))")
            .as_deref(),
        Some("CDZ0101"),
        "without an @param site there is no generated Param — (Param.width) is unbound"
    );
    // The accessor is typed by the annotation: a second @param generates a second op, and both
    // resolve under one generated Param effect (one effect, one op per site).
    assert!(
        reject_code(
            "(module m \
                   (pragma param (param (: widget slider)) (: width Int64)) \
                   (pragma param (param (: widget number)) (: height Int64)) \
                   (def (main) (host (Param) (+ (Param.width) (Param.height)))) \
                 (export main))"
        )
        .is_none(),
        "two @param sites generate two accessors under one Param effect, both resolve"
    );
    // The config kv is OPTIONAL to the scan: a bare `(param)` (no widget metadata) still generates the
    // accessor — the scan keys on the param name + declared TYPE, not the widget (which feeds only the
    // later manifest). So a config-less @param resolves its accessor, not a reject.
    assert!(
        reject_code(
            "(module m \
                   (pragma param (param) (: width Int64)) \
                   (def (main) (host (Param) (Param.width))) \
                 (export main))"
        )
        .is_none(),
        "an @param with no widget config still generates its typed accessor"
    );
    // B-INVARIANT: an UNTYPED `@!param` (a `(pragma param (param …) name)` whose binder is a BARE name,
    // no `(: name Type)`) has no accessor type to generate and is REJECTED (CDZ0602, the malformed-
    // directive code) by the pragma-registry `param` arm — the accessor's result type IS the annotation
    // type, so an un-typed param is not silently dropped. (The sidecar's typed-shape scan never matches a
    // bare-name binder, so it generates nothing; the registry arm is the coded reject.)
    assert_eq!(
        reject_code(
            "(module m (pragma param (param (: widget slider)) width) (def (main) 0) (export main))"
        )
        .as_deref(),
        Some("CDZ0602"),
        "an untyped @!param (bare-name binder, no `: Type`) is rejected — the B-invariant, an accessor needs a result type"
    );
}

#[test]
fn the_old_at_param_annotation_still_generates_a_param_accessor_migration_compat() {
    // MIGRATION COMPAT (the `@param`→`@!param` transition): the sidecar scans BOTH the new module-
    // directive `(pragma param (param …) (: name Type))` AND the OLD following-form annotation
    // `(: (@ (param …) name) Type)` (`param_pragma_parts`'s second arm). The old annotation STILL PARSES
    // (v-syntax kept it), so accepting it keeps every not-yet-migrated consumer working — the CAD `.cdz`
    // showcases and the guide's embedded `.cdz` model strings still write `@param` while they migrate at
    // their own pace. The whole corpus + every other lib test uses the NEW pragma shape, so WITHOUT this
    // test the compat arm has NO behavioral coverage: a refactor dropping it would keep the gate green
    // while silently unbinding `Param.*` in every unmigrated consumer. This pins the arm until the
    // later cleanup (dropped only once no `@param` annotation remains tree-wide).
    assert!(
        reject_code(
            "(module m \
                   (: (@ (param (: widget slider)) width) Int64) \
                   (def (main) (host (Param) (Param.width))) \
                 (export main))"
        )
        .is_none(),
        "the OLD `(: (@ (param …) name) Type)` annotation still generates the Param effect so (Param.width) resolves — dual-scan migration compat"
    );
    // The old-annotation arm honors the SAME multi-site contract as the new pragma: two old-annotation
    // @param sites generate two ops under one Param effect, both resolve.
    assert!(
        reject_code(
            "(module m \
                   (: (@ (param (: widget slider)) width) Int64) \
                   (: (@ (param (: widget number)) height) Int64) \
                   (def (main) (host (Param) (+ (Param.width) (Param.height)))) \
                 (export main))"
        )
        .is_none(),
        "two OLD-annotation @param sites generate two accessors under one Param effect, both resolve"
    );
}

#[test]
fn a_nested_at_param_pragma_is_a_misplaced_directive_not_a_module_parameter() {
    // PLACEMENT (v-syntax coordination 2026-07-18): `@!param` is a MODULE directive — it parameterizes
    // the whole module (operator ruling, like `@!default-fraction`), so it is well-placed ONLY as a
    // direct top-level member of the program root. A `(pragma param …)` NESTED in a def body / a value
    // position is misplaced. v-syntax confirmed the parser does no placement enforcement (it parses a
    // pragma identically at any depth); the placement judgment is a compile-time semantic one that lives
    // in this crate's pragma pass (the same pass that owns the `param` registry arm). The guard reports a
    // coded CDZ0602 placement fault as the PRIMARY (sorted-first) error, rather than letting the nested
    // pragma's config names (`widget`, `slider`, …) raise only a confusing CDZ0101 unbound cascade.
    assert_eq!(
        reject_code(
            "(module m \
                   (def (helper) (do (pragma param (param (: widget slider)) (: width Int64)) 5)) \
                   (def (main) 0) \
                 (export main))"
        )
        .as_deref(),
        Some("CDZ0602"),
        "a nested `(pragma param …)` is a misplaced module directive — CDZ0602 is the primary fault"
    );
    // And the sidecar does NOT act on a nested pragma: it generates no accessor (so `Param.<name>` from a
    // nested-only declaration is unbound) and surfaces no manifest row. The scan_manifest half:
    use crate::param_sidecar::scan_manifest;
    let ast = crate::testkit::parse(
        "(module m \
               (def (helper) (do (pragma param (param (: widget slider)) (: buried Int64)) 5)) \
               (pragma param (param (: widget slider)) (: real Int64)) \
               (def (main) 0) \
             (export main))",
    );
    let recs = scan_manifest(&ast);
    assert_eq!(
        recs.len(),
        1,
        "only the TOP-LEVEL @!param is a manifest row — the nested `buried` pragma is skipped"
    );
    assert_eq!(
        recs[0].name, "real",
        "the surfaced manifest row is the top-level param, not the nested one"
    );
}

#[test]
fn a_rational_param_desugars_to_num_den_scalar_accessors() {
    // @param sidecar Rational brick (v-effects #13): a heap `Rational` has no host boundary form, so a
    // `@param(…) rate : Rational` desugars to TWO scalar `Int64` accessors `rate-num`/`rate-den` and each
    // `(Param.rate)` USE is rewritten to `(Rational.of (Param.rate-num) (Param.rate-den))`. So a program
    // using a Rational @param RESOLVES (no CDZ0101) — the rewritten use references the two generated
    // scalar ops, not a single un-generated `rate` op. (The run-with-host-values path — num=7/den=2 → 7/2
    // — is covered by the 26-runtime-params corpus; the host-response wiring lives in cdz-run.)
    assert!(
        reject_code(
            "(module m \
                   (pragma param (param (: widget slider)) (: rate Rational)) \
                   (def (main) (host (Param) (Param.rate))) \
                 (export main))"
        )
        .is_none(),
        "a Rational @param desugars to num/den scalar accessors + a guest Rational.of, so it resolves"
    );
    // The desugar is SURGICAL to the rational param: referencing `Param.rate-num` DIRECTLY also resolves
    // (the generated scalar op exists), confirming the num/den pair — not a `rate` op — is what's emitted.
    assert!(
        reject_code(
            "(module m \
                   (pragma param (param (: widget slider)) (: rate Rational)) \
                   (def (main) (host (Param) (+ (Param.rate-num) (Param.rate-den)))) \
                 (export main))"
        )
        .is_none(),
        "the generated scalar accessors rate-num/rate-den resolve directly — the num/den pair is emitted"
    );
    // A scalar param alongside a rational one is UNAFFECTED: the scalar keeps its single typed op and its
    // `(Param.width)` use is NOT rewritten (only rational-typed uses recombine). Both resolve together.
    assert!(
        reject_code(
            "(module m \
                   (pragma param (param (: widget slider)) (: width Int64)) \
                   (pragma param (param (: widget slider)) (: rate Rational)) \
                   (def (main) (host (Param) (+ (Param.width) (Param.rate-num)))) \
                 (export main))"
        )
        .is_none(),
        "a scalar @param beside a rational one keeps its single accessor; only the rational use recombines"
    );
}

#[test]
fn a_qty_rational_magnitude_param_desugars_to_num_den_plus_a_guest_qty_of() {
    // @param sidecar Qty-Rational (Length) brick (v-effects #13 B2): a `(Qty Rational <unit>)` — a
    // Rational-MAGNITUDE quantity, the `@param … : Length` shape — has no host boundary form either. The
    // magnitude crosses as the SAME two scalar `Int64` num/den accessors; the guest recombines with
    // `Rational.of` and RE-ATTACHES the unit guest-side via `Qty.of(…, <unit>)` (the unit is a
    // compile-time value erased at the boundary). So a program using such a @param RESOLVES.
    assert!(
            reject_code(
                "(module m \
                   (pragma param (param (: widget slider)) (: len (Qty Rational (Unit.base #\"meter\")))) \
                   (def (main) (host (Param) (Qty.value (Param.len)))) \
                 (export main))"
            )
            .is_none(),
            "a (Qty Rational unit) @param desugars to num/den scalars + a guest Qty.of, so it resolves"
        );
    // Same as bare Rational, the num/den scalar accessors exist — referencing `Param.len-num` directly
    // resolves, confirming the num/den pair (not a single `len` op) is what's generated for the Qty case.
    assert!(
            reject_code(
                "(module m \
                   (pragma param (param (: widget slider)) (: len (Qty Rational (Unit.base #\"meter\")))) \
                   (def (main) (host (Param) (+ (Param.len-num) (Param.len-den)))) \
                 (export main))"
            )
            .is_none(),
            "the generated len-num/len-den scalars resolve — a Qty-Rational param emits the num/den pair"
        );
    // A `(Qty Int64 …)` (scalar-INNER magnitude) is NOT a num/den case — an Int64 magnitude rides the
    // ordinary scalar host path (a Qty of a scalar crosses as its inner scalar), so it keeps ONE op and
    // its `(Param.size)` use is NOT rewritten. Only a Rational MAGNITUDE triggers the num/den desugar.
    assert!(
            reject_code(
                "(module m \
                   (pragma param (param (: widget slider)) (: size (Qty Int64 (Unit.base #\"meter\")))) \
                   (def (main) (host (Param) (Qty.value (Param.size)))) \
                 (export main))"
            )
            .is_none(),
            "a (Qty Int64 unit) @param stays scalar-inner (one op, no num/den rewrite) — only Rational splits"
        );
}

#[test]
fn eval_of_a_non_compile_time_ast_names_the_form_not_an_unbound_eval() {
    // `eval` desugars ONLY a compile-time-visible AST (`(quote …)` / literal `Ast.*`); a runtime /
    // non-Ast argument does not desugar, so the `eval` head fell through to `resolve` as "unbound name
    // `eval`" — MISLEADING (as if `eval` were a typo, even offering a did-you-mean to a near name). The
    // message now NAMES the real situation: `eval` is a recognized form that executes only a
    // compile-time AST, so a runtime/non-Ast argument has nothing to reconstruct.
    let msg = |src: &str| -> String {
        crate::diagnostics(&mut crate::db::Db::load(parse(src)))
            .into_iter()
            .find(|d| d.code.as_deref() == Some("CDZ0101"))
            .unwrap_or_else(|| panic!("expected CDZ0101 for {src}"))
            .message
    };
    for src in [
        "(module m (def (main) (eval 5)) (export main))", // a scalar (non-Ast)
        "(module m (def (f (: a Ast)) (eval a)) (export f))", // a runtime Ast
    ] {
        let m = msg(src);
        assert!(
            m.contains("COMPILE-TIME-VISIBLE AST") && !m.contains("did you mean"),
            "eval of a non-compile-time AST names the form, not an unbound-name typo: {m}"
        );
    }
    // NO OVER-REACH: a bare `eval`-shaped typo that is NOT an `(eval …)` head still gets the ordinary
    // unbound-name did-you-mean (a near def wins), not the eval-form message.
    let typo = "(module m (def (evil) 5) (def (main) (evel)) (export main))";
    assert!(
        crate::diagnostics(&mut crate::db::Db::load(parse(typo)))
            .iter()
            .any(|d| d.message.contains("did you mean `evil`?")),
        "a near-eval typo keeps the ordinary unbound path"
    );
}

#[test]
fn eval_reconstructs_an_ast_list_whose_payload_is_a_native_list_ctor_literal() {
    // M2 regression guard: `eval` desugars `(Ast.List <list-literal>)` by reconstructing the source the
    // list denotes. The M2 printer renders a list literal as the native `[…]` (a ctor-LEAF-KIND head),
    // which the reader re-reads as `#list(…)` — so a guide example whose ML toggle round-trips through the
    // printer hands `eval` a NATIVE list-ctor payload, not the legacy name-head `(list …)`. Before the fix,
    // `list_elems` recognized only the name (`(list …)`) and string (`("list" …)`) heads, so the native
    // payload had "nothing to reconstruct" → a spurious CDZ0101 on the ML surface while the authored
    // s-expr passed (a surface DIVERGENCE). `list_elems` now recognizes all three head spellings via
    // `compound_form_of`, so `eval` folds identically regardless of how the list literal was spelled.
    // Reconstructs to `(double 21)` → 42, so the whole program compiles clean.
    assert!(
        reject_code(
            "(module m (def (double x) (* 2 x)) \
                   (def (main) (eval (Ast.List #list((Ast.Name \"double\") (Ast.Int 21))))) \
                 (export main))"
        )
        .is_none(),
        "eval reconstructs an Ast.List whose payload is a NATIVE list-ctor literal (M2 printer form)"
    );
    // Control: the legacy name-head `(list …)` payload still reconstructs (no regression on the old form).
    assert!(
        reject_code(
            "(module m (def (double x) (* 2 x)) \
                   (def (main) (eval (Ast.List (list (Ast.Name \"double\") (Ast.Int 21))))) \
                 (export main))"
        )
        .is_none(),
        "eval still reconstructs an Ast.List whose payload is the legacy name-head (list …)"
    );
}

#[test]
fn a_record_field_key_colliding_with_a_param_stays_beta_immune_through_a_native_field_pair() {
    // M2b regression guard (§9 flagship reducer blast radius): a record field KEY is a LABEL, β-IMMUNE to
    // argument substitution. `is_binder_occurrence`'s field-key arm recognized only the transitional
    // name-head `(= …)` ascription; post-M2b a record's fields carry the NATIVE `FieldPair` leaf, whose
    // head is NOT `Name("=")`. So inlining a def whose record has a field key equal to a param
    // (`(def (make (: id …) (: name …)) (record (= id id) (= name name)))`, then `(. (make 3 4) id)`)
    // β-substituted the argument for the KEY `id` → `(record (= 3 3) …)` → CDZ0201 "record field key must
    // be a name". This reds reducer.cdz/verdict.cdz/checker-lib.cdz/check.cdz (field keys collide with
    // params). The arm now recognizes BOTH FieldPair spellings via field_pair_parts/field_pair, so the key
    // stays immune and the projection folds to 3.
    assert!(
        reject_code(
            "(module m \
                   (def (make (: id Int64) (: name Int64)) #record((= id id) (= name name))) \
                   (def (main) (. (make 3 4) id)) \
                 (export main))"
        )
        .is_none(),
        "a record field key colliding with a param is β-immune (native FieldPair), no spurious CDZ0201"
    );
    // The field VALUE (second child) is NOT immune — it legitimately references the param, so it IS
    // substituted: `(make 3 4).name` folds to 4 (value `name` → arg 4), key `name` stays the label.
    assert!(
        reject_code(
            "(module m \
                   (def (make (: id Int64) (: name Int64)) #record((= id id) (= name name))) \
                   (def (main) (. (make 3 4) name)) \
                 (export main))"
        )
        .is_none(),
        "the field VALUE still substitutes (references the param) while the KEY stays the label"
    );
}

#[test]
fn eval_of_a_quote_with_a_non_reifiable_leaf_names_the_literal_not_nothing_to_reconstruct() {
    // A `(quote …)` IS compile-time-visible, so the generic "nothing to reconstruct" (runtime / non-
    // constant) phrasing is WRONG when the quote declined only because it carries a leaf the `Ast` sum
    // has no variant for: a `#"…"` symbol or a `#\c` char (NOT a `b"…"` bytes literal — that reifies
    // to `Ast.Bytes` now, operator seq 113). The message must NAME the offending literal kind so the
    // author knows WHY, not imply a runtime argument. (v-diag / concierge routed this; the root — no
    // `Ast.Symbol`/`Ast.Char` variant — is intended
    // until the symbols vertical, so the fix is the DIAGNOSTIC, not the decline.)
    let msg = |src: &str| -> String {
        crate::diagnostics(&mut crate::db::Db::load(parse(src)))
            .into_iter()
            .find(|d| d.code.as_deref() == Some("CDZ0101"))
            .unwrap_or_else(|| panic!("expected CDZ0101 for {src}"))
            .message
    };
    // A symbol literal inside the quote (the reported `Unit.of #"meter"` shape, plus a bare symbol).
    for (src, phrase) in [
        (
            "(module m (def (main) (eval (quote (Unit.of #\"meter\")))) (export main))",
            "symbol literal",
        ),
        (
            "(module m (def (main) (eval (quote #\"m\"))) (export main))",
            "symbol literal",
        ),
    ] {
        let m = msg(src);
        assert!(
            m.contains(phrase) && m.contains("no `Ast` leaf variant"),
            "eval of a quote carrying a non-reifiable leaf names the literal: {m}"
        );
        assert!(
            !m.contains("nothing to reconstruct"),
            "the misleading runtime/non-constant phrasing must NOT be used for a compile-time quote \
                 that declined on a non-reifiable leaf: {m}"
        );
    }
    // NO REGRESSION: a genuinely runtime/non-Ast eval argument STILL gets the "nothing to reconstruct"
    // message (there is no non-reifiable leaf to name — the argument simply is not a constant AST).
    assert!(
        msg("(module m (def (f (: a Ast)) (eval a)) (export f))")
            .contains("nothing to reconstruct"),
        "a runtime Ast argument keeps the reconstruct-nothing message"
    );
    // A `b"…"` bytes literal is NOT non-reifiable: it reifies to `Ast.Bytes` (operator seq 113), so
    // `(eval (quote b"hi"))` reconstructs + COMPILES CLEAN — not merely "no CDZ0101". Asserting the
    // module compiles with NO error at all is strictly stronger: a regression that re-added a Bytes arm
    // to `first_non_reifiable_leaf` (spurious CDZ0101) OR that broke the reconstruct some OTHER way
    // (leaving a different reject) would both fail here, whereas a CDZ0101-only check would miss the latter.
    let bytes_diags = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def (main) (eval (quote b\"hi\"))) (export main))",
    )));
    assert!(
        bytes_diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "eval of a quoted bytes literal must COMPILE CLEAN — Ast.Bytes reifies + reconstructs; got: {bytes_diags:?}"
    );
}

#[test]
fn an_ast_operand_in_arithmetic_names_the_compile_time_metadata_misuse() {
    // An `Ast` value used in an ARITHMETIC/comparison position — `(eval (quasiquote (+ (unquote (quote
    // …)) 1)))` (the spliced `(quote …)` reconstructs to an `Ast` the surrounding `+` can't consume),
    // or a bare `(+ (quote x) 1)` — used to draw the GENERIC "a Ast and an Int64 are different types",
    // reading as an ordinary user type error. It now names the real category: `Ast` is compile-time
    // metadata, not a runtime value (corpus-bugfix breaker issue). CDZ0201.
    let msg = |src: &str| -> String {
        crate::diagnostics(&mut crate::db::Db::load(parse(src)))
            .into_iter()
            .find(|d| d.code.as_deref() == Some("CDZ0201"))
            .unwrap_or_else(|| panic!("expected CDZ0201 for {src}"))
            .message
    };
    for src in [
        // the breaker probe: eval of a template with a runtime `(quote …)` splice
        "(module m (def (main) (eval (quasiquote (+ (unquote (quote (* 2 3))) 1)))) (export main))",
        // a bare Ast literal in arithmetic
        "(module m (def (main) (+ (quote x) 1)) (export main))",
    ] {
        let m = msg(src);
        assert!(
            m.contains("`Ast` value is compile-time metadata")
                && m.contains("runtime splice")
                && !m.contains("a Ast and"),
            "an Ast arith operand names the compile-time-metadata misuse (not the generic clash): {m}"
        );
    }
    // NO false change: a NON-Ast cross-kind clash keeps the generic boundary message (with the correct
    // article), NOT the Ast-specific one.
    let generic = msg("(module m (def (main) (< 1 \"x\")) (export main))");
    assert!(
        generic.contains("an Int64 and a String are different types")
            && !generic.contains("compile-time metadata"),
        "a non-Ast cross-kind clash keeps the generic message: {generic}"
    );
}

#[test]
fn quasiquote_selectively_evaluates_at_unquote_holes() {
    // 12-metaprogramming §Quasiquote Constructs AST With Selective Evaluation: `(quasiquote T)`
    // reifies like quote, EXCEPT an active `(unquote e)` hole EVALUATES `e` and inserts its value.
    // `crate::quote::reify_active` rewrites the quasiquote, keeping an active unquote's operand LIVE
    // (wrapped `(Ast.Int e)`) so `e` resolves/types as ordinary code. Each below compiles clean; the
    // gate checks the built ASTs' values/equalities.
    // An active unquote embedding a runtime (let-bound) value builds the same node a const fold does,
    // so `` `(f ,x) `` (x=1) equals `(quote (f 1))`.
    assert!(
        reject_code(
            "(module m (def (main) \
                   (let ((x 1)) (= (quasiquote (f (unquote x))) (quote (f 1))))) \
                 (export main))"
        )
        .is_none(),
        "an unquoted runtime value builds the same AST as quote"
    );
    // An unquote MUST evaluate its operand — an UNBOUND name in it is the ordinary CDZ0101, NOT
    // swallowed into inert quoted AST (metaprogramming.md: selective EVALUATION, not a second quote).
    assert_eq!(
        reject_code("(module m (def (main) (quasiquote (a (unquote (+ b 1))))) (export main))")
            .as_deref(),
        Some("CDZ0101"),
        "an active unquote of an unbound name is rejected, not quoted"
    );
    // Quasiquote NESTS: ``(+ ,,x) evaluates only the INNER unquote (depth reaches 1 there); the outer
    // `quasiquote`/`unquote` reify as inert structure. Compiles clean (builds the nested AST value).
    assert!(
        reject_code(
            "(module m (def (main) \
                   (let ((x 2)) (quasiquote (quasiquote (+ (unquote (unquote x))))))) \
                 (export main))"
        )
        .is_none(),
        "a nested quasiquote evaluates only the inner unquote"
    );
    // An ACTIVE splice (`,@`) of a NON-LIST is still CDZ0201. The active splice now reifies (see
    // `active_unquote_splicing_flattens_a_list`), so the pre-desugar `collect_quote_body_syntax` walk
    // no longer sees the splice; the reject is preserved by the `ast-splice-lift` operand check in
    // `check_application` — a `provably_not_list` operand (Int64 `5`) has no elements to splice.
    assert_eq!(
            reject_code("(module m (def (main) (let ((x 5)) (quasiquote (f (unquote-splicing x))))) (export main))")
                .as_deref(),
            Some("CDZ0201"),
            "splicing a non-list is CDZ0201 (the ast-splice-lift operand check, not a miscompile)"
        );
    // An active unquote of a BOOL or STRING literal now LIFTS to the matching `Ast` leaf (`Ast.Bool`/
    // `Ast.Str`) — those value forms are realized. `` `(f ,true) `` and `` `(f ,"x") `` build the same
    // node quote of that literal produces, so they compile clean and equal the quoted form.
    assert!(
        reject_code(
            "(module m (def (main) \
                   (= (quasiquote (f (unquote true))) (quote (f true)))) \
                 (export main))"
        )
        .is_none(),
        "an active unquote of a boolean literal lifts to Ast.Bool (equals the quoted form)"
    );
    assert!(
        reject_code(
            "(module m (def (main) \
                   (= (quasiquote (f (unquote \"x\"))) (quote (f \"x\")))) \
                 (export main))"
        )
        .is_none(),
        "an active unquote of a string literal lifts to Ast.Str (equals the quoted form)"
    );
    // A literal the `Ast` sum has no value variant for yet (a FLOAT — no `Ast.Float`) still cannot
    // lift. It DECLINES honestly (a Todo: "quasiquote produces an AST value (not yet built)"), NOT the
    // leaky CDZ0201 "variant constructor's payload has declared type Int64, but Float64 was applied"
    // the naive `(Ast.Int 2.0)` wrap produced — whose coercion fix would have silently rewritten the
    // author's `2.0`→`2`. The reifier bails so no misleading coded reject + no value-corrupting fix.
    {
        let src = "(module m (def (main) (quasiquote (unquote 2.0))) (export main))";
        let d = reject_full(src);
        assert!(
            d.as_ref().is_none_or(|d| {
                d.code.is_none() && !d.message.contains("variant constructor's payload")
            }),
            "an active unquote of a float literal declines honestly, not the leaky Ast.Int \
                 payload error: {d:?}"
        );
    }
    // REGRESSION GUARD: an active unquote of a NAME (`,n` — a let-bound var or a param) is NOT a literal,
    // it is runtime code that stays LIVE and wraps `(Ast.Int n)`. A `Leaf::Name` is a `Struct::Atom`, so
    // the non-int-LITERAL bail above must EXCLUDE names — else `` `(op-const ,n) `` (n let-bound) that
    // builds an Ast RESULT regresses to a spurious "produces an AST value (not yet built)" decline (the
    // 5 quasiquote corpus cases). Here the result is an `Ast`, so it compiles clean end-to-end.
    assert!(
            reject_code(
                "(module m (def (main) (let ((n 42)) (quasiquote (op-const (unquote n))))) (export main))"
            )
            .is_none(),
            "an active unquote of a let-bound NAME building an Ast result must reify (not bail as a literal)"
        );
}

#[test]
fn active_unquote_splicing_flattens_a_list() {
    // 12-metaprogramming §Unquote-Splicing: an active `,@e` in a quasiquote splices the ELEMENTS of the
    // list `e` into the surrounding form (vs `,e`, which inserts `e` as one element). `reify_active`
    // rewrites `` `(f ,@xs) `` to `(Ast.List (List.concat (list (Ast.Name "f")) (ast-splice-lift xs)))`
    // where the compiler-internal `ast-splice-lift : (List Int64) → (List Ast)` constant-folds each Int
    // element into an `(Ast.Int e)` node. The splice run is const-folded, so a let-bound constant list
    // flows through. Builds the SAME AST as writing the elements out longhand.
    assert!(
        reject_code(
            "(module m (def (main) \
                   (let ((xs (list 1 2 3))) (= (quasiquote (f (unquote-splicing xs))) \
                                               (quote (f 1 2 3))))) \
                 (export main))"
        )
        .is_none(),
        "an active splice of a constant Int list flattens its elements into the surrounding form"
    );
    // The splice-lift is TYPE-DIRECTED across the scalar leaves, not Int-only: each constant element is
    // wrapped in the `Ast` leaf its kind denotes (Float64→`Ast.Float`, Bool→`Ast.Bool`, String→
    // `Ast.Str`), matching the active-unquote `ast-lift` leaf set. Each equality pins the lifted shape
    // against the longhand quote of the same elements — a wrong-tagged leaf (e.g. the old unconditional
    // `Ast.Int` wrap) would make the equality false, and a decline would surface as a non-None reject.
    for (src, kind) in [
        (
            "(module m (def (main) \
                   (let ((xs (list 1.5 2.5))) (= (quasiquote (f (unquote-splicing xs))) \
                                                 (quote (f 1.5 2.5))))) (export main))",
            "float",
        ),
        (
            "(module m (def (main) \
                   (let ((xs (list true false))) (= (quasiquote (f (unquote-splicing xs))) \
                                                    (quote (f true false))))) (export main))",
            "bool",
        ),
        (
            "(module m (def (main) \
                   (let ((xs (list \"a\" \"bb\"))) (= (quasiquote (f (unquote-splicing xs))) \
                                                   (quote (f \"a\" \"bb\"))))) (export main))",
            "string",
        ),
    ] {
        assert!(
            reject_code(src).is_none(),
            "an active splice of a constant {kind} list lifts each element to its matching Ast leaf"
        );
    }
    // An element ALREADY of type `Ast` splices by IDENTITY (a list of pre-built AST fragments) — the
    // same identity `ast-lift` gives an already-`Ast` operand. The fragments appear unchanged, not
    // re-wrapped, so the reified list equals the longhand quote of the same fragments.
    assert!(
        reject_code(
            "(module m (def (main) \
                   (let ((xs (list (Ast.Int 7) (Ast.Int 8)))) \
                     (= (quasiquote (f (unquote-splicing xs))) (quote (f 7 8))))) (export main))"
        )
        .is_none(),
        "an active splice of a constant list of Ast values splices the fragments by identity"
    );
    // A NESTED-list element has no scalar value leaf this increment, so the splice DECLINES (a Todo, the
    // runtime splice map is unbuilt) rather than mis-lifting — reject-don't-miscompile. It is not a
    // CDZ0201 non-list error (the operand IS a list); it simply cannot fold yet.
    assert_eq!(
        reject_code(
            "(module m (def (main) \
                   (let ((xs (list (list 1) (list 2)))) (quasiquote (f (unquote-splicing xs)))) ) \
                 (export main))"
        )
        .as_deref(),
        None,
        "splicing a list of nested lists declines (Ast unbuilt), it is not a CDZ0201 non-list error"
    );
    // The operand of `,@` MUST be a list: `provably_not_list` types (an Int64 literal or a let-bound
    // Int64) have no elements to splice → CDZ0201, matching the pre-desugar reject. The message is the
    // `ast-splice-lift` operand check, not the generic apply-arity path.
    for src in [
        "(module m (def (main) (quasiquote (f (unquote-splicing 5)))) (export main))",
        "(module m (def (main) (let ((x 5)) (quasiquote (f (unquote-splicing x))))) (export main))",
    ] {
        assert_eq!(
            reject_code(src).as_deref(),
            Some("CDZ0201"),
            "splicing a non-list operand is CDZ0201: {src}"
        );
    }
}

#[test]
fn ast_encode_decode_round_trip_over_constant_values() {
    // 12-metaprogramming / ast-encoding.md §The Encoding Is A Bijection: `Ast.encode : Ast → Bytes`
    // serializes an AST value to its canonical bytes, `Ast.decode : Bytes → (Result Ast e)` is the
    // TOTAL inverse. Both constant-fold this increment (a compile-time-visible AST / Bytes value).
    //
    // (1) The BYTE FORMAT is locked by an equality against a hand-written `Bytes.of`: `(Ast.Int 7)`
    //     encodes NON-LOSSILY as tag 0x00 + 1 sign byte (0 = non-negative) + a 4-byte LE magnitude
    //     length (1) + the big-endian minimal magnitude (`07`) → `(0 0 1 0 0 0 7)`. This pins the
    //     declared-default layout (the contract pins the bijection, not the bytes — this test is what
    //     needs updating if the format is versioned; it moved from a fixed 8-byte i64 to this
    //     arbitrary-precision sign+magnitude form for the non-lossy quoted-Ast directive).
    assert!(
        reject_code(
            "(module m (def (main) \
                   (= (Ast.encode (Ast.Int 7)) (Bytes.of (list 0 0 1 0 0 0 7)))) (export main))"
        )
        .is_none(),
        "Ast.encode of (Ast.Int 7) is the canonical bytes 0x00 + sign 0 + len 1 (LE) + magnitude 0x07"
    );
    // (2) A quote-built and a constructor-built AST of the SAME tree encode to IDENTICAL bytes (the
    //     one-canonical-byte-form requirement) — they are the ONE value form.
    assert!(
            reject_code(
                "(module m (def (main) (= (Ast.encode (quote 42)) (Ast.encode (Ast.Int 42)))) (export main))"
            )
            .is_none(),
            "equal trees encode identically, however constructed"
        );
    // (3) decode(encode t) = t over a leaf AND a compound (the round-trip), matched through the total
    //     `(Ok a)` arm.
    for tree in [
        "(Ast.Int 7)",
        "(Ast.List (list (Ast.Name \"g\") (Ast.Int 5)))",
    ] {
        let src = format!(
            "(module m (def (main) (match (Ast.decode (Ast.encode {tree})) \
                   ((Ok a) (= a {tree})) ((Err _) false))) (export main))"
        );
        assert!(
            reject_code(&src).is_none(),
            "encode∘decode round-trips to an equal value: {tree}"
        );
    }
    // (4) decode is TOTAL over EXTERNAL bytes: a non-canonical sequence (bad tag) → `Err`, NOT a trap;
    //     and valid canonical bytes FOLLOWED BY a trailing byte → `Err` (decode consumes the WHOLE
    //     input or reports an error — a valid prefix is not silently accepted).
    assert!(
        reject_code(
            "(module m (def (main) (match (Ast.decode (Bytes.of (list 255 255 255))) \
                   ((Ok _) 1) ((Err _) 0))) (export main))"
        )
        .is_none(),
        "decoding non-canonical bytes yields Err, not a trap"
    );
    assert!(
            reject_code(
                "(module m (def (main) \
                   (match (Ast.decode (Bytes.concat (Ast.encode (Ast.Int 7)) (Bytes.of (list 99)))) \
                     ((Ok _) 1) ((Err _) 0))) (export main))"
            )
            .is_none(),
            "canonical bytes plus a trailing byte decode to Err (whole-input bijection)"
        );
}

#[test]
fn a_list_sub_pattern_destructures_a_sum_variant_payload() {
    // The decision-tree matcher destructures a `(list …)` sub-pattern inside a sum-variant PAYLOAD —
    // the general capability quote patterns rest on (`` `(+ ,a ,b) `` desugars to `(Ast.List (list
    // (Ast.Name "+") a b))`, whose `(list …)` payload binds element-wise). A fixed-arity list pattern
    // tests length + descends each element at `[Payload, Elem(i)]` (folded against a constant list —
    // the corpus quote-pattern scrutinees are constant `(quote …)` values). SCOPE: constant fold only
    // (a runtime list payload declines); a `.. rest` in a payload declines (task #51 runtime tail).
    // A user sum wrapping a list, binding two elements — checks clean (it compiles + folds to 3).
    assert!(
        reject_code(
            "(module m (type W (Wrap (List Int64))) \
                   (def (main) (match (W.Wrap (list 1 2)) ((W.Wrap (list a b)) (+ a b)) (_ 0))) \
                 (export main))"
        )
        .is_none(),
        "a (list a b) sub-pattern binds a sum variant's list payload"
    );
    // A quote pattern `` `(+ ,a ,b) `` IS `(Ast.List (list (Ast.Name "+") a b))` — the literal head
    // `+` matches `(Ast.Name "+")` by equality, `,a`/`,b` bind the operand sub-ASTs. Against `(quote
    // (+ 3 5))` the arm returns `b` = `(Ast.Int 5)`; the desugar + list-payload matcher compose.
    assert!(
        reject_code(
            "(module m (def (main) \
                   (match (quote (+ 3 5)) (`(+ ,a ,b) b) (other other))) \
                 (export main))"
        )
        .is_none(),
        "a quote pattern binds an unquoted operand via the Ast.List list-payload matcher"
    );
    // A literal HEAD constrains by equality: `` `(+ ,a ,b) `` does NOT match a `-`-headed form, so
    // control falls to the catch-all. (The string-literal payload `(Ast.Name "+")` folds by value.)
    assert!(
        reject_code(
            "(module m (def (main) \
                   (match (quote (- 3 5)) (`(+ ,a ,b) 1) (other 0))) \
                 (export main))"
        )
        .is_none(),
        "a literal head in a quote pattern constrains by equality (falls through on a mismatch)"
    );
    // A `.. rest` tail inside a variant payload now BINDS (a `RestFrom(lead)` step, folded against a
    // constant list) — `(W.Wrap (list a .. rest))` over `(list 1 2 3)` binds `a`=1 (and `rest`=[2,3]).
    // Compiles clean.
    assert!(
        reject_code(
            "(module m (type W (Wrap (List Int64))) \
                   (def (main) (match (W.Wrap (list 1 2 3)) ((W.Wrap (list a .. rest)) a) (_ 0))) \
                 (export main))"
        )
        .is_none(),
        "a rest-binder list sub-pattern binds the leading element + tail"
    );
}

#[test]
fn a_scalar_compared_to_a_value_of_erased_type_param_lowers_to_a_scalar_compare() {
    // REGRESSION (v-iterators fused-iterator step): comparing a scalar literal against a value whose
    // static type is an UNRESOLVED type-param var mis-lowered. A value projected from a GENERIC-variant
    // payload — `match it (Iter.Mk(s, f) …)` reaching a tuple element `h` whose type is the erased param
    // `a` in `Mk(s, s -> Option((a, s)))` — compared as `(= h 1)` had `type_of(h)` read the ungrounded
    // var `_` (the final substitution isn't applied at that query) even though the checker UNIFIED it
    // with the `Int64` literal. So `is_scalar(h) && is_scalar(1)` was false and `=` fell through to a
    // `value-eq` HEAP walk — which then DECLINED downstream with "borrowing op operand has an ownership
    // this backend cannot yet prove" because the scalar `1` is a bare `ConstInt`, not a heap operand.
    // Fixed: if EITHER operand is a proven scalar, the comparison is scalar (unification guarantees the
    // other side is that same scalar), so route to `Core::Compare` — the emit grounds the width from the
    // scalar side. This was an inference/lowering mis-route, NOT a Perceus/borrow bug (that decline was
    // the symptom). The construction of the same generic variant was fixed separately (bca5da9e0);
    // this is the runtime STEP (calling the stored closure then comparing its projected result).
    let program = "type Iter = | Mk(s, s -> Option((a, s)))\n\
                       def step_once(it) = match it with | Iter.Mk(s, f) => f(s)\n\
                       def main() = match step_once(Iter.Mk([1, 2], fn(s) => match s with\n\
                       | [] => Option.None(unit)\n\
                       | [h, .. t] => Option.Some((h, t)))) with\n\
                       | Option.None(_) => 0\n\
                       | Option.Some(p) => (match p with | (h, _rest) => if h == 1 then 42 else 7)\n\
                       export { main }";
    let parsed = cadenza_syntax::parser::read_ml(program);
    assert!(parsed.ok(), "the ML program parses: {:?}", parsed.errors);
    let bytes = cadenza_syntax::codec::encode(&parsed.arenas);
    let arenas = crate::codec::decode(&bytes).expect("rcdzc decode");
    let component = compile_component(&crate::codec::encode(&arenas)).expect(
        "comparing a scalar against a value of erased-type-param type must lower (scalar compare), \
             not decline via a value-eq heap walk on a scalar operand",
    );
    // The COMPILE succeeding (above) IS the regression guard: the scalar compare LOWERS rather than
    // declining via a value-eq heap walk. It emits a VALID component. The RUN VALUE (step's head is 1 →
    // the (= h 1) arm → 42) is corpus/conformance territory — this is an ML-surface program the s-expr
    // corpus cannot carry (v-syntax pins the parse→desugar; the corpus pins the desugared arena's run).
    let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    v.validate_all(&component)
        .expect("the erased-type-param scalar-compare component validates");
}

// (a_cross_type_variant_pattern_is_rejected_not_type_confused was already fully covered by corpus
// 05-compound-types (the cross-type variant-pattern section): "a match on a value of one sum type rejects
// a pattern from a different sum type" (T value vs Option Some → CDZ0203), "a cross-type variant pattern
// whose payload width differs is rejected" (U.A on T.A → CDZ0203), "a same-type sum match is accepted and
// binds the payload" (→ 5), plus "a runtime Option carrying a user sum is matched by a nested constructor
// pattern" (the legitimately-nested cross-sum control). Redundant, removed.)
#[test]
fn a_guard_condition_must_be_bool_and_its_faults_surface() {
    // A guarded arm's guard `(guard <pattern> <cond>)` is a boolean predicate gating the arm, so
    // `<cond>` must be Bool — like an `if` condition. A non-Bool guard (`(guard x (+ x 1))`, an Int64)
    // used to compile, using a non-boolean as a branch condition; and a fault INSIDE the guard (an
    // unbound name) was silently accepted because the guard cond was never walked. Both now surface.
    let err = |src: &str| -> crate::abi::Diagnostic {
        crate::diagnostics(&mut crate::db::Db::load(parse(src)))
            .into_iter()
            .find(|d| d.severity == crate::abi::Severity::Error)
            .unwrap_or_else(|| panic!("no error for {src}"))
    };
    // A non-Bool guard condition → CDZ0203, the same "must be Bool" the `if` condition gives.
    let d =
        err("(module m (def (g (: n Int64)) (match n ((guard x (+ x 1)) x) (_ 0))) (export g))");
    assert_eq!(d.code.as_deref(), Some("CDZ0203"), "got: {}", d.message);
    assert!(
        d.message.contains("guard condition must be Bool") && d.message.contains("Int64"),
        "names the guard + the offending type: {}",
        d.message
    );
    // A String guard is likewise rejected (the check is general, not int-specific).
    assert!(
        err("(module m (def (g (: n Int64)) (match n ((guard x \"y\") x) (_ 0))) (export g))")
            .message
            .contains("guard condition must be Bool"),
    );
    // A fault INSIDE a guard condition (an unbound name) now surfaces — the cond is walked.
    assert_eq!(
        err("(module m (def (g (: n Int64)) (match n ((guard x (> x zzz)) x) (_ 0))) (export g))")
            .code
            .as_deref(),
        Some("CDZ0101"),
    );
    // NO false positive: a valid Bool guard compiles clean.
    assert!(
        crate::diagnostics(&mut crate::db::Db::load(parse(
            "(module m (def (g (: n Int64)) (match n ((guard x (> x 0)) x) (_ 0))) (export g))"
        )))
        .iter()
        .all(|d| d.severity != crate::abi::Severity::Error),
        "a Bool guard produces no fault"
    );
}

#[test]
fn a_guarded_scalar_match_desugars_to_an_if_and_goes_branchless() {
    // A GUARDED scalar match is `(if guard body else)` — so it must get the same branchless treatment
    // the plain `if` does (bool-materialization / select), not a structured `if`/`else` block.
    // `(match x ((guard n (> n 100)) 1) (_ 0))` is `(if (> x 100) 1 0)` → bool-materialization:
    // `gt_s ; extend`, no `If`/`Else`/`End`. (A match with a guard cannot use `br_table`, so the
    // desugar loses no dispatch table.)
    use crate::backend::wasm::lir::Lir;
    use crate::backend::wasm::select::select_function_of;
    let mut db = crate::db::Db::load(crate::testkit::parse(
        "(module m (def (f (: x Int64)) (match x ((guard n (> n 100)) 1) (_ 0))) (export f))",
    ));
    let layout = crate::layout::compute(&mut db).expect("layout");
    let d = db.def_by_name("f").expect("f");
    let sig = db.defs[d].params.clone();
    let params: Vec<_> = sig
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
    let code = select_function_of(&mut db, body, &params, &layout, Some(d))
        .expect("select")
        .code;
    assert!(
        !code
            .iter()
            .any(|i| matches!(i, Lir::If(_) | Lir::Else | Lir::End)),
        "a guarded scalar match with constant arms bool-materializes — no if/else block: {code:?}"
    );
    assert!(
        code.iter().any(|i| matches!(i, Lir::I64GtS)),
        "the guard condition `(> n 100)` emits its comparison: {code:?}"
    );
    // The recursive guarded-wildcard `find` loop still tail-loops through the desugar (no stack blowup).
    let mut db2 = crate::db::Db::load(crate::testkit::parse(
        "(module m (def (find (: n Int64)) (match n ((guard x (> x 2)) x) (_ (find (+ n 1))))) \
               (def (main) 0) (export main))",
    ));
    let layout2 = crate::layout::compute(&mut db2).expect("layout");
    let df = db2.def_by_name("find").expect("find");
    let (fp, fb) = {
        let sig = db2.defs[df].params.clone();
        let ps: Vec<_> = sig
            .into_iter()
            .map(|p| {
                let b = db2
                    .ast
                    .as_form(p, ":")
                    .and_then(|t| t.first().copied())
                    .unwrap_or(p);
                (b, crate::infer::type_of(&mut db2, b))
            })
            .collect();
        (ps, db2.defs[df].body.expect("body"))
    };
    let fcode = select_function_of(&mut db2, fb, &fp, &layout2, Some(df))
        .expect("select find")
        .code;
    assert!(
        fcode.iter().any(|i| matches!(i, Lir::Loop(_))),
        "a guarded-wildcard tail-recursive match still compiles to a loop: {fcode:?}"
    );
}

#[test]
fn a_fused_match_arm_binder_re_resolves_on_both_backends_not_shared_as_a_capture() {
    // REGRESSION for the fix to the rest-binder fix (v-rust-backend bisected): the enclosing-capture
    // share in `clone_subtree_db_for_fused` must NOT over-share the FUSED match's OWN arm binder. A
    // Result-pipeline `(match (step1 n) ((Ok v) (step2 v)) ((Err e) (Err e)))` fuses; its arm binders
    // `v`/`e` are `SumPayload` reading the FUSED scrutinee `(step1 n)`. The rust `emit_sum_payload`
    // resolves a binder by its (scrutinee, path), so an over-SHARED binder reads the ORIGINAL now-
    // detached switch → "sum payload has no bound match arm" decline (wasm passed → backend-divergent).
    // FIX: `clone_subtree_db_for_fused` copies a `SumPayload` whose scrutinee IS the fused scrutinee
    // (the arm's own binder — re-resolve against the branch) and shares only one reading an ENCLOSING
    // match (a genuine capture, the rest-binder `c` case). Pins that BOTH backends EMIT the pipeline.
    use crate::testkit::parse;
    let src = "(module m \
          (def (step1 (: n Int64)) (if (< n 0) (Err \"neg\") (Ok n))) \
          (def (step2 (: v Int64)) (if (< v 10) (Ok (+ v 1)) (Err \"big\"))) \
          (def (main (: n Int64)) (match (step1 n) ((Ok v) (step2 v)) ((Err e) (Err e)))) \
          (export main))";
    // WASM emit (always passed — the baseline side).
    let mut dbw = crate::db::Db::load(parse(src));
    let layw = crate::layout::compute(&mut dbw).expect("layout");
    crate::backend::emit(crate::backend::Target::Wasm, &mut dbw, &layw, None, None)
        .expect("a Result-pipeline match must emit wasm");
    // RUST emit (the regressed side — must NOT decline "sum payload has no bound match arm").
    let mut dbr = crate::db::Db::load(parse(src));
    let layr = crate::layout::compute(&mut dbr).expect("layout");
    crate::backend::emit(crate::backend::Target::Rust, &mut dbr, &layr, None, None).expect(
            "a Result-pipeline match must emit rust — the fused arm binder re-resolves, not shared as a capture",
        );
}

#[test]
fn a_nested_match_binder_inside_a_fused_arm_re_resolves_both_backends() {
    // PR#1030 (doc-vs-code reconcile): pins that the narrow `fused_scrut == Some(scrutinee)` copy-test in
    // clone_subtree_db_for_fused correctly handle a NESTED match binder INSIDE the cloned arm (whose
    // scrutinee sits within the clone but is NOT the outer fused scrutinee)? Outer fusion fires on
    // `(match (if c (Some n) (None)) …)`; the Some-arm carries its OWN inner match `(match (g v) …)`
    // whose binder `w` reads `(g v)` — within the cloned arm, ≠ the outer fused scrutinee.
    use crate::testkit::parse;
    let src = "(module m \
          (def (g (: v Int64)) (if (< v 5) (Ok v) (Err (- 0 1)))) \
          (def (f (: c Bool) (: n Int64)) \
            (match (if c (Some n) (None)) \
              ((Some v) (match (g v) ((Ok w) (+ w 100)) ((Err e) e))) \
              ((None) 0))) \
          (def (main) (f true 3)) \
          (export main))";
    let mut dbw = crate::db::Db::load(parse(src));
    let layw = crate::layout::compute(&mut dbw).expect("layout");
    crate::backend::emit(crate::backend::Target::Wasm, &mut dbw, &layw, None, None)
        .expect("PR1030 probe: nested-fused-match-in-arm must emit wasm");
    let mut dbr = crate::db::Db::load(parse(src));
    let layr = crate::layout::compute(&mut dbr).expect("layout");
    crate::backend::emit(crate::backend::Target::Rust, &mut dbr, &layr, None, None)
        .expect("PR1030 probe: nested-fused-match-in-arm must emit rust");
}

#[test]
fn an_inlined_nested_match_in_a_fused_arm_re_resolves_both_backends() {
    // PR#1030 (doc-vs-code reconcile, the (b) shape): an INLINED helper whose body is a match, called
    // inside the cloned arm of an outer fused match. Inlining PINS the inner match's binders via
    // β-substitution (`resolved_subtrees`), so this is the ONLY path where the copy_payload check
    // (`fused_scrut == Some(scrutinee)`) actually runs on a nested binder whose scrutinee is within
    // the clone but ≠ the outer fused scrutinee. If the narrow check under-copies, rust declines
    // "sum payload has no bound match arm" (the a5f7cfafb divergence signature).
    use crate::testkit::parse;
    let src = "(module m \
          (def (inner (: r (Result Int64 Int64))) (match r ((Ok w) (+ w 100)) ((Err e) e))) \
          (def (f (: c Bool) (: n Int64)) \
            (match (if c (Some n) (None)) \
              ((Some v) (inner (if (< v 5) (Ok v) (Err (- 0 1))))) \
              ((None) 0))) \
          (def (main) (f true 3)) \
          (export main))";
    let mut dbw = crate::db::Db::load(parse(src));
    let layw = crate::layout::compute(&mut dbw).expect("layout");
    crate::backend::emit(crate::backend::Target::Wasm, &mut dbw, &layw, None, None)
        .expect("PR1030 probe2: inlined-nested-match-in-fused-arm must emit wasm");
    let mut dbr = crate::db::Db::load(parse(src));
    let layr = crate::layout::compute(&mut dbr).expect("layout");
    crate::backend::emit(crate::backend::Target::Rust, &mut dbr, &layr, None, None)
        .expect("PR1030 probe2: inlined-nested-match-in-fused-arm must emit rust");
}

#[test]
fn a_string_payload_variant_is_not_misjudged_nullary() {
    // REGRESSION: a variant carrying a `String` payload — `(type Tag (Named String) Anon)` — must
    // construct, not be misjudged NULLARY. `eval::encode_ty` (the `Ty → type-value AST` round-trip a
    // constructor scheme takes) had NO `Ty::String` arm, so it hit the `_ => Unit` catch-all: the
    // `Named` ctor's `(-> String Tag)` scheme round-tripped to `(-> Unit Tag)`, and applying `Named`
    // to a `String` arg then unified "cannot unify Unit with String". (The strings vertical added
    // `Ty::String` + `decode_ty`'s `"String"` arm but forgot the `encode_ty` side — the exact hole the
    // `Bytes` arm's comment warned about.) Fixed by `Ty::String => push_name("String")`. Compile-only:
    // a runtime String projection needs the composed runtime (the corpus 13-strings cases run it).
    let src = "(module m (type Tag (Named String) Anon) \
                     (def (main) (match (Tag.Named \"hi\") ((Tag.Named s) 1) (Tag.Anon 0))) \
                   (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "a String-payload variant must COMPILE — not reject Named as nullary (encode_ty must round-trip String)"
    );
}

#[test]
fn a_quantity_type_round_trips_through_encode_and_decode() {
    // L1-0 SAFETY (the DESIGN doc §9 mandatory test): a `Ty::Qty` MUST survive the `Ty → type-value
    // AST → Ty` round-trip that every constructor scheme takes (`eval::encode_typeval` /
    // `resolve::decode_ty`, reached via `typeval_of`). A MISSING `encode_ty`/`decode_ty` arm would
    // silently encode a quantity as `Unit` (the catch-all) — the exact hole that mis-typed `Bytes`
    // and `String` before their arms landed — so a `(-> T (Qty T u))` scheme would round-trip to
    // `(-> T Unit)` and every quantity op would mis-type. This pins the round-trip for the
    // dimensionless unit, a base unit, a derived (velocity) unit, and a squared unit.
    use crate::db::Db;
    use crate::eval::{encode_typeval, typeval_of};
    use crate::testkit::parse;
    use crate::ty::{Ty, Unit};
    let ast = parse("(module m (def (main) 0) (export main))");
    let mut db = Db::load(ast);
    let cases = vec![
        // A dimensionless quantity over Int64 — the empty unit map.
        Ty::Qty {
            inner: Box::new(Ty::int64()),
            unit: Unit::one(),
        },
        // A base-unit quantity over Float64 — `{meter: 1}`.
        Ty::Qty {
            inner: Box::new(Ty::float64()),
            unit: Unit::base("meter"),
        },
        // A DERIVED unit (velocity) over Float64 — meter·second⁻¹, `{meter: 1, second: -1}`.
        Ty::Qty {
            inner: Box::new(Ty::float64()),
            unit: Unit::base("meter").div(&Unit::base("second")),
        },
        // A SQUARED unit (area) over Float64 — `{meter: 2}`.
        Ty::Qty {
            inner: Box::new(Ty::float64()),
            unit: Unit::base("meter").pow(2),
        },
    ];
    for ty in cases {
        let node = encode_typeval(&mut db, &ty);
        let decoded = typeval_of(&mut db, node).unwrap_or_else(|| {
            panic!(
                "a {} type-value must decode",
                ty.render_name(&db.name_ctx())
            )
        });
        assert_eq!(
            decoded,
            ty,
            "a {} type MUST survive the encode/decode round-trip (a missing arm would encode it as Unit)",
            ty.render_name(&db.name_ctx())
        );
    }
}

#[test]
fn a_bigint_type_round_trips_through_encode_and_decode() {
    // B0 SAFETY (the DESIGN-bigint doc §10 mandatory test, the Ty::Qty lesson): `Ty::BigInt` MUST
    // survive the `Ty → type-value AST → Ty` round-trip (`eval::encode_typeval` /
    // `resolve::decode_ty`, via `typeval_of`). A MISSING `encode_ty`/`decode_ty` arm would silently
    // encode `BigInt` as `Unit` (the catch-all) — mis-typing a `(-> (Int N) BigInt)` conversion
    // scheme to `(-> (Int N) Unit)` — exactly the hole the `Bytes`/`String`/`Char`/`Symbol` arms
    // closed. Test bare `BigInt` AND `BigInt` NESTED in a compound (where a missing arm bites: a
    // tuple/variant-payload boxes and would collapse the element to `Unit`).
    use crate::db::Db;
    use crate::eval::{encode_typeval, typeval_of};
    use crate::testkit::parse;
    use crate::ty::Ty;
    let ast = parse("(module m (def (main) 0) (export main))");
    let mut db = Db::load(ast);
    let cases = vec![
        Ty::BigInt,
        // Nested in a tuple — the case a missing arm silently mis-encodes.
        Ty::Tuple(vec![Ty::BigInt, Ty::int64()].into()),
        // Nested in a list element.
        Ty::List(Box::new(Ty::BigInt)),
    ];
    for ty in cases {
        let node = encode_typeval(&mut db, &ty);
        let decoded = typeval_of(&mut db, node).unwrap_or_else(|| {
            panic!(
                "a {} type-value must decode",
                ty.render_name(&db.name_ctx())
            )
        });
        assert_eq!(
            decoded,
            ty,
            "a {} type MUST survive the encode/decode round-trip (a missing arm would encode BigInt as Unit)",
            ty.render_name(&db.name_ctx())
        );
    }
}

#[test]
fn a_rational_type_round_trips_through_encode_and_decode() {
    // B4-0 SAFETY (the DESIGN-bigint doc §10 mandatory test, the Ty::Qty lesson): `Ty::Rational` MUST
    // survive the `Ty → type-value AST → Ty` round-trip. A MISSING `encode_ty`/`decode_ty` arm would
    // silently encode `Rational` as `Unit` (the catch-all) — mis-typing any `(… → Rational)` scheme.
    // Test bare `Rational` AND `Rational` NESTED in a compound (where a missing arm bites).
    use crate::db::Db;
    use crate::eval::{encode_typeval, typeval_of};
    use crate::testkit::parse;
    use crate::ty::Ty;
    let ast = parse("(module m (def (main) 0) (export main))");
    let mut db = Db::load(ast);
    let cases = vec![
        Ty::Rational,
        Ty::Tuple(vec![Ty::Rational, Ty::int64()].into()),
        Ty::List(Box::new(Ty::Rational)),
    ];
    for ty in cases {
        let node = encode_typeval(&mut db, &ty);
        let decoded = typeval_of(&mut db, node).unwrap_or_else(|| {
            panic!(
                "a {} type-value must decode",
                ty.render_name(&db.name_ctx())
            )
        });
        assert_eq!(
            decoded,
            ty,
            "a {} type MUST survive the encode/decode round-trip (a missing arm would encode it as Unit)",
            ty.render_name(&db.name_ctx())
        );
    }
}

// (a_bigint_fixed_int_mix_is_the_numeric_no_promotion_error_cdz0301 migrated to corpus 06-numeric-model: a
// BigInt/fixed-int mix → CDZ0301 + a `BigInt.of` coercion fix (new case); the Rational/int mix + `Rational.of-int`
// fix enriches the existing "a rational operation does not silently promote an integer operand" case; the Bool/int
// control (stays generic CDZ0203) is the existing non-numeric-operand case. All PASS wasm.)

#[test]
fn a_recursive_bigint_result_from_a_match_binder_propagates_to_a_two_self_call_arith_arm() {
    use crate::testkit::parse;
    // A recursive fn whose RESULT type is fixed BigInt by ONE arm's sum-payload binder, whose RECURSIVE
    // arm is `(+ (s a) (s b))` — TWO self-calls, no anchoring literal — must type-check (result BigInt),
    // NOT reject CDZ0203 "match arms differ: BigInt vs Int64". While `s`'s scheme solve is in flight,
    // each self-call `(s a)` types the recursion-guard's provisional `Any`; the `+` arith arm saw two
    // `Any` operands, failed to match the BigInt arm, and committed to the generic deferred-`Int` scheme
    // → frozen `Int`, conflicting with the `BigInt` binder arm. Fix: an arith op with an `Any` operand
    // WHILE a scheme solve is in flight defers to `Any` (uncached), so a clean re-solve — once the
    // self-calls ground BigInt — types `+`-over-two-BigInts as BigInt. (v-metaprogramming's Ast.Int
    // Int64→BigInt flip surfaced it via a recursive Ast evaluator; reduced here to a plain user sum.)
    for src in [
        // sum-recursion: (B a b) recurses on two sub-trees, summed
        "(module m (type T (L BigInt) (B T T)) \
             (def (s (: t T)) (match t ((T.L n) n) ((T.B a b) (+ (s a) (s b))) (_ 0N))) \
             (def (main) (s (T.B (T.L 3N) (T.L 4N)))) (export main))",
        // list-recursion (v-metaprog's exact repro shape): (Branch (list a b))
        "(module m (type Tree (Leaf BigInt) (Branch (List Tree))) \
             (def (st (: t Tree)) \
               (match t ((Tree.Leaf n) n) ((Tree.Branch (list a b)) (+ (st a) (st b))) (_ 0N))) \
             (def (main) (st (Tree.Branch (list (Tree.Leaf 3N) (Tree.Leaf 4N))))) (export main))",
    ] {
        assert!(
            compile_component(&crate::codec::encode(&parse(src))).is_ok(),
            "a recursive BigInt result from a match binder must reach the two-self-call `+` arm \
                 (was CDZ0203 arms-differ): {src}"
        );
    }
    // DISCIPLINE (must NOT regress): the same shape at Int64 still type-checks as Int64 (the defer only
    // fires under an in-flight scheme solve and re-grounds to the operands' real type — Int64 here).
    assert!(
        compile_component(&crate::codec::encode(&parse(
            "(module m (type T (L Int64) (B T T)) \
                 (def (s (: t T)) (match t ((T.L n) n) ((T.B a b) (+ (s a) (s b))) (_ 0))) \
                 (def (main) (s (T.B (T.L 3) (T.L 4)))) (export main))"
        )))
        .is_ok(),
        "the Int64 recursive two-self-call fold still type-checks (defer re-grounds to Int64)"
    );
    // And a genuine BigInt/Int64 MIX still rejects CDZ0301 (the defer did not weaken no-promotion).
    assert_eq!(
        reject_code("(module m (def (f (: n Int64)) (+ (BigInt.of n) 1)) (export f))").as_deref(),
        Some("CDZ0301"),
        "the provisional-operand defer does not weaken the BigInt/fixed-int no-promotion rule"
    );
}

#[test]
fn a_unit_scale_distinguishes_type_identity_from_dimensional_compatibility() {
    // F2-0: a unit carries a compile-time SCALE (num/den) alongside its dimension (exponent map).
    // `same_dimension` compares the MAP alone (gates `+`/`compare` compatibility — `meter` and
    // `kilometer` are the SAME dimension, so combinable); `==` compares MAP + SCALE (type identity —
    // `meter` and `kilometer` are DISTINCT types). `scaled` applies a prefix; the scales multiply
    // under `mul`/`pow`.
    use crate::ty::Unit;
    let meter = Unit::base("meter");
    let km = meter.scaled(1000, 1).expect("kilo scales");
    // Same dimension (both length), different type identity (different scale).
    assert!(
        meter.same_dimension(&km),
        "meter and kilometer share the length dimension"
    );
    assert_ne!(
        meter, km,
        "meter and kilometer are DISTINCT units (different scale)"
    );
    assert_eq!(
        meter.scale(),
        (1, 1),
        "a base unit is the scale-1 reference"
    );
    assert_eq!(
        km.scale(),
        (1000, 1),
        "a kilo-prefixed unit scales the reference by 1000"
    );
    // A different DIMENSION is not compatible.
    let second = Unit::base("second");
    assert!(
        !meter.same_dimension(&second),
        "meter and second are different dimensions"
    );
    // `at_reference` drops the scale (the common unit a conversion lands at).
    assert_eq!(
        km.at_reference(),
        meter,
        "km at its reference scale IS meter"
    );
    // Prefix scales MULTIPLY under `pow`: (km)² has scale 10⁶.
    assert_eq!(km.pow(2).scale(), (1_000_000, 1), "(km)² scales by 10^6");
    // A milli prefix is an exact rational scale 1/1000 (a machine-int ratio, no bignum).
    let mm = meter.scaled(1, 1000).expect("milli scales");
    assert_eq!(
        mm.scale(),
        (1, 1000),
        "millimeter scales the reference by 1/1000"
    );
    assert!(
        meter.same_dimension(&mm) && meter != mm,
        "mm: same dimension, distinct unit"
    );
}

#[test]
fn an_unknown_unit_string_literal_fix_keeps_the_string_delimiter() {
    // WHITE-BOX RESIDUAL of the unknown-unit did-you-mean (its symbol-form fix, near-miss/compose
    // guidance, and Unit.define-base cases are now the corpus 18 unknown-unit cases). Keeps the facet
    // the corpus grade cannot see: when the Unit.of argument is a plain STRING `"metre"` (not the
    // `#"…"` symbol), the rename fix PRESERVES the string delimiter (`"meter"`) — a string arg surfaces
    // a different PRIMARY diagnostic in the grade path, so the did-you-mean rides a non-primary fault
    // the corpus does not match; this pins it directly.
    let d = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def (main) (Qty.of 5 (Unit.of \"metre\"))) (export main))",
    )))
    .into_iter()
    .find(|d| d.message.contains("unknown unit"))
    .expect("no unknown-unit fault");
    let fix = d
        .fix
        .as_ref()
        .expect("the string-form unknown-unit did-you-mean carries a fix");
    assert_eq!(
        fix.replacement, "\"meter\"",
        "a string-literal unit argument keeps the \"…\" delimiter"
    );
}
#[test]
fn remainder_on_same_dimension_integer_quantities_is_well_formed() {
    use crate::testkit::parse;
    // `%` on same-dimension INTEGER quantities is well-formed — `7m % 3m = 1m` (operator ruling
    // 2026-08-28: same-dimension mod makes sense). It mirrors `+`/`-` (same dimension in, SAME unit out)
    // and runs the remainder on the erased magnitudes, so `Qty.value` of `(% 7m 3m)` is `7 % 3 = 1`.
    let ok = "(module m (def (main) ((. Qty value) \
             (% ((. Qty of) 7 ((. Unit base) #\"meter\")) ((. Qty of) 3 ((. Unit base) #\"meter\"))))) \
             (export main))";
    // Compiles cleanly (the run value 7 % 3 = 1 is corpus-covered by 18-units-of-measure).
    assert!(
        compile_component(&crate::codec::encode(&parse(ok))).is_ok(),
        "same-dimension integer remainder on quantities must be well-formed (7m % 3m = 1m)"
    );
    // A cross-DIMENSION remainder is a dimensional error (CDZ0501), exactly like `+`/`-` across
    // dimensions — a remainder requires equal dimensions.
    let cross = reject_full(
            "(module m (def (main) ((. Qty value) \
             (% ((. Qty of) 7 ((. Unit base) #\"meter\")) ((. Qty of) 3 ((. Unit base) #\"second\"))))) \
             (export main))",
        )
        .expect("cross-dimension remainder is rejected");
    assert!(
        cross.message.contains("incompatible dimension"),
        "cross-dimension % names the dimensional cause: {}",
        cross.message
    );
    // A FLOAT-inner quantity `%` is CDZ0301 (a remainder is an integer operation) — the SAME code a
    // bare float `%` gets; it must NOT leak the operator's `∀a.(Int a)→…` scheme.
    let flt = reject_full(
            "(module m (def (main) ((. Qty value) \
             (% ((. Qty of) 7.0 ((. Unit base) #\"meter\")) ((. Qty of) 3.0 ((. Unit base) #\"meter\"))))) \
             (export main))",
        )
        .expect("float quantity remainder is rejected");
    assert!(
        flt.message.contains("floating-point or rational quantity"),
        "float quantity % names the integer-only cause: {}",
        flt.message
    );
}

// (an_if_join_over_different_unit_quantities_names_the_scale_not_a_shadowed_declaration migrated to corpus
// 18-units-of-measure: an if-join over km-vs-m same-dimension quantities → CDZ0203 "SAME dimension at
// DIFFERENT units" + (not "shadows a built-in"); a cross-dimension if-join stays a plain mismatch. PASS wasm.)

// (a_list_of_different_unit_quantities_names_the_scale_not_two_identical_looking_types migrated to corpus
// 18-units-of-measure: a list-element join over km-vs-m same-dimension quantities → CDZ0201 "SAME dimension
// at DIFFERENT units"; a cross-dimension list clash stays a plain mismatch. PASS wasm.)

#[test]
fn a_nominal_over_float32_qty_stored_as_a_map_value_emits_valid_wasm() {
    // MISCOMPILE REGRESSION (v-rust-backend flagged the wasm twin of their rust float_width_of fix): a
    // NOMINAL newtype over a Float32 quantity — `(type Len (Q (Qty Float32 meter)))` — stored as a map
    // value emitted INVALID wasm (`expected f32, found f64`). `peel_qty_ty` peeled a RAW `Ty::Qty` with
    // NO strip_nominal, so a `Nominal(Len, Qty{Float32})` missed the `Ty::Qty` arm and fell to the f64
    // default → an `f64.const` where `box-float32` wanted f32. Fixed: `peel_qty_ty` now does
    // strip_nominal → peel Ty::Qty → strip_nominal (the strip_nominal lockstep the integer `int_ty_of`
    // maintains). `cdz check` passed the mis-lowered program, so the guard is that it VALIDATES.
    let src = "(module m \
                     (type Len (Q (Qty Float32 (Unit.base #\"meter\")))) \
                     (def (main) \
                       (match ((. Map lookup) \
                                ((. Map insert) ((. Map empty)) 1 \
                                 (Len.Q ((. Qty of) ((. Float32 of) 2.5) ((. Unit base) #\"meter\")))) 1) \
                         ((Some _) 1) \
                         ((None) 0))) \
                     (export main))";
    let bytes = component(src);
    wasmparser::validate(&bytes).expect(
        "a nominal newtype over a Float32 Qty as a map value must emit valid wasm (f32, not f64)",
    );
}

#[test]
fn a_float32_qty_stored_as_a_map_value_emits_valid_wasm_through_lookup() {
    // MISCOMPILE REGRESSION (the Float32 twin of the narrow-Int map-value case): a `(Qty Float32 meter)`
    // stored as a MAP VALUE, read back via `Map.lookup` + unwrapped, emitted an INVALID module —
    // `expected f32, found f64`. A quantity over a Float32 erases to its inner f32 slot (boxed via
    // `box-float32`), but the `ConstFloat`/`ConstFloatNan` emit read the solved type to pick f32-vs-f64
    // WITHOUT peeling `Ty::Qty`, so a `(Qty Float32)` magnitude fell to the f64 default → an `f64.const`
    // where `box-float32` wanted an f32. Fixed by peeling `Ty::Qty` in the ConstFloat width readers
    // (both backends — the rust twin emitted `f64::from_bits` into an `f32` map slot → E0308). `cdz check`
    // passed the mis-lowered program, so the precise guard is that the emitted component VALIDATES.
    let src = "(module m (def (main) ((. Qty value) \
                     (match ((. Map lookup) \
                              ((. Map insert) ((. Map empty)) 1 \
                               ((. Qty of) ((. Float32 of) 2.5) ((. Unit base) #\"meter\"))) 1) \
                       ((Some q) q) \
                       ((None) ((. Qty of) ((. Float32 of) 0.0) ((. Unit base) #\"meter\")))))) \
                     (export main))";
    let bytes = component(src);
    wasmparser::validate(&bytes)
        .expect("a Float32 Qty read back via Map.lookup must emit valid wasm (f32, not f64)");
}

#[test]
fn a_standard_unit_abbreviation_registry_aliases_its_canonical_spelling() {
    // A standard ABBREVIATION resolves to the SAME family unit as its canonical spelling (`km` =
    // `kilometer`, `m` = `meter`, `ms` = `millisecond`). The converting-SUM RUN half (`1.0 km + 500.0 m`
    // = 1500 m) is corpus-covered by 18-units-of-measure "a standard unit abbreviation resolves to its
    // canonical unit in a converting sum"; this rcdzc test keeps the direct family-table registry check.
    // The registry carries the abbreviations across dimensions, each aliasing its canonical row's
    // conversion (one source of truth) — a direct check that the family table resolves them.
    let fams = crate::prelude::unit_families();
    for (abbr, canonical) in [
        ("km", "kilometer"),
        ("cm", "centimeter"),
        ("ft", "foot"),
        ("ms", "millisecond"),
        ("min", "minute"),
        ("h", "hour"),
        ("kB", "kilobyte"),
        ("MiB", "mebibyte"),
        ("Hz", "hertz"),
        ("GHz", "gigahertz"),
    ] {
        assert_eq!(
            fams.get(abbr),
            fams.get(canonical),
            "abbreviation `{abbr}` must resolve to the same conversion as `{canonical}`"
        );
    }
    // `in` is the `in` KEYWORD, not a unit ident — it must NOT be registered as an inch abbreviation
    // (a `5 in` quantity is a parse error, handled at the surface, not an unknown-unit lookup).
    assert!(
        !fams.contains_key("in"),
        "`in` is a keyword and must not be a unit abbreviation"
    );
}

#[test]
fn radian_and_degree_are_first_class_units_in_separate_dimensions() {
    // ANGLE units (operator ruling — CAD revolve/rotate angles get their own family, like meter/km).
    // `radian` and `degree` are SEPARATE base DIMENSIONS, NOT one angle dimension: their conversion is
    // IRRATIONAL (180° = π rad, π has no exact Rational), and every family unit keys to an EXACT
    // rational ratio — so one shared dimension would break the exact-Rational invariant. As distinct
    // dimensions each is exact WITHIN itself and mixing them rejects CDZ0501 (honest — not exactly
    // interconvertible). `rad`/`deg`/`radians`/`degrees` alias their canonical spelling.
    let fams = crate::prelude::unit_families();
    for name in ["radian", "degree", "radians", "degrees", "rad", "deg"] {
        assert!(fams.contains_key(name), "angle unit `{name}` is registered");
    }
    assert_eq!(fams.get("rad"), fams.get("radian"), "`rad` = `radian`");
    assert_eq!(fams.get("deg"), fams.get("degree"), "`deg` = `degree`");
    assert_eq!(
        fams.get("radians"),
        fams.get("radian"),
        "`radians` = `radian`"
    );
    // Distinct DIMENSIONS: radian's and degree's conversions differ (their dimension component is a
    // different base), so they are NOT the same unit and never silently interconvert.
    assert_ne!(
        fams.get("radian"),
        fams.get("degree"),
        "radian and degree are DISTINCT dimensions (no exact interconversion)"
    );
    // Exact WITHIN a dimension: `5 degree + 90 degree` COMPILES (Int64 magnitude, a valid same-dimension
    // add). Its RUN value (magnitudes add via Qty.value → 95) is corpus-covered by 18-units-of-measure
    // "adding two quantities of the same dimension keeps that dimension" / "a runtime-magnitude same-unit
    // sum adds the erased magnitudes".
    assert!(
        compile_component(&crate::codec::encode(&parse(
            "(do (def (main) ((. Qty value) \
                   (+ ((. Qty of) 5 ((. Unit of) #\"degree\")) \
                      ((. Qty of) 90 ((. Unit of) #\"degree\"))))) (export main))"
        )))
        .is_ok(),
        "a degree + degree same-dimension sum compiles"
    );
    // Mixing dimensions REJECTS CDZ0501 — degree + radian is not exactly interconvertible, so it is a
    // dimension mismatch, not a silent conversion.
    assert_eq!(
        compile_component(&crate::codec::encode(&parse(
            "(do (def (main) (+ ((. Qty of) 1 ((. Unit of) #\"degree\")) \
                   ((. Qty of) 1 ((. Unit of) #\"radian\")))) (export main))"
        )))
        .err()
        .and_then(|d| d.code.as_deref().map(str::to_string))
        .as_deref(),
        Some("CDZ0501"),
        "degree + radian must reject CDZ0501 (incompatible dimension)"
    );
}

#[test]
fn a_unit_conflict_anchors_to_a_user_node() {
    // CDZ0502 must carry a source location, not print an unanchored `cdz:` prefix. The unit-conflict
    // rejects (built-in redecl + duplicate declaration) now `.at()` the offending declaration's
    // base-unit occurrence, so the error maps to `file:line:col`.
    let src = "(module m (Unit.define #\"foot\" (Unit.of #\"meter\") 2 1) \
                   (def (main) 0) (export main))";
    let mut db = crate::db::Db::load(parse(src));
    let d = crate::diagnostics(&mut db)
        .into_iter()
        .find(|d| d.code.as_deref() == Some("CDZ0502"))
        .expect("a CDZ0502 diagnostic");
    let node = d
        .node
        .expect("CDZ0502 must carry a node, not be unanchored");
    assert!(
        db.is_user_node(crate::ast::StructId(node)),
        "node {node} must be a user node so it maps to a source location"
    );
}

// unit_in_across_dimensions_is_cdz0501 (`(Unit.in meter (Qty.of 3.0 second))` → CDZ0501 naming both
// "second" + "meter") migrated to corpus 18-units-of-measure "Unit.in to a unit of a different dimension
// is a compile-time error" — enriched that case with (message "second") (message "meter"). rcdzc test
// deleted (corpus-covered).
// chaining_two_unit_in_conversions_is_a_clean_cdz0501_not_a_terse_runtime_decline migrated to corpus
// 18-units-of-measure "chaining two Unit.in conversions is a compile-time error — the inner one already
// unwrapped": enriched that case with (message "converts a QUANTITY") (message "which is not a quantity")
// (message "Qty.of") (not "of a non-quantity") — the coded CDZ0501 + the unwrap/re-wrap-with-Qty.of repair
// + the absence of the terse backend 'Unit.in of a non-quantity' decline. The e2e repair (1 inch → 127/50
// cm) is corpus "a chained Unit.in re-wrapped with Qty.of converts inch to cm exactly (127/50)". rcdzc
// test deleted (corpus-covered).
#[test]
fn unit_in_of_a_non_numeric_operand_names_the_type_without_the_self_contradictory_plain_number() {
    // Sibling of the Qty.value fix (Copilot PR#602 pattern), found by a proactive infer.rs audit: the
    // Unit.in-non-quantity CDZ0501 message ALSO hardcoded "— a plain number, not a quantity", firing for
    // ANY non-quantity operand → a Bool operand printed the self-contradictory "a Bool — a plain number".
    // Now: names the real type + "which is not a quantity" generally; the "conversion unwrapped it / chain
    // re-wrap with Qty.of" hint is appended ONLY for a NUMERIC operand (the chained-Unit.in mistake, pinned
    // by the test above). This pins the NON-numeric path.
    let src = "(do (def (main) ((. Unit in) ((. Unit of) #\"meter\") true)) (export main))";
    let err = compile_component(&crate::codec::encode(&parse(src)))
        .expect_err("Unit.in of a Bool must reject");
    assert_eq!(
        err.code.as_deref(),
        Some("CDZ0501"),
        "coded CDZ0501: {}",
        err.message
    );
    assert!(
        err.message.contains("a Bool") && err.message.contains("which is not a quantity"),
        "names the operand's real type (Bool) + not-a-quantity: {}",
        err.message
    );
    assert!(
        !err.message.contains("plain number") && !err.message.contains("Qty.of"),
        "a NON-numeric operand must NOT print 'a plain number' (self-contradictory) nor the numeric-only Qty.of chain hint: {}",
        err.message
    );
}

#[test]
fn qty_value_of_a_conversion_result_is_a_clean_cdz0501_not_a_no_machine_representation_decline() {
    // Breaker/corpus-bugfix report: `Qty.value` of a `Unit.in` conversion RESULT — `(Qty.value (Unit.in
    // inch (Qty.of 5 foot)))` — declined "function return type has no machine representation" at the
    // backend while `cdz check` passed (a check-vs-compile gap). ROOT: `Unit.in`/`as` UNWRAPS to a bare
    // number (Q3), so `Qty.value` is applied to a plain Int, and its type arm returned `Ty::Any`
    // ("faulted elsewhere") — but nothing faulted it, so the un-representable `Any` slipped to the
    // backend. Now a `Qty.value`-of-a-non-quantity check in check_application rejects it CDZ0501 at
    // compile, naming the operand type + the "drop the Qty.value" repair. The bare-number sibling of the
    // chained-Unit.in reject.
    let src = "(do (def (main) ((. Qty value) ((. Unit in) ((. Unit of) #\"inch\") \
                     ((. Qty of) 5 ((. Unit of) #\"foot\"))))) (export main))";
    let err = compile_component(&crate::codec::encode(&parse(src)))
        .expect_err("Qty.value of a conversion result (a bare number) must reject");
    assert_eq!(
        err.code.as_deref(),
        Some("CDZ0501"),
        "Qty.value of a bare number (a Unit.in result) is a coded CDZ0501, not an uncoded no-machine-rep decline"
    );
    assert!(
        err.message.contains("recovers a quantity")
                && err.message.contains("which is not a quantity")
                // A NUMERIC operand (this Unit.in result is a bare Int) keeps the "conversion already
                // unwrapped it — drop the Qty.value" repair hint; a non-numeric operand does not (see the
                // Bool sibling below, which must NOT print the self-contradictory "a plain number").
                && err.message.contains("already UNWRAPS to a bare number"),
        "the message names the operand type + that it is not a quantity + the numeric-operand unwrap repair hint: {}",
        err.message
    );
    assert!(
        !err.message.contains("no machine representation"),
        "the terse backend 'no machine representation' decline must no longer surface: {}",
        err.message
    );
    // The two components each compile ALONE: convert-alone (Unit.in → 60) and extract-alone
    // (Qty.value of a genuine Qty → 5) — only the redundant composition was the gap. Guard the
    // extract-alone still type-checks (compiles) so the reject is scoped to the non-quantity operand.
    let extract_alone =
        "(do (def (main) ((. Qty value) ((. Qty of) 5 ((. Unit of) #\"foot\")))) (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(extract_alone))).is_ok(),
        "Qty.value of a genuine quantity still compiles (the reject is scoped to a non-quantity operand)"
    );
}

#[test]
fn qty_value_of_a_non_numeric_operand_names_the_type_without_the_self_contradictory_plain_number() {
    // Copilot (PR#602): the CDZ0501 Qty.value-not-a-quantity message hardcoded "— a plain number, not a
    // quantity", which fires for ANY non-quantity operand — so a Bool operand printed the
    // self-contradictory "this operand is a Bool — a plain number, not a quantity". The message now names
    // the real type + "which is not a quantity" GENERALLY, and appends the "a conversion already unwrapped
    // it — drop the Qty.value" hint ONLY for a numeric operand (the numeric sibling test above pins that
    // hint). This pins the NON-numeric path: a Bool operand is named + declared not-a-quantity, and must
    // NOT be called "a plain number".
    let src = "(do (def (main) ((. Qty value) true)) (export main))";
    let err = compile_component(&crate::codec::encode(&parse(src)))
        .expect_err("Qty.value of a Bool must reject");
    assert_eq!(
        err.code.as_deref(),
        Some("CDZ0501"),
        "coded CDZ0501: {}",
        err.message
    );
    assert!(
        err.message.contains("a Bool") && err.message.contains("which is not a quantity"),
        "names the operand's real type (Bool) + not-a-quantity: {}",
        err.message
    );
    assert!(
        !err.message.contains("plain number") && !err.message.contains("UNWRAPS"),
        "a NON-numeric operand must NOT print 'a plain number' (self-contradictory) nor the numeric-only unwrap hint: {}",
        err.message
    );
}

#[test]
fn registering_a_family_unit_twice_with_conflicting_conversions_is_an_error() {
    // Operator ask: a name→conversion must be a FUNCTION — registering the same unit name with a
    // DIFFERENT dimension or scale is an error (returns the offending name → CDZ0502 at a future user
    // family-declaration surface), while a duplicate that AGREES is idempotent. `register_families`
    // is the gate the built-in table and any user family flow through.
    use crate::prelude::register_families;
    // A conflict: `foot` twice with different scales (dimension is the `(base, exponent)` list).
    let conflict = register_families(
        [
            ("foot", &[("meter", 1i64)][..], 381i128, 1250i128),
            ("foot", &[("meter", 1)][..], 1, 3), // a bogus, disagreeing scale
        ]
        .into_iter(),
    );
    assert_eq!(
        conflict.err().as_deref(),
        Some("foot"),
        "a name registered with two different conversions is a conflict"
    );
    // A conflict on the DIMENSION too (same name, different dimension).
    assert!(
        register_families(
            [
                ("x", &[("meter", 1i64)][..], 1i128, 1i128),
                ("x", &[("second", 1)][..], 1, 1)
            ]
            .into_iter()
        )
        .is_err(),
        "same name under two dimensions conflicts"
    );
    // An AGREEING duplicate is idempotent (harmless), not an error.
    let ok = register_families(
        [
            ("inch", &[("meter", 1i64)][..], 127i128, 5000i128),
            ("inch", &[("meter", 1)][..], 127, 5000),
        ]
        .into_iter(),
    );
    assert_eq!(
        ok.ok().and_then(|m| m.get("inch").cloned()),
        Some((vec![("meter".to_string(), 1)], 127, 5000)),
        "an agreeing re-registration is idempotent"
    );
    // A DERIVED-dimension unit (a rate) registers with a multi-entry dimension.
    let rate = register_families(
        [(
            "mbps",
            &[("byte", 1i64), ("second", -1)][..],
            1_000_000i128,
            8i128,
        )]
        .into_iter(),
    );
    assert_eq!(
        rate.ok().and_then(|m| m.get("mbps").cloned()),
        // Canonicalized (sorted by base): byte before second.
        Some((
            vec![("byte".to_string(), 1), ("second".to_string(), -1)],
            1_000_000,
            8
        )),
        "a derived-dimension rate unit registers with its full exponent map"
    );
    // The built-in table itself registers without conflict (it is validated through the same gate).
    assert!(crate::prelude::unit_families().contains_key("foot"));
    assert!(crate::prelude::unit_families().contains_key("mbps"));
}

#[test]
fn a_generic_sum_with_a_type_param_in_a_tuple_or_record_payload_is_not_nullary() {
    // REGRESSION: a GENERIC sum whose variant carries a TUPLE or RECORD payload MENTIONING a type
    // parameter — `(type Box (B (Tuple a Int64)) N)` / `(type Box (B (Record (val a))) N)` — must
    // construct, not be misread as NULLARY. `type_in_env` (which reduces a generic variant ctor's
    // `(meta t)` type-lambda to its scheme) handled `Int`/`Fn`/`List`/`Sum`/`UInt` compound payloads
    // but had NO `Tuple`/`Record` arm, so a param nested in a tuple/record payload made the ctor arrow
    // unreadable → `variant_payload_type` = None → `B` looked NULLARY → CDZ0201 on the construction.
    // A bare/`List a`/`Option a` payload worked (those arms existed); the gap was tuple + record.
    // Fixed by adding `TupleCtor`/`RecordCtor` arms to `type_in_env` (reduce each element/field type
    // under the env). The bug was a compile-time REJECT (the ctor looked nullary → CDZ0201), so
    // COMPILING the construction is the precise guard; the runs are exercised by the corpus cases "a
    // generic sum with a type parameter inside a tuple/record payload …" (which link the runtime).
    let tup = "(module m (type Box (B (Tuple a Int64)) N) \
                     (def (main) (match (Box.B (tuple 7 8)) ((Box.B (tuple x y)) (+ x y)) (Box.N 0))) \
                   (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(tup))).is_ok(),
        "a generic sum with a type param in a TUPLE payload must compile — not reject B as nullary"
    );
    let rec = "(module m (type Box (B (Record (val a))) N) \
                     (def (main) (match (Box.B (record (= val 7))) ((Box.B r) (. r val)) (Box.N 0))) \
                   (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(rec))).is_ok(),
        "a generic sum with a type param in a RECORD payload must compile — not reject B as nullary"
    );
}

#[test]
fn a_variant_payload_with_a_nested_comma_tuple_over_two_type_params_is_not_nullary() {
    // REGRESSION (v-iterators, the fused-iterator `Mk(state, step: state -> Option(elem, state))`
    // shape): a variant payload containing a NESTED tuple written in the ML COMMA spelling `(a, s)` —
    // `type Iter = | Mk(s, s -> Option((a, s)))` — could not be CONSTRUCTED: every `Iter.Mk(x, f)`
    // reported CDZ0201 "a nullary variant takes the unit value", as if `Mk` had no payload. The DECL
    // type-checked; only construction failed, and it failed for a closure, an annotated closure, OR a
    // named function — so NOT an inference/element-tie gap (the type was fully known). Root cause: an
    // ML comma-tuple `(a, s)` resolves to `Resolved::Tuple`, but `type_in_env` (which reduces the
    // ctor's `(meta t)` type-lambda to its scheme) only had a `Prim::TupleCtor` arm for the `(Tuple …)`
    // APPLICATION spelling — the `Resolved::Tuple` spelling fell to the concrete `typeval_of` arm,
    // which cannot reduce a param-bearing tuple → `None`. That made `scheme_of` for the whole ctor
    // `None`, so `variant_payload_type` read `None` → `Mk` looked NULLARY. Fixed by adding a
    // `Resolved::Tuple` arm to `type_in_env` (reduce each element under the env), the comma-spelling
    // twin of the `TupleCtor` arm the previous regression added. This is a compile-time reject, so
    // COMPILING the construction is the precise guard. ML source (the spelling that produces the
    // `Resolved::Tuple`); a runtime step is a separate backend concern (a closure-in-variant ownership
    // decline, not this construction bug).
    let program = "type Iter = | Mk(s, s -> Option((a, s)))\n\
                       def popper(s) = match s with\n\
                       | [] => Option.None(unit)\n\
                       | [h, .. t] => Option.Some((h, t))\n\
                       def from_list(xs) = Iter.Mk(xs, popper)\n\
                       def main() = match from_list([1, 2, 3]) with\n\
                       | Iter.Mk(_s0, _step) => 0\n\
                       export { main }";
    let compile_ml = |program: &str| -> bool {
        let parsed = cadenza_syntax::parser::read_ml(program);
        assert!(parsed.ok(), "the ML program parses: {:?}", parsed.errors);
        let bytes = cadenza_syntax::codec::encode(&parsed.arenas);
        let arenas = crate::codec::decode(&bytes).expect("rcdzc decode");
        compile_component(&crate::codec::encode(&arenas)).is_ok()
    };
    // GENERIC: the iterator shape, payload nests `(a, s)` over two type params.
    assert!(
        compile_ml(program),
        "constructing a variant whose payload nests a comma-tuple `(a, s)` over TWO type params must \
             compile — the ctor must not be misread as nullary (CDZ0201)"
    );
    // GROUND: the same shape fully monomorphic — `(Bool, Int64)` nested in the payload. This exercised
    // the `typeval_of` (concrete reducer) sibling gap, which ALSO lacked a `Resolved::Tuple` arm.
    let ground = "type Pair = | Mk(Int64, Int64 -> Option((Bool, Int64)))\n\
                      def build(f) = Pair.Mk(1, f)\n\
                      def main() = 0\n\
                      export { main }";
    assert!(
        compile_ml(ground),
        "a GROUND variant payload nesting a comma-tuple `(Bool, Int64)` must compile too — the \
             concrete `typeval_of` reducer must reduce a `Resolved::Tuple`, not read the ctor as nullary"
    );
}

#[test]
fn a_const_adapter_iterator_chain_fuses_to_zero_call_indirect() {
    // FUSION REGRESSION GATE (v-iterators request, operator's zero-cost criterion). The operator's
    // headline acceptance for the fused iterator is that a CONST-annotated adapter chain
    // (from-list |> map |> filter |> sum) devirtualizes + fuses so the emitted wasm has NO
    // `call_indirect` — the closures are known at their call sites (const params on `filter-step`'s
    // `step`/`p` and `drive`'s `step`/`g`), so each apply becomes a direct call, not a table dispatch.
    // The iterators package `@tests` check the VALUE (24); nothing pinned the emit SHAPE, so a future
    // compiler change (closure-devirt / spec-memo degrading) could silently UN-fuse to N `call_indirect`
    // and every value test would stay green. opt-sweep does NOT cover it either (it asserts value-
    // equivalence across O0..O3 — an un-fused chain is value-equal, just slower). This gate compiles the
    // exact fusing witness and asserts BOTH the value (24, correctness) AND 0 `call_indirect` (the
    // zero-cost property). Witness source: v-iterators' fleet queue witness-fusion-gate-const-adapter.
    let program = "type Iter = | Mk(List(Int64), List(Int64) -> Option((Int64, List(Int64))))\n\
            def from-list(xs) = Iter.Mk(xs, fn(s) => match s with\n\
              | [] => Option.None(unit)\n\
              | [h, .. t] => Option.Some((h, t)))\n\
            def map(it, f: Int64 -> Int64) = match it with\n\
              | Iter.Mk(s0, step) => Iter.Mk(s0, fn(s) => match step(s) with\n\
                | Option.None(_) => Option.None(unit)\n\
                | Option.Some(p) => (match p with | (x, s2) => Option.Some((f(x), s2))))\n\
            def filter-step(const step: List(Int64) -> Option((Int64, List(Int64))), s: List(Int64), const p: Int64 -> Bool) =\n\
              match step(s) with\n\
              | Option.None(_) => Option.None(unit)\n\
              | Option.Some(pr) => (match pr with | (x, s2) => if p(x) then Option.Some((x, s2)) else filter-step(step, s2, p))\n\
            def filter(it, p: Int64 -> Bool) = match it with\n\
              | Iter.Mk(s0, step) => Iter.Mk(s0, fn(s) => filter-step(step, s, p))\n\
            def drive(const step: List(Int64) -> Option((Int64, List(Int64))), s: List(Int64), acc: Int64, const g: Int64 -> Int64 -> Int64) =\n\
              match step(s) with\n\
              | Option.None(_) => acc\n\
              | Option.Some(p) => (match p with | (x, s2) => drive(step, s2, g(acc, x), g))\n\
            def sum(it) = match it with | Iter.Mk(s, step) => drive(step, s, 0, fn(a, x) => a + x)\n\
            def main() = sum(filter(map(from-list([1, 2, 3, 4, 5]), fn(x) => x * 2), fn(x) => x > 4))\n\
            export { main }";
    let parsed = cadenza_syntax::parser::read_ml(program);
    assert!(parsed.ok(), "fusion witness parses: {:?}", parsed.errors);
    let bytes = cadenza_syntax::codec::encode(&parsed.arenas);
    let arenas = crate::codec::decode(&bytes).expect("rcdzc decode");
    let component = compile_component(&crate::codec::encode(&arenas)).expect("compile");
    // This gate pins the emit SHAPE (0 call_indirect); the VALUE (24) is already covered by the
    // iterators package `@tests` and by running the corpus — and running a heap-using (List) program
    // here would need the value-heap runtime linked into the test harness, which this shape-only gate
    // does not require. So compile + count opcodes; do not run.
    // The zero-cost property: the const-adapter closures devirtualize, so NO table dispatch remains.
    let indirect = super::super::count_opcode(&component, |op| {
        matches!(op, wasmparser::Operator::CallIndirect { .. })
    });
    assert_eq!(
        indirect, 0,
        "the const-annotated adapter chain must FUSE to 0 call_indirect (the operator's zero-cost \
             criterion); a non-zero count means it silently UN-fused to runtime closure dispatch"
    );
}

#[test]
fn shadowing_a_prelude_payload_type_name_is_a_plain_rebind_not_a_phantom_variant_fault() {
    // Defining a value named after a prelude type — `(def (Int64) 1)` — must be a plain rebind, not a
    // fault. The variant-payload validation walked ALL type declarations (INCLUDING the prelude's), so
    // a prelude sum whose payload is typed `Int64`/`String` re-validated against the user's now-shadowed
    // namespace, found the name bound to a nullary FUNCTION (not a type), and reported "a variant
    // payload requires a type" — at the PRELUDE payload node, which has no source span, so the fault
    // printed with NO `line:col` and named a "variant payload" the user never wrote. Gating the walk on
    // `is_user_node` fixes it: a prelude decl's payloads are not re-checked against the user namespace.
    for name in ["Int64", "String"] {
        let src = format!("(module m (def ({name}) 1) (def (main) ({name})) (export main))");
        assert!(
            compile_component(&crate::codec::encode(&parse(&src))).is_ok(),
            "shadowing prelude type `{name}` with a nullary def must compile cleanly"
        );
    }
    // The check is NOT weakened: a USER variant that itself names a shadowed prelude type as a payload
    // DOES still fault — `Int64` is genuinely bound to a value there, so `(A Int64)` is a non-type
    // payload — and the fault lands at the USER's payload node (a real span), not the prelude's.
    let shadow_and_use = compile_component(&crate::codec::encode(&parse(
        "(module m (def (Int64) 1) (type C (A Int64)) (def (main) 0) (export main))",
    )))
    .expect_err("using a value-shadowed `Int64` as a payload is a non-type payload");
    assert_eq!(
        shadow_and_use.code.as_deref(),
        Some("CDZ0203"),
        "got: {}",
        shadow_and_use.message
    );
}

/// A `(Record (field Type)…)` PARAMETER ANNOTATION whose field TYPE is unknown — `(: r (Record (x
/// Nonesuch)))` — reports ONLY the bad type `Nonesuch`, not the field LABEL `x`. `param_annotation_faults`
/// used to `collect` the whole record type expression as a VALUE, mis-resolving the label `x` as an
/// unbound NAME (a misleading "unbound name `x`") alongside the real "unbound name `Nonesuch`". Now it
/// uses the record-aware type-position split (the same `push_payload_type_positions` /
/// `validate_type_position` the variant-payload check uses), which skips field labels and validates only
/// the field TYPES.
#[test]
fn an_unknown_type_in_a_record_parameter_annotation_names_only_the_type_not_the_field_label() {
    use crate::testkit::parse;
    // All THREE annotation sites — a PARAMETER annotation, a VALUE annotation `(: value T)`, and a
    // LET-BINDER annotation — share the record-aware validator, so a record-type annotation with a bad
    // field TYPE names only the type (`Nonesuch`), never the field LABEL (`x`/`a`/`b`). Before, each
    // site's naive value-`collect` fallback mis-resolved the label as an unbound value name.
    for src in [
        // parameter annotation
        "(module m (def (g (: r (Record (x Nonesuch)))) r) (export g))",
        // nested: the deep field type is the only fault, no labels flagged.
        "(module m (def (g (: r (Record (a (Record (b Nonesuch)))))) r) (export g))",
        // value annotation `(: value T)`
        "(module m (def (main) (: 5 (Record (x Nonesuch)))) (export main))",
        // let-binder annotation
        "(module m (def (main) (let (((: r (Record (x Nonesuch))) (record (x 5)))) r)) (export main))",
    ] {
        let diags = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
        assert!(
            diags
                .iter()
                .any(|d| d.code.as_deref() == Some("CDZ0101") && d.message.contains("`Nonesuch`")),
            "the unknown field type is named: {src} -> {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        // The field LABELS (`x` / `a` / `b`) must NOT be reported unbound — they are labels, not values.
        assert!(
            !diags.iter().any(|d| d.message.contains("unbound name `x`")
                || d.message.contains("unbound name `a`")
                || d.message.contains("unbound name `b`")),
            "a record-type field LABEL must not be reported unbound: {src} -> {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }
    // NO false positive: a well-formed record annotation compiles clean.
    assert!(
        !crate::diagnostics(&mut crate::db::Db::load(parse(
            "(module m (def (g (: r (Record (x Int64) (y Bool)))) r) (export g))"
        )))
        .iter()
        .any(|d| d.severity == crate::abi::Severity::Error),
        "a well-formed record parameter annotation is clean"
    );
}

/// A match every UNGUARDED arm of which yields the SAME value COLLAPSES to that value — the probe
/// chain is dropped (the match analogue of `(if c x x)` → `x`). `(match a (1 x) (2 x) (_ x))` always
/// returns `x`, so it lowers to just `x` with no `i64.eq`/branch on `a`. Sound because the scrutinee
/// here is a trap-free parameter (nothing to preserve by evaluating it); a trapping scrutinee is
/// covered by `a_trapping_scrutinee_of_an_all_same_match_is_still_evaluated`.

// (a_non_wildcard_pattern_after_a_literal_still_needs_a_wildcard migrated to corpus 02-binding-and-control:
// two literal arms with no wildcard → non-exhaustive CDZ0210. PASS wasm.)

// (a_bool_match_missing_a_literal_is_still_non_exhaustive migrated to corpus 02-binding-and-control: a Bool
// match with only true / only false / two-of-the-same-literal is non-exhaustive (CDZ0210); both-arms is
// exhaustive and runs; an Int64 literal match without a wildcard stays CDZ0210. All PASS wasm.)

#[test]
fn do_local_record_binding_projection_is_not_a_destructure() {
    // A DO-BLOCK value-def binds a bare name to a record; a later projection reads a field. `compute.rs`
    // routes the value-def through `lower_let` as a SELF-KEYED `(V, V)` binding, and the destructure-let
    // fast-path once misfired on it — reading the record VALUE `.0` as a destructure PATTERN and routing
    // `(def p0 #record(…))` through a single-arm match of the record against itself, which match_tree
    // then rejected as a non-exhaustive sum match (spurious CDZ0210, corpus-15 regression). Both the
    // top-level and the do-local forms must compile cleanly (the do-local binding is a bare name, never a
    // destructure). Pins the `.0 != .1` guard in `lower_let`.
    assert_eq!(
        reject_code("(module m (def (main) (. #record((= x 1) (= y 2)) y)) (export main))")
            .as_deref(),
        None,
        "top-level record projection compiles"
    );
    assert_eq!(
        reject_code(
            "(module m (def (main) (do (def p0 #record((= x 1) (= y 2))) (. p0 y))) (export main))"
        )
        .as_deref(),
        None,
        "a do-local record binding + projection is a bare-name binding, not a destructure — no CDZ0210"
    );
}
