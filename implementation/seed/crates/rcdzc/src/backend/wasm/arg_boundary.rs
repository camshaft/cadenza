//! ABI boundary-shape analysis for compound export arguments (extracted from `mod.rs`).
//!
//! The pure, `Db`-reading predicates the resource-escape emitters (`emit_*_resource` in the parent
//! module) call to decide how a fixed-shape compound argument — an option/result/sum payload, a
//! scalar-or-nested tuple, a record field — crosses the host boundary, and to build its rebuild plan.
//! `closure_boundary_reject`/`_byte` classify a closure `call`-boundary type (accept as an aliased
//! width, or decline with a precise message). No behavior change: this is a mechanical relocation of a
//! cohesive cluster of `mod.rs` free functions; imports are function-local and travel with each body.

use crate::db::Db;
use crate::diag::Reject;

/// Emit the CLOSURE-RESOURCE component (C-HOST-1): an export whose RESULT is a closure `(-> A… R)` crosses
/// as a component resource with a `call` method the host invokes. The export body lowers NORMALLY (its
/// `Core::Closure` builds the cell + lifts the lambda into `layout.lifted`); the core's `make` calls the
/// export then `resource.new`s the cell, and `call` recovers it (`resource.rep`) + `call_indirect`s the
/// lifted body. Reuses the value-heap runtime (the cell is a heap allocation) via
/// `assemble_closure_resource`. First cut: the closure's args + result are the aliased scalar widths
/// (`abi_val_type`); a compound/closure arg or an un-representable result declines.
/// The decline for a closure `role` (`"argument"` / `"result"` / `"parameter"`) whose type `ty` cannot
/// cross the closure `call` host boundary. When `ty` is UNCONSTRAINED — `Any`, or an unresolved
/// unification variable `Ty::Var(_)` that inference never grounded (which `render_name` prints as a raw
/// `?7`) — the "no scalar representation" phrasing is misleading (it reads as if a real type is
/// unsupported), and dumping the internal `?7` at the user is the leaky-message anti-pattern. Both denote
/// the same thing: the usual cause is a PARTIAL APPLICATION escaping as an export result (e.g. an
/// entrypoint returning `(f 1)` for a two-parameter `f`), whose remaining parameter has no solved type.
/// Say THAT. A concrete-but-unrepresentable type (a compound, a nested closure) keeps the precise
/// "no scalar host-boundary representation (only aliased widths cross yet)" message.
pub(super) fn closure_boundary_reject(
    role: &str,
    ty: &crate::ty::Ty,
    ncx: &crate::ty::NameCtx,
) -> Reject {
    if matches!(ty, crate::ty::Ty::Any | crate::ty::Ty::Var(_)) {
        Reject::decline(format!(
            "a closure crossing the host boundary has an unconstrained {role} type — the entrypoint's \
             result is a closure whose {role} type inference never fixed (a partial application like \
             `(f 1)` for a two-parameter `f`, or a closure with an unannotated parameter); a closure \
             crosses the boundary only with concrete, aliased-width scalar {role}s",
        ))
    } else {
        Reject::decline(format!(
            "a closure {role} of type {} has no scalar host-boundary representation (only aliased-width \
             scalars — every s8/u8/…/s64/u64 int, bool, f32/f64 — cross the closure `call` boundary)",
            ty.render_name(ncx)
        ))
    }
}

/// The COMPONENT-model boundary byte a closure arg/result/param crosses under — every ALIASED-WIDTH
/// SCALAR the ordinary export boundary supports (each of s8/u8/s16/u16/s32/u32/s64/u64, bool, f32/f64).
/// Wider than the runtime-op ABI table's `abi_val_type` (which models only u32/s64/bool/f64 for the
/// value-heap ops), because a closure's `call` boundary is a plain component functype — it needs only the
/// component primitive byte + the core valtype (from `valtype_of`), neither tied to the runtime-op set.
///
/// WARNING: Restricted to genuine SCALARS: `comp_valtype_of` ALSO returns a byte for a `Tuple` (the u32 HANDLE it
/// is threaded as BETWEEN in-program functions) and for a `Nominal`-over-compound, but those are opaque
/// runtime handles, NOT values the host can construct or read across the `call` boundary — a closure with a
/// COMPOUND arg/result must decline (that widening is a separate later increment). So this accepts only
/// Int/Bool/Float (peeling a nominal to its underlying scalar first); everything else is `None`.
pub(super) fn closure_boundary_byte(ty: &crate::ty::Ty) -> Option<u8> {
    use crate::ty::Ty;
    match ty.strip_nominal() {
        Ty::Int(_) | Ty::Bool | Ty::Float(_) => crate::backend::wasm::lir::comp_valtype_of(ty),
        _ => None,
    }
}

