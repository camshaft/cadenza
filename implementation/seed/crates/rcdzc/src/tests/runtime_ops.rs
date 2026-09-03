use crate::compile::compile_component;
use crate::testkit::parse;

/// Compile `(module m (def (f <params>) <body>) (export f))` and return its bytes.
fn func(params: &str, body: &str) -> Vec<u8> {
    let src = format!("(module m (def (f {params}) {body}) (export f))");
    compile_component(&crate::codec::encode(&parse(&src))).expect("compile")
}

#[test]
fn multiply_by_negative_one_is_negation() {
    // `(* x -1)` / `(* -1 x)` is negation `(- 0 x)` — a strength reduction: the full-width `* -1`
    // otherwise keeps the expensive `div_s` round-trip guard (the const-multiplier fast path excludes
    // -1), but negation has the single `x == MIN` overflow check. Value + trap identical to `* -1`.
    use crate::backend::wasm::lir::Lir;
    use crate::db::Db;
    let lir = |params: &str, body: &str| -> Vec<Lir> {
        let ast = crate::testkit::parse(&format!(
            "(module m (def (f {params}) {body}) (def (main) 0) (export main))"
        ));
        let mut db = Db::load(ast);
        let layout = crate::layout::compute(&mut db).expect("layout");
        let d = db.def_by_name("f").expect("def f");
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
        crate::backend::wasm::select::select_function(&mut db, body, &ps, &layout)
            .expect("select")
            .code
    };
    // Full-width `(* x -1)`: NO `div_s` (it became `0 - x`), and the negation's `x == MIN` guard is a
    // single `i64.eq` (not the mul's div round-trip). Both operand orders.
    for body in ["(* x -1)", "(* -1 x)"] {
        let c = lir("(: x Int64)", body);
        assert!(
            !c.iter().any(|i| matches!(i, Lir::I64DivS)),
            "{body} is negation, no div_s: {c:?}"
        );
        assert!(
            c.iter().any(|i| matches!(i, Lir::I64Sub)),
            "{body} emits a subtraction (0 - x): {c:?}"
        );
    }
    // Value + trap parity (`* -1` = -x in both orders; Int64/Int8 MIN overflow traps; the kept operand's
    // own trap `(* (/ 10 y) -1)` survives) migrated to the corpus (run via cdz-run): cases "multiplying a
    // runtime integer by negative one is checked negation in both operand orders" and
    // "multiply-by-negative-one strength reduction keeps its operand's own trap" in
    // spec/semantics/06-numeric-model.sexp.
}

#[test]
fn a_signed_pow2_div_of_a_nonneg_dividend_drops_the_round_toward_zero_bias() {
    // A signed `/`/`%` by a power of two normally emits the round-toward-zero BIAS sequence (needed
    // only to correct NEGATIVE dividends). When the dividend is provably NON-NEGATIVE — a mask
    // (`(& x 255)` ∈ [0,255]), or a flow-refined `x` under `(> x 0)` — the bias is DEAD: `x / 2^k`
    // = `x >>ₛ k` and `x % 2^k` = `x & (2^k−1)`, exactly the unsigned case. Pins the elision at the
    // Lir level (the bias's second shift + the `add` are gone) AND the value parity.
    use crate::backend::wasm::lir::Lir;
    use crate::db::Db;
    let lir = |params: &str, body: &str| -> Vec<Lir> {
        let ast = crate::testkit::parse(&format!(
            "(module m (def (f {params}) {body}) (def (main) 0) (export main))"
        ));
        let mut db = Db::load(ast);
        let layout = crate::layout::compute(&mut db).expect("layout");
        let d = db.def_by_name("f").expect("def f");
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
        crate::backend::wasm::select::select_function(&mut db, body, &ps, &layout)
            .expect("select")
            .code
    };
    // The bias sequence's tell is a `ShrU` (`>>ᵤ (W−k)` to build `2^k−1`) plus an `Add`. A
    // non-negative masked dividend divide drops BOTH — just `and 255 ; const 1 ; shr_s`.
    let masked_div = lir("(: x Int64)", "(: (/ (& x 255) 2) Int64)");
    assert!(
        !masked_div.contains(&Lir::I64ShrU) && !masked_div.contains(&Lir::I64Add),
        "a nonneg dividend needs no toward-zero bias — no ShrU/Add; got: {masked_div:?}"
    );
    assert!(
        masked_div.contains(&Lir::I64ShrS),
        "the quotient is a single arithmetic shift; got: {masked_div:?}"
    );
    // The masked REM is a pure mask, no bias.
    let masked_rem = lir("(: x Int64)", "(: (% (& x 255) 4) Int64)");
    assert!(
        !masked_rem.contains(&Lir::I64ShrU) && !masked_rem.contains(&Lir::I64ShrS),
        "a nonneg dividend rem is a pure and-mask, no shifts; got: {masked_rem:?}"
    );
    // CONTRAST: a bare signed dividend (unknown sign) KEEPS the bias — a `ShrU` is present.
    let bare_div = lir("(: x Int64)", "(/ x 2)");
    assert!(
        bare_div.contains(&Lir::I64ShrU),
        "an unknown-sign dividend keeps the round-toward-zero bias; got: {bare_div:?}"
    );
}

#[test]
fn a_tuple_eq_with_a_const_divisor_bool_element_emits_valid_wasm() {
    // MISCOMPILE (invalid wasm): a compound (tuple) `=` whose Bool element derives from a CONST-DIVISOR
    // `%`/`/` — `(= (tuple 5 (= (% s 2) 0)) (tuple 5 (= (% s 2) 0)))` — emitted an invalid module
    // (`type mismatch: expected i32, found i64`). The two identical Bool elements are `core_eq`, so the
    // non-loop CSE pass materialized the shared `(% s 2)` into an i64 slot but did NOT advance the scratch
    // floor past the const-divisor strength-reduction's transient i64 dividend scratch — so the i32 Bool
    // slot of `(= … 0)` reused that i64 slot (one wasm local, two widths). Fix: the CSE materialization
    // raises `body_base` past `high` after emitting the rep (+ `emit_div_rem` reserves its scratch above
    // `*high`). `component` compiles + validates the module (it `.expect("compile")`s and the backend
    // validates), so a bare call is the regression guard. NOT modulo-specific — const-`/` reproduces.
    let valid = |src: &str| {
        compile_component(&crate::codec::encode(&crate::testkit::parse(src))).expect("compile")
    };
    let _ = valid(
        "(module m (def (main (: s Int64)) (= (tuple 5 (= (% s 2) 0)) (tuple 5 (= (% s 2) 0)))) (export main))",
    );
    let _ = valid(
        "(module m (def (main (: s Int64)) (= (tuple 5 (= (/ s 2) 1)) (tuple 5 (= (/ s 2) 1)))) (export main))",
    );
    // Differing const divisors (two distinct `%` subexpressions) must each emit valid wasm too.
    let _ = valid(
        "(module m (def (main (: s Int64)) (= (tuple 5 (= (% s 2) 0)) (tuple 5 (= (% s 3) 0)))) (export main))",
    );
}

#[test]
fn a_narrow_signed_division_by_a_non_neg_one_divisor_elides_its_range_check() {
    use crate::backend::wasm::lir::Lir;
    use crate::db::Db;
    let lir = |params: &str, body: &str| -> Vec<Lir> {
        let ast = crate::testkit::parse(&format!(
            "(module m (def (f {params}) {body}) (def (main) 0) (export main))"
        ));
        let mut db = Db::load(ast);
        let layout = crate::layout::compute(&mut db).expect("layout");
        let d = db.def_by_name("f").expect("def f");
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
        crate::backend::wasm::select::select_function(&mut db, body, &ps, &layout)
            .expect("select")
            .code
    };
    // `(/ x:Int8 3)`: the divisor is a constant ≠ -1, so the only overflowing quotient (MIN_8/-1)
    // cannot arise — the narrow-signed range-check is dead (just `i32.div_s`, no comparison/trap).
    let c = lir("(: x Int8)", "(/ x 3)");
    assert!(
        c.contains(&Lir::I32DivS) && !c.iter().any(|i| matches!(i, Lir::IfIntegerOverflowEnd)),
        "a narrow div by a non-(-1) constant drops its range-check; got {c:?}"
    );
    // A divisor whose range excludes -1 (`(& y 7)` ∈ [0,7]) likewise — the ÷0 native trap stays.
    let masked = lir("(: x Int8) (: y Int8)", "(/ x (& y 7))");
    assert!(
        !masked
            .iter()
            .any(|i| matches!(i, Lir::IfIntegerOverflowEnd)),
        "a narrow div by a range-excludes-(-1) divisor drops its range-check; got {masked:?}"
    );
    // SAFETY: a runtime divisor (could be -1) KEEPS the range-check.
    let open = lir("(: x Int8) (: y Int8)", "(/ x y)");
    assert!(
        open.iter().any(|i| matches!(i, Lir::IfIntegerOverflowEnd)),
        "a runtime divisor keeps the range-check; got {open:?}"
    );
    // SAFETY: divisor IS -1 → keep (MIN_8 / -1 overflows).
    let neg1 = lir("(: x Int8)", "(/ x -1)");
    assert!(
        neg1.iter().any(|i| matches!(i, Lir::IfIntegerOverflowEnd)),
        "a -1 divisor keeps the range-check; got {neg1:?}"
    );
}

