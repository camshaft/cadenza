//! IR Generator - converts evaluated values to IR.
//!
//! This module generates target-independent IR from Cadenza's evaluated values.
//! The IR is in SSA form and can be used for optimization passes and code generation.
//!
//! Note: IR generation happens after evaluation, not directly from AST. This means
//! we work with Values rather than AST nodes, making the transformation simpler.

use super::{
    BinOp as IrBinOp, BlockBuilder, BlockId, FunctionBuilder, FunctionId, IrBlock, IrBuilder,
    IrConst, SourceLocation, ValueId,
};
use crate::{
    diagnostic::{Diagnostic, Result},
    env::Env,
    interner::InternedString,
    special_form,
    typeinfer::{InferType, TypeEnv, TypeInferencer},
    value::{Type, UserFunction, Value},
};
use cadenza_syntax::ast::Expr;
use std::collections::HashMap;

/// Context for IR generation from AST.
///
/// Tracks SSA values, variable bindings, and types during IR generation.
pub struct IrGenContext<'a> {
    /// Maps variable names to their SSA value IDs.
    variables: HashMap<InternedString, ValueId>,
    /// Type environment for type inference.
    type_env: TypeEnv,
    /// Reference to the evaluator environment for looking up special forms.
    env: &'a Env,
}

impl<'a> IrGenContext<'a> {
    /// Create a new IR generation context.
    pub(crate) fn new(env: &'a Env) -> Self {
        Self {
            variables: HashMap::new(),
            type_env: TypeEnv::new(),
            env,
        }
    }

    /// Bind a variable name to an SSA value and add its type to the environment.
    pub fn bind_var(&mut self, name: InternedString, value: ValueId, ty: &InferType) {
        self.variables.insert(name, value);
        self.type_env.insert(name, ty.clone());
    }

    /// Look up a variable binding.
    pub fn lookup_var(&self, name: InternedString) -> Option<ValueId> {
        self.variables.get(&name).copied()
    }

    /// Get the type environment.
    pub fn type_env(&self) -> &TypeEnv {
        &self.type_env
    }

    /// Get a mutable reference to the type environment.
    pub fn type_env_mut(&mut self) -> &mut TypeEnv {
        &mut self.type_env
    }

    /// Get a reference to the evaluator environment.
    pub fn env(&self) -> &Env {
        self.env
    }
}

/// State for IR generation with support for multiple basic blocks.
///
/// This structure manages the function builder and tracks completed blocks
/// during IR generation. It allows special forms to create and manage
/// multiple basic blocks for control flow.
pub struct IrGenState<'a> {
    /// The function builder that owns all blocks.
    func_builder: &'a mut FunctionBuilder,
    /// The current block being built.
    pub(crate) current_block: Option<BlockBuilder>,
}

impl<'a> IrGenState<'a> {
    /// Create a new IR generation state with an initial block.
    fn new(func_builder: &'a mut FunctionBuilder, initial_block: BlockBuilder) -> Self {
        Self {
            func_builder,
            current_block: Some(initial_block),
        }
    }

    /// Get a mutable reference to the current block.
    ///
    /// # Panics
    ///
    /// Panics if there is no current block (e.g., after it has been terminated).
    pub fn current_block(&mut self) -> &mut BlockBuilder {
        self.current_block
            .as_mut()
            .expect("No current block available")
    }

    /// Create a new block and return its ID.
    ///
    /// The new block is not set as the current block.
    pub fn create_block(&mut self) -> (BlockId, BlockBuilder) {
        let block = self.func_builder.block();
        let id = block.id();
        (id, block)
    }

    /// Allocate a block ID without creating the block yet.
    ///
    /// This is useful for control flow where you need block IDs before the blocks are ready.
    pub fn alloc_block_id(&mut self) -> BlockId {
        self.func_builder.alloc_block_id()
    }

    /// Create a block with a specific ID.
    ///
    /// The block ID must have been allocated with `alloc_block_id`.
    pub fn create_block_with_id(&mut self, id: BlockId) -> BlockBuilder {
        self.func_builder.block_with_id(id)
    }

