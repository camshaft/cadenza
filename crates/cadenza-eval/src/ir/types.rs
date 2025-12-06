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
        write!(f, "%{}", self.0)
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
        write!(f, "@func_{}", self.0)
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
    /// %result = const <value>
    Const {
        result: ValueId,
        value: IrConst,
        source: SourceLocation,
    },

    /// Binary operation
    /// %result = binop <op> %lhs %rhs
    BinOp {
        result: ValueId,
        op: BinOp,
        lhs: ValueId,
        rhs: ValueId,
        source: SourceLocation,
    },

    /// Unary operation
    /// %result = unop <op> %operand
    UnOp {
        result: ValueId,
        op: UnOp,
        operand: ValueId,
        source: SourceLocation,
    },

    /// Function call
    /// %result = call <func> (%arg1, %arg2, ...)
    Call {
        result: Option<ValueId>, // None for void returns
        func: FunctionId,
        args: Vec<ValueId>,
        source: SourceLocation,
    },

    /// Create a record
    /// %result = record { field1: %val1, field2: %val2, ... }
    /// Field names stored separately from values for efficient cloning
    /// field_values[i] corresponds to field_names[i]
    Record {
        result: ValueId,
        field_names: Arc<[InternedString]>, // Shared across all instances of this record type
        field_values: Vec<ValueId>,         // Parallel array: values[i] is value for names[i]
        source: SourceLocation,
    },

    /// Field access
    /// %result = field %record .field_name
    Field {
        result: ValueId,
        record: ValueId,
        field: InternedString,
        source: SourceLocation,
    },

    /// Create a list/tuple
    /// %result = tuple (%elem1, %elem2, ...)
    Tuple {
        result: ValueId,
        elements: Vec<ValueId>,
        source: SourceLocation,
    },

    /// Phi node for SSA (join point for values from different blocks)
    /// %result = phi [%val1 from <block1>], [%val2 from <block2>], ...
    Phi {
        result: ValueId,
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
            IrInstr::Const { result, value, .. } => {
                write!(f, "{} = const {}", result, value)
            }
            IrInstr::BinOp {
                result,
                op,
                lhs,
                rhs,
                ..
            } => {
                write!(f, "{} = {} {} {}", result, op, lhs, rhs)
            }
            IrInstr::UnOp {
                result,
                op,
                operand,
                ..
            } => {
                write!(f, "{} = {} {}", result, op, operand)
            }
            IrInstr::Call {
                result, func, args, ..
            } => {
                if let Some(res) = result {
                    write!(f, "{} = call {}(", res, func)?;
                } else {
                    write!(f, "call {}(", func)?;
                }
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", arg)?;
                }
                write!(f, ")")
            }
            IrInstr::Record {
                result,
                field_names,
                field_values,
                ..
            } => {
                write!(f, "{} = record {{ ", result)?;
                for (i, (name, value)) in field_names.iter().zip(field_values.iter()).enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", name, value)?;
                }
                write!(f, " }}")
            }
            IrInstr::Field {
                result,
                record,
                field,
                ..
            } => {
                write!(f, "{} = field {}.{}", result, record, field)
            }
            IrInstr::Tuple {
                result, elements, ..
            } => {
                write!(f, "{} = tuple (", result)?;
                for (i, elem) in elements.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", elem)?;
                }
                write!(f, ")")
            }
            IrInstr::Phi {
                result, incoming, ..
            } => {
                write!(f, "{} = phi [", result)?;
                for (i, (val, block)) in incoming.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{} from {}", val, block)?;
                }
                write!(f, "]")
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
                write!(f, "br {}, then: {}, else: {}", cond, then_block, else_block)
            }
            IrTerminator::Jump { target, .. } => {
                write!(f, "jmp {}", target)
            }
            IrTerminator::Return { value, .. } => {
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
        writeln!(f, "{}:", self.id)?;
        for instr in &self.instructions {
            writeln!(f, "    {}", instr)?;
        }
        writeln!(f, "    {}", self.terminator)
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
        write!(f, "function {}(", self.name)?;
        for (i, param) in self.params.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", param)?;
        }
        writeln!(f, ") -> {} {{", self.return_ty)?;

        for block in &self.blocks {
            write!(f, "{}", block)?;
        }

        writeln!(f, "}}")
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
                write!(f, "export {} as function {}", self.name, func_id)
            }
            IrExportKind::Constant(val_id) => {
                write!(f, "export {} as constant {}", self.name, val_id)
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
        writeln!(f, "; IR Module")?;
        writeln!(f)?;

        for func in &self.functions {
            writeln!(f, "{}", func)?;
        }

        if !self.exports.is_empty() {
            writeln!(f, "; Exports")?;
            for export in &self.exports {
                writeln!(f, "{}", export)?;
            }
        }

        Ok(())
    }
}