/// A FIXED-SHAPE SUM closure ARGUMENT — an `(Option scalar)` OR a `(Result scalar scalar)` — that crosses the
/// DIRECT-CALL boundary as a native component `option<payload>` / `result<ok,err>` (the canonical ABI flattens
/// either to `(disc: i32, payload)` core params, the payload slot the JOIN of both cases' scalars). Returns
/// the `ArgSlot` (`OptionScalar`/`Result`, minting the boundary type), the flattened payload's core valtype
/// (the `disc` is always i32), and the [`serialize::SumArgRebuild`] the core `call` uses to rebuild the cell
/// (branch on the boundary disc → `sum-new`). `None` unless `ty` is a two-variant Option/Result whose
/// payload-bearing variant(s) each carry ONE aliased-width scalar; a nullary+payload sum is Option, two
/// payloads is Result. A general user sum needs a NAMED `variant<…>` (out of scope). For Result, both
/// payloads must be the SAME core width (their flattened join is one param). Discs are DECL indices; the
/// boundary disc is the component-model convention (option Some=1; result Ok=0/Err=1).
#[allow(clippy::type_complexity)]
pub(super) fn fixed_shape_option_scalar_arg(
    db: &mut Db,
    ty: &crate::ty::Ty,
) -> Option<(
    crate::backend::wasm::envelope::ArgSlot,
    Vec<crate::backend::wasm::lir::ValType>,
    crate::backend::wasm::serialize::SumArgRebuild,
)> {
    use crate::backend::wasm::envelope::ArgSlot;
    use crate::backend::wasm::serialize::{SumArgArm, SumArgRebuild, SumArmPayload};
    use crate::ty::Ty;
    let Ty::Sum { decl, args, .. } = ty.strip_nominal() else {
        return None;
    };
    // Snapshot the decl's params + per-variant payload-occurrence lists into owned data, so the `db.ast`
    // reads below don't overlap the `decl_ref` borrow.
    let (params, variant_payloads): (Vec<String>, Vec<Vec<crate::ast::StructId>>) = {
        let decl_ref = db.type_decl_by_occ(*decl)?;
        if decl_ref.variants.len() != 2 {
            return None; // EXACTLY two variants (Option = `(Some a) None`; Result = `(Ok a) (Err b)`).
        }
        (
            decl_ref.params.clone(),
            decl_ref
                .variants
                .iter()
                .map(|v| v.payloads.clone())
                .collect(),
        )
    };
    // The instantiated TYPE of a variant's ONE generic payload (a param `a` → `args[pi]`). `None` if the
    // payload occurrence is not a bare generic parameter (Option/Result payloads are generic; a concrete-payload
    // user sum is the named-`variant<…>` widening, out of scope here).
    let resolve_payload_ty = |db: &mut Db, occ: crate::ast::StructId| -> Option<crate::ty::Ty> {
        let pname = db
            .ast
            .head_name(occ)
            .or_else(|| db.ast.as_name(occ))?
            .to_string();
        let pi = params.iter().position(|p| *p == pname)?;
        args.get(pi).cloned()
    };
    // The instantiated scalar-field-rebuild for a variant's ONE generic SCALAR payload.
    // Returns `(payload_ty, box_op, extend)`; `None` if the variant is not exactly one aliased-width scalar.
    let resolve_scalar_payload = |db: &mut Db,
                                  occ: crate::ast::StructId|
     -> Option<(crate::ty::Ty, &'static str, Option<bool>)> {
        let pty = resolve_payload_ty(db, occ)?;
        let crate::backend::wasm::serialize::FieldRebuild::Scalar { box_op, extend } =
            scalar_field_rebuild(&pty)?
        else {
            return None;
        };
        Some((pty, box_op, extend))
    };
    // Two nullary → a bare enum (not option/result); handle by payload counts.
    let counts: Vec<usize> = variant_payloads.iter().map(|p| p.len()).collect();
    match counts.as_slice() {
        // OPTION shape: exactly one nullary + one single-payload variant. Crosses as `option<payload>`
        // (Some=boundary disc 1). arm order: which decl index is the payload variant. The payload is either an
        // aliased-width SCALAR (crosses as `option<scalar>`, one flattened leaf) or a fixed-shape COMPOUND
        // tuple/record (crosses as `option<tuple<…>>` — both formers anonymous-allowed, so no `variant` wall;
        // its leaves flatten depth-first after the disc, and the Some arm rebuilds the payload cell).
        [0, 1] | [1, 0] => {
            let (payload_i, nullary_i) = if counts[0] == 1 {
                (0u32, 1u32)
            } else {
                (1u32, 0u32)
            };
            let payload_occ = variant_payloads[payload_i as usize][0];
            let payload_ty = resolve_payload_ty(db, payload_occ)?;
            if let Some((box_op, extend)) = match scalar_field_rebuild(&payload_ty) {
                Some(crate::backend::wasm::serialize::FieldRebuild::Scalar { box_op, extend }) => {
                    Some((box_op, extend))
                }
                _ => None,
            } {
                // SCALAR payload → `option<scalar>`.
                let payload_byte = closure_boundary_byte(&payload_ty)?;
                let payload_vt = crate::backend::wasm::lir::valtype_of(&payload_ty)?;
                Some((
                    ArgSlot::OptionScalar(payload_byte),
                    vec![payload_vt],
                    SumArgRebuild {
                        base_param: 1,
                        boundary_true_disc: 1, // component `option<T>` sends Some=1
                        arm_true: SumArgArm {
                            decl_disc: payload_i,
                            payload: SumArmPayload::Scalar {
                                box_op,
                                extend,
                                wrap_join: false, // option has a single payload — no wider join to recover
                            },
                        },
                        arm_false: SumArgArm {
                            decl_disc: nullary_i,
                            payload: SumArmPayload::Nullary,
                        },
                    },
                ))
            } else {
                // COMPOUND payload → `option<tuple<…>>`: the payload's leaves flatten depth-first after the disc.
                let (_leaf_bytes, leaf_vts, rebuild_fields, shape_fields) =
                    nested_fixed_shape_tuple_arg(&payload_ty)?;
                Some((
                    ArgSlot::OptionCompound(shape_fields),
                    leaf_vts,
                    SumArgRebuild {
                        base_param: 1,
                        boundary_true_disc: 1, // component `option<T>` sends Some=1
                        arm_true: SumArgArm {
                            decl_disc: payload_i,
                            payload: SumArmPayload::Compound(rebuild_fields),
                        },
                        arm_false: SumArgArm {
                            decl_disc: nullary_i,
                            payload: SumArmPayload::Nullary,
                        },
                    },
                ))
            }
        }
        // RESULT shape: two single-scalar-payload variants (Ok a, Err b). Crosses as `result<ok,err>` — the
        // FIRST-declared variant is `ok` (boundary disc 0), the second `err` (disc 1). `resolve_scalar_payload`
        // gives each's box op. The canonical ABI flattens `result<ok,err>` to `(disc: i32, payload: JOIN)` where
        // the payload core valtype is the JOIN of the two sides — the WIDER core (i64 if either side is i64,
        // else i32). When the two sides are the same core width, both read the join directly (the original
        // same-width case). When they DIFFER (one i64, one i32-core — e.g. `Result Int64 Int32`), the narrow
        // side arrives widened into the joined i64 and must `i32.wrap_i64` to recover its bits before its own
        // (re-)extend — the `wrap_join` flag. Proven by the diff-width Result oracle.
        [1, 1] => {
            let (ok_ty, ok_box, ok_ext) = resolve_scalar_payload(db, variant_payloads[0][0])?;
            let (err_ty, err_box, err_ext) = resolve_scalar_payload(db, variant_payloads[1][0])?;
            let ok_byte = closure_boundary_byte(&ok_ty)?;
            let err_byte = closure_boundary_byte(&err_ty)?;
            let ok_vt = crate::backend::wasm::lir::valtype_of(&ok_ty)?;
            let err_vt = crate::backend::wasm::lir::valtype_of(&err_ty)?;
            use crate::backend::wasm::lir::ValType;
            // The joined payload core valtype: i64 if EITHER side is i64, else i32 (both i32-core). Floats are
            // their own core width; a float↔int mix has no common numeric join here, so decline.
            let join_vt = match (ok_vt, err_vt) {
                (a, b) if a == b => a,
                (ValType::I64, ValType::I32) | (ValType::I32, ValType::I64) => ValType::I64,
                _ => return None, // f32↔f64 / int↔float mixed join — a later widening
            };
            // A side whose own core is NARROWER than the join arrives widened into it → wrap to recover.
            let ok_wrap = ok_vt != join_vt;
            let err_wrap = err_vt != join_vt;
            Some((
                ArgSlot::Result(ok_byte, err_byte),
                vec![join_vt],
                SumArgRebuild {
                    base_param: 1,
                    boundary_true_disc: 0, // component `result<ok,err>` sends Ok=0
                    arm_true: SumArgArm {
                        decl_disc: 0, // Ok = the first-declared variant
                        payload: SumArmPayload::Scalar {
                            box_op: ok_box,
                            extend: ok_ext,
                            wrap_join: ok_wrap,
                        },
                    },
                    arm_false: SumArgArm {
                        decl_disc: 1, // Err = the second
                        payload: SumArmPayload::Scalar {
                            box_op: err_box,
                            extend: err_ext,
                            wrap_join: err_wrap,
                        },
                    },
                },
            ))
            // If EITHER scalar payload resolution failed above, `resolve_scalar_payload`'s `?` already returned
            // None; the COMPOUND-payload Result path is a separate classifier (`fixed_shape_result_compound_arg`)
            // tried by the caller when this returns None.
        }
        _ => None, // two nullary (bare enum), or a multi-payload variant — out of scope
    }
}

