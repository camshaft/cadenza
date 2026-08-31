//! `lower::arith_fold` — arithmetic / boolean / bitwise lowering + constant folding & algebraic
//! simplification, split out of `lower.rs`. Covers quantity/unit combination, rational/bigint/float
//! lowering, integer arithmetic + negate, the boolean/comparison/bitwise algebraic simplifications
//! (short-circuit, complementary/subsuming comparisons, bitwise absorption/xor/shift collapse), and the
//! constant folders (`fold_arith`/`fold_short_circuit`/`fold_shift_bitwise_at_width`). Behaviour-
//! preserving move: items keep their visibility (`pub(crate) use arith_fold::*` in `lower` re-exports
//! each at its own vis); private items are `pub(super)` and reach the tree via `use super::*`.

use super::*;

/// Conditional-constant-propagation helper: if `branch` reduces to an inner `(if c' A B)` whose
/// condition `c'` is EQUIVALENT to the enclosing `cond` (via `core_equiv` — a pure-core structural
/// match), return the occurrence of the arm the enclosing branch's known truth of `cond` selects — `A`
/// when `cond_is_true` (the then-branch, where `cond` holds), `B` otherwise (the else-branch, where it
/// does not). Also handles the NEGATED case: when `c'` is the boolean negation of `cond` (`(not cond)`,
/// or `cond` is `(not c')`), the known truth of `cond` implies the OPPOSITE truth of `c'`, so the FLIPPED
/// arm is selected — `(if c A (if (not c) B D))` takes `B` in the else-branch (where `c` is false, so
/// `(not c)` is true). `None` if `branch` is not such a nested `if` (leave it unchanged). The returned
/// occurrence is REUSED as-is (no synthesis); it was resolved in the same scope, so lowering it in the
/// branch's place is sound. `reduce_to_if` chases refs/annotations and stops at a kept multi-use binding,
/// so a `let`-named inner `if` is not peeled (its value lives in a slot). Only the DIRECT nested `if` is
/// collapsed here; deeper propagation happens because the rewritten branch re-lowers and can collapse
/// again.
pub(super) fn collapse_repeated_cond(
    db: &mut Db,
    cond: StructId,
    branch: StructId,
    cond_is_true: bool,
) -> Option<StructId> {
    let (inner_cond, inner_then, inner_else) = crate::eval::reduce_to_if(db, branch)?;
    if core_equiv(db, cond, inner_cond) {
        // `c'` == `cond`: same truth → `cond_is_true` picks the inner then, else the inner else.
        Some(if cond_is_true { inner_then } else { inner_else })
    } else if is_negation_of(db, cond, inner_cond) {
        // `c'` == `!cond`: OPPOSITE truth → flip which arm survives.
        Some(if cond_is_true { inner_else } else { inner_then })
    } else {
        None
    }
}

/// Whether the cores at `a` and `b` are boolean NEGATIONS of each other — one is `Core::Not { operand }`
/// with `operand` `core_equiv` to the other. Both orders are tried (`a` is `(not b)` or `b` is `(not a)`).
/// `not` is total and pure, and `core_equiv` matches only pure cores, so a matched pair is two pure
/// booleans of exactly opposite truth. Used by `collapse_repeated_cond` to propagate a known condition
/// into a nested `if` guarded by that condition's negation.
pub(super) fn is_negation_of(db: &mut Db, a: StructId, b: StructId) -> bool {
    let one_way = |db: &mut Db, x: StructId, y: StructId| -> bool {
        matches!(core_of(db, x), Core::Not { operand } if core_equiv(db, operand, y))
    };
    one_way(db, a, b) || one_way(db, b, a)
}

/// Lower an ARITHMETIC application: FOLD it when its operands fold to constants — evaluate at compile
/// time with a CHECKED operation, so a provable overflow is a build error (CDZ0304 poison) rather than
/// a shipped runtime trap (`reference-compiler.md` §A Compile-Provable Trap Fails The Build). An
/// operand that is not a constant stays a runtime `Arith` (its wasm op selected from the solved width
/// at selection); a poison operand propagates.
/// Whether the arithmetic application at `id` is over QUANTITIES whose inner numeric type is a FLOAT —
/// so `+`/`-`/`*`/`/` must run the float operation on the erased inner values (a quantity's operator is
/// polymorphic over its inner numeric). Reads the node's solved type: `+`/`-`/comparison keep the
/// operands' unit so the RESULT is `(Qty Float …)`; `*`/`/` compose units so the result is still a
/// quantity — either way a `Ty::Qty { inner: Float }` result marks the float case. Falls back to the
/// first operand's type when the result is not itself a quantity (a `Qty.value`-peeled position).
pub(super) fn quantity_inner_is_float(db: &mut Db, id: StructId, args: &[StructId]) -> bool {
    let is_qty_float = |t: &crate::ty::Ty| matches!(t, crate::ty::Ty::Qty { inner, .. } if matches!(**inner, crate::ty::Ty::Float(_)));
    if is_qty_float(&crate::infer::type_of(db, id)) {
        return true;
    }
    // The result may not be a quantity (a comparison yields Bool); check the first operand.
    args.first()
        .map(|&a| is_qty_float(&crate::infer::type_of(db, a)))
        .unwrap_or(false)
}

/// Whether the operation is over a quantity with a RATIONAL inner magnitude — `(Qty Rational u)`. Checked
/// (like `quantity_inner_is_float`) on the result then the first operand, so a comparison (Bool result)
/// still routes by its operand. Such an op runs EXACT RATIONAL arithmetic on the erased inner magnitudes.
pub(super) fn quantity_inner_is_rational(db: &mut Db, id: StructId, args: &[StructId]) -> bool {
    let is_qty_rat = |t: &crate::ty::Ty| matches!(t, crate::ty::Ty::Qty { inner, .. } if matches!(**inner, crate::ty::Ty::Rational));
    if is_qty_rat(&crate::infer::type_of(db, id)) {
        return true;
    }
    args.first()
        .map(|&a| is_qty_rat(&crate::infer::type_of(db, a)))
        .unwrap_or(false)
}

/// Whether this `+`/`-`/`*`/`/` is over a quantity with a BIGINT inner magnitude — a `(Qty BigInt u)`.
/// A BigInt inner is a heap HANDLE (i32), not a fixnum, so its arithmetic must route to the runtime
/// `bigint-*` ops (`lower_bigint_arith`), exactly as a bare-BigInt `+` does — NOT the default integer
/// path (which treats the operand as an i64 fixnum → an i32/i64 miscompile). The plain `bigint_operand`
/// check misses this because a quantity's solved type is `Ty::Qty { inner: BigInt }`, not `Ty::BigInt`;
/// this peels the quantity to see the inner, mirroring `quantity_inner_is_rational`.
pub(super) fn quantity_inner_is_bigint(db: &mut Db, id: StructId, args: &[StructId]) -> bool {
    let is_qty_big = |t: &crate::ty::Ty| matches!(t, crate::ty::Ty::Qty { inner, .. } if matches!(**inner, crate::ty::Ty::BigInt));
    if is_qty_big(&crate::infer::type_of(db, id)) {
        return true;
    }
    args.first()
        .map(|&a| is_qty_big(&crate::infer::type_of(db, a)))
        .unwrap_or(false)
}

/// Whether the two operands are quantities of the SAME dimension at DIFFERENT scales — a mixed-unit
/// combine that must convert to the reference (`1 km + 500 m`). `false` when either is not a quantity,
/// they differ in dimension (that is CDZ0501, reported in `infer`), or the scales are equal (the common
/// same-unit case — no conversion, the ordinary arith path). Reads the operands' solved units.
pub(super) fn quantity_scales_differ(db: &mut Db, args: &[StructId]) -> bool {
    let (a, b) = (
        crate::infer::type_of(db, args[0]),
        crate::infer::type_of(db, args[1]),
    );
    match (&a, &b) {
        (crate::ty::Ty::Qty { unit: ua, .. }, crate::ty::Ty::Qty { unit: ub, .. }) => {
            ua.same_dimension(ub) && ua.scale() != ub.scale()
        }
        _ => false,
    }
}

/// Whether the two operands are quantities of the SAME unit — same dimension AND same scale (`meter` vs
/// `meter`, NOT `km` vs `m`). Routes a same-unit quantity COMPARISON through the erased-magnitude compare
/// (the units are identical, so no conversion). The both-quantity complement of `quantity_scales_differ`
/// (which routes the DIFFERENT-scale case through conversion); a cross-DIMENSION pair is neither (CDZ0501
/// in `infer`). Reads the operands' solved units.
pub(super) fn quantity_same_unit_pair(db: &mut Db, args: &[StructId]) -> bool {
    let (a, b) = (
        crate::infer::type_of(db, args[0]),
        crate::infer::type_of(db, args[1]),
    );
    match (&a, &b) {
        (crate::ty::Ty::Qty { unit: ua, .. }, crate::ty::Ty::Qty { unit: ub, .. }) => {
            ua.same_dimension(ub) && ua.scale() == ub.scale()
        }
        _ => false,
    }
}

/// Lower a MIXED-UNIT combine `(op a b)` where `a` and `b` are quantities of one dimension at different
/// scales: convert EACH operand to the dimension's REFERENCE unit by its exact scale (`value * num /
/// den` in the inner type T), then apply `op` at the reference. Folds the CONSTANT case — the operands
/// erase to a `Core::ConstInt`/`ConstFloat`, each scaled exactly (Int) or by round-to-nearest (Float),
/// per spec §48 ("los[es] precision only where the underlying numeric type is itself inexact"). A
/// non-constant operand DECLINES (the runtime scale-multiply is a later increment). `+`/`-` fold to the
/// converted numeric (rendered back as `(Qty <sum> <reference-unit>)` by the value form); a comparison
/// folds to a `ConstBool`.
pub(super) fn lower_quantity_combine(
    db: &mut Db,
    id: StructId,
    op: Prim,
    lhs: StructId,
    rhs: StructId,
) -> Core {
    // Each operand's scale to the reference (num/den) — read off its solved unit.
    let scale_of = |db: &mut Db, arg: StructId| -> Option<(i128, i128)> {
        match crate::infer::type_of(db, arg) {
            crate::ty::Ty::Qty { unit, .. } => Some(unit.scale()),
            _ => None,
        }
    };
    let (ln, ld) = match scale_of(db, lhs) {
        Some(s) => s,
        None => return Core::Poison(Reject::decline("mixed-unit combine: non-quantity operand")),
    };
    let (rn, rd) = match scale_of(db, rhs) {
        Some(s) => s,
        None => return Core::Poison(Reject::decline("mixed-unit combine: non-quantity operand")),
    };
    // The inner numeric type decides how conversion + the op run.
    let inner_is_float = matches!(
        crate::infer::type_of(db, lhs),
        crate::ty::Ty::Qty { inner, .. } if matches!(*inner, crate::ty::Ty::Float(_))
    );
    let lc = core_of(db, lhs);
    let rc = core_of(db, rhs);
    if inner_is_float {
        // FLOAT: convert each to the reference by `v * num / den` (rounding), then run the op.
        if let (Some(lv), Some(rv)) = (float_of_core(&lc), float_of_core(&rc)) {
            // CONSTANT operands — fold exactly at compile time.
            let l = lv * (ln as f64) / (ld as f64);
            let r = rv * (rn as f64) / (rd as f64);
            return fold_float_combine(op, l, r);
        }
        // RUNTIME operand(s) — synthesize the scale conversion as real float arithmetic and lower it.
        return lower_runtime_combine(db, op, lhs, (ln, ld), rhs, (rn, rd), true);
    }
    // RATIONAL inner: convert each operand to the reference EXACTLY — `v * (num/den)` =
    // `(vn*num)/(vd*den)`, renormalized — then combine (`+`/`-`/`*`/`/` or a comparison) exactly. This is
    // THE mixing case done without rounding: `1 inch + 1 mm` = 127/5000 + 1/1000 = 33/1250 m, exact.
    let inner_is_rational = matches!(
        crate::infer::type_of(db, lhs),
        crate::ty::Ty::Qty { inner, .. } if matches!(*inner, crate::ty::Ty::Rational)
    );
    if inner_is_rational {
        if let (Core::ConstRational(lvn, lvd), Core::ConstRational(rvn, rvd)) = (&lc, &rc) {
            let l = normalized_rational(
                lvn.mul(&IntValue::from_i128(ln)),
                lvd.mul(&IntValue::from_i128(ld)),
            );
            let r = normalized_rational(
                rvn.mul(&IntValue::from_i128(rn)),
                rvd.mul(&IntValue::from_i128(rd)),
            );
            // Fold the op over the two converted rationals directly (they are constant `ConstRational`s).
            let (Core::ConstRational(ln2, ld2), Core::ConstRational(rn2, rd2)) = (&l, &r) else {
                return Core::Poison(Reject::decline("mixed-unit rational conversion trapped"));
            };
            return match op {
                Prim::Add => normalized_rational(ln2.mul(rd2).add(&rn2.mul(ld2)), ld2.mul(rd2)),
                Prim::Sub => normalized_rational(ln2.mul(rd2).sub(&rn2.mul(ld2)), ld2.mul(rd2)),
                Prim::Mul => normalized_rational(ln2.mul(rn2), ld2.mul(rd2)),
                Prim::Div => normalized_rational(ln2.mul(rd2), ld2.mul(rn2)),
                // A comparison of the two converted rationals: `a/b <=> c/d ⇔ a·d <=> c·b`.
                Prim::Lt | Prim::Gt | Prim::Le | Prim::Ge | Prim::Eq => {
                    let ord = ln2.mul(rd2).cmp(&rn2.mul(ld2));
                    Core::ConstBool(compare_ord(op, ord))
                }
                _ => Core::Poison(Reject::decline("mixed-unit rational: unsupported operator")),
            };
        }
        // RUNTIME — convert each operand to the reference by `value * (Rational.of num den)` (an EXACT
        // rational multiply, no rounding) via `convert_operand_ast_rational`, then run the combine op on
        // the two converted Rationals (which lowers through the runtime `rational-*` ops). Mirrors the
        // BigInt runtime arm below; the exact-rational analogue of the Int/BigInt runtime scale multiply.
        let lconv = match convert_operand_ast_rational(db, lhs, ln, ld) {
            Some(n) => n,
            None => {
                return Core::Poison(Reject::decline(
                    "runtime mixed-unit Rational combine over a non-Qty.of operand is not supported",
                ));
            }
        };
        let rconv = match convert_operand_ast_rational(db, rhs, rn, rd) {
            Some(n) => n,
            None => {
                return Core::Poison(Reject::decline(
                    "runtime mixed-unit Rational combine over a non-Qty.of operand is not supported",
                ));
            }
        };
        let head = db.push_name(combine_op_name(op));
        let app = db.push_list(vec![head, lconv, rconv]);
        return core_of(db, app);
    }
    // BIGINT inner: convert each operand to the reference by `value * num / den` in UNBOUNDED bigint
    // arithmetic (the heap-handle magnitudes can't take the i128 fold below — that path is for fixnum
    // Int). A constant pair folds exactly over `IntValue` bignum (mul + truncating divmod); a runtime one
    // synthesizes `value * (BigInt.of num) / (BigInt.of den)` per operand (`convert_operand_ast_bigint`)
    // so the conversion `*`/`/` route to the runtime bigint ops, then the combine op runs on the two
    // converted BigInt values. This is the mixed-scale analogue of the BigInt Unit.in arm.
    let inner_is_bigint = matches!(
        crate::infer::type_of(db, lhs),
        crate::ty::Ty::Qty { inner, .. } if matches!(*inner, crate::ty::Ty::BigInt)
    );
    if inner_is_bigint {
        // Convert `v * n / d` over IntValue bignum (exact mul, truncating divmod). `None` if a value is
        // not a constant BigInt (→ runtime path below).
        let conv_const = |v: &IntValue, n: i128, d: i128| -> Option<IntValue> {
            let scaled = v.mul(&IntValue::from_i128(n));
            scaled.divmod(&IntValue::from_i128(d)).map(|(q, _)| q)
        };
        if let (Core::ConstInt(lv), Core::ConstInt(rv)) = (&lc, &rc)
            && let (Some(l), Some(r)) = (conv_const(lv, ln, ld), conv_const(rv, rn, rd))
        {
            // Both converted to reference-unit BigInts — fold the op (arith → a BigInt ConstInt;
            // comparison → a ConstBool via the exact bignum compare).
            return match op {
                Prim::Add => Core::ConstInt(l.add(&r)),
                Prim::Sub => Core::ConstInt(l.sub(&r)),
                Prim::Lt | Prim::Gt | Prim::Le | Prim::Ge | Prim::Eq => {
                    Core::ConstBool(compare_ord(op, l.cmp(&r)))
                }
                _ => Core::Poison(Reject::decline("mixed-unit bigint: unsupported operator")),
            };
        }
        // RUNTIME — synthesize `(op (value_l * (BigInt.of ln) / (BigInt.of ld)) (value_r * …))` and lower.
        let lconv = match convert_operand_ast_bigint(db, lhs, ln, ld) {
            Some(n) => n,
            None => {
                return Core::Poison(Reject::decline(
                    "runtime mixed-unit BigInt combine over a non-Qty.of operand is not supported",
                ));
            }
        };
        let rconv = match convert_operand_ast_bigint(db, rhs, rn, rd) {
            Some(n) => n,
            None => {
                return Core::Poison(Reject::decline(
                    "runtime mixed-unit BigInt combine over a non-Qty.of operand is not supported",
                ));
            }
        };
        let head = db.push_name(combine_op_name(op));
        let app = db.push_list(vec![head, lconv, rconv]);
        return core_of(db, app);
    }
    // INT (and other exact inner): convert each by `v * num / den` over i128 (exact; truncates on a
    // non-whole ratio, per opting into integer math).
    if let (Some(lv), Some(rv)) = (int_of_core(&lc), int_of_core(&rc)) {
        // CONSTANT operands — fold.
        let conv = |v: i128, n: i128, d: i128| -> Option<i128> { v.checked_mul(n).map(|x| x / d) };
        let (l, r) = match (conv(lv, ln, ld), conv(rv, rn, rd)) {
            (Some(l), Some(r)) => (l, r),
            _ => {
                return Core::Poison(Reject::coded(
                    Code::ConstTrap,
                    "mixed-unit conversion overflows the machine range",
                ));
            }
        };
        // Fold the op over the two reference-converted magnitudes, THEN range-check an arithmetic
        // result against the quantity's INNER integer width — exactly as `lower_arith` does for a bare
        // `+`/`-`. `fold_int_combine` evaluates over i128, so a NARROW overflow whose true result still
        // fits i128 (`100 + 100 = 200` over `(Qty Int8 u)`) folds to a valid `ConstInt` and would
        // otherwise slip through to a backend CDZ0302 ("a literal that doesn't fit") — and, because that
        // gate lives only in the backend, `cdz check` would MISS it entirely. It is a constant OPERATION
        // whose defined outcome is a TRAP (the sum overflows the type), NOT an out-of-range literal, so it
        // is CDZ0304 (`ConstTrap`) — the SAME code the bare `(+ (Int8.of 100) (Int8.of 100))` gets. The
        // inner width is read off `id`'s solved `Ty::Qty { inner: Int … }` (a comparison result is a Bool,
        // unaffected). Units are erased, so a quantity's arithmetic obeys the inner numeric type's rule.
        let folded = fold_int_combine(op, l, r);
        if let Core::ConstInt(ref r) = folded
            && let crate::ty::Ty::Qty { inner, .. } = crate::infer::type_of(db, id)
            && let crate::ty::Ty::Int(it) = *inner
            && !r.fits_width(it.ground_signed(), it.ground_width())
        {
            trace!(target: "rcdzc::fold", node = id.0, "mixed/same-unit Int combine result overflows the inner narrow width → CDZ0304");
            return Core::Poison(Reject::coded(
                Code::ConstTrap,
                "this constant arithmetic operation overflows its integer type (a \
                 compile-provable overflow traps)",
            ));
        }
        return folded;
    }
    // RUNTIME operand(s) — synthesize the scale conversion as real integer arithmetic and lower it.
    lower_runtime_combine(db, op, lhs, (ln, ld), rhs, (rn, rd), false)
}

/// The runtime path of a mixed-unit combine: synthesize `(op (convert lhs) (convert rhs))` as ordinary
/// arithmetic over the operands' ERASED magnitudes and lower it — the scale multiply the source denotes
/// by naming two units, emitted as real code (units-of-measure.md §A Unit Conversion Is The Arithmetic
/// The Source Denotes: "the scale multiply reaches the emitted component only when a magnitude is a
/// runtime value"). Each operand converts to the reference by `value * num / den` (float: `*.`/`/.`;
/// int: `*`/`/`), built from the quantity's value occurrence + synthesized constant factors. A
/// non-`Qty.of` operand (no reusable value occurrence) declines.
#[allow(clippy::too_many_arguments)]
pub(super) fn lower_runtime_combine(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    (ln, ld): (i128, i128),
    rhs: StructId,
    (rn, rd): (i128, i128),
    is_float: bool,
) -> Core {
    let lconv = match convert_operand_ast(db, lhs, ln, ld, is_float) {
        Some(n) => n,
        None => {
            return Core::Poison(Reject::decline(
                "runtime mixed-unit combine over a non-Qty.of operand is not supported",
            ));
        }
    };
    let rconv = match convert_operand_ast(db, rhs, rn, rd, is_float) {
        Some(n) => n,
        None => {
            return Core::Poison(Reject::decline(
                "runtime mixed-unit combine over a non-Qty.of operand is not supported",
            ));
        }
    };
    // Build `(op-name lconv rconv)` with the ONE arithmetic/comparison operator (a float inner routes it
    // to float arithmetic by operand type) so it lowers through the ordinary arith/comparison path — the
    // converted operands are bare numerics.
    let op_name = combine_op_name(op);
    let head = db.push_name(op_name);
    let app = db.push_list(vec![head, lconv, rconv]);
    core_of(db, app)
}

/// The erased magnitude of a quantity `operand` as a reusable arena node. A directly-written `(Qty.of x
/// u)` yields its value occurrence `x` (`qty_value_occ`); ANY OTHER quantity expression — a `*`/`/`-
/// computed quantity, a let-bound one — is not a literal `Qty.of`, so fall back to `(Qty.value operand)`,
/// the explicit unwrap that re-lowers `operand` and erases the unit. Both are the erased inner numeric.
/// This is the shared fallback the mixed-scale `convert_operand_ast*` helpers and `lower_qty_pow` use so
/// a runtime combine/conversion works over a COMPUTED quantity operand, not only a literal `Qty.of` (the
/// literal-only `qty_value_occ` alone declined those — e.g. `(+ (* (Qty n km) 2N) (Qty 500 m))`). Total:
/// a non-quantity operand still resolves through `Qty.value`'s own lowering (which faults if inapt).
pub(super) fn qty_magnitude_occ(db: &mut Db, operand: StructId) -> StructId {
    match crate::eval::qty_value_occ(db, operand) {
        Some(v) => v,
        None => {
            let dot = db.push_name(".");
            let qty = db.push_name("Qty");
            let value_key = db.push_name("value");
            let qty_value_head = db.push_list(vec![dot, qty, value_key]);
            db.push_list(vec![qty_value_head, operand])
        }
    }
}

/// Synthesize an arena node for a quantity operand's magnitude CONVERTED to the reference: `value * num
/// / den`, using the ordinary numeric operators (float `*.`/`/.` for a float inner, int `*`/`/`
/// otherwise). `value` is the quantity's magnitude (`qty_magnitude_occ` — a literal `Qty.of`'s value
/// occurrence, else `(Qty.value operand)`). When the scale is 1/1 the value passes through unconverted.
pub(super) fn convert_operand_ast(
    db: &mut Db,
    operand: StructId,
    num: i128,
    den: i128,
    is_float: bool,
) -> Option<StructId> {
    let value = qty_magnitude_occ(db, operand);
    // Scale 1/1 — no conversion, use the value as-is.
    if num == 1 && den == 1 {
        return Some(value);
    }
    // The ONE arithmetic operator `*`/`/` — a float `num.0` operand routes it to float arithmetic by the
    // operand type (there is no distinct `*.`/`/.`); `is_float` only picks the literal spelling.
    // `(* value num)` — multiply by the scale numerator (a `num.0` float literal for a float inner).
    let mut node = value;
    if num != 1 {
        let n_lit = num_literal(db, num, is_float);
        let mul_head = db.push_name("*");
        node = db.push_list(vec![mul_head, node, n_lit]);
    }
    // `(/ … den)` — divide by the denominator.
    if den != 1 {
        let d_lit = num_literal(db, den, is_float);
        let div_head = db.push_name("/");
        node = db.push_list(vec![div_head, node, d_lit]);
    }
    Some(node)
}

/// The runtime BigInt analogue of [`convert_operand_ast`]: synthesize `value * (BigInt.of num) /
/// (BigInt.of den)` for a `Unit.in` over a BigInt-magnitude quantity. The value occurrence is q's erased
/// BigInt magnitude (a heap handle); the scale factors are `(BigInt.of …)` so the `*`/`/` see a BigInt
/// operand and route to the runtime `bigint-*` ops (`bigint_operand` dispatch). Division TRUNCATES toward
/// zero (integer/bigint division), matching the fixed-Int arm. `None` if q's value occurrence is not
/// recoverable (a non-`Qty.of` runtime magnitude — a later increment).
pub(super) fn convert_operand_ast_bigint(
    db: &mut Db,
    operand: StructId,
    num: i128,
    den: i128,
) -> Option<StructId> {
    let value = qty_magnitude_occ(db, operand);
    // `(BigInt.of <n>)` — a bigint scale literal. `BigInt.of` is member access `(. BigInt of)`.
    let bigint_of = |db: &mut Db, n: i128| -> StructId {
        let dot = db.push_name(".");
        let bigint = db.push_name("BigInt");
        let of = db.push_name("of");
        let head = db.push_list(vec![dot, bigint, of]);
        let lit = db.push_atom(crate::ast::Leaf::Int {
            value: IntValue::from_i128(n),
            radix: crate::ast::Radix::Dec,
        });
        db.push_list(vec![head, lit])
    };
    let mut node = value;
    if num != 1 {
        let n_big = bigint_of(db, num);
        let mul_head = db.push_name("*");
        node = db.push_list(vec![mul_head, node, n_big]);
    }
    if den != 1 {
        let d_big = bigint_of(db, den);
        let div_head = db.push_name("/");
        node = db.push_list(vec![div_head, node, d_big]);
    }
    Some(node)
}

/// The runtime Rational analogue: synthesize `value * (Rational.of num den)` for a `Unit.in` over a
/// Rational-magnitude quantity. The value occurrence is q's erased Rational handle; the scale is a SINGLE
/// exact rational literal `(Rational.of num den)`, so the `*` is one runtime `rational-mul` (routed by
/// `rational_operand`, which peels Qty) — EXACT, no rounding and no separate divide (a rational carries
/// its own denominator). `Unit.in` UNWRAPS → a bare Rational. Scale 1/1 is the identity (value unchanged).
/// `None` if q's value occurrence is not recoverable (a non-`Qty.of` runtime magnitude — a later increment).
pub(super) fn convert_operand_ast_rational(
    db: &mut Db,
    operand: StructId,
    num: i128,
    den: i128,
) -> Option<StructId> {
    let value = qty_magnitude_occ(db, operand);
    if num == 1 && den == 1 {
        return Some(value);
    }
    // `(Rational.of <num> <den>)` — an exact rational scale literal. `Rational.of` is member access.
    let dot = db.push_name(".");
    let rational = db.push_name("Rational");
    let of = db.push_name("of");
    let head = db.push_list(vec![dot, rational, of]);
    let n_lit = db.push_atom(crate::ast::Leaf::Int {
        value: IntValue::from_i128(num),
        radix: crate::ast::Radix::Dec,
    });
    let d_lit = db.push_atom(crate::ast::Leaf::Int {
        value: IntValue::from_i128(den),
        radix: crate::ast::Radix::Dec,
    });
    let scale = db.push_list(vec![head, n_lit, d_lit]);
    let mul_head = db.push_name("*");
    Some(db.push_list(vec![mul_head, value, scale]))
}

/// A synthesized numeric literal node for a machine integer `v` — a float decimal `v.0` when `is_float`,
/// else an integer literal. Used for the constant scale factors a runtime conversion multiplies by.
pub(super) fn num_literal(db: &mut Db, v: i128, is_float: bool) -> StructId {
    if is_float {
        // Build the exact decimal for `v` (a whole number, always finite).
        match crate::ast::Decimal::from_f64(v as f64) {
            Some(d) => db.push_atom(crate::ast::Leaf::Float(d)),
            // Unreachable for a whole scale factor; fall back to an integer literal.
            None => db.push_atom(crate::ast::Leaf::Int {
                value: IntValue::from_i128(v),
                radix: crate::ast::Radix::Dec,
            }),
        }
    } else {
        db.push_atom(crate::ast::Leaf::Int {
            value: IntValue::from_i128(v),
            radix: crate::ast::Radix::Dec,
        })
    }
}

/// The ordinary arithmetic/comparison operator NAME for a mixed-unit combine `op` — the ONE operator
/// spelling per prim (`+`/`-`/`<`/…). A float inner routes `+`/`-` to float arithmetic by the operand
/// type at lowering (no distinct `+.`), so the spelling is inner-type-independent, exactly as the
/// comparisons already were.
pub(super) fn combine_op_name(op: Prim) -> &'static str {
    match op {
        Prim::Add => "+",
        Prim::Sub => "-",
        Prim::Lt => "<",
        Prim::Gt => ">",
        Prim::Le => "<=",
        Prim::Ge => ">=",
        Prim::Eq => "=",
        // Only additive/comparison ops reach a mixed-unit combine.
        _ => "+",
    }
}