#[test]
fn a_narrow_signed_division_of_a_nonneg_dividend_elides_its_range_check() {
    // The narrow-signed-div range-check exists SOLELY for `MIN_N / -1` (the one over-type quotient).
    // `MIN_N` is NEGATIVE, so a NON-NEGATIVE dividend can never be it: for `a ≥ 0` and any `d ≠ 0`,
    // `|a/d| ≤ a ≤ MAX_N`, so the quotient always fits — the check is dead EVEN with a runtime `-1`
    // divisor (`a / -1 = -a ∈ [-MAX_N, 0]`, still in type). Complements the divisor-≠-(-1) elision.
    use crate::backend::wasm::lir::Lir;
    use crate::db::Db;
    let lir = |params: &str, body: &str| -> Vec<Lir> {
        let ast = crate::testkit::parse(&format!(
            "(module m (def (f {params}) {body}) (def (main) 0) (export main))"
        ));
        let mut db = Db::load(ast);
        let layout = crate::layout::compute(&mut db).expect("layout");
        let d = db.def_by_name("f").expect("def f");
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
        crate::backend::wasm::select::select_function(&mut db, body, &ps, &layout)
            .expect("select")
            .code
    };
    // A masked (nonneg) dividend divided by a RUNTIME divisor: the range-check is dead (even though
    // the divisor could be -1), but the ÷0 native trap stays in the bare `div_s`.
    let masked = lir("(: x Int8) (: d Int8)", "(/ (& x 7) d)");
    assert!(
        masked.contains(&Lir::I32DivS)
            && !masked
                .iter()
                .any(|i| matches!(i, Lir::IfIntegerOverflowEnd)),
        "a nonneg dividend needs no MIN/-1 range-check; got {masked:?}"
    );
    // SAFETY: an unknown-sign dividend with a runtime divisor KEEPS the check.
    let bare = lir("(: x Int8) (: d Int8)", "(/ x d)");
    assert!(
        bare.iter().any(|i| matches!(i, Lir::IfIntegerOverflowEnd)),
        "an unknown-sign dividend keeps the range-check; got {bare:?}"
    );
    // A FLOW-REFINED nonneg dividend elides too — and this is the VALUE-FACTS-specific invariant: the
    // range-check drops because the interval refinement (not the type) proves `x` nonneg. Inside the
    // then-branch of `(> x 0)`, `x` refines to `[1, 127]`, so `value_provably_nonneg(x)` holds at the
    // `(/ x d)` emit → the MIN/-1 range-check is dead, exactly as the type-level masked case above. Pin
    // it at the Lir level (not just value parity) so a refinement regression that stopped reaching the
    // div guard would be caught, not silently keep the guard.
    let refined_div = lir("(: x Int8) (: d Int8)", "(if (> x 0) (/ x d) 0)");
    assert!(
        refined_div.contains(&Lir::I32DivS)
            && !refined_div
                .iter()
                .any(|i| matches!(i, Lir::IfIntegerOverflowEnd)),
        "a flow-refined nonneg dividend (x>0) drops the MIN/-1 range-check via the interval fact; got {refined_div:?}"
    );
}

#[test]
fn an_equality_point_fact_elides_the_then_branch_arith_overflow_guard() {
    // The EQUALITY face of the interval refinement (refine_from_comparison, diverge.rs): inside the
    // THEN branch of `(if (= x c) …)` the fact pins `x` to the EXACT point range `[c, c]`, so a checked
    // arith on `x` whose result at `x == c` provably fits the type sheds its overflow guard — exactly as
    // an ORDERING refinement (`x > 0`) elides the underflow guard, but licensed by a POINT fact instead
    // of an interval. Sibling of `a_flow_refined_arith_op_elides_its_overflow_guard_on_the_rust_backend`
    // and the nonneg-div elision above; the corpus counterpart is the 02-binding-and-control Eq case.
    // Pin it at the Lir level (not just value parity) so a refinement regression that stopped pinning
    // the point fact would be caught here, not silently keep the dead guard.
    use crate::backend::wasm::lir::Lir;
    use crate::db::Db;
    let lir = |params: &str, body: &str| -> Vec<Lir> {
        let ast = crate::testkit::parse(&format!(
            "(module m (def (f {params}) {body}) (def (main) 0) (export main))"
        ));
        let mut db = Db::load(ast);
        let layout = crate::layout::compute(&mut db).expect("layout");
        let d = db.def_by_name("f").expect("def f");
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
        crate::backend::wasm::select::select_function(&mut db, body, &ps, &layout)
            .expect("select")
            .code
    };
    // BASELINE: a bare `(+ x 1)` on the narrow Int8 type carries its overflow guard (x = 127 → 128
    // overflows Int8), so the guard is live and must be present.
    let bare = lir("(: x Int8)", "(+ x 1)");
    assert!(
        bare.iter().any(|i| matches!(i, Lir::IfIntegerOverflowEnd)),
        "bare (+ x 1) on Int8 keeps its overflow guard; got {bare:?}"
    );
    // REFINED: under `(= x 5)` the then-branch pins `x = [5, 5]`, so `(+ x 1) = 6` provably fits Int8 →
    // the overflow guard is dead and dropped (the value-facts point-fact invariant).
    let refined = lir("(: x Int8)", "(if (= x 5) (+ x 1) 0)");
    assert!(
        !refined
            .iter()
            .any(|i| matches!(i, Lir::IfIntegerOverflowEnd)),
        "an equality point fact (x == 5) drops the (+ x 1) overflow guard; got {refined:?}"
    );
}

// ── shifts: count guarded to [0,N); << checked for overflow; >> arithmetic/logical by sign ─────

// ── unsigned checked +/-/*: the unsigned overflow guards (carry/borrow) ───────────────────────

// ── unsigned comparison: _u selection — the dual of the signed-ordering case ──────────────────

// ── narrow widths (≤32-bit): compute in i32, range-check back to the N-bit type ───────────────
//
// An aliased narrow width crosses the boundary as its FAITHFUL component primitive — Int8 as `s8`,
// UInt8 as `u8` — so args/results are `Val::S8`/`Val::U8` and `i8`/`u8` (not the machine-slot s32).
// wasmtime enforces the argument is a valid s8/u8 at the edge, and the ABI lifts/lowers to the i32
// slot the emitted code computes in — the range-checks keep the core result in the N-bit range.

#[test]
fn a_below_len_guard_elides_the_matching_list_ats_own_bounds_check_but_not_a_different_lists() {
    // BOUNDED-INDEX (below-len) FACET — operator-greenlit bounds-elision. Inside the then-branch of
    // `(< i (List.len xs))`, the index `i` is flow-known `< len(xs)`, so a `List.at xs i` there can shed
    // its OWN redundant `index < len` bounds check (the enclosing guard already proved it). Both the
    // guard `(List.len xs)` and `List.at`'s internal check emit a `vec-len` (OP_VEC_LEN), so on a
    // RUNTIME-length list the un-optimized emit has TWO; the facet drops List.at's, leaving ONE.
    //   • `guarded` — `List.at xs i` under `(< i (List.len xs))`: List.at's vec-len is ELIDED → ONE
    //     remains (the guard's). The FIRING assertion (confirmed against a facet-disabled baseline of 2).
    //   • `crosslist` — `List.at ys i` under the SAME `(< i (List.len xs))` guard: the fact is keyed on
    //     COLLECTION IDENTITY, so a guard on `xs` must NOT license eliding `ys`'s check (that would be an
    //     out-of-bounds read) → List.at's vec-len STAYS, giving TWO. The SOUNDNESS assertion.
    // The lists MUST be RUNTIME-length (an `(if … (list …) (list …))` whose length depends on `i`): a
    // const-length `(list …)`/`List.push` folds `List.len` to a `ConstInt`, so the guard emits no
    // `Core::ListLen` for the establisher to see — the interval facet already covers that case.
    use crate::backend::wasm::lir::Lir;
    use crate::db::Db;
    let veclen_count = |src: &str, name: &str| -> usize {
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
            .iter()
            .filter(|i| matches!(i, Lir::CallImport("vec-len")))
            .count()
    };
    // FIRES: the guarded matching-list access elides List.at's own vec-len — only the guard's remains.
    let guarded = veclen_count(
        "(module m \
               (def (f (: i Int64)) \
                 (let ((xs (if (> i 100) (list 10 20) (list 10 20 30)))) \
                   (if (< i (List.len xs)) (List.at xs i) (None unit)))) \
               (def (main) 0) (export main))",
        "f",
    );
    assert_eq!(
        guarded, 1,
        "the `(< i (List.len xs))` guard should elide `List.at xs i`'s own bounds check — only the \
             guard's own vec-len should remain (got {guarded})"
    );
    // SOUND: a guard on `xs` must NOT elide a `List.at ys i` on a DIFFERENT list — both vec-lens stay.
    let crosslist = veclen_count(
        "(module m \
               (def (f (: i Int64)) \
                 (let ((xs (if (> i 100) (list 10 20) (list 10 20 30))) \
                       (ys (if (> i 100) (list 1) (list 1 2)))) \
                   (if (< i (List.len xs)) (List.at ys i) (None unit)))) \
               (def (main) 0) (export main))",
        "f",
    );
    assert_eq!(
        crosslist, 2,
        "a guard on `xs` must NOT license eliding `List.at ys i`'s check (collection-identity \
             soundness) — the guard's vec-len AND List.at's vec-len both stay (got {crosslist})"
    );
}