    /// Complete the current block with a terminator and add it to the function.
    ///
    /// After calling this, there is no current block.
    pub fn complete_current_block(&mut self, block: IrBlock, next_value_id: u32) {
        self.func_builder.add_block(block, next_value_id);
        self.current_block = None;
    }
}

/// IR Generator - converts evaluated values to IR.
pub struct IrGenerator {
    builder: IrBuilder,
    /// Maps function names to their function IDs for call generation.
    functions: HashMap<InternedString, FunctionId>,
    /// Type inferencer for determining expression types.
    type_inferencer: TypeInferencer,
}

impl IrGenerator {
    /// Create a new IR generator.
    pub fn new() -> Self {
        Self {
            builder: IrBuilder::new(),
            functions: HashMap::new(),
            type_inferencer: TypeInferencer::new(),
        }
    }

    /// Helper to create a fresh type variable for a parameter.
    fn create_param_type_var(&mut self, name: InternedString, ctx: &mut IrGenContext) {
        let type_var = self.type_inferencer.fresh_var();
        let infer_ty = InferType::Var(type_var);
        ctx.type_env_mut().insert(name, infer_ty);
    }

    /// Helper to infer a concrete type from an expression.
    ///
    /// Returns the concrete type if inference succeeds, otherwise Unknown.
    fn infer_concrete_type(&mut self, expr: &Expr, ctx: &IrGenContext) -> Type {
        self.type_inferencer
            .infer_expr(expr, ctx.type_env())
            .ok()
            .and_then(|ty| ty.to_concrete().ok())
            .unwrap_or(Type::Unknown)
    }

    /// Generate IR for a constant value.
    ///
    /// Converts a Cadenza `Value` to an `IrConst`.
    pub fn value_to_const(&self, value: &Value) -> Option<IrConst> {
        match value {
            Value::Nil => Some(IrConst::Nil),
            Value::Bool(b) => Some(IrConst::Bool(*b)),
            Value::Integer(i) => Some(IrConst::Integer(*i)),
            Value::Float(f) => Some(IrConst::Float(*f)),
            Value::String(s) => Some(IrConst::String(InternedString::new(s))),
            Value::Quantity {
                value,
                unit: _,
                dimension,
            } => {
                // Convert DerivedDimension to a single Dimension for IR
                // For now, use the base dimension if it exists, otherwise skip
                // TODO: Properly represent derived dimensions in IR
                if let Some((dim, _power)) = dimension.numerator.first() {
                    Some(IrConst::Quantity {
                        value: *value,
                        dimension: *dim,
                    })
                } else {
                    // No base dimension, treat as plain float
                    Some(IrConst::Float(*value))
                }
            }
            // Non-constant values
            _ => None,
        }
    }

    /// Generate IR for a user function.
    ///
    /// Converts a UserFunction value to an IR function.
    /// Returns the function ID on success.
    pub fn gen_function(&mut self, func: &UserFunction, env: &Env) -> Result<FunctionId> {
        let name = func.name;

        // Create a type environment for inference
        let mut ctx = IrGenContext::new(env);

        // Create type variables for parameters
        let param_types: Vec<(InternedString, Type)> = func
            .params
            .iter()
            .map(|p| {
                self.create_param_type_var(*p, &mut ctx);
                (*p, Type::Unknown) // We'll compute concrete type later if possible
            })
            .collect();

        // Infer the return type by inferring the body expression
        let return_ty = self.infer_concrete_type(&func.body, &ctx);

        let mut func_builder = self.builder.function(name, param_types.clone(), return_ty);
        // Register the function early so recursive calls can find it
        let func_id = func_builder.id();
        self.functions.insert(name, func_id);

        // Create the entry block
        let entry_block = func_builder.block();

        // Reset context for IR generation (parameters get bound as SSA values)
        let mut ctx = IrGenContext::new(env);

        // Bind parameters to their SSA values (v0, v1, ...)
        // Also add them to the type environment
        for (i, param_name) in func.params.iter().enumerate() {
            self.create_param_type_var(*param_name, &mut ctx);
            ctx.variables.insert(*param_name, ValueId(i as u32));
        }

        // Create state for multi-block generation
        let mut state = IrGenState::new(&mut func_builder, entry_block);

        // Generate IR for the function body
        let result = self.gen_expr_with_state(&func.body, &mut state, &mut ctx)?;

        // Complete the current block with a return
        let block = state
            .current_block
            .take()
            .expect("No current block available for function return");
        let (block_inst, next_val) = block.ret(Some(result), self.dummy_source());
        state.complete_current_block(block_inst, next_val);

        // Build the function
        let ir_func = func_builder.build();
        self.builder.add_function(ir_func);

        Ok(func_id)
    }

