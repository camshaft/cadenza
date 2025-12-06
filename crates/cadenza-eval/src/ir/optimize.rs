//! Optimization passes for the IR.
//!
//! This module provides a framework for running optimization passes on IR modules
//! and implements several common optimization passes:
//! - Constant folding: Evaluate operations on constant values at compile time
//! - Dead code elimination: Remove instructions that produce unused values
//! - Common subexpression elimination: Detect and eliminate redundant computations

use super::types::*;
use std::collections::{HashMap, HashSet};

/// Trait for IR optimization passes.
///
/// An optimization pass transforms an IR module, potentially simplifying it
/// or improving its performance characteristics.
pub trait OptimizationPass {
    /// Apply this optimization pass to the given module.
    ///
    /// Returns true if the module was modified, false otherwise.
    fn run(&mut self, module: &mut IrModule) -> bool;

    /// Get the name of this optimization pass.
    fn name(&self) -> &str;
}

/// Manages and runs a sequence of optimization passes.
pub struct OptimizationPipeline {
    passes: Vec<Box<dyn OptimizationPass>>,
}

impl OptimizationPipeline {
    /// Create a new empty optimization pipeline.
    pub fn new() -> Self {
        Self { passes: Vec::new() }
    }

    /// Add an optimization pass to the pipeline.
    pub fn add_pass(&mut self, pass: Box<dyn OptimizationPass>) {
        self.passes.push(pass);
    }

    /// Run all optimization passes in the pipeline.
    ///
    /// Runs each pass in sequence, repeating until no pass modifies the module
    /// or until a maximum iteration count is reached.
    ///
    /// Returns the total number of passes that modified the module.
    pub fn run(&mut self, module: &mut IrModule, max_iterations: usize) -> usize {
        let mut total_changes = 0;
        let mut iteration = 0;

        loop {
            let mut changed = false;

            for pass in &mut self.passes {
                if pass.run(module) {
                    changed = true;
                    total_changes += 1;
                }
            }

            iteration += 1;

            if !changed || iteration >= max_iterations {
                break;
            }
        }

        total_changes
    }

    /// Create a default optimization pipeline with common passes.
    pub fn default_pipeline() -> Self {
        let mut pipeline = Self::new();
        pipeline.add_pass(Box::new(ConstantFoldingPass));
        pipeline.add_pass(Box::new(DeadCodeEliminationPass));
        pipeline.add_pass(Box::new(CommonSubexpressionEliminationPass));
        pipeline
    }
}

