//! Intermediate Representation (IR) for Cadenza
//!
//! This module defines a typed, SSA-like intermediate representation that serves as
//! the target for compilation and the input for optimization and code generation.
//!
//! # Design Principles
//!
//! - **Target-independent**: Can be lowered to JavaScript, Rust, WASM, or other targets
//! - **SSA form**: Single Static Assignment for easier optimization
//! - **Typed**: All values have explicit types
//! - **Simple**: Easy to generate, analyze, and transform
//!
//! # IR Structure
//!
//! The IR is organized as:
//! - [`IrModule`]: Top-level container with functions and exports
//! - [`IrFunction`]: Function with parameters, blocks, and return type
//! - [`IrBlock`]: Basic block containing instructions
//! - [`IrInstr`]: Individual instructions (binop, call, etc.)
//!
//! # Example
//!
//! ```text
//! function @add_one(%x: Integer) -> Integer {
//!   block_0 (entry):
//!     %0 = const 1
//!     %1 = binop Add %x %0
//!     ret %1
//! }
//! ```

pub mod optimize;

use crate::{InternedString, Type};
use crate::unit::Dimension;
use std::fmt;

/// A unique identifier for values in SSA form
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ValueId(pub u32);

impl fmt::Display for ValueId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "%{}", self.0)
    }
}

/// A unique identifier for basic blocks
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub u32);

impl fmt::Display for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "block_{}", self.0)
    }
}

/// A unique identifier for functions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FunctionId(pub u32);

impl fmt::Display for FunctionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@fn_{}", self.0)
    }
}

/// IR instruction
#[derive(Debug, Clone)]
pub enum IrInstr {
    /// Load a constant value
    /// %result = const <value>
    Const {
        result: ValueId,
        value: IrConst,
    },

    /// Binary operation
    /// %result = binop <op> %lhs %rhs
    BinOp {
        result: ValueId,
        op: BinOp,
        lhs: ValueId,
        rhs: ValueId,
    },

    /// Unary operation
    /// %result = unop <op> %operand
    UnOp {
        result: ValueId,
        op: UnOp,
        operand: ValueId,
    },

    /// Function call
    /// %result = call <func> (%arg1, %arg2, ...)
    Call {
        result: Option<ValueId>, // None for void returns
        func: FunctionId,
        args: Vec<ValueId>,
    },

    /// Create a record
    /// %result = record { field1: %val1, field2: %val2, ... }
    Record {
        result: ValueId,
        fields: Vec<(InternedString, ValueId)>,
    },

    /// Field access
    /// %result = field %record .field_name
    Field {
        result: ValueId,
        record: ValueId,
        field: InternedString,
    },

    /// Create a list/tuple
    /// %result = tuple (%elem1, %elem2, ...)
    Tuple {
        result: ValueId,
        elements: Vec<ValueId>,
    },

    /// Phi node for SSA (join point for values from different blocks)
    /// %result = phi [%val1 from <block1>], [%val2 from <block2>], ...
    Phi {
        result: ValueId,
        incoming: Vec<(ValueId, BlockId)>,
    },
}

impl IrInstr {
    /// Get the result value produced by this instruction, if any
    pub fn result_value(&self) -> Option<ValueId> {
        match self {
            IrInstr::Const { result, .. }
            | IrInstr::BinOp { result, .. }
            | IrInstr::UnOp { result, .. }
            | IrInstr::Record { result, .. }
            | IrInstr::Field { result, .. }
            | IrInstr::Tuple { result, .. }
            | IrInstr::Phi { result, .. } => Some(*result),
            IrInstr::Call { result, .. } => *result,
        }
    }
}

/// Constants in IR
#[derive(Debug, Clone)]
pub enum IrConst {
    Nil,
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(InternedString),
    /// Quantity with dimension (e.g., 5.0 meters)
    Quantity { value: f64, dimension: Dimension },
}

/// Binary operators
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Rem,

    // Comparison
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,

    // Logical
    And,
    Or,

    // Bitwise (future)
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BinOp::Add => write!(f, "Add"),
            BinOp::Sub => write!(f, "Sub"),
            BinOp::Mul => write!(f, "Mul"),
            BinOp::Div => write!(f, "Div"),
            BinOp::Rem => write!(f, "Rem"),
            BinOp::Eq => write!(f, "Eq"),
            BinOp::Ne => write!(f, "Ne"),
            BinOp::Lt => write!(f, "Lt"),
            BinOp::Le => write!(f, "Le"),
            BinOp::Gt => write!(f, "Gt"),
            BinOp::Ge => write!(f, "Ge"),
            BinOp::And => write!(f, "And"),
            BinOp::Or => write!(f, "Or"),
            BinOp::BitAnd => write!(f, "BitAnd"),
            BinOp::BitOr => write!(f, "BitOr"),
            BinOp::BitXor => write!(f, "BitXor"),
            BinOp::Shl => write!(f, "Shl"),
            BinOp::Shr => write!(f, "Shr"),
        }
    }
}

/// Unary operators
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,    // Numeric negation
    Not,    // Logical not
    BitNot, // Bitwise not (future)
}

impl fmt::Display for UnOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnOp::Neg => write!(f, "Neg"),
            UnOp::Not => write!(f, "Not"),
            UnOp::BitNot => write!(f, "BitNot"),
        }
    }
}

/// Block terminator (control flow instructions)
#[derive(Debug, Clone)]
pub enum IrTerminator {
    /// Conditional branch
    /// br %cond, then: <then_block>, else: <else_block>
    Branch {
        cond: ValueId,
        then_block: BlockId,
        else_block: BlockId,
    },

    /// Unconditional jump
    /// jmp <target_block>
    Jump { target: BlockId },

