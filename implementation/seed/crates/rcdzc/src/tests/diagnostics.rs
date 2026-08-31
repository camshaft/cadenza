use crate::abi::Artifact;
use crate::ast::StructId;
use crate::backend::Target;
use crate::compile::compile;
use crate::db::Db;
use crate::testkit::parse;

/// Compile a program and return its first error diagnostic.
fn first_error(src: &str) -> crate::abi::Diagnostic {
    let ast = parse(src);
    let bytes = crate::codec::encode(&ast);
    let out = compile(
        &[Artifact::new(Artifact::KIND_AST, "m", bytes)],
        &[Target::Wasm],
    );
    out.diagnostics
        .into_iter()
        .find(|d| d.severity == crate::abi::Severity::Error)
        .expect("an error")
}

/// seq-286: the umbrella `CDZ0900` unsupported-construct decline. `Reject::unsupported` carries the
/// code, STILL grades as a decline (`is_decline`), and renders on the wire exactly like any coded
/// error (`error [CDZ0900]: …`) so the corpus decline-code grader parses it with no special-casing.
#[test]
fn unsupported_construct_decline_carries_cdz0900_and_is_still_a_decline() {
    use crate::diag::{Code, Reject};
    assert_eq!(Code::UnsupportedConstruct.code(), "CDZ0900");

    let r = Reject::unsupported("a widget of this shape is not supported");
    assert_eq!(r.code, Some(Code::UnsupportedConstruct));
    assert!(
        r.is_decline(),
        "a CDZ0900 unsupported-construct is a DECLINE (safe not-yet), not a program-is-wrong reject"
    );
    // A codeless decline is still a decline; a program-is-wrong coded reject is NOT.
    assert!(Reject::decline("x").is_decline());
    assert!(!Reject::coded(Code::TypeMismatch, "y").is_decline());

    // Wire shape: severity Error, code Some("CDZ0900") — same shape as any error diagnostic.
    let d = crate::abi_bridge::diagnostic_from_reject(&r);
    assert_eq!(d.severity, crate::abi::Severity::Error);
    assert_eq!(d.code.as_deref(), Some("CDZ0900"));
    assert_eq!(d.message, "a widget of this shape is not supported");
}

fn all_errors(src: &str) -> Vec<crate::abi::Diagnostic> {
    let ast = parse(src);
    let bytes = crate::codec::encode(&ast);
    let out = compile(
        &[Artifact::new(Artifact::KIND_AST, "m", bytes)],
        &[Target::Wasm],
    );
    out.diagnostics
        .into_iter()
        .filter(|d| d.severity == crate::abi::Severity::Error)
        .collect()
}

// (a_duplicate_record_field_carries_a_delete_the_duplicate_fix migrated to corpus 05-compound-types
//  "a record with a duplicate field name is a type error" — enhanced with (fix (kind delete) (unverified))
//  now that the corpus (error ...) form grades fix-quality (C1 #5255).)

// (a_wrong_arity_record_or_map_entry_offers_a_delete_the_surplus_fix migrated to corpus 05-compound-types:
//  surplus record-field + map-entry (name-alias + primitive) → (fix (kind delete)); too-few record + map
//  → (no-fix). All CDZ0201 with the entry-shape message.)

// (a_wrong_arity_type_annotation_names_the_operand_count_at_every_arity migrated to corpus 07-type-system:
//  malformed `(: …)` annotation arity → CDZ0201 naming the form + actual part count ("0/1/3 part(s) … here"),
//  each with (not "takes exactly") guarding the emit-path dedup-filter regression; + a well-formed
//  two-operand control that runs clean.)

// (a_duplicate_field_in_a_record_type_is_rejected_like_the_value_form — its record-TYPE dup-field rejects
// were already pinned in corpus 05-compound-types ("a record TYPE with a duplicate field name is a type
// error" [annotation] + "a duplicate field in a variant's record-type payload is a type error" [payload],
// both CDZ0201); this batch ENHANCED those two cases with the message + fix facets the rust test added:
// (message "record type names field `x` more than once")(fix (kind delete)). --case grades the message;
// the delete fix is source+sibling-proven. The distinct-fields no-regression control is covered broadly.)

// (a_duplicate_export_carries_a_delete_the_duplicate_fix migrated to corpus 11-modules "a duplicate export
//  clause for the same name is rejected" — enhanced with (fix (kind delete)) now that the corpus (error ...)
//  form grades fix-quality (C1 #5255).)

// (a_duplicate_sum_variant_op_and_map_key_each_carry_a_delete_fix migrated to corpus: dup variant + dup op
//  → 11-modules "a duplicate sum variant declaration carries a delete fix" / "…effect operation…";
//  dup map key → 05-compound-types "a duplicate literal map key carries a delete fix". All CDZ0201 (fix (kind delete)).)

// (a_duplicate_type_declaration_is_rejected_and_carries_a_delete_fix migrated to corpus 11-modules
//  "a duplicate type declaration is rejected with a delete fix" (CDZ0201 + (fix (kind delete))) + the
//  no-overreach twin "two distinct type names are not a duplicate" — fix-quality graded via C1 #5255.)
// (an_integer_operand_to_a_float_operator_offers_an_of_int_coercion_fix migrated to corpus 06-numeric-model
//  "an integer operand to a float operator offers an of-int coercion fix" — CDZ0301 with the multi-substring
//  message (no implicit conversion / Float64 / Int64) + (fix (kind wrap) (replacement "(Float64.of-int …)")
//  (unverified)); the multi-message form landed via C1 #5277.)

// (a_non_integer_float_annotated_int_names_the_fix_path_not_a_bare_mismatch migrated to corpus
// 06-numeric-model by ENHANCING the two existing drop-fraction cases with the message facets it
// protected: "a non-integer float literal annotated an integer type carries NO drop-fraction fix"
// ((: 2.5 Int64) → CDZ0203 (message "fractional part")(message "annotate a float type")
// (message "round/truncate")(no-fix) — names WHY + the two real paths, not a bare mismatch) + "an
// integer-valued float literal annotated an integer type offers a drop-the-fraction fix (Int64)"
// ((: 3.0 Int64) → CDZ0203 (message "drop the fractional form")(fix (kind replace)(replacement "3"))
// — the guard that the clean literal-retype path survives). --case grades the message facets.)

#[test]
fn a_partial_application_of_a_builtin_operation_declines_honestly_naming_the_op() {
    // A built-in operation applied to too FEW arguments — `(. List at) (list 1)`, missing the index —
    // is a partial application: a genuine not-yet-built construct (it would need a runtime closure). It
    // used to leak the INTERNAL `reduce_ctor` sentinel `error: not a type constructor` (the op fell
    // through lower's full-arity arms into the constructor catch-all). Now it declines HONESTLY, naming
    // the operation from its `Operand.key` surface spelling and stating the real limitation.
    let d = first_error("(module m (def (main) ((. List at) (list 1))) (export main))");
    assert!(
        !d.message.contains("not a type constructor"),
        "the internal reduce_ctor sentinel must not surface: {}",
        d.message
    );
    assert!(
        d.message.contains("`List.at`")
            && d.message.contains("wrong arity")
            && d.message.contains("runtime closure"),
        "names the op + the real limitation: {}",
        d.message
    );
    // A FULL application of the same op still compiles (the honest decline did not over-fire).
    let full = all_errors("(module m (def (main) ((. List at) (list 1) 0)) (export main))");
    assert!(
        full.is_empty(),
        "a fully-applied operation is not declined: {:?}",
        full.iter().map(|e| &e.message).collect::<Vec<_>>()
    );
}

// (a_narrower_int_operand_to_a_float_operator_nests_the_int64_widening migrated to corpus 06-numeric-model:
// "a NARROWER integer operand to a float operator nests the Int64 widening in the of-int wrap"
// ((+ 2.0 x), x:Int32 → CDZ0301 (message "no implicit conversion")(fix (kind wrap)
// (replacement "(Float64.of-int (Int64.of …))")) — a bare (Float64.of-int x) would itself fail since
// of-int takes Int64, so the one-shot fix widens first). --case grades code+message; the nested
// replacement facet is graded by nix corpus-grade (identical to the landed annotation-position sibling
// "a non-literal NARROWER integer annotated a float nests the Int64 widening in the of-int wrap").)

// (a_non_numeric_mismatch_to_a_float_operator_carries_no_coercion_fix migrated to corpus 06-numeric-model
//  "a non-numeric operand mixed with a float operator carries NO coercion fix" (CDZ0203 (no-fix)).)
// (an_integer_width_mismatch_offers_an_of_conversion_fix migrated to corpus 06-numeric-model:
// "an integer-width mismatch under an operator offers an of-conversion wrap fix" ((+ a b), a:Int32
// b:Int64 → CDZ0301 (fix (kind wrap)(replacement-contains "(Int32.of ")(unverified))) + the ROUND-TRIP
// "applying the integer-width of-conversion wrap clears the mismatch and runs" ((+ a (Int32.of b)),
// f(5,3) → 8 : Int32). NOTE: the applied form now COMPILES AND RUNS — the "declines at emit" the old
// rust comment described (checked int `.of` narrowing of a runtime operand) has since been fixed, so
// the corpus pins a real run value, a stronger pin than the old "CDZ0301 is cleared" type-level assert.
// --case grades the reject code + the run value; the (Int32.of …) wrap facet by nix corpus-grade
// (replacement-contains precedent: the "(Int64.of "/"(Int8.of " annotation-position siblings are landed).)

// (a_float_precision_mismatch_names_floats_and_offers_an_of_conversion_fix migrated to corpus
// 06-numeric-model: "a float precision mismatch under an operator names floats and offers an
// of-conversion wrap fix" (CDZ0301, (message "floating-point precisions differ")(message "a float")
// (fix (kind wrap)(replacement "(Float32.of …)")(unverified))) + "a float annotation to a wider
// precision offers an of-conversion wrap fix" (CDZ0203, (Float64.of …) wrap) + the two ROUND-TRIP
// value cases "applying the {operator,annotation} float coercion wrap recompiles and runs" (float
// `.of` is total, so the applied wrap RUNS: 4.0 and 1.5 — a stronger pin than the rust compiles-clean
// assert). The "not integer" negative + the not-"checked" label are the corpus-inexpressible remainder
// covered by the positive float-domain (message …) substrings.)

#[test]
fn over_application_offers_a_delete_the_extra_argument_fix() {
    // Applying MORE arguments than a function/ctor/operator accepts (CDZ0203/CDZ0201) carries a
    // `delete` fix removing the FIRST surplus argument. The SIMPLE ctor + user-fn over-application
    // delete-fix migrated to corpus 09-functions ("over-applying a constructor is a type error" +
    // "over-applying a named function by an extra argument is a type error", each now carrying
    // (fix (kind delete) (unverified))). What REMAINS here is corpus-inexpressible: the DEDUP /
    // no-secondary sub-cases (exactly-ONE error after a sibling reject is dropped), the 1-of-2 curry
    // no-false-positive, and the member-op multi-substring message — none expressible in the corpus
    // (error …) form (single (message …), no total-count / no-other-code).
    // A fixed-arity OPERATOR `(+ 1 2 3)` produces TWO faults (the grammar CDZ0201 "+ takes exactly 2
    // operands" + the generic CDZ0203 over-application). They are the same defect — dedup keeps ONE,
    // the authoritative CDZ0201, carrying the delete fix on the surplus operand.
    let over = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def (f) (+ 1 2 3)) (export f))",
    )));
    let errs: Vec<_> = over
        .iter()
        .filter(|d| d.severity == crate::abi::Severity::Error)
        .collect();
    assert_eq!(
        errs.len(),
        1,
        "operator over-application reports ONCE (dedup drops the CDZ0203 sibling): {errs:?}"
    );
    assert_eq!(
        errs[0].code.as_deref(),
        Some("CDZ0201"),
        "the authoritative arity reject"
    );
    assert_eq!(
        errs[0].fix.as_ref().map(|f| f.kind),
        Some(crate::abi::FixKind::Delete),
        "the surviving CDZ0201 carries the delete fix"
    );
    // A ONE-of-two `(+ 1)` now CURRIES (operator ruling: "operators should curry") into `\b. 1 + b` — it
    // is no longer an arity error, so it reports NO CDZ0201 "takes exactly 2 operands". (The former
    // too-few reject is retired for the 1-of-2 case; the ZERO-operand `(+)` and the over-applications
    // still fault.)
    let few = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def (f) (+ 1)) (export f))",
    )));
    assert!(
        few.iter().all(|d| !(d.code.as_deref() == Some("CDZ0201")
            && d.message.contains("takes exactly 2 operands"))),
        "a 1-of-2 partial operator curries into a closure, no arity reject: {few:?}"
    );
    // The COMPARISON `(< 1 2 3)` and float arithmetic `(+ 1.0 2.0 3.0)` (the ONE `+` over float
    // operands) share the exact shape — they route through `lower_comparison`/`lower_float_arith`,
    // which previously lacked the delete fix (so they DOUBLE-reported: CDZ0201 + an un-deduped
    // CDZ0203). Via the shared `binop_arity_reject` they now report ONCE with the fix, exactly like
    // integer `+`.
    for src in [
        "(module m (def (f) (< 1 2 3)) (export f))",
        "(module m (def (f) (+ 1.0 2.0 3.0)) (export f))",
    ] {
        let errs: Vec<_> = crate::diagnostics(&mut crate::db::Db::load(parse(src)))
            .into_iter()
            .filter(|d| d.severity == crate::abi::Severity::Error)
            .collect();
        assert_eq!(
            errs.len(),
            1,
            "a comparison/float operator over-application reports ONCE: {errs:?} for {src}"
        );
        assert_eq!(errs[0].code.as_deref(), Some("CDZ0201"), "for {src}");
        assert_eq!(
            errs[0].fix.as_ref().map(|f| f.kind),
            Some(crate::abi::FixKind::Delete),
            "the surviving reject carries the delete fix for {src}"
        );
    }
    // A NAMED-MEMBER CONVERSION op over-applied — `(Int64.of 5 6)` / `(Float64.of 1.0 2.0)` — routes
    // through `lower`'s `lower_conversion`/`lower_float_of`, which emit a coded CDZ0201 "of takes
    // exactly 1 operand" ALONGSIDE `infer`'s member over-application CDZ0203 "`Int64.of` takes 1
    // argument, but 2 were given" (with the delete fix). They are the same defect — dedup keeps ONE,
    // the op-NAMING CDZ0203 with its fix, dropping the bare emit-path arity reject.
    for (src, op) in [
        (
            "(module m (def (main) (Int64.of 5 6)) (export main))",
            "Int64.of",
        ),
        (
            "(module m (def (main) (Float64.of 1.0 2.0)) (export main))",
            "Float64.of",
        ),
    ] {
        let errs: Vec<_> = crate::diagnostics(&mut crate::db::Db::load(parse(src)))
            .into_iter()
            .filter(|d| d.severity == crate::abi::Severity::Error)
            .collect();
        assert_eq!(
            errs.len(),
            1,
            "a member-op conversion over-application reports ONCE (emit arity reject deduped): {errs:?} for {src}"
        );
        assert_eq!(errs[0].code.as_deref(), Some("CDZ0203"), "for {src}");
        assert!(
            errs[0].message.contains(op) && errs[0].message.contains("were given"),
            "the surviving reject NAMES the op `{op}`: {}",
            errs[0].message
        );
        assert_eq!(
            errs[0].fix.as_ref().map(|f| f.kind),
            Some(crate::abi::FixKind::Delete),
            "the surviving CDZ0203 carries the delete fix for {src}"
        );
    }
}

// (a_wrong_type_constructor_payload_offers_the_same_coercion_fix_as_an_argument migrated to corpus 07-type-system:
//  variant-ctor payload VALUE mismatch -> CDZ0201 + the same coercion fixes as argument position — Int8→Int64
//  (Int64.of wrap), 3.0→Int64 ("3" drop-fraction replace), String→Bytes (String.to-bytes wrap), Bool→Int64 (no-fix).)
// (a_bare_literal_over_int64_that_fits_uint64_offers_an_annotate_uint64_fix migrated to corpus 06-numeric-model:
//  bare-literal past Int64.max -> CDZ0201 annotate-UInt64 wrap fix (2^64-1 + the 2^63 boundary), past UInt64.max
//  -> annotate-BigInt fix, and the BigInt.of cascade -> no-fix + "write the literal directly as a" message.)
// (a_non_aliased_int_width_target_carries_no_conversion_fix migrated to corpus 06-numeric-model
//  "a mixed-int-width operator with a non-aliased target width carries NO conversion fix" (CDZ0301 (no-fix)).)

// (an_int_annotation_mismatch_offers_an_of_conversion_fix migrated to corpus 06-numeric-model:
//  "an int annotation to a wider/narrower type offers an of-conversion wrap fix" ((fix (kind wrap)
//  (replacement-contains "(Int64.of "/"(Int8.of ")) + "a bool value annotated an integer type carries
//  NO coercion fix" ((no-fix)).)

// (an_integer_valued_float_literal_annotated_int_offers_a_drop_the_fraction_fix migrated to corpus
//  06-numeric-model: the drop-fraction Replace cases (Int64 "3" / Int8 "100") + the no-fix halves
//  (non-integer 2.5 / out-of-range 500.0 → CDZ0203 (no-fix)).)

// (an_integer_literal_annotated_a_float_offers_an_add_the_fraction_fix migrated to corpus 06-numeric-model:
//  the add-fraction Replace cases (Float64 "3.0" / Float32 "5.0") + the no-fix half (Bool annotated
//  Float → CDZ0203 (no-fix)).)