    /// Generate IR for an expression using IrGenState for multi-block support.
    ///
    /// Returns the SSA value ID for the result of the expression.
    fn gen_expr_with_state(
        &mut self,
        expr: &Expr,
        state: &mut IrGenState,
        ctx: &mut IrGenContext,
    ) -> Result<ValueId> {
        let source = self.dummy_source();

        match expr {
            Expr::Literal(lit) => self.gen_literal(lit, state.current_block(), source),
            Expr::Ident(ident) => self.gen_ident(ident, ctx),
            Expr::Apply(apply) => self.gen_apply_with_state(apply, state, ctx, source),
            _ => Err(Diagnostic::syntax(format!(
                "Unsupported expression type for IR generation: {:?}",
                expr
            ))),
        }
    }

    /// Generate IR for an expression (legacy single-block API).
    ///
    /// Returns the SSA value ID for the result of the expression.
    fn gen_expr(
        &mut self,
        expr: &Expr,
        block: &mut BlockBuilder,
        ctx: &mut IrGenContext,
    ) -> Result<ValueId> {
        let source = self.dummy_source();

        match expr {
            Expr::Literal(lit) => self.gen_literal(lit, block, source),
            Expr::Ident(ident) => self.gen_ident(ident, ctx),
            Expr::Apply(apply) => self.gen_apply(apply, block, ctx, source),
            _ => Err(Diagnostic::syntax(format!(
                "Unsupported expression type for IR generation: {:?}",
                expr
            ))),
        }
    }

    /// Generate IR for a literal expression.
    fn gen_literal(
        &mut self,
        lit: &cadenza_syntax::ast::Literal,
        block: &mut BlockBuilder,
        source: SourceLocation,
    ) -> Result<ValueId> {
        use cadenza_syntax::ast::LiteralValue;

        let value = lit
            .value()
            .ok_or_else(|| Diagnostic::syntax("Missing literal value"))?;

        let (const_val, ty) = match value {
            LiteralValue::Integer(i) => {
                let text = i.syntax().text();
                let value = text
                    .to_string()
                    .parse::<i64>()
                    .map_err(|e| Diagnostic::syntax(format!("Invalid integer literal: {}", e)))?;
                (IrConst::Integer(value), Type::Integer)
            }
            LiteralValue::Float(f) => {
                let text = f.syntax().text();
                let value = text
                    .to_string()
                    .parse::<f64>()
                    .map_err(|e| Diagnostic::syntax(format!("Invalid float literal: {}", e)))?;
                (IrConst::Float(value), Type::Float)
            }
            LiteralValue::String(s) => {
                let text = s.syntax().text().interned();
                (IrConst::String(text), Type::String)
            }
            LiteralValue::StringWithEscape(_) => {
                // For now, treat escaped strings as regular strings
                // TODO: Properly handle escape sequences
                return Err(Diagnostic::syntax(
                    "String with escapes not yet supported in IR generation",
                ));
            }
        };

        Ok(block.const_val(const_val, ty, source))
    }

    /// Generate IR for an identifier (variable reference).
    fn gen_ident(
        &mut self,
        ident: &cadenza_syntax::ast::Ident,
        ctx: &IrGenContext,
    ) -> Result<ValueId> {
        let name = ident.syntax().text().interned();
        ctx.lookup_var(name).ok_or_else(|| {
            Diagnostic::syntax(format!("Undefined variable in IR generation: {}", name))
        })
    }