/// A `(Result ok err)` closure ARG where AT LEAST ONE side's payload is a fixed-shape TUPLE/record (a compound)
/// — the compound-Result-payload path, the counterpart to [`fixed_shape_option_scalar_arg`]'s compound Option.
/// It crosses as a native `result<ok, err>` whose ok/err valtypes are each a primitive (scalar side) or a
/// minted `tuple<…>` (compound side). The canonical ABI flattens it to `(disc: i32, <joined leaves…>)`: each
/// arm's payload flattens to a leaf list, and the two are JOINED position-by-position (the join length = the
/// LONGER arm; each position's width = the wider arm's leaf there). The guest rebuilds the selected arm's cell
/// over a PREFIX of the joined slots. SCOPE this increment: each shared position has the SAME core width across
/// both arms (no per-leaf `wrap` needed) — a differing per-position width is a later widening (declines here).
/// Returns `(ArgSlot::ResultCompound, joined leaf vts, SumArgRebuild)` or `None` (not a 2-payload Result, a
/// non-fixed-shape payload, or a differing per-position join width).
pub(super) fn fixed_shape_result_compound_arg(
    db: &mut Db,
    ty: &crate::ty::Ty,
) -> Option<(
    crate::backend::wasm::envelope::ArgSlot,
    Vec<crate::backend::wasm::lir::ValType>,
    crate::backend::wasm::serialize::SumArgRebuild,
)> {
    use crate::backend::wasm::envelope::{ArgSlot, ResultSide};
    use crate::backend::wasm::serialize::{SumArgArm, SumArgRebuild, SumArmPayload};
    use crate::ty::Ty;
    let Ty::Sum { decl, args, .. } = ty.strip_nominal() else {
        return None;
    };
    let (params, variant_payloads): (Vec<String>, Vec<Vec<crate::ast::StructId>>) = {
        let decl_ref = db.type_decl_by_occ(*decl)?;
        if decl_ref.variants.len() != 2 {
            return None;
        }
        (
            decl_ref.params.clone(),
            decl_ref
                .variants
                .iter()
                .map(|v| v.payloads.clone())
                .collect(),
        )
    };
    // BOTH variants must carry exactly one payload (a Result `(Ok a) (Err b)` — not an Option's nullary arm).
    if variant_payloads.iter().any(|p| p.len() != 1) {
        return None;
    }
    // The instantiated payload TYPE of a variant (a generic param `a` → `args[pi]`).
    let payload_ty = |db: &mut Db, occ: crate::ast::StructId| -> Option<Ty> {
        let pname = db
            .ast
            .head_name(occ)
            .or_else(|| db.ast.as_name(occ))?
            .to_string();
        let pi = params.iter().position(|p| *p == pname)?;
        args.get(pi).cloned()
    };
    let ok_ty = payload_ty(db, variant_payloads[0][0])?;
    let err_ty = payload_ty(db, variant_payloads[1][0])?;
    // At least ONE side must be compound (else the all-scalar path handles it); classify each side.
    let classify_side = |ty: &Ty| -> Option<(
        ResultSide,
        Vec<crate::backend::wasm::lir::ValType>,
        SumArmPayload,
        u32,
    )> {
        // Returns (boundary side, leaf vts, arm payload, decl_disc-placeholder). A scalar → `Scalar`; a
        // fixed-shape tuple/record → `Compound` (its depth-first leaves). Neither → None.
        if let Some(crate::backend::wasm::serialize::FieldRebuild::Scalar { box_op, extend }) =
            scalar_field_rebuild(ty)
        {
            let byte = closure_boundary_byte(ty)?;
            let vt = crate::backend::wasm::lir::valtype_of(ty)?;
            Some((
                ResultSide::Scalar(byte),
                vec![vt],
                SumArmPayload::Scalar {
                    box_op,
                    extend,
                    wrap_join: false,
                },
                0,
            ))
        } else if let Some((_leaf_bytes, leaf_vts, rebuild_fields, shape)) =
            nested_fixed_shape_tuple_arg(ty)
        {
            Some((
                ResultSide::Compound(shape),
                leaf_vts,
                SumArmPayload::Compound(rebuild_fields),
                0,
            ))
        } else {
            None
        }
    };
    let (ok_side, ok_vts, ok_payload, _) = classify_side(&ok_ty)?;
    let (err_side, err_vts, err_payload, _) = classify_side(&err_ty)?;
    // Require at least one compound side (the all-scalar Result is the other path).
    if matches!(ok_side, ResultSide::Scalar(_)) && matches!(err_side, ResultSide::Scalar(_)) {
        return None;
    }
    // Position-wise join: the join length is the LONGER arm; each shared position must have the SAME core width
    // across arms (this increment). A differing width (would need per-leaf wrap inside the compound rebuild) or
    // a length where the shorter arm's leaves don't PREFIX-match declines.
    let join_len = ok_vts.len().max(err_vts.len());
    let mut joined = Vec::with_capacity(join_len);
    for i in 0..join_len {
        match (ok_vts.get(i), err_vts.get(i)) {
            (Some(a), Some(b)) if a == b => joined.push(*a),
            (Some(a), None) => joined.push(*a),
            (None, Some(b)) => joined.push(*b),
            _ => return None, // differing per-position width — a later widening
        }
    }
    Some((
        ArgSlot::ResultCompound(ok_side, err_side),
        joined,
        SumArgRebuild {
            base_param: 1,
            boundary_true_disc: 0, // component `result<ok,err>` sends Ok=0
            arm_true: SumArgArm {
                decl_disc: 0,
                payload: ok_payload,
            },
            arm_false: SumArgArm {
                decl_disc: 1,
                payload: err_payload,
            },
        },
    ))
}

