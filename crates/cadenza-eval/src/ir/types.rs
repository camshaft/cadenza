//! Core types for the Cadenza IR.

use crate::{Dimension, InternedString, Type};
use std::sync::Arc;

/// Source location for tracking origins of IR nodes.
/// Used to generate source maps for JavaScript and accurate stack traces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceLocation {
    pub file: InternedString,
    pub line: u32,
    pub column: u32,
}

/// A unique identifier for values in SSA form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ValueId(pub u32);

impl std::fmt::Display for ValueId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "v{}", self.0)
    }
}

/// A unique identifier for basic blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub u32);

impl std::fmt::Display for BlockId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "block_{}", self.0)
    }
}

/// A unique identifier for functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FunctionId(pub u32);

impl std::fmt::Display for FunctionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "func{}", self.0)
    }
}

/// Constants in IR.
#[derive(Debug, Clone, PartialEq)]
pub enum IrConst {
    Nil,
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(InternedString),
    /// Quantity with dimension (e.g., 5.0 meters)
    Quantity {
        value: f64,
        dimension: Dimension,
    },
}

impl std::fmt::Display for IrConst {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IrConst::Nil => write!(f, "nil"),
            IrConst::Bool(b) => write!(f, "{}", b),
            IrConst::Integer(i) => write!(f, "{}", i),
            IrConst::Float(fl) => write!(f, "{}", fl),
            IrConst::String(s) => write!(f, "\"{}\"", s),
            // Note: Using Debug for Dimension since it doesn't implement Display
            IrConst::Quantity { value, dimension } => write!(f, "{}{:?}", value, dimension),
        }
    }
}

/// Binary operators.
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

impl std::fmt::Display for BinOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BinOp::Add => write!(f, "add"),
            BinOp::Sub => write!(f, "sub"),
            BinOp::Mul => write!(f, "mul"),
            BinOp::Div => write!(f, "div"),
            BinOp::Rem => write!(f, "rem"),
            BinOp::Eq => write!(f, "eq"),
            BinOp::Ne => write!(f, "ne"),
            BinOp::Lt => write!(f, "lt"),
            BinOp::Le => write!(f, "le"),
            BinOp::Gt => write!(f, "gt"),
            BinOp::Ge => write!(f, "ge"),
            BinOp::And => write!(f, "and"),
            BinOp::Or => write!(f, "or"),
            BinOp::BitAnd => write!(f, "bitand"),
            BinOp::BitOr => write!(f, "bitor"),
            BinOp::BitXor => write!(f, "bitxor"),
            BinOp::Shl => write!(f, "shl"),
            BinOp::Shr => write!(f, "shr"),
        }
    }
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,    // Numeric negation
    Not,    // Logical not
    BitNot, // Bitwise not (future)
}

impl std::fmt::Display for UnOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnOp::Neg => write!(f, "neg"),
            UnOp::Not => write!(f, "not"),
            UnOp::BitNot => write!(f, "bitnot"),
        }
    }
}

/// IR instruction.
#[derive(Debug, Clone, PartialEq)]
pub enum IrInstr {
    /// Load a constant value
    /// %result: ty = const <value>
    Const {
        result: ValueId,
        ty: Type,
        value: IrConst,
        source: SourceLocation,
    },

    /// Binary operation
    /// %result: ty = binop <op> %lhs %rhs
    BinOp {
        result: ValueId,
        ty: Type,
        op: BinOp,
        lhs: ValueId,
        rhs: ValueId,
        source: SourceLocation,
    },

    /// Unary operation
    /// %result: ty = unop <op> %operand
    UnOp {
        result: ValueId,
        ty: Type,
        op: UnOp,
        operand: ValueId,
        source: SourceLocation,
    },

    /// Function call
    /// %result: ty = call <func> (%arg1, %arg2, ...)
    Call {
        result: Option<ValueId>, // None for void returns
        ty: Type,
        func: FunctionId,
        args: Vec<ValueId>,
        source: SourceLocation,
    },