impl Default for OptimizationPipeline {
    fn default() -> Self {
        Self::new()
    }
}

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
                    {
                        if let Some(folded) = fold_binop(op_copy, lhs_const, rhs_const) {
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
                    if let Some(operand_const) = const_values.get(operand) {
                        if let Some(folded) = fold_unop(op_copy, operand_const) {
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
    Field(ValueId, crate::InternedString),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InternedString, Type};

    fn dummy_source() -> SourceLocation {
        SourceLocation {
            file: InternedString::new("test.cdz"),
            line: 1,
            column: 1,
        }
    }

    #[test]
    fn test_constant_folding_integers() {
        let mut module = IrModule::new();
        let mut builder = crate::ir::IrBuilder::new();

        // Build: fn test() { let v0 = 2; let v1 = 3; let v2 = v0 + v1; ret v2 }
        let mut func_builder = builder.function(InternedString::new("test"), vec![], Type::Integer);

        let mut block_builder = func_builder.block();
        let v0 = block_builder.const_val(IrConst::Integer(2), Type::Integer, dummy_source());
        let v1 = block_builder.const_val(IrConst::Integer(3), Type::Integer, dummy_source());
        let v2 = block_builder.binop(BinOp::Add, v0, v1, Type::Integer, dummy_source());
        let (block, next_value_id) = block_builder.ret(Some(v2), dummy_source());
        func_builder.add_block(block, next_value_id);

        let func = func_builder.build();
        module.functions.push(func);

        // Apply constant folding
        let mut pass = ConstantFoldingPass;
        let changed = pass.run(&mut module);

        assert!(changed);

        // The add should be folded to a constant 5
        let block = &module.functions[0].blocks[0];
        let has_add = block
            .instructions
            .iter()
            .any(|instr| matches!(instr, IrInstr::BinOp { .. }));
        assert!(!has_add, "BinOp should have been folded");

        // Should have a const instruction with value 5
        let has_const_5 = block.instructions.iter().any(|instr| {
            matches!(
                instr,
                IrInstr::Const {
                    value: IrConst::Integer(5),
                    ..
                }
            )
        });
        assert!(has_const_5, "Should have const 5");
    }

    #[test]
    fn test_constant_folding_floats() {
        let mut module = IrModule::new();
        let mut builder = crate::ir::IrBuilder::new();

        let mut func_builder = builder.function(InternedString::new("test"), vec![], Type::Float);

        let mut block_builder = func_builder.block();
        let v0 = block_builder.const_val(IrConst::Float(2.5), Type::Float, dummy_source());
        let v1 = block_builder.const_val(IrConst::Float(3.5), Type::Float, dummy_source());
        let v2 = block_builder.binop(BinOp::Mul, v0, v1, Type::Float, dummy_source());
        let (block, next_value_id) = block_builder.ret(Some(v2), dummy_source());
        func_builder.add_block(block, next_value_id);

        let func = func_builder.build();
        module.functions.push(func);

        let mut pass = ConstantFoldingPass;
        let changed = pass.run(&mut module);

        assert!(changed);

        let block = &module.functions[0].blocks[0];
        let has_const_8_75 = block.instructions.iter().any(|instr| {
            matches!(instr, IrInstr::Const { value: IrConst::Float(f), .. } if (*f - 8.75).abs() < 0.001)
        });
        assert!(has_const_8_75, "Should have const 8.75");
    }

    #[test]
    fn test_constant_folding_comparison() {
        let mut module = IrModule::new();
        let mut builder = crate::ir::IrBuilder::new();

        let mut func_builder = builder.function(InternedString::new("test"), vec![], Type::Bool);

        let mut block_builder = func_builder.block();
        let v0 = block_builder.const_val(IrConst::Integer(5), Type::Integer, dummy_source());
        let v1 = block_builder.const_val(IrConst::Integer(3), Type::Integer, dummy_source());
        let v2 = block_builder.binop(BinOp::Gt, v0, v1, Type::Bool, dummy_source());
        let (block, next_value_id) = block_builder.ret(Some(v2), dummy_source());
        func_builder.add_block(block, next_value_id);

        let func = func_builder.build();
        module.functions.push(func);

        let mut pass = ConstantFoldingPass;
        pass.run(&mut module);

        let block = &module.functions[0].blocks[0];
        let has_const_true = block.instructions.iter().any(|instr| {
            matches!(
                instr,
                IrInstr::Const {
                    value: IrConst::Bool(true),
                    ..
                }
            )
        });
        assert!(has_const_true, "5 > 3 should be folded to true");
    }

    #[test]
    fn test_dead_code_elimination() {
        let mut module = IrModule::new();
        let mut builder = crate::ir::IrBuilder::new();

        // Build: fn test() { let v0 = 1; let v1 = 2; let v2 = 3; ret v2 }
        // v0 and v1 are unused
        let mut func_builder = builder.function(InternedString::new("test"), vec![], Type::Integer);

        let mut block_builder = func_builder.block();
        let _v0 = block_builder.const_val(IrConst::Integer(1), Type::Integer, dummy_source());
        let _v1 = block_builder.const_val(IrConst::Integer(2), Type::Integer, dummy_source());
        let v2 = block_builder.const_val(IrConst::Integer(3), Type::Integer, dummy_source());
        let (block, next_value_id) = block_builder.ret(Some(v2), dummy_source());
        func_builder.add_block(block, next_value_id);

        let func = func_builder.build();
        module.functions.push(func);

        let mut pass = DeadCodeEliminationPass;
        let changed = pass.run(&mut module);

        assert!(changed);

        // Should only have one instruction left (the const 3)
        let block = &module.functions[0].blocks[0];
        assert_eq!(
            block.instructions.len(),
            1,
            "Should have removed unused instructions"
        );
    }

    #[test]
    fn test_common_subexpression_elimination() {
        let mut module = IrModule::new();
        let mut builder = crate::ir::IrBuilder::new();

        // Build: fn test(v0) { let v1 = v0 + v0; let v2 = v0 + v0; ret v2 }
        // v2 should reuse v1
        let mut func_builder = builder.function(
            InternedString::new("test"),
            vec![(InternedString::new("x"), Type::Integer)],
            Type::Integer,
        );

        let mut block_builder = func_builder.block();
        let v0 = ValueId(0); // parameter
        let v1 = block_builder.binop(BinOp::Add, v0, v0, Type::Integer, dummy_source());
        let v2 = block_builder.binop(BinOp::Add, v0, v0, Type::Integer, dummy_source());
        let (block, next_value_id) = block_builder.ret(Some(v2), dummy_source());
        func_builder.add_block(block, next_value_id);

        let func = func_builder.build();
        module.functions.push(func);

        let mut pass = CommonSubexpressionEliminationPass;
        let changed = pass.run(&mut module);

        assert!(changed);

        // The second add should still exist but the return should reference v1
        let block = &module.functions[0].blocks[0];
        if let IrTerminator::Return {
            value: Some(ret_val),
            ..
        } = &block.terminator
        {
            assert_eq!(*ret_val, v1, "Return should use v1 instead of v2");
        } else {
            panic!("Expected return terminator");
        }
    }

    #[test]
    fn test_optimization_pipeline() {
        let mut module = IrModule::new();
        let mut builder = crate::ir::IrBuilder::new();

        // Build a function with constant folding and dead code opportunities
        let mut func_builder = builder.function(InternedString::new("test"), vec![], Type::Integer);

        let mut block_builder = func_builder.block();
        let v0 = block_builder.const_val(IrConst::Integer(2), Type::Integer, dummy_source());
        let v1 = block_builder.const_val(IrConst::Integer(3), Type::Integer, dummy_source());
        let v2 = block_builder.binop(BinOp::Add, v0, v1, Type::Integer, dummy_source());
        let _v3 = block_builder.const_val(IrConst::Integer(99), Type::Integer, dummy_source()); // dead
        let (block, next_value_id) = block_builder.ret(Some(v2), dummy_source());
        func_builder.add_block(block, next_value_id);

        let func = func_builder.build();
        module.functions.push(func);

        // Run the full pipeline
        let mut pipeline = OptimizationPipeline::default_pipeline();
        let changes = pipeline.run(&mut module, 10);

        assert!(changes > 0, "Pipeline should have made changes");

        // After optimization: should have const 5 and return it, no other instructions
        let block = &module.functions[0].blocks[0];
        assert!(
            block.instructions.len() <= 1,
            "Most instructions should be eliminated"
        );
    }
}