/// A fixed-shape sum PARAM field whose arm(s) carry a `list<u8>` (Bytes) or an all-nullary WIT `enum` payload
/// — the reducer response's `answer: result<payload, error>` = `result<list<u8>, enum>`, which
/// [`fixed_shape_option_scalar_arg`] does not read (it covers scalar/compound arms). Builds the
/// [`serialize::FieldRebuild::Sum`] and APPENDS the flattened `(disc: i32, payload-join…)` core valtypes to
/// `param_vts`, mirroring `emit_sum_field`/`flattened_param_count`. Handles a two-variant Option (one nullary +
/// one payload) or Result (two payloads), each payload Nullary / aliased-width Scalar / `list<u8>` (Bytes) /
/// all-nullary enum. The disc conventions match [`fixed_shape_option_scalar_arg`]: Option `Some` = boundary
/// disc 1, Result `Ok` = boundary disc 0; an enum arm assumes the guest declares the cases in WIT order (the
/// boundary disc IS the decl disc). `None` for any other shape (a compound/general-variant payload, a
/// >2-variant sum, a differing per-position join width) — a later slice.
#[allow(clippy::type_complexity)]
pub(super) fn fixed_shape_sum_param_arg(
    db: &mut Db,
    gty: &crate::ty::Ty,
    param_vts: &mut Vec<u8>,
) -> Option<crate::backend::wasm::serialize::FieldRebuild> {
    use crate::backend::wasm::lir::{ValType, valtype_of};
    use crate::backend::wasm::serialize::{FieldRebuild, SumArgArm, SumArgRebuild, SumArmPayload};
    use crate::ty::Ty;
    let Ty::Sum { decl, args, .. } = gty.strip_nominal() else {
        return None;
    };
    let (params, variant_payloads): (Vec<String>, Vec<Vec<crate::ast::StructId>>) = {
        let decl_ref = db.type_decl_by_occ(*decl)?;
        if decl_ref.variants.len() != 2 {
            return None;
        }
        (
            decl_ref.params.clone(),
            decl_ref
                .variants
                .iter()
                .map(|v| v.payloads.clone())
                .collect(),
        )
    };
    // The instantiated payload TYPE of a variant's ONE generic payload (a param `a` → `args[pi]`).
    let payload_ty = |db: &mut Db, occ: crate::ast::StructId| -> Option<Ty> {
        let pname = db
            .ast
            .head_name(occ)
            .or_else(|| db.ast.as_name(occ))?
            .to_string();
        let pi = params.iter().position(|p| *p == pname)?;
        args.get(pi).cloned()
    };
    // Classify one payload type into its arm payload + flattened leaf valtypes. `list<u8>` → Bytes `(ptr,len)`;
    // an all-nullary sum (WIT `enum`) → Enum (one disc); an aliased-width scalar → Scalar; else decline.
    let classify = |db: &mut Db, pty: &Ty| -> Option<(SumArmPayload, Vec<ValType>)> {
        match pty.strip_nominal() {
            Ty::Bytes => Some((SumArmPayload::Bytes, vec![ValType::I32, ValType::I32])),
            Ty::Sum { decl: sd, .. } => {
                let all_nullary = {
                    let dref = db.type_decl_by_occ(*sd)?;
                    dref.variants.iter().all(|v| v.payloads.is_empty())
                };
                if all_nullary {
                    Some((SumArmPayload::Enum, vec![ValType::I32]))
                } else {
                    None // a payload-carrying nested variant — a later slice
                }
            }
            other => {
                let crate::backend::wasm::serialize::FieldRebuild::Scalar { box_op, extend } =
                    scalar_field_rebuild(other)?
                else {
                    return None;
                };
                Some((
                    SumArmPayload::Scalar {
                        box_op,
                        extend,
                        wrap_join: false,
                    },
                    vec![valtype_of(other)?],
                ))
            }
        }
    };
    let counts: Vec<usize> = variant_payloads.iter().map(|p| p.len()).collect();
    let (rebuild, join_vts): (SumArgRebuild, Vec<ValType>) = match counts.as_slice() {
        // OPTION: one nullary + one single-payload variant. `Some` = boundary disc 1.
        [0, 1] | [1, 0] => {
            let (payload_i, nullary_i) = if counts[0] == 1 {
                (0usize, 1)
            } else {
                (1usize, 0)
            };
            let pty = payload_ty(db, variant_payloads[payload_i][0])?;
            let (payload, vts) = classify(db, &pty)?;
            (
                SumArgRebuild {
                    base_param: 0, // IGNORED for a field (the disc is read at the record cursor)
                    boundary_true_disc: 1,
                    arm_true: SumArgArm {
                        decl_disc: payload_i as u32,
                        payload,
                    },
                    arm_false: SumArgArm {
                        decl_disc: nullary_i as u32,
                        payload: SumArmPayload::Nullary,
                    },
                },
                vts,
            )
        }
        // RESULT: two single-payload variants (Ok a, Err b). `Ok` = boundary disc 0. The payload slot is the
        // position-wise JOIN of the two arms' leaves (this slice: same core width per shared position).
        [1, 1] => {
            let ok_pty = payload_ty(db, variant_payloads[0][0])?;
            let err_pty = payload_ty(db, variant_payloads[1][0])?;
            let (ok_payload, ok_vts) = classify(db, &ok_pty)?;
            let (err_payload, err_vts) = classify(db, &err_pty)?;
            let join_len = ok_vts.len().max(err_vts.len());
            let mut joined = Vec::with_capacity(join_len);
            for i in 0..join_len {
                match (ok_vts.get(i), err_vts.get(i)) {
                    (Some(a), Some(b)) if a == b => joined.push(*a),
                    (Some(a), None) => joined.push(*a),
                    (None, Some(b)) => joined.push(*b),
                    _ => return None, // differing per-position width — a later widening
                }
            }
            (
                SumArgRebuild {
                    base_param: 0,
                    boundary_true_disc: 0,
                    arm_true: SumArgArm {
                        decl_disc: 0,
                        payload: ok_payload,
                    },
                    arm_false: SumArgArm {
                        decl_disc: 1,
                        payload: err_payload,
                    },
                },
                joined,
            )
        }
        _ => return None, // two nullary (a bare enum field), or a multi-payload variant — out of scope
    };
    // The canon lift flattens the sum to `(disc: i32, payload-join…)`.
    param_vts.push(ValType::I32.byte());
    param_vts.extend(join_vts.iter().map(|vt| vt.byte()));
    Some(FieldRebuild::Sum(Box::new(rebuild)))
}