    /// Create a record
    /// %result: ty = record { field1: %val1, field2: %val2, ... }
    /// Field names stored separately from values for efficient cloning
    /// field_values[i] corresponds to field_names[i]
    Record {
        result: ValueId,
        ty: Type,
        field_names: Arc<[InternedString]>, // Shared across all instances of this record type
        field_values: Vec<ValueId>,         // Parallel array: values[i] is value for names[i]
        source: SourceLocation,
    },

    /// Field access
    /// %result: ty = field %record .field_name
    Field {
        result: ValueId,
        ty: Type,
        record: ValueId,
        field: InternedString,
        source: SourceLocation,
    },

    /// Create a list/tuple
    /// %result: ty = tuple (%elem1, %elem2, ...)
    Tuple {
        result: ValueId,
        ty: Type,
        elements: Vec<ValueId>,
        source: SourceLocation,
    },

    /// Phi node for SSA (join point for values from different blocks)
    /// %result: ty = phi [%val1 from <block1>], [%val2 from <block2>], ...
    Phi {
        result: ValueId,
        ty: Type,
        incoming: Vec<(ValueId, BlockId)>,
        source: SourceLocation,
    },
}

impl IrInstr {
    /// Get the result value ID if this instruction produces a value.
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

    /// Get the source location of this instruction.
    pub fn source(&self) -> &SourceLocation {
        match self {
            IrInstr::Const { source, .. }
            | IrInstr::BinOp { source, .. }
            | IrInstr::UnOp { source, .. }
            | IrInstr::Call { source, .. }
            | IrInstr::Record { source, .. }
            | IrInstr::Field { source, .. }
            | IrInstr::Tuple { source, .. }
            | IrInstr::Phi { source, .. } => source,
        }
    }
}

impl std::fmt::Display for IrInstr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IrInstr::Const {
                result, ty, value, ..
            } => {
                // For constants with type annotation: let v0: integer = const 42
                write!(f, "let {}: {} = const {}", result, ty, value)
            }
            IrInstr::BinOp {
                result,
                ty,
                op,
                lhs,
                rhs,
                ..
            } => {
                // Binary operations tagged with binop: let v2: integer = binop add v0 v1
                write!(f, "let {}: {} = binop {} {} {}", result, ty, op, lhs, rhs)
            }
            IrInstr::UnOp {
                result,
                ty,
                op,
                operand,
                ..
            } => {
                // Unary operations tagged with unop: let v1: integer = unop neg v0
                write!(f, "let {}: {} = unop {} {}", result, ty, op, operand)
            }
            IrInstr::Call {
                result,
                ty,
                func,
                args,
                ..
            } => {
                // Function calls with call prefix
                if let Some(res) = result {
                    write!(f, "let {}: {} = call {}", res, ty, func)?;
                } else {
                    write!(f, "call {}", func)?;
                }
                for arg in args.iter() {
                    write!(f, " {}", arg)?;
                }
                Ok(())
            }
            IrInstr::Record {
                result,
                ty,
                field_names,
                field_values,
                ..
            } => {
                // Records with record prefix
                write!(f, "let {}: {} = record {{ ", result, ty)?;
                for (i, (name, value)) in field_names.iter().zip(field_values.iter()).enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{} = {}", name, value)?;
                }
                write!(f, " }}")
            }
            IrInstr::Field {
                result,
                ty,
                record,
                field,
                ..
            } => {
                // Field access with field prefix: let v2: integer = field v1.field
                write!(f, "let {}: {} = field {}.{}", result, ty, record, field)
            }
            IrInstr::Tuple {
                result,
                ty,
                elements,
                ..
            } => {
                // Tuples/lists tagged with list
                write!(f, "let {}: {} = list [", result, ty)?;
                for (i, elem) in elements.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", elem)?;
                }
                write!(f, "]")
            }
            IrInstr::Phi {
                result,
                ty,
                incoming,
                ..
            } => {
                // Phi nodes as function calls: let v3: integer = phi v1 block_1 v2 block_2
                write!(f, "let {}: {} = phi", result, ty)?;
                for (val, block) in incoming.iter() {
                    write!(f, " {} {}", val, block)?;
                }
                Ok(())
            }
        }
    }
}

