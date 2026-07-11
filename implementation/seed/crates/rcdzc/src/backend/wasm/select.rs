//! `select` — instruction selection for the wasm backend: the core (A-normal, structured) form of a
//! definition body linearized into a flat `Vec<Lir>`.
//!
//! This is the wasm backend's linearization of the core (`backends-and-targets.md` §A Backend
//! Linearizes The Core Only If Its Target Is Linear). It reads a node's core form (via
//! [`crate::lower::core_of`]) and its solved type (via [`crate::infer::type_of`]) — the machine
//! representation is a READ-OFF of the solved type (`reference-compiler.md` §A Value's Machine
//! Representation Follows Its Solved Type At Selection), not a guess from the node's shape. It is
//! where a deferred integer width GROUNDS to its machine width, and where a literal that does not fit
//! its solved width DECLINES rather than emitting a truncated value.
//!
//! A construct the flat rung cannot express declines (`reference-compiler.md` §A Guarded Operation
//! Reserves Bounded Scratch Or Declines). Stage 0's slice — a constant, a two-way `if` — lowers to a
//! constant push and a structured `if`/`else`/`end`, so nothing declines here yet.

use crate::ast::StructId;
use crate::backend::wasm::lir::{BlockType, Lir, ValType, valtype_of};
use crate::core::Core;
use crate::db::Db;
use crate::diag::{Code, Reject};
use crate::infer::type_of;
use crate::lower::core_of;
use crate::resolved::Prim;
use crate::ty::{IntTy, Ty};
use std::collections::HashMap;

/// A selected function body: its flat instruction sequence, the value types of its declared (non-
/// parameter) locals in slot order, its parameter value types, and its solved return type (for the
/// type section). Stage 0 bodies are nullary with no locals.
pub struct SelectedFunc {
    pub params: Vec<ValType>,
    pub ret: Ty,
    pub code: Vec<Lir>,
    pub declared: Vec<ValType>,
}

/// Select one NULLARY definition body (rooted at AST occurrence `body`) into its flat instruction
/// sequence. The return type is the body's solved type. Reads the core + type columns lazily.
pub fn select_body(db: &mut Db, body: StructId) -> Result<SelectedFunc, Reject> {
    select_function(db, body, &[])
}

/// Select a function body with `params` — each a `(name-occurrence, solved-type)`, in signature order.
/// The parameters occupy wasm local slots `0..n` in order; a `Core::Param` reference to a parameter
/// emits `local.get <slot>`. The return type is the body's solved type. A parameter whose type has no
/// machine representation (an unresolved/compound type) DECLINES here — an exported parameter needs a
/// definite scalar type (which an annotation supplies).
pub fn select_function(
    db: &mut Db,
    body: StructId,
    params: &[(StructId, Ty)],
) -> Result<SelectedFunc, Reject> {
    // Assign each parameter a local slot in order, and its wasm value type (its machine rep).
    let mut slot_of: HashMap<StructId, u32> = HashMap::new();
    let mut param_vts: Vec<ValType> = Vec::new();
    for (i, (binder, ty)) in params.iter().enumerate() {
        let vt = valtype_of(ty).ok_or_else(|| {
            Reject::decline("a function parameter's type has no machine representation")
        })?;
        slot_of.insert(*binder, i as u32);
        param_vts.push(vt);
    }
    let ret = type_of(db, body);
    let mut code = Vec::new();
    emit(db, body, &slot_of, &mut code)?;
    Ok(SelectedFunc {
        params: param_vts,
        ret,
        code,
        declared: Vec::new(),
    })
}