/// A FIXED-SHAPE SCALAR tuple/record closure ARGUMENT that crosses the DIRECT-CALL boundary as a native
/// component `tuple<…>` (the canonical ABI flattens it into scalar core params). Returns, for such a `ty`:
/// the per-field component boundary bytes (the envelope's `tuple<…>` type + the flattened core `call`
/// params), the per-field core valtypes, and the [`serialize::TupleArgRebuild`] the core `call` uses to
/// reassemble the cell from the flat fields. `None` if `ty` is not a tuple/record, or ANY field is not a
/// genuine aliased-width scalar (a NESTED compound or a variable-length collection field would need
/// recursive rebuild / runtime decode — out of this increment). A RECORD's fields are taken in the
/// canonical SORTED-key order (the value-heap cell's field order), matching how `Core::Record` lays them.
///
/// The boundary aggregate's layout is a function of the DECLARED TYPE `ty` alone — the per-field bytes and
/// their order come from the type's fields (a tuple's positional elements, a record's sorted-key values),
/// never from the order the compiler discovered or emitted them, so it is deterministic and fixed by the
/// type:
//= spec/contracts/component-abi.md#aggregate-layout-is-determined-by-type
//# The byte layout of an aggregate value that crosses the boundary MUST be determined solely by its declared type.
//= spec/contracts/component-abi.md#aggregate-layout-is-determined-by-type
//# The byte layout of an aggregate value that crosses the boundary MUST NOT depend on the order in which the compiler discovered or emitted its fields.
pub(super) fn fixed_shape_scalar_tuple_arg(
    ty: &crate::ty::Ty,
) -> Option<(
    Vec<u8>,
    Vec<crate::backend::wasm::lir::ValType>,
    crate::backend::wasm::serialize::TupleArgRebuild,
)> {
    use crate::ty::Ty;
    // The field types in cell order: a tuple's positional elements, or a record's sorted-key values.
    let fields: Vec<Ty> = match ty.strip_nominal() {
        Ty::Tuple(elems) => elems.iter().cloned().collect(),
        Ty::Record(map) => map.values().cloned().collect(), // BTreeMap → sorted-key order
        _ => return None,
    };
    if fields.is_empty() {
        return None; // a 0-field tuple/record has no host-constructible flattened form here
    }
    let mut comp_bytes = Vec::new();
    let mut core_vts = Vec::new();
    let mut rebuild_fields = Vec::new();
    for f in &fields {
        // Each field must be a genuine aliased-width scalar (Int/Bool/Float) — the only shapes the canonical
        // ABI flattens AND the cell rebuild boxes with a single op. A nested compound / collection → None.
        let cb = closure_boundary_byte(f)?;
        let vt = crate::backend::wasm::lir::valtype_of(f)?;
        rebuild_fields.push(scalar_field_rebuild(f)?);
        comp_bytes.push(cb);
        core_vts.push(vt);
    }
    Some((
        comp_bytes,
        core_vts,
        crate::backend::wasm::serialize::TupleArgRebuild {
            fields: rebuild_fields,
            base_param: 1, // the tuple is the SOLE closure arg → its leaves start at core param 1 (after self)
        },
    ))
}

