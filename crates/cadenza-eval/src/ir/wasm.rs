//! WebAssembly code generation from IR.
//!
//! This module generates WebAssembly binary format from the Cadenza IR.
//! It uses the wasm-encoder crate for binary generation and wasmprinter
//! for converting to WAT (WebAssembly Text format) for debugging.
//!
//! The generated WASM uses:
//! - WasmGC (Garbage Collection) proposal
//! - Reference types
//! - Component Model (for future interop)

use super::{
    BinOp, IrBlock, IrConst, IrFunction, IrInstr, IrModule, IrTerminator, UnOp, ValueId,
};
use crate::Type;
use std::collections::HashMap;
use wasm_encoder::*;

/// WASM code generator for IR.
pub struct WasmCodegen {
    /// The WASM module being built.
    module: Module,
    /// Type section for function signatures.
    types: TypeSection,
    /// Function section for function type indices.
    functions: FunctionSection,
    /// Code section for function bodies.
    code: CodeSection,
    /// Export section.
    exports: ExportSection,
    /// Map from IR function IDs to WASM function indices.
    function_indices: HashMap<super::FunctionId, u32>,
    /// Counter for WASM function indices.
    next_function_index: u32,
}

impl WasmCodegen {
    /// Create a new WASM code generator.
    pub fn new() -> Self {
        Self {
            module: Module::new(),
            types: TypeSection::new(),
            functions: FunctionSection::new(),
            code: CodeSection::new(),
            exports: ExportSection::new(),
            function_indices: HashMap::new(),
            next_function_index: 0,
        }
    }

    /// Generate WASM binary from IR module.
    pub fn generate(&mut self, ir: &IrModule) -> Result<Vec<u8>, String> {
        // Generate type signatures for all functions
        for func in &ir.functions {
            self.add_function_type(func)?;
        }

        // Generate function bodies
        for func in &ir.functions {
            self.add_function_code(func)?;
        }

        // Generate exports
        for export in &ir.exports {
            match &export.kind {
                super::IrExportKind::Function(func_id) => {
                    if let Some(&func_idx) = self.function_indices.get(func_id) {
                        self.exports
                            .export(&export.name.to_string(), ExportKind::Func, func_idx);
                    }
                }
                super::IrExportKind::Constant(_) => {
                    // Constants are not exported as WASM exports directly
                    // They would need to be wrapped in getter functions
                }
            }
        }

        // Assemble the module in the correct section order
        // Order: Type, Import, Function, Table, Memory, Global, Export, Start, Element, DataCount, Code
        self.module.section(&self.types);
        self.module.section(&self.functions);
        self.module.section(&self.exports);
        self.module.section(&self.code);

        // Use std::mem::replace to move out the module
        let module = std::mem::replace(&mut self.module, Module::new());
        Ok(module.finish())
    }

    /// Add a function type signature.
    fn add_function_type(&mut self, func: &IrFunction) -> Result<(), String> {
        // Convert parameter types to WASM types
        let param_types: Vec<ValType> = func
            .params
            .iter()
            .map(|p| self.type_to_wasm(&p.ty))
            .collect::<Result<Vec<_>, _>>()?;

        // Convert return type to WASM type
        let return_type = self.type_to_wasm(&func.return_ty)?;
        let results = vec![return_type];

        // Add to type section
        let type_idx = self.types.len();
        self.types
            .ty()
            .function(param_types, results);

        // Add to function section
        self.functions.function(type_idx);

        // Record the function index
        self.function_indices
            .insert(func.id, self.next_function_index);
        self.next_function_index += 1;

        Ok(())
    }

    /// Add a function's code (body).
    fn add_function_code(&mut self, func: &IrFunction) -> Result<(), String> {
        let mut function = Function::new(vec![]); // No locals for now

        // Generate code for the entry block
        // For simplicity, we'll generate a linear sequence first
        // TODO: Handle multiple blocks and control flow properly
        for block in &func.blocks {
            self.generate_block(&mut function, block)?;
        }

        self.code.function(&function);
        Ok(())
    }

