//! Integration test demonstrating IR optimization passes.

use cadenza_eval::{InternedString, Type, ir::*};

fn dummy_source() -> SourceLocation {
    SourceLocation {
        file: InternedString::new("test.cdz"),
        line: 1,
        column: 1,
    }
}

#[test]
fn test_optimization_pipeline_integration() {
    // Build a function with opportunities for all three optimizations:
    // fn test() {
    //   let v0 = 2      // constant
    //   let v1 = 3      // constant
    //   let v2 = v0 + v1 // can be folded to 5
    //   let v3 = v0 + v1 // duplicate expression (CSE)
    //   let v4 = 99     // dead code (unused)
    //   ret v2
    // }

    let mut builder = IrBuilder::new();
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

    // Print before optimization
    println!("Before optimization:");
    println!("{}", module);
    println!();

    // Initial state: should have 5 instructions (2 consts, 2 adds, 1 dead const)
    assert_eq!(module.functions[0].blocks[0].instructions.len(), 5);

    // Run optimization pipeline
    let mut pipeline = OptimizationPipeline::default_pipeline();
    let changes = pipeline.run(&mut module, 10);

    // Print after optimization
    println!("After optimization ({} passes made changes):", changes);
    println!("{}", module);
    println!();

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

    let mut builder = IrBuilder::new();

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

    println!("Function with unused call result before optimization:");
    println!("{}", module);

    // Run DCE
    let mut pass = DeadCodeEliminationPass;
    pass.run(&mut module);

    println!("After dead code elimination:");
    println!("{}", module);

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
    let mut builder = IrBuilder::new();

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