/// Emit the flat instructions for the node at `id`, appending to `out`. `slots` maps a parameter's
/// name occurrence to its wasm local slot. Exhaustive over `Core`.
fn emit(
    db: &mut Db,
    id: StructId,
    slots: &HashMap<StructId, u32>,
    out: &mut Vec<Lir>,
) -> Result<(), Reject> {
    match core_of(db, id) {
        Core::ConstInt(v) => {
            // Ground the literal to the machine width its solved type fixes, and decline if it does
            // not fit that width (a compile-provable width violation — never a truncated value).
            let it = int_ty_of(db, id);
            let n = v.to_i64().ok_or_else(|| {
                Reject::coded(
                    Code::IntOutOfRange,
                    "integer literal does not fit its width",
                )
            })?;
            if it.ground_width() <= 32 {
                // A ≤32-bit integer occupies an i32 slot. Range-check against the width later; Stage 0
                // only produces i64, so this arm is structural until the widths stage exercises it.
                let narrowed = i32::try_from(n).map_err(|_| {
                    Reject::coded(
                        Code::IntOutOfRange,
                        "integer literal does not fit its 32-bit width",
                    )
                })?;
                out.push(Lir::ConstI32(narrowed));
            } else {
                out.push(Lir::ConstI64(n));
            }
            Ok(())
        }
        Core::ConstBool(b) => {
            out.push(Lir::ConstI32(if b { 1 } else { 0 }));
            Ok(())
        }
        Core::Unit => {
            // Unit occupies no slot and pushes nothing.
            Ok(())
        }
        // A record that SURVIVED folding to selection is used as a runtime value (not just to read a
        // field, which folds away). Constructing a compound at run time needs the value-heap runtime,
        // a later stage — so decline cleanly rather than emit a plausible-but-wrong sequence.
        Core::Record { .. } => Err(Reject::decline(
            "constructing a record at run time needs the value heap (not yet built)",
        )),
        Core::If { cond, then_, else_ } => {
            // Selection order matches wasm's structured `if`: push the condition, open the block with
            // the RESULT type (read off the node's solved type), then the two arms.
            emit(db, cond, slots, out)?;
            let block_ty = match type_of(db, id) {
                Ty::Unit => BlockType::Empty,
                other => match valtype_of(&other) {
                    Some(vt) => BlockType::Val(vt),
                    None => {
                        return Err(Reject::decline(
                            "if result type has no machine representation",
                        ));
                    }
                },
            };
            out.push(Lir::If(block_ty));
            emit(db, then_, slots, out)?;
            out.push(Lir::Else);
            emit(db, else_, slots, out)?;
            out.push(Lir::End);
            Ok(())
        }
        // A parameter reference — read its local slot. The slot was assigned in `select_function`; a
        // reference to a binder with no slot is a compiler bug (a param not in the signature), so
        // decline rather than emit a wrong `local.get`.
        Core::Param { binder } => match slots.get(&binder) {
            Some(&slot) => {
                out.push(Lir::LocalGet(slot));
                Ok(())
            }
            None => Err(Reject::decline("parameter reference has no local slot")),
        },
        // A runtime comparison: emit both operands, then the machine comparison selected from the
        // operands' width/signedness. The result is an i32 boolean.
        Core::Compare { op, lhs, rhs } => {
            emit(db, lhs, slots, out)?;
            emit(db, rhs, slots, out)?;
            let it = operand_int_ty(db, lhs, rhs);
            out.push(compare_op(op, it));
            Ok(())
        }
        // A runtime arithmetic op: emit both operands, then the machine op selected from the solved
        // width (i32 for a ≤32-bit integer, else i64). The width is a READ-OFF of the node's solved
        // type — the same read-off that boxes a scalar (`reference-compiler.md` §A Value's Machine
        // Representation Follows Its Solved Type At Selection). (Const arithmetic folds in `lower`;
        // this path is for a genuine runtime operand, which arrives with functions.)
        Core::Arith { op, lhs, rhs } => {
            let narrow = matches!(type_of(db, id), Ty::Int(it) if it.ground_width() <= 32);
            // An op the flat rung can emit today: push both operands, then the machine op. An op with
            // no Lir instruction yet (div/shift/bitwise) DECLINES rather than emit a wrong sequence —
            // the decline boundary the runtime-operands increment widens. (In this increment every
            // integer op FOLDS, so no runtime `Arith` reaches here at all; this keeps the path honest.)
            match arith_op(op, narrow) {
                Some(instr) => {
                    emit(db, lhs, slots, out)?;
                    emit(db, rhs, slots, out)?;
                    out.push(instr);
                    Ok(())
                }
                None => Err(Reject::decline(
                    "a runtime integer operation of this kind is not yet emitted",
                )),
            }
        }
        // A poison that reached selection is an unconditionally-reached fault; the poison collector
        // surfaces it before emission, so reaching here is a decline rather than emitted code.
        Core::Poison(reject) => Err(reject),
    }
}

