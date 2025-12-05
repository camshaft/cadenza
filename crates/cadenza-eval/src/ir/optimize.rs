//! IR Optimization passes
//!
//! This module provides a framework for optimizing Cadenza IR through configurable
//! optimization passes. Each pass implements the `IrOptimizationPass` trait and can
//! be composed into an optimization pipeline.
//!
//! # Example
//!
//! ```
//! use cadenza_eval::ir::optimize::{IrOptimizer, OptLevel, create_optimization_pipeline};
//! use cadenza_eval::ir::IrModule;
//!
//! let mut module = IrModule::new();
//! // ... build module ...
//!
//! let mut optimizer = create_optimization_pipeline(OptLevel::Basic);
//! optimizer.run(&mut module);
//! ```

use super::{BinOp, IrConst, IrInstr, IrModule, ValueId};
use std::collections::{HashMap, HashSet};

/// Trait for IR optimization passes
pub trait IrOptimizationPass {
    /// Run the optimization pass on the module
    /// Returns true if the module was modified
    fn run(&mut self, module: &mut IrModule) -> bool;

    /// Name of this optimization pass
    fn name(&self) -> &str;
}

/// Optimization pipeline - configurable sequence of passes
pub struct IrOptimizer {
    passes: Vec<Box<dyn IrOptimizationPass>>,
    max_iterations: usize,
}

impl IrOptimizer {
    pub fn new() -> Self {
        Self {
            passes: Vec::new(),
            max_iterations: 10,
        }
    }

    pub fn with_max_iterations(mut self, max_iterations: usize) -> Self {
        self.max_iterations = max_iterations;
        self
    }

    pub fn add_pass(&mut self, pass: Box<dyn IrOptimizationPass>) {
        self.passes.push(pass);
    }

    /// Run all passes until no changes occur or max iterations reached
    pub fn run(&mut self, module: &mut IrModule) {
        let mut changed = true;
        let mut iterations = 0;

        while changed && iterations < self.max_iterations {
            changed = false;
            for pass in &mut self.passes {
                if pass.run(module) {
                    changed = true;
                }
            }
            iterations += 1;
        }
    }
}

impl Default for IrOptimizer {
    fn default() -> Self {
        Self::new()
    }
}

/// Optimization levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptLevel {
    /// No optimizations
    None,
    /// Basic optimizations (constant folding, DCE)
    Basic,
    /// Aggressive optimizations (inlining, CSE, etc.)
    Aggressive,
}

/// Create a standard optimization pipeline for the given level
pub fn create_optimization_pipeline(level: OptLevel) -> IrOptimizer {
    let mut optimizer = IrOptimizer::new();

    match level {
        OptLevel::None => {
            // No optimizations
        }
        OptLevel::Basic => {
            optimizer.add_pass(Box::new(ConstantFolding));
            optimizer.add_pass(Box::new(DeadCodeElimination));
        }
        OptLevel::Aggressive => {
            optimizer.add_pass(Box::new(ConstantFolding));
            optimizer.add_pass(Box::new(DeadCodeElimination));
            optimizer.add_pass(Box::new(ConstantFolding)); // Run again after DCE
            // TODO: Add CSE, inlining, etc.
        }
    }

    optimizer
}

/// Constant folding optimization pass
///
/// Evaluates binary operations on constant values at compile time.
pub struct ConstantFolding;

impl IrOptimizationPass for ConstantFolding {
    fn name(&self) -> &str {
        "ConstantFolding"
    }

    fn run(&mut self, module: &mut IrModule) -> bool {
        let mut changed = false;

        // Build a map of value IDs to their constant values
        let mut constants: HashMap<ValueId, IrConst> = HashMap::new();

        for func in &mut module.functions {
            for block in &mut func.blocks {
                // First pass: collect constants
                for instr in &block.instructions {
                    if let IrInstr::Const { result, value } = instr {
                        constants.insert(*result, value.clone());
                    }
                }

                // Second pass: fold binary operations on constants
                for instr in &mut block.instructions {
                    if let IrInstr::BinOp {
                        result,
                        op,
                        lhs,
                        rhs,
                    } = instr
                    {
                        // Capture values before borrowing to avoid lifetime issues
                        let result_val = *result;
                        let op_val = *op;
                        let lhs_val = *lhs;
                        let rhs_val = *rhs;
                        
                        // Check if both operands are constants
                        if let (Some(lhs_const), Some(rhs_const)) =
                            (constants.get(&lhs_val), constants.get(&rhs_val))
                        {
                            // Try to fold the operation
                            if let Some(folded) = fold_binop(op_val, lhs_const, rhs_const) {
                                *instr = IrInstr::Const {
                                    result: result_val,
                                    value: folded.clone(),
                                };
                                constants.insert(result_val, folded);
                                changed = true;
                            }
                        }
                    }
                }
            }
        }

        changed
    }
}

