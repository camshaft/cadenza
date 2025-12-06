//! Common subexpression elimination optimization pass.
//!
//! This pass detects and eliminates redundant computations by reusing
//! previously computed values.

use super::{OptimizationPass, types::*};
use crate::InternedString;
use std::collections::HashMap;

/// Common subexpression elimination optimization pass.
///
/// This pass detects and eliminates redundant computations by reusing
/// previously computed values.
pub struct CommonSubexpressionEliminationPass;

impl OptimizationPass for CommonSubexpressionEliminationPass {
    fn run(&mut self, module: &mut IrModule) -> bool {
        let mut changed = false;

        for func in &mut module.functions {
            changed |= eliminate_common_subexpressions_in_function(func);
        }

        changed
    }

    fn name(&self) -> &str {
        "common_subexpression_elimination"
    }
}

/// Eliminate common subexpressions in a function.
fn eliminate_common_subexpressions_in_function(func: &mut IrFunction) -> bool {
    let mut changed = false;

    // Map from expression to the value that computed it
    // Key is a simplified representation of the computation
    let mut expr_map: HashMap<ExprKey, ValueId> = HashMap::new();

    // Map from old value to replacement value
    let mut replacements: HashMap<ValueId, ValueId> = HashMap::new();

    for block in &mut func.blocks {
        for instr in &mut block.instructions {
            // Create a key for this expression
            let key = match instr {
                IrInstr::BinOp { op, lhs, rhs, .. } => {
                    let lhs = *replacements.get(lhs).unwrap_or(lhs);
                    let rhs = *replacements.get(rhs).unwrap_or(rhs);
                    Some(ExprKey::BinOp(*op, lhs, rhs))
                }
                IrInstr::UnOp { op, operand, .. } => {
                    let operand = *replacements.get(operand).unwrap_or(operand);
                    Some(ExprKey::UnOp(*op, operand))
                }
                IrInstr::Field { record, field, .. } => {
                    let record = *replacements.get(record).unwrap_or(record);
                    Some(ExprKey::Field(record, *field))
                }
                _ => None,
            };

            if let Some(key) = key {
                let result = instr.result_value().unwrap();

                if let Some(&existing) = expr_map.get(&key) {
                    // We've seen this expression before, record replacement
                    replacements.insert(result, existing);
                    changed = true;
                } else {
                    // First time seeing this expression
                    expr_map.insert(key, result);
                }
            }

            // Apply replacements to operands
            match instr {
                IrInstr::BinOp { lhs, rhs, .. } => {
                    if let Some(&new_lhs) = replacements.get(lhs) {
                        *lhs = new_lhs;
                    }
                    if let Some(&new_rhs) = replacements.get(rhs) {
                        *rhs = new_rhs;
                    }
                }
                IrInstr::UnOp { operand, .. } => {
                    if let Some(&new_operand) = replacements.get(operand) {
                        *operand = new_operand;
                    }
                }
                IrInstr::Call { args, .. } => {
                    for arg in args {
                        if let Some(&new_arg) = replacements.get(arg) {
                            *arg = new_arg;
                        }
                    }
                }
                IrInstr::Field { record, .. } => {
                    if let Some(&new_record) = replacements.get(record) {
                        *record = new_record;
                    }
                }
                IrInstr::Record { field_values, .. } => {
                    for val in field_values {
                        if let Some(&new_val) = replacements.get(val) {
                            *val = new_val;
                        }
                    }
                }
                IrInstr::Tuple { elements, .. } => {
                    for elem in elements {
                        if let Some(&new_elem) = replacements.get(elem) {
                            *elem = new_elem;
                        }
                    }
                }
                IrInstr::Phi { incoming, .. } => {
                    for (val, _) in incoming {
                        if let Some(&new_val) = replacements.get(val) {
                            *val = new_val;
                        }
                    }
                }
                _ => {}
            }
        }

        // Apply replacements to terminator
        match &mut block.terminator {
            IrTerminator::Return { value: Some(v), .. } => {
                if let Some(&new_v) = replacements.get(v) {
                    *v = new_v;
                }
            }
            IrTerminator::Branch { cond, .. } => {
                if let Some(&new_cond) = replacements.get(cond) {
                    *cond = new_cond;
                }
            }
            _ => {}
        }
    }

    changed
}

/// Key for identifying equivalent expressions in CSE.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ExprKey {
    BinOp(BinOp, ValueId, ValueId),
    UnOp(UnOp, ValueId),
    Field(ValueId, InternedString),
}