/// The `f64` a constant float/int core holds (a quantity's erased inner), for the float conversion fold.
pub(super) fn float_of_core(c: &Core) -> Option<f64> {
    match c {
        Core::ConstFloat(d) => Some(f64::from_bits(d.to_f64_bits())),
        _ => None,
    }
}

/// The `i128` a constant int core holds (a quantity's erased inner), for the integer conversion fold.
pub(super) fn int_of_core(c: &Core) -> Option<i128> {
    match c {
        Core::ConstInt(v) => v.to_i128(),
        _ => None,
    }
}

/// Lower `(Unit.in target q)` — convert q's erased magnitude from its unit to `target` by
/// `value * (q.scale / target.scale)` in the inner type T (Float rounds, Int exact/truncates). A no-op
/// when the scales are equal. Folds the constant case; a runtime magnitude declines (the emitted runtime
/// scale-multiply is a later increment). The dimensional check (target vs q dimension) is
/// `check_application`'s (CDZ0501); here q is assumed same-dimension.
///
/// The conversion is exactly the SCALE ARITHMETIC the source denotes by naming the two units — the ratio
/// of the operand's scale to the target's, nothing the dimensional layer adds. A constant magnitude is
/// converted at compile time (folded to a `ConstFloat`/`ConstInt`, no runtime arithmetic), and the result
/// is a BARE numeric core — the `Ty::Qty` dimension is erased whether or not the scale multiply survives.
//= spec/capabilities/units-of-measure.md#a-unit-conversion-is-the-arithmetic-the-source-denotes
//# A conversion between two units of one dimension MUST be the scale arithmetic the source denotes by naming those units, not additional arithmetic the dimensional layer introduces, so that the emitted arithmetic is what the program means rather than an overhead the check imposes.
//= spec/capabilities/units-of-measure.md#a-unit-conversion-is-the-arithmetic-the-source-denotes
//# A unit conversion whose operands are compile-time constants MUST be computed at compile time, so that a conversion between constant quantities contributes no runtime arithmetic.
//= spec/capabilities/units-of-measure.md#a-unit-conversion-is-the-arithmetic-the-source-denotes
//# The dimension a quantity carries MUST be erased whether or not a scale conversion is emitted, so that the type-level dimensional information never survives into the component even when the scale arithmetic does.
pub(super) fn lower_unit_in(db: &mut Db, target: StructId, q: StructId) -> Core {
    // q's scale to the reference (read off its solved unit); the target's from `unit_of`.
    let (qn, qd) = match crate::infer::type_of(db, q) {
        crate::ty::Ty::Qty { unit, .. } => unit.scale(),
        _ => return Core::Poison(Reject::decline("Unit.in of a non-quantity")),
    };
    let (tn, td) = match crate::eval::unit_of(db, target) {
        Some(u) => u.scale(),
        None => return Core::Poison(Reject::decline("Unit.in target is not a unit")),
    };
    // The conversion factor is `q.scale / target.scale` = `(qn/qd) / (tn/td)` = `(qn*td) / (qd*tn)`.
    let inner_is_float = matches!(
        crate::infer::type_of(db, q),
        crate::ty::Ty::Qty { inner, .. } if matches!(*inner, crate::ty::Ty::Float(_))
    );
    // The conversion factor is `q.scale / target.scale` = `(qn*td) / (qd*tn)` — one combined ratio.
    let num = match qn.checked_mul(td) {
        Some(n) => n,
        None => {
            return Core::Poison(Reject::coded(
                Code::ConstTrap,
                "Unit.in conversion overflows",
            ));
        }
    };
    let den = match qd.checked_mul(tn) {
        Some(d) if d != 0 => d,
        _ => {
            return Core::Poison(Reject::coded(
                Code::ConstTrap,
                "Unit.in conversion overflows",
            ));
        }
    };
    let qc = core_of(db, q);
    // A RATIONAL magnitude converts EXACTLY: `value * (num/den)` = `(vn*num)/(vd*den)`, renormalized. This
    // is the whole point of a rational-magnitude unit (`1 inch in meter` = exactly 127/5000 m, no rounding).
    let inner_is_rational = matches!(
        crate::infer::type_of(db, q),
        crate::ty::Ty::Qty { inner, .. } if matches!(*inner, crate::ty::Ty::Rational)
    );
    if inner_is_rational {
        if let Core::ConstRational(vn, vd) = &qc {
            let scaled_num = vn.mul(&IntValue::from_i128(num));
            let scaled_den = vd.mul(&IntValue::from_i128(den));
            return normalized_rational(scaled_num, scaled_den);
        }
        // RUNTIME Rational — synthesize `value * (Rational.of num den)` and re-lower; the `*` sees a
        // Rational operand and routes to the runtime `rational-*` op (`quantity_inner_is_rational` /
        // `rational_operand` dispatch, both peel Qty). EXACT (rational multiply, no rounding); `Unit.in`
        // UNWRAPS → a bare Rational. Scale 1/1 is the identity (the value unchanged). Mirrors the BigInt
        // runtime arm.
        match convert_operand_ast_rational(db, q, num, den) {
            Some(node) => return core_of(db, node),
            None => {
                return Core::Poison(Reject::decline(
                    "Unit.in over a runtime non-Qty.of Rational magnitude is not supported",
                ));
            }
        }
    }
    // A BIGINT magnitude converts as `value * num / den` in UNBOUNDED bigint arithmetic — the value is a
    // heap handle, so the scale factors are materialized as `BigInt.of` and the `*`/`/` route to the
    // runtime bigint ops (`quantity_inner_is_bigint`/`bigint_operand` dispatch). `Unit.in` UNWRAPS, so the
    // result is a bare BigInt. A CONSTANT bigint folds via `IntValue` bignum (exact mul + truncating
    // divmod); a runtime one emits the bigint ops. A non-whole ratio TRUNCATES (integer division, the
    // same rule the fixed-Int arm uses). Scale 1/1 (reference→reference) is the identity — the value
    // unchanged.
    let inner_is_bigint = matches!(
        crate::infer::type_of(db, q),
        crate::ty::Ty::Qty { inner, .. } if matches!(*inner, crate::ty::Ty::BigInt)
    );
    if inner_is_bigint {
        if num == 1 && den == 1 {
            // Identity conversion — the erased BigInt value unchanged (still a bare BigInt).
            return qc;
        }
        if let Core::ConstInt(v) = &qc {
            // CONSTANT bigint — fold `v * num / den` exactly over IntValue bignum (mul is exact; div
            // truncates toward zero). Stays a BigInt-typed ConstInt (the emit choke-point materializes it).
            let scaled = v.mul(&IntValue::from_i128(num));
            return match scaled.divmod(&IntValue::from_i128(den)) {
                Some((quotient, _rem)) => Core::ConstInt(quotient),
                None => Core::Poison(Reject::coded(
                    Code::ConstTrap,
                    "Unit.in conversion divides by zero",
                )),
            };
        }
        // RUNTIME bigint — synthesize `(/ (* value (BigInt.of num)) (BigInt.of den))` and re-lower; the
        // `*`/`/` see a BigInt operand and route to the runtime bigint ops.
        match convert_operand_ast_bigint(db, q, num, den) {
            Some(node) => return core_of(db, node),
            None => {
                return Core::Poison(Reject::decline(
                    "Unit.in over a runtime non-Qty.of BigInt magnitude is not supported",
                ));
            }
        }
    }
    if inner_is_float {
        if let Some(v) = float_of_core(&qc) {
            // CONSTANT float magnitude — fold the conversion.
            let converted = v * (num as f64) / (den as f64);
            return match crate::ast::Decimal::from_f64(converted) {
                Some(d) => Core::ConstFloat(d),
                None => Core::Poison(Reject::decline("Unit.in float result has no finite form")),
            };
        }
    } else if let Some(v) = int_of_core(&qc) {
        // CONSTANT int magnitude — fold `v * num / den` (exact/truncating).
        return match v.checked_mul(num) {
            Some(scaled) => Core::ConstInt(IntValue::from_i128(scaled / den)),
            None => Core::Poison(Reject::coded(
                Code::ConstTrap,
                "Unit.in conversion overflows",
            )),
        };
    }
    // RUNTIME magnitude — synthesize `value * num / den` as real arithmetic over q's value occurrence
    // and lower it (the same scale-multiply the constant path folds, emitted as code).
    match convert_operand_ast(db, q, num, den, inner_is_float) {
        Some(node) => core_of(db, node),
        None => Core::Poison(Reject::decline(
            "Unit.in over a runtime non-Qty.of magnitude is not supported",
        )),
    }
}

/// Lower `(Qty.pow q n)` — raise q's erased magnitude to the `n`th power over the inner numeric type.
/// The unit is a compile-time concern (the solved `Ty::Qty` already carries `unit^n`); at runtime this
/// is just `value * value * … ` (`|n|` factors), synthesized as ordinary arithmetic over q's value
/// occurrence and re-lowered (so the constant case FOLDS through the normal arith path and a runtime
/// magnitude emits the multiplies). `n = 0` is the dimensionless `1` (the multiplicative identity in the
/// inner type). A NEGATIVE `n` is the reciprocal `1 / value^|n|` (an inverse unit like a frequency
/// `second⁻¹`) — the division runs in the inner type, so Float divides and Int TRUNCATES (`1 / 8 = 0`),
/// the documented "precision loss only where the numeric type is itself inexact / integer truncates".
pub(super) fn lower_qty_pow(db: &mut Db, q: StructId, exp: StructId) -> Core {
    let inner = match crate::infer::type_of(db, q) {
        crate::ty::Ty::Qty { inner, .. } => *inner,
        other => other,
    };
    let inner_is_float = matches!(inner, crate::ty::Ty::Float(_));
    let inner_is_bigint = matches!(inner, crate::ty::Ty::BigInt);
    let inner_is_rational = matches!(inner, crate::ty::Ty::Rational);
    let n = match crate::resolve::resolved_of(db, exp) {
        crate::resolved::Resolved::Int(v) => match v.to_i64() {
            Some(n) => n,
            None => return Core::Poison(Reject::decline("Qty.pow exponent out of range")),
        },
        _ => {
            return Core::Poison(Reject::decline(
                "Qty.pow exponent is not a compile-time integer",
            ));
        }
    };
    // The multiplicative identity `1` in q's INNER numeric type — `1.0` for a float inner, `(BigInt.of 1)`
    // for a BigInt inner, `(Rational.of 1 1)` for a Rational inner, else the fixed-width int `1`. The inner
    // type matters because the negative-exponent reciprocal `1 / value^|n|` (and the `n = 0` identity) must
    // be in the SAME numeric type as `value`: a bare Int `1` over a BigInt/Rational `value` is a numeric
    // mismatch (`Int64` vs `BigInt`), which slips past the check inside a quantity and surfaces as a backend
    // ownership error on the reciprocal divide (`1` and the heap handle disagree). Build `1` in-type here.
    let inner_one = |db: &mut Db| -> StructId {
        if inner_is_bigint {
            let dot = db.push_name(".");
            let bigint = db.push_name("BigInt");
            let of = db.push_name("of");
            let head = db.push_list(vec![dot, bigint, of]);
            let lit = db.push_atom(crate::ast::Leaf::Int {
                value: IntValue::from_i128(1),
                radix: crate::ast::Radix::Dec,
            });
            db.push_list(vec![head, lit])
        } else if inner_is_rational {
            let dot = db.push_name(".");
            let rational = db.push_name("Rational");
            let of = db.push_name("of");
            let head = db.push_list(vec![dot, rational, of]);
            let n_lit = db.push_atom(crate::ast::Leaf::Int {
                value: IntValue::from_i128(1),
                radix: crate::ast::Radix::Dec,
            });
            let d_lit = db.push_atom(crate::ast::Leaf::Int {
                value: IntValue::from_i128(1),
                radix: crate::ast::Radix::Dec,
            });
            db.push_list(vec![head, n_lit, d_lit])
        } else {
            num_literal(db, 1, inner_is_float)
        }
    };
    // `n = 0` — the dimensionless identity `1` (in the inner type).
    if n == 0 {
        let one = inner_one(db);
        return core_of(db, one);
    }
    // The erased magnitude to raise: a literal `(Qty.of x u)` yields its value occurrence `x`, ANY OTHER
    // quantity expression (a `/`-computed velocity, a let-bound quantity) falls back to `(Qty.value q)`
    // (`qty_magnitude_occ` — the shared literal-or-unwrap the mixed-scale converters also use). Both are
    // the erased inner numeric; `Qty.pow` raises that.
    let value = qty_magnitude_occ(db, q);
    // Build `value^|n|` = `(* (* … value value) value)` — `|n|` copies, left-nested — with the ONE
    // multiply operator `*`; a float `value` routes it to float arithmetic by the operand type at
    // lowering (no distinct `*.`), so the spelling is inner-type-independent.
    let mut node = value;
    for _ in 1..n.unsigned_abs() {
        let mul_head = db.push_name("*");
        node = db.push_list(vec![mul_head, node, value]);
    }
    // A negative exponent is the reciprocal `1 / value^|n|` (the ONE `/` operator, float-dispatched by
    // its operand type); a positive one is the power itself. Lower the synthesized node through the
    // ordinary arith path (so the constant case folds and a runtime magnitude emits the multiplies/div).
    if n < 0 {
        let one = inner_one(db);
        let div_head = db.push_name("/");
        node = db.push_list(vec![div_head, one, node]);
    }
    core_of(db, node)
}

/// Apply `op` to two converted FLOAT reference values, producing the result core: `+`/`-` a
/// `ConstFloat`, a comparison a `ConstBool`.
pub(super) fn fold_float_combine(op: Prim, l: f64, r: f64) -> Core {
    match op {
        Prim::Add | Prim::Sub => {
            let v = if matches!(op, Prim::Add) {
                l + r
            } else {
                l - r
            };
            match crate::ast::Decimal::from_f64(v) {
                Some(d) => Core::ConstFloat(d),
                None => Core::Poison(Reject::decline(
                    "mixed-unit float result has no finite form",
                )),
            }
        }
        Prim::Lt => Core::ConstBool(l < r),
        Prim::Gt => Core::ConstBool(l > r),
        Prim::Le => Core::ConstBool(l <= r),
        Prim::Ge => Core::ConstBool(l >= r),
        Prim::Eq => Core::ConstBool(l == r),
        _ => Core::Poison(Reject::decline("unexpected op in mixed-unit float combine")),
    }
}

/// Apply `op` to two converted INT reference values, producing the result core: `+`/`-` a `ConstInt`, a
/// comparison a `ConstBool`.
pub(super) fn fold_int_combine(op: Prim, l: i128, r: i128) -> Core {
    let arith = |v: Option<i128>| match v {
        Some(n) => Core::ConstInt(IntValue::from_i128(n)),
        None => Core::Poison(Reject::coded(
            Code::ConstTrap,
            "mixed-unit result overflows",
        )),
    };
    match op {
        Prim::Add => arith(l.checked_add(r)),
        Prim::Sub => arith(l.checked_sub(r)),
        Prim::Lt => Core::ConstBool(l < r),
        Prim::Gt => Core::ConstBool(l > r),
        Prim::Le => Core::ConstBool(l <= r),
        Prim::Ge => Core::ConstBool(l >= r),
        Prim::Eq => Core::ConstBool(l == r),
        _ => Core::Poison(Reject::decline("unexpected op in mixed-unit int combine")),
    }
}

/// The wrong-arity CDZ0201 reject shared by the fixed-arity BINARY operators — integer arithmetic
/// (`lower_arith`), FLOAT arithmetic (`lower_float_arith`), and COMPARISON (`lower_comparison`). All three
/// take exactly 2 operands; an OVER-application (`(+ 1 2 3)`, `(< 1 2 3)`, `(+ 1.0 2.0 3.0)`) has a
/// mechanical repair: DELETE the first surplus operand (`args[2]`) — the fixpoint removes each extra until
/// exactly 2 remain. A TOO-FEW application (`(+ 1)`) has nothing to delete → no fix. Carrying the delete
/// fix on THIS authoritative CDZ0201 is what lets `dedup_faults` drop the sibling CDZ0203 over-application
/// (which anchors at the same surplus node), so a binary operator over-application reports ONCE, with the
/// fix — the parity `lower_arith` had but `lower_comparison`/`lower_float_arith` lacked (they double-reported).
pub(super) fn binop_arity_reject(op: Prim, args: &[StructId]) -> Reject {
    let mut reject = Reject::coded(
        Code::Malformed,
        format!("{} takes exactly 2 operands", intrinsic_name(op)),
    );
    if args.len() > 2 {
        reject = reject.with_fix(crate::diag::Fix::delete_heuristic(
            args[2],
            "remove the extra operand",
        ));
    }
    reject
}

/// Build a NORMALIZED `Core::ConstRational` from a raw numerator/denominator pair, or `Core::Poison`
/// (a runtime TRAP) on a zero denominator. Normalization (numeric-model.md §An Exact Rational Has A
/// Canonical Normalized Form): divide both by `gcd(|num|, |den|)`, force the denominator strictly
/// positive (flip both signs if it is negative), so the sign lives on the numerator and equal values
/// share one byte form. `0/d` normalizes to `0/1`.
pub(super) fn normalized_rational(num: crate::ast::IntValue, den: crate::ast::IntValue) -> Core {
    use crate::ast::IntValue;
    if den.is_zero() {
        // A zero denominator has no value — a provable constant trap (CDZ0304, the rational analogue of a
        // constant ÷0). Name the repair like the sibling divide-by-zero CDZ0304 ("use a nonzero divisor")
        // rather than dead-ending at the bare fault: a rational `n/d` denotes a number only when `d` is
        // nonzero. The `contains("zero denominator")` lead stays stable for any matcher; the actionable
        // tail is additive. (`trap_kind` does not classify this reason, so no runtime-trap grading depends
        // on the exact text.)
        return Core::Poison(Reject::coded(
            Code::ConstTrap,
            "a rational with a zero denominator has no value — use a nonzero denominator",
        ));
    }
    if num.is_zero() {
        return Core::ConstRational(IntValue::zero(), IntValue::from_i64(1));
    }
    // Reduce to lowest terms.
    let g = num.gcd(&den); // non-negative
    let (mut n, mut d) = match (num.divmod(&g), den.divmod(&g)) {
        (Some((qn, _)), Some((qd, _))) => (qn, qd),
        _ => (num, den), // g is nonzero here (num,den both nonzero), so this never fires
    };
    // Force the denominator strictly positive: if it is negative, flip BOTH signs.
    if d.negative {
        n = n.neg();
        d = d.neg();
    }
    Core::ConstRational(n, d)
}

/// Fold a numeric LITERAL to a normalized `Core::ConstRational` — the value an annotation `(: lit
/// Rational)` (and, later, the `R` literal suffix) grounds to. An integer `k` is the exact `k/1`; a
/// decimal `significand·10^exp` is the exact `significand / 10^|exp|` (LOSSLESS — a `Decimal` captures
/// the source exactly, so `0.5` is precisely `1/2`, never a rounded `f64`): a non-negative exponent
/// scales the numerator (`12·10^2 / 1`), a negative one is the denominator `10^|exp|` (`5 / 10^1` for
/// `0.5`). `normalized_rational` reduces to lowest terms + puts the sign on the numerator. Returns
/// `None` for a non-literal expression (the annotation then erases normally). Only reached when the
/// annotation type is `Rational` (checked by the `Annot` arm).
/// If `id` is a numeric literal WRITTEN in a `(pragma default-fraction Rational)` module (recorded in the
/// load-time `default_fraction_literals` map, keyed by the ORIGINAL node so it survives β-copy), ground it
/// to a `Core::ConstRational` — the lowering side of the fraction default, reusing the annotation path's
/// [`rational_from_literal`]. `None` for a literal with no fraction default, so the caller keeps the
/// ordinary `ConstInt`/`ConstFloat`. The map only holds a `<T>` that reduced to `Rational` at load; we
/// re-check nothing here (the map's presence IS the "this literal defaults to Rational" fact — the same
/// map `infer` consulted to type it `Ty::Rational`, so the type and value stay in lockstep).
pub(super) fn default_fraction_rational(db: &mut Db, id: StructId) -> Option<Core> {
    if !db.default_fraction_literals.contains_key(&id) {
        return None;
    }
    // GUARD against an annotation override: the map records every numeric literal WRITTEN in the module,
    // including one inside `(: 5 Int64)`. For an annotated literal the `Annot` node fixes the type to
    // Int64, so grounding the inner literal to a `ConstRational` here would emit a rational VALUE for an
    // Int64-typed node — a miscompile (an invalid component). Fold to a rational ONLY when the literal's
    // SOLVED type is actually `Rational` (the unconstrained case the default governs); an annotation that
    // constrained it away from Rational leaves `type_of` ≠ Rational, so we keep the ordinary const.
    if !matches!(crate::infer::type_of(db, id), crate::ty::Ty::Rational) {
        return None;
    }
    rational_from_literal(db, id)
}

pub(super) fn rational_from_literal(db: &mut Db, expr: StructId) -> Option<Core> {
    use crate::ast::IntValue;
    // 10^k as an IntValue (k ≥ 0), by repeated multiply — no bignum dep, and `k` is a literal's decimal
    // digit count, so it is small.
    fn ten_pow(k: u64) -> IntValue {
        let ten = IntValue::from_i64(10);
        let mut acc = IntValue::from_i64(1);
        for _ in 0..k {
            acc = acc.mul(&ten);
        }
        acc
    }
    match crate::resolve::resolved_of(db, expr) {
        // An integer literal `k` grounds to `k/1`.
        crate::resolved::Resolved::Int(v) => Some(normalized_rational(v, IntValue::from_i64(1))),
        // A decimal `significand·10^exp` grounds to the exact fraction. The significand shares
        // `IntValue`'s big-endian magnitude representation, so it lifts directly (its sign is on `d.negative`).
        crate::resolved::Resolved::Float(d) => {
            let sig = IntValue {
                negative: d.negative,
                magnitude: d.significand.clone(),
            };
            let (num, den) = if d.exponent >= 0 {
                (sig.mul(&ten_pow(d.exponent as u64)), IntValue::from_i64(1))
            } else {
                (sig, ten_pow((-d.exponent) as u64))
            };
            Some(normalized_rational(num, den))
        }
        _ => None,
    }
}

/// Lower `Rational.of n d` — fold a constant numerator/denominator pair to a normalized
/// `Core::ConstRational` (or a zero-denominator trap), else emit the RUNTIME `Core::RationalOfInts`
/// (widen each int to a BigInt + `rational-of`, which normalizes + traps on a zero denominator at run
/// time — R3b). A poison operand propagates.
pub(super) fn lower_rational_of(db: &mut Db, num: StructId, den: StructId) -> Core {
    match (core_of(db, num), core_of(db, den)) {
        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
        (Core::ConstInt(n), Core::ConstInt(d)) => normalized_rational(n, d),
        _ => Core::RationalOfInts { num, den },
    }
}

/// Lower a RUNTIME `Rational.truncate r` as a DERIVATION over existing prims — no new runtime op. Synthesize
/// `(let ((__rtrunc r)) (Int64.of (/ ((. Rational numerator) __rtrunc) ((. Rational denominator) __rtrunc))))`
/// and lower THAT: `numerator`/`denominator` read the pair as BigInts, BigInt `/` truncates toward zero
/// (dividend-signed, matching the const `IntValue::divmod` fold), and `Int64.of` checked-narrows the small
/// quotient (TRAPS on overflow). `r` is referenced twice, so it is LET-BOUND once (`__rtrunc`) — computed a
/// single time, read by two occurrences (the `__` prefix cannot collide with a source name). The operand
/// node `r` is spliced verbatim into the binding (it is already resolved/typed in this scope; the original
/// `Rational.truncate` apply node is fully replaced by this subtree's core, so the splice reparents cleanly
/// exactly as `partial_ctor_eta_closure` splices its supplied args). `resolve_subtree` classifies the synth
/// nodes; `core_of` on the root produces the derivation's Core (the same shape a hand-written source
/// expression lowers to — verified to compile + run: `7/2 → 3`, `-7/2 → -3`).
pub(super) fn lower_rational_truncate(db: &mut Db, r: StructId) -> Core {
    let bind_name = "__rtrunc";
    // `(let ((__rtrunc r)) body)`.
    let binder_occ = db.push_name(bind_name);
    let binding = db.push_list(vec![binder_occ, r]);
    let bindings = db.push_list(vec![binding]);
    // `((. Rational numerator) __rtrunc)` and `((. Rational denominator) __rtrunc)` — a member access
    // applied to a fresh reference of the let binder (the `.` head form, per `(. List len)` precedent).
    let num_call = rational_member_call(db, "numerator", bind_name);
    let den_call = rational_member_call(db, "denominator", bind_name);
    // `(/ num_call den_call)` — BigInt truncating division (both operands are `Ty::BigInt`).
    let div_head = db.push_name("/");
    let quotient = db.push_list(vec![div_head, num_call, den_call]);
    // `(Int64.of quotient)` — the checked BigInt→Int64 narrowing (traps on overflow).
    let dot = db.push_name(".");
    let int64_mod = db.push_name("Int64");
    let of_key = db.push_name("of");
    let int64_of = db.push_list(vec![dot, int64_mod, of_key]);
    let narrowed = db.push_list(vec![int64_of, quotient]);
    let let_head = db.push_name("let");
    let derivation = db.push_list(vec![let_head, bindings, narrowed]);
    crate::resolve::resolve_subtree(db, derivation);
    core_of(db, derivation)
}

/// Build `((. Rational <member>) <bind_name>)` — a `Rational` module member access (`numerator`/
/// `denominator`) applied to a fresh reference of the let-bound name. Helper for `lower_rational_truncate`
/// (and the later floor/ceil/round derivations) so each reads the rational's component through the same
/// member surface the source would use. A FRESH name occurrence per call (a node has one parent).
pub(super) fn rational_member_call(db: &mut Db, member: &str, bind_name: &str) -> StructId {
    let dot = db.push_name(".");
    let rational_mod = db.push_name("Rational");
    let member_key = db.push_name(member);
    let member_access = db.push_list(vec![dot, rational_mod, member_key]);
    let arg_ref = db.push_name(bind_name);
    db.push_list(vec![member_access, arg_ref])
}

/// Build `(BigInt.of <n>)` — a BigInt-typed integer literal for a synthesized derivation (the `0`/`1` a
/// floor/ceil/round comparison + `±1` adjustment compares/combines with the BigInt numerator/remainder).
/// A fresh node each call.
pub(super) fn bigint_lit(db: &mut Db, n: i64) -> StructId {
    let lit = db.push_atom(crate::ast::Leaf::Int {
        value: crate::ast::IntValue::from_i64(n),
        radix: crate::ast::Radix::Dec,
    });
    let dot = db.push_name(".");
    let bigint_mod = db.push_name("BigInt");
    let of_key = db.push_name("of");
    let bigint_of = db.push_list(vec![dot, bigint_mod, of_key]);
    db.push_list(vec![bigint_of, lit])
}

/// Lower a RUNTIME `Rational.floor`/`ceil r` as a DERIVATION — `truncate` adjusted by ±1 off the remainder
/// sign. Synthesize (for FLOOR — ceil flips the comparison to `>` and the adjustment to `+`):
/// ```text
/// (let ((__rfc r))
///   (Int64.of
///     (let ((__q (/ (num __rfc) (den __rfc))))
///       (if (and (< (num __rfc) 0N) (not (= (% (num __rfc) (den __rfc)) 0N)))
///           (- __q 1N)
///           __q))))
/// ```
/// The toward-zero quotient `__q` is one too HIGH for a floor exactly when the value is NEGATIVE with a
/// nonzero remainder (`n < 0 ∧ rem ≠ 0`), so subtract 1 there; ceil is the mirror (positive + nonzero rem →
/// add 1). All BigInt ops (`/`, `%`, `<`/`>`, `=`) + the checked `Int64.of` narrowing already exist → no new
/// runtime op (hash-neutral). `__q` is let-bound (read twice); `num`/`den`/`rem` reads are fresh
/// `rational_member_call`s (each a distinct occurrence). Matches the const-fold arms above and the verified
/// formula (`floor(-7/2) = -4`, `ceil(7/2) = 4`).
pub(super) fn lower_rational_floor_ceil(db: &mut Db, r: StructId, is_floor: bool) -> Core {
    let outer = "__rfc";
    let q_name = "__q";
    // `(/ (num __rfc) (den __rfc))` — the toward-zero truncating quotient, let-bound to `__q`.
    let q_div = {
        let num = rational_member_call(db, "numerator", outer);
        let den = rational_member_call(db, "denominator", outer);
        let div_head = db.push_name("/");
        db.push_list(vec![div_head, num, den])
    };
    let q_binder = db.push_name(q_name);
    let q_binding = db.push_list(vec![q_binder, q_div]);
    let q_bindings = db.push_list(vec![q_binding]);
    // `(<cmp> (num __rfc) 0N)` — floor: `n < 0`; ceil: `n > 0`.
    let sign_test = {
        let num = rational_member_call(db, "numerator", outer);
        let zero = bigint_lit(db, 0);
        let cmp = db.push_name(if is_floor { "<" } else { ">" });
        db.push_list(vec![cmp, num, zero])
    };
    // `(not (= (% (num __rfc) (den __rfc)) 0N))` — a nonzero remainder (the fraction is not whole).
    let rem_nonzero = {
        let num = rational_member_call(db, "numerator", outer);
        let den = rational_member_call(db, "denominator", outer);
        let rem_head = db.push_name("%");
        let rem = db.push_list(vec![rem_head, num, den]);
        let zero = bigint_lit(db, 0);
        let eq_head = db.push_name("=");
        let is_zero = db.push_list(vec![eq_head, rem, zero]);
        let not_head = db.push_name("not");
        db.push_list(vec![not_head, is_zero])
    };
    let and_head = db.push_name("and");
    let cond = db.push_list(vec![and_head, sign_test, rem_nonzero]);
    // `(<±> __q 1N)` — floor subtracts 1, ceil adds 1 — on the adjustment branch; else plain `__q`.
    let adjusted = {
        let q_ref = db.push_name(q_name);
        let one = bigint_lit(db, 1);
        let op = db.push_name(if is_floor { "-" } else { "+" });
        db.push_list(vec![op, q_ref, one])
    };
    let q_else = db.push_name(q_name);
    let if_head = db.push_name("if");
    let if_expr = db.push_list(vec![if_head, cond, adjusted, q_else]);
    let inner_let_head = db.push_name("let");
    let inner_let = db.push_list(vec![inner_let_head, q_bindings, if_expr]);
    // `(Int64.of <inner_let>)` — checked narrowing (traps on overflow).
    let dot = db.push_name(".");
    let int64_mod = db.push_name("Int64");
    let of_key = db.push_name("of");
    let int64_of = db.push_list(vec![dot, int64_mod, of_key]);
    let narrowed = db.push_list(vec![int64_of, inner_let]);
    // `(let ((__rfc r)) <narrowed>)`.
    let outer_binder = db.push_name(outer);
    let outer_binding = db.push_list(vec![outer_binder, r]);
    let outer_bindings = db.push_list(vec![outer_binding]);
    let outer_let_head = db.push_name("let");
    let derivation = db.push_list(vec![outer_let_head, outer_bindings, narrowed]);
    crate::resolve::resolve_subtree(db, derivation);
    core_of(db, derivation)
}