/// Fold a binary operation on two constants
fn fold_binop(op: BinOp, lhs: &IrConst, rhs: &IrConst) -> Option<IrConst> {
    match (lhs, rhs) {
        // Integer operations
        (IrConst::Integer(l), IrConst::Integer(r)) => match op {
            BinOp::Add => Some(IrConst::Integer(l.wrapping_add(*r))),
            BinOp::Sub => Some(IrConst::Integer(l.wrapping_sub(*r))),
            BinOp::Mul => Some(IrConst::Integer(l.wrapping_mul(*r))),
            BinOp::Div if *r != 0 => Some(IrConst::Integer(l / r)),
            BinOp::Rem if *r != 0 => Some(IrConst::Integer(l % r)),
            BinOp::Eq => Some(IrConst::Bool(l == r)),
            BinOp::Ne => Some(IrConst::Bool(l != r)),
            BinOp::Lt => Some(IrConst::Bool(l < r)),
            BinOp::Le => Some(IrConst::Bool(l <= r)),
            BinOp::Gt => Some(IrConst::Bool(l > r)),
            BinOp::Ge => Some(IrConst::Bool(l >= r)),
            _ => None,
        },

        // Float operations
        (IrConst::Float(l), IrConst::Float(r)) => match op {
            BinOp::Add => Some(IrConst::Float(l + r)),
            BinOp::Sub => Some(IrConst::Float(l - r)),
            BinOp::Mul => Some(IrConst::Float(l * r)),
            BinOp::Div => Some(IrConst::Float(l / r)),
            BinOp::Eq => Some(IrConst::Bool((l - r).abs() < f64::EPSILON)),
            BinOp::Ne => Some(IrConst::Bool((l - r).abs() >= f64::EPSILON)),
            BinOp::Lt => Some(IrConst::Bool(l < r)),
            BinOp::Le => Some(IrConst::Bool(l <= r)),
            BinOp::Gt => Some(IrConst::Bool(l > r)),
            BinOp::Ge => Some(IrConst::Bool(l >= r)),
            _ => None,
        },

        // Boolean operations
        (IrConst::Bool(l), IrConst::Bool(r)) => match op {
            BinOp::And => Some(IrConst::Bool(*l && *r)),
            BinOp::Or => Some(IrConst::Bool(*l || *r)),
            BinOp::Eq => Some(IrConst::Bool(l == r)),
            BinOp::Ne => Some(IrConst::Bool(l != r)),
            _ => None,
        },

        _ => None,
    }
}

/// Dead code elimination pass
///
/// Removes instructions that compute values that are never used.
pub struct DeadCodeElimination;

impl IrOptimizationPass for DeadCodeElimination {
    fn name(&self) -> &str {
        "DeadCodeElimination"
    }

    fn run(&mut self, module: &mut IrModule) -> bool {
        let mut changed = false;

        for func in &mut module.functions {
            // Mark all used values
            let mut used = HashSet::new();
            mark_used_values(func, &mut used);

            // Remove instructions that produce unused values
            for block in &mut func.blocks {
                let original_len = block.instructions.len();
                block.instructions.retain(|instr| {
                    if let Some(result) = instr.result_value() {
                        if !used.contains(&result) {
                            return false; // Remove this instruction
                        }
                    }
                    true // Keep this instruction
                });
                if block.instructions.len() != original_len {
                    changed = true;
                }
            }
        }

        changed
    }
}