    /// Try to generate IR for a special form by name.
    ///
    /// Returns Some(result) if the name matches a known special form, None otherwise.
    /// This dynamically looks up the special form from the environment.
    fn try_gen_special_form(
        &mut self,
        name: &str,
        apply: &cadenza_syntax::ast::Apply,
        block: &mut BlockBuilder,
        ctx: &mut IrGenContext,
        source: SourceLocation,
    ) -> Option<Result<ValueId>> {
        let args = apply.all_arguments();

        // Look up the name in the environment
        let name_interned: InternedString = name.into();
        let value = ctx.env().get(name_interned)?;

        // Check if it's a special form
        if let Value::SpecialForm(special_form) = value {
            // Create a mutable closure for generating sub-expressions
            let mut gen_expr_adapter =
                |expr: &Expr, block: &mut BlockBuilder, ctx: &mut IrGenContext| {
                    self.gen_expr(expr, block, ctx)
                };

            // Call the special form's IR generation function
            Some(special_form.build_ir(&args, block, ctx, source, &mut gen_expr_adapter))
        } else {
            // Not a special form
            None
        }
    }

    /// Try to generate IR for a special form by name (state-based version).
    ///
    /// Returns Some(result) if the name matches a known special form, None otherwise.
    /// This version supports multi-block generation through IrGenState.
    /// Dynamically looks up the special form from the environment.
    fn try_gen_special_form_with_state(
        &mut self,
        name: &str,
        apply: &cadenza_syntax::ast::Apply,
        state: &mut IrGenState,
        ctx: &mut IrGenContext,
        source: SourceLocation,
    ) -> Option<Result<ValueId>> {
        let args = apply.all_arguments();

        // Look up the name in the environment
        let name_interned: InternedString = name.into();
        let value = ctx.env().get(name_interned)?;

        // Check if it's a special form
        if let Value::SpecialForm(special_form) = value {
            // TODO: The hardcoded check for "match" is a temporary solution.
            // A better approach would be to extend the SpecialForm trait with a method
            // indicating whether the form needs multi-block generation, or to unify
            // the single-block and multi-block APIs so all special forms can use IrGenState.
            // For now, "match" is the only special form that needs multi-block support.
            if name == "match" {
                // Create a mutable closure for generating sub-expressions with state
                let mut gen_expr_adapter =
                    |expr: &Expr, state: &mut IrGenState, ctx: &mut IrGenContext| {
                        self.gen_expr_with_state(expr, state, ctx)
                    };

                return Some(special_form::match_form::ir_match_with_state(
                    &args,
                    state,
                    ctx,
                    source,
                    &mut gen_expr_adapter,
                ));
            }

            // For other special forms, use the single-block API
            let block = state.current_block();
            let mut gen_expr_adapter =
                |expr: &Expr, block: &mut BlockBuilder, ctx: &mut IrGenContext| {
                    self.gen_expr(expr, block, ctx)
                };

            Some(special_form.build_ir(&args, block, ctx, source, &mut gen_expr_adapter))
        } else {
            // Not a special form
            None
        }
    }