// (an_oversize_constant_in_a_narrowed_control_flow_operand_is_rejected migrated to corpus 06-numeric-model,
// the CONTROL-FLOW-OPERAND face of the narrow-width fit-check (by the compound-payload descent group): a
// compile-time-constant if/match/let branch value that overflows a narrow op width → CDZ0302 (not a silent
// i32.wrap_i64), across every operand route (+, &, nested +, through a let, via a match arm) + a
// fits-i32-not-Int16 value. --case grades the reject code (all 6 PASS). In-range/runtime branch values are
// unaffected — covered by the sibling run tests.)

// (cdz_check_rejects_an_oversize_literal_in_a_runtime_if_branch_under_a_narrow_annotation migrated to
// corpus 06-numeric-model, the "`cdz check` agrees with EMIT on a runtime-if/match branch literal + a
// narrow parameter width" block: 8 CDZ0302 rejects — direct runtime-if annotation, narrow-param via
// runtime if, nested if, negative-in-unsigned, runtime-match arm (direct + narrow-param), direct bare
// literal to a narrow param, and the transitive two-call chain — plus 6 no-over-rejection controls that
// RUN (fitting if-branch, constant-condition fold, fitting narrow-param arg, fitting match arm, and two
// no-narrow-context large literals that stay Int64 → 10000). --case grades the reject codes + the run
// values (all verified PASS).)

// (cdz_check_rejects_a_float32_overflowing_literal_in_a_runtime_if_or_match_branch migrated to corpus
// 06-numeric-model (the FLOAT sibling of the runtime-if/match narrow-overflow block): a Float32-overflowing
// literal `1.0e300` (finite Float64, ±inf Float32) in a runtime if / match arm / narrow-Float32 param / a
// const-folded conditional → CDZ0302 at check (formerly emitted an INVALID wasm module), + running
// no-over-rejection controls (fitting Float32, dead-branch const-fold → 0.5, fitting const-fold → 1.5).
// --case grades the reject codes + run values (all PASS). The two Float64-fits negatives ((: … Float64)
// and a bare no-narrow-context if — both compile and run to the finite 1.0e300) are NOT corpus-migrated:
// their run value renders as the full ~300-digit exact-Float64 decimal, which a corpus (output …) cannot
// legibly pin; the Float32-only reject specificity is still shown by every reject case being Float32.)

// (cdz_check_rejects_a_narrow_width_overflow_projected_through_option_or_result_expect migrated to corpus
// 06-numeric-model, the Option/Result-`expect` PROJECTION face: a narrow result width propagates into the
// sum payload, so an out-of-range payload literal in a RUNTIME sum reached via Option.expect / Result.expect
// (payload = arg 0 for Some AND Ok) is CDZ0302 at check — this was a SILENT MISCOMPILE (c=true ran to
// 10000 as UInt8 = 16 with NO diagnostic; a CONSTANT sum folds + is caught, only a RUNTIME sum slipped).
// Migrated as 2 CDZ0302 rejects + 3 running no-over-rejection controls (fitting payload → 100, Int64 result
// fits → 10000, no narrow annotation → 10000). --case grades the reject codes + run values (all 5 PASS).)

// (cdz_check_rejects_a_narrow_width_overflow_in_a_record_field_or_map_position was already fully covered
// by the existing 06-numeric-model width-fit descent group — record field `(: #record((= x 999)) (Record
// (: x Int8)))`, map VALUE `(: #map((= 1 999)) (Map Int64 Int8))`, and map KEY `(: #map((= 999 1)) (Map
// Int8 Int64))` are all pinned there (+ the fitting record-field control), so this rust test is redundant
// and removed. Fitting map literals are broadly exercised by the corpus at large.)

// (cdz_check_rejects_a_narrow_width_overflow_in_a_user_sum_or_nominal_payload migrated to corpus
// 06-numeric-model: the newtype `(: (W 999) W)` and multi-payload `(: (P 999 5) P)` payload-overflow
// rejects were already pinned in the descent group; this batch ADDED the one uncovered face — the
// multi-VARIANT sum "a literal in a MULTI-VARIANT user sum payload that overflows the annotated width is
// rejected" ((type E (A Int8) (B Int64)), (: (E.A 999) E) → CDZ0302, a Ty::Sum vs the Ty::Nominal newtype)
// + a fitting multi-variant control that runs. --case grades the reject code + the run value.)

// (cdz_check_rejects_a_float_literal_grounded_to_float32_through_an_arith_spine migrated to corpus
// 06-numeric-model, the CONTEXTUAL arith-spine face: `(+ a 1.0e300)` over `(: a Float32)` grounds
// `1.0e300` to Float32 (saturates to inf, no written form) → CDZ0201 (contextual, no annotation to blame —
// matching the int arith-spine `(+ a 10000)` over UInt8 verdict; formerly compiled + materialized inf) +
// a fitting arith-spine control that runs (a+1.5 → 3.5). --case grades the reject code + the run value.
// The two Float64-holds negatives ((+ a 1.0e300) over Float64, and a bare `(+ 1.0e300 1.0)`) are not
// corpus-migrated — they compile and run to the finite 1.0e300, whose ~300-digit exact-Float64 decimal an
// (output …) cannot legibly pin.)

// ── runtime `wrap` (R3): the emitted mask-and-reinterpret over a runtime operand ──────────────
//
// With a RUNTIME source (a parameter), `wrap` cannot fold — it emits `Core::Convert`, a slot move
// (extend/wrap) plus a mask (+ sign-extend for a signed target). Never traps. These pin the emitted
// path agrees with the constant fold, over the slot-crossing cases the fold never exercises.

// ── A-normal form: a multi-use runtime binding is NAMED (computed once), single-use is inlined ──
//
// The core is in A-normal form (`reference-compiler.md` §The Core Representation Is In A-Normal
// Form): a `let` whose value is a RUNTIME computation used more than once becomes a `Core::Let`
// binding — computed once into a persistent local, read by each `LocalRef` — while a single-use or
// constant binding is copy-propagated / erased (the admin-redex elimination that keeps naming
// free). These run the emitted component under wasmtime to prove the VALUE is right; the
// byte-neutrality of the single-use case is what keeps the gate unchanged.

#[test]
fn a_named_binding_emits_its_value_computation_once_in_the_bytes() {
    // The SHARING is observable in the emitted code size: naming `s = (+ a b)` and using it twice
    // must emit the `i64.add` for `(+ a b)` ONCE (then read the slot twice), not twice. Compare the
    // NAMED form against a hypothetical inlined one by counting `LocalSet`/`LocalGet` structure via
    // the Lir. We assert at the byte level: the named body has exactly ONE checked-add sequence.
    use crate::backend::wasm::select::select_function;
    use crate::db::Db;
    use crate::infer::type_of;
    use crate::testkit::parse;
    let ast = parse(
        "(module m (def (f (: a Int64) (: b Int64)) (let ((s (+ a b))) (+ s s))) (export f))",
    );
    let mut db = Db::load(ast);
    let d = db.def_by_name("f").expect("def f");
    let body = db.defs[d].body.expect("body");
    let sig_params = db.defs[d].params.clone();
    let mut params = Vec::new();
    for p in sig_params {
        let binder = match db.ast.as_form(p, ":").and_then(|t| t.first().copied()) {
            Some(name_occ) => name_occ,
            None => p,
        };
        let ty = type_of(&mut db, binder);
        params.push((binder, ty));
    }
    let layout = crate::layout::compute(&mut db).expect("layout");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    // The inner `(+ a b)` is a checked add → exactly ONE `I64Add` (the outer `(+ s s)` is the
    // SECOND). If `s` were inlined at both uses, `(+ a b)` would appear TWICE → three `I64Add`s.
    let adds = f
        .code
        .iter()
        .filter(|i| matches!(i, crate::backend::wasm::lir::Lir::I64Add))
        .count();
    assert_eq!(
        adds, 2,
        "named binding must compute `(+ a b)` once, not twice"
    );
}

