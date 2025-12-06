//! Builder API for constructing IR.
//!
//! This module provides a convenient builder pattern for constructing IR modules,
//! functions, and basic blocks. It handles automatic ID assignment and provides
//! methods for emitting instructions.

use crate::*;
use cadenza_eval::{InternedString, Type};
use std::sync::Arc;

/// Builder for constructing IR modules.
pub struct IrBuilder {
    module: IrModule,
    next_function_id: u32,
}

impl IrBuilder {
    /// Create a new IR builder.
    pub fn new() -> Self {
        Self {
            module: IrModule::new(),
            next_function_id: 0,
        }
    }

    /// Start building a new function.
    pub fn function(
        &mut self,
        name: InternedString,
        params: Vec<(InternedString, Type)>,
        return_ty: Type,
    ) -> FunctionBuilder {
        let id = FunctionId(self.next_function_id);
        self.next_function_id += 1;
        
        let param_count = params.len() as u32;

        FunctionBuilder {
            id,
            name,
            params,
            return_ty,
            blocks: Vec::new(),
            next_block_id: 0,
            next_value_id: param_count, // Parameters use first N value IDs
        }
    }

    /// Add a completed function to the module.
    pub fn add_function(&mut self, func: IrFunction) -> FunctionId {
        let id = func.id;
        self.module.functions.push(func);
        id
    }

    /// Export a function.
    pub fn export_function(&mut self, name: InternedString, func_id: FunctionId) {
        self.module.exports.push(IrExport {
            name,
            kind: IrExportKind::Function(func_id),
        });
    }

    /// Export a constant.
    pub fn export_constant(&mut self, name: InternedString, value_id: ValueId) {
        self.module.exports.push(IrExport {
            name,
            kind: IrExportKind::Constant(value_id),
        });
    }

    /// Build and return the final IR module.
    pub fn build(self) -> IrModule {
        self.module
    }
}

impl Default for IrBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for constructing IR functions.
pub struct FunctionBuilder {
    id: FunctionId,
    name: InternedString,
    params: Vec<(InternedString, Type)>,
    return_ty: Type,
    blocks: Vec<IrBlock>,
    next_block_id: u32,
    next_value_id: u32,
}

impl FunctionBuilder {
    /// Create a new basic block.
    pub fn block(&mut self) -> BlockBuilder {
        let id = BlockId(self.next_block_id);
        self.next_block_id += 1;

        BlockBuilder {
            id,
            instructions: Vec::new(),
            start_value_id: self.next_value_id,
        }
    }

    /// Add a completed basic block to the function.
    /// Returns the block ID and updates the next value ID counter.
    pub fn add_block(&mut self, block: IrBlock, next_value_id: u32) -> BlockId {
        let id = block.id;
        self.next_value_id = next_value_id;
        self.blocks.push(block);
        id
    }

    /// Build and return the final IR function.
    /// The first block added becomes the entry block.
    pub fn build(self) -> IrFunction {
        let entry_block = self
            .blocks
            .first()
            .map(|b| b.id)
            .unwrap_or(BlockId(0));

        let params = self
            .params
            .into_iter()
            .enumerate()
            .map(|(i, (name, ty))| IrParam {
                name,
                ty,
                value_id: ValueId(i as u32),
            })
            .collect();

        IrFunction {
            id: self.id,
            name: self.name,
            params,
            return_ty: self.return_ty,
            blocks: self.blocks,
            entry_block,
        }
    }
}

/// Builder for constructing basic blocks.
pub struct BlockBuilder {
    id: BlockId,
    instructions: Vec<IrInstr>,
    start_value_id: u32,
}

impl BlockBuilder {
    /// Get the block ID.
    pub fn id(&self) -> BlockId {
        self.id
    }

    /// Get the next value ID that would be allocated.
    fn next_value_id(&self) -> u32 {
        self.start_value_id + self.instructions.iter().filter_map(|i| i.result_value()).count() as u32
    }

    /// Allocate a new SSA value ID.
    fn alloc_value(&mut self) -> ValueId {
        let count = self.instructions.iter().filter_map(|i| i.result_value()).count() as u32;
        ValueId(self.start_value_id + count)
    }