/// The flat wasm op for a runtime integer operation at the given width (i32 if `narrow`, else i64), or
/// `None` if this backend has no instruction for it yet (the caller declines). Only `+`/`-`/`*` are
/// emittable in this increment; division, shift, and bitwise ops need their Lir instructions + the
/// signedness read-off (a signed vs unsigned shift/divide), which the runtime-operands increment adds.
/// (Const folding handles all of them at compile time already; this is only the RUNTIME path.)
fn arith_op(op: crate::resolved::Prim, narrow: bool) -> Option<Lir> {
    use crate::resolved::Prim::*;
    Some(match (op, narrow) {
        (Add, false) => Lir::I64Add,
        (Sub, false) => Lir::I64Sub,
        (Mul, false) => Lir::I64Mul,
        (Add, true) => Lir::I32Add,
        (Sub, true) => Lir::I32Sub,
        (Mul, true) => Lir::I32Mul,
        // Not yet emittable as a runtime op — decline (the caller turns `None` into a clean decline).
        (Div | Rem | Shl | Shr | BitAnd | BitOr | BitXor, _) => return None,
        // A comparison is never carried by `Core::Arith` (it lowers via `lower_comparison` to a
        // `ConstBool` or declines); a type-constructor / ground-type prim is not an operation. Neither
        // reaches runtime arith selection.
        (Lt | Gt | Le | Ge | Eq | IntCtor | UIntCtor | FnCtor | BoolTy | UnitTy, _) => return None,
    })
}

/// The flat wasm comparison op for a relational prim over an operand integer type — the width chooses
/// i32 (≤32-bit, or a boolean operand) vs i64, and the signedness chooses `_s` vs (later) `_u`.
/// Equality is sign- and (for the same width) representation-agnostic. Width-64 signed integers and
/// booleans are what this increment produces, so only signed + eq variants are needed now.
fn compare_op(op: Prim, it: IntTy) -> Lir {
    let narrow = it.ground_width() <= 32;
    match (op, narrow) {
        (Prim::Eq, false) => Lir::I64Eq,
        (Prim::Lt, false) => Lir::I64LtS,
        (Prim::Gt, false) => Lir::I64GtS,
        (Prim::Le, false) => Lir::I64LeS,
        (Prim::Ge, false) => Lir::I64GeS,
        (Prim::Eq, true) => Lir::I32Eq,
        (Prim::Lt, true) => Lir::I32LtS,
        (Prim::Gt, true) => Lir::I32GtS,
        (Prim::Le, true) => Lir::I32LeS,
        (Prim::Ge, true) => Lir::I32GeS,
        // Not a comparison — `Core::Compare` only ever carries a comparison prim, so unreachable.
        _ => Lir::I64Eq,
    }
}

/// The integer type governing a runtime comparison's operands — read off whichever operand solves to
/// an integer (they unify to one type). A boolean comparison has no integer operand, so it grounds to
/// the ≤32-bit path via the default `i64`… (a bool is compared as an i32 — see `Compare` selection,
/// which reads the operand's own `valtype`). Falls back to signed-64.
fn operand_int_ty(db: &mut Db, lhs: StructId, rhs: StructId) -> IntTy {
    match type_of(db, lhs) {
        Ty::Int(it) => it,
        // A boolean operand is an i32; represent that as a ≤32-bit width so `compare_op` picks i32.
        Ty::Bool => IntTy {
            signed: true,
            width: crate::ty::Width::Fixed(32),
        },
        _ => match type_of(db, rhs) {
            Ty::Int(it) => it,
            Ty::Bool => IntTy {
                signed: true,
                width: crate::ty::Width::Fixed(32),
            },
            _ => IntTy::i64(),
        },
    }
}