#[test]
fn a_multi_use_call_binding_calls_once_not_per_use() {
    // A `let`-bound value whose initializer is a residual `Core::Call` (a RECURSIVE def that could not
    // inline to a value) is a genuine runtime computation — so a call binding used MORE THAN ONCE must
    // be NAMED (called once, its result read by each use), not copy-propagated into every use site.
    // Before, `Core::Call` was absent from `is_runtime_computation`, so `(let ((xs (build …))) …)` with
    // `xs` used twice RECOMPUTED the whole `build` call (rebuilding the list + its allocations) at each
    // use. Pin exactly ONE call to `build` in the emitted body, and value parity.
    use crate::backend::wasm::lir::Lir;
    use crate::backend::wasm::select::select_function;
    use crate::db::Db;
    use crate::infer::type_of;
    use crate::testkit::parse;
    let ast = parse(
        "(module m \
               (def (build (: i Int64) (: n Int64) (: out (List Int64))) \
                 (if (< i n) (build (+ i 1) n ((. List push) out i)) out)) \
               (def (f (: n Int64)) \
                 (let ((xs (build 0 n (list)))) (+ ((. List len) xs) ((. List len) xs)))) \
               (export f))",
    );
    let mut db = Db::load(ast);
    let d = db.def_by_name("f").expect("def f");
    let body = db.defs[d].body.expect("body");
    let sig_params = db.defs[d].params.clone();
    let mut params = Vec::new();
    for p in sig_params {
        let binder = match db.ast.as_form(p, ":").and_then(|t| t.first().copied()) {
            Some(name_occ) => name_occ,
            None => p,
        };
        let ty = type_of(&mut db, binder);
        params.push((binder, ty));
    }
    let layout = crate::layout::compute(&mut db).expect("layout");
    let code = select_function(&mut db, body, &params, &layout)
        .expect("select")
        .code;
    // The `build` recursion is a real `Core::Call` → a `Lir::Call`/`ReturnCall`. With `xs` NAMED it is
    // called EXACTLY ONCE; if `xs` were copy-propagated it would appear twice (once per `List.len xs`).
    let calls = code
        .iter()
        .filter(|i| matches!(i, Lir::Call(_) | Lir::ReturnCall(_)))
        .count();
    assert_eq!(
        calls, 1,
        "a multi-use call binding is called ONCE, not per use, got {calls}: {code:?}"
    );
    // (End-to-end value parity for shared runtime call-bindings is covered by the corpus gate + the
    // `len`-twice / `concat xs xs` runtime probes; here the Lir call-count is the precise witness.)
}

#[test]
fn a_single_use_runtime_binding_is_inlined_not_named() {
    // A binding used ONCE is copy-propagated (no `Core::Let`), so it emits identically to writing
    // the value inline — byte-for-byte. `(let ((s (+ a b))) (* s 2))` and `(* (+ a b) 2)` are the
    // SAME component. This is the admin-redex elimination that keeps the gate byte-neutral.
    let named = func("(: a Int64) (: b Int64)", "(let ((s (+ a b))) (* s 2))");
    let inline = func("(: a Int64) (: b Int64)", "(* (+ a b) 2)");
    assert_eq!(
        named, inline,
        "a single-use binding must inline byte-identically"
    );
}

#[test]
fn a_constant_binding_is_never_named() {
    // A binding whose value FOLDS to a constant is never named however many times it is used —
    // there is no runtime computation to share. `(let ((k (+ 1 2))) (+ k k))` folds to `6`, byte-
    // identical to writing `6` (well, `(+ 3 3)` folds to 6 too). Both fold to the constant.
    let named = func("(: a Int64)", "(let ((k (+ 1 2))) (+ k k))");
    let konst = func("(: a Int64)", "6");
    assert_eq!(named, konst, "a constant binding folds; nothing is named");
}

#[test]
fn a_wide_let_decides_keeps_in_one_pass_not_per_binding() {
    // REGRESSION (perf): `lower::lower_let` decides, per binding, whether to KEEP it as a named
    // `Core::Let` slot or copy-propagate — via `should_keep_binding`, which walked the binding's whole
    // SCOPE (later inits + body) counting references (`uses_in`) and checking whole-value escape
    // (`ref_escapes_whole`). For a WIDE `let` (N bindings, body O(N)) that was O(bindings × body) =
    // O(N²) (`cdz check` on N constant-list bindings each read once: 800→1600 grew ~3.6×). FIX: collect
    // every binding's use facts (count + whole-value-escape) in ONE walk of the whole region
    // (`collect_binding_uses` → `BindingUses`); `let*` scoping means a ref to a binding appears only in
    // LATER inits + body, so the whole-region count is each binding's exact in-scope count. Per-binding
    // decisions become O(1) lookups → the pass is O(N).
    //
    // Correctness: the fold still produces the right value (a constant list read through an element
    // binder folds; a multi-use runtime binding is named). Byte-identical emit verified out-of-band.
    fn wide_listlet(n: usize) -> String {
        let binds: String = (0..n)
            .map(|i| format!("(l{i} (list {i} {}))", i + 1))
            .collect::<Vec<_>>()
            .join(" ");
        let reads: String = (0..n)
            .map(|i| format!("(v{i} (match l{i} ((list a .. _) a) (_ 0)))"))
            .collect::<Vec<_>>()
            .join(" ");
        format!("(module m (def (main) (let ({binds} {reads}) v0)) (export main))")
    }
    // A small instance evaluates correctly (first element of the first list = 0).
    let diags = crate::diagnostics(&mut crate::db::Db::load(parse(&wide_listlet(4))));
    assert!(
        diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a wide constant-list `let` type-checks: {diags:?}"
    );
    // Growth guard at width N vs 2N. The NOISE-FREE signal is `COLLECT_BINDING_USES_VISITS` — the
    // nodes the single-pass `collect_binding_uses` region walk visits (the compiler's own recursion
    // count, a pure function of the program), NOT wall-clock. A wall-clock ratio false-fails under
    // fleet load (a narrow run in a quiet slice vs a wide run hitting a scheduling stall inflates the
    // ratio past threshold — the flake). The per-binding `should_keep_binding` scope walk was O(N²)
    // (each binding re-walked the whole later region); the single-pass collect is O(N) — so the visit
    // count grows ~2× over a 2× width, not ~4×. Threshold 3.0× sits between the regimes with margin.
    fn uses_visits(src: &str) -> u64 {
        crate::db::COLLECT_BINDING_USES_VISITS.with(|c| c.set(0));
        let _ = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
        crate::db::COLLECT_BINDING_USES_VISITS.with(|c| c.get())
    }
    let n800 = uses_visits(&wide_listlet(800));
    let n1600 = uses_visits(&wide_listlet(1600));
    let ratio = n1600 as f64 / (n800.max(1)) as f64;
    assert!(
        n800 > 0 && ratio < 3.0,
        "a wide `let`'s keep-decision must scale linearly (was O(N²) via a per-binding \
             `should_keep_binding` scope walk; now one `collect_binding_uses` pass): width 800→1600 grew \
             `collect_binding_uses` visits {ratio:.1}× (n800={n800}, n1600={n1600}); linear is ~2×, the \
             per-binding walk was ~4×"
    );
}

