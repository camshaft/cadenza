//! Tests for the IR types and basic construction.

use super::*;
use crate::{InternedString, Type};
use std::sync::Arc;

#[test]
fn test_value_id_display() {
    let id = ValueId(0);
    assert_eq!(id.to_string(), "v0");

    let id = ValueId(42);
    assert_eq!(id.to_string(), "v42");
}

#[test]
fn test_block_id_display() {
    let id = BlockId(0);
    assert_eq!(id.to_string(), "block_0");

    let id = BlockId(5);
    assert_eq!(id.to_string(), "block_5");
}

#[test]
fn test_function_id_display() {
    let id = FunctionId(0);
    assert_eq!(id.to_string(), "func0");

    let id = FunctionId(10);
    assert_eq!(id.to_string(), "func10");
}

#[test]
fn test_const_display() {
    assert_eq!(IrConst::Nil.to_string(), "nil");
    assert_eq!(IrConst::Bool(true).to_string(), "true");
    assert_eq!(IrConst::Bool(false).to_string(), "false");
    assert_eq!(IrConst::Integer(42).to_string(), "42");
    assert_eq!(IrConst::Float(3.15).to_string(), "3.15");
}

#[test]
fn test_binop_display() {
    assert_eq!(BinOp::Add.to_string(), "add");
    assert_eq!(BinOp::Sub.to_string(), "sub");
    assert_eq!(BinOp::Mul.to_string(), "mul");
    assert_eq!(BinOp::Div.to_string(), "div");
    assert_eq!(BinOp::Eq.to_string(), "eq");
    assert_eq!(BinOp::Ne.to_string(), "ne");
    assert_eq!(BinOp::Lt.to_string(), "lt");
}

#[test]
fn test_unop_display() {
    assert_eq!(UnOp::Neg.to_string(), "neg");
    assert_eq!(UnOp::Not.to_string(), "not");
    assert_eq!(UnOp::BitNot.to_string(), "bitnot");
}

fn dummy_source() -> SourceLocation {
    SourceLocation {
        file: InternedString::new("test.cdz"),
        line: 1,
        column: 0,
    }
}

#[test]
fn test_const_instr() {
    let instr = IrInstr::Const {
        result: ValueId(0),
        ty: Type::Integer,
        value: IrConst::Integer(42),
        source: dummy_source(),
    };

    assert_eq!(instr.to_string(), "let v0: integer = 42");
    assert_eq!(instr.result_value(), Some(ValueId(0)));
}

#[test]
fn test_binop_instr() {
    let instr = IrInstr::BinOp {
        result: ValueId(2),
        ty: Type::Integer,
        op: BinOp::Add,
        lhs: ValueId(0),
        rhs: ValueId(1),
        source: dummy_source(),
    };

    assert_eq!(instr.to_string(), "let v2: integer = add v0 v1");
    assert_eq!(instr.result_value(), Some(ValueId(2)));
}

#[test]
fn test_unop_instr() {
    let instr = IrInstr::UnOp {
        result: ValueId(1),
        ty: Type::Integer,
        op: UnOp::Neg,
        operand: ValueId(0),
        source: dummy_source(),
    };

    assert_eq!(instr.to_string(), "let v1: integer = neg v0");
    assert_eq!(instr.result_value(), Some(ValueId(1)));
}

#[test]
fn test_call_instr() {
    let instr = IrInstr::Call {
        result: Some(ValueId(3)),
        ty: Type::Integer,
        func: FunctionId(0),
        args: vec![ValueId(0), ValueId(1), ValueId(2)],
        source: dummy_source(),
    };

    assert_eq!(instr.to_string(), "let v3: integer = func0 v0 v1 v2");
    assert_eq!(instr.result_value(), Some(ValueId(3)));
}

#[test]
fn test_return_terminator() {
    let term = IrTerminator::Return {
        value: Some(ValueId(5)),
        source: dummy_source(),
    };

    assert_eq!(term.to_string(), "v5");
}

#[test]
fn test_branch_terminator() {
    let term = IrTerminator::Branch {
        cond: ValueId(0),
        then_block: BlockId(1),
        else_block: BlockId(2),
        source: dummy_source(),
    };

    assert_eq!(term.to_string(), "br v0 block_1 block_2");
}

#[test]
fn test_jump_terminator() {
    let term = IrTerminator::Jump {
        target: BlockId(3),
        source: dummy_source(),
    };

    assert_eq!(term.to_string(), "jmp block_3");
}