/// Mark all values that are used in the function
fn mark_used_values(func: &super::IrFunction, used: &mut HashSet<ValueId>) {
    for block in &func.blocks {
        // Mark values used by the terminator
        match &block.terminator {
            super::IrTerminator::Branch { cond, .. } => {
                used.insert(*cond);
            }
            super::IrTerminator::Return { value: Some(val) } => {
                used.insert(*val);
            }
            _ => {}
        }

        // Mark values used by instructions
        for instr in &block.instructions {
            match instr {
                IrInstr::BinOp { lhs, rhs, .. } => {
                    used.insert(*lhs);
                    used.insert(*rhs);
                }
                IrInstr::UnOp { operand, .. } => {
                    used.insert(*operand);
                }
                IrInstr::Call { args, .. } => {
                    for arg in args {
                        used.insert(*arg);
                    }
                }
                IrInstr::Record { fields, .. } => {
                    for (_, val) in fields {
                        used.insert(*val);
                    }
                }
                IrInstr::Field { record, .. } => {
                    used.insert(*record);
                }
                IrInstr::Tuple { elements, .. } => {
                    for elem in elements {
                        used.insert(*elem);
                    }
                }
                IrInstr::Phi { incoming, .. } => {
                    for (val, _) in incoming {
                        used.insert(*val);
                    }
                }
                IrInstr::Const { .. } => {
                    // Constants don't use other values
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{IrBuilder, IrTerminator};
    use crate::{InternedString, Type};

    #[test]
    fn test_constant_folding() {
        let mut builder = IrBuilder::new();

        // Create function: fn test() -> 42 + 8
        let _func_id = builder.start_function(InternedString::new("test"), Type::Integer);
        let _entry = builder.start_block();

        // %0 = const 42
        let v0 = builder.new_value();
        builder.emit(IrInstr::Const {
            result: v0,
            value: IrConst::Integer(42),
        });

        // %1 = const 8
        let v1 = builder.new_value();
        builder.emit(IrInstr::Const {
            result: v1,
            value: IrConst::Integer(8),
        });

        // %2 = binop Add %0 %1
        let v2 = builder.new_value();
        builder.emit(IrInstr::BinOp {
            result: v2,
            op: BinOp::Add,
            lhs: v0,
            rhs: v1,
        });

        builder.terminate(IrTerminator::Return { value: Some(v2) });

        let mut module = builder.finish();

        // Before optimization: should have 3 instructions (2 consts + 1 binop)
        assert_eq!(module.functions[0].blocks[0].instructions.len(), 3);

        // Run constant folding
        let mut cf = ConstantFolding;
        let changed = cf.run(&mut module);

        // Should have changed
        assert!(changed);

        // After optimization: binop should be folded to const
        let last_instr = &module.functions[0].blocks[0].instructions[2];
        if let IrInstr::Const { value, .. } = last_instr {
            if let IrConst::Integer(val) = value {
                assert_eq!(*val, 50);
            } else {
                panic!("Expected integer constant");
            }
        } else {
            panic!("Expected const instruction after folding");
        }
    }

    #[test]
    fn test_dead_code_elimination() {
        let mut builder = IrBuilder::new();

        // Create function with unused computation
        let _func_id = builder.start_function(InternedString::new("test"), Type::Integer);
        let _entry = builder.start_block();

        // %0 = const 42 (used)
        let v0 = builder.new_value();
        builder.emit(IrInstr::Const {
            result: v0,
            value: IrConst::Integer(42),
        });

        // %1 = const 8 (unused)
        let v1 = builder.new_value();
        builder.emit(IrInstr::Const {
            result: v1,
            value: IrConst::Integer(8),
        });

        // %2 = const 10 (unused)
        let v2 = builder.new_value();
        builder.emit(IrInstr::Const {
            result: v2,
            value: IrConst::Integer(10),
        });

        builder.terminate(IrTerminator::Return { value: Some(v0) });

        let mut module = builder.finish();

        // Before: 3 instructions
        assert_eq!(module.functions[0].blocks[0].instructions.len(), 3);

        // Run DCE
        let mut dce = DeadCodeElimination;
        let changed = dce.run(&mut module);

        assert!(changed);

        // After: should have only 1 instruction (the used constant)
        assert_eq!(module.functions[0].blocks[0].instructions.len(), 1);
    }

    #[test]
    fn test_optimization_pipeline() {
        let mut builder = IrBuilder::new();

        // Create function: fn test() -> { let x = 1 + 2; let y = 3 + 4; x }
        let _func_id = builder.start_function(InternedString::new("test"), Type::Integer);
        let _entry = builder.start_block();

        // Compute x = 1 + 2
        let v0 = builder.new_value();
        builder.emit(IrInstr::Const {
            result: v0,
            value: IrConst::Integer(1),
        });
        let v1 = builder.new_value();
        builder.emit(IrInstr::Const {
            result: v1,
            value: IrConst::Integer(2),
        });
        let x = builder.new_value();
        builder.emit(IrInstr::BinOp {
            result: x,
            op: BinOp::Add,
            lhs: v0,
            rhs: v1,
        });

        // Compute y = 3 + 4 (unused)
        let v2 = builder.new_value();
        builder.emit(IrInstr::Const {
            result: v2,
            value: IrConst::Integer(3),
        });
        let v3 = builder.new_value();
        builder.emit(IrInstr::Const {
            result: v3,
            value: IrConst::Integer(4),
        });
        let y = builder.new_value();
        builder.emit(IrInstr::BinOp {
            result: y,
            op: BinOp::Add,
            lhs: v2,
            rhs: v3,
        });

        builder.terminate(IrTerminator::Return { value: Some(x) });

        let mut module = builder.finish();

        // Before: 6 instructions
        assert_eq!(module.functions[0].blocks[0].instructions.len(), 6);

        // Run full pipeline
        let mut optimizer = create_optimization_pipeline(OptLevel::Basic);
        optimizer.run(&mut module);

        // After: constant folding + DCE should leave us with just 1 const
        // The binop gets folded to const 3, then the unused y computation is eliminated
        assert!(module.functions[0].blocks[0].instructions.len() <= 2);
    }
}