    /// Generate code for a basic block.
    fn generate_block(&self, func: &mut Function, block: &IrBlock) -> Result<(), String> {
        // Generate instructions
        for instr in &block.instructions {
            self.generate_instruction(func, instr)?;
        }

        // Generate terminator
        self.generate_terminator(func, &block.terminator)?;

        Ok(())
    }

    /// Generate code for an IR instruction.
    fn generate_instruction(&self, func: &mut Function, instr: &IrInstr) -> Result<(), String> {
        match instr {
            IrInstr::Const { value, ty, .. } => {
                self.generate_const(func, value, ty)?;
            }
            IrInstr::BinOp {
                op, lhs, rhs, ty, ..
            } => {
                // Note: In a proper implementation, we'd need to track the stack
                // and generate appropriate local.get instructions for lhs/rhs
                // For now, this is a placeholder
                self.generate_binop(func, *op, *lhs, *rhs, ty)?;
            }
            IrInstr::UnOp { op, operand, ty, .. } => {
                self.generate_unop(func, *op, *operand, ty)?;
            }
            IrInstr::Call { func: _, args, .. } => {
                // Load arguments (would need proper stack management)
                for _arg in args {
                    // Placeholder: would generate local.get or other load instructions
                }
                // Generate call instruction
                // func.instruction(&Instruction::Call(func_idx));
            }
            IrInstr::Record { .. } => {
                // Records would require struct types from GC proposal
                return Err("Record types not yet implemented for WASM".to_string());
            }
            IrInstr::Field { .. } => {
                return Err("Field access not yet implemented for WASM".to_string());
            }
            IrInstr::Tuple { .. } => {
                // Tuples could be represented as structs or arrays
                return Err("Tuple types not yet implemented for WASM".to_string());
            }
            IrInstr::Phi { .. } => {
                // Phi nodes are typically handled during SSA deconstruction
                return Err("Phi nodes should be eliminated before WASM codegen".to_string());
            }
        }
        Ok(())
    }

    /// Generate code for a constant.
    fn generate_const(&self, func: &mut Function, value: &IrConst, _ty: &Type) -> Result<(), String> {
        match value {
            IrConst::Nil => {
                // Nil could be represented as ref.null
                func.instruction(&Instruction::I32Const(0));
            }
            IrConst::Bool(b) => {
                func.instruction(&Instruction::I32Const(if *b { 1 } else { 0 }));
            }
            IrConst::Integer(i) => {
                func.instruction(&Instruction::I64Const(*i));
            }
            IrConst::Float(f) => {
                func.instruction(&Instruction::F64Const((*f).into()));
            }
            IrConst::String(_s) => {
                // Strings would require GC arrays or imports
                return Err("String constants not yet implemented for WASM".to_string());
            }
            IrConst::Quantity { value, .. } => {
                // Treat quantities as floats for now
                func.instruction(&Instruction::F64Const((*value).into()));
            }
        }
        Ok(())
    }