    /// Emit a constant instruction.
    pub fn const_val(&mut self, value: IrConst, source: SourceLocation) -> ValueId {
        let result = self.alloc_value();
        self.instructions.push(IrInstr::Const {
            result,
            value,
            source,
        });
        result
    }

    /// Emit a binary operation.
    pub fn binop(
        &mut self,
        op: BinOp,
        lhs: ValueId,
        rhs: ValueId,
        source: SourceLocation,
    ) -> ValueId {
        let result = self.alloc_value();
        self.instructions.push(IrInstr::BinOp {
            result,
            op,
            lhs,
            rhs,
            source,
        });
        result
    }

    /// Emit a unary operation.
    pub fn unop(&mut self, op: UnOp, operand: ValueId, source: SourceLocation) -> ValueId {
        let result = self.alloc_value();
        self.instructions.push(IrInstr::UnOp {
            result,
            op,
            operand,
            source,
        });
        result
    }

    /// Emit a function call.
    pub fn call(
        &mut self,
        func: FunctionId,
        args: Vec<ValueId>,
        source: SourceLocation,
    ) -> ValueId {
        let result = self.alloc_value();
        self.instructions.push(IrInstr::Call {
            result: Some(result),
            func,
            args,
            source,
        });
        result
    }

    /// Emit a void function call (no return value).
    pub fn call_void(&mut self, func: FunctionId, args: Vec<ValueId>, source: SourceLocation) {
        self.instructions.push(IrInstr::Call {
            result: None,
            func,
            args,
            source,
        });
    }

    /// Emit a record construction.
    pub fn record(
        &mut self,
        field_names: Arc<[InternedString]>,
        field_values: Vec<ValueId>,
        source: SourceLocation,
    ) -> ValueId {
        let result = self.alloc_value();
        self.instructions.push(IrInstr::Record {
            result,
            field_names,
            field_values,
            source,
        });
        result
    }

    /// Emit a field access.
    pub fn field(
        &mut self,
        record: ValueId,
        field: InternedString,
        source: SourceLocation,
    ) -> ValueId {
        let result = self.alloc_value();
        self.instructions.push(IrInstr::Field {
            result,
            record,
            field,
            source,
        });
        result
    }

    /// Emit a tuple construction.
    pub fn tuple(&mut self, elements: Vec<ValueId>, source: SourceLocation) -> ValueId {
        let result = self.alloc_value();
        self.instructions.push(IrInstr::Tuple {
            result,
            elements,
            source,
        });
        result
    }

    /// Emit a phi node.
    pub fn phi(&mut self, incoming: Vec<(ValueId, BlockId)>, source: SourceLocation) -> ValueId {
        let result = self.alloc_value();
        self.instructions.push(IrInstr::Phi {
            result,
            incoming,
            source,
        });
        result
    }

    /// Complete the block with a return terminator.
    /// Returns the block and the next value ID to use.
    pub fn ret(self, value: Option<ValueId>, source: SourceLocation) -> (IrBlock, u32) {
        let next_value_id = self.next_value_id();
        (
            IrBlock {
                id: self.id,
                instructions: self.instructions,
                terminator: IrTerminator::Return { value, source },
            },
            next_value_id,
        )
    }

    /// Complete the block with a conditional branch.
    /// Returns the block and the next value ID to use.
    pub fn branch(
        self,
        cond: ValueId,
        then_block: BlockId,
        else_block: BlockId,
        source: SourceLocation,
    ) -> (IrBlock, u32) {
        let next_value_id = self.next_value_id();
        (
            IrBlock {
                id: self.id,
                instructions: self.instructions,
                terminator: IrTerminator::Branch {
                    cond,
                    then_block,
                    else_block,
                    source,
                },
            },
            next_value_id,
        )
    }

    /// Complete the block with an unconditional jump.
    /// Returns the block and the next value ID to use.
    pub fn jump(self, target: BlockId, source: SourceLocation) -> (IrBlock, u32) {
        let next_value_id = self.next_value_id();
        (
            IrBlock {
                id: self.id,
                instructions: self.instructions,
                terminator: IrTerminator::Jump { target, source },
            },
            next_value_id,
        )
    }
}