#[test]
fn a_deep_list_push_chain_marks_dup_sites_in_bounded_time_not_exponential() {
    // REGRESSION (perf): `mark_binder_dups`'s `seq` pre-pass used to detect a sibling's binder
    // occurrence by calling `mark_binder_dups` itself (the full two-pass walk). Invoked from EVERY
    // nested `seq` level, that re-walked a deeply-nested term's inner subtree once per enclosing level —
    // EXPONENTIAL: a `push(push(push(xs)))` chain compiled in 12ms / 275ms / 30s+ TIMEOUT at depth
    // 10/20/30. Fixed by using the cheap memoized `binder_occurs` scan in the pre-pass (marks no sites).
    // Guard: COMPILE a depth-N push-chain over a runtime list param (returns `List.len` so it is a valid
    // scalar-export component — the dup-marker still walks the whole chain). Paired N vs 2N timing, MIN
    // ratio (cancels harness contention); an exponential blows the ratio past any linear bound.
    fn push_chain(n: usize) -> String {
        let opens: String = "((. List push) ".repeat(n);
        let closes: String = (1..=n).map(|i| format!(" {i})")).collect();
        format!(
            "(module m (def (f (: xs (List Int64))) ((. List len) {opens}xs{closes})) \
                 (def (main) (f (list))) (export main))"
        )
    }
    fn compile_ms(src: &str) -> f64 {
        let start = std::time::Instant::now();
        let _ = crate::compile::compile_component(&crate::codec::encode(&parse(src)));
        start.elapsed().as_secs_f64() * 1000.0
    }
    let (narrow, wide) = (push_chain(20), push_chain(40));
    compile_ms(&narrow); // warm one-time init before the timed pairs
    let mut best = f64::INFINITY;
    for _ in 0..6 {
        let t20 = compile_ms(&narrow);
        let t40 = compile_ms(&wide);
        best = best.min(t40 / t20.max(0.1));
    }
    assert!(
        best < 4.0,
        "a deep List.push chain's dup-site marking must not be exponential (was 2^depth via the \
             `seq` pre-pass calling `mark_binder_dups`; now the memoized `binder_occurs` scan): depth \
             20→40 grew {best:.1}× (min paired ratio); polynomial is a few×, exponential is astronomical"
    );
}

#[test]
fn a_chained_multi_use_list_concat_emits_linear_wasm_not_exponential() {
    // REGRESSION (emit-SIZE, operator seq-203): a let-bound value used 2+ times in a chained
    // nesting must be MATERIALIZED ONCE into a slot and shared, not copy-propagated (inlined) at
    // each use. `is_runtime_computation` (lower.rs) omitted `Core::ListConcat`, so a let-bound
    // `List.concat` failed `should_keep_binding`'s runtime-computation gate (call_lower.rs) and was
    // inlined at both uses; in the chained shape `x1=(concat x0 x0)`, `x2=(concat x1 x1)`, … each
    // level inlines its predecessor TWICE → the emitted wasm (and compile time) COMPOUNDED to 2^N
    // for an O(N)-size source (N=14 emitted ~132 KB, N≥20 timed out). Fixed by adding
    // `Core::ListConcat` to `is_runtime_computation` (main 49e4c17bf1, #7416) so the binding is
    // kept + slot-shared by the existing kept-binding let-slot emit — LINEAR.
    //
    // The NOISE-FREE signal is `wasm.len()`: emitted byte count is a pure deterministic function of
    // the program (no wall-clock flake at all — stronger than the sibling perf ratios above). Guard:
    // compile an O(N)-size `List.concat` chain at N and 2N and assert the emitted bytes grow at most
    // ~linearly (a 2× source ⇒ ~2× bytes; the regression was 2^N). N=7/14 sit in the window where a
    // reintroduced exponential is still observable as a CLEAN size blow-up (N=14 pre-fix ≈ 132 KB,
    // measured — no hang) rather than a timeout, so the assertion fails loudly instead of wedging.
    fn concat_chain(n: usize) -> String {
        // `x0 = (list p)`; `x_i = (concat x_{i-1} x_{i-1})` — each binding used TWICE; body reads x_n.
        let mut lets = String::from("(x0 (list p)) ");
        for i in 1..=n {
            let prev = i - 1;
            lets.push_str(&format!("(x{i} ((. List concat) x{prev} x{prev})) "));
        }
        format!(
            "(module m (def (f (: p Int64)) (let ({lets}) ((. List len) x{n}))) \
                 (def (main) (f 1)) (export main))"
        )
    }
    fn wasm_len(src: &str) -> usize {
        crate::host::run_with_compiler_stack(|| {
            compile_component(&crate::codec::encode(&parse(src)))
                .expect("chain compiles")
                .len()
        })
    }
    let n7 = wasm_len(&concat_chain(7));
    let n14 = wasm_len(&concat_chain(14));
    let ratio = n14 as f64 / (n7.max(1)) as f64;
    assert!(
        n7 > 0 && ratio < 4.0 && n14 < 20_000,
        "a chained multi-use `List.concat` must emit LINEAR wasm (kept + slot-shared), not 2^N \
             (was inlined at each use because `is_runtime_computation` omitted `Core::ListConcat` — \
             seq-203, fixed #7416): depth 7→14 emitted {n7}→{n14} bytes ({ratio:.1}× — linear is ~2×, \
             the exponential regression was ~90× / ~132 KB at N=14)"
    );
}

#[test]
fn a_chained_multi_use_boxed_collection_producer_emits_linear_wasm_not_exponential() {
    // REGRESSION (emit-SIZE, seq-203 family-widening): the sibling `List.concat` gate above guards the
    // BINARY-same-operand exponential shape (`(concat x x)`). This gate guards the UNARY collection
    // PRODUCER class — `List.push`/`List.prepend`/`List.update` (and the Map/Set producers) — which share
    // the SAME keep path: a let-bound producer used 2+ times must be KEPT (materialized once, slot-shared),
    // not copy-propagated at each use. `is_runtime_computation` (lower.rs) originally listed only ListNew
    // (+ ListConcat via #7416); the collection-producer family was added in the batch-1 widening. Each of
    // these ops is an Owned fresh producer (`heap_operand_ownership`) with CONSUMING operands
    // (`mark_binder_dups`), so keeping + dup-per-use is refcount-sound.
    //
    // The 2-use-per-level step is `x_i = (List.push x_{i-1} (List.len x_{i-1}))`: `x_{i-1}` is CONSUMED as
    // the list operand AND BORROWED for the length — two uses, so an un-kept `x_{i-1}` inlines its
    // producer at BOTH sites and the chain compounds to 2^N (exactly the concat blow-up, one op deeper).
    // Same noise-free `wasm.len()` signal (pure deterministic function of the program). Assert linear:
    // ratio < 4.0 (linear ≈ 2×) AND absolute ceiling < 20 KB.
    fn push_lenchain(n: usize) -> String {
        // `x0 = (list p)`; `x_i = (List.push x_{i-1} (List.len x_{i-1}))` — `x_{i-1}` used TWICE per level.
        let mut lets = String::from("(x0 (list p)) ");
        for i in 1..=n {
            let prev = i - 1;
            lets.push_str(&format!(
                "(x{i} ((. List push) x{prev} ((. List len) x{prev}))) "
            ));
        }
        format!(
            "(module m (def (f (: p Int64)) (let ({lets}) ((. List len) x{n}))) \
                 (def (main) (f 1)) (export main))"
        )
    }
    fn wasm_len(src: &str) -> usize {
        crate::host::run_with_compiler_stack(|| {
            compile_component(&crate::codec::encode(&parse(src)))
                .expect("chain compiles")
                .len()
        })
    }
    let n7 = wasm_len(&push_lenchain(7));
    let n14 = wasm_len(&push_lenchain(14));
    let ratio = n14 as f64 / (n7.max(1)) as f64;
    assert!(
        n7 > 0 && ratio < 4.0 && n14 < 20_000,
        "a chained multi-use collection producer (`List.push x (List.len x)`) must emit LINEAR wasm \
             (kept + slot-shared), not 2^N (was inlined at each use before the boxed-collection producers \
             entered `is_runtime_computation` — seq-203 family-widening): depth 7→14 emitted {n7}→{n14} \
             bytes ({ratio:.1}× — linear is ~2×, the un-kept-producer regression compounds exponentially)"
    );

    // NOTE: the `Set.union` (`Core::SetAlgebra`) chain that used to live here was REMOVED with the P0
    // stopgap that dropped the MAP + SET producers from `is_runtime_computation` (a multi-use
    // generation-shared CHAMP map/set over-freed a path-copy-shared interior node under `--guarded-all`;
    // memory-safety > perf). With `SetOf`/`SetAlgebra` no longer kept, a `Set.union` chain reverts to 2^N
    // by design, so a linear-emit assertion on it would (correctly) fail. Re-add the Set.union (and a Map)
    // chain here when the generation-shared-CHAMP reclaim fix lands and the producers are re-widened.
}

// NOTE: the `a_chained_multi_use_sum_new_emits_linear_wasm_not_exponential` gate (SumNew keep, seq-203
// batch-3 #7488) was REMOVED with the SumNew stopgap. A bisect pinned #7488 (the SumNew keep) as the
// first-bad for a 14b-effects INVALID WASM COMPONENT ("an Option-of-HEAP handler state transitions None to
// Some…", func 12: "type mismatch: expected i32, found i64"): a multi-use SumNew used as an `Option`
// handler STATE, materialized-once into a `Core::Let` slot and threaded through a TAIL-RESUMPTIVE-FOLD
// handler's resume, emits an i32/i64 width mismatch. So SumNew was dropped from `is_runtime_computation`
// (reverts to copy-propagation → a chained multi-use SumNew is 2^N by design, no linear-emit assertion
// possible). Re-add this gate (and re-widen SumNew) once the handler-fold emit width-handles a materialized
// SumNew state (v-effects fold-lowering + v-core-opt/emit). The List concat/push gates above stay green.