/// A NESTED fixed-shape compound closure ARGUMENT (a tuple/record whose fields may THEMSELVES be fixed-shape
/// tuples/records, recursively — all leaves aliased-width scalars). Returns the DEPTH-FIRST flattened leaf
/// component bytes + leaf core valtypes (the canonical ABI flattens a nested `tuple<…, tuple<…>>` to its leaf
/// scalars), the recursive [`serialize::FieldRebuild`] tree the core `call` rebuilds the nested cell from, and
/// the [`envelope::TupleFieldShape`] tree the envelope mints the nested `tuple<…>` types from. `None` if `ty`
/// is not a tuple/record, is empty, or any LEAF is not an aliased-width scalar. Companion to
/// [`fixed_shape_scalar_tuple_arg`] (the all-scalar-fields case): a shape of only `Scalar` fields is that same
/// flat case, so callers use this and check whether any field is `Nested`.
#[allow(clippy::type_complexity)]
pub(super) fn nested_fixed_shape_tuple_arg(
    ty: &crate::ty::Ty,
) -> Option<(
    Vec<u8>,
    Vec<crate::backend::wasm::lir::ValType>,
    Vec<crate::backend::wasm::serialize::FieldRebuild>,
    Vec<crate::backend::wasm::envelope::TupleFieldShape>,
)> {
    use crate::backend::wasm::envelope::TupleFieldShape;
    use crate::backend::wasm::serialize::FieldRebuild;
    use crate::ty::Ty;
    let fields: Vec<Ty> = match ty.strip_nominal() {
        Ty::Tuple(elems) => elems.iter().cloned().collect(),
        Ty::Record(map) => map.values().cloned().collect(), // sorted-key order
        _ => return None,
    };
    if fields.is_empty() {
        return None;
    }
    let mut leaf_bytes = Vec::new();
    let mut leaf_vts = Vec::new();
    let mut rebuild_fields = Vec::new();
    let mut shape_fields = Vec::new();
    for f in &fields {
        if let Some(sfr) = scalar_field_rebuild(f) {
            // A scalar leaf: one flattened core param.
            let cb = closure_boundary_byte(f)?;
            let vt = crate::backend::wasm::lir::valtype_of(f)?;
            leaf_bytes.push(cb);
            leaf_vts.push(vt);
            rebuild_fields.push(sfr);
            shape_fields.push(TupleFieldShape::Scalar(cb));
        } else if let Some((sub_bytes, sub_vts, sub_rebuild, sub_shape)) =
            nested_fixed_shape_tuple_arg(f)
        {
            // A nested fixed-shape compound field: its leaves flatten into the SAME depth-first sequence.
            // A guest-constructed tuple/record — its slots are the construction order (identity).
            leaf_bytes.extend(sub_bytes);
            leaf_vts.extend(sub_vts);
            let ident: Vec<u32> = (0..sub_rebuild.len() as u32).collect();
            rebuild_fields.push(FieldRebuild::Nested(sub_rebuild, ident));
            shape_fields.push(TupleFieldShape::Nested(sub_shape));
        } else {
            return None; // a field that is neither an aliased-width scalar nor a fixed-shape compound
        }
    }
    Some((leaf_bytes, leaf_vts, rebuild_fields, shape_fields))
}

/// The [`serialize::FieldRebuild::Scalar`] for one aliased-width scalar field (Int/Bool/Float, peeling a
/// nominal): the box op + whether a NARROW int needs an i32→i64 extend before `box-int`. `None` if `f` is not
/// an aliased-width scalar. Mirrors `select::box_op_ty` + `emit_box_i32_to_i64_extend`.
pub(super) fn scalar_field_rebuild(
    f: &crate::ty::Ty,
) -> Option<crate::backend::wasm::serialize::FieldRebuild> {
    use crate::backend::wasm::serialize::FieldRebuild;
    use crate::ty::Ty;
    let (box_op, extend) = match f.strip_nominal() {
        Ty::Int(it) => {
            let signed = it.ground_signed();
            // Extend only when the field sits in an I32 SLOT (ground width ≤ 32 = `int_valtype` I32 =
            // `is_narrow_int`'s `slot32`), NOT `< 64`: a MID-WIDTH int (33..63) is already an i64 slot, so
            // an `i64.extend_i32_*` on it would be invalid wasm — the same slot-vs-`<64` gate the Qty
            // scalar-box takes. (Latent here: a mid-width scalar FIELD has no boundary representation, so a
            // compound param carrying one is rejected before this rebuild; kept in lockstep with the
            // canonical `is_narrow_int` truth defensively.)
            let extend = if it.ground_width() <= 32 {
                Some(signed)
            } else {
                None
            };
            ("box-int", extend)
        }
        Ty::Bool => ("box-bool", None),
        Ty::Float(ft) if ft.ground_width() == 64 => ("box-float", None),
        Ty::Float(ft) if ft.ground_width() == 32 => ("box-float32", None),
        _ => return None,
    };
    Some(FieldRebuild::Scalar { box_op, extend })
}

/// The per-FIELD ABI for ONE fixed-shape scalar tuple/record type (the data
/// [`fixed_shape_scalar_tuple_arg`] builds, minus the `base_param`). Returns `(comp_bytes, core_vts,
/// rebuild_fields)` or `None` if `ty` is not a tuple/record of aliased-width scalars.
#[allow(clippy::type_complexity)]
pub(super) fn tuple_field_abi(
    ty: &crate::ty::Ty,
) -> Option<(
    Vec<u8>,
    Vec<crate::backend::wasm::lir::ValType>,
    Vec<crate::backend::wasm::serialize::FieldRebuild>,
)> {
    use crate::ty::Ty;
    let fields: Vec<Ty> = match ty.strip_nominal() {
        Ty::Tuple(elems) => elems.iter().cloned().collect(),
        Ty::Record(map) => map.values().cloned().collect(),
        _ => return None,
    };
    if fields.is_empty() {
        return None;
    }
    let mut comp_bytes = Vec::new();
    let mut core_vts = Vec::new();
    let mut rebuild_fields = Vec::new();
    for f in &fields {
        let cb = closure_boundary_byte(f)?;
        let vt = crate::backend::wasm::lir::valtype_of(f)?;
        rebuild_fields.push(scalar_field_rebuild(f)?);
        comp_bytes.push(cb);
        core_vts.push(vt);
    }
    Some((comp_bytes, core_vts, rebuild_fields))
}

/// The flattened `call`-boundary description for a compound closure argument on the direct-call path:
/// `(tuple field component bytes, full flattened arg core valtypes, prefix scalar bytes, suffix scalar
/// bytes, the rebuild)`. Named so the producer functions and the `tuple_arg` binding share one type
/// (clippy's `type_complexity`) rather than repeating the 5-tuple.
pub(super) type CompoundArgBoundary = (
    Vec<u8>,
    Vec<crate::backend::wasm::lir::ValType>,
    Vec<u8>,
    Vec<u8>,
    crate::backend::wasm::serialize::TupleArgRebuild,
);

