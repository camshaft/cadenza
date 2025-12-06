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

use super::{BinOp, BlockId, IrBlock, IrConst, IrFunction, IrInstr, IrModule, IrTerminator, UnOp, ValueId};
use crate::Type;
use std::collections::{HashMap, HashSet};
use wasm_encoder::*;

/// Tracks where SSA values are located in WASM (parameters, locals, or stack).
struct ValueLocationTracker {
    /// Maps SSA ValueId to WASM local index.
    /// Function parameters are locals 0..N, other values get locals N+1..
    value_to_local: HashMap<ValueId, u32>,
    /// Next available local index for allocating new locals.
    next_local_idx: u32,
}

impl ValueLocationTracker {
    /// Create a new tracker for a function with the given parameters.
    fn new(params: &[super::IrParam]) -> Self {
        let num_params = params.len() as u32;
        let mut value_to_local = HashMap::new();

        // Map parameter ValueIds to their local indices
        for (idx, param) in params.iter().enumerate() {
            value_to_local.insert(param.value_id, idx as u32);
        }

        Self {
            value_to_local,
            next_local_idx: num_params,
        }
    }

    /// Allocate a local for a ValueId if it doesn't already have one.
    /// Returns the local index.
    fn allocate_local(&mut self, value_id: ValueId) -> u32 {
        if let Some(&local_idx) = self.value_to_local.get(&value_id) {
            local_idx
        } else {
            let local_idx = self.next_local_idx;
            self.next_local_idx += 1;
            self.value_to_local.insert(value_id, local_idx);
            local_idx
        }
    }

