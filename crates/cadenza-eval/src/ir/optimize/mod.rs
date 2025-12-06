//! Optimization passes for the IR.
//!
//! This module provides a framework for running optimization passes on IR modules
//! and implements several common optimization passes:
//! - Constant folding: Evaluate operations on constant values at compile time
//! - Dead code elimination: Remove instructions that produce unused values
//! - Common subexpression elimination: Detect and eliminate redundant computations

use super::types as ir_types;

// Re-export types module for submodules
use ir_types as types;

mod common_subexpression_elimination;
mod constant_folding;
mod dead_code_elimination;

pub use common_subexpression_elimination::CommonSubexpressionEliminationPass;
pub use constant_folding::ConstantFoldingPass;
pub use dead_code_elimination::DeadCodeEliminationPass;

use super::types::IrModule;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InternedString, Type};
    use ir_types::*;

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

    #[test]
    fn test_optimization_pipeline_with_cse() {
        // Build a function with opportunities for all three optimizations:
        // fn test() {
        //   let v0 = 2      // constant
        //   let v1 = 3      // constant
        //   let v2 = v0 + v1 // can be folded to 5
        //   let v3 = v0 + v1 // duplicate expression (CSE)
        //   let v4 = 99     // dead code (unused)
        //   ret v2
        // }

        let mut builder = crate::ir::IrBuilder::new();
        let mut func_builder = builder.function(InternedString::new("test"), vec![], Type::Integer);

        let mut block_builder = func_builder.block();
        let v0 = block_builder.const_val(IrConst::Integer(2), Type::Integer, dummy_source());
        let v1 = block_builder.const_val(IrConst::Integer(3), Type::Integer, dummy_source());
        let v2 = block_builder.binop(BinOp::Add, v0, v1, Type::Integer, dummy_source());
        let _v3 = block_builder.binop(BinOp::Add, v0, v1, Type::Integer, dummy_source()); // duplicate
        let _v4 = block_builder.const_val(IrConst::Integer(99), Type::Integer, dummy_source()); // dead
        let (block, next_value_id) = block_builder.ret(Some(v2), dummy_source());
        func_builder.add_block(block, next_value_id);

        let func = func_builder.build();
        let mut module = IrModule::new();
        module.functions.push(func);

        // Initial state: should have 5 instructions (2 consts, 2 adds, 1 dead const)
        assert_eq!(module.functions[0].blocks[0].instructions.len(), 5);

        // Run optimization pipeline
        let mut pipeline = OptimizationPipeline::default_pipeline();
        let changes = pipeline.run(&mut module, 10);

        assert!(changes > 0, "Pipeline should have made changes");

        // After optimization:
        // - Constant folding: v0 + v1 should become const 5
        // - CSE: duplicate v0 + v1 should be eliminated
        // - DCE: the unused const 99 should be removed
        // Result should be: just one const 5 and return it
        let optimized_instrs = &module.functions[0].blocks[0].instructions;

        // Should have significantly fewer instructions
        assert!(
            optimized_instrs.len() <= 1,
            "Expected at most 1 instruction after optimization, got {}",
            optimized_instrs.len()
        );

        // The remaining instruction should be a constant 5
        if let Some(IrInstr::Const {
            value: IrConst::Integer(val),
            ..
        }) = optimized_instrs.first()
        {
            assert_eq!(*val, 5, "Expected constant to be folded to 5");
        }
    }

    #[test]
    fn test_dead_code_with_side_effects() {
        // Function calls should not be eliminated even if their result is unused
        // because they might have side effects

        let mut builder = crate::ir::IrBuilder::new();

        // First create a dummy function to call
        let mut dummy_func = builder.function(InternedString::new("dummy"), vec![], Type::Integer);
        let mut block_builder = dummy_func.block();
        let val = block_builder.const_val(IrConst::Integer(42), Type::Integer, dummy_source());
        let (block, next_val) = block_builder.ret(Some(val), dummy_source());
        dummy_func.add_block(block, next_val);
        let dummy_id = builder.add_function(dummy_func.build());

        // Now create test function that calls it
        let mut test_func = builder.function(InternedString::new("test"), vec![], Type::Integer);
        let mut block_builder = test_func.block();
        let _call_result = block_builder.call(dummy_id, vec![], Type::Integer, dummy_source());
        let ret_val = block_builder.const_val(IrConst::Integer(1), Type::Integer, dummy_source());
        let (block, next_val) = block_builder.ret(Some(ret_val), dummy_source());
        test_func.add_block(block, next_val);
        builder.add_function(test_func.build());

        let mut module = builder.build();

        // Run DCE
        let mut pass = DeadCodeEliminationPass;
        pass.run(&mut module);

        // The call instruction should still be present even though its result is unused
        let test_fn = &module.functions[1]; // test function is the second function
        let has_call = test_fn.blocks[0]
            .instructions
            .iter()
            .any(|instr| matches!(instr, IrInstr::Call { .. }));
        assert!(
            has_call,
            "Call instruction should not be eliminated (might have side effects)"
        );
    }

    #[test]
    fn test_constant_folding_preserves_types() {
        let mut module = IrModule::new();
        let mut builder = crate::ir::IrBuilder::new();

        // Test that constant folding preserves the original type
        let mut func_builder = builder.function(InternedString::new("test"), vec![], Type::Float);

        let mut block_builder = func_builder.block();
        let v0 = block_builder.const_val(IrConst::Float(2.5), Type::Float, dummy_source());
        let v1 = block_builder.const_val(IrConst::Float(2.5), Type::Float, dummy_source());
        let v2 = block_builder.binop(BinOp::Add, v0, v1, Type::Float, dummy_source());
        let (block, next_value_id) = block_builder.ret(Some(v2), dummy_source());
        func_builder.add_block(block, next_value_id);

        let func = func_builder.build();
        module.functions.push(func);

        let mut pass = ConstantFoldingPass;
        pass.run(&mut module);

        // Check that the folded constant still has Float type
        let block = &module.functions[0].blocks[0];
        let has_float_const = block.instructions.iter().any(|instr| {
            matches!(
                instr,
                IrInstr::Const {
                    ty: Type::Float,
                    value: IrConst::Float(5.0),
                    ..
                }
            )
        });
        assert!(
            has_float_const,
            "Folded constant should preserve Float type"
        );
    }
}