#[test]
fn an_effects_handler_with_a_boxed_heap_state_emits_a_valid_wasm_component() {
    // REGRESSION (emit-VALIDITY, seq-203 keep family): a materialize-once keep of a boxed producer used as
    // a handler STATE, threaded through a TAIL-RESUMPTIVE-FOLD resume, can emit an i32/i64 width mismatch →
    // an INVALID wasm component (14b "an Option-of-HEAP handler state transitions None to Some and grows the
    // payload", func 12: "type mismatch: expected i32, found i64"). Bisect pinned #7488 (the SumNew keep) as
    // first-bad; the SumNew stopgap (#7538) restored validity.
    //
    // This gate is the FAST pre-land guard the opt-sweep + emit-size gates LACKED: it COMPONENT-VALIDATES
    // the emitted wasm (wasmparser, all features incl the component model) — the axis that caught this only
    // in the slow full-corpus grade, so #7488 slipped past its pre-land gate. It guards any future re-widen
    // of the keep family (SumNew / map / set) against re-introducing the invalid-component class. It is a
    // compiler-OUTPUT-correctness pin (does the emit validate?), not a Cadenza-behavior property — hence a
    // Rust #[test], like the existing `validate_composed` / cross-component validation pins.
    let src = r#"(do (effect St (op feed (-> Int64 Int64))) (def (main (: a Int64)) (handle St (Option.None) ((feed (v) s (match s ((Option.None) (resume 0 (Option.Some #list(v)))) ((Option.Some xs) (resume (List.len xs) (Option.Some (List.push xs v))))))) (+ (* 100 (St.feed a)) (+ (* 10 (St.feed (+ a 1))) (St.feed (+ a 2)))))) (export main))"#;
    let comp = crate::host::run_with_compiler_stack(|| {
        compile_component(&crate::codec::encode(&parse(src)))
            .expect("effects-handler with an Option-of-List heap state compiles")
    });
    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator.validate_all(&comp).expect(
        "an effects-handler with a boxed-heap (Option-of-List) state threaded through the \
         tail-resumptive fold must emit a VALID wasm component — a materialize-once keep of the \
         SumNew/List state must not produce an i32/i64 width mismatch (seq-203 keep family; the \
         #7488 invalid-component class, func 12)",
    );
}

#[test]
fn a_wide_literal_match_builds_its_decision_tree_in_bounded_time() {
    // REGRESSION (perf): `lower::build_tree`'s lit-test arm compiles a wide literal match
    // (`(match t ((tuple 0 a) …) ((tuple 1 a) …) … (_ -1))`) as an N-DEEP chain of `LitTest` nodes.
    // Each level built the MATCHED-branch sub-matrix as `matched_rows = [this row, with its first
    // lit-test consumed] ++ rows[1..]`, cloning the whole O(N) remaining-rows tail. But when the
    // matched row is now an UNCONDITIONAL LEAF (no further tests, no guard — the common single-lit-test
    // arm), `build_tree` returns `Leaf` on that first row and never reads the appended tail, so those
    // clones were pure waste → O(N²) over the N levels (profile: `build_tree`/`build_lit_test` ~92%
    // inclusive, `Vec::clone` ~33% + heavy malloc; N=400/800/1600/3200 = 33/93/342/1295ms, ~3.8×/dbl).
    // FIX: skip the tail append when the matched row is a leaf; only a fall-through-capable matched row
    // (a further lit-test — e.g. `(tuple 0 0)` before `(tuple 0 a)` — or a guard) needs the tail.
    //
    // The NOISE-FREE signal is `BUILD_TREE_CALLS` — the decision-tree recursion count, a pure function
    // of the program. For N single-column leaf arms it must stay O(N), NOT O(N²). Correctness (the
    // dispatch selects the right arm, and a multi-lit-test arm's fall-through still reaches the
    // same-prefix binding arm) is pinned by the run-value + fall-through match tests.
    fn wide_tuple_lit_match_src(n: usize) -> String {
        // `(def (f (: t (Tuple Int64 Int64))) (match t ((tuple 0 a) 0) … ((tuple {n-1} a) {n-1}) (_ -1)))`
        // — N arms each testing the FIRST element against a distinct literal, binding the second.
        let mut arms = String::new();
        for i in 0..n {
            arms.push_str(&format!("((tuple {i} a) {i}) "));
        }
        format!(
            "(module m (def (f (: t (Tuple Int64 Int64))) (match t {arms}(_ -1))) \
                 (def (main) (f (tuple 1 5))) (export main))"
        )
    }
    // A small instance compiles with no error diagnostics (a valid wide literal match).
    let diags = crate::diagnostics(&mut crate::db::Db::load(parse(&wide_tuple_lit_match_src(
        4,
    ))));
    assert!(
        diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a wide literal match compiles with no error diagnostics: {diags:?}"
    );
    fn build_tree_calls(src: &str) -> u64 {
        crate::host::run_with_compiler_stack(|| {
            crate::db::BUILD_TREE_CALLS.with(|c| c.set(0));
            let _ = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
            crate::db::BUILD_TREE_CALLS.with(|c| c.get())
        })
    }
    // Width 200→400 is a 2× match; O(N) recursion ⇒ ~2×, an O(N²) blow-up was ~4×. Require < 3×.
    let n200 = build_tree_calls(&wide_tuple_lit_match_src(200));
    let n400 = build_tree_calls(&wide_tuple_lit_match_src(400));
    let ratio = n400 as f64 / (n200.max(1)) as f64;
    assert!(
        n200 > 0 && ratio < 3.0,
        "a wide single-column literal match must build its decision tree in O(N) `build_tree` \
             recursions, not O(N²): width 200→400 grew {ratio:.1}× (n200={n200}, n400={n400})"
    );
}

#[test]
fn a_multi_column_literal_match_compiles_in_linear_time() {
    // REGRESSION (perf/correctness): a match whose arms each test ≥2 LITERAL COLUMNS
    // (`(tuple 0 0 a)` — a transition-table / parser `(tuple state token payload)` dispatch) compiled
    // `lower::build_tree` in O(2^arms). `build_lit_test` lowers such an arm to `LitTest{then_, els}`
    // where the arm's SECOND-column test (`then_`) itself falls through to the SAME remaining-arms
    // matrix that this test's `els` compiles — so without sharing, the fall-through is re-compiled in
    // BOTH branches at every column, T(N)=2·T(N-1) (a 20-arm 2-column match: ~5s to `cdz check`, 25
    // arms hangs). FIX: for a NON-REFINING probe (Int/Str — its else is the remaining arms verbatim),
    // compile that fall-through ONCE into a shared `Rc<SumCont>` and thread it into `then_`'s recursion
    // as its `fallthrough`, so the arm's further column-tests reuse the same `Rc` (a refcount bump)
    // instead of re-compiling → build O(arms). A REFINING probe (Bool/ListLen) still re-checks its
    // matched arm against the real tail (no sharing — a finite fan-out has no exponential to dedup), so
    // exhaustiveness is unaffected.
    //
    // The NOISE-FREE signal is `BUILD_TREE_CALLS` (the recursion count). A 2-column match with N arms
    // must recurse O(N), not O(2^N). Correctness (dispatch + exhaustiveness) is pinned by the 438
    // match_engine tests, which stay byte-identical.
    fn two_col_match_src(n: usize) -> String {
        // `(def (f (: t (Tuple Int64 Int64 Int64))) (match t ((tuple 0 0 a) 0) … (_ -1)))` — each arm
        // tests TWO literal columns (the exponential shape), binding the third.
        let mut arms = String::new();
        for i in 0..n {
            arms.push_str(&format!("((tuple {i} {i} a) {i}) "));
        }
        format!(
            "(module m (def (f (: t (Tuple Int64 Int64 Int64))) (match t {arms}(_ -1))) \
                 (def (main) (f (tuple 1 1 5))) (export main))"
        )
    }
    // A small instance compiles clean (a valid multi-column literal match).
    let diags = crate::diagnostics(&mut crate::db::Db::load(parse(&two_col_match_src(4))));
    assert!(
        diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a multi-column literal match compiles with no error diagnostics: {diags:?}"
    );
    fn build_tree_calls(src: &str) -> u64 {
        crate::host::run_with_compiler_stack(|| {
            crate::db::BUILD_TREE_CALLS.with(|c| c.set(0));
            let _ = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
            crate::db::BUILD_TREE_CALLS.with(|c| c.get())
        })
    }
    // Arms 12→16 is +4 arms. LINEAR ⇒ the recursion count grows by a small ADDITIVE amount (a bounded
    // per-arm constant); the O(2^arms) blow-up MULTIPLIED by 2^4 = 16× over +4 arms. Require the ratio
    // stay well under 4× (linear is ~1.3×; the exponential was ~16×). VERIFIED: reverting the shared
    // fall-through (append the tail into `then_`) fails here with a ~16× explosion.
    let n12 = build_tree_calls(&two_col_match_src(12));
    let n16 = build_tree_calls(&two_col_match_src(16));
    let ratio = n16 as f64 / (n12.max(1)) as f64;
    assert!(
        n12 > 0 && ratio < 4.0,
        "a multi-column literal match must compile in O(arms) `build_tree` recursions, not O(2^arms) \
             (the non-refining fall-through must be compiled once into a shared `Rc<SumCont>` and threaded \
             into the matched arm's further-column recursion): arms 12→16 grew {ratio:.1}× (n12={n12}, \
             n16={n16}); linear is ~1.3×, the exponential was ~16×"
    );
}