/// Block terminator (control flow instructions).
#[derive(Debug, Clone, PartialEq)]
pub enum IrTerminator {
    /// Conditional branch
    /// br %cond, then: <then_block>, else: <else_block>
    Branch {
        cond: ValueId,
        then_block: BlockId,
        else_block: BlockId,
        source: SourceLocation,
    },

    /// Unconditional jump
    /// jmp <target_block>
    Jump {
        target: BlockId,
        source: SourceLocation,
    },

    /// Return from function
    /// ret %value
    Return {
        value: Option<ValueId>, // None for void functions
        source: SourceLocation,
    },
}

impl std::fmt::Display for IrTerminator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IrTerminator::Branch {
                cond,
                then_block,
                else_block,
                ..
            } => {
                // Branch as function call with juxtaposition: br cond then_block else_block
                write!(f, "br {} {} {}", cond, then_block, else_block)
            }
            IrTerminator::Jump { target, .. } => {
                // Jump as function call: jmp target_block
                write!(f, "jmp {}", target)
            }
            IrTerminator::Return { value, .. } => {
                // Return with ret keyword
                if let Some(val) = value {
                    write!(f, "ret {}", val)
                } else {
                    write!(f, "ret")
                }
            }
        }
    }
}

/// Basic block - sequence of instructions.
#[derive(Debug, Clone, PartialEq)]
pub struct IrBlock {
    pub id: BlockId,
    pub instructions: Vec<IrInstr>,
    pub terminator: IrTerminator,
}

impl std::fmt::Display for IrBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Block as a call with indented content
        writeln!(f, "    block {} =", self.id)?;
        for instr in &self.instructions {
            writeln!(f, "        {}", instr)?;
        }
        // Terminator as the final expression
        writeln!(f, "        {}", self.terminator)
    }
}

/// Function parameter.
#[derive(Debug, Clone, PartialEq)]
pub struct IrParam {
    pub name: InternedString,
    pub ty: Type,
    pub value_id: ValueId, // SSA value for this parameter
}

impl std::fmt::Display for IrParam {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Parameter with type annotation: v0: integer
        write!(f, "{}: {}", self.value_id, self.ty)
    }
}

/// Function in IR.
#[derive(Debug, Clone, PartialEq)]
pub struct IrFunction {
    pub id: FunctionId,
    pub name: InternedString,
    pub params: Vec<IrParam>,
    pub return_ty: Type,
    pub blocks: Vec<IrBlock>,
    pub entry_block: BlockId,
}

impl std::fmt::Display for IrFunction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Type annotation as @t attribute
        write!(f, "@t")?;
        for param in &self.params {
            write!(f, " {}", param.ty)?;
        }
        writeln!(f, " -> {}", self.return_ty)?;

        // Function signature with parameter names
        write!(f, "fn {}", self.name)?;
        for param in &self.params {
            write!(f, " {}", param.name)?;
        }
        writeln!(f, " =")?;

        // All blocks explicitly named, no inlining
        for block in &self.blocks {
            write!(f, "{}", block)?;
        }

        Ok(())
    }
}

/// Exported items from a module.
#[derive(Debug, Clone, PartialEq)]
pub struct IrExport {
    pub name: InternedString,
    pub kind: IrExportKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IrExportKind {
    Function(FunctionId),
    Constant(ValueId),
}

impl std::fmt::Display for IrExport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            IrExportKind::Function(func_id) => {
                // Export as a comment
                write!(f, "# export {} as function {}", self.name, func_id)
            }
            IrExportKind::Constant(val_id) => {
                write!(f, "# export {} as constant {}", self.name, val_id)
            }
        }
    }
}

/// Complete IR module.
#[derive(Debug, Clone, PartialEq)]
pub struct IrModule {
    pub functions: Vec<IrFunction>,
    pub exports: Vec<IrExport>,
}

impl IrModule {
    /// Create a new empty IR module.
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

impl std::fmt::Display for IrModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "# IR Module")?;
        writeln!(f)?;

        for func in &self.functions {
            writeln!(f, "{}", func)?;
            writeln!(f)?;
        }

        if !self.exports.is_empty() {
            writeln!(f, "# Exports")?;
            for export in &self.exports {
                writeln!(f, "{}", export)?;
            }
        }

        Ok(())
    }
}
