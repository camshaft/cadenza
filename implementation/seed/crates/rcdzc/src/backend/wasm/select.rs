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
use crate::backend::wasm::lir::{valtype_of, BlockType, Lir, ValType};
use crate::core::Core;
use crate::db::Db;
use crate::diag::{Code, Reject};
use crate::infer::type_of;
use crate::lower::core_of;
use crate::ty::{IntTy, Ty};

/// A selected function body: its flat instruction sequence, the value types of its declared (non-
/// parameter) locals in slot order, its parameter value types, and its solved return type (for the
/// type section). Stage 0 bodies are nullary with no locals.
pub struct SelectedFunc {
    pub params: Vec<ValType>,
    pub ret: Ty,
    pub code: Vec<Lir>,
    pub declared: Vec<ValType>,
}

/// Select one definition body (rooted at AST occurrence `body`) into its flat instruction sequence.
/// The return type is the body's solved type. Reads the core + type columns lazily.
pub fn select_body(db: &mut Db, body: StructId) -> Result<SelectedFunc, Reject> {
    let ret = type_of(db, body);
    let mut code = Vec::new();
    emit(db, body, &mut code)?;
    Ok(SelectedFunc { params: Vec::new(), ret, code, declared: Vec::new() })
}

/// Emit the flat instructions for the node at `id`, appending to `out`. Exhaustive over `Core`.
fn emit(db: &mut Db, id: StructId, out: &mut Vec<Lir>) -> Result<(), Reject> {
    match core_of(db, id) {
        Core::ConstInt(v) => {
            // Ground the literal to the machine width its solved type fixes, and decline if it does
            // not fit that width (a compile-provable width violation — never a truncated value).
            let it = int_ty_of(db, id);
            let n = v.to_i64().ok_or_else(|| {
                Reject::coded(Code::IntOutOfRange, "integer literal does not fit its width")
            })?;
            if it.ground_width() <= 32 {
                // A ≤32-bit integer occupies an i32 slot. Range-check against the width later; Stage 0
                // only produces i64, so this arm is structural until the widths stage exercises it.
                let narrowed = i32::try_from(n).map_err(|_| {
                    Reject::coded(Code::IntOutOfRange, "integer literal does not fit its 32-bit width")
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
        Core::Record { .. } => {
            Err(Reject::decline("constructing a record at run time needs the value heap (not yet built)"))
        }
        Core::If { cond, then_, else_ } => {
            // Selection order matches wasm's structured `if`: push the condition, open the block with
            // the RESULT type (read off the node's solved type), then the two arms.
            emit(db, cond, out)?;
            let block_ty = match type_of(db, id) {
                Ty::Unit => BlockType::Empty,
                other => match valtype_of(&other) {
                    Some(vt) => BlockType::Val(vt),
                    None => {
                        return Err(Reject::decline("if result type has no machine representation"));
                    }
                },
            };
            out.push(Lir::If(block_ty));
            emit(db, then_, out)?;
            out.push(Lir::Else);
            emit(db, else_, out)?;
            out.push(Lir::End);
            Ok(())
        }
        // A runtime arithmetic op: emit both operands, then the machine op selected from the solved
        // width (i32 for a ≤32-bit integer, else i64). The width is a READ-OFF of the node's solved
        // type — the same read-off that boxes a scalar (`reference-compiler.md` §A Value's Machine
        // Representation Follows Its Solved Type At Selection). (Const arithmetic folds in `lower`;
        // this path is for a genuine runtime operand, which arrives with functions.)
        Core::Arith { op, lhs, rhs } => {
            emit(db, lhs, out)?;
            emit(db, rhs, out)?;
            let narrow = matches!(type_of(db, id), Ty::Int(it) if it.ground_width() <= 32);
            out.push(arith_op(op, narrow));
            Ok(())
        }
        // A poison that reached selection is an unconditionally-reached fault; the poison collector
        // surfaces it before emission, so reaching here is a decline rather than emitted code.
        Core::Poison(reject) => Err(reject),
    }
}

/// The flat wasm op for an arithmetic operation at the given width (i32 if `narrow`, else i64).
fn arith_op(op: crate::resolved::Prim, narrow: bool) -> Lir {
    use crate::resolved::Prim::*;
    match (op, narrow) {
        (Add, false) => Lir::I64Add,
        (Sub, false) => Lir::I64Sub,
        (Mul, false) => Lir::I64Mul,
        (Add, true) => Lir::I32Add,
        (Sub, true) => Lir::I32Sub,
        (Mul, true) => Lir::I32Mul,
        // A `Core::Arith` only ever carries an arith prim (its op comes from an arith-gated lowering),
        // so a non-arith prim reaching arith selection is unreachable.
        (IntCtor | UIntCtor | FnCtor | BoolTy | UnitTy, _) => {
            unreachable!("non-arith prim reached arith selection")
        }
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
}