/// The integer type of the node at `id`, if its solved type is an integer — used to ground a literal's
/// width at selection. Defaults to the signed-64 instance when the node is not an integer (a
/// defensive fallback; a `ConstInt` node always types as an integer).
fn int_ty_of(db: &mut Db, id: StructId) -> IntTy {
    match type_of(db, id) {
        Ty::Int(it) => it,
        _ => IntTy::i64(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{if_program, scalar_program};

    #[test]
    fn selects_a_literal_to_i64_const() {
        let (ast, body) = scalar_program();
        let mut db = Db::load(ast);
        let f = select_body(&mut db, body).expect("select");
        assert_eq!(f.code, vec![Lir::ConstI64(42)]);
        assert!(f.ret.agrees_with(&Ty::int64()));
    }

    #[test]
    fn selects_an_if_to_a_structured_block() {
        let (ast, if_node) = if_program();
        let mut db = Db::load(ast);
        let f = select_body(&mut db, if_node).expect("select");
        // (if false 1 2) → i32.const 0 ; if (result i64) ; i64.const 1 ; else ; i64.const 2 ; end
        assert_eq!(
            f.code,
            vec![
                Lir::ConstI32(0),
                Lir::If(BlockType::Val(ValType::I64)),
                Lir::ConstI64(1),
                Lir::Else,
                Lir::ConstI64(2),
                Lir::End,
            ]
        );
    }

    // ── runtime lowering: a parameterized function body selects to local reads + machine ops ──────
    //
    // These select a FUNCTION body standalone (as `select_function`, the path an exported function
    // takes) — the parameters are runtime values, so their references become `local.get` and the
    // operation is a runtime machine op, NOT folded. Asserted at the Lir level (no export/run yet).

    /// Locate def `name`'s parameter name-occurrences (seeing through `(: a T)`) and body, plus solve
    /// each param's type — the inputs `select_function` takes for an exported parameterized function.
    fn function_of(db: &mut Db, name: &str) -> (Vec<(StructId, Ty)>, StructId) {
        let d = db.def_by_name(name).expect("def present");
        let sig_params = db.defs[d].params.clone();
        let body = db.defs[d].body.expect("body");
        let mut params = Vec::new();
        for p in sig_params {
            // The name occurrence a reference binds to — bare `a` or the inner name of `(: a T)`.
            let binder = match db.ast.as_form(p, ":").and_then(|t| t.first().copied()) {
                Some(name_occ) => name_occ,
                None => p,
            };
            let ty = type_of(db, binder);
            params.push((binder, ty));
        }
        (params, body)
    }

    #[test]
    fn a_parameterized_addition_selects_to_locals_and_i64_add() {
        // (def (add (: a Int64) (: b Int64)) (+ a b)) — the body is a RUNTIME add over two params, so
        // it selects to `local.get 0 ; local.get 1 ; i64.add` (no fold — the params are unknown).
        let ast = crate::testkit::parse(
            "(module m (def (add (: a Int64) (: b Int64)) (+ a b)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let (params, body) = function_of(&mut db, "add");
        let f = select_function(&mut db, body, &params).expect("select");
        assert_eq!(f.params, vec![ValType::I64, ValType::I64]);
        assert_eq!(
            f.code,
            vec![Lir::LocalGet(0), Lir::LocalGet(1), Lir::I64Add]
        );
        assert!(f.ret.agrees_with(&Ty::int64()));
    }

    #[test]
    fn a_parameterized_comparison_selects_to_a_signed_compare() {
        // (def (lt (: a Int64) (: b Int64)) (< a b)) — a runtime signed comparison, result Bool (i32).
        let ast = crate::testkit::parse(
            "(module m (def (lt (: a Int64) (: b Int64)) (< a b)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let (params, body) = function_of(&mut db, "lt");
        let f = select_function(&mut db, body, &params).expect("select");
        assert_eq!(
            f.code,
            vec![Lir::LocalGet(0), Lir::LocalGet(1), Lir::I64LtS]
        );
        assert_eq!(f.ret, Ty::Bool);
    }

    #[test]
    fn a_parameterized_body_mixing_a_param_and_a_constant() {
        // (def (inc (: a Int64)) (+ a 1)) — `a` is a runtime local, `1` a constant: the add stays
        // runtime (one operand is not constant), so `local.get 0 ; i64.const 1 ; i64.add`.
        let ast = crate::testkit::parse(
            "(module m (def (inc (: a Int64)) (+ a 1)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let (params, body) = function_of(&mut db, "inc");
        let f = select_function(&mut db, body, &params).expect("select");
        assert_eq!(
            f.code,
            vec![Lir::LocalGet(0), Lir::ConstI64(1), Lir::I64Add]
        );
    }
}