    /// Generate IR for an application using IrGenState (state-based version).
    fn gen_apply_with_state(
        &mut self,
        apply: &cadenza_syntax::ast::Apply,
        state: &mut IrGenState,
        ctx: &mut IrGenContext,
        source: SourceLocation,
    ) -> Result<ValueId> {
        // Check if this is an operator application
        let callee = apply
            .callee()
            .ok_or_else(|| Diagnostic::syntax("Missing callee in application"))?;

        // Extract the name/operator from the callee
        let name_opt = match &callee {
            Expr::Ident(ident) => {
                let text = ident.syntax().text().interned();
                Some(text.to_string())
            }
            Expr::Synthetic(syn) => {
                let id = syn.identifier();
                Some(id.to_string())
            }
            Expr::Op(op) => {
                let text = op.syntax().text().interned();
                Some(text.to_string())
            }
            _ => None,
        };

        if let Some(name) = name_opt {
            // Try to dispatch to a special form's IR generation
            if let Some(result) =
                self.try_gen_special_form_with_state(&name, apply, state, ctx, source)
            {
                return result;
            }

            // If it's an operator and not a special form, handle with hardcoded logic
            if let Expr::Op(_) = &callee {
                // Get the operator
                let ir_op = self.map_operator(&name)?;

                // Get arguments
                let args = apply.all_arguments();
                if args.len() != 2 {
                    return Err(Diagnostic::syntax(format!(
                        "Binary operator {} expects 2 arguments, got {}",
                        name,
                        args.len()
                    )));
                }

                // Generate IR for operands
                let lhs = self.gen_expr_with_state(&args[0], state, ctx)?;
                let rhs = self.gen_expr_with_state(&args[1], state, ctx)?;

                // Infer the type of the binary operation
                let inferred_ty = self.infer_concrete_type(&Expr::Apply(apply.clone()), ctx);

                // Emit binary operation with inferred type
                let block = state.current_block();
                return Ok(block.binop(ir_op, lhs, rhs, inferred_ty, source));
            }

            // Not an operator - try to look up as a function
            let func_name = InternedString::new(&name);
            let func_id = self.functions.get(&func_name).copied().ok_or_else(|| {
                Diagnostic::syntax(format!("Unknown function in IR generation: {}", func_name))
            })?;

            // Generate IR for arguments
            let args = apply.all_arguments();
            let arg_values: Result<Vec<ValueId>> = args
                .iter()
                .map(|arg| self.gen_expr_with_state(arg, state, ctx))
                .collect();
            let arg_values = arg_values?;

            // Infer the return type of the function call
            let inferred_ty = self.infer_concrete_type(&Expr::Apply(apply.clone()), ctx);

            // Emit call instruction with inferred return type
            let block = state.current_block();
            return Ok(block.call(func_id, arg_values, inferred_ty, source));
        }

        Err(Diagnostic::syntax(format!(
            "Unsupported callee type for IR generation: {:?}",
            callee
        )))
    }

    /// Generate IR for an application (function call or operator).
    fn gen_apply(
        &mut self,
        apply: &cadenza_syntax::ast::Apply,
        block: &mut BlockBuilder,
        ctx: &mut IrGenContext,
        source: SourceLocation,
    ) -> Result<ValueId> {
        // Check if this is an operator application
        let callee = apply
            .callee()
            .ok_or_else(|| Diagnostic::syntax("Missing callee in application"))?;

        // Extract the name/operator from the callee
        let name_opt = match &callee {
            Expr::Ident(ident) => {
                let text = ident.syntax().text().interned();
                Some(text.to_string())
            }
            Expr::Synthetic(syn) => {
                let id = syn.identifier();
                Some(id.to_string())
            }
            Expr::Op(op) => {
                let text = op.syntax().text().interned();
                Some(text.to_string())
            }
            _ => None,
        };

        if let Some(name) = name_opt {
            // Try to dispatch to a special form's IR generation
            if let Some(result) = self.try_gen_special_form(&name, apply, block, ctx, source) {
                return result;
            }

            // If it's an operator and not a special form, handle with hardcoded logic
            if let Expr::Op(_) = &callee {
                // Get the operator
                let ir_op = self.map_operator(&name)?;

                // Get arguments
                let args = apply.all_arguments();
                if args.len() != 2 {
                    return Err(Diagnostic::syntax(format!(
                        "Binary operator {} expects 2 arguments, got {}",
                        name,
                        args.len()
                    )));
                }

                // Generate IR for operands
                let lhs = self.gen_expr(&args[0], block, ctx)?;
                let rhs = self.gen_expr(&args[1], block, ctx)?;

                // Infer the type of the binary operation
                // Note: We need to clone Apply to wrap it as Expr for type inference
                let inferred_ty = self.infer_concrete_type(&Expr::Apply(apply.clone()), ctx);

                // Emit binary operation with inferred type
                return Ok(block.binop(ir_op, lhs, rhs, inferred_ty, source));
            }

            // Not an operator - try to look up as a function
            let func_name = InternedString::new(&name);
            let func_id = self.functions.get(&func_name).copied().ok_or_else(|| {
                Diagnostic::syntax(format!("Unknown function in IR generation: {}", func_name))
            })?;

            // Generate IR for arguments
            let args = apply.all_arguments();
            let arg_values: Result<Vec<ValueId>> = args
                .iter()
                .map(|arg| self.gen_expr(arg, block, ctx))
                .collect();
            let arg_values = arg_values?;

            // Infer the return type of the function call
            // Note: We need to clone Apply to wrap it as Expr for type inference
            let inferred_ty = self.infer_concrete_type(&Expr::Apply(apply.clone()), ctx);

            // Emit call instruction with inferred return type
            return Ok(block.call(func_id, arg_values, inferred_ty, source));
        }

        Err(Diagnostic::syntax(format!(
            "Unsupported callee type for IR generation: {:?}",
            callee
        )))
    }