#[test]
fn test_basic_block() {
    let block = IrBlock {
        id: BlockId(0),
        instructions: vec![
            IrInstr::Const {
                result: ValueId(0),
                ty: Type::Integer,
                value: IrConst::Integer(5),
                source: dummy_source(),
            },
            IrInstr::Const {
                result: ValueId(1),
                ty: Type::Integer,
                value: IrConst::Integer(10),
                source: dummy_source(),
            },
            IrInstr::BinOp {
                result: ValueId(2),
                ty: Type::Integer,
                op: BinOp::Add,
                lhs: ValueId(0),
                rhs: ValueId(1),
                source: dummy_source(),
            },
        ],
        terminator: IrTerminator::Return {
            value: Some(ValueId(2)),
            source: dummy_source(),
        },
    };

    let output = block.to_string();
    assert!(output.contains("# block_0:"));
    assert!(output.contains("let v0: integer = 5"));
    assert!(output.contains("let v1: integer = 10"));
    assert!(output.contains("let v2: integer = add v0 v1"));
    assert!(output.contains("v2"));
}

#[test]
fn test_function() {
    let func = IrFunction {
        id: FunctionId(0),
        name: InternedString::new("add_numbers"),
        params: vec![
            IrParam {
                name: InternedString::new("a"),
                ty: Type::Integer,
                value_id: ValueId(0),
            },
            IrParam {
                name: InternedString::new("b"),
                ty: Type::Integer,
                value_id: ValueId(1),
            },
        ],
        return_ty: Type::Integer,
        blocks: vec![IrBlock {
            id: BlockId(0),
            instructions: vec![IrInstr::BinOp {
                result: ValueId(2),
                ty: Type::Integer,
                op: BinOp::Add,
                lhs: ValueId(0),
                rhs: ValueId(1),
                source: dummy_source(),
            }],
            terminator: IrTerminator::Return {
                value: Some(ValueId(2)),
                source: dummy_source(),
            },
        }],
        entry_block: BlockId(0),
    };

    let output = func.to_string();
    assert!(output.contains("fn add_numbers"));
    assert!(output.contains("v0: integer"));
    assert!(output.contains("v1: integer"));
    assert!(output.contains("let v2: integer = add v0 v1"));
    assert!(output.contains("v2"));
}

#[test]
fn test_module() {
    let module = IrModule {
        functions: vec![IrFunction {
            id: FunctionId(0),
            name: InternedString::new("main"),
            params: vec![],
            return_ty: Type::Integer,
            blocks: vec![IrBlock {
                id: BlockId(0),
                instructions: vec![IrInstr::Const {
                    result: ValueId(0),
                    ty: Type::Integer,
                    value: IrConst::Integer(42),
                    source: dummy_source(),
                }],
                terminator: IrTerminator::Return {
                    value: Some(ValueId(0)),
                    source: dummy_source(),
                },
            }],
            entry_block: BlockId(0),
        }],
        exports: vec![IrExport {
            name: InternedString::new("main"),
            kind: IrExportKind::Function(FunctionId(0)),
        }],
    };

    let output = module.to_string();
    assert!(output.contains("# IR Module"));
    assert!(output.contains("fn main"));
    assert!(output.contains("# export main as function func0"));
}

#[test]
fn test_empty_module() {
    let module = IrModule::new();
    assert!(module.functions.is_empty());
    assert!(module.exports.is_empty());
}

#[test]
fn test_builder_simple_function() {
    let mut builder = IrBuilder::new();

    // Build a function that returns 42
    let mut func_builder =
        builder.function(InternedString::new("get_answer"), vec![], Type::Integer);

    let mut block = func_builder.block();
    let val = block.const_val(IrConst::Integer(42), Type::Integer, dummy_source());
    let (block, next_val) = block.ret(Some(val), dummy_source());
    let entry_id = func_builder.add_block(block, next_val);

    let func = func_builder.build();
    assert_eq!(func.name, InternedString::new("get_answer"));
    assert_eq!(func.params.len(), 0);
    assert_eq!(func.return_ty, Type::Integer);
    assert_eq!(func.blocks.len(), 1);
    assert_eq!(func.entry_block, entry_id);

    let func_id = builder.add_function(func);
    builder.export_function(InternedString::new("get_answer"), func_id);

    let module = builder.build();
    assert_eq!(module.functions.len(), 1);
    assert_eq!(module.exports.len(), 1);
}