    /// Generate code for a binary operation.
    fn generate_binop(
        &self,
        func: &mut Function,
        op: BinOp,
        _lhs: ValueId,
        _rhs: ValueId,
        ty: &Type,
    ) -> Result<(), String> {
        // Note: In a real implementation, we'd first load lhs and rhs onto the stack
        // For now, we just generate the operation instruction

        // For unknown types, default to integer operations
        let effective_ty = if matches!(ty, Type::Unknown) {
            &Type::Integer
        } else {
            ty
        };

        match op {
            BinOp::Add => match effective_ty {
                Type::Integer => {
                    func.instruction(&Instruction::I64Add);
                }
                Type::Float => {
                    func.instruction(&Instruction::F64Add);
                }
                _ => return Err(format!("Add not supported for type {:?}", ty)),
            },
            BinOp::Sub => match effective_ty {
                Type::Integer => {
                    func.instruction(&Instruction::I64Sub);
                }
                Type::Float => {
                    func.instruction(&Instruction::F64Sub);
                }
                _ => return Err(format!("Sub not supported for type {:?}", ty)),
            },
            BinOp::Mul => match effective_ty {
                Type::Integer => {
                    func.instruction(&Instruction::I64Mul);
                }
                Type::Float => {
                    func.instruction(&Instruction::F64Mul);
                }
                _ => return Err(format!("Mul not supported for type {:?}", ty)),
            },
            BinOp::Div => match effective_ty {
                Type::Integer => {
                    func.instruction(&Instruction::I64DivS);
                }
                Type::Float => {
                    func.instruction(&Instruction::F64Div);
                }
                _ => return Err(format!("Div not supported for type {:?}", ty)),
            },
            BinOp::Rem => match effective_ty {
                Type::Integer => {
                    func.instruction(&Instruction::I64RemS);
                }
                _ => return Err(format!("Rem not supported for type {:?}", ty)),
            },
            BinOp::Eq => match effective_ty {
                Type::Integer => {
                    func.instruction(&Instruction::I64Eq);
                }
                Type::Float => {
                    func.instruction(&Instruction::F64Eq);
                }
                _ => return Err(format!("Eq not supported for type {:?}", ty)),
            },
            BinOp::Ne => match effective_ty {
                Type::Integer => {
                    func.instruction(&Instruction::I64Ne);
                }
                Type::Float => {
                    func.instruction(&Instruction::F64Ne);
                }
                _ => return Err(format!("Ne not supported for type {:?}", ty)),
            },
            BinOp::Lt => match effective_ty {
                Type::Integer => {
                    func.instruction(&Instruction::I64LtS);
                }
                Type::Float => {
                    func.instruction(&Instruction::F64Lt);
                }
                _ => return Err(format!("Lt not supported for type {:?}", ty)),
            },
            BinOp::Le => match effective_ty {
                Type::Integer => {
                    func.instruction(&Instruction::I64LeS);
                }
                Type::Float => {
                    func.instruction(&Instruction::F64Le);
                }
                _ => return Err(format!("Le not supported for type {:?}", ty)),
            },
            BinOp::Gt => match effective_ty {
                Type::Integer => {
                    func.instruction(&Instruction::I64GtS);
                }
                Type::Float => {
                    func.instruction(&Instruction::F64Gt);
                }
                _ => return Err(format!("Gt not supported for type {:?}", ty)),
            },
            BinOp::Ge => match effective_ty {
                Type::Integer => {
                    func.instruction(&Instruction::I64GeS);
                }
                Type::Float => {
                    func.instruction(&Instruction::F64Ge);
                }
                _ => return Err(format!("Ge not supported for type {:?}", ty)),
            },
            BinOp::And | BinOp::Or => {
                // Logical operations on booleans (represented as i32)
                return Err("Logical operations not yet implemented".to_string());
            }
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => {
                return Err("Bitwise operations not yet implemented".to_string());
            }
        }

        Ok(())
    }

    /// Generate code for a unary operation.
    fn generate_unop(
        &self,
        func: &mut Function,
        op: UnOp,
        _operand: ValueId,
        ty: &Type,
    ) -> Result<(), String> {
        // Note: In a real implementation, we'd first load operand onto the stack

        match op {
            UnOp::Neg => match ty {
                Type::Integer => {
                    // i64.const 0, operand, i64.sub
                    func.instruction(&Instruction::I64Const(0));
                    // Load operand here
                    func.instruction(&Instruction::I64Sub);
                }
                Type::Float => {
                    func.instruction(&Instruction::F64Neg);
                }
                _ => return Err(format!("Neg not supported for type {:?}", ty)),
            },
            UnOp::Not => {
                // Logical not: i32.eqz
                func.instruction(&Instruction::I32Eqz);
            }
            UnOp::BitNot => {
                return Err("Bitwise not not yet implemented".to_string());
            }
        }

        Ok(())
    }