// (a_non_literal_integer_annotated_a_float_offers_an_of_int_wrap migrated to corpus 06-numeric-model:
//  Int64→Float64 (replacement "(Float64.of-int …)"), Int32→Float64 nested "(Float64.of-int (Int64.of …))",
//  and the literal control (: 3 Float64) → (replacement "3.0"). All CDZ0203.)
#[test]
fn a_cross_kind_operator_clash_uses_the_correct_indefinite_article() {
    use crate::ty::{FloatTy, IntTy, Ty};
    // `Ty::render_with_article` prefixes the correct indefinite article for a message that reads
    // "<this> and <that> are different types". The article keys off the type's SOUND: the signed
    // integers (`Int…`, "eye-nt") take `an`; `UInt…` ("yoo") and every other name take `a`. A naive
    // first-letter rule would wrongly say "an UInt8", so this must be sound-based.
    assert_eq!(
        Ty::Int(IntTy::fixed(true, 64)).render_with_article(&crate::ty::NameCtx::new(&[])),
        "an Int64"
    );
    assert_eq!(
        Ty::Int(IntTy::fixed(true, 32)).render_with_article(&crate::ty::NameCtx::new(&[])),
        "an Int32"
    );
    assert_eq!(
        Ty::Int(IntTy::fixed(false, 8)).render_with_article(&crate::ty::NameCtx::new(&[])),
        "a UInt8"
    );
    assert_eq!(
        Ty::Float(FloatTy::f64()).render_with_article(&crate::ty::NameCtx::new(&[])),
        "a Float64"
    );
    assert_eq!(
        Ty::Bool.render_with_article(&crate::ty::NameCtx::new(&[])),
        "a Bool"
    );
    assert_eq!(
        Ty::String.render_with_article(&crate::ty::NameCtx::new(&[])),
        "a String"
    );
    // Other VOWEL-initial names also take `an` (was the bug: only `Int…` did, so `Ast`/`Any`/`Option`
    // wrongly read "a Ast"). `Bytes`/`Char` stay `a`; `Unit`/`UInt…` keep `a` (the "yoo" exception).
    assert_eq!(
        Ty::Bytes.render_with_article(&crate::ty::NameCtx::new(&[])),
        "a Bytes"
    );
    assert_eq!(
        Ty::Unit.render_with_article(&crate::ty::NameCtx::new(&[])),
        "a Unit"
    );
}

// (a_mismatched_comparison_drops_the_uncoded_heap_walk_decline migrated to corpus 07-type-system: the 3
//  cross-kind compares (< 1 "x", < / = of #tuple vs #list) each pin CDZ0201 "different types" + (count 1)
//  dedup AND (no-diagnostic "needs a heap walk") — the uncoded consequent decline is dropped, via the
//  program-scoped (no-diagnostic) lever #6765 that (not …)/(count) can't express.)
// (a_match_pattern_head_naming_a_non_variant_suggests_the_nearest_variant migrated to corpus 05-compound-types
//  (adjacent to "a match pattern naming a non-existent variant is a coded rejection"): the pattern-head
//  variant-typo enrichment — near-typo did-you-mean + replace fix / far-typo "closest matches" list + no-fix,
//  across qualified (C.Alph→CDZ0201), bare (Alph→CDZ0101), and wrong-sum (D.Gamma→CDZ0203) heads.)
#[test]
fn a_newtype_scrutinees_own_ctor_pattern_still_matches() {
    // WHITE-BOX residual (compile-validity — corpus-inexpressible). The near/far wrong-ctor SUGGESTION
    // halves over a single-variant `(type A (Xyz))` newtype scrutinee (which erases to a `Ty::Nominal`,
    // once a separate reject path with no suggestion; now shares `enrich_pattern_head_suggestion` with
    // the boxed-sum path) migrated to corpus 05-compound-types ("a near-typo/far-typo ctor over a
    // single-variant newtype …"). What stays here: the newtype's OWN ctor pattern `(Mk n)` still matches
    // + binds — no false reject from the shared enrich path — a compile_component `.is_ok()` pin.
    assert!(
        crate::compile::compile_component(&crate::codec::encode(&parse(
            "(module m (type UserId (Mk Int64)) (def (f (: u UserId)) (match u ((Mk n) n))) (export f))"
        )))
        .is_ok(),
        "the newtype's own ctor pattern still matches"
    );
}