/// The boundary description for a fixed-shape compound closure argument with a NESTED compound field (the
/// direct-call nested-compound path): `(depth-first flattened leaf component bytes, FULL flattened core
/// valtypes incl. any prefix/suffix scalars, the recursive rebuild with `base_param`, the recursive envelope
/// type shape, prefix scalar bytes, suffix scalar bytes)`. The envelope mints the inner `tuple<…>` types from
/// the `TupleFieldShape` tree; the core rebuilds the nested cell from the `TupleArgRebuild`. For the SOLE-arg
/// case prefix/suffix are empty + `leaf_bytes` == the flattened leaves; for the AMONG-SCALARS case the vts
/// include the prefix/suffix scalars and `base_param` is shifted past the prefix.
pub(super) type NestedCompoundArgBoundary = (
    Vec<u8>,
    Vec<crate::backend::wasm::lir::ValType>,
    crate::backend::wasm::serialize::TupleArgRebuild,
    Vec<crate::backend::wasm::envelope::TupleFieldShape>,
    Vec<u8>,
    Vec<u8>,
);

/// A per-GROUP direct-call compound-arg boundary (distinct-sig): the tuple's per-field component bytes +
/// prefix scalar bytes + suffix scalar bytes + the `TupleArgRebuild`. Like [`CompoundArgBoundary`] but without
/// the full flattened core vts (a group carries those in `arg_vts` directly).
pub(super) type GroupCompoundArg = (
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    crate::backend::wasm::serialize::TupleArgRebuild,
);

/// EXACTLY ONE fixed-shape scalar tuple/record argument AMONG scalar args (the compound-arg-alongside-scalars
/// direct-call path). Given the closure's `arg_tys`, if precisely one is a fixed-shape scalar tuple/record and
/// every OTHER arg is an aliased-width scalar, returns the flattened `call` boundary (prefix scalar bytes +
/// vts, the tuple's field bytes + vts + `TupleArgRebuild` with `base_param`, suffix scalar bytes + vts). The
/// core `call` receives `[prefix scalars, tuple fields, suffix scalars]` flattened; the body pushes prefix
/// scalars, rebuilds the tuple, pushes suffix scalars. `None` if not exactly one tuple among scalars.
pub(super) fn single_compound_among_scalars(
    arg_tys: &[crate::ty::Ty],
) -> Option<CompoundArgBoundary> {
    // Find the single compound (tuple/record) arg; every other arg must be an aliased-width scalar.
    let tuple_positions: Vec<usize> = arg_tys
        .iter()
        .enumerate()
        .filter(|(_, t)| tuple_field_abi(t).is_some())
        .map(|(i, _)| i)
        .collect();
    if tuple_positions.len() != 1 || arg_tys.len() < 2 {
        return None; // zero or >1 tuple, or the sole-tuple case (handled by `fixed_shape_scalar_tuple_arg`)
    }
    let tpos = tuple_positions[0];
    let (field_bytes, field_vts, rebuild_fields) = tuple_field_abi(&arg_tys[tpos])?;
    let mut prefix_bytes = Vec::new();
    let mut suffix_bytes = Vec::new();
    let mut all_vts = Vec::new(); // flattened core call params: prefix scalars, then tuple fields, then suffix
    for (i, t) in arg_tys.iter().enumerate() {
        if i == tpos {
            all_vts.extend_from_slice(&field_vts);
            continue;
        }
        // Every non-tuple arg must be an aliased-width scalar.
        let cb = closure_boundary_byte(t)?;
        let vt = crate::backend::wasm::lir::valtype_of(t)?;
        if i < tpos {
            prefix_bytes.push(cb);
        } else {
            suffix_bytes.push(cb);
        }
        all_vts.push(vt);
    }
    // The tuple's leaves start at core param `1 + prefix.len()` (after `self`=0 + the prefix scalars).
    let base_param = 1 + prefix_bytes.len() as u32;
    Some((
        field_bytes, // the tuple's OWN field bytes → the `tuple<…>` defined type
        all_vts,
        prefix_bytes,
        suffix_bytes,
        crate::backend::wasm::serialize::TupleArgRebuild {
            fields: rebuild_fields,
            base_param,
        },
    ))
}

/// The boundary description for a NESTED fixed-shape compound arg AMONG scalar args: like
/// [`NestedCompoundArgBoundary`] but with the interleaving prefix/suffix scalar bytes (the nested tuple sits
/// at its own position among aliased-width scalars). `(full flattened core vts, prefix scalar bytes, suffix
/// scalar bytes, the recursive rebuild with shifted `base_param`, the recursive `TupleFieldShape`)`.
pub(super) type NestedAmongScalarsBoundary = (
    Vec<crate::backend::wasm::lir::ValType>,
    Vec<u8>,
    Vec<u8>,
    crate::backend::wasm::serialize::TupleArgRebuild,
    Vec<crate::backend::wasm::envelope::TupleFieldShape>,
);