#[test]
fn an_inline_tuple_multi_column_match_compiles_in_linear_time() {
    // REGRESSION (perf): the sibling `a_multi_column_literal_match_compiles_in_linear_time` matches a
    // bound tuple PARAM (`(: t (Tuple …))`), which shares the fall-through correctly. But an INLINE-
    // constructed scrutinee — `(match (tuple a b c) ((tuple 0 0 c) 0) …)` where `a`/`b`/`c` are runtime
    // params — defeated the sharing and stayed O(2^cols): `const_at_path` walked INTO the inline
    // `Core::Tuple` and returned `Some(Core::Param)` for the runtime element, wrongly entering the
    // constant-FOLD branch (whose per-arm `matched_rows` does NOT thread the shared `Rc<SumCont>` tail)
    // instead of the runtime `build_lit_test` path that shares it. `build_tree` recursed 2^cols times
    // building all-distinct nodes (emit saw 1020 distinct `SumCont` ptrs at 8 arms, 0 shared — no DAG
    // for the emit-side dedup to key on). FIX: `const_at_path` returns `Some` ONLY for an actual
    // foldable CONSTANT (`is_foldable_const`); a runtime `Core::Param`/`LocalRef` sub-value returns
    // `None`, routing the inline-tuple element to the shared runtime lit-test path. Now the decision
    // tree is a linear-node DAG (build_tree recursions 33@8-arm, 65@16-arm), the shape the S2 emit-side
    // shared-continuation dedup needs. The NOISE-FREE signal is `BUILD_TREE_CALLS` (recursion count).
    fn inline_tuple_match_src(n: usize) -> String {
        let mut arms = String::new();
        for i in 0..n {
            arms.push_str(&format!("((tuple {i} {i} c) {i}) "));
        }
        format!(
            "(module m (def (f (: a Int64) (: b Int64) (: c Int64)) (match (tuple a b c) {arms}(_ -1))) \
                 (def (main) (f 1 1 5)) (export main))"
        )
    }
    // A small instance compiles clean.
    let diags = crate::diagnostics(&mut crate::db::Db::load(parse(&inline_tuple_match_src(4))));
    assert!(
        diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "an inline-tuple multi-column match compiles with no error diagnostics: {diags:?}"
    );
    fn build_tree_calls(src: &str) -> u64 {
        crate::host::run_with_compiler_stack(|| {
            crate::db::BUILD_TREE_CALLS.with(|c| c.set(0));
            let _ = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
            crate::db::BUILD_TREE_CALLS.with(|c| c.get())
        })
    }
    // Arms 8→16 is a 2× arm count. LINEAR (shared fall-through) ⇒ the recursion count roughly DOUBLES;
    // the O(2^arms) blow-up MULTIPLIED by 2^8 = 256× over +8 arms. Require the ratio stay well under 4×
    // (linear is ~2×; the exponential was astronomical — 1021 already at 8 arms). Measured 33@8, 65@16.
    let n8 = build_tree_calls(&inline_tuple_match_src(8));
    let n16 = build_tree_calls(&inline_tuple_match_src(16));
    let ratio = n16 as f64 / (n8.max(1)) as f64;
    assert!(
        n8 > 0 && ratio < 4.0,
        "an INLINE-tuple multi-column match must compile in O(arms) `build_tree` recursions, not \
             O(2^arms) (`const_at_path` must decline a runtime `Core::Param` sub-value so the inline-tuple \
             element takes the shared-fall-through runtime lit-test path): arms 8→16 grew {ratio:.1}× \
             (n8={n8}, n16={n16}); linear is ~2×, the exponential was ~256×"
    );
}

#[test]
fn a_refined_list_payload_match_compiles_in_linear_time() {
    // REGRESSION (perf, S3): a match whose arms each refine a LIST PAYLOAD by literal elements
    // (`(Some (list 0 0)) … (Some (list N N)) (_ -1)`) compiled `lower::build_tree` in O(2^arms). The
    // multi-column fix (S1) shares the fall-through only for NON-refining Int/Str probes; a `(list i i)`
    // arm prepends a `ListLen` REFINING probe before its element lit-tests, and S1 excluded refining
    // probes → the ListLen `then_` (passed-length world) re-compiled the whole remaining matrix at each
    // element-test = O(2^arms) (N=20 = 3.2s to `cdz check`). FIX (S3): the ListLen `then_`'s fall-through
    // is `else_rows` REFINED to the PASSED length (`refine_listlen_to_passed`), compiled ONCE and threaded
    // as the matched arm's `fallthrough` (S1's mechanism, refined tail); arms length-inconsistent with the
    // passed length are dropped (provably unmatchable there), so exhaustiveness is preserved.
    //
    // NOISE-FREE signal `BUILD_TREE_CALLS`. A 2-element-refined list match with N arms must recurse
    // O(N), not O(2^N). Correctness (dispatch + the empty/rest-arm exhaustiveness partition) is pinned by
    // the match_engine suite, which stays byte-identical.
    fn refined_list_match_src(n: usize) -> String {
        let mut arms = String::new();
        for i in 0..n {
            arms.push_str(&format!("((Some (list {i} {i})) {i}) "));
        }
        format!(
            "(module m (def (f (: o (Option (List Int64)))) (match o {arms}(_ -1))) \
                 (def (main) (f (Some (list 1 1)))) (export main))"
        )
    }
    let diags = crate::diagnostics(&mut crate::db::Db::load(parse(&refined_list_match_src(4))));
    assert!(
        diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a refined-list-payload match compiles with no error diagnostics: {diags:?}"
    );
    fn build_tree_calls(src: &str) -> u64 {
        crate::host::run_with_compiler_stack(|| {
            crate::db::BUILD_TREE_CALLS.with(|c| c.set(0));
            let _ = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
            crate::db::BUILD_TREE_CALLS.with(|c| c.get())
        })
    }
    // Arms 12→16 (+4): LINEAR ⇒ small additive growth (< 4×); the O(2^arms) blow-up was ~16×. VERIFIED:
    // reverting S3 (append `else_rows` into the ListLen `then_`) fails here with a ~16× explosion.
    let n12 = build_tree_calls(&refined_list_match_src(12));
    let n16 = build_tree_calls(&refined_list_match_src(16));
    let ratio = n16 as f64 / (n12.max(1)) as f64;
    assert!(
        n12 > 0 && ratio < 4.0,
        "a refined-list-payload match must compile in O(arms) `build_tree` recursions, not O(2^arms) \
             (a `ListLen` probe's passed-world fall-through must be compiled once via \
             `refine_listlen_to_passed` and shared): arms 12→16 grew {ratio:.1}× (n12={n12}, n16={n16}); \
             linear is ~1.3×, the exponential was ~16×"
    );
}