    /// Get the local index for a ValueId. Returns None if the value isn't in a local.
    fn get_local(&self, value_id: ValueId) -> Option<u32> {
        self.value_to_local.get(&value_id).copied()
    }
}

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
        self.types.ty().function(param_types, results);

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
        // Create a value location tracker for this function
        let mut tracker = ValueLocationTracker::new(&func.params);

        // First pass: analyze which values need locals
        // For now, we'll allocate a local for every SSA value (simple but correct)
        for block in &func.blocks {
            for instr in &block.instructions {
                if let Some(result) = instr.result_value() {
                    tracker.allocate_local(result);
                }
            }
        }

        // Determine local types for non-parameter locals
        let mut local_types = vec![];
        // We need to know the type of each allocated local
        // For now, we'll collect them from instructions
        let mut value_types: HashMap<ValueId, &Type> = HashMap::new();
        for param in &func.params {
            value_types.insert(param.value_id, &param.ty);
        }
        for block in &func.blocks {
            for instr in &block.instructions {
                if let Some(result) = instr.result_value() {
                    let ty = match instr {
                        IrInstr::Const { ty, .. }
                        | IrInstr::BinOp { ty, .. }
                        | IrInstr::UnOp { ty, .. }
                        | IrInstr::Call { ty, .. }
                        | IrInstr::Record { ty, .. }
                        | IrInstr::Field { ty, .. }
                        | IrInstr::Tuple { ty, .. }
                        | IrInstr::Phi { ty, .. } => ty,
                    };
                    value_types.insert(result, ty);
                }
            }
        }

        // Build the locals list (excluding parameters)
        // Create a reverse mapping for efficient lookup
        let mut local_to_value: HashMap<u32, ValueId> = HashMap::new();
        for (&value_id, &local_idx) in &tracker.value_to_local {
            local_to_value.insert(local_idx, value_id);
        }

        for local_idx in func.params.len() as u32..tracker.next_local_idx {
            let value_id = local_to_value
                .get(&local_idx)
                .ok_or_else(|| format!("Local {} has no corresponding ValueId", local_idx))?;

            let ty = value_types
                .get(value_id)
                .ok_or_else(|| format!("No type found for value {}", value_id))?;
            let wasm_ty = self.type_to_wasm(ty)?;
            local_types.push((1, wasm_ty));
        }

        let mut function = Function::new(local_types);

        // Generate code for all blocks with proper control flow
        self.generate_function_body(&mut function, func, &tracker)?;

        self.code.function(&function);
        Ok(())
    }

    /// Generate the complete function body with proper control flow.
    /// 
    /// This method analyzes the IR's basic block structure and generates
    /// structured WASM control flow (if/else/block/loop).
    fn generate_function_body(
        &self,
        func: &mut Function,
        ir_func: &IrFunction,
        tracker: &ValueLocationTracker,
    ) -> Result<(), String> {
        // Build a map of block IDs to blocks for quick lookup
        let mut blocks: HashMap<BlockId, &IrBlock> = HashMap::new();
        for block in &ir_func.blocks {
            blocks.insert(block.id, block);
        }

        // Start with the entry block (not in a control structure)
        self.generate_block_recursive(func, ir_func.entry_block, &blocks, tracker, &mut HashSet::new(), false)?;
        
        Ok(())
    }

    /// Generate code for a block and its successors recursively.
    /// 
    /// The `visited` set tracks which blocks have been generated to avoid infinite loops.
    /// The `in_control_structure` flag indicates if this block is nested within an if-else.
    /// 
    /// # Limitations
    /// 
    /// This implementation works for simple control flow patterns (if-then-else where
    /// branches return directly). It has limitations:
    /// - The visited set prevents generating blocks reached through multiple paths,
    ///   which means merge blocks with phi nodes won't work correctly yet.
    /// - Complex control flow graphs may require block restructuring algorithms
    ///   (e.g., Relooper) to map to WASM's structured control flow.
    fn generate_block_recursive(
        &self,
        func: &mut Function,
        block_id: BlockId,
        blocks: &HashMap<BlockId, &IrBlock>,
        tracker: &ValueLocationTracker,
        visited: &mut HashSet<BlockId>,
        in_control_structure: bool,
    ) -> Result<(), String> {
        // Check if we've already generated this block
        // NOTE: This prevents handling merge blocks with multiple predecessors.
        // A more sophisticated implementation would need to use br/br_if for jumps.
        if visited.contains(&block_id) {
            return Ok(());
        }
        visited.insert(block_id);

        let block = blocks
            .get(&block_id)
            .ok_or_else(|| format!("Block {} not found", block_id))?;

        // Generate instructions for this block
        for (idx, instr) in block.instructions.iter().enumerate() {
            // Check for tail call optimization
            let is_last_instr = idx == block.instructions.len() - 1;
            let can_tail_call = is_last_instr
                && matches!(instr, IrInstr::Call { .. })
                && matches!(&block.terminator, IrTerminator::Return { .. })
                && !in_control_structure; // Don't use tail call in nested structures

            if can_tail_call {
                if let IrInstr::Call {
                    result,
                    func: func_id,
                    args,
                    ..
                } = instr
                {
                    // Check if return value matches
                    let ret_value_matches = match &block.terminator {
                        IrTerminator::Return { value, .. } => {
                            (result.is_none() && value.is_none())
                                || (result.is_some() && result.as_ref() == value.as_ref())
                        }
                        _ => false,
                    };

                    if ret_value_matches {
                        self.generate_tail_call(func, *func_id, args, tracker)?;
                        return Ok(()); // Tail call ends the block
                    }
                }
            }

            // Generate the instruction normally
            self.generate_instruction(func, instr, tracker)?;
        }

        // Generate terminator
        match &block.terminator {
            IrTerminator::Return { value, .. } => {
                if let Some(value_id) = value {
                    let local_idx = tracker
                        .get_local(*value_id)
                        .ok_or_else(|| format!("No local for return value {}", value_id))?;
                    func.instruction(&Instruction::LocalGet(local_idx));
                }
                // Only emit End if we're not in a control structure (if-else)
                if !in_control_structure {
                    func.instruction(&Instruction::End);
                }
            }
            IrTerminator::Branch {
                cond,
                then_block,
                else_block,
                ..
            } => {
                // Load condition
                let cond_local = tracker
                    .get_local(*cond)
                    .ok_or_else(|| format!("No local for branch condition {}", cond))?;
                func.instruction(&Instruction::LocalGet(cond_local));

                // Try to detect if-then-else-merge pattern for structured control flow
                // For now, generate a simple if-else structure and recurse into branches
                // Note: This is a simplified implementation that may not handle all cases
                
                // Emit if instruction (result type is based on the blocks' return)
                // For simplicity, use empty type (no result on stack from if)
                func.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));

                // Generate then block (nested in control structure)
                self.generate_block_recursive(func, *then_block, blocks, tracker, visited, true)?;

                // Emit else
                func.instruction(&Instruction::Else);

                // Generate else block (nested in control structure)
                self.generate_block_recursive(func, *else_block, blocks, tracker, visited, true)?;

                // End if-else
                func.instruction(&Instruction::End);
                
                // If we're at function level (not nested in another control structure),
                // and both branches returned, we still need to close the function body
                if !in_control_structure {
                    func.instruction(&Instruction::End);
                }
            }
            IrTerminator::Jump { target, .. } => {
                // For now, just recurse into the target block
                // In a more sophisticated implementation, this would use WASM's br instruction
                self.generate_block_recursive(func, *target, blocks, tracker, visited, in_control_structure)?;
            }
        }

        Ok(())
    }

    /// Generate code for an IR instruction.
    fn generate_instruction(
        &self,
        func: &mut Function,
        instr: &IrInstr,
        tracker: &ValueLocationTracker,
    ) -> Result<(), String> {
        match instr {
            IrInstr::Const {
                result, value, ty, ..
            } => {
                // Generate the constant and store it in the result local
                self.generate_const(func, value, ty)?;
                let local_idx = tracker
                    .get_local(*result)
                    .ok_or_else(|| format!("No local for value {}", result))?;
                func.instruction(&Instruction::LocalSet(local_idx));
            }
            IrInstr::BinOp {
                result,
                op,
                lhs,
                rhs,
                ty,
                ..
            } => {
                self.generate_binop(func, *op, *lhs, *rhs, ty, tracker)?;
                // Store result in local
                let local_idx = tracker
                    .get_local(*result)
                    .ok_or_else(|| format!("No local for value {}", result))?;
                func.instruction(&Instruction::LocalSet(local_idx));
            }
            IrInstr::UnOp {
                result,
                op,
                operand,
                ty,
                ..
            } => {
                self.generate_unop(func, *op, *operand, ty, tracker)?;
                // Store result in local
                let local_idx = tracker
                    .get_local(*result)
                    .ok_or_else(|| format!("No local for value {}", result))?;
                func.instruction(&Instruction::LocalSet(local_idx));
            }
            IrInstr::Call {
                result,
                func: func_id,
                args,
                ..
            } => {
                // Load arguments onto stack in order
                for &arg_value_id in args {
                    let arg_local = tracker
                        .get_local(arg_value_id)
                        .ok_or_else(|| format!("No local for argument value {}", arg_value_id))?;
                    func.instruction(&Instruction::LocalGet(arg_local));
                }

                // Get the WASM function index for this IR function
                let func_idx = self
                    .function_indices
                    .get(func_id)
                    .copied()
                    .ok_or_else(|| format!("Unknown function ID in call: {:?}", func_id))?;

                // Emit call instruction
                func.instruction(&Instruction::Call(func_idx));

                // Store result if function returns a value
                if let Some(result_id) = result {
                    let local_idx = tracker
                        .get_local(*result_id)
                        .ok_or_else(|| format!("No local for call result {}", result_id))?;
                    func.instruction(&Instruction::LocalSet(local_idx));
                }
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
    fn generate_const(
        &self,
        func: &mut Function,
        value: &IrConst,
        _ty: &Type,
    ) -> Result<(), String> {
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
        lhs: ValueId,
        rhs: ValueId,
        ty: &Type,
        tracker: &ValueLocationTracker,
    ) -> Result<(), String> {
        // Load LHS from local
        let lhs_local = tracker
            .get_local(lhs)
            .ok_or_else(|| format!("No local for LHS value {}", lhs))?;
        func.instruction(&Instruction::LocalGet(lhs_local));

        // Load RHS from local
        let rhs_local = tracker
            .get_local(rhs)
            .ok_or_else(|| format!("No local for RHS value {}", rhs))?;
        func.instruction(&Instruction::LocalGet(rhs_local));

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
        operand: ValueId,
        ty: &Type,
        tracker: &ValueLocationTracker,
    ) -> Result<(), String> {
        match op {
            UnOp::Neg => match ty {
                Type::Integer => {
                    // Integer negation: compute 0 - operand
                    func.instruction(&Instruction::I64Const(0));
                    // Load operand from local
                    let operand_local = tracker
                        .get_local(operand)
                        .ok_or_else(|| format!("No local for operand value {}", operand))?;
                    func.instruction(&Instruction::LocalGet(operand_local));
                    // Subtract: 0 - operand
                    func.instruction(&Instruction::I64Sub);
                }
                Type::Float => {
                    // Float negation has a dedicated instruction
                    // Load operand from local
                    let operand_local = tracker
                        .get_local(operand)
                        .ok_or_else(|| format!("No local for operand value {}", operand))?;
                    func.instruction(&Instruction::LocalGet(operand_local));
                    func.instruction(&Instruction::F64Neg);
                }
                _ => return Err(format!("Neg not supported for type {:?}", ty)),
            },
            UnOp::Not => {
                // Logical not: operand == 0
                // Load operand from local
                let operand_local = tracker
                    .get_local(operand)
                    .ok_or_else(|| format!("No local for operand value {}", operand))?;
                func.instruction(&Instruction::LocalGet(operand_local));
                // Operand is on stack as i32 (boolean)
                func.instruction(&Instruction::I32Eqz);
            }
            UnOp::BitNot => {
                return Err("Bitwise not not yet implemented".to_string());
            }
        }

        Ok(())
    }

    /// Generate code for a tail call (return_call instruction).
    /// This optimizes the pattern of call + return into a single return_call.
    fn generate_tail_call(
        &self,
        func: &mut Function,
        func_id: super::FunctionId,
        args: &[ValueId],
        tracker: &ValueLocationTracker,
    ) -> Result<(), String> {
        // Load arguments onto stack in order (same as regular call)
        for &arg_value_id in args {
            let arg_local = tracker
                .get_local(arg_value_id)
                .ok_or_else(|| format!("No local for argument value {}", arg_value_id))?;
            func.instruction(&Instruction::LocalGet(arg_local));
        }

        // Get the WASM function index for this IR function
        let func_idx = self
            .function_indices
            .get(&func_id)
            .copied()
            .ok_or_else(|| format!("Unknown function ID in tail call: {:?}", func_id))?;

        // Emit return_call instruction - this performs call + return in one instruction
        func.instruction(&Instruction::ReturnCall(func_idx));

        // End the function body (needed to close the implicit function block)
        // Note: Even though return_call is a terminating instruction, the function's
        // implicit block still needs to be closed with End
        func.instruction(&Instruction::End);

        Ok(())
    }

    /// Convert an IR type to a WASM value type.
    fn type_to_wasm(&self, ty: &Type) -> Result<ValType, String> {
        match ty {
            Type::Nil => Ok(ValType::I32),  // Represent nil as i32 0
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
    use crate::{InternedString, ir::*};

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
        assert!(
            result.is_ok(),
            "Failed to generate WASM: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_type_to_wasm() {
        let codegen = WasmCodegen::new();

        assert_eq!(codegen.type_to_wasm(&Type::Integer).unwrap(), ValType::I64);
        assert_eq!(codegen.type_to_wasm(&Type::Float).unwrap(), ValType::F64);
        assert_eq!(codegen.type_to_wasm(&Type::Bool).unwrap(), ValType::I32);
        assert!(codegen.type_to_wasm(&Type::String).is_err());
    }

    #[test]
    fn test_generate_function_with_call() {
        // Create a helper function: fn add(a, b) = a + b
        let add_func = IrFunction {
            id: FunctionId(0),
            name: InternedString::new("add"),
            params: vec![
                IrParam {
                    name: InternedString::new("a"),
                    value_id: ValueId(0),
                    ty: Type::Integer,
                },
                IrParam {
                    name: InternedString::new("b"),
                    value_id: ValueId(1),
                    ty: Type::Integer,
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

        // Create a caller function: fn compute(x) = add(x, 5)
        let compute_func = IrFunction {
            id: FunctionId(1),
            name: InternedString::new("compute"),
            params: vec![IrParam {
                name: InternedString::new("x"),
                value_id: ValueId(0),
                ty: Type::Integer,
            }],
            return_ty: Type::Integer,
            blocks: vec![IrBlock {
                id: BlockId(0),
                instructions: vec![
                    IrInstr::Const {
                        result: ValueId(1),
                        ty: Type::Integer,
                        value: IrConst::Integer(5),
                        source: dummy_source(),
                    },
                    IrInstr::Call {
                        result: Some(ValueId(2)),
                        ty: Type::Integer,
                        func: FunctionId(0), // Call to add
                        args: vec![ValueId(0), ValueId(1)],
                        source: dummy_source(),
                    },
                ],
                terminator: IrTerminator::Return {
                    value: Some(ValueId(2)),
                    source: dummy_source(),
                },
            }],
            entry_block: BlockId(0),
        };

        let module = IrModule {
            functions: vec![add_func, compute_func],
            exports: vec![],
        };

        let mut codegen = WasmCodegen::new();
        let result = codegen.generate(&module);
        assert!(
            result.is_ok(),
            "Failed to generate WASM with function call: {:?}",
            result.err()
        );

        // Convert to WAT to verify the structure
        let binary = result.unwrap();
        let wat = binary_to_wat(&binary);
        assert!(wat.is_ok(), "Failed to convert to WAT: {:?}", wat.err());

        let wat_text = wat.unwrap();
        println!("Generated WAT:\n{}", wat_text);

        // Verify the WAT contains the expected elements
        assert!(wat_text.contains("call 0")); // Call to function 0 (add)
    }

    #[test]
    fn test_generate_recursive_function() {
        // Create a recursive factorial function: fn fact(n) = if n <= 1 then 1 else n * fact(n - 1)
        // For now, just test the structure with a simple recursive call: fn countdown(n) = countdown(n - 1)
        // (without control flow, this would infinite loop, but we're just testing codegen)

        let countdown_func = IrFunction {
            id: FunctionId(0),
            name: InternedString::new("countdown"),
            params: vec![IrParam {
                name: InternedString::new("n"),
                value_id: ValueId(0),
                ty: Type::Integer,
            }],
            return_ty: Type::Integer,
            blocks: vec![IrBlock {
                id: BlockId(0),
                instructions: vec![
                    IrInstr::Const {
                        result: ValueId(1),
                        ty: Type::Integer,
                        value: IrConst::Integer(1),
                        source: dummy_source(),
                    },
                    IrInstr::BinOp {
                        result: ValueId(2),
                        ty: Type::Integer,
                        op: BinOp::Sub,
                        lhs: ValueId(0),
                        rhs: ValueId(1),
                        source: dummy_source(),
                    },
                    IrInstr::Call {
                        result: Some(ValueId(3)),
                        ty: Type::Integer,
                        func: FunctionId(0), // Recursive call to self
                        args: vec![ValueId(2)],
                        source: dummy_source(),
                    },
                ],
                terminator: IrTerminator::Return {
                    value: Some(ValueId(3)),
                    source: dummy_source(),
                },
            }],
            entry_block: BlockId(0),
        };

        let module = IrModule {
            functions: vec![countdown_func],
            exports: vec![],
        };

        let mut codegen = WasmCodegen::new();
        let result = codegen.generate(&module);
        assert!(
            result.is_ok(),
            "Failed to generate WASM with recursive call: {:?}",
            result.err()
        );

        // Convert to WAT to verify the structure
        let binary = result.unwrap();
        let wat = binary_to_wat(&binary);
        assert!(wat.is_ok(), "Failed to convert to WAT: {:?}", wat.err());

        let wat_text = wat.unwrap();
        println!("Generated WAT for recursive function:\n{}", wat_text);

        // Verify the WAT contains the recursive call
        assert!(wat_text.contains("call 0")); // Recursive call to function 0
    }

    #[test]
    fn test_generate_function_with_branch() {
        // Create a simpler function with conditional that returns directly from each branch
        // fn sign(x) = if x < 0 then -1 else 1
        // This avoids phi nodes by having each branch return directly

        let sign_func = IrFunction {
            id: FunctionId(0),
            name: InternedString::new("sign"),
            params: vec![IrParam {
                name: InternedString::new("x"),
                value_id: ValueId(0),
                ty: Type::Integer,
            }],
            return_ty: Type::Integer,
            blocks: vec![
                // Entry block: compare x < 0
                IrBlock {
                    id: BlockId(0),
                    instructions: vec![
                        IrInstr::Const {
                            result: ValueId(1),
                            ty: Type::Integer,
                            value: IrConst::Integer(0),
                            source: dummy_source(),
                        },
                        IrInstr::BinOp {
                            result: ValueId(2),
                            ty: Type::Integer, // Use operand type for comparison
                            op: BinOp::Lt,
                            lhs: ValueId(0),
                            rhs: ValueId(1),
                            source: dummy_source(),
                        },
                    ],
                    terminator: IrTerminator::Branch {
                        cond: ValueId(2),
                        then_block: BlockId(1),
                        else_block: BlockId(2),
                        source: dummy_source(),
                    },
                },
                // Then block: return -1
                IrBlock {
                    id: BlockId(1),
                    instructions: vec![IrInstr::Const {
                        result: ValueId(3),
                        ty: Type::Integer,
                        value: IrConst::Integer(-1),
                        source: dummy_source(),
                    }],
                    terminator: IrTerminator::Return {
                        value: Some(ValueId(3)),
                        source: dummy_source(),
                    },
                },
                // Else block: return 1
                IrBlock {
                    id: BlockId(2),
                    instructions: vec![IrInstr::Const {
                        result: ValueId(4),
                        ty: Type::Integer,
                        value: IrConst::Integer(1),
                        source: dummy_source(),
                    }],
                    terminator: IrTerminator::Return {
                        value: Some(ValueId(4)),
                        source: dummy_source(),
                    },
                },
            ],
            entry_block: BlockId(0),
        };

        let module = IrModule {
            functions: vec![sign_func],
            exports: vec![],
        };

        let mut codegen = WasmCodegen::new();
        let result = codegen.generate(&module);
        assert!(
            result.is_ok(),
            "Failed to generate WASM with branch: {:?}",
            result.err()
        );

        // Convert to WAT to verify the structure
        let binary = result.unwrap();
        let wat = binary_to_wat(&binary);
        assert!(wat.is_ok(), "Failed to convert to WAT: {:?}", wat.err());

        let wat_text = wat.unwrap();
        println!("Generated WAT for function with branch:\n{}", wat_text);

        // Verify the WAT contains conditional structures
        assert!(wat_text.contains("if")); // WebAssembly if instruction
    }
}