/// EXACTLY ONE NESTED fixed-shape compound argument AMONG scalar args (the nested-compound-alongside-scalars
/// direct-call path). Like [`single_compound_among_scalars`] but the sole compound has a NESTED compound field
/// (so its leaves flatten recursively + the envelope mints inner `tuple<…>` types). The compound must have a
/// nested field (else `single_compound_among_scalars` handles the flat case); every other arg an aliased-width
/// scalar. Returns the full flattened core vts (prefix scalars, then the compound's depth-first leaves, then
/// suffix scalars), the prefix/suffix boundary bytes, the recursive rebuild (with `base_param` past the
/// prefix), and the recursive shape. `None` if not exactly one compound (with a nested field) among scalars.
pub(super) fn nested_compound_among_scalars(
    arg_tys: &[crate::ty::Ty],
) -> Option<NestedAmongScalarsBoundary> {
    let compound_positions: Vec<usize> = arg_tys
        .iter()
        .enumerate()
        .filter(|(_, t)| nested_fixed_shape_tuple_arg(t).is_some())
        .map(|(i, _)| i)
        .collect();
    if compound_positions.len() != 1 || arg_tys.len() < 2 {
        return None; // zero or >1 compound, or the sole case (handled by the sole nested path)
    }
    let cpos = compound_positions[0];
    let (_leaf_bytes, leaf_vts, rebuild_fields, shape) =
        nested_fixed_shape_tuple_arg(&arg_tys[cpos])?;
    // Only take this path when the compound genuinely has a NESTED field (else it is a flat tuple among
    // scalars — handled by `single_compound_among_scalars`).
    let has_nested = shape.iter().any(|f| {
        matches!(
            f,
            crate::backend::wasm::envelope::TupleFieldShape::Nested(_)
        )
    });
    if !has_nested {
        return None;
    }
    let mut prefix_bytes = Vec::new();
    let mut suffix_bytes = Vec::new();
    let mut all_vts = Vec::new(); // prefix scalars, then the compound's depth-first leaves, then suffix scalars
    for (i, t) in arg_tys.iter().enumerate() {
        if i == cpos {
            all_vts.extend_from_slice(&leaf_vts);
            continue;
        }
        let cb = closure_boundary_byte(t)?;
        let vt = crate::backend::wasm::lir::valtype_of(t)?;
        if i < cpos {
            prefix_bytes.push(cb);
        } else {
            suffix_bytes.push(cb);
        }
        all_vts.push(vt);
    }
    let base_param = 1 + prefix_bytes.len() as u32;
    Some((
        all_vts,
        prefix_bytes,
        suffix_bytes,
        crate::backend::wasm::serialize::TupleArgRebuild {
            fields: rebuild_fields,
            base_param,
        },
        shape,
    ))
}

/// A fixed-shape compound closure argument with a NESTED compound field — the SOLE arg OR among aliased-width
/// scalars — as a [`NestedCompoundArgBoundary`]. The SOLE case (arity 1) has empty prefix/suffix + `base_param`
/// 1 and `leaf_bytes` == the flattened leaves; the AMONG-SCALARS case (arity > 1) carries prefix/suffix + a
/// shifted `base_param` (and empty `leaf_bytes` — the shape drives the mint). `None` unless exactly one
/// compound (with a nested field) among scalars. Shared by every closure emit path's `nested_tuple` binding.
pub(super) fn nested_sole_or_among_scalars(
    arg_tys: &[crate::ty::Ty],
) -> Option<NestedCompoundArgBoundary> {
    if arg_tys.len() == 1 {
        let (lb, lv, rf, shape) = nested_fixed_shape_tuple_arg(&arg_tys[0])?;
        // Only this path when there IS a nested field (else the all-scalar case is a flat `tuple_arg`).
        let has_nested = shape.iter().any(|f| {
            matches!(
                f,
                crate::backend::wasm::envelope::TupleFieldShape::Nested(_)
            )
        });
        has_nested.then_some((
            lb,
            lv,
            crate::backend::wasm::serialize::TupleArgRebuild {
                fields: rf,
                base_param: 1,
            },
            shape,
            Vec::new(),
            Vec::new(),
        ))
    } else {
        nested_compound_among_scalars(arg_tys)
            .map(|(all_vts, pre, suf, rb, shape)| (Vec::new(), all_vts, rb, shape, pre, suf))
    }
}

/// The boundary description for TWO OR MORE fixed-shape tuple/record args (the N-compound-args direct-call
/// path): `(the ordered `ArgSlot` list, the FULL flattened core valtypes, one `TupleArgRebuild` per tuple slot
/// in arg order)`. Each arg is a scalar (crossing as its primitive byte) or a fixed-shape tuple/record (its
/// leaves flattened by the canonical ABI, rebuilt in-guest from the `TupleArgRebuild` at its `base_param`).
/// `None` unless there are ≥2 tuple args (the ≤1-tuple cases are the existing `fixed_shape_scalar_tuple_arg` /
/// `single_compound_among_scalars` / nested paths, kept byte-identical). Every leaf must be an aliased-width
/// scalar (a nested fixed-shape compound field is allowed — it flattens recursively). `base_param` counts from
/// 1 (after `self`) across the flattened leaves of every preceding arg.
#[allow(clippy::type_complexity)]
pub(super) fn multi_compound_args(
    arg_tys: &[crate::ty::Ty],
) -> Option<(
    Vec<crate::backend::wasm::envelope::ArgSlot>,
    Vec<crate::backend::wasm::lir::ValType>,
    Vec<crate::backend::wasm::serialize::TupleArgRebuild>,
)> {
    use crate::backend::wasm::envelope::ArgSlot;
    // Require ≥2 tuple/record args; fewer is handled by the sole / among-scalars single-tuple classifiers.
    let n_tuples = arg_tys
        .iter()
        .filter(|t| nested_fixed_shape_tuple_arg(t).is_some())
        .count();
    if n_tuples < 2 {
        return None;
    }
    let mut slots = Vec::with_capacity(arg_tys.len());
    let mut all_vts = Vec::new();
    let mut rebuilds = Vec::new();
    let mut next_param: u32 = 1; // core param 0 is `self`; the flattened leaves start at 1
    for t in arg_tys {
        if let Some((_leaf_bytes, leaf_vts, rebuild_fields, shape)) =
            nested_fixed_shape_tuple_arg(t)
        {
            let base_param = next_param;
            next_param += leaf_vts.len() as u32;
            all_vts.extend(leaf_vts);
            slots.push(ArgSlot::Tuple(shape));
            rebuilds.push(crate::backend::wasm::serialize::TupleArgRebuild {
                fields: rebuild_fields,
                base_param,
            });
        } else if let Some(b) = closure_boundary_byte(t) {
            // A plain aliased-width scalar arg interleaved among the tuples.
            let vt = crate::backend::wasm::lir::valtype_of(t)?;
            next_param += 1;
            all_vts.push(vt);
            slots.push(ArgSlot::Scalar(b));
        } else {
            return None; // an arg that is neither a fixed-shape compound nor an aliased-width scalar
        }
    }
    Some((slots, all_vts, rebuilds))
}