#[test]
fn test_builder_add_function() {
    let mut builder = IrBuilder::new();

    // Build a function: fn add(a, b) = a + b
    let mut func_builder = builder.function(
        InternedString::new("add"),
        vec![
            (InternedString::new("a"), Type::Integer),
            (InternedString::new("b"), Type::Integer),
        ],
        Type::Integer,
    );

    let mut block = func_builder.block();
    // Parameters are %0 and %1
    let result = block.binop(
        BinOp::Add,
        ValueId(0),
        ValueId(1),
        Type::Integer,
        dummy_source(),
    );
    let (block, next_val) = block.ret(Some(result), dummy_source());
    func_builder.add_block(block, next_val);

    let func = func_builder.build();
    assert_eq!(func.params.len(), 2);
    assert_eq!(func.params[0].value_id, ValueId(0));
    assert_eq!(func.params[1].value_id, ValueId(1));

    // Check the result value is %2 (after parameters %0 and %1)
    assert_eq!(result, ValueId(2));

    let func_id = builder.add_function(func);
    builder.export_function(InternedString::new("add"), func_id);

    let module = builder.build();
    let output = module.to_string();
    assert!(output.contains("fn add"));
    assert!(output.contains("v0: integer"));
    assert!(output.contains("v1: integer"));
    assert!(output.contains("let v2: integer = add v0 v1"));
}

#[test]
fn test_builder_conditional() {
    let mut builder = IrBuilder::new();

    // Build a function with conditional:
    // fn abs(x) = if x < 0 then -x else x
    let mut func_builder = builder.function(
        InternedString::new("abs"),
        vec![(InternedString::new("x"), Type::Integer)],
        Type::Integer,
    );

    // Create block IDs ahead of time
    let mut entry_block = func_builder.block();
    let _entry_id = entry_block.id();

    let mut then_block = func_builder.block();
    let then_id = then_block.id();

    let else_block = func_builder.block();
    let else_id = else_block.id();

    // Entry block: check if x < 0
    let zero = entry_block.const_val(IrConst::Integer(0), Type::Integer, dummy_source());
    let cond = entry_block.binop(BinOp::Lt, ValueId(0), zero, Type::Bool, dummy_source());
    let (entry_block_inst, next_val) = entry_block.branch(cond, then_id, else_id, dummy_source());
    func_builder.add_block(entry_block_inst, next_val);

    // Then block: return -x
    let neg_x = then_block.unop(UnOp::Neg, ValueId(0), Type::Integer, dummy_source());
    let (then_block_inst, next_val) = then_block.ret(Some(neg_x), dummy_source());
    func_builder.add_block(then_block_inst, next_val);

    // Else block: return x
    let (else_block_inst, next_val) = else_block.ret(Some(ValueId(0)), dummy_source());
    func_builder.add_block(else_block_inst, next_val);

    let func = func_builder.build();
    assert_eq!(func.blocks.len(), 3);

    builder.add_function(func);
    let module = builder.build();
    let output = module.to_string();

    // Verify the structure
    assert!(output.contains("fn abs"));
    assert!(output.contains("br v"));
    assert!(output.contains("neg v0"));
    assert!(output.contains("v0"));
}

#[test]
fn test_builder_record_construction() {
    let mut builder = IrBuilder::new();

    let mut func_builder = builder.function(
        InternedString::new("make_point"),
        vec![
            (InternedString::new("x"), Type::Integer),
            (InternedString::new("y"), Type::Integer),
        ],
        Type::Record(vec![
            (InternedString::new("x"), Type::Integer),
            (InternedString::new("y"), Type::Integer),
        ]),
    );

    let mut block = func_builder.block();
    let record_ty = Type::Record(vec![
        (InternedString::new("x"), Type::Integer),
        (InternedString::new("y"), Type::Integer),
    ]);
    let record = block.record(
        Arc::from([InternedString::new("x"), InternedString::new("y")]),
        vec![ValueId(0), ValueId(1)],
        record_ty,
        dummy_source(),
    );
    let (block, next_val) = block.ret(Some(record), dummy_source());
    func_builder.add_block(block, next_val);

    let func = func_builder.build();
    builder.add_function(func);

    let module = builder.build();
    let output = module.to_string();
    assert!(output.contains("{ x = v0, y = v1 }"));
}