/// Lower a RUNTIME `Rational.round r` as a DERIVATION — NEAREST integer, ties HALF-AWAY-FROM-ZERO. Synthesize:
/// ```text
/// (let ((__rr r))
///   (Int64.of
///     (let ((__num (numerator __rr)) (__den (denominator __rr)))
///       (let ((__q (/ __num __den)) (__rem (% __num __den)))
///         (let ((__abs (if (< __rem 0N) (- 0N __rem) __rem)))
///           (if (>= (* 2N __abs) __den)
///               (if (< __rem 0N) (- __q 1N) (+ __q 1N))
///               __q))))))
/// ```
/// The toward-zero quotient `__q` is adjusted AWAY from zero (by the sign of the remainder, which equals the
/// value's sign) when twice the |remainder| is ≥ the denominator — i.e. the fractional part is ≥ ½. Using
/// `≥` (not `>`) makes an exact-half tie round AWAY (`2·|rem| = __den` at ½), the settled half-away ruling.
/// Multi-use values (`__num`/`__den`/`__q`/`__rem`/`__abs`) are LET-BOUND (each read 2–3×). All BigInt ops
/// (`/`, `%`, `*`, `-`, `<`, `>=`) + the checked `Int64.of` narrowing already exist → hash-neutral. Matches
/// the const-fold arm + the verified formula (`1/2 → 1`, `-1/2 → -1`, `3/2 → 2`, `5/2 → 3`, `7/3 → 2`).
pub(super) fn lower_rational_round(db: &mut Db, r: StructId) -> Core {
    let outer = "__rr";
    // `(let ((__num (numerator __rr)) (__den (denominator __rr))) …)`.
    let num_call = rational_member_call(db, "numerator", outer);
    let den_call = rational_member_call(db, "denominator", outer);
    let num_binding = {
        let b = db.push_name("__num");
        db.push_list(vec![b, num_call])
    };
    let den_binding = {
        let b = db.push_name("__den");
        db.push_list(vec![b, den_call])
    };
    let numden_bindings = db.push_list(vec![num_binding, den_binding]);
    // `(let ((__q (/ __num __den)) (__rem (% __num __den))) …)`.
    let q_binding = {
        let div_head = db.push_name("/");
        let n = db.push_name("__num");
        let d = db.push_name("__den");
        let div = db.push_list(vec![div_head, n, d]);
        let b = db.push_name("__q");
        db.push_list(vec![b, div])
    };
    let rem_binding = {
        let rem_head = db.push_name("%");
        let n = db.push_name("__num");
        let d = db.push_name("__den");
        let rem = db.push_list(vec![rem_head, n, d]);
        let b = db.push_name("__rem");
        db.push_list(vec![b, rem])
    };
    let qrem_bindings = db.push_list(vec![q_binding, rem_binding]);
    // `(let ((__abs (if (< __rem 0N) (- 0N __rem) __rem))) …)` — the remainder's magnitude.
    let abs_binding = {
        let rem_ref = db.push_name("__rem");
        let zero = bigint_lit(db, 0);
        let lt = db.push_name("<");
        let neg_test = db.push_list(vec![lt, rem_ref, zero]);
        let zero2 = bigint_lit(db, 0);
        let rem_ref2 = db.push_name("__rem");
        let sub = db.push_name("-");
        let negated = db.push_list(vec![sub, zero2, rem_ref2]);
        let rem_ref3 = db.push_name("__rem");
        let if_head = db.push_name("if");
        let abs = db.push_list(vec![if_head, neg_test, negated, rem_ref3]);
        let b = db.push_name("__abs");
        db.push_list(vec![b, abs])
    };
    let abs_bindings = db.push_list(vec![abs_binding]);
    // The tie test `(>= (* 2N __abs) __den)`.
    let tie_test = {
        let two = bigint_lit(db, 2);
        let abs_ref = db.push_name("__abs");
        let mul = db.push_name("*");
        let doubled = db.push_list(vec![mul, two, abs_ref]);
        let den_ref = db.push_name("__den");
        let ge = db.push_name(">=");
        db.push_list(vec![ge, doubled, den_ref])
    };
    // The away-from-zero adjustment `(if (< __rem 0N) (- __q 1N) (+ __q 1N))`.
    let adjusted = {
        let rem_ref = db.push_name("__rem");
        let zero = bigint_lit(db, 0);
        let lt = db.push_name("<");
        let neg_test = db.push_list(vec![lt, rem_ref, zero]);
        let q1 = db.push_name("__q");
        let one1 = bigint_lit(db, 1);
        let sub = db.push_name("-");
        let minus = db.push_list(vec![sub, q1, one1]);
        let q2 = db.push_name("__q");
        let one2 = bigint_lit(db, 1);
        let add = db.push_name("+");
        let plus = db.push_list(vec![add, q2, one2]);
        let if_head = db.push_name("if");
        db.push_list(vec![if_head, neg_test, minus, plus])
    };
    let q_else = db.push_name("__q");
    let if_head = db.push_name("if");
    let tie_if = db.push_list(vec![if_head, tie_test, adjusted, q_else]);
    // Nest the lets: abs over (q,rem) over (num,den).
    let let_head3 = db.push_name("let");
    let abs_let = db.push_list(vec![let_head3, abs_bindings, tie_if]);
    let let_head2 = db.push_name("let");
    let qrem_let = db.push_list(vec![let_head2, qrem_bindings, abs_let]);
    let let_head1 = db.push_name("let");
    let numden_let = db.push_list(vec![let_head1, numden_bindings, qrem_let]);
    // `(Int64.of <numden_let>)` — checked narrowing (traps on overflow).
    let dot = db.push_name(".");
    let int64_mod = db.push_name("Int64");
    let of_key = db.push_name("of");
    let int64_of = db.push_list(vec![dot, int64_mod, of_key]);
    let narrowed = db.push_list(vec![int64_of, numden_let]);
    // `(let ((__rr r)) <narrowed>)`.
    let outer_binder = db.push_name(outer);
    let outer_binding = db.push_list(vec![outer_binder, r]);
    let outer_bindings = db.push_list(vec![outer_binding]);
    let outer_let_head = db.push_name("let");
    let derivation = db.push_list(vec![outer_let_head, outer_bindings, narrowed]);
    crate::resolve::resolve_subtree(db, derivation);
    core_of(db, derivation)
}

/// The two `IntValue`s of a constant rational OPERAND (already normalized), or `None` if `id` did not fold
/// to a compile-time constant (a runtime rational — the caller then emits the runtime `rational-*` op, R3b).
///
/// A `Core::ConstInt(n)` counts as the rational `n/1`: this function only classifies operands of a RATIONAL
/// op (`lower_rational_arith`/`lower_rational_cmp`), so an integer-valued operand there IS a constant
/// rational whose denominator is 1 — the whole number n as an exact rational. This matters because a
/// projection/const-eval fold of an INTEGER-VALUED Rational field folds to a bare `Core::ConstInt` (its
/// numerator), not a `Core::ConstRational` (e.g. #3543's nullary-record field fold: `(. (rect) w)` with a
/// Rational field `w = 4` folds to `ConstInt(4)`). Without this arm the constant pair `(* 4 3)` failed to
/// recognize its operands as rationals, fell through to the RUNTIME `RationalBinOp`, and — since a bare
/// `ConstInt` has no heap ownership class at the borrowing-op emit — DECLINED ("borrowing op operand has an
/// ownership this backend cannot yet prove"; the notebook Rational-field regression breaker bisected to
/// #3543). Folding to `n/1` here reduces the whole op to a `Core::ConstRational`, so no runtime op (and no
/// borrow classification) is reached at all — the fully-general const-fold outcome.
pub(super) fn const_rational_of(
    db: &mut Db,
    id: StructId,
) -> Option<(crate::ast::IntValue, crate::ast::IntValue)> {
    match core_of(db, id) {
        Core::ConstRational(n, d) => Some((n, d)),
        // An integer-valued rational operand: `n` as the exact rational `n/1` (denominator 1).
        Core::ConstInt(n) => Some((n, crate::ast::IntValue::from_i64(1))),
        _ => None,
    }
}

/// Lower a rational `+`/`-`/`*`/`/` — fold a CONSTANT pair to a normalized `Core::ConstRational` via
/// exact `IntValue` bignum arithmetic, or (R3b) emit the runtime `Core::RationalBinOp` when an operand is
/// RUNTIME-valued (the runtime `rational-*` op computes + normalizes on the limb library). A poison
/// propagates. The constant formulas keep the result normalized (`normalized_rational` re-reduces): `a/b +
/// c/d = (ad+cb)/(bd)`, `a/b - c/d = (ad-cb)/(bd)`, `a/b * c/d = (ac)/(bd)`, `a/b ÷ c/d = (ad)/(bc)`
/// (division by `0/1` → a zero denominator → trap, exactly `Rational.of`'s zero-denom trap).
///
/// `Rational` is a declared-EXACT numeric type, and this arithmetic loses NO precision on EITHER path: the
/// constant fold works over `IntValue` bignum numerators/denominators and the runtime op over the runtime
/// BigInt limb library — no fixed width to overflow, no rounding — so an exact rational operation's result
/// is the exact number whether it folds or runs.
//= spec/capabilities/numeric-model.md#exact-arithmetic-is-exact
//# An operation on values of a numeric type declared exact MUST NOT lose precision.
pub(super) fn lower_rational_arith(db: &mut Db, op: Prim, lhs: StructId, rhs: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, lhs) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, rhs) {
        return Core::Poison(r);
    }
    let (Some((a, b)), Some((c, d))) = (const_rational_of(db, lhs), const_rational_of(db, rhs))
    else {
        // A RUNTIME operand — emit the runtime `rational-*` op (R3b). The op computes + normalizes on the
        // runtime limb library; a poison operand already returned above.
        let rat_op = match op {
            Prim::Add => crate::core::RationalOp::Add,
            Prim::Sub => crate::core::RationalOp::Sub,
            Prim::Mul => crate::core::RationalOp::Mul,
            Prim::Div => crate::core::RationalOp::Div,
            _ => return Core::Poison(Reject::decline("not a Rational arithmetic op")),
        };
        return Core::RationalBinOp {
            op: rat_op,
            lhs,
            rhs,
        };
    };
    let (num, den) = match op {
        Prim::Add => (a.mul(&d).add(&c.mul(&b)), b.mul(&d)),
        Prim::Sub => (a.mul(&d).sub(&c.mul(&b)), b.mul(&d)),
        Prim::Mul => (a.mul(&c), b.mul(&d)),
        Prim::Div => (a.mul(&d), b.mul(&c)),
        _ => return Core::Poison(Reject::decline("not a Rational arithmetic op")),
    };
    normalized_rational(num, den)
}

/// Lower a rational comparison `<`/`>`/`<=`/`>=`/`=` — fold a CONSTANT pair to a `Core::ConstBool` by
/// comparing the two normalized rationals EXACTLY: `a/b <=> c/d` ⇔ `a*d <=> c*b` (both denominators are
/// strictly positive after normalization, so cross-multiplication preserves the order direction), or
/// (R3b) emit `Core::RationalCmp` (the runtime `rational-cmp` op + a fixed compare-with-zero) when an
/// operand is RUNTIME-valued. A poison propagates.
pub(super) fn lower_rational_cmp(db: &mut Db, op: Prim, lhs: StructId, rhs: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, lhs) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, rhs) {
        return Core::Poison(r);
    }
    let (Some((a, b)), Some((c, d))) = (const_rational_of(db, lhs), const_rational_of(db, rhs))
    else {
        // A RUNTIME operand — emit `Core::RationalCmp` (`rational-cmp` + a fixed compare-with-zero, R3b).
        return Core::RationalCmp { op, lhs, rhs };
    };
    let ord = a.mul(&d).cmp(&c.mul(&b));
    Core::ConstBool(compare_ord(op, ord))
}

/// True iff either operand of a binary op has solved type `Ty::Rational` — the signal to route `+`/`-`/
/// `*`/`/`/comparison to the exact rational fold. (A `Rational`/other mix never reaches lowering —
/// `check_application` rejected it CDZ0301 — so if ONE operand is a Rational the other is too.)
pub(super) fn rational_operand(db: &mut Db, args: &[StructId]) -> bool {
    // A bare `Rational` OR a quantity over a Rational magnitude — a `(Qty Rational u)` erases to its
    // inner Rational core, so a comparison of two such quantities folds through the same rational path
    // (they are same-dimension same-reference-unit under the model, a same-unit compare of the erased
    // rationals). Peel `Ty::Qty` to its inner; without this a `(< (Qty Rational) (Qty Rational))` fell to
    // the scalar compare and DECLINED ("compound needs a heap walk").
    args.iter().any(|&a| {
        matches!(
            peel_qty_inner_ty(crate::infer::type_of(db, a)),
            crate::ty::Ty::Rational
        )
    })
}

/// The inner numeric type of a `(Qty T u)`, or the type itself when not a quantity — so a quantity's
/// magnitude arithmetic/comparison routes by its ERASED inner numeric (a quantity erases to its inner
/// value's core). Shared by `rational_operand`/`bigint_operand` so a `(Qty Rational/BigInt u)` takes the
/// exact rational/bigint path rather than the fixnum scalar path.
pub(super) fn peel_qty_inner_ty(ty: crate::ty::Ty) -> crate::ty::Ty {
    match ty {
        crate::ty::Ty::Qty { inner, .. } => *inner,
        other => other,
    }
}

/// True iff either operand of a binary op has solved type `Ty::BigInt` — the signal to route `+`/`-`/
/// `*`/`/` to the runtime BigInt arithmetic instead of the fixed-width int fold. (A `BigInt`/fixed mix
/// never reaches lowering — `check_application` rejected it CDZ0301 — so if ONE operand is a BigInt the
/// other is too.)
pub(super) fn bigint_operand(db: &mut Db, args: &[StructId]) -> bool {
    // A bare `BigInt` OR a quantity over a BigInt magnitude (`(Qty BigInt u)` erases to its inner BigInt
    // handle) — peel `Ty::Qty` so a `(< (Qty BigInt) (Qty BigInt))` routes to the bigint comparison
    // (`bigint-cmp`) rather than declining as a compound scalar compare. The arithmetic `+`/`-`/`*`/`/`
    // over a BigInt-inner quantity is dispatched separately (`quantity_inner_is_bigint`); this covers the
    // COMPARISON path, which reads `bigint_operand` in `lower_comparison`.
    args.iter().any(|&a| {
        matches!(
            peel_qty_inner_ty(crate::infer::type_of(db, a)),
            crate::ty::Ty::BigInt
        )
    })
}

/// True iff either operand of a binary op has solved type `Ty::Float` — the signal to remap `+`/`-`/`*`/
/// `/` to the float prim (`FAdd`…) and route to `lower_float_arith` instead of the integer fold. There is
/// no distinct `+.`; floating-point arithmetic is dispatched here on the operand type, like the BigInt/
/// Rational operands. (A `Float`/int mix never reaches lowering — `check_application` rejected it CDZ0301
/// — so if ONE operand is a Float the other is too.)
pub(super) fn float_operand(db: &mut Db, args: &[StructId]) -> bool {
    // Peel `Ty::Qty` before reading the inner type — a `(Qty Float64 u)` erases to a bare f64, so a
    // comparison of two same-unit Float-inner quantities is a plain scalar float compare and must route to
    // the float path (`lower_comparison`'s FloatCompare), NOT decline as "a compound value needs a heap
    // walk". Without the peel a `(< (Qty x meter) (Qty 5.0 meter))` saw `Ty::Qty` (not `Ty::Float`), missed
    // this dispatch, and fell through to the compound-comparison decline — a gap masked until runtime float
    // ordering landed (before that even the bare float compare declined). Mirrors `bigint_operand`/
    // `rational_operand`, which already peel `Ty::Qty` for the same reason. (A mixed-scale quantity
    // comparison is handled earlier by `lower_quantity_combine`, which converts to the reference; this
    // covers the SAME-scale case that falls through to the generic comparison dispatch.)
    args.iter().any(|&a| {
        matches!(
            peel_qty_inner_ty(crate::infer::type_of(db, a)),
            crate::ty::Ty::Float(_)
        )
    })
}