    /// Map operator string to IR binary operator.
    fn map_operator(&self, op: &str) -> Result<IrBinOp> {
        match op {
            "+" => Ok(IrBinOp::Add),
            "-" => Ok(IrBinOp::Sub),
            "*" => Ok(IrBinOp::Mul),
            "/" => Ok(IrBinOp::Div),
            "%" => Ok(IrBinOp::Rem),
            "==" => Ok(IrBinOp::Eq),
            "!=" => Ok(IrBinOp::Ne),
            "<" => Ok(IrBinOp::Lt),
            "<=" => Ok(IrBinOp::Le),
            ">" => Ok(IrBinOp::Gt),
            ">=" => Ok(IrBinOp::Ge),
            "&&" => Ok(IrBinOp::And),
            "||" => Ok(IrBinOp::Or),
            _ => Err(Diagnostic::syntax(format!("Unsupported operator: {}", op))),
        }
    }

    /// Create a dummy source location.
    /// TODO: Extract actual file, line, column from expr.span()
    fn dummy_source(&self) -> SourceLocation {
        SourceLocation {
            file: InternedString::new("input"),
            line: 1,
            column: 0,
        }
    }

    /// Build and return the final IR module.
    pub fn build(self) -> super::IrModule {
        self.builder.build()
    }

    /// Get a reference to the underlying IrBuilder.
    pub fn builder(&self) -> &IrBuilder {
        &self.builder
    }

    /// Get a mutable reference to the underlying IrBuilder.
    pub fn builder_mut(&mut self) -> &mut IrBuilder {
        &mut self.builder
    }

    /// Check if a function with the given name already exists in the IR module.
    pub fn has_function(&self, name: InternedString) -> bool {
        self.builder
            .module()
            .functions
            .iter()
            .any(|f| f.name == name)
    }
}