#[test]
fn many_match_patterns_typoing_one_variant_suggest_the_memoized_winner() {
    // The nearest-variant suggestion for a mistyped match-pattern head is MEMOIZED per (sum-decl,
    // mistyped-key), so a WIDE sum matched with a stale variant name from N sites (a renamed variant
    // still named at N match arms) shares one edit-distance scan instead of re-running it each — the
    // O(N²) fix. N defs each match `(T.V0x)` (a typo of `V0`) on an 8-variant sum. The identical
    // (code, message) faults dedup in the surfaced set, but every one exercises the memoized lookup
    // during lowering; this locks in that the memo yields the CORRECT winner `V0` (not a stale/empty
    // answer) — a wrong memo key or a mis-combined result would surface a different variant or none.
    let n = 15;
    let variants = (0..8)
        .map(|i| format!("V{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let defs = (0..n)
        .map(|i| format!("(def (d{i} (: t T)) (match t ((T.V0x) {i}) (_ -1)))"))
        .collect::<Vec<_>>()
        .join(" ");
    let src =
        format!("(module m (type T {variants}) {defs} (def (main) (d0 (T.V0))) (export main))");
    let mut db = crate::db::Db::load(parse(&src));
    let sugg: Vec<String> = crate::diagnostics(&mut db)
        .into_iter()
        .filter(|d| d.code.as_deref() == Some("CDZ0201"))
        .filter_map(|d| d.fix.map(|f| f.replacement))
        .collect();
    assert!(
        !sugg.is_empty(),
        "the typo'd `V0x` pattern is reported: {sugg:?}"
    );
    assert!(
        sugg.iter().all(|s| s == "V0"),
        "every surfaced suggestion is the memoized winner `V0`: {sugg:?}"
    );
}

#[test]
fn many_wrong_sum_ctor_arms_list_the_matched_variants_in_bounded_time() {
    // REGRESSION (perf): a match-arm ctor that is a valid variant of a DIFFERENT sum (`(B.Wrong)`
    // against an `A` scrutinee) is a FAR miss, so `lower::enrich_pattern_head_suggestion` lists the
    // scrutinee sum's closest variants via `suggest::closest_matches` — which SORTS all N variants by
    // edit distance (O(N log N)). fix-26 memoized only the TIER-1 nearest WINNER; the TIER-2 far-miss
    // LIST re-ran per site, so N wrong-sum arms against a wide N-variant sum were O(N² log N) (cdz
    // check 400/800/1600 = 264/944/3780ms, ~3.5×/doubling). FIX: memoize the closest-matches list per
    // `(decl, key)` in `db.variant_closest_matches` (the far-miss twin of `variant_suggest_winner`) +
    // build the variant-names list only on a memo MISS.
    //
    // Correctness: the far-miss enrichment still LISTS the matched type's variants (the diagnostic the
    // author needs), just computed once per distinct query.
    fn wrong_sum_src(n: usize) -> String {
        let variants: String = (0..n)
            .map(|i| format!("(V{i})"))
            .collect::<Vec<_>>()
            .join(" ");
        let defs: String = (0..n)
            .map(|i| format!("(def (f{i} (: a A)) (match a ((B.Wrong) {i}) (_ 0)))"))
            .collect::<Vec<_>>()
            .join(" ");
        let binds: String = (0..n)
            .map(|i| format!("(r{i} (f{i} (V0)))"))
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "(module m (type A {variants}) (type B (Wrong)) {defs} \
                   (def (main) (let ({binds}) r0)) (export main))"
        )
    }
    // The far-miss enrichment still lists the matched type's variants (a `— closest matches:` message).
    let diags = crate::diagnostics(&mut crate::db::Db::load(parse(&wrong_sum_src(8))));
    assert!(
        diags.iter().any(|d| {
            d.code.as_deref() == Some("CDZ0203") && d.message.contains("closest matches:")
        }),
        "a wrong-sum ctor arm lists the matched type's variants: {diags:?}"
    );
    // Growth guard at N vs 2N wrong-sum arms against an N-variant sum. The NOISE-FREE signal is
    // `VARIANT_CLOSEST_MATCHES_MISSES` — the far-miss "closest matches" sorts actually COMPUTED, NOT
    // wall-clock. A wall-clock ratio false-fails under fleet load (a narrow run in a quiet slice vs a
    // wide run hitting a scheduling stall inflates the ratio past threshold — the flake). The per-site
    // sort re-ran once per wrong arm → O(N) misses (and O(N² log N) work); the per-`(decl, key)` memo
    // collapses them: all N arms here share the ONE key `(B, "Wrong")`, so misses stay CONSTANT (1)
    // regardless of N. Assert the miss count does NOT grow with N (a revert to the per-site sort makes
    // it grow ~linearly with the arm count).
    fn closest_misses(src: &str) -> u64 {
        crate::db::VARIANT_CLOSEST_MATCHES_MISSES.with(|c| c.set(0));
        let _ = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
        crate::db::VARIANT_CLOSEST_MATCHES_MISSES.with(|c| c.get())
    }
    let m400 = closest_misses(&wrong_sum_src(400));
    let m800 = closest_misses(&wrong_sum_src(800));
    assert!(
        m400 > 0 && m800 <= m400 + 1,
        "N wrong-sum-ctor arms sharing one wrong head must sort closest-matches ONCE (memoized per \
             (decl, key)), not per site: 400→800 arms grew the closest-matches sorts from {m400} to {m800} \
             (memoized is constant ≈1; the per-site sort grew ~linearly with the arm count)"
    );
}

#[test]
fn many_typod_field_accesses_of_one_wide_record_suggest_in_bounded_time() {
    // REGRESSION (perf): a `(. r k)` on a field `k` the record lacks reports "no field `k` — did you
    // mean?" via `infer::no_field_reject`, which builds the record's O(fields) name list and
    // edit-distance-scans it TWICE (`nearest` for the fix + `did_you_mean` for the message). A WIDE
    // record (N fields) with a typo'd field accessed from N sites re-ran that per access → O(N²)
    // (cdz check 400/800/1600 = 111/402/1135ms, ~3.4×/doubling). This is the record-field twin of the
    // variant did-you-mean (fix-26/45), which was memoized but this site was not. FIX: memoize the
    // (winner, hint) pair per `(reduced-record occ, key)` in `db.no_field_suggestion` — N accesses over
    // ONE record share its reduced occurrence, so the suggestion computes once.
    //
    // Correctness: the enrichment still names the nearest field (`— did you mean` / `— closest
    // matches:`), just computed once per distinct query.
    fn wide_record_typos(n: usize) -> String {
        let rec: String = format!(
            "(record {})",
            (0..n)
                .map(|i| format!("(= k{i} {i})"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let accesses: String = (0..n)
            .map(|i| format!("(v{i} (. rr k0x))"))
            .collect::<Vec<_>>()
            .join(" ");
        format!("(module m (def (main) (let ((rr {rec}) {accesses}) v0)) (export main))")
    }
    // The enrichment still suggests the nearest field for the typo'd `k0x` access.
    let diags = crate::diagnostics(&mut crate::db::Db::load(parse(&wide_record_typos(8))));
    assert!(
        diags.iter().any(|d| {
            d.code.as_deref() == Some("CDZ0212")
                && d.message.contains("no field")
                && (d.message.contains("did you mean") || d.message.contains("closest matches"))
        }),
        "a typo'd field access is enriched with a suggestion: {diags:?}"
    );
    // Growth guard at N vs 2N typo'd accesses of an N-field record. The NOISE-FREE signal is
    // `NO_FIELD_SUGGESTION_MISSES` — the field-name-list builds + edit-distance scans actually
    // COMPUTED, NOT wall-clock. A wall-clock ratio false-fails under fleet load (a narrow run in a
    // quiet slice vs a wide run hitting a scheduling stall inflates the ratio past threshold — the
    // flake). The per-access build+double-scan re-ran once per access → O(N) misses (and O(N²) work);
    // the per-`(record occ, key)` memo collapses them: all N accesses here share the ONE key
    // `(rr, "k0x")`, so misses stay CONSTANT (1) regardless of N. Assert the miss count does NOT grow
    // with N (a revert to the per-access scan makes it grow ~linearly with the access count).
    fn field_misses(src: &str) -> u64 {
        crate::db::NO_FIELD_SUGGESTION_MISSES.with(|c| c.set(0));
        let _ = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
        crate::db::NO_FIELD_SUGGESTION_MISSES.with(|c| c.get())
    }
    let m400 = field_misses(&wide_record_typos(400));
    let m800 = field_misses(&wide_record_typos(800));
    assert!(
        m400 > 0 && m800 <= m400 + 1,
        "N typo'd accesses of one wide record sharing one missing key must build+scan the field-name \
             list ONCE (memoized per (record occ, key)), not per access: 400→800 accesses grew the \
             `no_field_suggestion` computes from {m400} to {m800} (memoized is constant ≈1; the per-access \
             scan grew ~linearly with the access count)"
    );
}

// (an_int_let_binder_annotation_mismatch_offers_an_of_conversion_fix + a_known_type_let_binder_mismatch_keeps_its_coercion_fix
//  migrated to corpus 07-type-system: let-binder annotation MISMATCH -> CDZ0203 "a binder annotated T is bound to a value
//  of type U" + the same coercion fixes as a value annotation — Int64.of wrap, "3.0"/"3" literal retype (replace),
//  Float64.of-int wrap, String.to-bytes wrap, (Some …) sum-wrap, and Bool->Int64 no-fix.)
#[test]
fn a_non_exhaustive_match_on_a_function_param_surfaces_in_the_diagnostics_query() {
    // The `diagnostics()` query (what `cdz check`/`--json`/`fix` run) checks well-formedness over
    // EVERY def body, but the reached-poison (lowering) walk runs only on nullary EXPORTED bodies — so
    // a non-exhaustive match on a function PARAMETER (the common case) was silently missed by `check`.
    // Now `collect_node`'s match arm surfaces the CDZ0210 (with its "add the missing arm" fix), whether
    // the def is exported or not, so an agent using `check` sees the actionable fix.
    let src_exported = "(module m (type C (A) (B) (D)) \
             (def (f (: c C)) (match c ((A) 1) ((B) 2))) (export f))";
    let d: Vec<_> = crate::diagnostics(&mut Db::load(parse(src_exported)))
        .into_iter()
        .filter(|d| d.code.as_deref() == Some("CDZ0210"))
        .collect();
    assert_eq!(d.len(), 1, "one non-exhaustive fault: {d:?}");
    assert!(
        d[0].message.contains("`D`") && d[0].message.contains("not covered"),
        "names the missing variant: {}",
        d[0].message
    );
    let fix = d[0].fix.as_ref().expect("carries the add-arm fix");
    assert_eq!(fix.kind, crate::abi::FixKind::InsertInto);
    assert_eq!(fix.replacement, "(D (trap \"TODO: D\"))");

    // A NON-exported function's non-exhaustive match is caught too — it escapes emission entirely
    // (dead, never laid out), so this is the only place it is reported.
    let src_unexported = "(module m (type C (A) (B) (D)) \
             (def (f (: c C)) (match c ((A) 1) ((B) 2))) (def (main) 0) (export main))";
    assert!(
        crate::diagnostics(&mut Db::load(parse(src_unexported)))
            .iter()
            .any(|d| d.code.as_deref() == Some("CDZ0210")),
        "a non-exhaustive match in an uncalled function is still flagged"
    );

    // An EXHAUSTIVE match stays clean — no false positive.
    let src_ok = "(module m (type C (A) (B) (D)) \
             (def (f (: c C)) (match c ((A) 1) ((B) 2) ((D) 3))) (export f))";
    assert!(
        crate::diagnostics(&mut Db::load(parse(src_ok)))
            .iter()
            .all(|d| d.code.as_deref() != Some("CDZ0210")),
        "an exhaustive match produces no non-exhaustive fault"
    );
}

#[test]
fn a_mistyped_variant_pattern_on_a_function_param_surfaces_in_the_diagnostics_query() {
    // The pattern-fault TWIN of the non-exhaustiveness case above. A MISTYPED variant pattern head
    // (`((C.Gren) …)` on `(type C Red Green)`) is a CODED CDZ0201 carrying a "did you mean `Green`?"
    // REPLACE fix — but it was produced ONLY by the emit-path lowering walk, which runs on nullary
    // EXPORTED bodies alone. So a variant typo in ANY parameterized function's match silently PASSED
    // `cdz check` (exit 0, no diagnostic) while `compile` rejected it — hiding the very fix from the
    // fast check path. `collect`'s match arm now surfaces it whether the def takes parameters or not.
    let src = "(module m (type C Red Green) \
             (def (g (: c C)) (match c ((C.Red) 1) ((C.Gren) 2))) (export g))";
    let d: Vec<_> = crate::diagnostics(&mut Db::load(parse(src)))
        .into_iter()
        .filter(|d| d.code.as_deref() == Some("CDZ0201"))
        .collect();
    assert_eq!(
        d.len(),
        1,
        "one mistyped-variant fault on the parameterized body, reported exactly once: {d:?}"
    );
    assert!(
        d[0].message.contains("`Gren`") && d[0].message.contains("did you mean `Green`?"),
        "names the mistyped variant AND the near one: {}",
        d[0].message
    );
    let fix = d[0]
        .fix
        .as_ref()
        .expect("carries the did-you-mean replace fix");
    assert_eq!(fix.kind, crate::abi::FixKind::Replace);
    assert_eq!(fix.replacement, "Green");

    // The nullary-EXPORTED case (already reached by the lowering walk) still reports EXACTLY ONE — the
    // infer-side and emit-side copies anchor at the same key node and `dedup_faults` collapses them.
    let src_nullary = "(module m (type C Red Green) \
             (def (g) (match (C.Red) ((C.Red) 1) ((C.Gren) 2))) (export g))";
    let dn: Vec<_> = crate::diagnostics(&mut Db::load(parse(src_nullary)))
        .into_iter()
        .filter(|d| d.code.as_deref() == Some("CDZ0201"))
        .collect();
    assert_eq!(dn.len(), 1, "nullary body: one fault, not a double: {dn:?}");

    // A CORRECT variant pattern on a parameterized body stays clean — no false positive.
    let src_ok = "(module m (type C Red Green) \
             (def (g (: c C)) (match c ((C.Red) 1) ((C.Green) 2))) (export g))";
    assert!(
        crate::diagnostics(&mut Db::load(parse(src_ok)))
            .iter()
            .all(|d| d.code.as_deref() != Some("CDZ0201")),
        "a correct variant match produces no field fault"
    );
}

#[test]
fn a_binary_operator_over_or_under_application_on_a_function_param_surfaces_in_the_query() {
    // The binop-ARITY twin of the mistyped-variant case above. A fixed-arity operator applied to a
    // count other than 2 has a CLEAR operator-specific CDZ0201 "+ takes exactly 2 operands" (with a
    // delete-surplus fix on an over-application) — but it was produced ONLY by the emit-path lowering
    // walk (nullary-EXPORTED bodies). So in a PARAMETERIZED body, `cdz check` reported only the GENERIC
    // CDZ0203 "applied N arguments to a function of arity M …" for the over-application and NOTHING for
    // the under-application, while `compile` rejected both with the operator message. `collect`'s Apply
    // arm now surfaces the operator CDZ0201 whether the def takes parameters or not.

    // OVER-application `(+ n 1 2)` on a parameter: one CDZ0201 with the operator message + delete fix,
    // and the generic CDZ0203 is deduped away (reported exactly once, not twice).
    let over = "(module m (def (g (: n Int64)) (+ n 1 2)) (export g))";
    let d: Vec<_> = crate::diagnostics(&mut Db::load(parse(over)))
        .into_iter()
        .filter(|d| d.severity == crate::abi::Severity::Error)
        .collect();
    assert_eq!(
        d.len(),
        1,
        "one arity fault, generic CDZ0203 deduped: {d:?}"
    );
    assert_eq!(d[0].code.as_deref(), Some("CDZ0201"));
    assert!(
        d[0].message.contains("takes exactly 2 operands"),
        "the clear operator message, not the generic arity phrasing: {}",
        d[0].message
    );
    let fix = d[0].fix.as_ref().expect("carries the delete-surplus fix");
    assert_eq!(fix.kind, crate::abi::FixKind::Delete);

    // ONE-of-two `(+ n)` on a parameter now CURRIES (operator ruling: "operators should curry") — it is
    // the first-class partial `\b. n + b`, NOT an arity error, so `check` reports NO CDZ0201 arity fault
    // for it. (The former "takes exactly 2 operands" under-application report is retired for the 1-of-2
    // case; a ZERO-operand `(+)` and an OVER-application still fault — covered above/below.)
    let under = "(module m (def (g (: n Int64)) (+ n)) (export g))";
    let du: Vec<_> = crate::diagnostics(&mut Db::load(parse(under)))
        .into_iter()
        .filter(|d| {
            d.severity == crate::abi::Severity::Error && d.code.as_deref() == Some("CDZ0201")
        })
        .collect();
    assert!(
        du.is_empty(),
        "a 1-of-2 partial operator curries into a closure, so no arity fault: {du:?}"
    );

    // A comparison and the arithmetic operator over FLOAT operands take the same path (the message
    // names the operator). Float arithmetic reuses the ONE `+` — over-applying it (`(+ x 1.0 2.0)`,
    // `x : Float64`) is the same arity fault, named `+`, not a distinct `+.`.
    for (src, op) in [
        ("(module m (def (g (: n Int64)) (< n 1 2)) (export g))", "<"),
        (
            "(module m (def (g (: x Float64)) (+ x 1.0 2.0)) (export g))",
            "+",
        ),
    ] {
        let dc: Vec<_> = crate::diagnostics(&mut Db::load(parse(src)))
            .into_iter()
            .filter(|d| d.severity == crate::abi::Severity::Error)
            .collect();
        assert_eq!(dc.len(), 1, "{op}: one arity fault: {dc:?}");
        assert_eq!(dc[0].code.as_deref(), Some("CDZ0201"));
        assert!(
            dc[0]
                .message
                .contains(&format!("{op} takes exactly 2 operands")),
            "{op}: names the operator: {}",
            dc[0].message
        );
    }

    // A WELL-FORMED 2-operand application on a parameter stays clean — no false positive.
    let ok = "(module m (def (g (: n Int64)) (+ n 1)) (export g))";
    assert!(
        crate::diagnostics(&mut Db::load(parse(ok)))
            .iter()
            .all(|d| d.code.as_deref() != Some("CDZ0201")),
        "a correct 2-operand application produces no arity fault"
    );

    // A USER-function over-application keeps its OWN generic CDZ0203 (no operator CDZ0201 is minted for
    // a non-operator head) — the accessor is scoped to the fixed-arity binary operators.
    let userfn = "(module m (def (f (: x Int64)) x) (def (g (: n Int64)) (f n 1 2)) (export g))";
    let df: Vec<_> = crate::diagnostics(&mut Db::load(parse(userfn)))
        .into_iter()
        .filter(|d| d.severity == crate::abi::Severity::Error)
        .collect();
    assert_eq!(df.len(), 1, "user-fn over-app reports once: {df:?}");
    assert_eq!(df[0].code.as_deref(), Some("CDZ0203"), "{}", df[0].message);
}

#[test]
fn an_unbound_name_anchors_to_a_user_node() {
    // The diagnostic for an unbound name carries a node index, and it is a genuine USER node (below
    // the program's node count) — the front-end can map it to the `nope` occurrence.
    let d = first_error("(module m (def (main) nope) (export main))");
    assert_eq!(d.code.as_deref(), Some("CDZ0101"));
    let node = d.node.expect("unbound-name diagnostic must carry a node");
    // It resolves to a real user node — the same identity the span table is keyed by.
    let ast = parse("(module m (def (main) nope) (export main))");
    let db = Db::load(ast);
    assert!(
        db.is_user_node(StructId(node)),
        "node {node} must be a user node"
    );
}

#[test]
fn a_wrong_type_arg_to_a_user_function_anchors_to_the_call_site() {
    // Calling a user function with a wrong-type argument — `(helper true)` where `helper`'s body is
    // `(+ x 1)` — β-reduces to `(+ true 1)` on SYNTHESIZED nodes, whose CDZ0203 once reported with
    // NO node (an unanchored `cdz:`/`file:` prefix, no line:col). The reduced-body fault is now
    // re-anchored to the CALL SITE when it landed on a non-user node, so the error carries a real
    // user node the front-end maps to `file:line:col`.
    let src = "(module m (def (helper x) (+ x 1)) (def (main) (helper true)) (export main))";
    let d = first_error(src);
    assert_eq!(d.code.as_deref(), Some("CDZ0203"));
    let node = d
        .node
        .expect("the mismatch must carry a node, not be unanchored");
    let db = Db::load(parse(src));
    assert!(
        db.is_user_node(StructId(node)),
        "node {node} must be a user node so it maps to a source location"
    );
}

#[test]
fn a_fault_inside_an_argument_keeps_its_own_precise_anchor() {
    // The call-site re-anchor fires ONLY when the reduced-body fault landed on a synthesized node. A
    // fault genuinely inside an ARGUMENT sub-expression — `(id (+ 1 true))` — is on a real user node,
    // so it keeps its OWN anchor and never regresses to unanchored.
    let src = "(module m (def (id x) x) (def (main) (id (+ 1 true))) (export main))";
    let d = first_error(src);
    assert_eq!(d.code.as_deref(), Some("CDZ0203"));
    let node = d.node.expect("the mismatch carries a node");
    let db = Db::load(parse(src));
    assert!(
        db.is_user_node(StructId(node)),
        "node {node} must be a real user node the front-end can map"
    );
}

#[test]
fn a_provable_overflow_does_not_leak_a_synthesized_node() {
    // `(+ Int64.max 1)` proves an overflow (CDZ0304). The fold runs over evaluator-SYNTHESIZED
    // nodes (the built `Int64` module / reduced operands), but the reported origin must be either a
    // real user node or unanchored — NEVER a synthesized/prelude id that would mis-map. This is the
    // boundary invariant the operator flagged.
    let d = first_error("(module m (def (main) (+ (. Int64 max) 1)) (export main))");
    assert_eq!(d.code.as_deref(), Some("CDZ0304"));
    if let Some(node) = d.node {
        let ast = parse("(module m (def (main) (+ (. Int64 max) 1)) (export main))");
        let db = Db::load(ast);
        assert!(
            db.is_user_node(StructId(node)),
            "reported node {node} must be a user node, not a prelude/synthesized id"
        );
    }
    // (An unanchored `None` is acceptable — the fault came from synthesized nodes with no source.)
}

#[test]
fn a_type_mismatch_anchors_within_the_program() {
    // An `if` with a non-Bool condition (CDZ0203) anchors to a user node in the program.
    let d = first_error("(module m (def (main) (if 5 1 2)) (export main))");
    assert_eq!(d.code.as_deref(), Some("CDZ0203"));
    if let Some(node) = d.node {
        let ast = parse("(module m (def (main) (if 5 1 2)) (export main))");
        let db = Db::load(ast);
        assert!(
            db.is_user_node(StructId(node)),
            "node {node} must be a user node"
        );
    }
}

// ── Dead-trap warning (CDZ0305) — a non-error diagnostic riding alongside a produced artifact.
// A computation that PROVABLY traps but whose value is unobserved (an unprojected element, an
// unreferenced binding, an unused argument) is eliminated (`core-semantics.md` §A Trap Occurs Only
// Where Its Computation Is Observed) — conformant, so the build SUCCEEDS, but a warning is emitted.

/// Compile a program and return its warning diagnostics (severity `Warning`) — asserting the
/// component WAS produced (a warning must ride alongside a success, never a denial).
fn warnings_of(src: &str) -> Vec<crate::abi::Diagnostic> {
    let bytes = crate::codec::encode(&parse(src));
    // Through the SAME host-stack guard the bin uses (`host.rs`) — a dead NON-NORMALIZING binding
    // (`((fn (v0) (v0 v0)) (fn (v1) (v1 (v1 v1))))`) recurses deep during the fold before the reduction
    // work-budget declines, and would SIGABRT a default `cargo test` worker's ≈2 MB stack. Sizing the
    // stack from `DESCENT_DEPTH_LIMIT` lets the budget guard — not the native stack — bound it.
    let out = crate::host::run_with_compiler_stack(|| {
        compile(
            &[Artifact::new(Artifact::KIND_AST, "m", bytes)],
            &[Target::Wasm],
        )
    });
    assert!(
        out.artifact(Target::Wasm.artifact_kind()).is_some(),
        "a warning must accompany a PRODUCED component, but compilation failed: {:?}",
        out.diagnostics
    );
    out.diagnostics
        .into_iter()
        .filter(|d| d.severity == crate::abi::Severity::Warning)
        .collect()
}

#[test]
fn an_eliminated_provable_trap_warns_but_still_compiles() {
    // Each shape drops a computation that PROVABLY traps because its value is unobserved: an
    // unprojected tuple element, an unread record field, an unreferenced let binding, an argument
    // bound to an unused parameter. All compile (the value is not observed) AND warn CDZ0305.
    for src in [
        "(module m (def (main) (. (tuple 42 (/ 100 0)) 0)) (export main))",
        "(module m (def (main) (. (record (= a 42) (= b (/ 100 0))) a)) (export main))",
        "(module m (def (main) (let ((t (/ 100 0))) 5)) (export main))",
        "(module m (def (f x y) x) (def (main) (f 7 (/ 100 0))) (export main))",
    ] {
        // Exactly one DEAD-TRAP warning (CDZ0305). Some shapes also carry an unused-binding warning
        // (CDZ0306 — the dropped `let t`, the unused param `y`), which is a separate, correct signal;
        // filter to the dead-trap code so this test pins the dead-trap behavior specifically.
        let dead: Vec<_> = warnings_of(src)
            .into_iter()
            .filter(|d| d.code.as_deref() == Some("CDZ0305"))
            .collect();
        assert_eq!(
            dead.len(),
            1,
            "expected exactly one dead-trap (CDZ0305) warning for `{src}`, got {dead:?}"
        );
    }
    // The dead-trap warning fires for EVERY provably-trapping constant, not only integer ÷0 — pin the
    // full trap-kind axis `core-semantics.md §285` names, so a future change that breaks the fold's
    // trap-proof for one kind (e.g. modulo, the rational zero-denominator, or overflow) is caught. Each
    // is a 0-use `let` binding whose init provably traps at compile time; all compile (value unobserved)
    // AND warn CDZ0305 exactly once. (Integer ÷0 is covered by the position sweep above; these add %0,
    // the zero-denominator `Rational.of`, and a constant overflow past the Int64 default.)
    for trap_init in [
        "(% 100 0)",                 // integer modulo by zero (CDZ0304-class trap)
        "(Rational.of 1 0)",         // a rational with a zero denominator
        "(+ 9223372036854775807 1)", // a constant Int64 overflow (max + 1)
    ] {
        let src = format!("(module m (def (main) (let ((t {trap_init})) 5)) (export main))");
        let dead: Vec<_> = warnings_of(&src)
            .into_iter()
            .filter(|d| d.code.as_deref() == Some("CDZ0305"))
            .collect();
        assert_eq!(
            dead.len(),
            1,
            "a 0-use let binding whose init is `{trap_init}` must warn CDZ0305 (dead trap) exactly \
                 once and still compile, got {dead:?}"
        );
    }
}

#[test]
fn a_reachable_const_trap_warning_names_the_specific_trap_kind() {
    // CDZ0309 (potentially-reachable const trap) must say WHICH trap kind the demoted operation will
    // raise (operator 2026-08-27: "we want to say what kind of trap it is"). Each of these puts a
    // compile-provable trap of a distinct kind in a RUNTIME-guarded `if` else branch; the warning names
    // the kind (parenthesized) while keeping the "potentially reachable trap" wording the corpus grades on.
    for (trap_expr, kind) in [
        ("(/ 1 0)", "divide by zero"),
        (
            "(* 9223372036854775807 9223372036854775807)",
            "overflows Int64",
        ),
        ("(<< 1 100)", "out of range"),
    ] {
        let src =
            format!("(module m (def (main (: n Int64)) (if (> n 0) 7 {trap_expr})) (export main))");
        let warns: Vec<_> = warnings_of(&src)
            .into_iter()
            .filter(|d| d.code.as_deref() == Some("CDZ0309"))
            .collect();
        assert_eq!(
            warns.len(),
            1,
            "exactly one CDZ0309 for `{trap_expr}`, got {warns:?}"
        );
        let msg = &warns[0].message;
        assert!(
            msg.contains("potentially reachable trap"),
            "keeps the corpus-graded wording: {msg}"
        );
        assert!(
            msg.contains(kind),
            "names the specific trap kind `{kind}` for `{trap_expr}`: {msg}"
        );
    }
}

#[test]
fn an_unused_non_normalizing_let_init_warns_but_still_compiles() {
    // The dead-computation warning (CDZ0305) covers a computation with NO VALUE, not only a trapping
    // one: a non-normalizing self-application `((fn (v0) (v0 v0)) (fn (v1) (v1 (v1 v1))))` (no normal
    // form) bound to an UNUSED let-binder is dead code — the fold eliminates it, the program compiles
    // (`main => 0`), and a CDZ0305 warning flags the likely bug. This is DCE consistency: an
    // un-observed non-normalizing binding is elided exactly as an un-observed trap is (rather than
    // reducing dead code), while the SAME term USED is the hard CDZ0999 error.
    // `warnings_of` asserts the component WAS produced (dead code elided → it compiles) and returns
    // the warnings; exactly one CDZ0305 dead-computation warning (CDZ0306 unused-`y` rides alongside).
    let src = "(module m (def (main) (let ((y ((fn (v0) (v0 v0)) (fn (v1) (v1 (v1 v1)))))) 0)) (export main))";
    let dead: Vec<_> = warnings_of(src)
        .into_iter()
        .filter(|d| d.code.as_deref() == Some("CDZ0305"))
        .collect();
    assert_eq!(
        dead.len(),
        1,
        "expected one CDZ0305 warning for the dead non-normalizing binding, got {dead:?}"
    );
    assert!(
        dead[0].message.contains("does not reduce to a value"),
        "the warning names the non-normalizing reason: {}",
        dead[0].message
    );
    // The SAME term USED is a hard CDZ0999 error (the component is DENIED, not merely warned).
    let used = "(module m (def (main) (let ((y ((fn (v0) (v0 v0)) (fn (v1) (v1 (v1 v1)))))) y)) (export main))";
    // Same host-stack guard as `warnings_of` above — the USED non-normalizing term recurses just as
    // deep during the fold, so this direct `compile` must not run on the default test stack either.
    let out = crate::host::run_with_compiler_stack(|| {
        compile(
            &[Artifact::new(
                Artifact::KIND_AST,
                "m",
                crate::codec::encode(&parse(used)),
            )],
            &[Target::Wasm],
        )
    });
    assert!(
        out.artifact(Target::Wasm.artifact_kind()).is_none()
            && out
                .diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some("CDZ0999")),
        "a USED non-normalizing binding is rejected CDZ0999, not just warned: {:?}",
        out.diagnostics
    );
    // NO FALSE POSITIVE: a NORMAL unused let-init gets no dead-computation warning.
    let normal = "(module m (def (main) (let ((y (+ 1 2))) 0)) (export main))";
    assert!(
        warnings_of(normal)
            .into_iter()
            .all(|d| d.code.as_deref() != Some("CDZ0305")),
        "a normal unused let-init must not get a dead-computation warning"
    );
}

#[test]
fn a_dead_trap_warning_anchors_to_a_user_node() {
    // The warning carries a node index that resolves to a real user node — the front-end maps it to
    // the trapping computation's span, never a prelude/synthesized id.
    let src = "(module m (def (main) (. (tuple 42 (/ 100 0)) 0)) (export main))";
    let ws = warnings_of(src);
    assert_eq!(ws.len(), 1);
    let node = ws[0].node.expect("a dead-trap warning must carry a node");
    let db = Db::load(parse(src));
    assert!(
        db.is_user_node(StructId(node)),
        "node {node} must be a user node"
    );
}

// (an_observed_provable_trap_is_an_error_not_a_warning migrated to corpus: the provable-trap →
//  CDZ0304 compile-deny rule is covered by 28-compiler-primitives "(error CDZ0304 (message
//  \"division by zero\"))"; the observed-tuple-element instance exercised no distinct path.)

#[test]
fn a_clean_program_and_an_unprovable_trap_do_not_warn() {
    // A clean projection warns not at all; a RUNTIME (unprovable) trap in a dropped position is not
    // flagged either — the warning fires only on a PROVABLE trap, so no false positive on code the
    // compiler cannot prove traps.
    assert!(warnings_of("(module m (def (main) (. (tuple 42 7) 0)) (export main))").is_empty());
    assert!(
        warnings_of("(module m (def (main (: x Int64)) (. (tuple 42 (/ 100 x)) 0)) (export main))")
            .is_empty(),
        "a runtime (unprovable) trap in a dropped position must NOT warn"
    );
}

/// The CDZ0306 unused-binding warning messages from `src`. Uses `diagnostics()` directly (the
/// export-independent fault+warning set `cdz check` drives) rather than `warnings_of` — a program
/// exporting a parameterized function does not emit a component, but its diagnostics are still
/// defined (that is the point of the export-independent query).
fn unused_of(src: &str) -> Vec<String> {
    let mut db = Db::load(parse(src));
    crate::diagnostics(&mut db)
        .into_iter()
        .filter(|d| d.code.as_deref() == Some("CDZ0306"))
        .map(|d| d.message)
        .collect()
}

/// The CDZ0306 unused-binding DIAGNOSTICS (full records, so a test can read the carried fix).
fn unused_diags(src: &str) -> Vec<crate::abi::Diagnostic> {
    let mut db = Db::load(parse(src));
    crate::diagnostics(&mut db)
        .into_iter()
        .filter(|d| d.code.as_deref() == Some("CDZ0306"))
        .collect()
}

/// EVERY diagnostic (faults + warnings) the `cdz check`/LSP path reports — so a test can assert on the
/// FULL set (e.g. that a consequent warning is NOT emitted alongside a primary fault).
fn diags_of(src: &str) -> Vec<crate::abi::Diagnostic> {
    let mut db = Db::load(parse(src));
    crate::diagnostics(&mut db)
}

/// A RECORD sub-pattern NESTED inside a tuple/list/constructor match pattern whose field value is itself
/// a further COMPOUND — `(tuple (record (x (tuple a b))) c)` — is not yet wired (a record field projects
/// by NAME and there is no name-keyed `PathStep` to compose a FURTHER descent below the field). It must
/// BIND — NOT leak a misleading CDZ0101 "unbound name `a`" AND NOT decline CDZ0900 (§235: a record field
/// value binder may be any nested pattern to any depth). A binder BELOW a nested-record field value in a
/// tuple match arm — `(tuple (record (x (tuple a b))) c)`, field `x`'s value the further `(tuple a b)` —
/// now WIRES via a `Resolved::RecordField` reading field `x` then descending `sub_path = [Elem(0/1)]` into
/// the tuple (a record field read lowers to `Elem(<sorted-slot>)`, so the descent is positional `Elem`
/// steps — no new `PathStep`). `cdz check` on a PARAMETERIZED body (via match_pattern_fault, not only the
/// emit walk) must be CLEAN. The BARE-binder cases (`(tuple (record (x a)) c)`) wire via an empty sub_path;
/// this is the DEEPER (compound-field-value) twin, formerly the CDZ0900 gap #6838/#6850.
#[test]
fn a_deeper_nested_record_match_field_binds_via_sub_path() {
    // Body references the deeper binder `a` below a nested-record field's own compound value. `f` is
    // parameterized (non-nullary), so this exercises the check≡compile path — it must CHECK CLEAN now.
    let src = "(module m (def (f (: t (Tuple (Record (x (Tuple Int64 Int64))) Int64))) \
               (match t ((tuple (record (x (tuple a b))) c) (+ (+ a b) c)))) (export f))";
    let all = diags_of(src);
    assert!(
        all.iter().all(|d| d.code.as_deref() != Some("CDZ0101")),
        "no misleading 'unbound name' for a deeper nested-record binder: {all:?}"
    );
    assert!(
        all.iter().all(|d| !d
            .message
            .contains("record sub-pattern nested inside a tuple/list/constructor match pattern")),
        "the deeper nested-compound MATCH field binder now BINDS via the RecordField sub_path (§235) — \
         no CDZ0900 'not supported' decline: {all:?}"
    );
}

#[test]
fn two_bare_none_record_fields_do_not_cross_contaminate_via_a_let_bound_record() {
    // REGRESSION (v-agent-harness report): TWO bare `None()` fields in one LET-BOUND record literal used
    // to cross-contaminate their `Option` element vars. Each `None()` types as `Option(?0)` (the
    // nullary-variant scheme var, memoized per node), so both fields SHARED `?0`; when the record's type
    // (built by the `Resolved::Record` field loop) unified against the expected param type in one
    // `Subst`, one field borrowed its sibling's element type — `resumes : Option Bytes` was inferred
    // `Option Outcome`, a spurious CDZ0203. FIX: each record field's free vars are freshened into a
    // DISJOINT block, so sibling `None()` fields solve independently against their own expected field
    // type. `apply` takes a record with two DIFFERENTLY-typed Option fields, both bound `None` via a
    // `let`, then applied — must type-check with NO CDZ0203 field-mismatch (the cross-contamination
    // fault). Asserts the diagnostics are clean rather than running it (a `Bytes`-bearing record needs
    // the value-heap runtime the lib-test linker doesn't stage).
    let src = "(module m \
           (type Outcome (Ok Int64) (Err Int64)) \
           (def (apply (: evt (Record (: a (Option Bytes)) (: b (Option Outcome)) (: c Int64)))) (. evt c)) \
           (def (main) (let ((evt (record (= a (None)) (= b (None)) (= c 9)))) (apply evt))) \
           (export main))";
    let all = diags_of(src);
    assert!(
        all.iter().all(|d| d.code.as_deref() != Some("CDZ0203")),
        "two bare None() record fields must not cross-contaminate (no CDZ0203 field mismatch): {all:?}"
    );
}

#[test]
fn two_bare_none_record_fields_passed_as_a_direct_arg_do_not_cross_contaminate() {
    // REGRESSION (residual of the let-bound fix above): the SAME two-bare-`None()` record, passed as a
    // DIRECT call argument instead of `let`-bound, took a DIFFERENT type-building path — the call's
    // synthesized `(: arg paramtype)` check reflects a compound-literal arg via `reflected_ty`, whose
    // RecordNew arm rebuilt field types WITHOUT the disjoint-freshening the `compound_ctor_type` /
    // `Resolved::Record` paths got. So both `None()` fields reflected `Option(?0)` sharing var 0 and
    // the record unify hit `Bytes` vs `Outcome` on the shared var — a spurious CDZ0203, even though the
    // let-bound twin type-checked. FIX: freshen each field/element into a disjoint block in
    // `reflected_ty`'s RecordNew + TupleNew arms too. Same shape as the let-bound test, arg inlined.
    let src = "(module m \
           (type Outcome (Ok Int64) (Err Int64)) \
           (def (apply (: evt (Record (: a (Option Bytes)) (: b (Option Outcome)) (: c Int64)))) (. evt c)) \
           (def (main) (apply (record (= a (None)) (= b (None)) (= c 9)))) \
           (export main))";
    let all = diags_of(src);
    assert!(
        all.iter().all(|d| d.code.as_deref() != Some("CDZ0203")),
        "two bare None() record fields passed as a direct arg must not cross-contaminate (no CDZ0203): {all:?}"
    );
}

// (a_wrong_typed_option_field_in_a_direct_record_arg_still_rejects migrated to corpus
//  07-type-system "a wrong-typed Option field in a direct record arg still rejects" — CDZ0203, backend-agnostic.)

#[test]
fn nested_and_sibling_bare_none_compounds_in_a_direct_arg_do_not_cross_contaminate() {
    // COVERAGE hardening for the direct-arg `reflected_ty` freshening: the fix freshens RECURSIVELY
    // (each RecordNew/TupleNew arm freshens its already-reflected element), so a bare `None()` nested
    // inside a compound-inside-a-compound must also solve independently of its siblings. The flat
    // regression above only exercises one record level; this locks the RECURSIVE path. A record arg
    // carries: a `(tuple (None) (None))` field (two Option siblings inside a nested TUPLE), a sibling
    // top-level `(None)` field of a THIRD Option type, and a plain field — every `None` must ground to
    // its own expected element type with NO CDZ0203 cross-contamination. Diagnostics-only (a
    // `Bytes`-bearing record needs the value-heap runtime the lib-test linker doesn't stage).
    let src = "(module m \
           (type Outcome (Ok Int64) (Err Int64)) \
           (def (apply (: e (Record (: pair (Tuple (Option Bytes) (Option Outcome))) (: d (Option Int64)) (: c Int64)))) (. e c)) \
           (def (main) (apply (record (= pair (tuple (None) (None))) (= d (None)) (= c 9)))) \
           (export main))";
    let all = diags_of(src);
    assert!(
        all.iter().all(|d| d.code.as_deref() != Some("CDZ0203")),
        "nested + sibling bare None() compounds in a direct arg must not cross-contaminate (no CDZ0203): {all:?}"
    );
}

#[test]
fn a_def_named_quote_binds_its_parameter_and_is_not_hijacked_by_reification() {
    // `quote`/`quasiquote` are grammar heads recognized STRUCTURALLY only when they head an
    // EXPRESSION — like `if`/`match`/`bin`, all freely definable as ordinary function names because
    // a definition's SIGNATURE is never resolved as an expression. Quote reification, however, is a
    // shape-driven PRE-PASS over every `(quote …)`/`(quasiquote …)` node, and it wrongly rewrote the
    // def signature `(quote x)` into `(Ast.Name "x")`, ERASING the parameter binder — the body's `x`
    // then resolved CDZ0101 "unbound name". `quote::binder_position_nodes` now excludes a
    // def-signature / fn-params list from reification, so a user function named `quote` scans as
    // ordinary and its parameter binds. No diagnostic at all (no CDZ0101, no CDZ0306-unused).
    for name in ["quote", "quasiquote"] {
        let src = format!("(module m (def ({name} (: x Int64)) (+ x 2)) (export {name}))");
        let diags = diags_of(&src);
        assert!(
            diags.is_empty(),
            "a def named `{name}` must bind its parameter (no CDZ0101/CDZ0306): {:?}",
            diags
                .iter()
                .map(|d| (d.code.clone(), d.message.clone()))
                .collect::<Vec<_>>()
        );
    }
    // Regression guard on the OTHER direction: a genuine `(quote …)` in EXPRESSION position still
    // reifies to an `Ast` value (it is NOT left as a bare quote/decline). `(quote 1)` == `(Ast.Int 1)`.
    let genuine = "(module m (def (main) (= (quote 1) (Ast.Int 1))) (export main))";
    assert!(
        diags_of(genuine)
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a genuine quote expression must still reify: {:?}",
        diags_of(genuine)
    );
}

#[test]
fn a_recursive_functions_used_parameter_is_not_flagged_unused() {
    // A RECURSIVE function freshens its parameter binder when its body is resolved (and the
    // accumulator transform may rewrite the body entirely), so a reference resolves to a
    // SYNTHESIZED param copy, not the original occurrence. The usage check keys on the resolution
    // KIND (does a body reference resolve to a `Param`?), not the target occ, so it is not fooled:
    // `n` here is used three times → it must NOT warn (the reported false positive).
    let src = "(module m (def (sm n) (if (= n 0) 0 (+ n (sm (- n 1))))) (export sm))";
    assert!(
        unused_of(src).is_empty(),
        "a used recursive param must not warn: {:?}",
        unused_of(src)
    );
    // A genuinely-unused parameter still warns (the check is real, not just disabled for
    // functions): `z` is never referenced.
    let with_unused = "(module m (def (f p z) (+ p 1)) (export f))";
    let u = unused_of(with_unused);
    assert_eq!(u.len(), 1, "the truly-unused param z warns: {u:?}");
    assert!(u[0].contains("`z`"), "{u:?}");
}

/// A MATCH-ARM pattern binder its arm body never references is unused — the match-arm analogue of an
/// unused `let` binding / parameter, warned CDZ0306 with a `_`-prefix fix. A reference to a match
/// binder resolves to a `SumPayload`/scrutinee-`Ref` (not the binder's own occ), so the `used`-occ set
/// misses it; a scope-correct NAME check (`used_match_binder_names`) decides usage. Shadowing is
/// honored (the arm binder resolution wins over an outer same-named param).
#[test]
fn an_unused_match_arm_binder_warns_with_an_underscore_fix() {
    // A variant-payload binder never used in its arm → CDZ0306 + a `_x` fix.
    let d = unused_diags(
        "(module m (def (main) (match (Some 5) ((Some x) 0) ((None) 1))) (export main))",
    );
    assert_eq!(d.len(), 1, "the unused variant binder x warns: {d:?}");
    assert!(d[0].message.contains("`x`"), "{d:?}");
    assert_eq!(
        d[0].fix.as_ref().map(|f| f.replacement.as_str()),
        Some("_x"),
        "carries the `_`-prefix fix: {d:?}"
    );
    // A tuple-pattern binder unused (b), the other (a) used → only b warns.
    let tup = unused_of(
        "(module m (def (g (: t (Tuple Int64 Int64))) (match t ((tuple a b) a))) (export g))",
    );
    assert_eq!(
        tup.len(),
        1,
        "only the unused tuple binder b warns: {tup:?}"
    );
    assert!(tup[0].contains("`b`"), "{tup:?}");
    // NO false positive: a USED binder (bare, nested, tuple), a `_`-prefixed binder, and a binder used
    // in a nested payload are all clean.
    for ok in [
        "(module m (def (main) (match (Some 5) ((Some x) x) ((None) 1))) (export main))",
        "(module m (def (main) (match (Some 5) ((Some _x) 0) ((None) 1))) (export main))",
        "(module m (def (g (: t (Tuple Int64 Int64))) (match t ((tuple a b) (+ a b)))) (export g))",
        "(module m (def (g (: o (Option (Option Int64)))) (match o ((Some (Some y)) y) (_ 0))) (export g))",
        // A GUARDED binder used only in the guard COND (not the body) is used — the usage scan must
        // cover the cond subtree, not just the arm body. (Regression: the resolution-kind heuristic
        // mis-classified the cond's `Ref`-to-scrutinee occurrence and false-flagged this binder.)
        "(module m (def (f (: n Int64)) (match n ((guard x (> x 0)) 5) (_ 0))) (export f))",
        // A guarded scalar binder used in BOTH cond and body.
        "(module m (def (f (: n Int64)) (match n ((guard x (> x 0)) x) (_ 0))) (export f))",
    ] {
        assert!(
            unused_of(ok).is_empty(),
            "a used/underscored match binder must not warn: {ok} -> {:?}",
            unused_of(ok)
        );
    }
    // A genuinely-unused GUARDED binder (bound but referenced in NEITHER the cond nor the body) still
    // warns — the guard-cond scan widens usage, it does not blanket-suppress. Here the cond tests `n`,
    // the body is a constant, so `x` is dead.
    let dead_guard = unused_of(
        "(module m (def (f (: n Int64)) (match n ((guard x (> n 0)) 5) (_ 0))) (export f))",
    );
    assert_eq!(
        dead_guard.len(),
        1,
        "a guard binder used in neither cond nor body still warns: {dead_guard:?}"
    );
    assert!(dead_guard[0].contains("`x`"), "{dead_guard:?}");
}

#[test]
fn a_bin_pattern_byte_order_modifier_is_not_an_unused_match_binding() {
    // adv-59 (breaker): the unused-binding lint walked a `(bin <seg>…)` pattern's raw AST and collected
    // a segment's TRAILING atoms as match binders — so the byte-order MODIFIER `le` in `(bin (u16 n le))`
    // false-flagged `warning [CDZ0306] unused match binding: le`. Worse, the suggested `_le` fix
    // HARD-ERRORS (the segment stops parsing as a modifier → CDZ0201 "the only integer bin-segment
    // modifier is `le`" + CDZ0101 unbound `n`), so following the lint BREAKS a working program — and it
    // fired on the corpus's own pinned `le` idiom (16-binary-matching:165). A corpus pin can't guard
    // this (the gate ignores warnings), so this lint unit test is the guard. FIX: `arm_pattern_binders`
    // descends only a bin segment's SLOT (`seg[1]`), never the kind head / modifier / width / size atoms.
    // `le` must NOT warn; the used binder `n` must NOT warn.
    assert!(
        unused_of(
            "(module m (def (f (: b Bytes)) (match b ((bin (u16 n le)) n) (_ -1))) (export f))"
        )
        .is_empty(),
        "a bin-pattern `le` modifier + a used slot binder must not warn CDZ0306: {:?}",
        unused_of(
            "(module m (def (f (: b Bytes)) (match b ((bin (u16 n le)) n) (_ -1))) (export f))"
        )
    );
    // Every int-segment face with `le` is clean (u16/u32/i16 all parse `le` as a modifier, not a binder).
    for src in [
        "(module m (def (f (: b Bytes)) (match b ((bin (u32 n le)) n) (_ -1))) (export f))",
        "(module m (def (f (: b Bytes)) (match b ((bin (i16 n le)) n) (_ -1))) (export f))",
    ] {
        assert!(
            unused_of(src).is_empty(),
            "no int-segment `le` face may warn CDZ0306: {src} -> {:?}",
            unused_of(src)
        );
    }
    // NOT over-suppressed: a bin SLOT binder that is genuinely UNUSED (body ignores it) STILL warns —
    // only the modifier is excluded, not the slot. Here `n` is bound (with `le`) but the body returns 0.
    let dead = unused_of(
        "(module m (def (f (: b Bytes)) (match b ((bin (u16 n le)) 0) (_ -1))) (export f))",
    );
    assert_eq!(
        dead.len(),
        1,
        "an unused bin slot binder still warns: {dead:?}"
    );
    assert!(
        dead[0].contains("`n`"),
        "the warning is on the slot binder n, not le: {dead:?}"
    );
    // A `bytes` segment's dependent-SIZE reference (`len`) is not a new binder either; the slot `body`
    // is used, so this arm is clean (no false CDZ0306 on `len`).
    assert!(
            unused_of("(module m (def (f (: b Bytes)) (match b ((bin (u8 len) (bytes body len)) (Bytes.len body)) (_ -1))) (export f))")
                .is_empty(),
            "a bytes-segment dependent-size ref must not warn, and a used slot is clean: {:?}",
            unused_of("(module m (def (f (: b Bytes)) (match b ((bin (u8 len) (bytes body len)) (Bytes.len body)) (_ -1))) (export f))")
        );
}

#[test]
fn a_bin_segment_size_operand_name_is_counted_used_not_flagged_cdz0306() {
    // REGRESSION (v-lsp: red squiggles in the guide binary-matching chapter): a name bound by an int
    // segment `(u8 n)` and used as the dependent SIZE of a later segment `(bytes body n)` was NOT
    // counted as used — CDZ0306 false-flagged `n` "never used". The size use lives IN THE PATTERN (not
    // the arm body), so `used_match_binder_names(body)` missed it; and the syntactic binder walk even
    // collected the size `n` as a bogus SECOND binder. Fix: `bin_pattern_size_occs` marks each segment
    // size operand a use + excludes it from the binder candidates. `n` used as `(bytes body n)`'s size
    // → NO CDZ0306 on `n`.
    let u = unused_of(
        "(module m (def (main) (match (Bytes.of (list 2 10 20)) \
               ((bin (u8 n) (bytes body n)) (Bytes.len body)) (_ 0))) (export main))",
    );
    assert!(
        !u.iter().any(|m| m.contains("`n`")),
        "a bin-segment SIZE operand `n` is a use of the earlier binder, not an unused binding: {u:?}"
    );
    // NO false positive AT ALL here: `body` is read (Bytes.len body) and `n` is the size use.
    assert!(
        u.is_empty(),
        "both bin binders are used (body read, n as size) — no CDZ0306: {u:?}"
    );
    // The utf8 dependent-size operand is likewise a use (not flagged); `k` sizes the utf8 segment.
    let uk = unused_of(
        "(module m (def (main) (match (Bytes.of (list 2 104 105)) \
               ((bin (u8 k) (utf8 s k)) (Bytes.len (Bytes.of (list 1)))) (_ 0))) (export main))",
    );
    assert!(
        !uk.iter().any(|m| m.contains("`k`")),
        "a utf8 dependent-size operand `k` is a use, not unused: {uk:?}"
    );
    assert!(
        uk.iter().any(|m| m.contains("`s`")),
        "a genuinely-unused segment binder `s` STILL warns (no over-suppression): {uk:?}"
    );
    // CONTROL — over-suppression guard: two genuinely-unused int-segment binders (neither used as a
    // body ref nor a size operand) BOTH still warn.
    let unused = unused_of(
        "(module m (def (main) (match (Bytes.of (list 10 20)) \
               ((bin (u8 x) (u8 y)) 5) (_ 0))) (export main))",
    );
    assert!(
        unused.iter().any(|m| m.contains("`x`")) && unused.iter().any(|m| m.contains("`y`")),
        "genuinely-unused segment binders still warn: {unused:?}"
    );
}

/// A `.`-MEMBER pattern `C.R` (a NULLARY variant used as a whole arm pattern, printed `(. C R)`) binds
/// NOTHING — its segments are the TYPE and VARIANT names, not binders. The unused-binding pass must not
/// treat the first segment (`C`) as a spuriously-unused binder. (Regression: `collect_arm_binder_leaves`
/// recursed a compound's args generically, collecting `C` from `(. C R)` → a bogus CDZ0306 "unused
/// binding `C`" with a nonsense `_C` rename on every nullary-variant match arm.)
#[test]
fn a_dotted_nullary_variant_arm_pattern_binds_nothing_and_never_warns_unused() {
    // A total match over a three-nullary-variant sum, each arm a bare `C.x` member — no binders, so
    // NO CDZ0306 at all.
    let clean =
        "(module m (type C R G B) (def (f (: c C)) (match c (C.R 1) (C.G 2) (C.B 3))) (export f))";
    assert!(
        unused_of(clean).is_empty(),
        "a dotted nullary-variant arm binds nothing — no spurious `C` warning: {:?}",
        unused_of(clean)
    );
    // The suppression is SURGICAL — it silences only the member HEAD, not real binders. A genuinely
    // unused payload binder alongside a dotted arm still warns (only `n`, never `C`).
    let mixed = "(module m (type C R G B) (def (f (: c C) (: o (Option Int64))) (match o ((Some n) (match c (C.R 1) (C.G 2) (C.B 3))) ((None) 0))) (export f))";
    let u = unused_of(mixed);
    assert_eq!(u.len(), 1, "only the unused payload binder n warns: {u:?}");
    assert!(u[0].contains("`n`"), "warns n, not C: {u:?}");
}

/// A BARE (un-dotted) nullary-variant arm pattern (`TInt` in `(match a (TInt …) (TBool …))`) is the
/// CONSTRUCTOR, not a binder — even when the match is NESTED inside another match's arm. `match_arm_binds`
/// treated a bare-name arm pattern as introducing a binding of that name; scope resolution is
/// scope-FIRST, so a NESTED `(match b (TInt 1) (TBool 2))` under an outer `(TInt …)` arm had its inner
/// `TInt` resolve to the "binder" the outer arm appeared to introduce instead of to the variant ctor.
/// That mis-resolution drew spurious CDZ0306 "unused binding `TInt`" + CDZ0213 "unreachable arm" on the
/// inner arms — though the program compiled and ran correctly (the runtime value was the ctor's). A bare
/// variant name never binds, so `match_arm_binds` now returns `None` for one (via `variant_ctor_by_name`,
/// the same index `resolve_name`'s bare-variant step consults), at every nesting depth.
#[test]
fn a_bare_nested_nullary_variant_arm_is_a_ctor_not_a_binder_and_never_warns() {
    // The queue repro (mlrepro-false-warning-nested-nullary-match-unused-binding): the INNER bare
    // nullary match drew CDZ0306 + CDZ0213. Now CLEAN — no warnings, exit 0.
    let nested = "(module m (type Ty TInt TBool) \
            (def (classify (: a Ty) (: b Ty)) \
              (match a (TInt (match b (TInt 1) (TBool 2))) (TBool 3))) \
            (export classify))";
    let all = diags_of(nested);
    assert!(
        all.iter().all(|d| d.code.as_deref() != Some("CDZ0306")),
        "no spurious 'unused binding' on a bare nested nullary-variant arm: {all:?}"
    );
    assert!(
        all.iter().all(|d| d.code.as_deref() != Some("CDZ0213")),
        "no spurious 'unreachable arm' on a bare nested nullary-variant arm: {all:?}"
    );
    // BEHAVIOR PRESERVED: the inner `TInt` is a CTOR, so `classify(TInt, TBool)` selects the `TBool => 2`
    // arm (a catch-all binder would have matched `TBool` at the FIRST inner arm and returned 1). The
    // dispatch VALUE (=2) is ordinary nested nullary-variant match, covered by the corpus; here we only
    // need the program to COMPILE (the warning regression must not block it).
    let prog = "(module m (type Ty TInt TBool) \
            (def (classify (: a Ty) (: b Ty)) \
              (match a (TInt (match b (TInt 1) (TBool 2))) (TBool 3))) \
            (def (main) (classify TInt TBool)) (export main))";
    crate::compile::compile_component(&crate::codec::encode(&parse(prog)))
        .expect("the nested nullary-variant program compiles");
    // The SAME inner match at TOP LEVEL was always clean — pin it stays clean (no regression the other way).
    let top = "(module m (type Ty TInt TBool) \
            (def (classify2 (: b Ty)) (match b (TInt 1) (TBool 2))) (export classify2))";
    assert!(
        diags_of(top)
            .iter()
            .all(|d| d.code.as_deref() != Some("CDZ0306") && d.code.as_deref() != Some("CDZ0213")),
        "a top-level bare nullary-variant match stays clean: {:?}",
        diags_of(top)
    );
}

/// A match whose PATTERN is malformed (rejected — a `(tuple a b c)` against a 2-tuple, a `(list … .. r
/// b)` with a binder after the rest) must NOT also emit consequent CDZ0306 "unused binding" warnings
/// for the binders inside that rejected pattern: those binders never bind, so their "unusedness" is a
/// CONSEQUENCE of the pattern fault, not an INDEPENDENT problem. This was a check≡compile discrepancy —
/// `cdz compile` bails at the first fault set (only the CDZ0201 shows), but `cdz check`/`diagnostics()`
/// collects faults AND warnings, and without the poison guard it appended spurious CDZ0306s. The
/// match-arm binder pass now skips a match whose `core_of` POISONS (the same lowering the CDZ0201 comes
/// from — they can never disagree).
#[test]
fn a_well_formed_pattern_still_warns_its_genuinely_unused_binders() {
    // The MALFORMED-pattern-no-consequent-CDZ0306 halves (a too-wide `#tuple(a b c)` on a 2-tuple, and a
    // malformed list-rest `#list(a .. rest b)` — each CDZ0201 with NO consequent "unused binding" warning
    // for the dead binders) migrated to corpus 05-compound-types via the program-scoped `(no-diagnostic
    // "unused binding")` lever (#6765). What STAYS here (needs a COMPILING program that emits the CDZ0306
    // warning — a `(Tuple)`/`(List)` entry-param would decline at the export boundary, so these are white-box
    // warning-count controls): NOT-over-suppressed — a well-formed pattern still warns its genuinely-unused
    // binders (the suppression is scoped to a poisoned pattern, not the whole binder pass).
    // NOT over-suppressed: a WELL-FORMED pattern whose body TRAPS (a poison in the BODY, not the
    // pattern — the match's `core_of` is NOT a poison) still warns its genuinely-unused binders.
    let trap_body = "(module m (def (f (: t (Tuple Int64 Int64))) (match t ((tuple a b) (trap \"x\")))) (export f))";
    let u = unused_of(trap_body);
    assert_eq!(
        u.len(),
        2,
        "a well-formed pattern with a trapping body still warns its 2 unused binders: {u:?}"
    );
    // NOT over-suppressed: an ordinary well-formed match still warns a genuinely-unused binder.
    assert_eq!(
        unused_of(
            "(module m (def (f (: o (Option Int64))) (match o ((Some x) 0) ((None) 1))) (export f))"
        )
        .len(),
        1,
        "a well-formed match's unused binder still warns"
    );
}

/// A LIST-scrutinee match arm whose head is a NAME used as a pattern constructor — `(Zorp x)` (unbound)
/// or `(List.Cons h t)` (`List` has no such member) — is a CODED fault surfaced by `cdz check` in EVERY
/// body, including a PARAMETERIZED / self-RECURSIVE one. A list scrutinee has no user constructors, so
/// such a head can never be a valid list pattern; it used to reach `lower_match_list`'s generic UNCODED
/// "a list match arm that is not an element pattern or a binder is not yet supported" decline. That
/// uncoded decline is produced ONLY by the emit-path lowering walk (`collect_reached_poisons`), which
/// runs on nullary-EXPORTED bodies alone (a recursive function is emitted once, never inlined into an
/// exported body's reduction), so `cdz check` was SILENT (exit 0) while `cdz compile` declined — a
/// check≡compile discrepancy, and the very "did you mean?" message hidden from the fast path. The list
/// matcher now propagates the head's OWN coded poison (CDZ0101 unbound / CDZ0201 non-member) — the LIST
/// twin of the sum matcher's `variant_disc_of`-miss path — so `type_errors`' `match_pattern_fault`
/// accessor surfaces it. `diags_of` runs the exact `crate::diagnostics` (check) path.
#[test]
fn a_well_formed_recursive_list_fold_checks_clean_not_a_false_arm_head_fault() {
    // WHITE-BOX residual. The reject halves — an UNBOUND arm head (`Zorp`) → CDZ0101 and a NON-MEMBER
    // intrinsic-module arm head (`(. List Nil)`/`(. List Cons)`) → CDZ0201, both surfaced by `check` even
    // over a self-recursive body — migrated to corpus 05-compound-types ("an unbound match arm head over a
    // recursive body …" + "a non-member intrinsic-module arm head …"). What stays here (corpus-inexpressible:
    // a `(List …)` entry param has no scalar boundary → declines at the export path, not a clean value case):
    // the no-false-alarm control that a WELL-FORMED recursive list fold (element + rest binder patterns)
    // stays clean — this path fires only on a ctor-shaped head, never a legitimate `(list …)` pattern or binder.
    let ok = "(module m (def (go (: acc Int64) (: rest (List Int64))) \
                  (match rest ((list) acc) ((list h .. t) (go (+ acc h) t)))) (export go))";
    // Bind once — `diags_of` recompiles the module, so evaluating it in both the predicate and the
    // failure message would double the compile cost (PR #1167 review).
    let ok_diags = diags_of(ok);
    assert!(
        ok_diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a well-formed recursive list fold still checks clean: {ok_diags:?}"
    );
}

/// The MAP + SCALAR-path twins of the list gap above: a match arm whose head is a name-as-constructor
/// over a MAP scrutinee, or over a scrutinee that ROUTED TO THE SCALAR PATH (a Map/List with no
/// structural `(map …)`/`(list …)` arm falls through there), was an UNCODED lowering-only decline that
/// `cdz check` missed on a parameterized / recursive body. Both now propagate the arm head's own coded
/// poison (CDZ0101 unbound / CDZ0201 non-member) so `match_pattern_fault` surfaces it in check —
/// completing the coverage across all three matcher paths (list, map, scalar-fallthrough).
#[test]
fn a_bogus_map_or_scalar_path_arm_head_over_a_recursive_body_is_a_coded_fault_in_check() {
    // MAP matcher: a real `(map …)` arm routes to `lower_match_map`, and the sibling `(Zorp …)` arm
    // head — unbound — is reported by check (was silent). `go` is self-recursive.
    let map_arm = "(module m (def (go (: mp (Map Int64 Int64))) \
                       (match mp ((map (1 v)) v) ((Zorp x) (go mp)) (_ 0))) (export go))";
    let all = diags_of(map_arm);
    assert!(
        all.iter()
            .any(|d| d.code.as_deref() == Some("CDZ0101") && d.message.contains("Zorp")),
        "check reports the unbound map-arm head as CDZ0101 (was silent): {all:?}"
    );
    // SCALAR-FALLTHROUGH: a Map scrutinee with NO structural `(map …)` arm routes to the scalar path;
    // the `(Zorp x)` compound head there is likewise surfaced (was the scalar path's uncoded
    // "not a scalar literal or `_`" decline).
    let scalar_path = "(module m (def (go (: mp (Map Int64 Int64))) \
                          (match mp ((Zorp x) (go mp)) (_ 0))) (export go))";
    let all = diags_of(scalar_path);
    assert!(
        all.iter()
            .any(|d| d.code.as_deref() == Some("CDZ0101") && d.message.contains("Zorp")),
        "check reports the scalar-path compound arm head as CDZ0101 (was silent): {all:?}"
    );
    // NO false alarm: a WELL-FORMED runtime map match (a `(map …)` arm + catch-all) stays clean — the
    // coded-head propagation fires only on an unbound/non-member ctor head, never a real pattern.
    let ok = "(module m (def (go (: mp (Map Int64 Int64))) \
                  (match mp ((map (1 v)) v) (_ 0))) (export go))";
    // Bind once — `diags_of` recompiles the module (PR #1167 review).
    let ok_diags = diags_of(ok);
    assert!(
        ok_diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a well-formed runtime map match still checks clean: {ok_diags:?}"
    );
}

/// A SET is matched by ELEMENT-MEMBERSHIP patterns (`lower_match_set`, the keys-only twin of
/// `lower_match_map`). The single-`..` REST form `#set(e… .. rest)` now LOWERS (slice 2 — `rest` resolves
/// to `Resolved::SetRest` via `binder_in` Case 6set-rest, typed `(Set E)`, and the desugar binds it to a
/// `Set.remove` chain); behavior lives in corpus 05. Only the MALFORMED two-`..` `#set(1 .. r1 .. r2)`
/// stays a coded, check-surfaced rest-SHAPE CDZ0201 — a white-box pin here (a two-`..` surface does not
/// ML-round-trip, same as the #map two-`..` case, so it can't be a corpus input). `check` ≡ `compile`.
#[test]
fn a_set_rest_pattern_lowers_single_dotdot_and_rejects_malformed_two_dotdot() {
    // The TWO-`..` `#set(1 .. r1 .. r2)` form is a MALFORMED rest (`..` not followed by exactly one binder)
    // → the coded, check-surfaced rest-SHAPE CDZ0201 (the set twin of the list/map rest-shape message).
    let pat = "#set(1 .. r1 .. r2)";
    let src =
        format!("(module m (def (f (: s (Set Int64))) (match s ({pat} 0) (_ 9))) (export f))");
    let all = diags_of(&src);
    assert!(
        all.iter().any(|d| d.code.as_deref() == Some("CDZ0201")
            && d.message.contains("exactly one binder after")),
        "the two-`..` `{pat}` set match pattern surfaces the coded CDZ0201 rest-SHAPE rejection in check \
         (malformed `..`; the single-`..` rest + membership behavior live in corpus 05): {all:?}"
    );
    // DUAL-READ PIN (M3 reader-flip guard): the LEGACY name-alias spelling `(set …)` routes to the SAME set
    // matcher as the native `#set(…)` leaf — `compound_form_of(_, Set)` recognizes both. A WELL-FORMED
    // single-`..` rest via the alias now LOWERS (slice 2): its `rest` binds (Case 6set-rest) and the body
    // reads it CLEAN — NO CDZ0201 "not yet supported" (superseded), NO CDZ0101 unbound. Locks that the
    // reader-flip transition keeps the alias routing to the working matcher, not a silent/uncoded decline.
    let alias = "(module m (def (f (: s (Set Int64))) (match s ((set 1 .. r) (Set.len r)) (_ 9))) (export f))";
    let alias_diags = diags_of(alias);
    assert!(
        !alias_diags
            .iter()
            .any(|d| matches!(d.code.as_deref(), Some("CDZ0201") | Some("CDZ0101"))),
        "the legacy alias `(set 1 .. r)` single-`..` rest now LOWERS (slice 2) — its `rest` binds + the \
         body reads it clean, no CDZ0201/CDZ0101 (dual-read routes to the working matcher): {alias_diags:?}"
    );
    // A whole-value-binder match over a HEAP-BACKED Set scrutinee (no `#set(…)` pattern) is a GENUINE
    // not-yet-built DECLINE: the compiler cannot yet emit a match over a heap-backed Set/Map scrutinee
    // (even a trivial whole-binder), so it produces CDZ0900 "matching a compound value needs a heap walk"
    // and NO artifact. Per seq-286 (v-deferral ruling A) that decline MUST be VISIBLE + coded in `check` —
    // it was formerly SILENTLY masked by a `dedup_faults` self-suppression bug (a lone coded decline
    // dropped itself at its own coded node), which made `check` wrongly report clean while `compile`
    // declined. The self-suppression fix (`coded_nodes` counts only genuine rejects) now surfaces it. The
    // separate "should a whole-binder over a heap collection emit WITHOUT a walk?" capability question is a
    // match-lowering follow-up (v-deferral catalogs a `MatchOverHeapCollectionScrutinee` DeclineId); if
    // built, this decline vanishes and the assertion reverts to checks-clean.
    let sc = "(module m (def (f (: s (Set Int64))) (match s (whole 0) (_ 9))) (export f))";
    let sc_diags = diags_of(sc);
    assert!(
        sc_diags
            .iter()
            .any(|d| d.severity == crate::abi::Severity::Error
                && d.code.as_deref() == Some("CDZ0900")
                && d.message.contains("needs a heap walk")),
        "a match over a heap-backed Set scrutinee surfaces the coded CDZ0900 heap-walk decline in check \
         (was silently self-suppressed; seq-286 requires a not-yet-built decline be visible): {sc_diags:?}"
    );
}

/// A `#set(e…)` membership pattern element is an ordinary VALUE EXPRESSION (the set twin of a map KEY),
/// NOT a binder — a set has no positional structure to bind (core-semantics §A Set Is Matched By
/// Element-Membership Patterns; v-spec-oracle-blessed). Two consequences this pins (v-rcdzc-ts-2 edge-hunt):
///  1. An IN-SCOPE name element (`#set(k)`, `k` a param used only for its membership value) must NOT be
///     collected as a match binder — it was, yielding a spurious CDZ0306 "unused match binding `k`" (whose
///     `_k` rename would also break the membership test). `arm_pattern_binders` now skips set elements.
///  2. An UNBOUND name element (`#set(a)`, `a` not in scope) is a plain unbound VALUE reference → CDZ0101,
///     exactly as any unbound value expression — the element is not a binder, so this is correct (a set
///     genuinely cannot bind; the fix is to test membership with an in-scope value, not to bind).
#[test]
fn a_set_membership_element_is_a_value_expression_not_a_binder() {
    // (1) An in-scope element used only for membership: NO spurious CDZ0306 on `k`.
    let ok =
        "(module m (def (f (: s (Set Int64)) (: k Int64)) (match s (#set(k) 9) (_ 0))) (export f))";
    let ok_diags = diags_of(ok);
    assert!(
        !ok_diags
            .iter()
            .any(|d| d.code.as_deref() == Some("CDZ0306") && d.message.contains("`k`")),
        "an in-scope `#set(k)` membership element (a value expression, not a binder) must NOT warn CDZ0306 \
         unused-binding on `k`: {ok_diags:?}"
    );
    // (2) An unbound element is a plain unbound value reference (CDZ0101) — a set element is not a binder —
    // carrying a STEER that names the membership semantics (a diagnostic-quality note requested by
    // v-rcdzc-test-shrink + blessed by v-spec-oracle ruling #6685: code stays CDZ0101, message guides).
    let unb = "(module m (def (main) (match #set(1 2) (#set(a) 9) (_ 0))) (export main))";
    let unb_diags = diags_of(unb);
    assert!(
        unb_diags
            .iter()
            .any(|d| d.code.as_deref() == Some("CDZ0101")
                && d.message.contains("unbound name `a`")
                && d.message.contains("does not bind")
                && d.message.contains("Set.contains")),
        "an unbound `#set(a)` element resolves as an unbound value expression (CDZ0101, NOT a CDZ0201 \
         kind-reject per ruling #6685), with a steer that a set names members by value and does not bind: \
         {unb_diags:?}"
    );
}

/// A binder BELOW a NESTED record field in a BINDING pattern — `#record((= x #tuple(c d)))` binds `c`/`d`
/// inside the record field `x`'s tuple value — is a not-yet-wired composition (a record field projects by
/// NAME, and there is no name-keyed `PathStep` to compose a projection with a further descent; the match
/// arm's Case 6rec-nested-decline declines the SAME shape). The binding path MUST give that clean decline —
/// NOT a misleading CDZ0101 "unbound `c`" at the body reference. The bug: the binding-path nested-decline
/// loop read only the LEGACY 2-element `(x value)` field form and MISSED the M3-nativized 3-element
/// `(= x value)` FieldPair (an `=`-migration miss, the sibling of the map effects-fold #6790), so a NATIVE
/// nested field's deeper binder fell through to a spurious CDZ0101 while `check` declined — a check≡compile
/// diagnostic split. Now both forms reach the decline; no cascade CDZ0101.
#[test]
fn a_deeper_positional_binder_below_a_native_nested_record_binding_field_binds() {
    // A POSITIONAL (tuple) compound below a native record BINDING field — `#record((= x #tuple(c d)))` —
    // now BINDS (§235 slice 2, the binding twin of the slice-1 match path): `resolve::last_binder_named`
    // wires it to a `RecordField` (empty path, field `x`, `sub_path = [Elem(0/1)]`) and
    // `check_binding_pattern` accepts the positional field value (`is_positional_field_value`). Must be
    // CLEAN — no CDZ0101 "unbound" cascade, no CDZ0900 "not supported" decline.
    let src = "(module m (def (f #record((= x #tuple(c d)))) (+ c d)) (export f))";
    let diags = diags_of(src);
    assert!(
        !diags.iter().any(|d| d.code.as_deref() == Some("CDZ0101")),
        "no misleading 'unbound name' cascade for the positional deeper binding-field binder: {diags:?}"
    );
    assert!(
        !diags.iter().any(|d| d
            .message
            .contains("nested compound sub-pattern inside a record binding pattern")),
        "the positional deeper binding-field binder BINDS via the RecordField sub_path (§235) — no CDZ0900 \
         'not supported' decline: {diags:?}"
    );
}

/// §235 full nested-record descent: a nested RECORD below a record binding field — `#record((= x #record((=
/// y v))))`, the field value itself a RECORD — now BINDS via a `RecordField` whose `sub_path` is a NAME-keyed
/// `RecordSubStep::Field(y)` (resolved to `Elem(<slot>)` at lowering). Must be CLEAN: no CDZ0101 "unbound",
/// no CDZ0900 "not supported" decline. (A VARIANT below a field stays deferred — refutable, a separate path.)
#[test]
fn a_record_below_a_record_binding_field_binds_via_name_keyed_sub_path() {
    let src = "(module m (def (f #record((= x #record((= y v)))))  v) (export f))";
    let diags = diags_of(src);
    assert!(
        !diags.iter().any(|d| d.code.as_deref() == Some("CDZ0101")),
        "no misleading 'unbound name' cascade for the nested-record binding field binder: {diags:?}"
    );
    assert!(
        !diags.iter().any(|d| d
            .message
            .contains("nested compound sub-pattern inside a record binding pattern")),
        "a nested record below a binding field now BINDS (§235 name-keyed sub_path) — no CDZ0900 decline: {diags:?}"
    );
}

/// A `(.. operand)` spread node in a `#map`/`#record` CONSTRUCTION entry gives the CLEAR pattern-only `..`
/// CDZ0201 that `#list`/`#set`/`#tuple` already give — NOT the misleading map/record entry-shape "add the
/// leading `=`" (the entry-shape check would misread `(.. m)` as a malformed `(key value)` pair). Reported
/// by v-syntax (construction-spread decline-quality); the fix recognizes the `..` node BEFORE the
/// entry-shape rejection in `resolve_map` / `read_record_fields`.
#[test]
fn a_spread_in_a_map_or_record_construction_entry_names_the_pattern_only_dotdot_rule() {
    for src in [
        "(module m (def (main) #map((.. m) (= 3 4))) (export main))",
        "(module m (def (main) #record((.. r) (= b 2))) (export main))",
    ] {
        let diags = diags_of(src);
        assert!(
            diags.iter().any(|d| d.code.as_deref() == Some("CDZ0201")
                && d.message.contains("`..` is a rest/spread marker")),
            "map/record construction spread must name the pattern-only `..` rule (like list/set/tuple): {diags:?}"
        );
        assert!(
            !diags
                .iter()
                .any(|d| d.message.contains("add the leading `=`")),
            "map/record construction spread must NOT misdiagnose as a malformed `(key value)` entry: {diags:?}"
        );
    }
}

/// A RECORD match pattern is now IMPLEMENTED (record match landed — the match twin of the record
/// binding pattern). A field-binder reference in the body must CHECK CLEAN (resolve to a scrutinee
/// projection, Case 6rec), NOT leak the old misleading "unbound name" CDZ0101 nor the superseded "not
/// yet supported" CDZ0201. This test was the v-diagnostics regression pin for the UNIMPLEMENTED era
/// (2026-07-16); it now pins that the field binder resolves and the arm type-checks. (The whole-binder
/// + projection form — the old workaround — remains valid too.)
#[test]
fn a_record_match_pattern_is_named_not_leaked_as_an_unbound_field_binder() {
    // Body references the field binder `a`. `f` is a parameterized (non-nullary) body, so this
    // exercises the check≡compile path — it must be CLEAN now that record match resolves the binder.
    let uses_binder = "(module m (def (f (: r (Record (x Int64)))) \
                           (match r ((record (x a)) a))) (export f))";
    let all = diags_of(uses_binder);
    assert!(
        all.iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a record match binding a field checks clean now that record match is implemented: {all:?}"
    );
    assert!(
        all.iter().all(|d| d.code.as_deref() != Some("CDZ0101")
            && !d
                .message
                .contains("record match pattern is not yet supported")),
        "no misleading unbound-name and no superseded 'not yet supported' decline: {all:?}"
    );
    // The WHOLE-value binder + field projection form (the former workaround) still checks clean.
    let workaround = "(module m (def (f (: r (Record (x Int64)))) \
                          (match r (whole (. whole x)))) (export f))";
    assert!(
        diags_of(workaround)
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "the whole-binder + projection form still checks clean: {:?}",
        diags_of(workaround)
    );
}

/// A MAP match pattern with a MALFORMED `..` rest (a `..` not followed by exactly one binder) reports
/// the clear rest-shape CDZ0201 — the map twin of the list's "a list rest pattern is `(list p… .. rest)`
/// — exactly one binder after `..`" — NOT a misleading "unbound name" for a value/rest binder. Before,
/// `map_pattern_of` collapsed a malformed `..` to `None`, so the arm's binders failed the inert-binder
/// classifier and the body reference resolved UNBOUND (masking the real fault, v-diagnostics note). Now
/// the resolver keeps those binders inert (`map_form_is_malformed_rest`) and both the map matcher and a
/// body-reference (resolve Case Mmr, co-anchored at the pattern) surface the SAME coded rest-shape
/// message, deduped to ONE diagnostic. Sibling of the record-pattern fix.
#[test]
fn a_map_match_pattern_with_a_malformed_rest_names_the_shape_not_an_unbound_binder() {
    // Corpus-covered facets CITE-DELETED to 05-compound-types: the SINGLE-`..` non-final malformed rest
    // (top-level `(map (1 v) .. rest (2 w))` + the `w`-after-`..` body-ref twin) is :18844, and the
    // NESTED-in-variant-payload twin `(Wrap (map … .. r (2 x)))` is :18849 — both CDZ0201 rest-shape,
    // native `#map` form. What STAYS here as legit white-box residue: (a) the TWO-`..` classic-form
    // no-unbound-leak pin below — its NATIVE `#map` form currently LEAKS a spurious CDZ0101 (queue repro
    // #47, routed to v-ast-compound); keep the classic pin until that fix lands, then migrate to native
    // corpus; (b) the well-formed "no false alarm" controls — a `(Map …)` entry parameter has no scalar
    // boundary representation, so they DECLINE at the export path (not clean runnable value cases).
    // Two `..` markers — clear rest-shape message, no unbound leak (classic form; native form is queue #47).
    let two_dots = "(module m (def (f (: mp (Map Int64 Int64))) \
                        (match mp ((map (1 v) .. r1 .. r2) v) (_ 0))) (export f))";
    let all = diags_of(two_dots);
    assert!(
        all.iter()
            .any(|d| d.code.as_deref() == Some("CDZ0201")
                && d.message.contains("map rest pattern is")),
        "two `..` markers report the rest-shape CDZ0201: {all:?}"
    );
    assert!(
        all.iter().all(|d| d.code.as_deref() != Some("CDZ0101")),
        "no unbound-name leak for the two-`..` case: {all:?}"
    );
    // NATIVE #map twin of the two-`..` case — the M2 surface `#map((= k v) …)` with FieldPair entries.
    // `map_form_binds_name` only recognized the legacy 2-element `(k v)` entry, so the native value binder
    // `v` was not classified inert on a malformed rest → the two-`..` NATIVE case leaked a spurious CDZ0101
    // on `v` that the classic form suppressed (v-rcdzc-test-shrink report 2026-08-30). Now the malformed-rest
    // helper reads the FieldPair value too, matching the classic path: clean CDZ0201, no unbound leak.
    let native_two_dots = "(module m (def (f (: mp (Map Int64 Int64))) \
                        (match mp (#map((= 1 v) .. r1 .. r2) v) (_ 0))) (export f))";
    let all = diags_of(native_two_dots);
    assert!(
        all.iter()
            .any(|d| d.code.as_deref() == Some("CDZ0201")
                && d.message.contains("map rest pattern is")),
        "native #map two-`..` reports the rest-shape CDZ0201: {all:?}"
    );
    assert!(
        all.iter().all(|d| d.code.as_deref() != Some("CDZ0101")),
        "no unbound-name leak for the NATIVE #map two-`..` value binder (v-rcdzc-test-shrink): {all:?}"
    );
    // NO false alarm: a WELL-FORMED map rest pattern (`.. rest` final, one binder) checks clean.
    let ok = "(module m (def (f (: mp (Map Int64 Int64))) \
                  (match mp ((map (1 v) .. rest) v) (_ 0))) (export f))";
    // Bind once — `diags_of` recompiles the module (PR #1167 review).
    let ok_diags = diags_of(ok);
    assert!(
        ok_diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a well-formed map rest pattern still checks clean: {ok_diags:?}"
    );
    // NO false alarm: a WELL-FORMED nested map (`.. r` final) still checks clean (the nested MALFORMED
    // twin is cite-deleted to corpus 05-compound-types:18849).
    let nested_ok = "(module m (type W (Wrap (Map Int64 Int64))) \
                         (def (f (: w W)) (match w ((Wrap (map (1 v) .. r)) v) (_ 0))) (export f))";
    // Bind once — `diags_of` recompiles the module (PR #1167 review).
    let nested_ok_diags = diags_of(nested_ok);
    assert!(
        nested_ok_diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a well-formed nested map rest pattern still checks clean: {nested_ok_diags:?}"
    );
}

/// The LIST twin of the map malformed-rest case above: a `(list …)` pattern whose `..` is followed by
/// MORE than one binder (`(list a .. b c)`) reports the clear rest-shape CDZ0201 and — when the body
/// references one of the SURPLUS post-`..` binders (`c`) — does NOT also leak a misleading CDZ0101
/// "unbound name". `lower_match_list` always faulted the shape, but `find_rest_binder_in_list_pattern`
/// recognizes ONLY the single `dd + 1` rest binder, so a body ref to `c` fell through to `resolve_name`
/// → a spurious unbound cascade on top of the real fault (the same class the map twin fixed,
/// v-diagnostics note 2026-07-16). The resolver now resolves such a reference to the SAME coded
/// rest-shape decline (Case Lmr / `match_arm_malformed_list_binds`), co-anchored at the list pattern so
/// the same-node dedup collapses it into ONE primary diagnostic.
#[test]
fn a_malformed_list_rest_pattern_names_the_shape_not_an_unbound_surplus_binder() {
    // Reject facets cite-deleted to corpus 05-compound-types: the single surplus binder is :18878, and
    // the TWO-surplus + NESTED-in-variant-payload facets are the two cases just below it (all CDZ0201
    // rest-shape, no unbound leak, native #list form — the native list path has no classic/native
    // discrepancy, unlike the #map two-`..` case in queue #47). What STAYS here: the well-formed no-false-
    // alarm control — a `(List …)` entry parameter has no scalar boundary, so it declines at the export
    // path (not a clean runnable value case).
    // NO false alarm: a WELL-FORMED list rest pattern (one binder after `..`, body reads it) checks clean.
    let ok = "(module m (def (f (: xs (List Int64))) \
                  (match xs ((list x .. rest) (List.len rest)) (_ 0))) (export f))";
    // Bind once — `diags_of` recompiles the module, so evaluating it in both the predicate and the
    // failure message would double the compile cost (PR #1167 review).
    let ok_diags = diags_of(ok);
    assert!(
        ok_diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a well-formed list rest pattern still checks clean: {ok_diags:?}"
    );
}

/// A malformed match PATTERN's CDZ0201 anchors at the OFFENDING PATTERN node (`(tuple a b c)`,
/// `(list … .. …)`), not the enclosing `(match …)`. The pattern-shape rejects in `pattern_constraints`
/// / `lower_match_list` carry the faulting `pat` node explicitly (`.at(pat)`); without it,
/// `collect_reached_poisons` stamped the coarse whole-match node, so the editor squiggle covered the
/// entire match instead of the one wrong pattern.
#[test]
fn a_malformed_match_pattern_anchors_at_the_pattern_not_the_whole_match() {
    // The reported node's HEAD is the pattern constructor, not `match` — the precise-anchor signal.
    fn anchor_head(src: &str) -> Option<String> {
        let ast = parse(src);
        let bytes = crate::codec::encode(&ast);
        let out = compile(
            &[Artifact::new(Artifact::KIND_AST, "m", bytes)],
            &[Target::Wasm],
        );
        let d = out
            .diagnostics
            .iter()
            .find(|d| d.code.as_deref() == Some("CDZ0201"))
            .expect("a CDZ0201");
        let node = d.node.expect("the CDZ0201 carries an anchor node");
        let db = Db::load(parse(src));
        db.ast.head_name(StructId(node)).map(str::to_string)
    }
    // A too-wide tuple pattern anchors at the `(tuple …)` pattern, NOT the `(match …)`.
    assert_eq!(
        anchor_head(
            "(module m (def (f (: t (Tuple Int64 Int64))) (match t ((tuple a b c) a))) (export f))"
        )
        .as_deref(),
        Some("tuple"),
        "the tuple-arity CDZ0201 anchors at the tuple pattern, not the match"
    );
    // A malformed list-rest pattern anchors at the `(list …)` pattern, NOT the `(match …)`.
    assert_eq!(
            anchor_head("(module m (def (f (: xs (List Int64))) (match xs ((list a .. rest b) a) (_ 0))) (export f))")
                .as_deref(),
            Some("list"),
            "the list-rest CDZ0201 anchors at the list pattern, not the match"
        );
}

/// The SCALAR-match well-formedness rejects (a pattern-type mismatch, a malformed guard) anchor at the
/// offending ARM PATTERN, not the enclosing `(match …)`. These fire in the scalar-probe path
/// (`lower_match_scalar` / the list-arm classifier), a DIFFERENT code path from the sum decision-tree
/// `pattern_constraints`, so they need their own `.at(pat)`. The anchor node must NOT be the match.
#[test]
fn a_scalar_match_pattern_fault_anchors_at_the_pattern_not_the_match() {
    // Whether the CDZ0201's anchor node is NOT the enclosing `(match …)` (its head is not `match`).
    fn anchors_off_the_match(src: &str) -> bool {
        let ast = parse(src);
        let bytes = crate::codec::encode(&ast);
        let out = compile(
            &[Artifact::new(Artifact::KIND_AST, "m", bytes)],
            &[Target::Wasm],
        );
        let d = out
            .diagnostics
            .iter()
            .find(|d| d.code.as_deref() == Some("CDZ0201"))
            .expect("a CDZ0201");
        let node = d.node.expect("the CDZ0201 carries an anchor node");
        let db = Db::load(parse(src));
        db.ast.head_name(StructId(node)) != Some("match")
    }
    // A Bool literal pattern against an Int64 scrutinee → the reject anchors at the `true` pattern.
    assert!(
        anchors_off_the_match(
            "(module m (def (f (: n Int64)) (match n (true 1) (_ 0))) (export f))"
        ),
        "the pattern-type-mismatch CDZ0201 anchors at the literal pattern, not the match"
    );
    // A malformed guard `(guard <pat> <cond> extra)` in a SCALAR match → anchors at the guard pattern.
    assert!(
        anchors_off_the_match(
            "(module m (def (f (: n Int64)) (match n ((guard 0 (> n 1) extra) 1) (_ 0))) (export f))"
        ),
        "the scalar-path malformed-guard CDZ0201 anchors at the guard pattern, not the match"
    );
    // A malformed guard in a LIST match (the list-arm classifier path) → anchors at the guard pattern.
    assert!(
        anchors_off_the_match(
            "(module m (def (f (: xs (List Int64))) (match xs ((guard (list a) (> a 1) extra) a) (_ 0))) (export f))"
        ),
        "the list-path malformed-guard CDZ0201 anchors at the guard pattern, not the match"
    );
}

/// A value-position type fault anchors at the offending SUB-EXPRESSION, not the enclosing form: an
/// `if` with a non-Bool condition points at the CONDITION (`5` in `(if 5 …)`); an `and`/`or`/`not` with
/// a non-Bool operand points at the OPERAND; a variant constructor applied to a wrong-type payload
/// points at the ARGUMENT. Each reject carries its faulting sub-node explicitly (`.at(cond)` /
/// `.at(operand)` / `.at(arg)`); without it, the `collect` walk stamped the coarse enclosing-form node.
#[test]
fn a_value_position_type_fault_anchors_at_the_sub_expression() {
    // The reported node is a LEAF ATOM (the `5`/`7`/`"hi"` sub-expression), not the enclosing List form.
    fn anchor_is_atom(src: &str) -> bool {
        let ast = parse(src);
        let bytes = crate::codec::encode(&ast);
        let out = compile(
            &[Artifact::new(Artifact::KIND_AST, "m", bytes)],
            &[Target::Wasm],
        );
        let d = out
            .diagnostics
            .iter()
            .find(|d| d.severity == crate::abi::Severity::Error)
            .expect("an error");
        let node = d.node.expect("the fault carries an anchor node");
        let db = Db::load(parse(src));
        matches!(db.ast.get(StructId(node)), crate::ast::Struct::Atom(_))
    }
    // `if` condition, `and`/`or`/`not` operand, and a ctor payload all anchor at the offending atom.
    for (src, what) in [
        ("(module m (def (f) (if 5 1 2)) (export f))", "if condition"),
        (
            "(module m (def (f (: p Bool)) (and p 5)) (export f))",
            "and operand",
        ),
        ("(module m (def (f) (not 7)) (export f))", "not operand"),
        (
            "(module m (type P (Mk Int64)) (def (f) (P.Mk \"hi\")) (export f))",
            "ctor payload",
        ),
    ] {
        assert!(
            anchor_is_atom(src),
            "the {what} fault anchors at the offending atom, not the enclosing form"
        );
    }
}

// MIGRATED to corpus (09-functions.sexp): the anonymous-lambda unused-parameter warning + `_x` fix, the
// used/`_`-prefixed clean cases, and the def-param "warns exactly once (not doubled by the lambda pass)"
// facet (a `(count 1)` on the def-parameter case). Rust test an_unused_anonymous_lambda_parameter_warns
// deleted — language-independent + corpus-covered.

#[test]
fn a_wide_parameter_list_flags_exactly_the_unused_parameters() {
    // The unused-parameter check collects the SET of body-referenced parameter names in ONE walk (was
    // one full-body walk PER parameter → O(params × body) = O(N²) for a wide signature). This locks in
    // the set-membership verdict at width: a 12-param def whose body references only the EVEN-indexed
    // params must warn for EXACTLY the 6 odd-indexed ones — no false positives on the used ones, no
    // misses on the unused ones. Guards that the O(N)→set rewrite preserves the per-parameter verdict.
    let params = (0..12)
        .map(|i| format!("x{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    // Body sums the even-indexed params (x0 + x2 + … + x10) — a balanced-ish chain of references.
    let body = (0..12)
        .filter(|i| i % 2 == 0)
        .map(|i| format!("x{i}"))
        .reduce(|a, b| format!("(+ {a} {b})"))
        .unwrap();
    let src = format!("(module m (def (f {params}) {body}) (export f))");
    let u = unused_of(&src);
    assert_eq!(
        u.len(),
        6,
        "exactly the 6 odd-indexed params are unused: {u:?}"
    );
    for odd in [1, 3, 5, 7, 9, 11] {
        assert!(
            u.iter().any(|m| m.contains(&format!("`x{odd}`"))),
            "x{odd} (never referenced) must warn: {u:?}"
        );
    }
    for even in [0, 2, 4, 6, 8, 10] {
        assert!(
            !u.iter().any(|m| m.contains(&format!("`x{even}`"))),
            "x{even} (referenced) must NOT warn: {u:?}"
        );
    }
}

#[test]
fn an_unused_let_binding_and_parameter_warn() {
    // `b` (a let binding) and `q` (a parameter) are declared but never referenced → CDZ0306; `a`
    // and `p` are used, so they do not warn.
    let src = "(module m (def (f p q) (let ((a (: 1 Int64)) (b (: 2 Int64))) (+ a p))) \
                   (export f))";
    let u = unused_of(src);
    assert_eq!(u.len(), 2, "exactly b and q are unused: {u:?}");
    assert!(
        u.iter().any(|m| m.contains("`b`") && m.contains("binding")),
        "{u:?}"
    );
    assert!(
        u.iter()
            .any(|m| m.contains("`q`") && m.contains("parameter")),
        "{u:?}"
    );
}

#[test]
fn an_underscore_prefix_silences_the_unused_warning() {
    // Rust's convention: a name beginning with `_` is intentionally unused — no warning.
    let src = "(module m (def (f p _q) (let ((a (: 1 Int64)) (_b (: 2 Int64))) (+ a p))) \
                   (export f))";
    assert!(
        unused_of(src).is_empty(),
        "`_q`/`_b` are silenced: {:?}",
        unused_of(src)
    );
}

#[test]
fn an_unused_nonexported_definition_warns_but_a_used_or_exported_one_does_not() {
    // A nullary def nothing references (and not exported) is unused.
    let unused = "(module m (def (helper) (: 9 Int64)) (def (main) 42) (export main))";
    let u = unused_of(unused);
    assert_eq!(u.len(), 1, "helper is unused: {u:?}");
    assert!(
        u[0].contains("`helper`") && u[0].contains("definition"),
        "{u:?}"
    );
    // A def that IS referenced does not warn.
    assert!(
        unused_of("(module m (def (helper) (: 9 Int64)) (def (main) helper) (export main))")
            .is_empty(),
        "a used def must not warn"
    );
    // An EXPORTED def is part of the interface — never flagged.
    assert!(
        unused_of("(module m (def (helper) (: 9 Int64)) (export helper))").is_empty(),
        "an exported def must not warn"
    );
}

// ── Redundant-arm warning (CDZ0213) — a match arm an earlier arm already covers. A WARNING that
// rides alongside a produced component (the program is well-formed; first-match-wins makes the
// shadowed arm dead), the pattern dual of the non-exhaustiveness rejection.

/// The CDZ0213 redundant-arm warnings from `src` (asserting a component WAS produced — a warning
/// must accompany a success, never a denial).
fn redundant_arms_of(src: &str) -> Vec<crate::abi::Diagnostic> {
    warnings_of(src)
        .into_iter()
        .filter(|d| d.code.as_deref() == Some("CDZ0213"))
        .collect()
}

#[test]
fn a_duplicate_or_shadowed_match_arm_warns_but_still_compiles() {
    // Each shape has an arm an earlier arm already fully covers: a repeated variant, a repeated
    // literal, an arm after a catch-all, a repeated Option variant. All COMPILE (first-match wins)
    // and emit exactly one CDZ0213.
    for src in [
        "(module m (type C Red Green) (def (f (: c C)) (match c ((C.Red) 1) ((C.Red) 2) ((C.Green) 0))) (def (main) (f (C.Red))) (export main))",
        "(module m (def (f (: n Int64)) (match n (0 1) (0 2) (_ 3))) (def (main) (f 0)) (export main))",
        "(module m (def (f (: n Int64)) (match n (_ 1) (0 2))) (def (main) (f 5)) (export main))",
        "(module m (def (f (: o (Option Int64))) (match o ((Some n) n) ((Some m) m) ((None) 0))) (def (main) (f (Some 5))) (export main))",
    ] {
        let redundant = redundant_arms_of(src);
        assert_eq!(
            redundant.len(),
            1,
            "expected exactly one redundant-arm (CDZ0213) warning for `{src}`, got {redundant:?}"
        );
    }
}

#[test]
fn a_well_formed_match_with_distinct_or_refining_or_guarded_arms_does_not_warn() {
    // Negatives: distinct variants, distinct literals, a payload-REFINING arm (`(Some 0)` before
    // `(Some n)` — covers only the value 0, not the whole variant), and GUARDED same-variant arms
    // (the guard is conditional, so neither arm subsumes the other) must NOT warn.
    for src in [
        "(module m (type C Red Green Blue) (def (f (: c C)) (match c ((C.Red) 1) ((C.Green) 2) ((C.Blue) 3))) (def (main) (f (C.Red))) (export main))",
        "(module m (def (f (: n Int64)) (match n (0 1) (1 2) (_ 3))) (def (main) (f 0)) (export main))",
        "(module m (def (f (: o (Option Int64))) (match o ((Some 0) 100) ((Some n) n) ((None) 0))) (def (main) (f (Some 5))) (export main))",
        "(module m (type B (V Int64)) (def (f (: x B)) (match x ((guard (B.V n) (> n 0)) 1) ((B.V n) 0))) (def (main) (f (B.V 5))) (export main))",
    ] {
        assert!(
            redundant_arms_of(src).is_empty(),
            "a well-formed match must not warn CDZ0213: `{src}` got {:?}",
            redundant_arms_of(src)
        );
    }
}

#[test]
fn a_catch_all_after_the_specific_arms_saturate_a_finite_type_is_redundant() {
    // The DUAL of the exhaustiveness check: a catch-all (or any arm) is unreachable when the SPECIFIC
    // arms before it already cover EVERY value of a FINITE scrutinee type — all variants of a sum, or
    // both booleans. `(match c (R 1) (G 2) (B 3) (_ 4))` on `(type C R G B)`: R/G/B exhaust `C`, so the
    // `_` can never match → CDZ0213. Before, the pass flagged only a catch-all-then-arm or a duplicate;
    // a trailing catch-all after a complete specific cover slipped through (it looked "reachable"
    // because no earlier CATCH-ALL preceded it). Each of these emits exactly one CDZ0213.
    for src in [
        // A wildcard after all three sum variants.
        "(module m (type C R G B) (def (f (: c C)) (match c ((C.R) 1) ((C.G) 2) ((C.B) 3) (_ 4))) (def (main) (f (C.R))) (export main))",
        // A wildcard after both booleans.
        "(module m (def (f (: b Bool)) (match b (true 1) (false 2) (_ 3))) (def (main) (f true)) (export main))",
        // A duplicate VARIANT arm after the type is saturated (not just a wildcard).
        "(module m (type C R G B) (def (f (: c C)) (match c ((C.R) 1) ((C.G) 2) ((C.B) 3) ((C.R) 5))) (def (main) (f (C.R))) (export main))",
        // Option: both variants covered, then a wildcard.
        "(module m (def (f (: o (Option Int64))) (match o ((Some n) n) ((None) 0) (_ -1))) (def (main) (f (Some 5))) (export main))",
        // A SINGLE-VARIANT sum (an erased newtype `Ty::Nominal`): its sole constructor saturates it, so
        // a trailing `_` is dead. `finite_cover_size` reads the variant count off the nominal's decl.
        "(module m (type Id (Mk Int64)) (def (f (: x Id)) (match x ((Id.Mk n) n) (_ 0))) (def (main) (f (Id.Mk 5))) (export main))",
    ] {
        let redundant = redundant_arms_of(src);
        assert_eq!(
            redundant.len(),
            1,
            "a catch-all after a finite type is fully covered is redundant (CDZ0213): `{src}`, got {redundant:?}"
        );
    }
}

// (an_open_sum_match_requires_an_open_tail_wildcard_arm migrated to corpus 15-rows-and-open-sums:
//  "an open sum match without an open-tail wildcard arm is non-exhaustive" (CDZ0210), "a closed sum
//  match covering every named variant is exhaustive without a wildcard", and "an open sum match WITH
//  an open-tail wildcard arm is exhaustive and compiles" — all three arms, backend-agnostic.)

#[test]
fn a_wildcard_arm_over_an_open_sum_is_never_redundant() {
    // OS1 — the redundant-arm dual: over a CLOSED sum, a `_` after every variant is covered is
    // CDZ0213 (the finite type is saturated). Over an OPEN sum the `_` is the ONLY cover for the
    // row-variable tail, so it is NEVER redundant — `finite_cover_size` returns `None` for an open
    // sum, so its `_` never closes finite coverage. No CDZ0213 even with every named variant covered.
    let open_wildcard = redundant_arms_of(
        "(module m (type V (Known Int64) (Unknown Int64) .. r) \
             (def (f (: v V)) (match v ((Known n) n) ((Unknown n) n) (_ 0))) \
             (def (main) (f (Known 1))) (export main))",
    );
    assert!(
        open_wildcard.is_empty(),
        "a `_` over an open sum is never redundant (it covers the open tail): {open_wildcard:?}"
    );

    // CONTRAST — the SAME arms over a CLOSED sum DO saturate it, so the trailing `_` is CDZ0213.
    let closed_wildcard = redundant_arms_of(
        "(module m (type V (Known Int64) (Unknown Int64)) \
             (def (f (: v V)) (match v ((Known n) n) ((Unknown n) n) (_ 0))) \
             (def (main) (f (Known 1))) (export main))",
    );
    assert_eq!(
        closed_wildcard.len(),
        1,
        "a `_` after a CLOSED sum is fully covered IS redundant (CDZ0213): {closed_wildcard:?}"
    );
}

#[test]
fn a_single_variant_open_sum_is_not_erased_to_an_irrefutable_newtype() {
    // OS1 soundness edge — a CLOSED single-variant sum `(type Box (Wrap Int64))` erases to a newtype
    // whose sole constructor pattern `(Wrap n)` is IRREFUTABLE (no `_` needed). But the SAME sum
    // declared OPEN `(type Box (Wrap Int64) .. r)` is NOT a newtype: the row variable means a value's
    // discriminant is not statically `Wrap`, so `(Wrap n)` does NOT cover it and a `_` arm is
    // required (`type-system.md §206`). Without suppressing the erasure, the open sum wrongly compiled
    // (the newtype pattern was irrefutable → the exhaustiveness check never ran).

    // (a) The OPEN single-variant sum, sole-ctor arm only, NO `_` → non-exhaustive (CDZ0210).
    let open_missing = all_errors(
        "(module m (type Box (Wrap Int64) .. r) \
             (def (unwrap (: b Box)) (match b ((Wrap n) n))) \
             (def (main) (unwrap (Wrap 42))) (export main))",
    );
    assert!(
        open_missing
            .iter()
            .any(|d| d.code.as_deref() == Some("CDZ0210")),
        "an open single-variant sum without a `_` arm is non-exhaustive: {open_missing:?}"
    );

    // (b) The CLOSED single-variant sum (no `.. r`) with the SAME sole-ctor arm is exhaustive — its
    // newtype erasure makes the ctor irrefutable, no `_` required. Isolates open-ness as the cause.
    let closed_ok = all_errors(
        "(module m (type Box (Wrap Int64)) \
             (def (unwrap (: b Box)) (match b ((Wrap n) n))) \
             (def (main) (unwrap (Wrap 42))) (export main))",
    );
    assert!(
        !closed_ok
            .iter()
            .any(|d| d.code.as_deref() == Some("CDZ0210")),
        "a CLOSED single-variant sum's sole-ctor pattern is irrefutable (no `_` needed): {closed_ok:?}"
    );

    // (c) The OPEN single-variant sum WITH the `_` arm compiles clean AND still reads the named
    // variant's payload (the erasure suppression must not break the `Wrap` payload binder).
    let open_ok = all_errors(
        "(module m (type Box (Wrap Int64) .. r) \
             (def (unwrap (: b Box)) (match b ((Wrap n) n) (_ 0))) \
             (def (main) (unwrap (Wrap 42))) (export main))",
    );
    assert!(
        open_ok.is_empty(),
        "an open single-variant sum with a `_` arm compiles clean: {open_ok:?}"
    );
}

#[test]
fn a_duplicate_or_shadowed_list_length_arm_is_redundant() {
    // LIST-match redundancy — the list-length analogue of the variant/literal duplicate & shadowing
    // checks. A list arm covers a length (exact `(list a)` = len 1, or a `≥ k` ray `(list a .. r)`); a
    // later arm whose lengths are all already covered is unreachable (`ArmCover::ListExact`/`ListFrom`
    // + the `min_list_from` subsumption). Each emits exactly one CDZ0213.
    for src in [
        // A duplicate exact length (both match length-1 lists).
        "(module m (def (f (: xs (List Int64))) (match xs ((list a) a) ((list b) 9) (_ 0))) (def (main) (f (list 1))) (export main))",
        // A duplicate empty-list arm.
        "(module m (def (f (: xs (List Int64))) (match xs ((list) 0) ((list) 9) (_ 1))) (def (main) (f (list))) (export main))",
        // A rest arm `(list a .. r)` [len ≥ 1] shadows a later exact `(list a b)` [len 2 ≥ 1].
        "(module m (def (f (: xs (List Int64))) (match xs ((list a .. r) a) ((list a b) 9) (_ 0))) (def (main) (f (list 1))) (export main))",
        // A zero-lead rest `(list .. r)` [every length] shadows a later `(list a)`.
        "(module m (def (f (: xs (List Int64))) (match xs ((list .. r) 0) ((list a) 9))) (def (main) (f (list))) (export main))",
    ] {
        let redundant = redundant_arms_of(src);
        assert_eq!(
            redundant.len(),
            1,
            "a duplicate/shadowed list-length arm is redundant (CDZ0213): `{src}`, got {redundant:?}"
        );
    }
}

#[test]
fn distinct_or_partly_covered_list_length_arms_do_not_warn() {
    // Negatives — no list arm's lengths are fully covered by an earlier one, so none is flagged.
    for src in [
        // Distinct lengths + a rest for the remainder: 0, 1, then ≥ 1 — the rest is NOT redundant (it
        // covers ≥ 2, which no earlier arm did) and neither exact arm shadows another.
        "(module m (def (f (: xs (List Int64))) (match xs ((list) 0) ((list a) a) ((list a .. r) 9))) (def (main) (f (list))) (export main))",
        // Distinct exact lengths.
        "(module m (def (f (: xs (List Int64))) (match xs ((list a) a) ((list a b) 9) (_ 0))) (def (main) (f (list 1))) (export main))",
        // A rest of lead 2 [len ≥ 2] does NOT cover a later exact len-1 `(list a)` — length 1 ∉ [2, ∞).
        "(module m (def (f (: xs (List Int64))) (match xs ((list a b .. r) a) ((list a) 7) (_ 0))) (def (main) (f (list 1))) (export main))",
    ] {
        assert!(
            redundant_arms_of(src).is_empty(),
            "a match whose list arms cover distinct lengths must not warn CDZ0213: `{src}` got {:?}",
            redundant_arms_of(src)
        );
    }
}

#[test]
fn a_structurally_duplicate_tuple_or_nested_ctor_arm_is_redundant() {
    // EXACT-DUPLICATE detection for TUPLE / REFINING-CONSTRUCTOR arms (`ArmCover::Shape`) — two arms of
    // the same structural shape (binders normalized to `_`, literals by value) match the same region, so
    // the later is unreachable. The tuple/nested analogue of the variant/literal/list duplicate check.
    for src in [
        // A duplicate tuple arm (`(tuple true a)` vs `(tuple true b)` — same shape `(t b:true _)`).
        "(module m (def (f (: t (Tuple Bool Int64))) \
               (match t ((tuple true a) a) ((tuple true b) b) ((tuple false c) c))) \
             (def (main) (f (tuple true 1))) (export main))",
        // A duplicate NESTED-ctor arm (`(Some (Some x))` vs `(Some (Some y))`).
        "(module m (def (f (: o (Option (Option Int64)))) \
               (match o ((Some (Some x)) x) ((Some (Some y)) y) ((Some (None)) 0) ((None) -1))) \
             (def (main) (f (None))) (export main))",
        // A duplicate tuple arm with a refining element + literal (`(tuple (Some x) 0)` twice).
        "(module m (def (f (: t (Tuple (Option Int64) Int64))) \
               (match t ((tuple (Some x) 0) x) ((tuple (Some y) 0) y) (_ 9))) \
             (def (main) (f (tuple (None) 1))) (export main))",
    ] {
        let redundant = redundant_arms_of(src);
        assert_eq!(
            redundant.len(),
            1,
            "a structurally-duplicate tuple/nested-ctor arm is redundant (CDZ0213): `{src}`, got {redundant:?}"
        );
    }
}

#[test]
fn distinct_tuple_or_nested_ctor_arms_do_not_warn() {
    // Negatives — no arm structurally repeats an earlier one, so none is flagged. Critically, a set of
    // REFINING arms that jointly EXHAUST a nested sum (`(Some (Some x)) + (Some (None)) + (None)`) must
    // NOT be mis-saturated: a `Shape` cover is PARTIAL (covers only part of the `Some` variant), so it
    // does not count toward the 2-variant `Option` saturation and no arm is wrongly flagged.
    for src in [
        // Exhaustive nested-Option refinement — three distinct shapes, no duplicate, no over-saturation.
        "(module m (def (f (: o (Option (Option Int64)))) \
               (match o ((Some (Some x)) x) ((Some (None)) 0) ((None) -1))) \
             (def (main) (f (None))) (export main))",
        // Distinct tuple first-column values.
        "(module m (def (f (: t (Tuple Bool Int64))) \
               (match t ((tuple true a) a) ((tuple false b) b))) \
             (def (main) (f (tuple true 1))) (export main))",
        // Distinct refining-element LITERALS in the same tuple shape.
        "(module m (def (f (: t (Tuple (Option Int64) Int64))) \
               (match t ((tuple (Some x) 0) x) ((tuple (Some x) 1) x) (_ 9))) \
             (def (main) (f (tuple (None) 2))) (export main))",
    ] {
        assert!(
            redundant_arms_of(src).is_empty(),
            "a match whose tuple/nested arms are structurally distinct must not warn CDZ0213: `{src}` got {:?}",
            redundant_arms_of(src)
        );
    }
}

#[test]
fn a_refining_arm_shadowed_by_an_earlier_full_variant_cover_is_redundant() {
    // VARIANT-REFINEMENT SUBSUMPTION: a FULL-variant cover `(Some _)` matches every value of the `Some`
    // variant, so a LATER refining arm of the SAME variant (`(Some (Some x))`, an `ArmCover::Shape`) is
    // unreachable. Beyond the exact-duplicate `Shape` check (Inc 28) — this is a BROADER earlier arm
    // shadowing a NARROWER same-variant later one. Each source emits exactly one CDZ0213 on the later arm.
    for src in [
        // `(Some _)` [full Some] then `(Some (Some x))` [refining Some] — the refinement is dead.
        "(module m (def (f (: o (Option (Option Int64)))) \
               (match o ((Some _) 0) ((Some (Some x)) x) ((None) -1))) \
             (def (main) (f (None))) (export main))",
        // A bare binder payload `(Some p)` is also a full cover; a later `(Some (None))` is shadowed.
        "(module m (def (f (: o (Option (Option Int64)))) \
               (match o ((Some p) 1) ((Some (None)) 2) ((None) -1))) \
             (def (main) (f (None))) (export main))",
        // A refining TUPLE payload after a full tuple-payload cover (`(Some (tuple _ _))` covers whole Some).
        "(module m (def (f (: o (Option (Tuple Bool Int64)))) \
               (match o ((Some (tuple _ _)) 0) ((Some (tuple true c)) c) ((None) -1))) \
             (def (main) (f (None))) (export main))",
    ] {
        let redundant = redundant_arms_of(src);
        assert_eq!(
            redundant.len(),
            1,
            "a refining arm shadowed by an earlier full-variant cover is redundant (CDZ0213): `{src}`, got {redundant:?}"
        );
    }
    // FALSE-POSITIVE guards: a refining arm is NOT shadowed when NO earlier arm covered its variant in
    // FULL — a refinement BEFORE the full cover is reachable, and refinements of a variant never covered
    // in full (`(Some (Some x))` + `(Some (None))`, jointly exhausting Some but neither a full cover) do
    // not shadow each other.
    for src in [
        // The refinement comes FIRST — reachable; the later full `(Some _)` is broader, not shadowed.
        "(module m (def (f (: o (Option (Option Int64)))) \
               (match o ((Some (Some x)) x) ((Some _) 0) ((None) -1))) \
             (def (main) (f (None))) (export main))",
        // Two refinements of Some, no full-Some cover — jointly exhaustive, neither shadows the other.
        "(module m (def (f (: o (Option (Option Int64)))) \
               (match o ((Some (Some x)) x) ((Some (None)) 0) ((None) -1))) \
             (def (main) (f (None))) (export main))",
    ] {
        assert!(
            redundant_arms_of(src).is_empty(),
            "a refinement not shadowed by an earlier full cover must not warn CDZ0213: `{src}` got {:?}",
            redundant_arms_of(src)
        );
    }
}

#[test]
fn an_all_wildcard_tuple_arm_is_a_catch_all_that_shadows_later_arms() {
    // An ALL-IRREFUTABLE tuple `(tuple x y)` / `(tuple _ _)` matches EVERY value of its tuple type — it
    // is a whole-type CatchAll (`is_irrefutable_cover`), so any arm after it is unreachable. The
    // product-subsumption whole-tuple case; before, only a BARE `_`/binder was a catch-all and a broader
    // tuple arm silently shadowed with no warning. Composes through nesting (`(tuple _ (tuple a b))`) and
    // a ctor payload (`(Some (tuple _ _))` covers the whole `Some` variant). Each emits exactly one CDZ0213.
    for src in [
        // A binder-only tuple arm shadows a later refining tuple arm.
        "(module m (def (f (: t (Tuple Bool Int64))) \
               (match t ((tuple x y) y) ((tuple true c) c))) \
             (def (main) (f (tuple true 1))) (export main))",
        // `(tuple _ _)` before a literal arm.
        "(module m (def (f (: t (Tuple Bool Int64))) \
               (match t ((tuple _ _) 0) ((tuple true c) c))) \
             (def (main) (f (tuple true 1))) (export main))",
        // NESTED all-wildcard tuple is still a whole cover.
        "(module m (def (f (: t (Tuple Bool (Tuple Int64 Int64)))) \
               (match t ((tuple x (tuple a b)) a) ((tuple true (tuple c d)) c))) \
             (def (main) (f (tuple true (tuple 1 2)))) (export main))",
    ] {
        let redundant = redundant_arms_of(src);
        assert_eq!(
            redundant.len(),
            1,
            "an all-wildcard tuple catch-all shadows the later arm (CDZ0213): `{src}`, got {redundant:?}"
        );
    }
    // FALSE-POSITIVE guard: an all-wildcard tuple as the SOLE arm is exhaustive, not self-redundant; and
    // a REFINING tuple arm BEFORE a wildcard tuple arm does not shadow it (the refinement covers less).
    for src in [
        "(module m (def (f (: t (Tuple Bool Int64))) (match t ((tuple x y) y))) \
             (def (main) (f (tuple true 1))) (export main))",
        "(module m (def (f (: t (Tuple Bool Int64))) \
               (match t ((tuple true a) a) ((tuple x y) y))) \
             (def (main) (f (tuple true 1))) (export main))",
    ] {
        assert!(
            redundant_arms_of(src).is_empty(),
            "an exhaustive / refining-before-wildcard tuple match must not warn CDZ0213: `{src}` got {:?}",
            redundant_arms_of(src)
        );
    }
}

#[test]
fn an_exhaustive_finite_match_without_a_trailing_catch_all_does_not_warn() {
    // The boundary: coverage closes AFTER the last covering arm, so an EXHAUSTIVE finite match with no
    // trailing arm has nothing to flag (no false positive). A wildcard that is REACHABLE (the specific
    // arms do NOT yet saturate the type) also must not warn, and an OPEN scalar type is never saturated
    // by literals, so its wildcard stays reachable.
    for src in [
        // Exhaustive sum, NO wildcard — the last arm closes coverage but nothing follows it.
        "(module m (type C R G B) (def (f (: c C)) (match c ((C.R) 1) ((C.G) 2) ((C.B) 3))) (def (main) (f (C.R))) (export main))",
        // Exhaustive bool, no wildcard.
        "(module m (def (f (: b Bool)) (match b (true 1) (false 2))) (def (main) (f true)) (export main))",
        // Missing a variant, so the wildcard is REACHABLE — not redundant.
        "(module m (type C R G B) (def (f (: c C)) (match c ((C.R) 1) ((C.G) 2) (_ 4))) (def (main) (f (C.R))) (export main))",
        // Open Int scalar — a finite set of literals never saturates it, so the wildcard is reachable.
        "(module m (def (f (: n Int64)) (match n (0 1) (1 2) (_ 3))) (def (main) (f 0)) (export main))",
    ] {
        assert!(
            redundant_arms_of(src).is_empty(),
            "no false positive: `{src}` got {:?}",
            redundant_arms_of(src)
        );
    }
}

#[test]
fn a_redundant_arm_warning_carries_a_delete_fix_for_the_whole_arm() {
    // The rustc-gold repair for an unreachable arm: DELETE it. The warning now carries a `delete` fix
    // targeting the ARM node (the `(pattern body)` list, the pattern's PARENT) — not the pattern alone
    // — so applying it removes pattern AND body together. Heuristic: a redundant arm is often a pattern
    // bug (the author meant a different, reachable pattern), so an agent confirms rather than applies blind.
    let src = "(module m (def (f (: n Int64)) (match n (0 1) (0 2) (_ 3))) (def (main) (f 0)) (export main))";
    let ws = redundant_arms_of(src);
    assert_eq!(ws.len(), 1);
    let fix = ws[0]
        .fix
        .as_ref()
        .expect("a redundant-arm warning carries a delete fix");
    assert_eq!(fix.kind, crate::abi::FixKind::Delete);
    assert!(
        !fix.verified,
        "a redundant arm is often a pattern bug — confirm, don't apply blind"
    );
    // The fix targets the ARM (`(pattern body)`), which is the PARENT of the warning's pattern node.
    let db = Db::load(parse(src));
    let pat = StructId(ws[0].node.expect("the warning carries the dead pattern"));
    let arm = db
        .parent_of(pat)
        .expect("the pattern has an enclosing arm node");
    assert_eq!(
        fix.node, arm.0,
        "the delete targets the whole arm, not just the pattern"
    );
}

#[test]
fn a_redundant_arm_warning_anchors_to_the_dead_arms_pattern() {
    // The warning carries the DEAD arm's PATTERN node — a real user node the front-end maps to the
    // redundant arm's span (not a prelude/synthesized id).
    let src = "(module m (type C Red Green) (def (f (: c C)) (match c ((C.Red) 1) ((C.Red) 2) ((C.Green) 0))) (def (main) (f (C.Red))) (export main))";
    let ws = redundant_arms_of(src);
    assert_eq!(ws.len(), 1);
    let node = ws[0]
        .node
        .expect("a redundant-arm warning must carry a node");
    let db = Db::load(parse(src));
    assert!(
        db.is_user_node(StructId(node)),
        "node {node} must be a user node"
    );
}

// MIGRATED to corpus (03-equality-and-observation.sexp, "a runtime float compare is a coded CDZ0203"):
// the CDZ0203 rejection + the full actionable message (IEEE-partial-order reason + the `<`/`<=`/`>`/`>=`
// redirect, as three AND-required `(message …)` clauses) are corpus-asserted; the "named repair compiles
// clean" witness is covered by the `(= (< a b) true)` over-Float64 case in the same chapter. Rust test
// compare_on_a_float_names_the_relational_operators_as_the_fix deleted — language-independent + corpus-covered.

#[test]
fn compare_of_a_compound_with_an_unorderable_leaf_names_the_component_wise_route() {
    // A COMPOUND (tuple/record/list/sum) is ordered lexicographically ONLY when every leaf offers a
    // total order. A float (or bytes/set/map) leaf INSIDE a compound makes the whole compound
    // un-orderable (a float offers only the IEEE partial order; §319 / core-semantics.md
    // #compound-ordering-is-lexicographic), so `compare` over it declines — but the message must not
    // dead-end: it names the actionable route, comparing the orderable components individually. This
    // pins that redirect (lower.rs compound-`compare` arm) so a refactor can't degrade it. Runtime
    // float params inside a tuple so it reaches lowering (a constant compound would fold).
    let d = first_error(
        "(module m (def (f (: x Float64) (: y Float64)) (Ordering.of (tuple x 1) (tuple y 2))) (export f))",
    );
    // An uncoded DECLINE (the un-orderable-leaf carve-out), not a coded rejection.
    assert_eq!(
        d.code, None,
        "an un-orderable-leaf compound compare is an uncoded decline: {}",
        d.message
    );
    assert!(
        d.message.contains("no total order the compiler can walk")
                // names WHY (the offending leaf kinds) AND the component-wise route
                && d.message.contains("float/bytes/set/map leaf")
                && d.message.contains("compare its orderable components individually"),
        "the decline names the un-orderable-leaf reason AND the component-wise route: {}",
        d.message
    );
    // ROUND-TRIP witness: the named route — comparing the orderable component (the Int field) on its
    // own — compiles clean, so the redirect points at a form that type-checks.
    let ast = crate::testkit::parse(
        "(module m (def (f (: x Float64) (: y Float64)) (Ordering.of 1 2)) (export f))",
    );
    let out = compile(
        &[Artifact::new(
            Artifact::KIND_AST,
            "m",
            crate::codec::encode(&ast),
        )],
        &[Target::Wasm],
    );
    assert!(
        !out.diagnostics
            .iter()
            .any(|d| d.severity == crate::abi::Severity::Error),
        "the component-wise route (compare the orderable Int component) compiles clean: {:?}",
        out.diagnostics
    );
}

#[test]
fn ordering_a_compound_with_an_unorderable_leaf_is_a_carve_out_not_a_not_yet_built_decline() {
    // The RELATIONAL-operator (`<`/`<=`/`>`/`>=`) sibling of the compound-`compare` carve-out. A
    // relational op reaching the compound decline ALWAYS has an un-orderable leaf: an all-orderable
    // compound already took the runtime `ValueCmp` ordering arm. So a `<` over a float-leaf tuple is a
    // PERMANENT carve-out (a float has only the IEEE partial order), NOT the "needs a heap walk (not yet
    // built)" the equality path names — that message MISLED (read as a temporary limit a later slice
    // lifts). The message now mirrors `compare`: names the un-orderable leaf + the component-wise route.
    let d = first_error(
        "(module m (def (f (: x Float64) (: y Float64)) (< (tuple x 1) (tuple y 2))) (export f))",
    );
    assert_eq!(
        d.code, None,
        "an un-orderable-leaf compound ordering is an uncoded decline: {}",
        d.message
    );
    assert!(
        d.message
            .contains("has no total order, so it cannot be ordered")
            && d.message.contains("float, set, or map leaf")
            && d.message
                .contains("order its orderable components individually"),
        "the ordering decline names the un-orderable-leaf reason + the component-wise route (NOT a \
             'not yet built' heap walk): {}",
        d.message
    );
    // It must NOT claim the misleading "not yet built" the equality path uses — this is permanent.
    assert!(
        !d.message.contains("not yet built"),
        "an ordering carve-out must not read as a temporary limitation: {}",
        d.message
    );
    // EQUALITY (`=`) over a float-leaf compound is SUPPORTED (the ValueEqShaped path) — no decline; so the
    // "needs a heap walk (not yet built)" message stays reachable only for genuinely-unbuilt cases.
    // ROUND-TRIP witness: the named route — ordering the orderable Int component alone — compiles clean.
    let ast = crate::testkit::parse(
        "(module m (def (f (: x Float64) (: y Float64)) (< 1 2)) (export f))",
    );
    let out = compile(
        &[Artifact::new(
            Artifact::KIND_AST,
            "m",
            crate::codec::encode(&ast),
        )],
        &[Target::Wasm],
    );
    assert!(
        !out.diagnostics
            .iter()
            .any(|d| d.severity == crate::abi::Severity::Error),
        "the component-wise route (order the Int component) compiles clean: {:?}",
        out.diagnostics
    );
}

// (a_mismatched_type_ordering_stays_a_single_coded_error_not_a_double_with_the_ordering_decline migrated to
//  corpus 07-type-system: `(< 1 "x")` cross-kind ordering compare -> CDZ0201 "different types" with (count 1)
//  pinning the dedup (the ordering carve-out decline is dropped, ONE fault not a double) — via the
//  cross-diagnostic (count N) lever that reclaims a former white-box dedup test.)
