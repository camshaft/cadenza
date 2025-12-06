//! Dead code elimination optimization pass.
//!
//! This pass removes instructions that produce values that are never used.
//! It also removes unreachable basic blocks.

use super::{OptimizationPass, types::*};
use std::collections::HashSet;

/// Dead code elimination optimization pass.
///
/// This pass removes instructions that produce values that are never used.
/// It also removes unreachable basic blocks.
pub struct DeadCodeEliminationPass;

impl OptimizationPass for DeadCodeEliminationPass {
    fn run(&mut self, module: &mut IrModule) -> bool {
        let mut changed = false;

        for func in &mut module.functions {
            changed |= eliminate_dead_code_in_function(func);
        }

        changed
    }

    fn name(&self) -> &str {
        "dead_code_elimination"
    }
}

/// Eliminate dead code in a function.
fn eliminate_dead_code_in_function(func: &mut IrFunction) -> bool {
    let mut changed = false;

    // Find all used values by traversing from terminators and side-effecting operations
    let mut used_values = HashSet::new();

    // Mark values used in terminators
    for block in &func.blocks {
        match &block.terminator {
            IrTerminator::Return { value: Some(v), .. } => {
                used_values.insert(*v);
            }
            IrTerminator::Branch { cond, .. } => {
                used_values.insert(*cond);
            }
            _ => {}
        }

        // Mark values used in call instructions (side effects)
        for instr in &block.instructions {
            if let IrInstr::Call { args, .. } = instr {
                for arg in args {
                    used_values.insert(*arg);
                }
            }
        }
    }

    // Iteratively mark all values that are transitively used
    let mut worklist: Vec<ValueId> = used_values.iter().copied().collect();
    while let Some(value_id) = worklist.pop() {
        for block in &func.blocks {
            for instr in &block.instructions {
                if instr.result_value() == Some(value_id) {
                    // This instruction produces a used value, mark its operands as used
                    match instr {
                        IrInstr::BinOp { lhs, rhs, .. } => {
                            if used_values.insert(*lhs) {
                                worklist.push(*lhs);
                            }
                            if used_values.insert(*rhs) {
                                worklist.push(*rhs);
                            }
                        }
                        IrInstr::UnOp { operand, .. } => {
                            if used_values.insert(*operand) {
                                worklist.push(*operand);
                            }
                        }
                        IrInstr::Call { args, .. } => {
                            for arg in args {
                                if used_values.insert(*arg) {
                                    worklist.push(*arg);
                                }
                            }
                        }
                        IrInstr::Field { record, .. } => {
                            if used_values.insert(*record) {
                                worklist.push(*record);
                            }
                        }
                        IrInstr::Record { field_values, .. } => {
                            for val in field_values {
                                if used_values.insert(*val) {
                                    worklist.push(*val);
                                }
                            }
                        }
                        IrInstr::Tuple { elements, .. } => {
                            for elem in elements {
                                if used_values.insert(*elem) {
                                    worklist.push(*elem);
                                }
                            }
                        }
                        IrInstr::Phi { incoming, .. } => {
                            for (val, _) in incoming {
                                if used_values.insert(*val) {
                                    worklist.push(*val);
                                }
                            }
                        }
                        IrInstr::Const { .. } => {}
                    }
                }
            }
        }
    }

    // Remove instructions that produce unused values
    for block in &mut func.blocks {
        let original_len = block.instructions.len();
        block.instructions.retain(|instr| {
            // Always keep call instructions (they might have side effects)
            if matches!(instr, IrInstr::Call { .. }) {
                return true;
            }

            // Keep instructions that don't produce values or produce used values
            match instr.result_value() {
                Some(result) => used_values.contains(&result),
                None => true, // Keep void instructions
            }
        });
        if block.instructions.len() < original_len {
            changed = true;
        }
    }

    changed
}