impl Default for IrGenerator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::Env;
    use cadenza_syntax::parse::parse;

    #[test]
    fn test_value_to_const() {
        let generator = IrGenerator::new();

        assert_eq!(generator.value_to_const(&Value::Nil), Some(IrConst::Nil));
        assert_eq!(
            generator.value_to_const(&Value::Bool(true)),
            Some(IrConst::Bool(true))
        );
        assert_eq!(
            generator.value_to_const(&Value::Integer(42)),
            Some(IrConst::Integer(42))
        );
        assert_eq!(
            generator.value_to_const(&Value::Float(2.5)),
            Some(IrConst::Float(2.5))
        );

        let s = String::from("hello");
        let expected_s = InternedString::new("hello");
        assert_eq!(
            generator.value_to_const(&Value::String(s)),
            Some(IrConst::String(expected_s))
        );
    }

    #[test]
    fn test_gen_function_simple() {
        let mut generator = IrGenerator::new();
        let env = Env::with_standard_builtins();

        // Create a simple function: fn add(a, b) = a + b
        let source = "a + b";
        let parsed = parse(source);
        let ast = parsed.ast();
        let body = ast.items().next().expect("No expression in AST");

        let func = UserFunction {
            name: InternedString::new("add"),
            params: vec![InternedString::new("a"), InternedString::new("b")],
            body,
            captured_env: Env::new(),
        };

        let func_id = generator
            .gen_function(&func, &env)
            .expect("Failed to generate IR");

        // Build the module and check the output
        let module = generator.build();
        let ir_text = module.to_string();

        println!("Generated IR:\n{}", ir_text);

        // Verify the function was generated
        assert_eq!(module.functions.len(), 1);
        assert_eq!(module.functions[0].name, InternedString::new("add"));
        assert_eq!(module.functions[0].params.len(), 2);
        assert_eq!(func_id, module.functions[0].id);

        // Verify the IR contains expected elements
        assert!(ir_text.contains("fn add"));
        assert!(ir_text.contains("binop add v0 v1"));
    }

    #[test]
    fn test_gen_function_with_literal() {
        let mut generator = IrGenerator::new();
        let env = Env::with_standard_builtins();

        // Create a function that returns a constant: fn get_answer() = 42
        let source = "42";
        let parsed = parse(source);
        let ast = parsed.ast();
        let body = ast.items().next().expect("No expression in AST");

        let func = UserFunction {
            name: InternedString::new("get_answer"),
            params: vec![],
            body,
            captured_env: Env::new(),
        };

        let _func_id = generator
            .gen_function(&func, &env)
            .expect("Failed to generate IR");

        let module = generator.build();
        let ir_text = module.to_string();

        println!("Generated IR:\n{}", ir_text);

        // Verify the function was generated
        assert_eq!(module.functions.len(), 1);
        assert_eq!(module.functions[0].name, InternedString::new("get_answer"));
        assert_eq!(module.functions[0].params.len(), 0);

        // Verify the IR contains expected elements
        assert!(ir_text.contains("fn get_answer"));
        assert!(ir_text.contains("const 42"));
    }

    #[test]
    fn test_gen_function_complex() {
        let mut generator = IrGenerator::new();
        let env = Env::with_standard_builtins();

        // Create a more complex function: fn calc(x, y) = x * 2 + y
        let source = "x * 2 + y";
        let parsed = parse(source);
        let ast = parsed.ast();
        let body = ast.items().next().expect("No expression in AST");

        let func = UserFunction {
            name: InternedString::new("calc"),
            params: vec![InternedString::new("x"), InternedString::new("y")],
            body,
            captured_env: Env::new(),
        };

        let _func_id = generator
            .gen_function(&func, &env)
            .expect("Failed to generate IR");

        let module = generator.build();
        let ir_text = module.to_string();

        println!("Generated IR:\n{}", ir_text);

        // Verify the function was generated
        assert_eq!(module.functions.len(), 1);
        assert_eq!(module.functions[0].name, InternedString::new("calc"));

        // Verify the IR contains expected operations
        assert!(ir_text.contains("fn calc"));
        assert!(ir_text.contains("mul"));
        assert!(ir_text.contains("add"));
    }

    #[test]
    fn test_gen_function_call_simple() {
        let mut generator = IrGenerator::new();
        let env = Env::with_standard_builtins();

        // First, create a simple helper function: fn double(x) = x + x
        let source = "x + x";
        let parsed = parse(source);
        let ast = parsed.ast();
        let body = ast.items().next().expect("No expression in AST");

        let double_func = UserFunction {
            name: InternedString::new("double"),
            params: vec![InternedString::new("x")],
            body,
            captured_env: Env::new(),
        };

        let _double_id = generator
            .gen_function(&double_func, &env)
            .expect("Failed to generate double function");

        // Now create a function that calls double: fn quadruple(y) = double(double(y))
        let source2 = "double(double(y))";
        let parsed2 = parse(source2);
        let ast2 = parsed2.ast();
        let body2 = ast2.items().next().expect("No expression in AST");

        let quadruple_func = UserFunction {
            name: InternedString::new("quadruple"),
            params: vec![InternedString::new("y")],
            body: body2,
            captured_env: Env::new(),
        };

        let _quadruple_id = generator
            .gen_function(&quadruple_func, &env)
            .expect("Failed to generate quadruple function");

        let module = generator.build();
        let ir_text = module.to_string();

        // Verify both functions were generated
        assert_eq!(module.functions.len(), 2);
        assert_eq!(module.functions[0].name, InternedString::new("double"));
        assert_eq!(module.functions[1].name, InternedString::new("quadruple"));

        // Verify the IR contains expected elements
        assert!(ir_text.contains("fn double"));
        assert!(ir_text.contains("fn quadruple"));
        assert!(ir_text.contains("call func0")); // Call to double
    }

    #[test]
    fn test_gen_function_call_with_args() {
        let mut generator = IrGenerator::new();
        let env = Env::with_standard_builtins();

        // Create a function: fn add(a, b) = a + b
        let source = "a + b";
        let parsed = parse(source);
        let ast = parsed.ast();
        let body = ast.items().next().expect("No expression in AST");

        let add_func = UserFunction {
            name: InternedString::new("add"),
            params: vec![InternedString::new("a"), InternedString::new("b")],
            body,
            captured_env: Env::new(),
        };

        let _add_id = generator
            .gen_function(&add_func, &env)
            .expect("Failed to generate add function");

        // Create a function that calls add: fn compute(x, y) = add(x * 2, y + 1)
        let source2 = "add(x * 2, y + 1)";
        let parsed2 = parse(source2);
        let ast2 = parsed2.ast();
        let body2 = ast2.items().next().expect("No expression in AST");

        let compute_func = UserFunction {
            name: InternedString::new("compute"),
            params: vec![InternedString::new("x"), InternedString::new("y")],
            body: body2,
            captured_env: Env::new(),
        };

        let _compute_id = generator
            .gen_function(&compute_func, &env)
            .expect("Failed to generate compute function");

        let module = generator.build();
        let ir_text = module.to_string();

        // Verify both functions were generated
        assert_eq!(module.functions.len(), 2);

        // Verify the IR contains expected elements
        assert!(ir_text.contains("fn add"));
        assert!(ir_text.contains("fn compute"));
        assert!(ir_text.contains("binop mul")); // x * 2
        assert!(ir_text.contains("binop add")); // y + 1 and a + b
        assert!(ir_text.contains("call func0")); // Call to add
    }

    #[test]
    fn test_gen_function_call_recursive() {
        let mut generator = IrGenerator::new();
        let env = Env::with_standard_builtins();

        // Create a simple recursive function: fn countdown(n) = countdown(n - 1)
        // (This is an infinite recursion, but that's fine for IR generation testing)
        let source = "countdown(n - 1)";
        let parsed = parse(source);
        let ast = parsed.ast();
        let body = ast.items().next().expect("No expression in AST");

        let countdown_func = UserFunction {
            name: InternedString::new("countdown"),
            params: vec![InternedString::new("n")],
            body,
            captured_env: Env::new(),
        };

        let _countdown_id = generator
            .gen_function(&countdown_func, &env)
            .expect("Failed to generate countdown function");

        let module = generator.build();
        let ir_text = module.to_string();

        // Verify the function was generated
        assert_eq!(module.functions.len(), 1);
        assert_eq!(module.functions[0].name, InternedString::new("countdown"));

        // Verify the IR contains recursive call
        assert!(ir_text.contains("fn countdown"));
        assert!(ir_text.contains("call func0")); // Recursive call to itself
        assert!(ir_text.contains("binop sub")); // n - 1
    }

    #[test]
    fn test_gen_function_with_match() {
        let mut generator = IrGenerator::new();
        let env = Env::with_standard_builtins();

        // Create a function with a match expression: fn sign(x) = match x > 0 (true -> 1) (false -> 0)
        let source = "match x > 0 (true -> 1) (false -> 0)";
        let parsed = parse(source);
        let ast = parsed.ast();
        let body = ast.items().next().expect("No expression in AST");

        let func = UserFunction {
            name: InternedString::new("sign"),
            params: vec![InternedString::new("x")],
            body,
            captured_env: Env::new(),
        };

        let func_id = generator
            .gen_function(&func, &env)
            .expect("Failed to generate IR");

        // Build the module and check the output
        let module = generator.build();
        let ir_text = module.to_string();

        // Verify the function was generated
        assert_eq!(module.functions.len(), 1);
        assert_eq!(module.functions[0].name, InternedString::new("sign"));
        assert_eq!(module.functions[0].params.len(), 1);
        assert_eq!(func_id, module.functions[0].id);

        // Verify the IR contains expected elements for control flow
        assert!(ir_text.contains("fn sign"), "Should have sign function");
        assert!(ir_text.contains("br "), "Should have branch instruction");
        assert!(ir_text.contains("block_1"), "Should have then block");
        assert!(ir_text.contains("block_2"), "Should have else block");
        assert!(ir_text.contains("block_3"), "Should have merge block");
        assert!(ir_text.contains("phi"), "Should have phi node");
        assert!(ir_text.contains("jmp"), "Should have jump instructions");
    }
}