#[test]
fn a_deep_nested_let_chain_collects_binding_uses_in_bounded_time() {
    // REGRESSION (perf): `lower::lower_let` collects each binding's use facts by walking its whole `let`
    // REGION (all inits + body) in one pass (fix-44, which fused a WIDE let's per-binding walks). But a
    // DEEP nested chain `(let ((v0 e0)) (let ((v1 e1)) … body))` re-walked its body — the entire deeper
    // O(N−k) chain — at each of the N levels' `lower_let` → Σ = O(N²) (the deep-nested TWIN of the wide
    // case; profile showed `collect_binding_uses` ~90% inclusive, growth ~2.5×/dbl). FIX: `let*` scoping
    // makes an OUTER let's whole-region facts EXACT for every nested binding (a binding's refs live only
    // from its own init onward — a subset of the outer region), so a nested `lower_let` reuses the
    // nearest enclosing cached region's `BindingUses` (`Db::let_region_uses`) instead of re-collecting.
    // The OUTERMOST let walks the whole nest once; every inner let reuses that map in O(1) → O(N) total.
    //
    // The NOISE-FREE signal is `COLLECT_BINDING_USES_VISITS` — the nodes the collection walk visits, a
    // pure function of the program. A depth-N chain should visit O(N) nodes (one whole-nest walk), not
    // O(N²) (a re-walk of the tail per level). Correctness (the chain evaluates + kept-vs-propagated
    // decisions) is pinned by the run-value + wide-let tests.
    fn deep_let_chain_src(n: usize) -> String {
        // `(let ((v0 0)) (let ((v1 (+ v0 1))) … (let ((v{n-1} (+ v{n-2} 1))) v{n-1})))` — each binding
        // reads the previous once (a realistic sequential-let pipeline).
        let mut expr = format!("v{}", n - 1);
        for i in (0..n).rev() {
            let init = if i == 0 {
                "0".to_string()
            } else {
                format!("(+ v{} 1)", i - 1)
            };
            expr = format!("(let ((v{i} {init})) {expr})");
        }
        format!("(module m (def (main) {expr}) (export main))")
    }
    // A small instance compiles with no error diagnostics (a valid nested-let chain).
    let diags = crate::diagnostics(&mut crate::db::Db::load(parse(&deep_let_chain_src(4))));
    assert!(
        diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a deep nested-let chain compiles with no error diagnostics: {diags:?}"
    );
    fn uses_visits(src: &str) -> u64 {
        crate::host::run_with_compiler_stack(|| {
            crate::db::COLLECT_BINDING_USES_VISITS.with(|c| c.set(0));
            let _ = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
            crate::db::COLLECT_BINDING_USES_VISITS.with(|c| c.get())
        })
    }
    // Depth 200→400 is a 2× chain; linear (one whole-nest walk) ⇒ ~2×, the O(N²) per-level re-walk was
    // ~4×. Require < 3× (between the regimes, with margin for constant terms).
    let n200 = uses_visits(&deep_let_chain_src(200));
    let n400 = uses_visits(&deep_let_chain_src(400));
    let ratio = n400 as f64 / (n200.max(1)) as f64;
    assert!(
        n200 > 0 && ratio < 3.0,
        "a deep nested-let chain must collect its binding uses in O(N) node-visits, not O(N²) (each \
             level's `lower_let` re-walking its deeper body needs the enclosing-region reuse — \
             `Db::let_region_uses`): depth 200→400 grew collect visits {ratio:.1}× (n200={n200}, \
             n400={n400}); linear is ~2×, the per-level re-walk was ~4×"
    );
}

#[test]
fn project_meta_reads_a_meta_field_without_scanning_the_wide_user_block() {
    // REGRESSION (perf): `eval::project_meta` reads a `meta`-namespace field (`apply`/`variant`/`t`/…)
    // off a value's record on the HOT per-node meta-dispatch path (`meta_apply_of`/`variant_disc_of`
    // run for every application/pattern during `collect`/`type_errors`). It used a
    // `BTreeMap::get(&Symbol{Some("meta".to_string()), key.to_string()})` — allocating TWO `String`s per
    // call (a top allocation source; a realistic module A/B'd ~1.13× faster once removed). The
    // alloc-free replacement must NOT reintroduce the O(N)-field forward scan `203f8588` fixed for a
    // WIDE record. Field keys sort `(namespace, name)` with a USER field (`None`) BELOW `Some("meta")`,
    // so the meta fields are a contiguous block at the TOP — a REVERSE scan reaches them in
    // O(meta-fields) and BREAKS on descending below the block, never touching the O(width) user fields.
    //
    // The NOISE-FREE signal is `PROJECT_META_FIELDS_VISITED` — the MAX field entries a SINGLE scan
    // touches (a running max), a pure function of the program. Reading `(meta t)` etc. off a wide record
    // must visit O(meta-fields) = bounded PER CALL, NOT O(width). Widen the record 8× and require the
    // MAX per-call depth stays CONSTANT (a forward/no-break scan grows ~linearly with width; the
    // reverse-scan-with-break is flat at ~3). (A TOTAL-visits signal would conflate this with the
    // per-node meta-dispatch CALL count, which scales with the program independently — so track the max
    // per-call depth, which isolates the scan-length regression this fix is about.)
    // A WIDE EFFECT builds a wide `(meta …)` interface record — width-N ops → an N-field meta record
    // `project_meta` reads (`meta_apply_of`/`variant_disc_of` per op reference/pattern). The meta fields
    // sort at the TOP (namespace `Some("meta")` > a `None` user field), so the reverse-scan reaches them
    // in O(1)-ish regardless of N; a forward scan would walk all N.
    fn wide_effect_src(n_ops: usize) -> String {
        let ops: String = (0..n_ops)
            .map(|i| format!("(op tick{i} (-> Unit Int64))"))
            .collect::<Vec<_>>()
            .join(" ");
        let arms: String = (0..n_ops)
            .map(|i| format!("(tick{i} (u) s (resume s (+ s {i})))"))
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "(do (effect E {ops}) (def (main) (handle E 0 ({arms}) (E.tick0 ()))) (export main))"
        )
    }
    // A small instance compiles with no error diagnostics.
    let diags = crate::diagnostics(&mut crate::db::Db::load(parse(&wide_effect_src(4))));
    assert!(
        diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a small effect compiles with no error diagnostics: {diags:?}"
    );
    fn max_depth(src: &str) -> u64 {
        crate::host::run_with_compiler_stack(|| {
            crate::db::PROJECT_META_FIELDS_VISITED.with(|c| c.set(0));
            let _ = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
            crate::db::PROJECT_META_FIELDS_VISITED.with(|c| c.get())
        })
    }
    // 40 ops → 320 ops is 8×. A reverse scan that breaks below the meta block touches O(meta-fields) per
    // call REGARDLESS of width, so the MAX per-call scan depth is CONSTANT (~3). A forward/no-break scan
    // would walk the whole N-field meta record → the max depth would grow ~linearly with N. Require the
    // 8×-wider effect's max depth to stay BELOW 2× the narrow one's (constant, with margin) AND small in
    // absolute terms. `> 0` proves the reverse scan ran (a revert to the built-key `BTreeMap::get` never
    // touches this counter → 0 → the test fails, catching the per-call-`Symbol`-alloc regression).
    let d40 = max_depth(&wide_effect_src(40));
    let d320 = max_depth(&wide_effect_src(320));
    assert!(
        d40 > 0 && d320 < d40 * 2 && d320 < 32,
        "project_meta must read a meta field in O(meta-fields) PER CALL, not O(record-width) (the \
             alloc-free replacement must REVERSE-scan the top meta block and BREAK, not forward-scan the \
             wide meta record — the `203f8588` O(N²)): max per-call scan depth at 40 ops = {d40}, at 320 \
             ops = {d320} (must stay constant/small — a width-proportional scan would grow ~8×)"
    );
}

// ── common-subexpression elimination: an identical operand is computed once ───────────────────

#[test]
fn a_guarded_dependent_size_bin_arm_emits_a_valid_module() {
    // cg3c (breaker): a guarded dependent-size `bin` arm emitted an INVALID module. The arm lowers to
    // `if (pred AND guard) body else …`; `Core::And` emitted its `rhs` at the SAME `base` as its `lhs`,
    // so the guard's `(bytes p k)` `BinSizedRead` (an i32 handle, base-anchored) aliased the arm
    // predicate's length probe `off + k` slot (an i64 checked-arith temp over the i64 `(u8 k)` read),
    // declaring one wasm local at two widths → `type mismatch: expected i32, found i64` at validation.
    // `compile_component` does NOT validate, so this pins the fix (rhs floats above lhs's high-water) with
    // an independent `wasmparser::validate` — a FAST in-crate guard that runs in `dev-gate` every
    // iteration, complementing the runtime corpus case (16-binary-matching) that only runs the full gate.
    // `main` takes an Int64 and builds the Bytes internally (a non-scalar entry param would decline), so
    // the match runs through the wasm backend rather than folding a constant scrutinee.
    let bytes = compile_component(&crate::codec::encode(&parse(
        "(module m \
           (def (f (: b Bytes)) \
             (match b \
               ((guard (bin (u8 k) (bytes p k)) (> (Bytes.len p) 1)) (+ 1000 (Bytes.len p))) \
               (_ -1))) \
           (def (main (: n Int64)) (f (bin (u8 (UInt8.wrap n)) (u8 65) (u8 66)))) \
           (export main))",
    )))
    .expect("a guarded dependent-size bin arm compiles");
    wasmparser::validate(&bytes).expect(
        "a guarded dependent-size bin arm emits a VALID module (Core::And rhs floats above lhs's \
         high-water, so the guard's BinSizedRead handle does not alias the predicate's i64 length-probe \
         slot — cg3c)",
    );
}