    /// Return from function
    /// ret %value
    Return { value: Option<ValueId> },
}

/// Basic block - sequence of instructions with a single entry and exit
#[derive(Debug, Clone)]
pub struct IrBlock {
    pub id: BlockId,
    pub instructions: Vec<IrInstr>,
    pub terminator: IrTerminator,
}

impl IrBlock {
    pub fn new(id: BlockId) -> Self {
        Self {
            id,
            instructions: Vec::new(),
            terminator: IrTerminator::Return { value: None },
        }
    }
}

/// Function parameter
#[derive(Debug, Clone)]
pub struct IrParam {
    pub name: InternedString,
    pub ty: Type,
    pub value_id: ValueId, // SSA value for this parameter
}

/// Function in IR
#[derive(Debug, Clone)]
pub struct IrFunction {
    pub id: FunctionId,
    pub name: InternedString,
    pub params: Vec<IrParam>,
    pub return_ty: Type,
    pub blocks: Vec<IrBlock>,
    pub entry_block: BlockId,
}

impl IrFunction {
    pub fn new(id: FunctionId, name: InternedString, return_ty: Type) -> Self {
        Self {
            id,
            name,
            params: Vec::new(),
            return_ty,
            blocks: Vec::new(),
            entry_block: BlockId(0),
        }
    }
}

/// Exported items from a module
#[derive(Debug, Clone)]
pub struct IrExport {
    pub name: InternedString,
    pub kind: IrExportKind,
}

#[derive(Debug, Clone)]
pub enum IrExportKind {
    Function(FunctionId),
    Constant(ValueId),
}

/// Complete IR module
#[derive(Debug, Clone)]
pub struct IrModule {
    pub functions: Vec<IrFunction>,
    pub exports: Vec<IrExport>,
}

impl IrModule {
    pub fn new() -> Self {
        Self {
            functions: Vec::new(),
            exports: Vec::new(),
        }
    }
}

impl Default for IrModule {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for constructing IR
pub struct IrBuilder {
    module: IrModule,
    current_function: Option<FunctionId>,
    current_block: Option<BlockId>,
    next_value_id: u32,
    next_block_id: u32,
    next_function_id: u32,
}

impl IrBuilder {
    pub fn new() -> Self {
        Self {
            module: IrModule::new(),
            current_function: None,
            current_block: None,
            next_value_id: 0,
            next_block_id: 0,
            next_function_id: 0,
        }
    }

    /// Allocate a new SSA value ID
    pub fn new_value(&mut self) -> ValueId {
        let id = ValueId(self.next_value_id);
        self.next_value_id += 1;
        id
    }

    /// Allocate a new block ID
    pub fn new_block(&mut self) -> BlockId {
        let id = BlockId(self.next_block_id);
        self.next_block_id += 1;
        id
    }

    /// Allocate a new function ID
    pub fn new_function_id(&mut self) -> FunctionId {
        let id = FunctionId(self.next_function_id);
        self.next_function_id += 1;
        id
    }

    /// Start building a new function
    pub fn start_function(&mut self, name: InternedString, return_ty: Type) -> FunctionId {
        let id = self.new_function_id();
        let function = IrFunction::new(id, name, return_ty);
        self.module.functions.push(function);
        self.current_function = Some(id);
        id
    }

    /// Get the current function being built
    fn current_function_mut(&mut self) -> &mut IrFunction {
        let func_id = self
            .current_function
            .expect("No current function in IrBuilder");
        self.module
            .functions
            .iter_mut()
            .find(|f| f.id == func_id)
            .expect("Current function not found")
    }

    /// Start a new basic block in the current function
    pub fn start_block(&mut self) -> BlockId {
        let block_id = self.new_block();
        let block = IrBlock::new(block_id);
        self.current_function_mut().blocks.push(block);
        self.current_block = Some(block_id);
        block_id
    }

    /// Get the current block being built
    fn current_block_mut(&mut self) -> &mut IrBlock {
        let block_id = self.current_block.expect("No current block in IrBuilder");
        let func = self.current_function_mut();
        func.blocks
            .iter_mut()
            .find(|b| b.id == block_id)
            .expect("Current block not found")
    }

    /// Emit an instruction in the current block
    pub fn emit(&mut self, instr: IrInstr) {
        self.current_block_mut().instructions.push(instr);
    }

    /// Set the terminator for the current block
    pub fn terminate(&mut self, terminator: IrTerminator) {
        self.current_block_mut().terminator = terminator;
    }

    /// Finish building and return the IR module
    pub fn finish(self) -> IrModule {
        self.module
    }
}

impl Default for IrBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ir_builder_basic() {
        let mut builder = IrBuilder::new();

        // Create a simple function: fn add_one(x) -> x + 1
        let func_name = InternedString::new("add_one");
        let func_id = builder.start_function(func_name, Type::Integer);

        // Entry block
        let entry = builder.start_block();
        assert_eq!(entry, BlockId(0));

        // %0 = const 1
        let one = builder.new_value();
        builder.emit(IrInstr::Const {
            result: one,
            value: IrConst::Integer(1),
        });

        // %1 = param x
        let x = builder.new_value();

        // %2 = binop Add %x %0
        let result = builder.new_value();
        builder.emit(IrInstr::BinOp {
            result,
            op: BinOp::Add,
            lhs: x,
            rhs: one,
        });

        // ret %2
        builder.terminate(IrTerminator::Return {
            value: Some(result),
        });

        let module = builder.finish();

        assert_eq!(module.functions.len(), 1);
        assert_eq!(module.functions[0].id, func_id);
        assert_eq!(module.functions[0].blocks.len(), 1);
        assert_eq!(module.functions[0].blocks[0].instructions.len(), 2);
    }
}