    /// Generate code for a terminator.
    fn generate_terminator(&self, func: &mut Function, term: &IrTerminator) -> Result<(), String> {
        match term {
            IrTerminator::Return { value, .. } => {
                if value.is_some() {
                    // Load the return value (would need proper stack management)
                }
                func.instruction(&Instruction::End);
            }
            IrTerminator::Branch { .. } => {
                return Err("Branch not yet implemented".to_string());
            }
            IrTerminator::Jump { .. } => {
                return Err("Jump not yet implemented".to_string());
            }
        }
        Ok(())
    }

    /// Convert an IR type to a WASM value type.
    fn type_to_wasm(&self, ty: &Type) -> Result<ValType, String> {
        match ty {
            Type::Nil => Ok(ValType::I32), // Represent nil as i32 0
            Type::Bool => Ok(ValType::I32), // Represent bool as i32
            Type::Integer => Ok(ValType::I64),
            Type::Float => Ok(ValType::F64),
            Type::Symbol | Type::String => {
                // Strings would use externref or GC array types
                Err("String type not yet supported in WASM".to_string())
            }
            Type::Fn(_) => {
                // Function types would use funcref
                Err("Function types not yet supported in WASM".to_string())
            }
            Type::Record(_) => {
                // Records would use struct types from GC proposal
                Err("Record types not yet supported in WASM".to_string())
            }
            Type::Tuple(_) => {
                // Tuples could use struct types
                Err("Tuple types not yet supported in WASM".to_string())
            }
            Type::List(_) => {
                // Lists would use GC arrays
                Err("List types not yet supported in WASM".to_string())
            }
            Type::Enum(_) | Type::Union(_) => {
                // Union types would need runtime type tags
                Err("Union/Enum types not yet supported in WASM".to_string())
            }
            Type::Type => {
                // Type values are compile-time only
                Err("Type values are compile-time only".to_string())
            }
            Type::Unknown => {
                // Unknown types from incomplete type inference
                // Default to i64 for now as it handles most numeric operations
                Ok(ValType::I64)
            }
        }
    }
}

impl Default for WasmCodegen {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert WASM binary to WAT (WebAssembly Text format).
pub fn binary_to_wat(binary: &[u8]) -> Result<String, String> {
    wasmprinter::print_bytes(binary).map_err(|e| format!("Failed to convert to WAT: {}", e))
}

/// Generate WAT from IR module.
pub fn generate_wat(ir: &IrModule) -> Result<String, String> {
    let mut codegen = WasmCodegen::new();
    let binary = codegen.generate(ir)?;
    binary_to_wat(&binary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::*;
    use crate::InternedString;

    fn dummy_source() -> SourceLocation {
        SourceLocation {
            file: InternedString::new("test.cdz"),
            line: 1,
            column: 0,
        }
    }

    #[test]
    fn test_generate_simple_function() {
        // Create a simple function that returns 42
        let func = IrFunction {
            id: super::super::FunctionId(0),
            name: InternedString::new("get_answer"),
            params: vec![],
            return_ty: Type::Integer,
            blocks: vec![IrBlock {
                id: super::super::BlockId(0),
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
            entry_block: super::super::BlockId(0),
        };

        let module = IrModule {
            functions: vec![func],
            exports: vec![],
        };

        let mut codegen = WasmCodegen::new();
        let result = codegen.generate(&module);
        assert!(result.is_ok(), "Failed to generate WASM: {:?}", result.err());
    }

    #[test]
    fn test_type_to_wasm() {
        let codegen = WasmCodegen::new();

        assert_eq!(codegen.type_to_wasm(&Type::Integer).unwrap(), ValType::I64);
        assert_eq!(codegen.type_to_wasm(&Type::Float).unwrap(), ValType::F64);
        assert_eq!(codegen.type_to_wasm(&Type::Bool).unwrap(), ValType::I32);
        assert!(codegen.type_to_wasm(&Type::String).is_err());
    }
}
