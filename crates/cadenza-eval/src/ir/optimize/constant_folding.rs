//! Constant folding optimization pass.
//!
//! This pass evaluates operations on constant values at compile time,
//! replacing them with their computed results.

use super::{OptimizationPass, types::*};
use std::collections::HashMap;

/// Constant folding optimization pass.
///
/// This pass evaluates operations on constant values at compile time,
/// replacing them with their computed results.
pub struct ConstantFoldingPass;

impl OptimizationPass for ConstantFoldingPass {
    fn run(&mut self, module: &mut IrModule) -> bool {
        let mut changed = false;

        for func in &mut module.functions {
            changed |= fold_constants_in_function(func);
        }

        changed
    }

    fn name(&self) -> &str {
        "constant_folding"
    }
}

/// Fold constants in a function.
fn fold_constants_in_function(func: &mut IrFunction) -> bool {
    let mut changed = false;

    // Build a map of constant values
    let mut const_values: HashMap<ValueId, IrConst> = HashMap::new();

    for block in &mut func.blocks {
        for instr in &mut block.instructions {
            match instr {
                IrInstr::Const { result, value, .. } => {
                    const_values.insert(*result, value.clone());
                }
                IrInstr::BinOp {
                    result,
                    ty,
                    op,
                    lhs,
                    rhs,
                    source,
                } => {
                    // Copy values we need before modifying
                    let result_id = *result;
                    let ty_copy = ty.clone();
                    let op_copy = *op;
                    let source_copy = *source;

                    // Try to fold binary operations with constant operands
                    if let (Some(lhs_const), Some(rhs_const)) =
                        (const_values.get(lhs), const_values.get(rhs))
                        && let Some(folded) = fold_binop(op_copy, lhs_const, rhs_const)
                    {
                        // Replace this instruction with a const
                        *instr = IrInstr::Const {
                            result: result_id,
                            ty: ty_copy,
                            value: folded.clone(),
                            source: source_copy,
                        };
                        const_values.insert(result_id, folded);
                        changed = true;
                    }
                }
                IrInstr::UnOp {
                    result,
                    ty,
                    op,
                    operand,
                    source,
                } => {
                    // Copy values we need before modifying
                    let result_id = *result;
                    let ty_copy = ty.clone();
                    let op_copy = *op;
                    let source_copy = *source;

                    // Try to fold unary operations with constant operands
                    if let Some(operand_const) = const_values.get(operand)
                        && let Some(folded) = fold_unop(op_copy, operand_const)
                    {
                        *instr = IrInstr::Const {
                            result: result_id,
                            ty: ty_copy,
                            value: folded.clone(),
                            source: source_copy,
                        };
                        const_values.insert(result_id, folded);
                        changed = true;
                    }
                }
                _ => {}
            }
        }
    }

    changed
}

/// Attempt to fold a binary operation on two constants.
fn fold_binop(op: BinOp, lhs: &IrConst, rhs: &IrConst) -> Option<IrConst> {
    match (lhs, rhs) {
        // Integer arithmetic
        (IrConst::Integer(a), IrConst::Integer(b)) => match op {
            BinOp::Add => Some(IrConst::Integer(a.wrapping_add(*b))),
            BinOp::Sub => Some(IrConst::Integer(a.wrapping_sub(*b))),
            BinOp::Mul => Some(IrConst::Integer(a.wrapping_mul(*b))),
            BinOp::Div if *b != 0 => Some(IrConst::Integer(a / b)),
            BinOp::Rem if *b != 0 => Some(IrConst::Integer(a % b)),
            BinOp::Eq => Some(IrConst::Bool(a == b)),
            BinOp::Ne => Some(IrConst::Bool(a != b)),
            BinOp::Lt => Some(IrConst::Bool(a < b)),
            BinOp::Le => Some(IrConst::Bool(a <= b)),
            BinOp::Gt => Some(IrConst::Bool(a > b)),
            BinOp::Ge => Some(IrConst::Bool(a >= b)),
            BinOp::BitAnd => Some(IrConst::Integer(a & b)),
            BinOp::BitOr => Some(IrConst::Integer(a | b)),
            BinOp::BitXor => Some(IrConst::Integer(a ^ b)),
            BinOp::Shl if *b >= 0 && *b < 64 => Some(IrConst::Integer(a << b)),
            BinOp::Shr if *b >= 0 && *b < 64 => Some(IrConst::Integer(a >> b)),
            _ => None,
        },
        // Float arithmetic
        (IrConst::Float(a), IrConst::Float(b)) => match op {
            BinOp::Add => Some(IrConst::Float(a + b)),
            BinOp::Sub => Some(IrConst::Float(a - b)),
            BinOp::Mul => Some(IrConst::Float(a * b)),
            BinOp::Div => Some(IrConst::Float(a / b)),
            BinOp::Eq => Some(IrConst::Bool(a == b)),
            BinOp::Ne => Some(IrConst::Bool(a != b)),
            BinOp::Lt => Some(IrConst::Bool(a < b)),
            BinOp::Le => Some(IrConst::Bool(a <= b)),
            BinOp::Gt => Some(IrConst::Bool(a > b)),
            BinOp::Ge => Some(IrConst::Bool(a >= b)),
            _ => None,
        },
        // Boolean logic
        (IrConst::Bool(a), IrConst::Bool(b)) => match op {
            BinOp::And => Some(IrConst::Bool(*a && *b)),
            BinOp::Or => Some(IrConst::Bool(*a || *b)),
            BinOp::Eq => Some(IrConst::Bool(a == b)),
            BinOp::Ne => Some(IrConst::Bool(a != b)),
            _ => None,
        },
        _ => None,
    }
}

/// Attempt to fold a unary operation on a constant.
fn fold_unop(op: UnOp, operand: &IrConst) -> Option<IrConst> {
    match operand {
        IrConst::Integer(n) => match op {
            UnOp::Neg => Some(IrConst::Integer(-n)),
            UnOp::BitNot => Some(IrConst::Integer(!n)),
            _ => None,
        },
        IrConst::Float(f) => match op {
            UnOp::Neg => Some(IrConst::Float(-f)),
            _ => None,
        },
        IrConst::Bool(b) => match op {
            UnOp::Not => Some(IrConst::Bool(!b)),
            _ => None,
        },
        _ => None,
    }
}