/// Lower a BigInt `+`/`-`/`*`/`/` to a runtime `Core::BigIntBinOp` (the runtime `bigint-*` op). Unlike
/// fixed-width arithmetic, this does NOT constant-fold: exact BigInt arithmetic needs compiler-side
/// bignum (rcdzc deliberately has no bignum crate — `IntValue` carries the value but not arithmetic), so
/// the unbounded arithmetic runs at RUN TIME via the runtime `Big` limb library (B3a). A poison operand
/// propagates. `div` traps on a zero divisor at run time (numeric-model — an unbounded range gives `n/0`
/// no value); the never-trapping add/sub/mul grow the magnitude as needed.
///
/// `BigInt` represents every integer with no bound, so this arithmetic never overflows, and the runtime
/// `bigint-*` limb ops grow the representation as the result requires rather than wrapping or trapping on
/// magnitude (only `n/0` — no value — traps).
//= spec/capabilities/numeric-model.md#an-arbitrary-precision-integer-has-unbounded-range
//# An arbitrary-precision integer type MUST represent every integer with no maximum or minimum bound, so that an arithmetic operation on it never overflows.
//= spec/capabilities/numeric-model.md#an-arbitrary-precision-integer-has-unbounded-range
//# An arithmetic operation on arbitrary-precision integers MUST NOT trap for the magnitude of its result, growing its representation as the result requires rather than wrapping or trapping.
pub(super) fn lower_bigint_arith(db: &mut Db, op: Prim, lhs: StructId, rhs: StructId) -> Core {
    let lc = core_of(db, lhs);
    let rc = core_of(db, rhs);
    if let Core::Poison(r) = lc {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = rc {
        return Core::Poison(r);
    }
    // A CONSTANT BigInt pair could fold exactly here (the `IntValue` bignum is available), BUT it
    // DELIBERATELY does NOT: the repeated-squaring idiom `a_i = (* a_{i-1} a_{i-1})` DOUBLES the bit-width
    // each level, so folding a depth-k chain computes a 2^k-bit number at COMPILE TIME — a
    // compile-time-blowup / hang on a small program (the `a_repeated_squaring_bigint_chain_diagnoses_in_
    // bounded_time` regression). Exact unbounded arithmetic is a RUNTIME op (the runtime grows the
    // magnitude lazily, and only if the value is actually demanded); the compiler stays bounded. The ONE
    // exception is a program that EXPORTS a single constant BigInt result — but that path is served by the
    // boundary value-form on a `Core::ConstInt` (a plain widened literal), not by folding an arithmetic
    // chain. So a runtime `bigint-*` op is emitted for EVERY BigInt arithmetic, constant operands or not.
    let big_op = match op {
        Prim::Add => crate::core::BigIntOp::Add,
        Prim::Sub => crate::core::BigIntOp::Sub,
        Prim::Mul => crate::core::BigIntOp::Mul,
        Prim::Div => crate::core::BigIntOp::Div,
        Prim::Rem => crate::core::BigIntOp::Rem,
        _ => return Core::Poison(Reject::decline("not a BigInt arithmetic op")),
    };
    Core::BigIntBinOp {
        op: big_op,
        lhs,
        rhs,
    }
}

/// Lower a BigInt comparison `<`/`>`/`<=`/`>=`/`=` to either a constant `Bool` fold or a runtime
/// `Core::BigIntCmp` (the runtime `bigint-cmp` op + a fixed compare-with-zero). A CONSTANT pair (both
/// operands `Core::ConstInt` — the shape a folded `(BigInt.of <constant>)` leaves) folds when both values
/// fit `i128` (`to_i128` reads the exact value; every constant a program is likely to compare fits, and
/// the runtime op covers the rest), comparing at 128-bit precision. A poison operand propagates. Otherwise
/// (a runtime operand) emit `Core::BigIntCmp`; the emit borrows both operands and applies the operator's
/// signed compare against the three-way `-1`/`0`/`1` result.
pub(super) fn lower_bigint_cmp(db: &mut Db, op: Prim, lhs: StructId, rhs: StructId) -> Core {
    let lc = core_of(db, lhs);
    let rc = core_of(db, rhs);
    match (lc, rc) {
        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
        // A constant BigInt pair — both carry the exact `IntValue`. Fold at 128-bit precision when both
        // fit; a value beyond i128 (astronomically large) falls through to the runtime op.
        (Core::ConstInt(a), Core::ConstInt(b)) => match (a.to_i128(), b.to_i128()) {
            (Some(x), Some(y)) => {
                let r = compare_ord(op, x.cmp(&y));
                trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded constant BigInt comparison (i128)");
                Core::ConstBool(r)
            }
            _ => Core::BigIntCmp { op, lhs, rhs },
        },
        _ => Core::BigIntCmp { op, lhs, rhs },
    }
}

/// Lower UNARY NEGATION `(- e)` (the ML prefix `-<expr>`, canonicalized to the arity-1 subtraction) as
/// `0 - e` at the operand's numeric type. Rather than a new negate op, synthesize a typed ZERO operand
/// and route to the SAME binary-subtraction machinery each numeric type already uses — so an `Int N`
/// gets the checked `x == MIN` overflow trap (the identical path the `x * -1 → (- 0 x)` strength
/// reduction takes), a `Float`/`Rational`/`BigInt` its own arithmetic, and a `Qty` negates its erased
/// magnitude while `type_of(id)` (the negation node, typed `(Qty …)` by `infer`) preserves the unit.
///
/// A FLOAT is negated by `-1.0 * e` (via `lower_float_arith`'s multiply), NOT `0.0 - e`: IEEE
/// `0.0 - (+0.0)` is `+0.0`, but negation must flip the sign of a zero (`-(+0.0) = -0.0`,
/// core-semantics.md §Floating-Point Equality Follows The Canonical Byte Form distinguishes them), and
/// `-1.0 * x` flips the sign bit for zero/inf/finite alike. Integer/Rational/BigInt `0 - e` is exact,
/// so those keep the subtraction form (and the int `MIN` trap).
pub(super) fn lower_negate(db: &mut Db, id: StructId, operand: StructId) -> Core {
    use crate::ty::Ty;
    // Propagate a poison operand (its own fault is the report).
    if let Core::Poison(r) = core_of(db, operand) {
        return Core::Poison(r);
    }
    let t = crate::infer::type_of(db, id);
    // The inner numeric type — for a `Qty` it is the erased magnitude's type; otherwise the type itself.
    let inner = match &t {
        Ty::Qty { inner, .. } => (**inner).clone(),
        other => other.clone(),
    };
    // For a `Qty` operand, negate the ERASED magnitude occurrence (`Qty.of`'s value); a bare numeric
    // operand negates directly. A runtime non-`Qty.of` quantity magnitude has no erased occurrence to
    // read — decline (the same gap `lower_qty_pow`/`lower_quantity_combine` decline on).
    let value = if matches!(t, Ty::Qty { .. }) {
        match crate::eval::qty_value_occ(db, operand) {
            Some(v) => v,
            None => {
                return Core::Poison(Reject::decline(
                    "negation of a runtime non-Qty.of quantity magnitude is not supported",
                ));
            }
        }
    } else {
        operand
    };
    match &inner {
        // FLOAT — `-1.0 * e` (sign-correct for ±0.0/inf), folded/emitted by `lower_float_arith`.
        Ty::Float(_) => {
            let neg_one = synth_core(
                db,
                Core::ConstFloat(match crate::ast::Decimal::from_f64(-1.0) {
                    Some(d) => d,
                    None => return Core::Poison(Reject::decline("-1.0 has no decimal form")),
                }),
                inner.clone(),
            );
            lower_float_arith(db, id, Prim::FMul, &[neg_one, value])
        }
        // BIGINT — `0 - e` via the runtime `bigint-sub` (never folds; grows as needed).
        Ty::BigInt => {
            let zero = synth_core(db, Core::ConstInt(IntValue::zero()), Ty::BigInt);
            lower_bigint_arith(db, Prim::Sub, zero, value)
        }
        // RATIONAL — `0 - e`, folded exactly (constant) or the runtime `rational-sub`.
        Ty::Rational => {
            let zero = synth_core(
                db,
                Core::ConstRational(IntValue::zero(), IntValue::from_i64(1)),
                Ty::Rational,
            );
            lower_rational_arith(db, Prim::Sub, zero, value)
        }
        // FIXED-WIDTH INTEGER — `0 - e` via `lower_arith`, which folds a constant (a `0 - MIN` overflow →
        // CDZ0304) and otherwise emits the checked runtime subtract (its `x == MIN` guard is negation's).
        Ty::Int(_) => {
            let zero = synth_core(db, Core::ConstInt(IntValue::zero()), inner.clone());
            lower_arith(db, id, Prim::Sub, &[zero, value])
        }
        // Not a numeric type — `infer` already rejected this (CDZ0201 "negation is not defined on …"); a
        // residual `Any` (an operand that faulted elsewhere) declines rather than fabricating a value.
        _ => Core::Poison(Reject::decline(
            "negation of a non-numeric operand (a fault is reported at inference)",
        )),
    }
}

pub(super) fn lower_arith(db: &mut Db, id: StructId, op: Prim, args: &[StructId]) -> Core {
    if args.len() != 2 {
        return Core::Poison(binop_arity_reject(op, args));
    }
    let lhs = core_of(db, args[0]);
    let rhs = core_of(db, args[1]);
    match (lhs, rhs) {
        (Core::ConstInt(a), Core::ConstInt(b)) => {
            // OVERFLOW POLICY (STAGE 2b, numeric-model §Overflow Behavior Is Configurable By Policy,
            // #5313/#5337): under a `(pragma overflow (signed|unsigned wrap))` module a CONSTANT `+`/`-`/`*`
            // that overflows must WRAP (two's-complement, mod 2^width) — NOT reject CDZ0304. `overflow_mode_of`
            // resolves this node's authoritative mode (module pragma by operand signedness > global manifest >
            // Trap; post-mono, #5686) — the SAME decision the backend codegen (2b runtime half) + the oracle
            // read, so const-fold cannot drift from runtime. A Wrap node folds to the EXACT result truncated
            // to the operand's solved width via `IntValue::wrap_to` (identical to what the runtime wrapping op
            // yields). Only `+`/`-`/`*` carry a policy (the spec's configurable ops); `/`/`%`/shift/bitwise
            // keep their existing folds. A non-overflowing op wraps to itself (no-op), so this changes nothing
            // except at overflow. `Trap` mode does NOT match here → falls through to the existing
            // overflow→CDZ0304 logic below, unchanged.
            if matches!(op, Prim::Add | Prim::Sub | Prim::Mul)
                && crate::infer::overflow_mode_of(db, id) == crate::db::OverflowMode::Wrap
                && let crate::ty::Ty::Int(it) = peel_qty_inner_ty(crate::infer::type_of(db, id))
            {
                let exact = match op {
                    Prim::Add => a.add(&b),
                    Prim::Sub => a.sub(&b),
                    _ => a.mul(&b), // Mul (the `matches!` guard admits only Add/Sub/Mul)
                };
                let wrapped = exact.wrap_to(it.ground_signed(), it.ground_width());
                trace!(target: "rcdzc::fold", node = id.0, op = intrinsic_name(op), "constant +/-/* WRAPS mod 2^width (pragma overflow wrap) instead of trapping");
                return Core::ConstInt(wrapped);
            }
            // ALGEBRAIC IDENTITY FIRST — before the i64 fold. `fold_arith` evaluates over `i64`, so an
            // operand at/above `2^63` (a legitimate `UInt64` constant, e.g. `UInt64.max = 2^64-1`) has no
            // `i64` and `fold_arith` rejects it CDZ0304 ("constant operand does not fit the integer width")
            // — a SPURIOUS reject of valid unsigned arithmetic (`(+ (: 18446744073709551615 UInt64) 0)`
            // declined instead of folding to the operand). The width-agnostic identities (`x+0`/`0+x`/`x-0`/
            // `x*1`/`1*x`/`x*0`) return an OPERAND unchanged (or a trap-free `0`), so they are correct at
            // ANY width WITHOUT an i64 evaluation — and the both-constant case never reached them before (it
            // dispatched straight to `fold_arith`; the `arith_identity` call below is only in the
            // not-both-constant fallthrough). Trying them here first folds the big-UInt64 identity cleanly.
            // Only fires when at least one operand is out of i64 range (the case `fold_arith` mishandles); an
            // in-range both-constant op keeps its exact `fold_arith` result (identical, so no behavior change
            // for the common case). A NON-identity big-UInt64 op (`(+ u64max 1)`) still falls to `fold_arith`
            // and its CDZ0304 — a genuine unsigned-overflow fold is a separable follow-up.
            if a.to_i64().is_none() || b.to_i64().is_none() {
                let lc = Core::ConstInt(a.clone());
                let rc = Core::ConstInt(b.clone());
                if let Some(simplified) = arith_identity(db, op, args[0], &lc, args[1], &rc) {
                    trace!(target: "rcdzc::fold", op = intrinsic_name(op), "big-unsigned constant identity folded (operand out of i64 range — bypassing the i64 fold)");
                    return simplified;
                }
                // GENERAL WIDE FOLD (an operand ≥ 2^63 has no `i64`, so `fold_arith`'s i64 path would
                // spuriously reject it CDZ0304 — but the SOLVED type is a wide UInt64 whose range the result
                // may well fit). Evaluate `Add`/`Sub`/`Mul`/`Div`/`Rem` over EXACT arbitrary-precision
                // `IntValue` (no i64 truncation), THEN range-check the result against the operands' solved
                // width — the SAME `fits_width` check the i64 path applies below. `(/ (: u64max-1 UInt64) 2)`
                // → the true quotient (fits UInt64 → folds); a genuine overflow (`(+ u64max 1)` → 2^64,
                // doesn't fit u64) → CDZ0304, exactly as the i64 path would for a narrow overflow. Trap
                // semantics: `divmod` returns `None` on a zero divisor (→ CDZ0304 divide-by-zero); UNSIGNED
                // has no `MIN/-1` trap (signed-only, and `IntValue::divmod` has no such special case), so an
                // unsigned `/`/`%` never spuriously traps. Shifts/bitwise stay on the i64 path below (a wide
                // shift-count/mask is out of scope here; they fall through to `fold_arith`). Only reached
                // when an operand exceeds i64 AND no identity fired, so the in-range fast path is untouched.
                let wide = match op {
                    Prim::Add => Some(a.add(&b)),
                    Prim::Sub => Some(a.sub(&b)),
                    Prim::Mul => Some(a.mul(&b)),
                    Prim::Div => a.divmod(&b).map(|(q, _)| q),
                    Prim::Rem => a.divmod(&b).map(|(_, r)| r),
                    _ => None, // shifts/bitwise/other — fall through to the i64 fold_arith path below
                };
                match (op, wide) {
                    // A defined result — range-check against the solved width (peeling `Ty::Qty` as the i64
                    // path does), fold if it fits, else the same OPERATION-overflow CDZ0304.
                    (Prim::Add | Prim::Sub | Prim::Mul | Prim::Div | Prim::Rem, Some(r)) => {
                        return match peel_qty_inner_ty(crate::infer::type_of(db, id)) {
                            crate::ty::Ty::Int(it)
                                if !r.fits_width(it.ground_signed(), it.ground_width()) =>
                            {
                                trace!(target: "rcdzc::fold", node = id.0, op = intrinsic_name(op), "wide constant arithmetic result overflows the solved width → CDZ0304");
                                Core::Poison(Reject::coded(
                                    Code::ConstTrap,
                                    "this constant arithmetic operation overflows its integer type (a \
                                     compile-provable overflow traps)",
                                ))
                            }
                            _ => {
                                trace!(target: "rcdzc::fold", op = intrinsic_name(op), "wide constant arithmetic folded exactly over IntValue (operand out of i64 range)");
                                Core::ConstInt(r)
                            }
                        };
                    }
                    // `Div`/`Rem` by a zero divisor — `divmod` gave `None`; the constant operation always
                    // traps (the same CDZ0304 the i64 path's `checked_div`/`checked_rem` → `None` produces).
                    (Prim::Div | Prim::Rem, None) => {
                        trace!(target: "rcdzc::fold", op = intrinsic_name(op), "wide constant divide/rem by zero → CDZ0304");
                        return Core::Poison(Reject::coded(
                            Code::ConstTrap,
                            format!(
                                "`{}` by the constant 0 always traps (divide by zero) — guard the divisor \
                                 or remove the division",
                                intrinsic_name(op)
                            ),
                        ));
                    }
                    // A shift/bitwise op (with a wide operand) is NOT handled in this wide-arith `match` —
                    // it falls through to the unified shift/bitwise fold just below (which covers BOTH the
                    // wide-operand and the small-operand-wide-result cases uniformly).
                    _ => {}
                }
            }
            // SHIFT / BITWISE over a fixed width, folded over the SOLVED width (NOT `fold_arith`'s i64 path).
            // Covers BOTH (a) a ≥2^63 UNSIGNED operand (`& (: u64max UInt64) …`, `>> big-u64 1`) — no i64, so
            // the i64 path would spuriously reject — AND (b) a SMALL-operand shift whose RESULT exceeds i64 but
            // fits the unsigned width (`(<< (: 1 UInt64) 63)` = 2^63; `checked_shl_i64` overflow-checked against
            // Int64 → spurious CDZ0304). `fold_shift_bitwise_at_width` range-checks against the SOLVED width and
            // handles UNSIGNED only (a SIGNED type returns `None` → the i64 path folds it, preserving arithmetic
            // sign-extending `>>`). Returns `None` for non-shift/bitwise ops → the i64 arith path handles
            // Add/Sub/Mul/Div/Rem. A shift/bitwise whose result fits i64 folds identically either way, so this
            // is behavior-preserving for the common narrow case and only fixes the previously-wrong unsigned
            // wide-result `<<`/`>>`/`&`/`|`/`^` outcomes.
            if let crate::ty::Ty::Int(it) = peel_qty_inner_ty(crate::infer::type_of(db, id))
                && let Some(folded) =
                    fold_shift_bitwise_at_width(op, &a, &b, it.ground_signed(), it.ground_width())
            {
                return folded;
            }
            // A CONSTANT `+`/`-`/`*`/`/`/`%` whose SOLVED type is WIDER than i64 (an unsigned 64-bit
            // type, whose max `2^64-1` is not i64-representable) where BOTH operands fit i64 but the exact
            // RESULT overflows i64 while still fitting the solved width — `(+ (: Int64.max UInt64) 2)` =
            // 2^63+1, a valid UInt64. `fold_arith`'s i64 `checked_add` would trap on the i64-overflowing
            // result → a SPURIOUS "overflows Int64" CDZ0304, even though the value is in range for UInt64.
            // (The ≥2^63-OPERAND wide-fold path above handles the case where an OPERAND lacks an i64; this
            // is its twin for a SMALL-OPERAND, WIDE-RESULT op — mirroring the shift/bitwise wide-result fix
            // just above.) Compute the result EXACTLY over `IntValue` (no i64 truncation) and range-check
            // against the solved width: fold if it fits, else the same OPERATION-overflow CDZ0304 the i64
            // path emits. Fires ONLY for a solved type that does NOT fit i64 (u64) — every narrower/signed
            // type keeps the i64 path unchanged (identical result when it fits, correct CDZ0304 when it
            // genuinely overflows the type), so this is behavior-preserving for the common case. A `/`/`%`
            // by zero (`divmod` → `None`) falls through to `fold_arith`'s divide-by-zero CDZ0304.
            if let crate::ty::Ty::Int(it) = peel_qty_inner_ty(crate::infer::type_of(db, id))
                && !it.fits_within(crate::ty::IntTy::i64())
            {
                let wide = match op {
                    Prim::Add => Some(a.add(&b)),
                    Prim::Sub => Some(a.sub(&b)),
                    Prim::Mul => Some(a.mul(&b)),
                    Prim::Div => a.divmod(&b).map(|(q, _)| q),
                    Prim::Rem => a.divmod(&b).map(|(_, r)| r),
                    _ => None, // shift/bitwise handled above; anything else falls to the i64 path
                };
                if let Some(r) = wide {
                    return if r.fits_width(it.ground_signed(), it.ground_width()) {
                        trace!(target: "rcdzc::fold", node = id.0, op = intrinsic_name(op), "wide-result constant arithmetic folded exactly over IntValue (result overflows i64 but fits the solved u64 width)");
                        Core::ConstInt(r)
                    } else {
                        trace!(target: "rcdzc::fold", node = id.0, op = intrinsic_name(op), "wide constant arithmetic result overflows the solved width → CDZ0304");
                        Core::Poison(Reject::coded(
                            Code::ConstTrap,
                            "this constant arithmetic operation overflows its integer type (a \
                             compile-provable overflow traps)",
                        ))
                    };
                }
            }
            // Fold over i64, THEN range-check the result against the op's SOLVED width. `fold_arith`
            // evaluates at the Stage i64 width, so a NARROW overflow whose true result still fits i64
            // (`255 + 1 = 256` over UInt8, `100 * 2 = 200` over Int8) folds to a valid `ConstInt` and
            // would otherwise slip through to a backend CDZ0302 ("a literal that doesn't fit"). But this
            // is a constant OPERATION whose defined outcome is a TRAP (the value overflows the type),
            // NOT an out-of-range literal — so it is CDZ0304 (`ConstTrap`), the SAME code the wide
            // `(+ Int64.max 1)` gets and the reject-don't-miscompile discipline the const-overflow /
            // List.update-OOB path already follows. (A direct out-of-range LITERAL `(: 256 UInt8)` is
            // still CDZ0302 at its own annotation — it is a literal, not an operation result.)
            match fold_arith(op, a, b) {
                // Peel `Ty::Qty` before reading the width: a same-unit quantity `+`/`-`/`*`/`/` over a
                // narrow-Int inner (`(Qty Int8 u)`) falls through to this generic arith path (its scales
                // are equal, so it is NOT a mixed-unit combine), and its solved type is `Ty::Qty { inner:
                // Int … }`, not a bare `Ty::Int`. Without the peel the width-check below never fired for a
                // quantity, so an inner-narrow overflow (`100 + 100` over `(Qty Int8 u)`) slipped through to
                // a backend CDZ0302 ("a literal that doesn't fit") — a wrong code (it is an OPERATION
                // overflow, not a literal), and one `cdz check` MISSED (the gate lives only in the backend).
                // Units are erased, so a quantity's arithmetic obeys the inner integer type's overflow rule.
                Core::ConstInt(r) => match peel_qty_inner_ty(crate::infer::type_of(db, id)) {
                    crate::ty::Ty::Int(it)
                        if !r.fits_width(it.ground_signed(), it.ground_width()) =>
                    {
                        trace!(target: "rcdzc::fold", node = id.0, op = intrinsic_name(op), "constant arithmetic result overflows the narrow width → CDZ0304");
                        Core::Poison(Reject::coded(
                            Code::ConstTrap,
                            "this constant arithmetic operation overflows its integer type (a \
                             compile-provable overflow traps)",
                        ))
                    }
                    _ => Core::ConstInt(r),
                },
                other => other,
            }
        }
        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
        // ALGEBRAIC IDENTITY: one operand is a constant whose value makes the op a NO-OP or a constant
        // result — the whole checked operation (and its overflow guard) is eliminated at lowering. Only
        // the identities that are SAFE at every width and never trap are applied (see `arith_identity`);
        // the RESULT keeps the op's solved type because the runtime operand shares it (binary-op
        // unification), and a `0`/`1` constant grounds to that width at selection.
        // A CONSTANT-ZERO DIVISOR with a RUNTIME numerator — `(/ n 0)` / `(% n 0)`. The divisor is the
        // compile-time literal `0`, so the operation ALWAYS traps regardless of `n` — there is no runtime
        // value of `n` that makes it valid. Reject CDZ0304 (the same code the both-constant `(/ 10 0)`
        // gets), rather than emitting a component that traps at run time (`numeric-model.md` §A Constant
        // Operation With No Value Is Rejected At Compile Time). This inherits the const-trap machinery's
        // BRANCH SHIELDING: the reached-poison walk does not descend an untaken `if` branch, so `(if false
        // (/ n 0) 1)` is NOT rejected (the trap is unreachable), exactly as the both-constant case is
        // shielded there. Distinct from `(/ n z)` with a runtime `z` that HAPPENS to be 0 at a call (a
        // genuine runtime trap — `z` is a variable, not the literal `0`, so this never fires for it).
        (_, Core::ConstInt(ref b)) if matches!(op, Prim::Div | Prim::Rem) && b.is_zero() => {
            trace!(target: "rcdzc::lower", op = intrinsic_name(op), "divide by a constant zero → CDZ0304 (always traps)");
            Core::Poison(Reject::coded(
                Code::ConstTrap,
                format!(
                    "`{}` by the constant 0 always traps (divide by zero) — guard the divisor or remove \
                     the division",
                    intrinsic_name(op)
                ),
            ))
        }
        (lc, rc) => {
            if let Some(simplified) = arith_identity(db, op, args[0], &lc, args[1], &rc) {
                trace!(target: "rcdzc::lower", op = intrinsic_name(op), "arithmetic identity simplified (op elided)");
                return simplified;
            }
            // OVERFLOW POLICY (STAGE 2c, numeric-model §Overflow Behavior Is Configurable By Policy — the
            // RUNTIME twin of the 2b const-fold above): under a `(pragma overflow (signed|unsigned wrap))`
            // module a RUNTIME `+`/`-`/`*` that overflows must WRAP (two's-complement, mod 2^width) — NOT
            // trap. `overflow_mode_of` resolves this node's authoritative mode off the SAME predicate the
            // both-const arm read (so const/runtime cannot drift; signedness-selective per #5686). We do NOT
            // emit a new backend op: the `WrappingAdd`/`WrappingSub`/`WrappingMul` prims already lower to the
            // raw machine op + a narrow re-normalize (mask/sign-extend, NO overflow guard) in every backend
            // (wasm/rust/cadenza) and are treated first-class by every pass (`is_trap_free`, `arith_identity`,
            // `const_eval`). So rewrite `+`/`-`/`*` to its wrapping twin here — identical value+width to what
            // the 2b const path yields, and to `UInt8.wrapping-add` etc. `Trap` mode / no-pragma leaves the op
            // unchanged (→ the checked-arith emit that traps, as before). Guarded to fixed-width `Int`
            // (peeling `Qty`) to match the const half exactly — BigInt/Rational lower elsewhere and never wrap.
            let op = if matches!(op, Prim::Add | Prim::Sub | Prim::Mul)
                && crate::infer::overflow_mode_of(db, id) == crate::db::OverflowMode::Wrap
                && matches!(
                    peel_qty_inner_ty(crate::infer::type_of(db, id)),
                    crate::ty::Ty::Int(_)
                ) {
                let w = match op {
                    Prim::Add => Prim::WrappingAdd,
                    Prim::Sub => Prim::WrappingSub,
                    _ => Prim::WrappingMul, // Mul (the `matches!` guard admits only Add/Sub/Mul)
                };
                trace!(target: "rcdzc::lower", node = id.0, op = intrinsic_name(op), "runtime +/-/* rewritten to its WRAPPING twin (pragma overflow wrap) — raw machine op, no trap guard");
                w
            } else {
                op
            };
            trace!(target: "rcdzc::lower", op = intrinsic_name(op), "arithmetic stays runtime (operand not constant)");
            Core::Arith {
                op,
                lhs: args[0],
                rhs: args[1],
            }
        }
    }
}

/// Lower a FLOAT arithmetic application (`+.`/`-.`/`*.`/`/.`). FOLDS two constant floats at the solved
/// float WIDTH (the `Decimal` operands round to the width's IEEE format, the op runs, the result rounds
/// back — round-to-nearest-even, the fixed deterministic mode); a non-constant operand DECLINES (runtime
/// float ops emit the machine `f64.add`/… in a later increment). Unlike integer arithmetic there is NO
/// checked-trap: an IEEE overflow yields an infinity — but a NON-FINITE result has no written value form
/// (the float-literal-overflow rule), so a fold to `±inf`/NaN DECLINES rather than producing a bad value.
pub(super) fn lower_float_arith(db: &mut Db, id: StructId, op: Prim, args: &[StructId]) -> Core {
    if args.len() != 2 {
        return Core::Poison(binop_arity_reject(op, args));
    }
    let lhs = core_of(db, args[0]);
    let rhs = core_of(db, args[1]);
    match (lhs, rhs) {
        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
        (Core::ConstFloat(a), Core::ConstFloat(b)) => {
            // The result WIDTH is the application's solved type (both operands unify to it). Fold at that
            // width: round each operand to the width's format, compute, round the result back.
            let width = match crate::infer::type_of(db, id) {
                crate::ty::Ty::Float(ft) => ft.ground_width(),
                _ => crate::ty::DEFAULT_FLOAT_WIDTH,
            };
            let fold_at = |x: f64, y: f64| -> f64 {
                let r = match op {
                    Prim::FAdd => x + y,
                    Prim::FSub => x - y,
                    Prim::FMul => x * y,
                    Prim::FDiv => x / y,
                    _ => f64::NAN,
                };
                // A `Float32` result rounds through binary32 (`as f32 as f64`), the fixed narrower mode;
                // `Float64` computes directly. Both round-to-nearest-even (the IEEE default wasm uses).
                if width == 32 { r as f32 as f64 } else { r }
            };
            let (x, y) = if width == 32 {
                (
                    f64::from_bits(a.to_f64_bits()) as f32 as f64,
                    f64::from_bits(b.to_f64_bits()) as f32 as f64,
                )
            } else {
                (
                    f64::from_bits(a.to_f64_bits()),
                    f64::from_bits(b.to_f64_bits()),
                )
            };
            let result = fold_at(x, y);
            match crate::ast::Decimal::from_f64(result) {
                Some(d) => {
                    trace!(target: "rcdzc::lower", op = intrinsic_name(op), width, "folded constant float op");
                    Core::ConstFloat(d)
                }
                // A non-finite result (overflow → ±inf, 0.0/.0 → NaN) has no written value form. This is
                // a PERMANENT correct-reject (one of the operator-ruled sound refusals), not a not-yet
                // feature gap — so per seq-32 it is a CODED REJECTION, not a codeless decline. Reuse the
                // CDZ0201 the non-finite float LITERAL sibling gives (resolve.rs, `Leaf::FloatInf`/`FloatNan`
                // "non-finite float value has no source literal form"): the same fault, surfacing from a
                // const FOLD instead of a source atom. (Reclassified off a codeless decline for the
                // v-cdz-smith reachability fuzz #6878; verdict from v-deferral-declines.)
                None => Core::Poison(Reject::coded(
                    crate::diag::Code::Malformed,
                    "a floating-point operation whose constant result is not finite (±inf or NaN) \
                     has no written value form",
                )),
            }
        }
        // A runtime float operand — emit the machine `f64.add`/`f32.add`/… at selection (the op's width
        // read off the solved type there, like the integer `Core::Arith`). Float ops never trap, so no
        // overflow guard — just the two operands + the machine op. A poison operand already returned above.
        _ => Core::Arith {
            op,
            lhs: args[0],
            rhs: args[1],
        },
    }
}

/// Lower a `Float64.of-int` / `Float32.of-int` — the TOTAL int→float conversion `Int64 → (Float N)`.
/// FOLD a constant integer to a `Core::ConstFloat` (the value as f64/f32, rounding to the nearest
/// representable float at the target width — total, never trapping); a runtime integer emits
/// `Core::Convert{op: FloatOfInt}` (select → `f{64,32}.convert_i64_s`). The target width is the node's
/// solved `Ty::Float`. No implicit promotion — the conversion is always written (numeric-model.md §A
/// Conversion Involving A Floating-Point Type Is Explicit).
pub(super) fn lower_float_of_int(db: &mut Db, id: StructId, args: &[StructId]) -> Core {
    if args.len() != 1 {
        return Core::Poison(Reject::coded(
            Code::Malformed,
            "of-int takes exactly 1 operand".to_string(),
        ));
    }
    let width = match crate::infer::type_of(db, id) {
        crate::ty::Ty::Float(ft) => ft.ground_width(),
        _ => {
            return Core::Poison(Reject::decline(
                "a float conversion target is not a definite float type",
            ));
        }
    };
    match core_of(db, args[0]) {
        Core::Poison(r) => Core::Poison(r),
        Core::ConstInt(v) => {
            // Fold: the integer's value as a float at the target width (round-to-nearest-even). A value
            // beyond the finite float range would round to ±inf — but an `Int64` is always finite in f64
            // (|Int64| < 2^63 ≪ f64 max), and f32 of an Int64 is finite too, so this never overflows.
            let Some(i) = v.to_i64() else {
                // A BigInt-magnitude constant (>i64) has no Int64 conversion source here — decline.
                return Core::Poison(Reject::decline(
                    "of-int of a value wider than Int64 is not supported",
                ));
            };
            let f = if width == 32 {
                i as f32 as f64
            } else {
                i as f64
            };
            match crate::ast::Decimal::from_f64(f) {
                Some(d) => {
                    trace!(target: "rcdzc::lower", width, "folded constant of-int to a float");
                    Core::ConstFloat(d)
                }
                None => Core::Poison(Reject::decline(
                    "a float conversion whose result is not finite has no value form",
                )),
            }
        }
        // A runtime integer operand — emit the machine int→float convert at selection (target width read
        // off the solved type there). Total, so no guard.
        _ => Core::Convert {
            op: Prim::FloatOfInt,
            operand: args[0],
        },
    }
}

/// Lower a `Float64.of` / `Float32.of` — the TOTAL float-WIDTH conversion `Float M → (Float N)` (promote
/// / demote / identity). FOLD a constant float in TWO steps: (1) read the source AT THE SOURCE OPERAND'S
/// OWN width (`const_float_bits_at_operand_width` — a `Float32` source literal IS its binary32 value, so
/// demote before converting; a `Float64` source is unchanged), then (2) round to the TARGET width (this
/// node's solved `Ty::Float`): narrowing rounds to nearest under the fixed mode, a promote/identity of
/// the source-width value is exact. Step (1) is what makes a WIDEN of a `Float32` source correct
/// (`(Float64.of (: 0.1 Float32))` promotes `0.1f32`, not the un-demoted f64 literal — the adv-61
/// fold-precision class in the width conversion). A runtime float emits `Core::Convert{op:FloatOf}`
/// (select → demote/promote/nothing). Total — a float always has an image at another float width.
pub(super) fn lower_float_of(db: &mut Db, id: StructId, args: &[StructId]) -> Core {
    if args.len() != 1 {
        return Core::Poison(Reject::coded(
            Code::Malformed,
            "of takes exactly 1 operand".to_string(),
        ));
    }
    let width = match crate::infer::type_of(db, id) {
        crate::ty::Ty::Float(ft) => ft.ground_width(),
        _ => {
            return Core::Poison(Reject::decline(
                "a float conversion target is not a definite float type",
            ));
        }
    };
    match core_of(db, args[0]) {
        Core::Poison(r) => Core::Poison(r),
        Core::ConstFloat(d) => {
            // Read the source value AT THE SOURCE OPERAND'S OWN width first — a `Float32` source literal IS
            // its binary32 value, so demote before converting (else a widen/identity `Float64.of` of a
            // `Float32` promotes the un-demoted f64 payload the reader parsed, diverging from the runtime
            // `f32.promote` of the real f32 slot: `(Float64.of (: 0.1 Float32))` → 0.1 instead of
            // 0.10000000149011612 — the adv-61 fold-precision class, here in the width conversion).
            let src = f64::from_bits(const_float_bits_at_operand_width(
                db,
                args[0],
                d.to_f64_bits(),
            ));
            // Then round the source value to the TARGET width: `as f32 as f64` for a Float32 target
            // (narrowing rounds to nearest binary32), the value unchanged for a Float64 target
            // (a promote/identity is exact). Rounding once at the target width matches the runtime op.
            let rounded = if width == 32 { src as f32 as f64 } else { src };
            match crate::ast::Decimal::from_f64(rounded) {
                Some(nd) => {
                    trace!(target: "rcdzc::lower", width, "folded constant float-width conversion");
                    Core::ConstFloat(nd)
                }
                None => Core::Poison(Reject::decline(
                    "a float conversion whose result is not finite has no value form",
                )),
            }
        }
        // A runtime float operand — emit the machine demote/promote at selection (source + target widths
        // read off the solved types there). Total, no guard.
        _ => Core::Convert {
            op: Prim::FloatOf,
            operand: args[0],
        },
    }
}

/// Apply a SAFE algebraic identity to a runtime arithmetic op with ONE constant operand, returning the
/// simplified core (the runtime operand's own core, or a constant) — or `None` when no identity applies
/// and the op stays a runtime `Arith`. `lc`/`rc` are the already-lowered operand cores; `lhs`/`rhs`
/// their AST occurrences. Every identity here is exact at EVERY width and never CHANGES the value; the
/// PASSTHROUGH identities keep the runtime operand (so its own traps still fire), while the ANNIHILATOR
/// identities (`x*0`, `x&0` → `0`) DISCARD the operand and so are applied ONLY when the discarded
/// operand cannot trap (`is_trap_free`) — else eliding it would drop a defined trap (`(* (/ a b) 0)`
/// must still trap on `b==0`; `numeric-model.md`/§div traps are defined outcomes, not to be optimized
/// away). Applied identities:
///  - `x + 0` = `0 + x` = `x - 0` = `x` (adding/subtracting 0 never overflows; keeps x);
///  - `x * 1` = `1 * x` = `x` (keeps x); `x * 0` = `0 * x` = `0` (ONLY if x is trap-free — discards x);
///  - `x | 0` = `0 | x` = `x ^ 0` = `0 ^ x` = `x` (keeps x); `x & 0` = `0 & x` = `0` (trap-free x only);
///  - `x << 0` = `x >> 0` = `x` (a zero shift COUNT is a no-op — count is the RIGHT operand; keeps x).
///
/// Deliberately NOT applied HERE: `0 - x` (negation traps at MIN), `x & allbits` (all-ones is width-
/// dependent), `0 << x` / `0 >> x` (a non-constant count must still trap if out of range). NOTE: the
/// STRENGTH REDUCTION `x * 2^k → x << k` is not a value-identity (it rewrites the op, not elides it), so
/// it lives at the SELECTION tier (`emit`'s `Core::Arith` Mul arm → `emit_mul_pow2_as_shift`), where the
/// shift's cheaper round-trip overflow check replaces the mul's division-based one — sound because a
/// left shift is EXACT multiplication by a power of two with the SAME defined overflow-trap
/// (`numeric-model.md` §Overflow Is Defined for shifts).
pub(super) fn arith_identity(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    lc: &Core,
    rhs: StructId,
    rc: &Core,
) -> Option<Core> {
    // A constant operand's value tested against a small literal (0 or 1), by value (magnitude-agnostic).
    let is =
        |c: &Core, k: i64| matches!(c, Core::ConstInt(v) if v.eq_value(&IntValue::from_i64(k)));
    let zero = || Core::ConstInt(IntValue::from_i64(0));
    match op {
        // `x + 0` / `0 + x` → x.
        Prim::Add if is(rc, 0) => Some(lc.clone()),
        Prim::Add if is(lc, 0) => Some(rc.clone()),
        // `x - 0` → x. (`0 - x` is negation — NOT an identity, would need a trap-checked negate.)
        Prim::Sub if is(rc, 0) => Some(lc.clone()),
        // `x * 1` / `1 * x` → x (keeps x).
        Prim::Mul if is(rc, 1) => Some(lc.clone()),
        Prim::Mul if is(lc, 1) => Some(rc.clone()),
        // `x * 0` / `0 * x` → 0 — DISCARDS x, so only when x cannot trap.
        Prim::Mul if is(rc, 0) && is_trap_free(db, lhs) => Some(zero()),
        Prim::Mul if is(lc, 0) && is_trap_free(db, rhs) => Some(zero()),
        // `x * -1` / `-1 * x` → NEGATION `(- 0 x)` — a strength reduction. A full-width `* -1` keeps the
        // expensive `div_s` round-trip overflow guard (the constant-multiplier fast path EXCLUDES `-1`,
        // since its `MIN/-1` bound is not i64-representable), but negation `0 - x` has the SAME single
        // overflow — `x == MIN` (`-MIN` is unrepresentable) — emitted as one `eq` check (the negation fast
        // path in `emit_machine_overflow_guard`). Value- AND trap-identical: `x * -1 == -x`, both overflow
        // iff `x == MIN`. Rewrite to `(- 0 x)`, synthesizing the `0` (the `Leaf::Int` node-synth pattern);
        // `x` STAYS an operand (the subtrahend), so its OWN traps are preserved — no `is_trap_free` guard
        // needed (unlike `* 0`, which discards `x`). (A NARROW `* -1` already sheds the div via the
        // narrow-product-fits-slot path, so this mainly helps full width — but the rewrite is correct at
        // every width: `0 - x` narrows/range-checks exactly as the narrow `* -1` result does.)
        Prim::Mul if is(rc, -1) || is(lc, -1) => {
            let x = if is(rc, -1) { lhs } else { rhs };
            let z = db.push_atom(crate::ast::Leaf::Int {
                value: IntValue::from_i64(0),
                radix: crate::ast::Radix::Dec,
            });
            trace!(target: "rcdzc::fold", "x * -1 → negation (- 0 x)");
            Some(Core::Arith {
                op: Prim::Sub,
                lhs: z,
                rhs: x,
            })
        }
        // WRAPPING arithmetic has the SAME algebraic identities as checked `+`/`*` — the wrap is total,
        // so it never traps and the fold is value-identical (`a +% 0 = a`, `a *% 1 = a`, `a *% 0 = 0`).
        // The keeping folds preserve the surviving operand's traps; the annihilator `*% 0` DISCARDS the
        // other operand, so it too is guarded on trap-freedom (`(/ x 0) *% 0` must still trap).
        Prim::WrappingAdd if is(rc, 0) => Some(lc.clone()),
        Prim::WrappingAdd if is(lc, 0) => Some(rc.clone()),
        // `a -% 0 = a` — the RIGHT-zero identity only (subtraction is not commutative, so `0 -% a` is the
        // negation of `a`, NOT `a`, and does not simplify here).
        Prim::WrappingSub if is(rc, 0) => Some(lc.clone()),
        Prim::WrappingMul if is(rc, 1) => Some(lc.clone()),
        Prim::WrappingMul if is(lc, 1) => Some(rc.clone()),
        Prim::WrappingMul if is(rc, 0) && is_trap_free(db, lhs) => Some(zero()),
        Prim::WrappingMul if is(lc, 0) && is_trap_free(db, rhs) => Some(zero()),
        // `x | 0` / `0 | x` / `x ^ 0` / `0 ^ x` → x.
        Prim::BitOr | Prim::BitXor if is(rc, 0) => Some(lc.clone()),
        Prim::BitOr | Prim::BitXor if is(lc, 0) => Some(rc.clone()),
        // `x & 0` / `0 & x` → 0 — DISCARDS x, so only when x cannot trap.
        Prim::BitAnd if is(rc, 0) && is_trap_free(db, lhs) => Some(zero()),
        Prim::BitAnd if is(lc, 0) && is_trap_free(db, rhs) => Some(zero()),
        // `x & M` / `M & x` → x when the constant `M` has ALL of `x`'s value bits set — a redundant mask.
        // An UNSIGNED width-N `x` lives in `[0, 2^N)`, so if `M`'s low N bits are all 1s the `&` cannot
        // clear anything (`x & M == x`). Restricted to UNSIGNED: a SIGNED value's slot high bits are sign
        // extension, which a mask WOULD clear (changing negatives), so `& fullmask` is not the identity
        // there. `x` keeps its own traps (the operand is returned). `x` is the value operand, `M` the
        // constant — `(& x M)` returns `lc` (x) when `rc` (M) masks x's whole width; symmetric for `(& M x)`.
        Prim::BitAnd if is_full_mask_for(db, lhs, rc) => Some(lc.clone()),
        Prim::BitAnd if is_full_mask_for(db, rhs, lc) => Some(rc.clone()),
        // OR-THEN-MASK ABSORPTION: `(& (| v C1) C2)` → `C2` when `C2 ⊆ C1` (`C2 & C1 == C2`). The inner OR
        // sets every bit of C1 (⊇ C2), the outer mask keeps only C2's bits — all of which are 1 — so the
        // result is exactly the constant C2, independent of `v`. `(& (| x 15) 15)` → 15. DISCARDS `v`, so
        // gated on `is_trap_free`. `C2` is the constant operand of the outer `&`; the inner `(| v C1)` is
        // the other. Both operand orders of the outer `&` are tried.
        Prim::BitAnd
            if let Core::ConstInt(c2) = rc
                && let Some(c2v) = c2.to_i64()
                && let Some(v) = or_then_mask_absorbs(db, lhs, c2v)
                && is_trap_free(db, v) =>
        {
            Some(rc.clone())
        }
        Prim::BitAnd
            if let Core::ConstInt(c2) = lc
                && let Some(c2v) = c2.to_i64()
                && let Some(v) = or_then_mask_absorbs(db, rhs, c2v)
                && is_trap_free(db, v) =>
        {
            Some(lc.clone())
        }
        // `x | M` / `M | x` → M when the constant `M` covers ALL of `x`'s value bits — the OR-SATURATION
        // dual of the `&`-mask elision. `x | M` sets every bit of M plus x's bits; if M already has all the
        // bits x could set (`is_full_mask_for`: x nonneg in `[0, 2^B)`, M's low B bits all 1), the OR adds
        // nothing NEW and the result is exactly M (`(| x 255)` with `x ∈ [0,255]` → 255). DISCARDS x (the
        // result is the constant M, not x), so — like `& 0`/`* 0` — only when x is TRAP-FREE (a trapping x
        // must still trap). Returns the CONSTANT operand's core (M). Same `is_full_mask_for` predicate as
        // the `&` fold, so it too fires on an emit-refined range via the emit-time sibling.
        Prim::BitOr if is_full_mask_for(db, lhs, rc) && is_trap_free(db, lhs) => Some(rc.clone()),
        Prim::BitOr if is_full_mask_for(db, rhs, lc) && is_trap_free(db, rhs) => Some(lc.clone()),
        // XOR CANCELLATION: `(^ (^ v w) w)` → `v` — the two XORs by the SAME `w` (constant OR runtime)
        // cancel (`w ^ w == 0`, and `v ^ 0 == v`). Handled BEFORE the nested-bitwise collapse so the
        // constant case produces `v` DIRECTLY rather than a residual `(^ v 0)` the collapse would leave
        // (which does not re-simplify). DISCARDS `w`, so gated on `is_trap_free(w)`. `v` stays, traps kept.
        Prim::BitXor
            if let Some((v, w)) = xor_cancels(db, lhs, rhs)
                && is_trap_free(db, w) =>
        {
            trace!(target: "rcdzc::fold", node = v.0, "XOR cancellation (^ (^ v w) w) → v");
            Some(core_of(db, v))
        }
        // IDEMPOTENT-BITWISE COLLAPSE: `(OP (OP v w) w)` → `(OP v w)` for `&`/`|` (idempotent: `w OP w == w`,
        // so re-applying `OP w` changes nothing), where the outer operand is `core_equiv` to `w` in the
        // inner op. Covers a RUNTIME `w` (`(| (| x y) y)` → `(| x y)`); the CONSTANT case already collapses
        // via `nested_bitwise_collapse` (`(| x (w|w))` = `(| x w)`). Unlike XOR-cancel, this KEEPS the inner
        // `(OP v w)` node — BOTH operands survive — so NO `is_trap_free` guard is needed (any trap in `v`/`w`
        // is still evaluated). Returns the inner node's core. `nested_shift_combine` — not `nested_bitwise_
        // collapse` — placement before it is fine (the collapse only fires on a CONSTANT operand, distinct
        // from this same-runtime-operand shape). `idempotent_bitwise_collapse` returns the inner node.
        Prim::BitAnd | Prim::BitOr
            if let Some(inner) = idempotent_bitwise_collapse(db, op, lhs, rhs) =>
        {
            trace!(target: "rcdzc::fold", node = inner.0, ?op, "idempotent bitwise (OP (OP v w) w) → (OP v w)");
            Some(core_of(db, inner))
        }
        // ABSORPTION LAW: `x & (x | y)` → `x` and `x | (x & y)` → `x` — a value combined with the DUAL op of
        // itself-with-anything absorbs to itself. The outer op is `&`/`|` and one operand is an inner op of
        // the DUAL kind (`| ` under `&`, `&` under `|`) that CONTAINS `x` (either side); the OTHER outer
        // operand is `x` (`core_equiv`). Result is `x`. DISCARDS the inner op's OTHER operand `y`, so gated
        // on `is_trap_free(y)` (a trapping `y` must still trap). `x` is returned so its own traps stay. Both
        // outer orders and both inner-operand positions are tried by `absorption_operand`.
        Prim::BitAnd | Prim::BitOr
            if let Some((x, y)) = absorption_operand(db, op, lhs, rhs)
                && is_trap_free(db, y) =>
        {
            trace!(target: "rcdzc::fold", node = x.0, ?op, "absorption law (x OP (x DUAL y)) → x");
            Some(core_of(db, x))
        }
        // NESTED-BITWISE COLLAPSE: `(OP (OP v C1) C2)` → `(OP v (C1 ⊙ C2))` for a TOTAL, ASSOCIATIVE
        // bitwise op — `&`/`|`/`^`. Two constant operations on the same value collapse to ONE by folding
        // the constants (`(& (& x 255) 15)`→`(& x 15)`, `(| (| x 5) 3)`→`(| x 7)`, `(^ (^ x 5) 3)`→`(^ x
        // 6)`); the `&` case's folded constant also enables downstream range folds. `v` keeps its own
        // traps (it stays the operand). Guarded on the shape (`nested_bitwise_collapse` returns `None`
        // when it does not apply) so the same-operand `& a a`/`| a a` fold below still fires. Verified
        // value-identical: each op is associative, so `(v OP C1) OP C2 == v OP (C1 ⊙ C2)`.
        Prim::BitAnd | Prim::BitOr | Prim::BitXor
            if let Some(folded) = nested_bitwise_collapse(db, op, lhs, lc, rhs, rc) =>
        {
            Some(folded)
        }
        // `x << 0` / `x >> 0` → x (a zero shift COUNT is a no-op; count is the right operand).
        Prim::Shl | Prim::Shr if is(rc, 0) => Some(lc.clone()),
        // NESTED SHIFT COLLAPSE: `(SH (SH v A) B)` → `(SH v (A+B))` for the SAME shift direction, A, B
        // constants, A+B < width. A RIGHT shift is TOTAL — shifting right by A then B drops the same low
        // A+B bits as one shift by A+B (both `>>ₛ` sign-fill and `>>ᵤ` zero-fill; the inner and outer `>>`
        // on the same-typed value are the same kind, so composing is exact). A LEFT shift is CHECKED (it
        // is exact `·2^count`, trapping on N-bit overflow) but STILL collapses trap-identically: magnitude
        // is MONOTONIC in the count, so `(v<<A)<<B` overflows on exactly the inputs `v<<(A+B)` does (inner
        // overflow ⟹ combined overflow, and combined overflow ⟹ the double's outer step overflows) — same
        // value `v·2^(A+B)` when neither traps, same trap set otherwise. Bounded by A+B < width for BOTH:
        // a combined count ≥ width is masked mod width by the machine op (wrong), and for `<<` it must also
        // TRAP as an out-of-range count — so only the in-range sum is faithful. `v` keeps its own traps (it
        // stays the operand). Guarded via `nested_shift_combine`.
        Prim::Shr | Prim::Shl if let Some(folded) = nested_shift_combine(db, op, lhs, rc) => {
            Some(folded)
        }
        // `(>>ᵤ x k)` → 0 when the LOGICAL right shift drops ALL of `x`'s significant bits — its provable
        // bit-bound `B <= k`. E.g. `(x & 15) >>ᵤ 4`: `x & 15` fits 4 bits, `>>ᵤ 4` shifts them all out → 0.
        // DISCARDS `x`, so gated on `is_trap_free` (a trapping operand's trap must survive). `k` must be a
        // valid IN-RANGE constant count (`< width`) — an out-of-range shift TRAPS rather than yielding 0,
        // so a too-large `k` is left for the runtime count-guard. `unsigned_value_bits` returns the bound
        // only for an unsigned logical-shift chain, so this never misfires on a signed `>>ₛ` (which
        // sign-extends, not zero-fills).
        Prim::Shr
            if is_trap_free(db, lhs)
                && let Core::ConstInt(k) = rc
                && let Some(k) = k.to_i64()
                && k >= 1
                && let Some(bits) = unsigned_value_bits(db, lhs)
                && (k as u32) < shift_width(db, lhs)
                && bits <= k as u32 =>
        {
            trace!(target: "rcdzc::fold", node = lhs.0, k, bits, "logical shift drops all significant bits → 0");
            Some(zero())
        }
        // `x / 1` → x (division by one is the identity; keeps x, so its own traps stay).
        Prim::Div if is(rc, 1) => Some(lc.clone()),
        // `x % 1` → 0 (every integer is divisible by 1) — DISCARDS x, so only when x cannot trap.
        Prim::Rem if is(rc, 1) && is_trap_free(db, lhs) => Some(zero()),
        // DIVIDEND-SMALLER-THAN-DIVISOR: when `x` is provably in `[0, C-1]` for a POSITIVE constant divisor
        // `C`, the truncating `x / C` is 0 and `x % C` is `x` — the divisor is too big to divide `x` even
        // once. `(/ (& x 7) 100)` → 0, `(% (& x 7) 100)` → `x & 7` (a masked/refined value modding by a
        // larger constant). Requires `x` NONNEGATIVE with a known upper bound `< C` (`value_range` lo ≥ 0,
        // hi < C) so truncation-toward-zero equals the mathematical result; a negative `x` (`-1 % 100 =
        // -1`, `-1 / 100 = 0`) is excluded for simplicity (the nonneg case is the masked/unsigned idiom).
        // The `/` DISCARDS `x` → gated on `is_trap_free`; the `%` KEEPS `x` (returns `lc`) so its traps
        // survive. `C ≥ 2` (the `/1`,`%1` identities above handle `C=1`; a constant `÷0` is a poison in
        // `lower` before here). Verified: for `0 ≤ x < C`, `x/C == 0` and `x%C == x`.
        Prim::Div
            if let Core::ConstInt(c) = rc
                && let Some(c) = c.to_i64()
                && c >= 2
                && dividend_below_divisor(db, lhs, c)
                && is_trap_free(db, lhs) =>
        {
            trace!(target: "rcdzc::fold", node = lhs.0, c, "dividend provably < divisor → x / C = 0");
            Some(zero())
        }
        Prim::Rem
            if let Core::ConstInt(c) = rc
                && let Some(c) = c.to_i64()
                && c >= 2
                && dividend_below_divisor(db, lhs, c) =>
        {
            trace!(target: "rcdzc::fold", node = lhs.0, c, "dividend provably < divisor → x % C = x");
            Some(lc.clone())
        }
        // NESTED-MODULO COLLAPSE: `(% (% v M) N)` → `(% v N)` when the outer divisor `N` DIVIDES the inner
        // `M` (`M % N == 0`), both positive constants. Since `M` is a multiple of `N`, reducing mod `M`
        // first then mod `N` gives the same residue as reducing mod `N` directly — for truncated (toward-
        // zero) division at every sign of `v` (`(x%100)%10 == x%10`, incl. negatives: `-25%100=-25`,
        // `-25%10=-5`, and `-25%10=-5` directly). One `rem` instead of two. `v` STAYS the operand of the
        // outer `% N`, so its own traps (and the outer `% N`'s ÷0 — impossible here, N≥2) are preserved; no
        // `is_trap_free` needed. Both divisors must be constants ≥ 2 and `N | M`.
        Prim::Rem
            if let Core::ConstInt(n) = rc
                && let Some(n) = n.to_i64()
                && n >= 2
                && let Core::Arith {
                    op: Prim::Rem,
                    lhs: v,
                    rhs: inner_div,
                } = core_of(db, lhs)
                && let Core::ConstInt(mm) = core_of(db, inner_div)
                && let Some(m) = mm.to_i64()
                && m >= 2
                && m % n == 0 =>
        {
            trace!(target: "rcdzc::fold", inner_m = m, outer_n = n, "nested modulo (% (% v M) N) → (% v N) (N | M)");
            Some(Core::Arith {
                op: Prim::Rem,
                lhs: v,
                rhs,
            })
        }

        // COMPLEMENT LAWS: `x & ~x` → 0 and `x | ~x` → -1 (all-ones), where `~x` is `(^ x -1)` (there is no
        // dedicated bit-NOT prim). A value AND its bitwise complement share NO set bit, so `&` is 0 and `|`
        // is every bit set. Both DISCARD `x` (the result does not depend on it), so gated on
        // `is_trap_free(x)` — a trapping `x` must still trap. `complement_pair` matches `v` against
        // `(^ v -1)` on either operand order.
        //
        // The `&` result 0 is valid at EVERY width/sign. But the `|` all-ones result is `-1` only for a
        // SIGNED type; an UNSIGNED width-N all-ones is `2^N − 1`, and a literal `-1` is OUT OF RANGE there
        // (`(: -1 UInt8)` is a CDZ0302 reject) — `arith_identity` has no width to synthesize `2^N−1`. So the
        // `|` fold is restricted to a SIGNED operand type, where `-1` IS the all-ones and representable;
        // an unsigned `x | ~x` keeps the runtime `or` (correct, just not folded).
        Prim::BitAnd if complement_pair(db, lhs, rhs).is_some_and(|v| is_trap_free(db, v)) => {
            trace!(target: "rcdzc::fold", "complement law x & ~x → 0");
            Some(zero())
        }
        Prim::BitOr
            if matches!(crate::infer::type_of(db, lhs), crate::ty::Ty::Int(it) if it.ground_signed())
                && complement_pair(db, lhs, rhs).is_some_and(|v| is_trap_free(db, v)) =>
        {
            trace!(target: "rcdzc::fold", "complement law x | ~x → -1 (all ones, signed)");
            Some(Core::ConstInt(IntValue::from_i64(-1)))
        }
        // SAME-OPERAND identities: the two operands are the SAME value (`core_equiv`), so the result is
        // determined regardless of that value. `core_equiv` matches only pure scalar cores, but the
        // operand may still be a checked op that TRAPS (`(- (/ a b) (/ a b))` — the `/` traps on b==0),
        // so a DISCARDING identity (`- a a → 0`, `^ a a → 0`) fires only when the operand is trap-free;
        // eliding a possibly-trapping operand would drop a defined trap. The KEEPING identities
        // (`& a a → a`, `| a a → a`) return the operand's own core, so its traps are preserved — always
        // safe. (`/ a a → 1` is NOT applied: `a == 0` traps ÷0, a defined outcome, so it is not an
        // identity.)
        Prim::Sub | Prim::BitXor if core_equiv(db, lhs, rhs) && is_trap_free(db, lhs) => {
            Some(zero())
        }
        Prim::BitAnd | Prim::BitOr if core_equiv(db, lhs, rhs) => Some(lc.clone()),
        _ => None,
    }
}

/// For an outer bitwise op with operands `(lhs, rhs)`, whether one operand is the bitwise COMPLEMENT of
/// the other — i.e. one is `v` and the other is `(^ v -1)` (`~v`). Returns the un-complemented value `v`
/// (so the caller can trap-check it, since the complement laws `x & ~x = 0` / `x | ~x = -1` DISCARD `x`).
/// Both operand orders are tried, and the `-1` may be on either side of the inner XOR. `None` otherwise.
pub(super) fn complement_pair(db: &mut Db, lhs: StructId, rhs: StructId) -> Option<StructId> {
    // Whether `maybe_not` is `(^ v -1)` for a `v` that is `core_equiv` to `other`.
    let is_not_of = |db: &mut Db, maybe_not: StructId, other: StructId| -> bool {
        let Core::Arith {
            op: Prim::BitXor,
            lhs: il,
            rhs: ir,
        } = core_of(db, maybe_not)
        else {
            return false;
        };
        let is_neg1 =
            |c: &Core| matches!(c, Core::ConstInt(v) if v.eq_value(&IntValue::from_i64(-1)));
        // `(^ v -1)` — the `-1` on the right (`v` = il) or left (`v` = ir), and `v` matches `other`.
        (is_neg1(&core_of(db, ir)) && core_equiv(db, il, other))
            || (is_neg1(&core_of(db, il)) && core_equiv(db, ir, other))
    };
    if is_not_of(db, rhs, lhs) {
        Some(lhs) // `(op v (^ v -1))`
    } else if is_not_of(db, lhs, rhs) {
        Some(rhs) // `(op (^ v -1) v)`
    } else {
        None
    }
}

/// Whether `lhs`/`rhs` are BOOLEAN complements — one is `v` and the other is `(not v)` (`Core::Not` whose
/// operand is `core_equiv` to the first). The `and`/`or` analogue of the bitwise `complement_pair`; drives
/// the boolean complement laws `(and a (not a))` → false, `(or a (not a))` → true. Both operand orders are
/// tried. (`Core::Not` is `lower`'s canonical boolean negation — a `(not (not a))` already cancelled in the
/// `Resolved::Not` fold, so a `Not` here wraps a non-`Not` operand.)
pub(super) fn bool_complement_pair(db: &mut Db, lhs: StructId, rhs: StructId) -> bool {
    let is_not_of = |db: &mut Db, maybe_not: StructId, other: StructId| -> bool {
        matches!(core_of(db, maybe_not), Core::Not { operand } if core_equiv(db, operand, other))
    };
    is_not_of(db, rhs, lhs) || is_not_of(db, lhs, rhs)
}

/// Fold a SHORT-CIRCUIT CONNECTIVE `(and/or lhs rhs)` (the `is_and` flag selects) into its simplest core.
/// Shared by the `Resolved::And` arm AND the `(if c a false)`→`(and c a)` / `(if c true b)`→`(or c b)`
/// if-encoded-connective rewrites — an if-shaped connective routes through the SAME boolean-algebra fold
/// family (constant short-circuit, idempotence, absorption, complement, and the comparison-pair folds).
///
/// A constant LEFT operand short-circuits WITHOUT evaluating `rhs` (a trapping/ill-formed `rhs` is
/// shielded, exactly as an `if`'s unselected branch): `(and false _)`→false, `(and true rhs)`→rhs;
/// `(or true _)`→true, `(or false rhs)`→rhs. Otherwise `lhs` is the always-evaluated short-circuit
/// condition; the arms below simplify against a constant/structural `rhs`, and the fallthrough emits a
/// `Core::And` the backend lowers to `if lhs then/else <rhs|const>`.
pub(super) fn fold_short_circuit(db: &mut Db, lhs: StructId, rhs: StructId, is_and: bool) -> Core {
    match core_of(db, lhs) {
        Core::ConstBool(b) => {
            // `and`: left decides when false (short-circuit to false), else the result is rhs.
            // `or`:  left decides when true  (short-circuit to true),  else the result is rhs.
            if b == is_and {
                core_of(db, rhs) // and-true → rhs ; or-false → rhs
            } else {
                Core::ConstBool(!is_and) // and-false → false ; or-true → true
            }
        }
        Core::Poison(r) => Core::Poison(r),
        // A constant RIGHT operand (the left is a non-constant runtime bool, ALWAYS evaluated — it is
        // the short-circuit condition). `(and p true)` / `(or p false)` → `p` (the neutral element,
        // keeps `p` so its effects/traps stay). `(and p false)` → `false` / `(or p true)` → `true`
        // (the ABSORBING element) — this DISCARDS `p`, so it is applied only when `p` is trap-free
        // (else `p`'s trap must still fire, so keep the `Core::And`). Mirrors the constant-left fold
        // above; completes the boolean-identity set. (Both-constant folded via the left arm already.)
        lc => match core_of(db, rhs) {
            Core::ConstBool(rb) if rb == is_and => lc, // and-true / or-false → p (neutral, keeps p)
            Core::ConstBool(_) if is_trap_free(db, lhs) => Core::ConstBool(!is_and), // absorbing
            // IDEMPOTENCE: `(and a a)` → `a` and `(or a a)` → `a` — a boolean combined with itself is
            // itself. The two operands are the SAME value (`core_equiv`), so the result is `a`. `lhs` is
            // the short-circuit condition, ALWAYS evaluated (and evaluated ONCE by returning its core),
            // so `a`'s own effects/traps are preserved regardless of the fold — no `is_trap_free` guard
            // needed (`lhs` runs exactly as it would as the condition; `rhs`, a re-evaluation of the
            // same pure value, is dropped). Mirrors the bitwise `(& a a)`/`(| a a)` same-operand fold.
            _ if core_equiv(db, lhs, rhs) => lc,
            // NESTED IDEMPOTENCE / ABSORPTION: `(and (and a b) a)` → `(and a b)` and `(or (or a b) a)` →
            // `(or a b)` — one operand is a nested SAME-connective `(and/or p q)` that already CONTAINS
            // the other operand (`p` or `q` is `core_equiv` to it), so re-conjoining/disjoining it is
            // redundant. Returns the nested node (all operands stay evaluated → trap-safe, like the
            // bitwise idempotent collapse c117). Only the SAME connective (`is_and` matches). Both outer
            // orders are tried by `bool_nested_idempotent`.
            _ if let Some(keep) = bool_nested_idempotent(db, lhs, rhs, is_and) => core_of(db, keep),
            // ABSORPTION LAW: `(and a (or a b))` → `a` and `(or a (and a b))` → `a` — a boolean combined
            // with the DUAL connective of itself-with-anything absorbs to itself (the short-circuit
            // analogue of the bitwise `x & (x|y)`→x / `x | (x&y)`→x fold, c118). One operand is an inner
            // `and`/`or` of the DUAL connective CONTAINING `x`; the other is `x`. Result is `x`. DISCARDS
            // the inner op's OTHER operand `y`, so gated on `is_trap_free(y)` — `y` is only conditionally
            // evaluated in the short-circuit original, so trap-freedom suffices to drop it. `x` is pure
            // (`core_equiv`) so returning it evaluates once with no trap. Both orders via
            // `bool_absorption_operand`.
            _ if let Some((x, y)) = bool_absorption_operand(db, lhs, rhs, is_and)
                && is_trap_free(db, y) =>
            {
                core_of(db, x)
            }
            // COMPLEMENT LAW: `(and a (not a))` → `false` and `(or a (not a))` → `true` — a boolean and
            // its negation are exhaustive+exclusive, so `and` is always false and `or` always true. The
            // boolean analogue of the bitwise `x & ~x`/`x | ~x` fold (c119). DISCARDS both operands (the
            // result is a constant), so gated on `is_trap_free(lhs)` — a trapping `a` must still trap
            // (`core_equiv` matches only pure cores, so `a` is pure anyway, but keep the guard explicit).
            // Both operand orders (`a`&`!a` / `!a`&`a`) are handled by `bool_complement_pair`.
            _ if bool_complement_pair(db, lhs, rhs) && is_trap_free(db, lhs) => {
                Core::ConstBool(!is_and) // and → false ; or → true
            }
            // COMPLEMENTARY-COMPARISON LAW: `(or (< a b) (>= a b))` → true, `(and (< a b) (>= a b))` →
            // false — two comparisons on the SAME operand PAIR whose operators are exact COMPLEMENTS
            // (`< `↔`>=`, `<=`↔`>`) partition the total order, so their `or` is exhaustive (always true)
            // and their `and` is exclusive (always false). A redundant range guard (`(or (< x c) (>= x
            // c))`). DISCARDS both operands, so gated on `is_trap_free` for each (a comparison is
            // trap-free iff its operands are; a `(< (/ a b) 5)` with a trapping `/` keeps the runtime
            // form). `complementary_comparisons` checks same-pair + complement-op.
            _ if complementary_comparisons(db, lhs, rhs)
                && is_trap_free(db, lhs)
                && is_trap_free(db, rhs) =>
            {
                Core::ConstBool(!is_and) // or → true ; and → false
            }
            // SUBSUMPTION: two comparisons on the SAME runtime operand `v` against constants with the
            // SAME operator (both `<`, both `<=`, both `>`, or both `>=`) — one implies the other, so
            // the redundant one drops. `and` keeps the STRONGER (tighter bound), `or` the WEAKER
            // (looser): `(and (< v 5) (< v 10))` → `(< v 5)`, `(or (< v 5) (< v 10))` → `(< v 10)`. The
            // kept comparison still evaluates `v` (its trap, if any, is preserved) — no operand is
            // dropped, only the redundant second bound. `subsuming_comparison` returns the occurrence to
            // keep (`lhs` or `rhs`).
            _ if let Some(keep) = subsuming_comparison(db, lhs, rhs, is_and) => core_of(db, keep),
            // COINCIDENT-POINT COLLAPSE: `(and (>= v c) (<= v c))` → `(= v c)` — two INCLUSIVE
            // opposite bounds pinning `v` to a single point ARE equality (`v>=c && v<=c ⟺ v==c`), so
            // three ops (`ge`+`le`+`and`) become one `eq`. Only under `and`; reuses the existing (proven
            // in-type) constant node, so no synthesis / no range guard. DISCARDS the second comparison,
            // so gated on `is_trap_free` for both (like the sibling disjoint/covering fold); the kept
            // `(= v c)` still evaluates `v`. Distinct from disjoint/covering (which folds `L>U` empty /
            // `L<=U+1` covering — the coincident `L==U` point is exactly what THIS fold handles).
            _ if is_and
                && let Some((v, c)) = coincident_point_eq(db, lhs, rhs)
                && is_trap_free(db, lhs)
                && is_trap_free(db, rhs) =>
            {
                Core::Compare {
                    op: Prim::Eq,
                    lhs: v,
                    rhs: c,
                }
            }
            // DISJOINT/COVERING INTERVAL: two comparisons on the SAME operand `v` vs constants forming
            // OPPOSITE-direction half-lines (one an upper bound `v ≤ U`, the other a lower bound `v ≥
            // L`). `and` (intersection `L ≤ v ≤ U`) is EMPTY iff `L > U` → `false`; `or` (union) COVERS
            // everything iff the half-lines touch/overlap (`L ≤ U+1`) → `true`. `(and (< x 5) (> x 10))`
            // → false, `(or (< x 5) (> x 3))` → true. Only the constant verdicts (a non-empty `and` /
            // gapped `or` is not a constant — kept). DISCARDS both operands, so gated on `is_trap_free`.
            _ if let Some(v) = disjoint_or_covering(db, lhs, rhs, is_and)
                && is_trap_free(db, lhs)
                && is_trap_free(db, rhs) =>
            {
                Core::ConstBool(v)
            }
            // EQUALITY-VS-RANGE: one operand is `(= x c)`, the other an ordering comparison `(cmp x k)`
            // on the SAME `x`. Whether `c` satisfies `(cmp c k)` (a compile-time test) decides:
            //   `and`: `sat` → `(= x c)` (the range is redundant given equality); `!sat` → `false`
            //          (equality contradicts the range). `(and (= x 5) (> x 0))` → `(= x 5)`,
            //          `(and (= x 5) (> x 100))` → false.
            //   `or`:  `sat` → `(cmp x k)` (equality is subsumed by the range it satisfies); `!sat` →
            //          keep both (not a constant — `x==c` adds one point outside the range).
            // Each DISCARDS one operand — gated on that operand's `is_trap_free`. `eq_vs_range` returns
            // `(eq_node, range_node, sat)`.
            _ if let Some((eq_node, range_node, sat)) = eq_vs_range(db, lhs, rhs) => {
                if is_and {
                    if sat && is_trap_free(db, range_node) {
                        core_of(db, eq_node) // range redundant → keep the equality
                    } else if !sat && is_trap_free(db, eq_node) && is_trap_free(db, range_node) {
                        Core::ConstBool(false) // contradiction
                    } else {
                        Core::And { lhs, rhs, is_and }
                    }
                } else if sat && is_trap_free(db, eq_node) {
                    core_of(db, range_node) // `or`: equality subsumed → keep the range
                } else {
                    Core::And { lhs, rhs, is_and }
                }
            }
            // REASSOCIATE TO EXPOSE A COMPARISON PAIR across a same-connective nested tree. The pairwise
            // comparison folds above only see the TWO DIRECT operands, so `(and (and (> x 0) (< x 100)) (> x
            // 5))` misses that `(> x 5)` subsumes the buried `(> x 0)`. When one operand is a same-connective
            // `(op P Q)` and the other is a comparison `C`, try folding `C` against `P` (and against `Q`) via
            // `fold_short_circuit`: if that pair COLLAPSES (to a constant or a single kept comparison — i.e.
            // NOT a plain two-operand `Core::And`), rebuild the tree with the collapsed result and the
            // remaining leaf. `(and (and (> x 0) (< x 100)) (> x 5))` → `(and (> x 5) (< x 100))`; a nested
            // COMPLEMENT `(and (and (< x y) …) (>= x y))` → false. SOUND only when every involved leaf (`C`,
            // `P`, `Q`) is TRAP-FREE: `and`/`or` is associative+commutative over pure booleans, so regrouping
            // and reordering is unobservable (no trap/effect order to preserve). `reassociate_comparison_pair`
            // returns the rebuilt `Core` or `None`.
            _ if let Some(folded) = reassociate_comparison_pair(db, lhs, rhs, is_and) => folded,
            _ => Core::And { lhs, rhs, is_and },
        },
    }
}

/// Reassociate a short-circuit `(op lhs rhs)` (connective `is_and`) to expose a COMPARISON PAIR that the
/// direct pairwise folds miss because it is split across a same-connective nested subtree. When one operand
/// is a nested `(op P Q)` (SAME connective) and the OTHER operand `C` is a comparison, this folds `C`
/// against `P` and against `Q` (via `fold_short_circuit`); if either pair COLLAPSES — the recursive fold
/// returns something OTHER than a plain two-operand `Core::And` of those same two nodes (a constant, or a
/// single subsuming comparison) — the whole tree is rebuilt as `(op collapsed remaining_leaf)`, dropping the
/// redundant comparison. `(and (and (> x 0) (< x 100)) (> x 5))` → `(and (> x 5) (< x 100))` (subsumption);
/// `(and (and (< x y) p) (>= x y))` → `false` (complement). Returns `None` when nothing collapses.
///
/// SOUNDNESS: fires ONLY when `C`, `P`, and `Q` are all TRAP-FREE. A short-circuit `and`/`or` over pure
/// (trap-free, effect-free) boolean operands is fully associative AND commutative — there is no evaluation
/// order or trap to preserve — so regrouping `(op (op P Q) C)` as `(op (op C P) Q)` and folding the exposed
/// `(op C P)` pair is behavior-identical. (A non-trap-free leaf could change WHICH branch's trap fires or
/// its order, so it is excluded — the tree stays as-is.) Both outer operand orders and both nested-operand
/// positions are tried.
pub(super) fn reassociate_comparison_pair(
    db: &mut Db,
    lhs: StructId,
    rhs: StructId,
    is_and: bool,
) -> Option<Core> {
    // Try: `nested` = a same-connective `(op P Q)`, `c` = the other operand (a comparison). Fold `c`
    // against each nested leaf; on a genuine collapse, rebuild `(op collapsed other_leaf)`.
    let try_side = |db: &mut Db, nested: StructId, c: StructId| -> Option<Core> {
        // `c` must be a comparison, and trap-free (a discarding/regrouping fold requires purity).
        if !matches!(core_of(db, c), Core::Compare { .. }) || !is_trap_free(db, c) {
            return None;
        }
        let Core::And {
            lhs: p,
            rhs: q,
            is_and: nested_is_and,
        } = core_of(db, nested)
        else {
            return None;
        };
        if nested_is_and != is_and {
            return None; // must be the SAME connective to reassociate
        }
        // Every leaf must be trap-free so the reassociation is unobservable.
        if !is_trap_free(db, p) || !is_trap_free(db, q) {
            return None;
        }
        // Fold `c` against P, keeping Q; then against Q, keeping P. A genuine collapse = the recursive fold
        // did NOT return a plain `Core::And` re-pairing the same two nodes (that would be no progress).
        let collapsed = |db: &mut Db, pair_a: StructId, pair_b: StructId| -> Option<Core> {
            let folded = fold_short_circuit(db, pair_a, pair_b, is_and);
            match folded {
                // No progress: the pair stayed a two-operand `and`/`or`. (Any other shape — ConstBool, a
                // single Compare, a Not, an Eq — is a real collapse.)
                Core::And { .. } => None,
                other => Some(other),
            }
        };
        if let Some(folded) = collapsed(db, c, p) {
            // `(op (op P Q) C)` → `(op folded(C,P) Q)`.
            let fid = synth_core(db, folded, crate::ty::Ty::Bool);
            return Some(fold_short_circuit(db, fid, q, is_and));
        }
        if let Some(folded) = collapsed(db, c, q) {
            // → `(op folded(C,Q) P)`.
            let fid = synth_core(db, folded, crate::ty::Ty::Bool);
            return Some(fold_short_circuit(db, fid, p, is_and));
        }
        None
    };
    try_side(db, lhs, rhs).or_else(|| try_side(db, rhs, lhs))
}

/// NESTED IDEMPOTENCE for a short-circuit `and`/`or`: when one outer operand is a nested `Core::And` of the
/// SAME connective (`is_and`) that already CONTAINS the other outer operand (one of its sides is
/// `core_equiv` to it), the outer re-application is redundant — `(and (and a b) a)` == `(and a b)`. Returns
/// the NESTED node to keep (all its operands stay evaluated → trap-safe, no operand dropped). Both outer
/// operand orders and both nested-operand positions are tried. `None` when the shape does not match.
pub(super) fn bool_nested_idempotent(
    db: &mut Db,
    lhs: StructId,
    rhs: StructId,
    is_and: bool,
) -> Option<StructId> {
    // `nested` is `(op p q)` with the SAME connective; `outer` must be `core_equiv` to `p` or `q`.
    let check = |db: &mut Db, nested: StructId, outer: StructId| -> Option<StructId> {
        let Core::And {
            lhs: p,
            rhs: q,
            is_and: nested_is_and,
        } = core_of(db, nested)
        else {
            return None;
        };
        if nested_is_and != is_and {
            return None;
        }
        (core_equiv(db, p, outer) || core_equiv(db, q, outer)).then_some(nested)
    };
    check(db, lhs, rhs).or_else(|| check(db, rhs, lhs))
}

/// The SHORT-CIRCUIT BOOLEAN ABSORPTION LAW: `(and a (or a b))` → `a` and `(or a (and a b))` → `a` (either
/// outer order, `a` on either side of the inner op). A boolean combined with the DUAL connective of
/// itself-with-anything absorbs to itself — the boolean analogue of the bitwise `x & (x|y)`→x / `x | (x&y)`
/// →x fold (c118, `absorption_operand`). The outer connective is `is_and`; one operand must be an inner
/// `Core::And` of the DUAL connective (`or` under `and`, `and` under `or`) that CONTAINS `x` (either side);
/// the OTHER outer operand is `x` (`core_equiv`). Returns `(x, y)` — the whole expression absorbs to `x`,
/// discarding the inner op's OTHER operand `y`. `x` is pure (`core_equiv` matches only pure cores) so
/// returning it evaluates it once with no trap; `y` may be arbitrary, so the caller gates `is_trap_free(y)`
/// (in the short-circuit original `y` is only conditionally evaluated, so trap-freedom is SUFFICIENT to
/// drop it soundly). Both outer orders and both inner-operand positions are tried.
pub(super) fn bool_absorption_operand(
    db: &mut Db,
    lhs: StructId,
    rhs: StructId,
    is_and: bool,
) -> Option<(StructId, StructId)> {
    // `inner` must be a `Core::And` of the DUAL connective; `outer_x` must be `core_equiv` to one operand.
    let check = |db: &mut Db, inner: StructId, outer_x: StructId| -> Option<(StructId, StructId)> {
        let Core::And {
            lhs: ip,
            rhs: iq,
            is_and: inner_is_and,
        } = core_of(db, inner)
        else {
            return None;
        };
        if inner_is_and == is_and {
            return None; // must be the DUAL connective (`or` under `and`, `and` under `or`)
        }
        if core_equiv(db, ip, outer_x) {
            Some((outer_x, iq)) // x matched ip → y is iq
        } else if core_equiv(db, iq, outer_x) {
            Some((outer_x, ip)) // x matched iq → y is ip
        } else {
            None
        }
    };
    check(db, lhs, rhs).or_else(|| check(db, rhs, lhs))
}

/// Whether `lhs`/`rhs` are two comparisons on the SAME operand pair whose operators are exact COMPLEMENTS
/// over the total order — `< `/`>=` or `<=`/`>` — so together they partition every value: their `or` is
/// always TRUE (exhaustive) and their `and` always FALSE (disjoint). `(or (< a b) (>= a b))` → true,
/// `(and (< a b) (>= a b))` → false. Requires BOTH to be `Core::Compare` with `core_equiv` operand pairs
/// (same order — `< a b` complements `>= a b`, NOT `>= b a`) and complementary ops. `=`/`Compare` are not
/// ordering complements and never match. Drives the complementary-comparison fold (caller trap-guards).
pub(super) fn complementary_comparisons(db: &mut Db, lhs: StructId, rhs: StructId) -> bool {
    let Core::Compare {
        op: lop,
        lhs: la,
        rhs: lb,
    } = core_of(db, lhs)
    else {
        return false;
    };
    let Core::Compare {
        op: rop,
        lhs: ra,
        rhs: rb,
    } = core_of(db, rhs)
    else {
        return false;
    };
    // Exact ordering complements: `<` ↔ `>=`, `<=` ↔ `>` (either assignment to lhs/rhs).
    let complement = matches!(
        (lop, rop),
        (Prim::Lt, Prim::Ge) | (Prim::Ge, Prim::Lt) | (Prim::Le, Prim::Gt) | (Prim::Gt, Prim::Le)
    );
    // Same operand pair in the SAME order (the operators already encode the direction).
    complement && core_equiv(db, la, ra) && core_equiv(db, lb, rb)
}

/// SUBSUMPTION between two comparisons on the SAME runtime operand `v` against CONSTANTS that form
/// SAME-DIRECTION half-lines (both upper bounds `v ≤ B`, or both lower `v ≥ B`) — one implies the other,
/// so `(and …)`/`(or …)` keeps just one. Returns the occurrence to KEEP (`lhs` or `rhs`), or `None` when
/// the pair is not two same-direction half-lines on the same `v`. `is_and` selects which survives: `and`
/// keeps the STRONGER (tighter) bound, `or` the WEAKER (looser).
///
/// Uses `comparison_halfline` to normalize each side to an INCLUSIVE bound (`v ≤ B` / `v ≥ B`), so MIXED
/// operators are handled uniformly — `(< v 5)` and `(<= v 4)` both normalize to `v ≤ 4` (and keeps either),
/// `(or (<= v 10) (< v 5))` → `v ≤ 10` (the looser). For two UPPER bounds the tighter is the SMALLER `B`;
/// for two LOWER bounds the tighter is the LARGER `B`. `comparison_halfline` already handles either operand
/// side (a mirrored `(< c v)` normalizes to a lower bound on `v`) and only the four ordering ops (`Eq`/
/// `Compare` are not half-lines, so a `(= x 5)`/`(= x 6)` pair returns `None` here — never mis-subsumed).
/// The kept comparison still evaluates `v`, so no trap drops. OPPOSITE-direction pairs are `None` here (the
/// disjoint/covering + coincident-point folds handle those).
pub(super) fn subsuming_comparison(
    db: &mut Db,
    lhs: StructId,
    rhs: StructId,
    is_and: bool,
) -> Option<StructId> {
    let (lv, l_upper, lb) = comparison_halfline(db, lhs)?;
    let (rv, r_upper, rb) = comparison_halfline(db, rhs)?;
    // Same operand, same direction (both upper or both lower) — an opposite-direction pair is a range, not
    // a subsumption (handled by `disjoint_or_covering`/`coincident_point_eq`).
    if l_upper != r_upper || !core_equiv(db, lv, rv) {
        return None;
    }
    // For UPPER bounds `v ≤ B`, the tighter (stronger) is the SMALLER B; for LOWER bounds `v ≥ B`, the
    // LARGER B. `and` keeps the stronger, `or` the weaker.
    let lhs_stronger = if l_upper { lb <= rb } else { lb >= rb };
    let keep_lhs = if is_and { lhs_stronger } else { !lhs_stronger };
    Some(if keep_lhs { lhs } else { rhs })
}

/// Normalize a `Core::Compare` on a runtime operand `v` against a constant into an INCLUSIVE half-line
/// bound on `v`, as `(v, is_upper, bound)` — `is_upper` means `v <= bound`, else `v >= bound`. Handles all
/// four ops on either operand side (`(< v c)` → `v <= c-1`; `(> v c)` → `v >= c+1`; `(< c v)` = `v > c` →
/// `v >= c+1`; etc). Bound arithmetic is `i128` so `c±1` never overflows at the i64 extremes. `None` when
/// the node is not a comparison of a runtime value against a constant. Used by `disjoint_or_covering`.
pub(super) fn comparison_halfline(db: &mut Db, id: StructId) -> Option<(StructId, bool, i128)> {
    let Core::Compare { op, lhs, rhs } = core_of(db, id) else {
        return None;
    };
    let as_int = |db: &mut Db, id: StructId| match core_of(db, id) {
        Core::ConstInt(v) => v.to_i64().map(|v| v as i128),
        _ => None,
    };
    // `(op v c)` (v on the left) or `(op c v)` (v on the right, which flips the operator's sense).
    let (v, c, v_left) = match (as_int(db, rhs), as_int(db, lhs)) {
        (Some(c), _) => (lhs, c, true),
        (_, Some(c)) => (rhs, c, false),
        _ => return None,
    };
    // Effective operator with `v` on the left (`(op c v)` mirrors: `<`↔`>`, `<=`↔`>=`).
    let eff = if v_left {
        op
    } else {
        match op {
            Prim::Lt => Prim::Gt,
            Prim::Gt => Prim::Lt,
            Prim::Le => Prim::Ge,
            Prim::Ge => Prim::Le,
            other => other,
        }
    };
    // To an inclusive bound: `v < c` ⇒ `v <= c-1`; `v <= c` ⇒ `v <= c`; `v > c` ⇒ `v >= c+1`; `v >= c` ⇒
    // `v >= c`. (`=`/`Compare` are not half-lines.)
    match eff {
        Prim::Lt => Some((v, true, c - 1)),
        Prim::Le => Some((v, true, c)),
        Prim::Gt => Some((v, false, c + 1)),
        Prim::Ge => Some((v, false, c)),
        _ => None,
    }
}

/// For two comparisons forming OPPOSITE-direction half-lines on the SAME operand `v` — one `v <= U`, the
/// other `v >= L` — decide whether their `and`/`or` is a CONSTANT. `and` (intersection `L <= v <= U`) is
/// EMPTY iff `L > U` → `Some(false)`; `or` (union) COVERS every value iff the half-lines touch or overlap
/// (`L <= U + 1`) → `Some(true)`. `None` when the pair is not opposite half-lines on the same `v`, or the
/// intersection is non-empty (`and`) / the union has a gap (`or`) — those stay runtime. `(and (< x 5) (> x
/// 10))` → false; `(or (< x 5) (> x 3))` → true. All bound math is `i128` (no overflow at i64 extremes).
pub(super) fn disjoint_or_covering(
    db: &mut Db,
    lhs: StructId,
    rhs: StructId,
    is_and: bool,
) -> Option<bool> {
    let (lv, l_upper, lb) = comparison_halfline(db, lhs)?;
    let (rv, r_upper, rb) = comparison_halfline(db, rhs)?;
    if l_upper == r_upper || !core_equiv(db, lv, rv) {
        return None; // need OPPOSITE directions on the SAME operand
    }
    // Order them: `u` = the upper bound `v <= U`, `l` = the lower bound `v >= L`.
    let (upper, lower) = if l_upper { (lb, rb) } else { (rb, lb) };
    if is_and {
        // Intersection `lower <= v <= upper` is empty iff `lower > upper`.
        (lower > upper).then_some(false)
    } else {
        // Union `v <= upper || v >= lower` covers all iff the pieces touch/overlap: `lower <= upper + 1`.
        (lower <= upper + 1).then_some(true)
    }
}

/// COINCIDENT-POINT COLLAPSE for `and`: `(and (>= v c) (<= v c))` (either operand order, `v` on either
/// side of each comparison) → `(= v c)`. Two INCLUSIVE opposite-direction bounds pinning `v` to a single
/// point ARE equality — `v >= c && v <= c ⟺ v == c` in any total order (sound for signed AND unsigned
/// integers alike; it is a pure order-theoretic fact, no sign assumption). Returns `(v, c_node)` to build
/// `Core::Compare { op: Eq, lhs: v, rhs: c_node }` — three ops (`ge` + `le` + `and`) collapse to one `eq`.
/// Restricted to the two INCLUSIVE ops (`>=`/`<=`) against the SAME i64 constant VALUE on both sides, and
/// REUSES an existing constant node (proven representable in `v`'s type — it typechecked against `v`), so
/// no constant is synthesized and no type-range guard is needed. The strictly-inclusive requirement also
/// keeps this distinct from the exclusive width-2 point `(and (> v (c-1)) (< v (c+1)))`, which would need a
/// synthesized `c` + a representability guard — deliberately left un-folded (conservative, no regression).
/// `None` unless the shape matches. DISCARDS the second comparison, so the caller gates on `is_trap_free`
/// for both operands (matches the sibling disjoint/covering fold); the kept `(= v c)` evaluates `v` once.
pub(super) fn coincident_point_eq(
    db: &mut Db,
    lhs: StructId,
    rhs: StructId,
) -> Option<(StructId, StructId)> {
    // From a `Core::Compare` on a runtime `v` against an i64 constant, return `(v, c_node, c_value, eff)`
    // where `eff` is the operator NORMALIZED to `v` on the left (`(op c v)` mirrors `<`↔`>`, `<=`↔`>=`).
    let bound_of = |db: &mut Db, id: StructId| -> Option<(StructId, StructId, i64, Prim)> {
        let Core::Compare { op, lhs: a, rhs: b } = core_of(db, id) else {
            return None;
        };
        let as_int = |db: &mut Db, id: StructId| match core_of(db, id) {
            Core::ConstInt(v) => v.to_i64(),
            _ => None,
        };
        // `(op v c)` (v on the left) or `(op c v)` (v on the right, which mirrors the operator).
        match (as_int(db, b), as_int(db, a)) {
            (Some(c), _) => Some((a, b, c, op)),
            (_, Some(c)) => Some((
                b,
                a,
                c,
                match op {
                    Prim::Lt => Prim::Gt,
                    Prim::Gt => Prim::Lt,
                    Prim::Le => Prim::Ge,
                    Prim::Ge => Prim::Le,
                    other => other,
                },
            )),
            _ => None,
        }
    };
    let (lv, lc_node, lc, leff) = bound_of(db, lhs)?;
    let (rv, _rc_node, rc, reff) = bound_of(db, rhs)?;
    // Same runtime operand, same constant VALUE, and the two effective ops are exactly `>=` and `<=`
    // (opposite INCLUSIVE bounds). Either assignment (`>= , <=` or `<= , >=`).
    if lc != rc || !core_equiv(db, lv, rv) {
        return None;
    }
    let inclusive_opposite = matches!((leff, reff), (Prim::Ge, Prim::Le) | (Prim::Le, Prim::Ge));
    if !inclusive_opposite {
        return None;
    }
    trace!(target: "rcdzc::fold", "coincident-point collapse (and (>= v c) (<= v c)) → (= v c)");
    // Reuse `lv` as the operand and lhs's constant node as `c` — both proven trap-free by the caller's gate.
    Some((lv, lc_node))
}

/// For two comparisons where one is an EQUALITY `(= x c)` and the other an ORDERING comparison `(cmp x k)`
/// on the SAME `x` (both constants), return `(eq_node, range_node, sat)` — `sat` = whether `c` satisfies
/// the range predicate `(cmp c k)`, computed at compile time. The caller decides the fold: for `and`, `sat`
/// keeps the equality (range redundant) / `!sat` is `false` (contradiction); for `or`, `sat` keeps the
/// range (equality subsumed). `None` unless exactly one side is a scalar `Eq` and the other a scalar
/// ordering comparison (`< > <= >=`), both on the SAME `x` (`core_equiv`) against i64 constants.
pub(super) fn eq_vs_range(
    db: &mut Db,
    lhs: StructId,
    rhs: StructId,
) -> Option<(StructId, StructId, bool)> {
    let as_const_i64 = |db: &mut Db, id: StructId| match core_of(db, id) {
        Core::ConstInt(v) => v.to_i64(),
        _ => None,
    };
    // Extract `(x, c)` from a `(= x c)` / `(= c x)` node (equality is symmetric).
    let eq_of = |db: &mut Db, id: StructId| -> Option<(StructId, i64)> {
        let Core::Compare {
            op: Prim::Eq,
            lhs: a,
            rhs: b,
        } = core_of(db, id)
        else {
            return None;
        };
        match (as_const_i64(db, b), as_const_i64(db, a)) {
            (Some(c), _) => Some((a, c)),
            (_, Some(c)) => Some((b, c)),
            _ => None,
        }
    };
    // Extract `(x, effective-op-with-x-on-left, k)` from an ordering comparison `(cmp x k)` / `(cmp k x)`.
    let range_of = |db: &mut Db, id: StructId| -> Option<(StructId, Prim, i64)> {
        let Core::Compare { op, lhs: a, rhs: b } = core_of(db, id) else {
            return None;
        };
        if !matches!(op, Prim::Lt | Prim::Gt | Prim::Le | Prim::Ge) {
            return None;
        }
        match (as_const_i64(db, b), as_const_i64(db, a)) {
            (Some(k), _) => Some((a, op, k)), // `(op x k)`
            (_, Some(k)) => Some((
                b,
                match op {
                    // `(op k x)` mirrors to x on the left.
                    Prim::Lt => Prim::Gt,
                    Prim::Gt => Prim::Lt,
                    Prim::Le => Prim::Ge,
                    Prim::Ge => Prim::Le,
                    other => other,
                },
                k,
            )),
            _ => None,
        }
    };
    // Try both assignments (eq on the left or right).
    let (eq_node, range_node, ex, c, rx, rop, k) =
        if let (Some((ex, c)), Some((rx, rop, k))) = (eq_of(db, lhs), range_of(db, rhs)) {
            (lhs, rhs, ex, c, rx, rop, k)
        } else if let (Some((ex, c)), Some((rx, rop, k))) = (eq_of(db, rhs), range_of(db, lhs)) {
            (rhs, lhs, ex, c, rx, rop, k)
        } else {
            return None;
        };
    if !core_equiv(db, ex, rx) {
        return None; // same `x`
    }
    // Does the equality's value `c` satisfy the range predicate `(rop c k)`?
    let sat = compare_ord(rop, c.cmp(&k));
    Some((eq_node, range_node, sat))
}

/// The NESTED-BITWISE COLLAPSE for an outer TOTAL, ASSOCIATIVE bitwise op (`&`/`|`/`^`) whose operands
/// are `(lhs, rhs)` (cores `lc`/`rc`): when one operand is `(OP v C1)` — the SAME op — and the OTHER is a
/// constant `C2`, returns `(OP v (C1 ⊙ C2))` where `⊙` is that op's constant fold — one op instead of
/// two. `(& (& v C1) C2)` → `(& v (C1&C2))`, `(| (| v C1) C2)` → `(| v (C1|C2))`, `(^ (^ v C1) C2)` →
/// `(^ v (C1^C2))`. `None` when neither shape matches (so the caller's later folds still fire). All three
/// ops are TOTAL (never trap) and ASSOCIATIVE, so no trap is dropped and the value is identical; `v` stays
/// the operand (its own traps preserved). The folded constant is a fresh `Leaf::Int` atom, lowered lazily
/// to `Core::ConstInt` and grounded to the op width at selection. (NOT for `+`/`-`/`*`/`<<` — those are
/// CHECKED, so `(v OP C1) OP C2` traps differently from `v OP (C1⊙C2)`.)
pub(super) fn nested_bitwise_collapse(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    lc: &Core,
    rhs: StructId,
    rc: &Core,
) -> Option<Core> {
    if !matches!(op, Prim::BitAnd | Prim::BitOr | Prim::BitXor) {
        return None;
    }
    let apply = |a: i64, b: i64| match op {
        Prim::BitAnd => a & b,
        Prim::BitOr => a | b,
        _ => a ^ b, // BitXor
    };
    // The `(v, C1)` of an inner `(OP v C1)` node with the SAME op, C1 a constant on either side.
    let nested_op_const = |db: &mut Db, inner: StructId| -> Option<(StructId, i64)> {
        let Core::Arith {
            op: inner_op,
            lhs: il,
            rhs: ir,
        } = core_of(db, inner)
        else {
            return None;
        };
        if inner_op != op {
            return None;
        }
        match (core_of(db, il), core_of(db, ir)) {
            (Core::ConstInt(c), _) => c.to_i64().map(|c| (ir, c)),
            (_, Core::ConstInt(c)) => c.to_i64().map(|c| (il, c)),
            _ => None,
        }
    };
    let combine = |db: &mut Db, inner: StructId, outer_c: i64| -> Option<Core> {
        let (v, inner_c) = nested_op_const(db, inner)?;
        let folded = apply(inner_c, outer_c);
        let fc = db.push_atom(crate::ast::Leaf::Int {
            value: IntValue::from_i64(folded),
            radix: crate::ast::Radix::Dec,
        });
        trace!(target: "rcdzc::fold", ?op, inner_c, outer_c, folded, "nested-bitwise collapse (OP (OP v C1) C2) → (OP v (C1⊙C2))");
        Some(Core::Arith {
            op,
            lhs: v,
            rhs: fc,
        })
    };
    // inner on the LEFT, constant C2 on the RIGHT.
    if let Core::ConstInt(c2) = rc
        && let Some(c2) = c2.to_i64()
        && let Some(folded) = combine(db, lhs, c2)
    {
        return Some(folded);
    }
    // constant C2 on the LEFT, inner on the RIGHT.
    if let Core::ConstInt(c2) = lc
        && let Some(c2) = c2.to_i64()
        && let Some(folded) = combine(db, rhs, c2)
    {
        return Some(folded);
    }
    None
}

/// XOR CANCELLATION for an outer `(^ lhs rhs)`: when one operand is `(^ v w)` and the OTHER is
/// `core_equiv` to `w`, the two XORs by `w` cancel — `(v ^ w) ^ w == v ^ (w ^ w) == v ^ 0 == v` (XOR is
/// associative/commutative and self-inverse). Returns `v`. Covers a CONSTANT `w` (`(^ (^ x 5) 5)` → x —
/// which `nested_bitwise_collapse` would leave as a residual `(^ x 0)`) AND a RUNTIME `w` (`(^ (^ x y) y)`
/// → x, the involution `nested_bitwise_collapse` cannot see). Both operand orders of the outer `^`, and
/// `w` on either side of the inner `^`, are tried. The result is `v` (its own traps preserved); `w` is
/// DISCARDED, so the caller gates on `is_trap_free(w)` — a trapping `w` (`(^ (^ v (/ a b)) (/ a b))` at
/// b==0) must still trap. Returns `(v, w)` so the caller can trap-check `w`.
pub(super) fn xor_cancels(
    db: &mut Db,
    lhs: StructId,
    rhs: StructId,
) -> Option<(StructId, StructId)> {
    // Try: `lhs` is the inner `(^ v w)`, `rhs` is the matching `w`.
    let check = |db: &mut Db, inner: StructId, outer_w: StructId| -> Option<(StructId, StructId)> {
        let Core::Arith {
            op: Prim::BitXor,
            lhs: il,
            rhs: ir,
        } = core_of(db, inner)
        else {
            return None;
        };
        // The outer operand `outer_w` must equal ONE side of the inner XOR; the OTHER side is `v`.
        if core_equiv(db, ir, outer_w) {
            Some((il, ir)) // (v, w)
        } else if core_equiv(db, il, outer_w) {
            Some((ir, il)) // (v, w) — inner XOR is commutative
        } else {
            None
        }
    };
    check(db, lhs, rhs).or_else(|| check(db, rhs, lhs))
}

/// IDEMPOTENT-BITWISE COLLAPSE for an outer `(op lhs rhs)` where `op` is `&` or `|`: when one operand is
/// an inner `(op v w)` (the SAME op) and the OTHER outer operand is `core_equiv` to `w`, return the inner
/// node — `(op (op v w) w) == (op v w)` because `&`/`|` are idempotent (`w op w == w`) and associative.
/// Covers a RUNTIME `w` the constant-folding `nested_bitwise_collapse` cannot (`(| (| x y) y)` → `(| x
/// y)`). Both outer orders and `w` on either side of the inner op are tried. The inner `(op v w)` node is
/// RETAINED, so both `v` and `w` are still evaluated — no trap is dropped (no `is_trap_free` needed).
pub(super) fn idempotent_bitwise_collapse(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    rhs: StructId,
) -> Option<StructId> {
    // `inner` must be `(op v w)` with the SAME op; `outer_w` must match one of its operands.
    let check = |db: &mut Db, inner: StructId, outer_w: StructId| -> Option<StructId> {
        let Core::Arith {
            op: inner_op,
            lhs: il,
            rhs: ir,
        } = core_of(db, inner)
        else {
            return None;
        };
        if inner_op != op {
            return None;
        }
        // The outer operand equals ONE side of the inner op → re-applying `op` by it is a no-op.
        if core_equiv(db, il, outer_w) || core_equiv(db, ir, outer_w) {
            Some(inner)
        } else {
            None
        }
    };
    check(db, lhs, rhs).or_else(|| check(db, rhs, lhs))
}

/// ABSORPTION LAW for an outer `(op lhs rhs)` where `op` is `&` or `|`: when one operand is an inner op of
/// the DUAL kind (`|` under `&`, `&` under `|`) that contains `x`, and the OTHER outer operand IS `x`
/// (`core_equiv`), the whole expression absorbs to `x` — `x & (x | y) == x`, `x | (x & y) == x`. Returns
/// `(x, y)` where `y` is the inner op's OTHER operand (the one absorbed away), so the caller can trap-check
/// `y` (it is DISCARDED). Both outer orders and both inner-operand positions are tried. `None` when the
/// shape does not match.
pub(super) fn absorption_operand(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    rhs: StructId,
) -> Option<(StructId, StructId)> {
    let dual = match op {
        Prim::BitAnd => Prim::BitOr,
        Prim::BitOr => Prim::BitAnd,
        _ => return None,
    };
    // `inner` must be `(dual p q)`; `outer_x` must equal one of `p`/`q` (that side is `x`, the other `y`).
    let check = |db: &mut Db, inner: StructId, outer_x: StructId| -> Option<(StructId, StructId)> {
        let Core::Arith {
            op: inner_op,
            lhs: ip,
            rhs: iq,
        } = core_of(db, inner)
        else {
            return None;
        };
        if inner_op != dual {
            return None;
        }
        if core_equiv(db, ip, outer_x) {
            Some((outer_x, iq)) // (x, y) — x matched ip, y is iq
        } else if core_equiv(db, iq, outer_x) {
            Some((outer_x, ip)) // (x, y) — x matched iq, y is ip
        } else {
            None
        }
    };
    check(db, lhs, rhs).or_else(|| check(db, rhs, lhs))
}

/// The NESTED SHIFT COLLAPSE for `(SH lhs rhs)` where `SH` is `Shr` OR `Shl`: when `lhs` is itself
/// `(SH v A)` — the SAME shift op — with a constant inner count A, and the outer count `rc` is a constant
/// B with `A + B < width`, returns `(SH v (A+B))` — one shift instead of two. `None` otherwise (so later
/// folds fire). For `>>`: the shift is total; inner and outer are the same kind (`>>ₛ`/`>>ᵤ`) on the
/// same-typed value, so composing drops the same low `A+B` bits as one shift by `A+B`. For `<<`: the
/// shift is CHECKED (exact `·2^count`, traps on N-bit overflow) but still collapses TRAP-IDENTICALLY —
/// magnitude is monotonic in the count, so `(v<<A)<<B` and `v<<(A+B)` overflow on exactly the same inputs
/// (inner overflow ⟹ combined; combined ⟹ the double's outer step) and agree on value otherwise. The
/// `A + B < width` bound is essential for BOTH: a combined count `≥ width` would be masked mod width by
/// the machine shift (`>>`) / must trap as an out-of-range count (`<<`), disagreeing with the double
/// shift. `v` keeps its traps (it stays the operand). The combined count `A+B` is a fresh `Leaf::Int`.
pub(super) fn nested_shift_combine(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    rc: &Core,
) -> Option<Core> {
    // Only the two shift ops, and the inner must be the SAME op (a `<<` inside a `>>` composes bits
    // differently and does not collapse).
    if !matches!(op, Prim::Shr | Prim::Shl) {
        return None;
    }
    // Outer count B must be a constant ≥ 1 (0 is handled by the `SH 0` identity).
    let Core::ConstInt(b) = rc else { return None };
    let b = b.to_i64().filter(|&b| b >= 1)?;
    // `lhs` must be an inner shift by the SAME op with a constant count A ≥ 1.
    let Core::Arith {
        op: inner_op,
        lhs: v,
        rhs: inner_count,
    } = core_of(db, lhs)
    else {
        return None;
    };
    if inner_op != op {
        return None;
    }
    let Core::ConstInt(a) = core_of(db, inner_count) else {
        return None;
    };
    let a = a.to_i64().filter(|&a| a >= 1)?;
    // Sound ONLY when the combined count stays in range for the SHIFTED VALUE's width (both shifts share
    // it — binary-op unification). A `width` of 0 (deferred) fails the guard, so no fold.
    let width = shift_width(db, v) as i64;
    if width == 0 || a + b >= width {
        return None;
    }
    let fc = db.push_atom(crate::ast::Leaf::Int {
        value: IntValue::from_i64(a + b),
        radix: crate::ast::Radix::Dec,
    });
    trace!(target: "rcdzc::fold", ?op, a, b, sum = a + b, "nested shift collapse (SH (SH v A) B) → (SH v (A+B))");
    Some(Core::Arith {
        op,
        lhs: v,
        rhs: fc,
    })
}

/// OR-THEN-MASK ABSORPTION: for an outer `(& inner C2)` whose `inner` is `(| v C1)` (C1 a constant on
/// either side), return `C2` when `C2`'s set bits are a SUBSET of `C1`'s (`C2 & C1 == C2`) — because
/// `(v | C1) & C2` forces every bit of `C2` to 1 (they are all in `C1`, which the OR sets) and clears the
/// rest, so the result is exactly `C2`, regardless of `v`. `(& (| x 15) 15)` → 15, `(& (| x 255) 15)` →
/// 15. Returns the CONSTANT `C2` occurrence (the outer mask, `c2_occ`) to reuse as the folded value.
/// `None` when the shape does not match or `C2 ⊄ C1`. The fold DISCARDS `v`, so the caller gates it on
/// `is_trap_free(v)` — the returned `Some` reports `v` so the caller can check it.
pub(super) fn or_then_mask_absorbs(db: &mut Db, inner: StructId, c2: i64) -> Option<StructId> {
    let Core::Arith {
        op: Prim::BitOr,
        lhs: il,
        rhs: ir,
    } = core_of(db, inner)
    else {
        return None;
    };
    // The inner OR's constant C1 (on either side); the other operand is `v`.
    let (v, c1) = match (core_of(db, il), core_of(db, ir)) {
        (Core::ConstInt(c), _) => (ir, c.to_i64()?),
        (_, Core::ConstInt(c)) => (il, c.to_i64()?),
        _ => return None,
    };
    // `C2 ⊆ C1` — every bit the outer mask keeps is one the inner OR already set to 1.
    if (c2 & c1) == c2 { Some(v) } else { None }
}

/// Whether masking the value at `val` with the constant `mask_core` is a NO-OP — i.e. `val & M == val`.
/// True iff `val`'s solved type is a resolved UNSIGNED integer of width `N` (`Sign::Fixed(false)` +
/// `Width::Fixed(N)`, `N < 64`) and the mask's low `N` bits are ALL set (`M & (2^N − 1) == 2^N − 1`). An
/// unsigned width-N value lives in `[0, 2^N)`, so a mask covering its whole range clears nothing. NOT
/// applied to signed types (the slot's high bits are sign extension a mask would wrongly clear) nor to
/// a 64-bit width (whose full mask `2^64−1` is not i64-representable here — and `& allbits` at 64 is a
/// separate case the `x & x` fold does not cover; skipped for simplicity, low value).
pub(super) fn is_full_mask_for(db: &mut Db, val: StructId, mask_core: &Core) -> bool {
    let Core::ConstInt(m) = mask_core else {
        return false;
    };
    let Some(m) = m.to_i64() else {
        return false;
    };
    let Some(bits) = unsigned_value_bits(db, val) else {
        return false; // not a provably-nonnegative value with a known ≤63-bit range.
    };
    // `2^bits − 1` — all bits the value can possibly set. `bits` is `1..=63`; at `bits == 63` the shift
    // `1i64 << 63` is `i64::MIN`, so `− 1` would OVERFLOW in a checked build (a latent panic) — that case
    // is exactly `i64::MAX` (all 63 low bits, the whole nonneg i64 range), so special-case it.
    let low = if bits >= 63 {
        i64::MAX
    } else {
        (1i64 << bits) - 1
    };
    (m & low) == low
}

/// For a `BitAnd` at emit time, whether the constant mask on ONE side covers the WHOLE provable range of
/// the value on the other side — so `v & M == v` and the `&` is redundant. Returns the VALUE operand to
/// emit alone (`Some(v)`), or `None` when neither side is such a redundant mask. This is the EMIT-TIME
/// sibling of the `is_full_mask_for` lower fold: identical soundness (a nonneg `v ∈ [0, 2^B)` whose bits
/// `M` all covers), but it consults `value_range` HERE — where the flow-refinement stack is populated — so
/// it fires on a refined value the lower fold could not see (`(if (and (>= x 0) (< x 256)) (& x 255) …)`:
/// under the branch `x ∈ [0,255]`, `x & 255 == x`). Both operand orders are tried. The `&` is TOTAL, so
/// eliding it drops no trap; returning the value operand preserves its own evaluation (and any trap in it).
pub(crate) fn redundant_and_mask_value(
    db: &mut Db,
    lhs: StructId,
    rhs: StructId,
) -> Option<StructId> {
    let rc = core_of(db, rhs);
    if is_full_mask_for(db, lhs, &rc) {
        return Some(lhs); // `(& v M)` with M covering v's range → v
    }
    let lc = core_of(db, lhs);
    if is_full_mask_for(db, rhs, &lc) {
        return Some(rhs); // `(& M v)` → v
    }
    None
}

/// For a `BitOr` at emit time, whether the constant `M` on ONE side covers the WHOLE provable range of the
/// value `v` on the other side — so `v | M == M` (OR-SATURATION) and the `|` is redundant. Returns the
/// CONSTANT operand (`Some(M_occ)`) to emit alone, or `None`. The emit-time sibling of the `BitOr`
/// OR-saturation lower fold, firing on a flow-refined `v` the lower fold cannot see (`(if (and (>= x 0)
/// (< x 256)) (| x 255) …)` → `x | 255 == 255` under `x ∈ [0,255]`). DISCARDS `v` (the result is the
/// constant M), so the caller must first confirm `v` is TRAP-FREE — a trapping `v` must still trap. Both
/// operand orders tried; returns whichever operand is the covering constant.
pub(crate) fn redundant_or_mask_const(
    db: &mut Db,
    lhs: StructId,
    rhs: StructId,
) -> Option<StructId> {
    // `(| v M)` — v on the left, constant M on the right covering v's range → M (rhs).
    let rc = core_of(db, rhs);
    if is_full_mask_for(db, lhs, &rc) && is_trap_free(db, lhs) {
        return Some(rhs);
    }
    // `(| M v)` — constant M on the left → M (lhs).
    let lc = core_of(db, lhs);
    if is_full_mask_for(db, rhs, &lc) && is_trap_free(db, rhs) {
        return Some(lhs);
    }
    None
}

/// Whether the dividend `val` is provably in `[0, divisor − 1]` for a positive `divisor` — so a truncating
/// `val / divisor` is `0` and `val % divisor` is `val` (the divisor is too large to divide `val` even
/// once). True iff `value_range(val)` is a NONNEGATIVE closed interval `[lo, hi]` with `lo >= 0` and
/// `hi < divisor`. Restricted to a nonnegative dividend: for `0 <= val < divisor`, both the mathematical
/// and the truncate-toward-zero results are exact (`val / divisor = 0`, `val % divisor = val`); a negative
/// dividend is excluded (its `value_range` lo is `< 0`, failing the check). `None`/unbounded range → false.
pub(super) fn dividend_below_divisor(db: &mut Db, val: StructId, divisor: i64) -> bool {
    matches!(value_range(db, val), Some((lo, Some(hi))) if lo >= 0 && hi < divisor)
}

/// An upper bound (in `1..=63`) on the number of LOW bits a runtime value can set — i.e. the value is
/// provably in `[0, 2^B)`. Derived from `value_range`: a value whose range is `[0, hi]` (nonnegative,
/// `hi` a nonneg i64) fits `bits(hi)` bits. `None` when the value is not provably nonnegative or has no
/// i64 upper bound. Drives the mask-elision (`& fullmask`) and shift-out (`>>ᵤ` all bits) folds.
pub(super) fn unsigned_value_bits(db: &mut Db, val: StructId) -> Option<u32> {
    match value_range(db, val) {
        // A nonnegative closed range `[0, hi]` → the significant-bit count of `hi` (≥ 1). `hi` is a
        // nonnegative i64, so `bits(hi) ∈ 1..=63`.
        Some((0, Some(hi))) if hi >= 0 => Some((64 - (hi as u64).leading_zeros()).max(1)),
        _ => None,
    }
}

/// The language WIDTH `N` of `val`'s resolved integer type — the range a shift COUNT is guarded to
/// `[0, N)`. Used by the shift-out-to-zero fold to confirm the constant count is IN-RANGE (an
/// out-of-range shift TRAPS, so it must NOT be folded to 0). `None` if the type is not a resolved
/// integer (a deferred width would guess the guard bound).
pub(super) fn shift_width(db: &mut Db, val: StructId) -> u32 {
    match crate::infer::type_of(db, val) {
        crate::ty::Ty::Int(it) => match it.width {
            crate::ty::Width::Fixed(n) => n,
            _ => 0, // deferred/var — treat as 0 so the `k < width` guard fails (no fold).
        },
        _ => 0,
    }
}

/// Whether the node at `id` lowers to a core that CANNOT TRAP at run time — so discarding it (an
/// annihilator identity like `x * 0 → 0`) loses no defined trap. CONSERVATIVE: only a value with no
/// checked operation anywhere inside it. Trap-free = a leaf (constant/param/local/unit), a wrap
/// (total), or a bitwise op / conversion / projection over trap-free operands. NOT trap-free = `+`/
/// `-`/`*`/`<<`/`>>` (overflow/count guards), `/`/`%` (÷0, MIN/-1), a call (its body may trap), an
/// `if`/`match` (a branch may trap), a sum/tuple/record construct (may allocate/box — treated as
/// possibly-effecting here). Reads the operand's already-lowered core recursively.
pub(crate) fn is_trap_free(db: &mut Db, id: StructId) -> bool {
    match core_of(db, id) {
        Core::ConstInt(_)
        | Core::ConstBool(_)
        | Core::ConstStr(_)
        | Core::ConstBytes(_)
        | Core::ConstChar(_)
        | Core::ConstFloat(_)
        | Core::ConstFloatNan
        | Core::Unit
        // A PARAMETER is caller-forced (its argument was already evaluated before the body runs), so a param
        // reference can never itself be an un-forced trap — always trap-free.
        | Core::Param { .. } => true,
        // A LOCAL REFERENCE reads an already-bound slot, so the READ is pure — but `is_trap_free` is used by
        // DISCARDING folds (`x OP x → const`, `x * 0 → 0`, `(if c a a) → a`, the absorbing/complement laws …)
        // to decide "safe to DROP this operand without eliding a trap". A `let` binding is LAZY (forced on
        // use), so DISCARDING the reference can drop the LAST forcing of a binding whose INIT traps → eliding
        // an OBSERVABLE trap (cdz-smith L2 differential: `(let ((v0 (r 2))) (< v0 v0))` folded to false,
        // dropping v0's trapping force; the Lean oracle correctly traps). So a reference is trap-free-TO-DISCARD
        // iff its BOUND VALUE is: follow the ref chain (`peel_ref_annot`) to the underlying init and check THAT.
        // (Consistent with the trap-observability precedent #4417 — a real trap is never silently elided.) An
        // unresolvable ref keeps the prior `true` (regression-safe — this only tightens the resolvable cases,
        // which are exactly the unsound discards).
        Core::LocalRef { .. } => {
            let init = peel_ref_annot(db, id);
            init == id || is_trap_free(db, init)
        }
        // PURE VALUE CONSTRUCTORS never trap in themselves — building a record/tuple/list/sum node is total;
        // only a trapping SUB-expression inside makes the whole thing trap. So a construction is trap-free
        // iff every field/element/payload it holds is. (This is what lets a discarding fold over a compound
        // fire — e.g. the `List.len` constant-arity fold drops the element VALUES, sound only when every
        // element construction is trap-free; a `(list _ (Rational.of 3 d))` with a runtime denominator is
        // NOT trap-free, so the fold declines and the runtime op preserves the element's trap.)
        Core::Record { fields } => fields.values().copied().all(|v| is_trap_free(db, v)),
        Core::Tuple { elems } | Core::ListNew { elems } => {
            elems.iter().copied().all(|e| is_trap_free(db, e))
        }
        Core::SumNew { payloads, .. } => payloads.iter().all(|&p| is_trap_free(db, p)),
        // Bitwise ops are total; a comparison never traps — trap-free if their operands are. The WRAPPING
        // arithmetic ops (`wrapping-add`/`wrapping-sub`/`wrapping-mul`) are ALSO total: they emit the raw
        // machine `add`/`sub`/`mul` with NO overflow guard (wasm's op wraps modulo the slot — that is their
        // whole point vs checked `+`/`-`/`*`), so they never trap and are trap-free when their operands
        // are. (Checked `Add`/`Sub`/`Mul` — with an overflow guard — stay in the possibly-trapping `_` arm below.)
        Core::Arith {
            op:
                Prim::BitAnd
                | Prim::BitOr
                | Prim::BitXor
                | Prim::WrappingAdd
                | Prim::WrappingSub
                | Prim::WrappingMul,
            lhs,
            rhs,
        }
        | Core::Compare { lhs, rhs, .. } => is_trap_free(db, lhs) && is_trap_free(db, rhs),
        // Boolean negation `not` is a single `i32.eqz` — total (never traps) — so trap-free if its operand
        // is. (Lets `(not a)` participate in a discarding fold, e.g. the boolean complement law
        // `(and (not a) a)` → false, whose `is_trap_free(lhs)` guard sees the `(not a)` lhs.)
        Core::Not { operand } => is_trap_free(db, operand),
        // `wrap` is total (never traps) — trap-free if its operand is.
        Core::Convert {
            op: Prim::Wrap,
            operand,
        } => is_trap_free(db, operand),
        // A COLLECTION COUNT (`List.len`/`Bytes.len`/`Map.size`/`Set.len`) is a TOTAL O(1) borrowing read —
        // it never traps — so it is trap-free when its collection operand is (a param/kept-local handle is;
        // a count of a trapping construction stays tied to that trap). This lets a length feed a discarding
        // fold (`(>= (List.len xs) 0)` → true drops the length) with its `[0, 2^32-1]` range from
        // `value_range`. The operand is the container handle for each.
        Core::ListLen { operand } | Core::BytesLen { operand } | Core::StrScalarLen { operand } => {
            is_trap_free(db, operand)
        }
        Core::MapSize { map } => is_trap_free(db, map),
        Core::SetLen { set } => is_trap_free(db, set),
        // A TUPLE/RECORD PROJECTION is a total borrowing read: its `index` is WITHIN the operand's static
        // arity (`type_errors` rejects an out-of-arity index at compile time — never a runtime OOB trap;
        // an arity-≥1 compound is never the immediate that `arr-get` would OOB on), so `arr-get(compound,
        // const-index)` cannot trap. Trap-free when the compound operand is. This lets an invariant field
        // read (`(. p 0)` over a pass-through tuple `p`) hoist out of a loop and a dead projection be
        // dropped by a discarding fold. (A `SumPayload` `Elem` step is EXCLUDED — reading a variant's
        // payload before its discriminant is checked CAN mismatch, so it stays possibly-trapping in `_`.)
        Core::Proj { operand, .. } => is_trap_free(db, operand),
        // A RIGHT SHIFT by a CONSTANT in-range count (`0 <= k < width`) never traps: `>>` cannot overflow
        // (its magnitude only shrinks), and a valid constant count trips no count-guard. So it is trap-free
        // when its value operand is. (A `<<` is EXCLUDED — it is exact `·2^k` and can overflow the type, so
        // it is genuinely trapping even with a valid count. A RUNTIME count is also excluded — an
        // out-of-range count traps.) This lets a `(>> x k)` feed a discarding fold: `(< (>>ᵤ x 60) 20)` on
        // a UInt64 (range `[0,15]`) folds to `true` without keeping a bogus "shift might trap" compare.
        Core::Arith {
            op: Prim::Shr,
            lhs,
            rhs,
        } if matches!(core_of(db, rhs), Core::ConstInt(k)
                if k.to_i64().is_some_and(|k| k >= 0 && (k as u32) < shift_width(db, lhs))) =>
        {
            is_trap_free(db, lhs)
        }
        // A `/` or `%` by a CONSTANT divisor `C ∉ {0, -1}` never traps: `C != 0` rules out ÷0, and `C != -1`
        // rules out the sole signed-division overflow `MIN / -1`. So it is trap-free when its dividend is.
        // (`C == 0` is a constant-trap poison in `lower` before here; `C == -1` keeps the guard. A RUNTIME
        // divisor is excluded — it could be 0 or -1.) Lets `(< (% (& x 255) 10) 10)` fold to `true`.
        Core::Arith {
            op: Prim::Div | Prim::Rem,
            lhs,
            rhs,
        } if matches!(core_of(db, rhs), Core::ConstInt(c)
                if matches!(c.to_i64(), Some(v) if v != 0 && v != -1)) =>
        {
            is_trap_free(db, lhs)
        }
        // Everything else — checked arithmetic (+/-/*), a LEFT shift, a runtime-count/-divisor shift or
        // div/rem, calls, control flow, heap constructs, poison — is conservatively treated as possibly-
        // trapping.
        _ => false,
    }
}

/// Whether the core at `id` CONTAINS a runtime call (`Core::Call`, `CallClosure`, or `HostCall`) anywhere
/// in the positions the mutual-/self-recursion loop transform threads a TAIL call through — the node
/// itself, an `if`'s branches, a `let`'s body, a `match`'s arms. Used to VETO the `(if c a false)`→`(and c
/// a)` rewrite when the branch that would become the connective's guarded `rhs` holds a tail call: the loop
/// transform (`body_has_member_tail_call`) only follows `if`/`let`/`match` tail positions, NOT `and`/`or`,
/// so burying a tail-recursive call inside a connective would defeat tail-loop conversion (a far bigger win
/// than a branchless boolean). Conservative — descends only tail positions, matching the transform's reach;
/// a call in a non-tail operand is not a tail edge and would not be lost, but treating the whole branch as
/// call-bearing here is safe (it only forgoes the rewrite). NOT the same as `!is_trap_free`: a checked-arith
/// boolean branch (`(> (+ x 1) 5)`) is call-free, so the rewrite still fires and stays sound (its trap is
/// shielded in the connective's guarded rhs exactly as in the `if`'s branch).
pub(super) fn tail_positions_have_call(db: &mut Db, id: StructId) -> bool {
    match core_of(db, id) {
        Core::Call { .. } | Core::CallClosure { .. } | Core::HostCall { .. } => true,
        Core::If { then_, else_, .. } => {
            tail_positions_have_call(db, then_) || tail_positions_have_call(db, else_)
        }
        Core::Let { body, .. } => tail_positions_have_call(db, body),
        Core::Match { arms, .. } => arms.iter().any(|a| tail_positions_have_call(db, a.body)),
        _ => false,
    }
}

/// Whether the nodes at `a` and `b` lower to the STRUCTURALLY IDENTICAL core — the basis for folding an
/// `if` whose two branches are the same (`(if c x x)` → `x`). CONSERVATIVE: matches only PURE
/// deterministic scalar cores (const / param / local-ref leaves; arithmetic / comparison / conversion /
/// projection over recursively-equal operands), so any other core (a call, a nested `if`, a heap
/// construct) compares unequal and the `if` is left intact. Every matched kind is a value that reads the
/// same whichever branch produces it, so collapsing the two branches to one is behavior-preserving.
/// (This is the `lower`-column twin of `select::core_eq`, kept here because `lower` owns the core.)
pub(super) fn core_equiv(db: &mut Db, a: StructId, b: StructId) -> bool {
    if a == b {
        return true;
    }
    match (core_of(db, a), core_of(db, b)) {
        (Core::ConstInt(x), Core::ConstInt(y)) => x.eq_value(&y),
        (Core::ConstBool(x), Core::ConstBool(y)) => x == y,
        (Core::Unit, Core::Unit) => true,
        (Core::Param { binder: x }, Core::Param { binder: y }) => x == y,
        (Core::LocalRef { binder: x }, Core::LocalRef { binder: y }) => x == y,
        (
            Core::Arith {
                op: ox,
                lhs: lx,
                rhs: rx,
            },
            Core::Arith {
                op: oy,
                lhs: ly,
                rhs: ry,
            },
        )
        | (
            Core::Compare {
                op: ox,
                lhs: lx,
                rhs: rx,
            },
            Core::Compare {
                op: oy,
                lhs: ly,
                rhs: ry,
            },
        ) => {
            // Base structural match: same operator, operands equal position-wise.
            let positional = ox == oy && core_equiv(db, lx, ly) && core_equiv(db, rx, ry);
            // COMMUTATIVITY of EQUALITY: `(= a b)` and `(= b a)` denote the identical boolean (equality is
            // symmetric), so accept the SWAPPED operand match too. Only `Eq` — `<`/`>`/`<=`/`>=` flip
            // direction when swapped, and this arm is shared with `Core::Arith` whose ops are never `Eq`, so
            // `ox == Eq` fires ONLY for a comparison. Guarded on both operands trap-free so the swap changes
            // no observable evaluation ORDER (a trapping operand's position could decide which trap fires
            // first; a pure operand's cannot). Lets `(and (= a b) (= b a))` fold to one `(= a b)` via the
            // idempotence path, which keys on `core_equiv`.
            positional
                || (ox == oy
                    && matches!(ox, Prim::Eq)
                    && is_trap_free(db, lx)
                    && is_trap_free(db, rx)
                    && core_equiv(db, lx, ry)
                    && core_equiv(db, rx, ly))
        }
        (
            Core::Convert {
                op: ox,
                operand: px,
            },
            Core::Convert {
                op: oy,
                operand: py,
            },
        ) => ox == oy && core_equiv(db, px, py),
        (
            Core::Proj {
                operand: px,
                index: ix,
            },
            Core::Proj {
                operand: py,
                index: iy,
            },
        ) => ix == iy && core_equiv(db, px, py),
        _ => false,
    }
}

/// Hoist a COMMON CONSTRUCTOR out of both `if` arms: when both branches build the SAME constructor —
/// the same `SumNew` discriminant + payload arity, a same-arity `Tuple`, a same-length `List`, or a
/// `Record` with the SAME KEY SET — the heap build is DUPLICATED across the two branches, differing only
/// in the payload/element/field occurrences. Build the constructor ONCE and push each DIFFERING position
/// down into its own
/// `(if c pᵢ qᵢ)`; a position that is `core_equiv` across the arms is shared directly.
/// `(if c (Some a) (Some b))` → `(Some (if c a b))`, `(if c (tuple a k) (tuple b k))` →
/// `(tuple (if c a b) k)`, and `(if c (record (x a) (y k)) (record (x b) (y k)))` →
/// `(record (x (if c a b)) (y k))` — ONE alloc + one set of field stores emitted instead of two
/// duplicated build sequences (a module-size win; the runtime alloc count is already one either way
/// since an `if` takes exactly one arm).
///
/// SOUND: exactly one branch's payloads ever MATERIALIZE either way — a differing position stays under
/// an `if`, so the untaken arm's payload is never evaluated or consumed and the Perceus consume-once
/// discipline is unchanged (a heap payload is dup'd/dropped by the inner `if` exactly as the original
/// arm's build did). A SHARED (identical) position is a PURE scalar — `core_equiv` matches only
/// const/param/local/arith/compare/convert/proj — so evaluating it once unconditionally reproduces the
/// always-taken arm's single evaluation (including any arith trap, which the taken arm also incurred).
/// `cond` is evaluated ONCE PER DIFFERING position: exactly one differing position is one evaluation,
/// matching the original `if`, so that COUNT case is sound; zero or ≥2 differing positions change the
/// count, so require a TRAP-FREE `cond` (it re-reads identically with no effect or trap to drop or
/// duplicate). But a trapping `cond` needs an ORDER check too: the original `if` evaluates `cond`
/// FIRST — before any payload — whereas a SHARED payload BEFORE the differing position is built OUTSIDE
/// the per-position `if` and so evaluates BEFORE `cond`. Since `core_equiv` admits trapping arith (a
/// `/` by a runtime divisor), a preceding shared payload that can trap would PREEMPT `cond`'s trap and
/// the hoist would observe the WRONG trap; so a trapping `cond` additionally requires every shared
/// payload preceding the differing position to be trap-free. Returns `None` (keep the `if`) when the
/// arms are not the same constructor, disagree in shape/key-set, or either cond guard fails. A poison
/// arm has a non-constructor core, so it never matches and the `if`'s existing poison handling stands.
pub(super) fn hoist_common_ctor(
    db: &mut Db,
    cond: StructId,
    then_: StructId,
    else_: StructId,
) -> Option<Core> {
    enum Shape {
        Sum(u32),
        Tuple,
        // A record's fields in a fixed KEY ORDER (the `BTreeMap`'s sorted keys), paired with the aligned
        // then/else value occurrences below; rebuilt into a `BTreeMap` after the per-position hoist.
        Record(Vec<crate::resolved::Symbol>),
        // A same-length list — positional like a tuple, rebuilt as a `ListNew` of the hoisted elements.
        List,
    }
    // Align each arm's positions into `(then_value, else_value)` pairs and record the reconstruction
    // shape. For keyed records the two arms must carry the IDENTICAL key set (a differing key set is a
    // different value, not a hoistable common constructor); the paired values are read in sorted-key
    // order so the rebuilt map re-pairs them by that same order.
    let (shape, pairs): (Shape, Vec<(StructId, StructId)>) =
        match (core_of(db, then_), core_of(db, else_)) {
            (
                Core::SumNew {
                    disc: dt,
                    payloads: pt,
                },
                Core::SumNew {
                    disc: de,
                    payloads: pe,
                },
            ) if dt == de && pt.len() == pe.len() => (
                Shape::Sum(dt),
                pt.iter().copied().zip(pe.iter().copied()).collect(),
            ),
            (Core::Tuple { elems: et }, Core::Tuple { elems: ee }) if et.len() == ee.len() => (
                Shape::Tuple,
                et.iter().copied().zip(ee.iter().copied()).collect(),
            ),
            // A LIST is positional like a tuple, but a list's LENGTH is part of its value (two lists of
            // different lengths are distinct values, not a common constructor) — so the same-length guard
            // both aligns the elements AND is a genuine value check. A list is HOMOGENEOUS (one element
            // type), so a per-element `(if c eᵢ fᵢ)` is well-typed. The backend builds it `vec-empty` +
            // per-element `vec-push`; hoisting shares that whole chain and selects only the differing
            // element, exactly as the tuple arm shares the `arr-alloc` + stores.
            (Core::ListNew { elems: et }, Core::ListNew { elems: ee }) if et.len() == ee.len() => (
                Shape::List,
                et.iter().copied().zip(ee.iter().copied()).collect(),
            ),
            (Core::Record { fields: ft }, Core::Record { fields: fe })
                if ft.len() == fe.len() && ft.keys().zip(fe.keys()).all(|(a, b)| a == b) =>
            {
                let keys: Vec<crate::resolved::Symbol> = ft.keys().cloned().collect();
                let pairs: Vec<(StructId, StructId)> =
                    keys.iter().map(|k| (ft[k], fe[k])).collect();
                (Shape::Record(keys), pairs)
            }
            _ => return None,
        };
    // Nothing to build (a nullary sum variant / empty tuple / empty record — no positions) offers no win
    // and would drop `cond` entirely; leave it to the enum-disc / identical-branch folds.
    if pairs.is_empty() {
        return None;
    }
    let mut diff = 0usize;
    let mut first_diff: Option<usize> = None;
    for (i, &(a, b)) in pairs.iter().enumerate() {
        if !core_equiv(db, a, b) {
            diff += 1;
            first_diff.get_or_insert(i);
        }
    }
    // A trapping `cond` needs both a count check AND an ORDER check before it may hoist.
    if !is_trap_free(db, cond) {
        // COUNT: `cond` is evaluated once per differing position; only ONE differing position matches
        // the original's single evaluation. Any other count would duplicate/drop a cond trap or effect.
        if diff != 1 {
            return None;
        }
        // ORDER: the original `if` evaluates `cond` FIRST — before ANY payload. But in the hoisted form
        // a SHARED (non-differing) payload BEFORE the differing position is built OUTSIDE the per-position
        // `if`, so it evaluates BEFORE `cond`. `core_equiv` admits trapping arith (a `/` by a runtime
        // divisor), so a preceding shared payload that traps would PREEMPT `cond`'s own trap — the hoist
        // would observe the WRONG trap. Only hoist a trapping cond when every shared payload preceding
        // the differing position is itself trap-free (then the first observable trap is still `cond`'s).
        let diff_idx = first_diff.expect("diff == 1 guarantees a differing position");
        for &(a, _) in &pairs[..diff_idx] {
            if !is_trap_free(db, a) {
                return None;
            }
        }
    }
    let mut vals: Vec<StructId> = Vec::with_capacity(pairs.len());
    for &(a, b) in &pairs {
        vals.push(if core_equiv(db, a, b) {
            a
        } else {
            synth_if_hoisted(db, cond, a, b)
        });
    }
    Some(match shape {
        Shape::Sum(disc) => Core::SumNew {
            disc,
            payloads: vals.into(),
        },
        Shape::Tuple => Core::Tuple { elems: vals.into() },
        Shape::List => Core::ListNew { elems: vals.into() },
        Shape::Record(keys) => Core::Record {
            fields: std::rc::Rc::new(keys.into_iter().zip(vals).collect()),
        },
    })
}

/// Hoist a COMMON OPERATOR out of both `if` arms — the arithmetic/conversion sibling of
/// `hoist_common_ctor`. When both branches apply the SAME operator to operands that mostly agree, the
/// operator (and its overflow guard, for checked arith) is DUPLICATED across the two branches:
///   `(if c (+ a 1) (+ b 1))`  → `(+ (if c a b) 1)`          — one checked add + one guard, not two
///   `(if c (* a k) (* b k))`  → `(* (if c a b) k)`
///   `(if c (< a k) (< b k))`  → `(< (if c a b) k)`          — one compare, not two (Compare is total)
///   `(if c (wrap a) (wrap b))`→ `(wrap (if c a b))`         — a unary `Convert`
/// Each DIFFERING operand position is pushed into its own `(if c pᵢ qᵢ)`; a `core_equiv` position is
/// shared directly, so the operator applies to exactly the operand tuple the taken arm would have used.
///
/// SOUND for ANY operator, INCLUDING a trapping checked op: the hoisted form applies the operator ONCE to
/// the SELECTED operands — the identical operand values the taken arm passed it — so it computes the same
/// result and traps under exactly the same condition (this is operand SELECTION under the op, NOT
/// reassociation: no operand ever crosses the operator or combines with a different partner). A shared
/// operand is `core_equiv` (a pure scalar — const/param/local/arith/compare/convert/proj) evaluated once,
/// reproducing the taken arm's single evaluation. `cond` is evaluated once per DIFFERING operand: exactly
/// one differing operand matches the original's single `cond` eval (unconditionally sound); 0 or ≥2
/// require a TRAP-FREE `cond` (the same guard `hoist_common_ctor` uses). Returns `None` unless both arms
/// are the same `Arith` operator, the same `Compare` operator (both over 2 operands), or the same
/// `Convert` op (1 operand) — a poison arm has neither core, so it never matches.
pub(super) fn hoist_common_arith(
    db: &mut Db,
    cond: StructId,
    then_: StructId,
    else_: StructId,
) -> Option<Core> {
    enum Head {
        Arith(Prim),
        Compare(Prim),
        FloatCompare(Prim, u32),
        Convert(Prim),
    }
    let (head, pairs): (Head, Vec<(StructId, StructId)>) =
        match (core_of(db, then_), core_of(db, else_)) {
            (
                Core::Arith {
                    op: ot,
                    lhs: lt,
                    rhs: rt,
                },
                Core::Arith {
                    op: oe,
                    lhs: le,
                    rhs: re,
                },
            ) if ot == oe => (Head::Arith(ot), vec![(lt, le), (rt, re)]),
            // A COMPARISON is total (never traps, no guard), so the hoist is value-safe: `(if c (< a k)
            // (< b k))` → `(< (if c a b) k)` computes the same boolean (the comparison of the SELECTED
            // operand against `k`) with ONE compare instead of two. Operands are paired POSITIONALLY (`==`
            // on the `Prim` requires the same operator, so no `<`-vs-`>` mixup); the `Eq`-commutativity
            // `core_equiv` allows is irrelevant here since each position selects its own actual operand.
            // Value-safe is NOT trap-ORDER-safe, though: the compare's OPERANDS can still be trapping arith
            // (e.g. `(/ 100 d)`), and a SHARED trapping operand hoisted ahead of a trapping `cond` would
            // preempt `cond`'s trap. The shared-operand order guard below (`!is_trap_free(db, cond)` →
            // preceding-operand scan) applies to this Head arm exactly as it does to `Arith`.
            (
                Core::Compare {
                    op: ot,
                    lhs: lt,
                    rhs: rt,
                },
                Core::Compare {
                    op: oe,
                    lhs: le,
                    rhs: re,
                },
            ) if ot == oe => (Head::Compare(ot), vec![(lt, le), (rt, re)]),
            // A canonical-byte FLOAT equality (`Core::FloatCompare`) is total exactly like an integer
            // `Compare` — it canonicalizes each operand's NaN and compares the resulting bit patterns
            // (`i32.eq`/`i64.eq`), never trapping on its own — so `(if c (= a k) (= b k))` over Float
            // operands hoists to `(= (if c a b) k)`, ONE canon-and-compare instead of two, on exactly the
            // same value-safety footing as `Compare`. Both arms must share the operator AND the WIDTH (an
            // f32 compare canonicalizes to i32 bits and an f64 to i64 bits — mixing them would emit two
            // different machine ops off one hoisted operand). The shared-operand trap-ORDER guard below
            // applies identically (a `FloatCompare` operand can still be a trapping `/`).
            (
                Core::FloatCompare {
                    op: ot,
                    lhs: lt,
                    rhs: rt,
                    width: wt,
                },
                Core::FloatCompare {
                    op: oe,
                    lhs: le,
                    rhs: re,
                    width: we,
                },
            ) if ot == oe && wt == we => (Head::FloatCompare(ot, wt), vec![(lt, le), (rt, re)]),
            (
                Core::Convert {
                    op: ot,
                    operand: pt,
                },
                Core::Convert {
                    op: oe,
                    operand: pe,
                },
            ) if ot == oe => (Head::Convert(ot), vec![(pt, pe)]),
            _ => return None,
        };
    let mut diff = 0usize;
    let mut first_diff: Option<usize> = None;
    for (i, &(a, b)) in pairs.iter().enumerate() {
        if !core_equiv(db, a, b) {
            diff += 1;
            first_diff.get_or_insert(i);
        }
    }
    // All operands identical → the two arms are already `core_equiv`; the identical-branches fold handles
    // that (and would have fired first). Nothing to hoist here.
    if diff == 0 {
        return None;
    }
    // A trapping `cond` needs both a COUNT check AND an ORDER check before it may hoist — the same guard
    // `hoist_common_ctor` uses (a shared operand is built OUTSIDE the per-position `if`, so it evaluates
    // BEFORE `cond`).
    if !is_trap_free(db, cond) {
        // COUNT: `cond` is evaluated once per differing operand; only ONE differing operand matches the
        // original's single evaluation. Any other count would duplicate/drop a cond trap or effect.
        if diff != 1 {
            return None;
        }
        // ORDER: the original `if` evaluates `cond` FIRST — before ANY operand. In the hoisted form a
        // SHARED operand PRECEDING the differing one is built outside the per-position `if`, so it runs
        // before `cond`. `core_equiv` admits trapping arith (a `/` by a runtime divisor is `core_equiv`
        // to itself), so a preceding shared operand that traps would PREEMPT `cond`'s own trap — observing
        // the WRONG trap kind. Only hoist a trapping cond when every shared operand preceding the differing
        // one is itself trap-free (then the first observable trap is still `cond`'s). For binary `Arith`
        // the only preceding position is lhs when the diff is at rhs; a unary `Convert` has a single
        // operand so a diff==1 Convert has no preceding shared operand and is unaffected.
        let diff_idx = first_diff.expect("diff == 1 guarantees a differing position");
        for &(a, _) in &pairs[..diff_idx] {
            if !is_trap_free(db, a) {
                return None;
            }
        }
    }
    let mut operands: Vec<StructId> = Vec::with_capacity(pairs.len());
    for &(a, b) in &pairs {
        operands.push(if core_equiv(db, a, b) {
            a
        } else {
            synth_if_hoisted(db, cond, a, b)
        });
    }
    Some(match head {
        Head::Arith(op) => Core::Arith {
            op,
            lhs: operands[0],
            rhs: operands[1],
        },
        Head::Compare(op) => Core::Compare {
            op,
            lhs: operands[0],
            rhs: operands[1],
        },
        Head::FloatCompare(op, width) => Core::FloatCompare {
            op,
            lhs: operands[0],
            rhs: operands[1],
            width,
        },
        Head::Convert(op) => Core::Convert {
            op,
            operand: operands[0],
        },
    })
}

/// The "not-yet-computed on a runtime string" DECLINE for a string operation whose `arg` did not fold
/// to a constant — BUT only when `arg` is actually a `String`. When `arg` is NOT a string (`(Symbol.of
/// 5)` — a type error `infer` already reports as CDZ0203), the "runtime string" wording is a lie that
/// shadows the real type error; emit a NEUTRAL decline instead so the coded CDZ0203 is the story the
/// reader sees. (A genuine runtime string keeps the precise `msg`, the honest "constant strings only"
/// increment note.)
pub(super) fn runtime_string_op_decline(db: &mut Db, arg: crate::ast::StructId, msg: &str) -> Core {
    if matches!(crate::infer::type_of(db, arg), crate::ty::Ty::String) {
        Core::Poison(Reject::decline(msg.to_string()))
    } else {
        // Not a string — the type mismatch (CDZ0203) is the authoritative report; do not claim a
        // "runtime string" it is not. This decline is generic (a lowering can't proceed on an
        // ill-typed operand) and defers to the coded type error.
        Core::Poison(Reject::decline(
            "this operation's operand is not a string (see the type error above)",
        ))
    }
}

/// The GENERIC "defer to the coded type error" decline text — the self-describing "(see the type error
/// above)" marker that [`crate::compile::dedup_faults`] drops whenever a member-op wrong-arg-type CDZ0203
/// is present (so an ill-typed operand is ONE primary `error:`). The typed-family sibling of
/// [`runtime_string_op_decline`]'s String-specific text.
pub(super) const ILL_TYPED_OPERAND_DECLINE: &str =
    "this operation's operand is not of the expected type (see the type error above)";

/// The decline for a collection/typed op whose operand did NOT resolve to the expected type. When the
/// operand is a DEFINITE MISMATCH — its top-level type is a known constructor that is not the expected
/// collection kind (`(Set.contains 5 …)` — `5` is an `Int`, definitely not a `Set`, even with a
/// deferred integer width) — the CDZ0203 type mismatch is authoritative (`infer` emits it), so emit the
/// neutral [`ILL_TYPED_OPERAND_DECLINE`] that `dedup_faults` drops: one primary error, never an
/// uncoded/misleading shadow. Otherwise — the operand IS the expected kind but with an UNSOLVED element/
/// key type, or its type is a bare `Ty::Var`/`Ty::Any` — `infer` has no definite CDZ0203, so keep
/// `unsolved_msg` (the honest "operand is not a solved X" report, the only report of the unsolved operand).
///
/// NB: the caller computes `is_definite_mismatch` by the top-level KIND (`!matches!(ty, Set(_) | Var | Any)`
/// etc.), NOT `is_fully_solved` — a deferred-width int literal is "not fully solved" yet is still a
/// definite non-collection whose CDZ0203 infer reports.
pub(super) fn ill_typed_operand_decline(is_definite_mismatch: bool, unsolved_msg: &str) -> Core {
    if is_definite_mismatch {
        Core::Poison(Reject::decline(ILL_TYPED_OPERAND_DECLINE))
    } else {
        Core::Poison(Reject::decline(unsolved_msg.to_string()))
    }
}

/// Fold a constant SHIFT/BITWISE op (`BitAnd`/`BitOr`/`BitXor`/`Shl`/`Shr`) over the SOLVED WIDTH's u128
/// two's-complement bit pattern, rather than `fold_arith`'s i64 path. This is correct at ANY fixed width
/// (including UInt64, whose values above `i64::MAX` the i64 path spuriously rejects) AND for a small-operand
/// shift whose RESULT exceeds i64 but fits the solved width (`(<< (: 1 UInt64) 63)` = 2^63 fits UInt64 —
/// `checked_shl_i64` wrongly overflow-checked against Int64). Returns `Some(Core)` (the folded `ConstInt`
/// or a CDZ0304 trap for an out-of-range shift count / an overflow past the width), or `None` when the op is
/// not a shift/bitwise (caller falls back to `fold_arith`). Operands reaching here are non-negative at their
/// width (a signed value ≥ 2^63 cannot type — CDZ0302 at its literal), so `>>` is a logical shift; bitwise
/// ops are total on the value bits (masked to width). Trap semantics match the i64 helpers, generalized to
/// the solved width: a shift COUNT ≥ width traps, a `<<` bit shifted past the width traps.
pub(super) fn fold_shift_bitwise_at_width(
    op: Prim,
    a: &IntValue,
    b: &IntValue,
    signed: bool,
    width: u32,
) -> Option<Core> {
    if !matches!(
        op,
        Prim::BitAnd | Prim::BitOr | Prim::BitXor | Prim::Shl | Prim::Shr
    ) {
        return None;
    }
    // UNSIGNED ONLY. A SIGNED type keeps `fold_arith`'s i64 path: (a) a signed `>>` is ARITHMETIC
    // (sign-extending) — `-256 >> 4 = -16` — which this logical-u128 fold would get wrong (it reads the
    // magnitude of a `wrap_to` result, so a negative value reads as 0); and (b) a signed value's high bits
    // are sign extension a bitwise mask would mismodel. The i64 path is already CORRECT for every signed
    // fixed width (its result range-check catches signed overflow); the ONLY thing it got wrong is an
    // UNSIGNED-width result above `i64::MAX` (`(<< (: 1 UInt64) 63)` = 2^63, `& (: u64max UInt64) …`), where a
    // logical width-masked fold is exactly right (unsigned operands are non-negative, `>>` is logical).
    if signed {
        return None;
    }
    // The operands' low-`width` bit patterns as u128 (`wrap_to` = the canonical two's-complement-at-width the
    // backend uses; a non-negative value's magnitude IS its bit pattern — `to_u128`).
    let bits = |v: &IntValue| -> u128 { v.wrap_to(signed, width).to_u128().unwrap_or(0) };
    let (xb, yb) = (bits(a), bits(b));
    let mask = if width >= 128 {
        u128::MAX
    } else {
        (1u128 << width) - 1
    };
    // `Ok(value)` folds; `Err(reason)` is a provable trap → CDZ0304 carrying the SPECIFIC cause (an
    // out-of-range shift count vs a shifted-result overflow), matching `fold_arith`/`const_trap_cause`'s
    // actionable style rather than one generic "count or overflow" message. Bitwise ops never trap.
    let folded: Result<u128, String> = match op {
        Prim::BitAnd => Ok((xb & yb) & mask),
        Prim::BitOr => Ok((xb | yb) & mask),
        Prim::BitXor => Ok((xb ^ yb) & mask),
        // `<<`: count must be in `0..width`; fold only when no bit is shifted PAST the width (else the checked
        // default traps — matching the i64 helper's `None`-on-overflow, but against the SOLVED width not i64).
        Prim::Shl => {
            let count = yb;
            if count >= width as u128 {
                Err(format!(
                    "shift count {count} is out of range for the {width}-bit type \
                     (a shift count must be 0..={})",
                    width - 1
                ))
            } else {
                match xb.checked_shl(count as u32) {
                    Some(s) if s == (s & mask) => Ok(s),
                    _ => Err(format!(
                        "the shifted result overflows the {width}-bit type \
                         (a `<<` by {count} moves a set bit past the width) — shift a smaller value \
                         or use a wider type"
                    )),
                }
            }
        }
        // `>>`: logical shift (operand non-negative at its width); count in `0..width`.
        Prim::Shr => {
            let count = yb;
            if count >= width as u128 {
                Err(format!(
                    "shift count {count} is out of range for the {width}-bit type \
                     (a shift count must be 0..={})",
                    width - 1
                ))
            } else {
                Ok((xb >> count) & mask)
            }
        }
        _ => unreachable!("guarded by the matches! above"),
    };
    Some(match folded {
        Ok(r) => {
            trace!(target: "rcdzc::fold", op = intrinsic_name(op), "constant shift/bitwise folded over the solved width");
            Core::ConstInt(IntValue::from_u128(r))
        }
        Err(reason) => {
            trace!(target: "rcdzc::fold", op = intrinsic_name(op), %reason, "constant shift traps → CDZ0304");
            Core::Poison(Reject::coded(
                Code::ConstTrap,
                format!("this constant `{}` traps: {reason}", intrinsic_name(op)),
            ))
        }
    })
}

/// Fold a constant arithmetic operation with a CHECKED evaluation. Both operands are compile-time
/// constants; if the operation's defined outcome on them is a trap (an overflow the checked default
/// forbids, or an operand outside the machine range the fold evaluates over), the result is a poison
/// carrying CDZ0304 — the build fails rather than shipping a runtime trap. On success the result is a
/// `ConstInt`. The evaluation is over `i64` (the Stage default integer); a later width stage
/// generalizes the range the check tests to the operands' solved width.
pub(super) fn fold_arith(op: Prim, a: IntValue, b: IntValue) -> Core {
    let (x, y) = match (a.to_i64(), b.to_i64()) {
        (Some(x), Some(y)) => (x, y),
        // An operand beyond the machine range the fold evaluates over — a provable width trap.
        _ => {
            return Core::Poison(Reject::coded(
                Code::ConstTrap,
                "constant operand does not fit the integer width",
            ));
        }
    };
    // Each integer op evaluates over `i64` (the Stage default width) with the DEFINED numeric-model
    // semantics; `None` marks a provable trap the checked default forbids (`numeric-model.md` §Overflow
    // Is Defined). A later width stage generalizes the range/count the checks test to the solved width.
    let checked = match op {
        Prim::Add => x.checked_add(y),
        Prim::Sub => x.checked_sub(y),
        Prim::Mul => x.checked_mul(y),
        // Division truncates toward zero; traps on a zero divisor and on `MIN / -1` (Rust's
        // `checked_div` returns `None` for both — exactly the two defined traps).
        Prim::Div => x.checked_div(y),
        // Remainder takes the dividend's sign; traps on a zero divisor. `MIN % -1` is 0 (no overflow),
        // but Rust's `%` panics there — `checked_rem` returns `None`, so special-case it to 0.
        Prim::Rem => {
            if y == -1 {
                Some(0)
            } else {
                x.checked_rem(y)
            }
        }
        // A left shift is exact multiplication by `2^count`: it traps on an out-of-range count
        // (< 0 or ≥ width) AND on overflow past the width — NOT wasm's silent mask-and-wrap.
        Prim::Shl => checked_shl_i64(x, y),
        // Arithmetic (sign-extending) right shift; traps on an out-of-range count, never overflows.
        Prim::Shr => checked_shr_i64(x, y),
        // Bitwise operations are total on the two's-complement value — never trap.
        Prim::BitAnd => Some(x & y),
        Prim::BitOr => Some(x | y),
        Prim::BitXor => Some(x ^ y),
        // A non-integer-binary prim never reaches the fold (`lower_arith` is only called for an
        // `is_arith` prim), so these arms are unreachable in practice; decline rather than panic.
        Prim::Lt
        | Prim::Gt
        | Prim::Le
        | Prim::Ge
        | Prim::Eq
        | Prim::Compare
        | Prim::Wrap
        | Prim::CheckedOf
        | Prim::IntCtor
        | Prim::UIntCtor
        | Prim::FnCtor
        | Prim::TupleCtor
        | Prim::RecordCtor
        | Prim::BoolTy
        | Prim::UnitTy
        | Prim::SumNew
        | Prim::SumCtor
        | Prim::TupleNew
        | Prim::RecordNew
        | Prim::RecordProject
        | Prim::RecordWithout
        | Prim::RecordMerge
        | Prim::RecordExtend
        | Prim::RecordWith
        | Prim::RecordPop
        | Prim::TupleCat
        | Prim::TupleSplitAt
        | Prim::TuplePop
        | Prim::ListNew
        | Prim::SetNew
        | Prim::ListLen
        | Prim::ListPush
        | Prim::ListPrepend
        | Prim::ListConcat
        | Prim::ListUpdate
        | Prim::ListAt
        | Prim::AstSpliceLift
        | Prim::AstLift
        | Prim::AstEncode
        | Prim::AstDecode
        | Prim::Blake3Of
        | Prim::ListCtor
        | Prim::BytesOf
        | Prim::BytesLen
        | Prim::BytesTy
        | Prim::StrScalarLen
        | Prim::StrByteLen
        | Prim::StrAt
        | Prim::StrScalarAt
        | Prim::StrConcat
        | Prim::StrSlice
        | Prim::StrToBytes
        | Prim::StrFromBytes
        | Prim::SumExpect
        | Prim::CheckedAdd
        | Prim::CheckedSub
        | Prim::CheckedMul
        | Prim::WrappingAdd
        | Prim::WrappingSub
        | Prim::WrappingMul
        | Prim::StringTy
        | Prim::BytesAt
        | Prim::BytesConcat
        | Prim::BytesSlice
        | Prim::BytesCompact
        // Float arithmetic is folded by `lower_float_arith` (an f64/f32 fold), not this integer fold.
        | Prim::FAdd
        | Prim::FSub
        | Prim::FMul
        | Prim::FDiv
        | Prim::FloatCtor
        | Prim::FloatOfInt
        | Prim::FloatOf
        | Prim::FloatNan
        | Prim::FloatInf
        | Prim::MapCtor
        | Prim::MapNew
        | Prim::MapEmpty
        | Prim::MapInsert
        | Prim::MapLookup
        | Prim::MapRemove
        | Prim::MapSize
        | Prim::MapSwap
        | Prim::MapTake
        | Prim::SetCtor
        | Prim::SetOf
                | Prim::MapToList
| Prim::SetToList
        | Prim::SetContains
        | Prim::SetLen
        | Prim::SetInsert
        | Prim::SetRemove
        | Prim::SetUnion
        | Prim::SetIntersection
        | Prim::SetDifference
        | Prim::CharTy
        | Prim::CharToInt
        | Prim::CharFromInt
        // Value.encode/decode are unary value↔Bytes conversions (they lower in their own apply-dispatch
        // arm, like Char.to-int), NOT integer binary ops — unreachable in the arith fold, decline.
        | Prim::ValueEncode
        | Prim::ValueDecode
        | Prim::SymbolTy
        | Prim::SymbolOf
        | Prim::SymbolToString
        // `BigIntTy` is a ground type-value builder (bare `BigInt` in type position → `Ty::BigInt`),
        // and `BigIntOf` is the unary widening conversion (folds in its own arm above) — neither is an
        // integer BINARY operation, like `StringTy`/`SymbolTy`/`SymbolOf`. `RationalTy` is likewise a
        // ground type-value builder (bare `Rational` → `Ty::Rational`), not an integer binary op.
        | Prim::BigIntTy
        | Prim::BigIntOf
        | Prim::RationalTy
        // `RationalOf`/`RationalOfInt`/`RationalValue`/`RationalNum`/`RationalDen` are rational construction/
        // conversion/accessor ops (they fold in their own arms), not integer binary operations.
        | Prim::RationalOf
        | Prim::RationalOfInt
        | Prim::RationalValue
        | Prim::RationalNum
        | Prim::RationalDen
        | Prim::RationalTruncate
        | Prim::RationalFloor
        | Prim::RationalCeil
        | Prim::RationalRound
        // The unit/quantity prims are compile-time unit builders / erasing quantity ops — never an
        // integer binary operation (a `Qty.of`/`Qty.value` lowers to its value argument, a unit builder
        // is reduced away by `eval`), so they never reach this integer fold.
        | Prim::UnitOne
        | Prim::UnitBase
        | Prim::UnitMul
        | Prim::UnitDiv
        | Prim::UnitPow
        | Prim::UnitPrefix
        | Prim::UnitOf
        | Prim::UnitDefine
        | Prim::UnitIn
        | Prim::QtyOf
        | Prim::QtyValue
        | Prim::QtyPow
        | Prim::QtyUnit
        | Prim::QtyCtor
        | Prim::TypeOf
        | Prim::TypeEq
        // `Type.ast`/`Type.ast-generic` fold to an `Ast` VALUE (type→AST reflection), never an integer op.
        | Prim::TypeAst { .. }
        // `trap` is the diverging primitive (lowered to `Core::Trap`), never an integer binary operation.
        | Prim::Trap
        // `print`/`read` are the AST-value text printer/reader (`Ast → String` / `String → Ast`), folded
        // in `lower_print`/`lower_read`, never an integer binary operation.
        | Prim::Print
        | Prim::Read
        // `Ast.module` is the self-reflection magic-constant (folds to the enclosing module's `Ast`
        // value at lowering — v-compiler-primitives' fill), never an integer binary operation.
        | Prim::ReflectModule
        | Prim::FEq
        | Prim::FLt
        | Prim::FLe
        | Prim::FGt
        | Prim::FGe => {
            return Core::Poison(Reject::decline("not an integer binary operation"));
        }
    };
    match checked {
        Some(n) => {
            trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = n, "folded constant integer op");
            Core::ConstInt(IntValue::from_i64(n))
        }
        // A provable trap — the checked default traps, and the compiler can prove it, so the build
        // fails (CDZ0304) rather than emitting a component that traps (`numeric-model.md` §A Constant
        // Operation With No Value Is Rejected At Compile Time).
        None => {
            trace!(target: "rcdzc::fold", op = intrinsic_name(op), "constant op traps → CDZ0304 (fails build)");
            Core::Poison(Reject::coded(
                Code::ConstTrap,
                format!(
                    "constant {} traps: {}",
                    intrinsic_name(op),
                    const_trap_cause(op, y),
                ),
            ))
        }
    }
}
