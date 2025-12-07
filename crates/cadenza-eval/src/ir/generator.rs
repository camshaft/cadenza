//! IR Generator - converts evaluated values to IR.
//!
//! This module generates target-independent IR from Cadenza's evaluated values.
//! The IR is in SSA form and can be used for optimization passes and code generation.
//!
//! Note: IR generation happens after evaluation, not directly from AST. This means
//! we work with Values rather than AST nodes, making the transformation simpler.

use super::{
    BinOp as IrBinOp, BlockBuilder, BlockId, FunctionBuilder, FunctionId, IrBuilder, IrConst,
    SourceLocation, ValueId,
};
use crate::{
    interner::InternedString,
    typeinfer::{InferType, TypeEnv, TypeInferencer},
    value::{Type, UserFunction, Value},
};
use cadenza_syntax::ast::Expr;
use std::collections::HashMap;

/// Context for IR generation from AST.
///
/// Tracks SSA values, variable bindings, and types during IR generation.
struct IrGenContext {
    /// Maps variable names to their SSA value IDs.
    variables: HashMap<InternedString, ValueId>,
    /// Type environment for type inference.
    type_env: TypeEnv,
}

impl IrGenContext {
    /// Create a new IR generation context.
    fn new() -> Self {
        Self {
            variables: HashMap::new(),
            type_env: TypeEnv::new(),
        }
    }

    /// Bind a variable name to an SSA value and add its type to the environment.
    fn bind_var(&mut self, name: InternedString, value: ValueId, ty: &InferType) {
        self.variables.insert(name, value);
        self.type_env.insert(name, ty.clone());
    }

    /// Look up a variable binding.
    fn lookup_var(&self, name: InternedString) -> Option<ValueId> {
        self.variables.get(&name).copied()
    }

    /// Get the type environment.
    fn type_env(&self) -> &TypeEnv {
        &self.type_env
    }

    /// Get a mutable reference to the type environment.
    fn type_env_mut(&mut self) -> &mut TypeEnv {
        &mut self.type_env
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
    pub fn gen_function(&mut self, func: &UserFunction) -> Result<FunctionId, String> {
        let name = func.name;

        // Debug: Check if body is a control flow expression
        eprintln!("IR Gen: Generating function {}", name);
        eprintln!("IR Gen: Body is control flow? {}", self.is_control_flow_expr(&func.body));

        // Create a type environment for inference
        let mut ctx = IrGenContext::new();

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

        let mut func_builder = self.builder.function(name, param_types.clone(), return_ty.clone());
        // Register the function early so recursive calls can find it
        let func_id = func_builder.id();
        self.functions.insert(name, func_id);

        // Reset context for IR generation (parameters get bound as SSA values)
        let mut ctx = IrGenContext::new();

        // Bind parameters to their SSA values (v0, v1, ...)
        // Also add them to the type environment
        for (i, param_name) in func.params.iter().enumerate() {
            self.create_param_type_var(*param_name, &mut ctx);
            ctx.variables.insert(*param_name, ValueId(i as u32));
        }

        // Check if the body is a control flow expression (match)
        if self.is_control_flow_expr(&func.body) {
            // Generate multi-block control flow
            eprintln!("IR Gen: Using multi-block control flow");
            self.gen_function_with_control_flow(&mut func_builder, &func.body, &mut ctx, return_ty)?;
        } else {
            // Simple single-block generation
            eprintln!("IR Gen: Using single-block generation");
            let mut block = func_builder.block();
            let result = self.gen_expr(&func.body, &mut block, &mut ctx)?;
            let (block_inst, next_val) = block.ret(Some(result), self.dummy_source());
            func_builder.add_block(block_inst, next_val);
        }

        // Build the function
        let ir_func = func_builder.build();
        self.builder.add_function(ir_func);

        Ok(func_id)
    }

    /// Check if an expression requires control flow (multiple blocks).
    fn is_control_flow_expr(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Apply(apply) => {
                if let Some(callee) = apply.callee() {
                    match callee {
                        Expr::Ident(ident) => {
                            let text = ident.syntax().text().to_string();
                            if text == "match" {
                                return true;
                            }
                        }
                        Expr::Synthetic(syn) => {
                            // If it's a __block__, check the expressions inside
                            if syn.identifier() == "__block__" {
                                for arg in apply.all_arguments() {
                                    if self.is_control_flow_expr(&arg) {
                                        return true;
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                false
            }
            _ => false,
        }
    }

    /// Generate IR for a function body that contains control flow.
    fn gen_function_with_control_flow(
        &mut self,
        func_builder: &mut FunctionBuilder,
        body: &Expr,
        ctx: &mut IrGenContext,
        _return_ty: Type,
    ) -> Result<(), String> {
        eprintln!("IR Gen: gen_function_with_control_flow called");
        // For now, only support match expressions at the top level (or inside __block__)
        if let Expr::Apply(apply) = body {
            if let Some(Expr::Ident(ident)) = apply.callee() {
                let text = ident.syntax().text().to_string();
                eprintln!("IR Gen: Found Ident callee: {}", text);
                if text == "match" {
                    return self.gen_match_control_flow(func_builder, apply, ctx, _return_ty);
                }
            } else if let Some(Expr::Synthetic(syn)) = apply.callee() {
                eprintln!("IR Gen: Found Synthetic callee: {}", syn.identifier());
                // If it's a __block__, find the match expression inside
                if syn.identifier() == "__block__" {
                    for arg in apply.all_arguments() {
                        if let Expr::Apply(inner_apply) = arg {
                            if let Some(Expr::Ident(inner_ident)) = inner_apply.callee() {
                                if inner_ident.syntax().text().to_string() == "match" {
                                    eprintln!("IR Gen: Found match inside __block__");
                                    return self.gen_match_control_flow(func_builder, &inner_apply, ctx, _return_ty);
                                }
                            }
                        }
                    }
                }
            }
        }
        
        eprintln!("IR Gen: No match expression found");
        Err("Unsupported control flow expression in function body".to_string())
    }

    /// Generate IR for a match expression with control flow.
    ///
    /// Creates the following structure:
    /// - Entry block: evaluate condition, branch
    /// - Then block: evaluate true case, return
    /// - Else block: evaluate false case, return
    fn gen_match_control_flow(
        &mut self,
        func_builder: &mut FunctionBuilder,
        apply: &cadenza_syntax::ast::Apply,
        ctx: &mut IrGenContext,
        _return_ty: Type,
    ) -> Result<(), String> {
        eprintln!("IR Gen: gen_match_control_flow called");
        let args = apply.all_arguments();
        
        eprintln!("IR Gen: match has {} arguments", args.len());
        
        // Match expects: condition, (true -> then_expr), (false -> else_expr)
        if args.len() < 3 {
            eprintln!("IR Gen: Not enough arguments for match");
            return Err("match requires at least 3 arguments: condition and two arms".to_string());
        }

        let cond_expr = &args[0];
        eprintln!("IR Gen: Condition expr: {:?}", cond_expr);
        
        // Parse the arms - each should be (true/false -> expr)
        // Clone the expressions we need to avoid lifetime issues
        let mut true_expr: Option<Expr> = None;
        let mut false_expr: Option<Expr> = None;
        
        for arm in &args[1..] {
            eprintln!("IR Gen: Processing arm");
            if let Expr::Apply(arm_apply) = arm {
                if let Some(Expr::Op(op)) = arm_apply.callee() {
                    eprintln!("IR Gen: Arm is Apply with Op: {}", op.syntax().text());
                    if op.syntax().text() == "->" {
                        let arm_args = arm_apply.all_arguments();
                        if arm_args.len() == 2 {
                            if let Expr::Ident(pattern_ident) = &arm_args[0] {
                                let pattern = pattern_ident.syntax().text().to_string();
                                eprintln!("IR Gen: Pattern: {}", pattern);
                                match pattern.as_str() {
                                    "true" => {
                                        true_expr = Some(arm_args[1].clone());
                                        eprintln!("IR Gen: Found true arm");
                                    }
                                    "false" => {
                                        false_expr = Some(arm_args[1].clone());
                                        eprintln!("IR Gen: Found false arm");
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }
        
        eprintln!("IR Gen: true_expr present: {}, false_expr present: {}", true_expr.is_some(), false_expr.is_some());
        
        let true_expr = true_expr.ok_or("match missing true arm")?;
        let false_expr = false_expr.ok_or("match missing false arm")?;

        // Create entry block and evaluate condition
        let mut entry_block = func_builder.block();
        let cond_value = self.gen_expr(cond_expr, &mut entry_block, ctx)?;
        
        // Allocate block IDs for then and else blocks
        let then_block_id = BlockId(func_builder.next_block_id);
        let else_block_id = BlockId(func_builder.next_block_id + 1);
        
        // Complete entry block with branch
        let (entry_block_ir, next_val) = entry_block.branch(
            cond_value,
            then_block_id,
            else_block_id,
            self.dummy_source(),
        );
        func_builder.add_block(entry_block_ir, next_val);
        
        // Create then block
        let mut then_block = func_builder.block();
        let then_value = self.gen_expr(&true_expr, &mut then_block, ctx)?;
        
        // Then block returns directly
        let (then_block_ir, next_val) = then_block.ret(Some(then_value), self.dummy_source());
        func_builder.add_block(then_block_ir, next_val);
        
        // Create else block
        let mut else_block = func_builder.block();
        let else_value = self.gen_expr(&false_expr, &mut else_block, ctx)?;
        
        // Else block also returns directly
        let (else_block_ir, _next_val) = else_block.ret(Some(else_value), self.dummy_source());
        func_builder.add_block(else_block_ir, _next_val);
        
        // Note: We're not creating a merge block with phi nodes yet.
        // That would be needed if we want to use the result of the match expression
        // in further computation. For now, each branch returns directly.
        
        Ok(())
    }

    /// Generate IR for an expression.
    ///
    /// Returns the SSA value ID for the result of the expression.
    fn gen_expr(
        &mut self,
        expr: &Expr,
        block: &mut BlockBuilder,
        ctx: &mut IrGenContext,
    ) -> Result<ValueId, String> {
        let source = self.dummy_source();

        match expr {
            Expr::Literal(lit) => self.gen_literal(lit, block, source),
            Expr::Ident(ident) => self.gen_ident(ident, ctx),
            Expr::Apply(apply) => self.gen_apply(apply, block, ctx, source),
            _ => Err(format!(
                "Unsupported expression type for IR generation: {:?}",
                expr
            )),
        }
    }

    /// Generate IR for a literal expression.
    fn gen_literal(
        &mut self,
        lit: &cadenza_syntax::ast::Literal,
        block: &mut BlockBuilder,
        source: SourceLocation,
    ) -> Result<ValueId, String> {
        use cadenza_syntax::ast::LiteralValue;

        let value = lit.value().ok_or("Missing literal value")?;

        let (const_val, ty) = match value {
            LiteralValue::Integer(i) => {
                let text = i.syntax().text();
                let value = text.to_string().parse::<i64>().map_err(|e| e.to_string())?;
                (IrConst::Integer(value), Type::Integer)
            }
            LiteralValue::Float(f) => {
                let text = f.syntax().text();
                let value = text.to_string().parse::<f64>().map_err(|e| e.to_string())?;
                (IrConst::Float(value), Type::Float)
            }
            LiteralValue::String(s) => {
                let text = s.syntax().text().to_string();
                (IrConst::String(InternedString::new(&text)), Type::String)
            }
            LiteralValue::StringWithEscape(_) => {
                // For now, treat escaped strings as regular strings
                // TODO: Properly handle escape sequences
                return Err("String with escapes not yet supported in IR generation".to_string());
            }
        };

        Ok(block.const_val(const_val, ty, source))
    }

    /// Generate IR for an identifier (variable reference).
    fn gen_ident(
        &mut self,
        ident: &cadenza_syntax::ast::Ident,
        ctx: &IrGenContext,
    ) -> Result<ValueId, String> {
        let text = ident.syntax().text().to_string();
        let name = InternedString::new(&text);
        ctx.lookup_var(name)
            .ok_or_else(|| format!("Undefined variable in IR generation: {}", name))
    }

    /// Generate IR for an application (function call or operator).
    fn gen_apply(
        &mut self,
        apply: &cadenza_syntax::ast::Apply,
        block: &mut BlockBuilder,
        ctx: &mut IrGenContext,
        source: SourceLocation,
    ) -> Result<ValueId, String> {
        // Check if this is an operator application
        let callee = apply.callee().ok_or("Missing callee in application")?;

        // Handle binary operators
        if let Expr::Op(op) = &callee {
            let op_text = op.syntax().text().to_string();

            // Check if this is the assignment operator with a let pattern
            if op_text == "=" {
                let args = apply.all_arguments();
                if args.len() == 2 {
                    // Check if LHS is [let, name] pattern
                    if let Expr::Apply(lhs_apply) = &args[0] {
                        if let Some(Expr::Ident(lhs_ident)) = lhs_apply.callee() {
                            let lhs_text = lhs_ident.syntax().text().to_string();
                            if lhs_text == "let" {
                                // This is a let binding: let name = value
                                return self
                                    .gen_let_binding(lhs_apply, &args[1], block, ctx, source);
                            }
                        }
                    }
                }
                // Regular assignment (not let) - not supported in IR yet
                return Err(
                    "Assignment operator (without let) not yet supported in IR generation"
                        .to_string(),
                );
            }

            // Get the operator
            let ir_op = self.map_operator(&op_text)?;

            // Get arguments
            let args = apply.all_arguments();
            if args.len() != 2 {
                return Err(format!(
                    "Binary operator {} expects 2 arguments, got {}",
                    op_text,
                    args.len()
                ));
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

        // Handle macro and function calls by identifier or synthetic
        let func_name_opt = match &callee {
            Expr::Ident(ident) => {
                let text = ident.syntax().text().to_string();
                Some(InternedString::new(&text))
            }
            Expr::Synthetic(syn) => {
                let id = syn.identifier();
                Some(InternedString::new(id))
            }
            _ => None,
        };

        if let Some(func_name) = func_name_opt {
            let func_name_str: &str = &func_name;

            // Check if this is a known macro
            if func_name_str == "__block__" {
                return self.gen_block(apply, block, ctx);
            } else if func_name_str == "__list__" {
                return self.gen_list(apply, block, ctx, source);
            } else if func_name_str == "match" {
                // Match expressions require control flow which isn't supported
                // in the current single-block expression generation
                return Err("match expressions not yet supported in IR generation - requires multi-block control flow".to_string());
            }

            // Look up the function ID
            let func_id = self
                .functions
                .get(&func_name)
                .copied()
                .ok_or_else(|| format!("Unknown function in IR generation: {}", func_name))?;

            // Generate IR for arguments
            let args = apply.all_arguments();
            let arg_values: Result<Vec<ValueId>, String> = args
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

        Err(format!(
            "Unsupported callee type for IR generation: {:?}",
            callee
        ))
    }

    /// Generate IR for a let binding: let name = value
    fn gen_let_binding(
        &mut self,
        lhs_apply: &cadenza_syntax::ast::Apply,
        value_expr: &Expr,
        block: &mut BlockBuilder,
        ctx: &mut IrGenContext,
        _source: SourceLocation,
    ) -> Result<ValueId, String> {
        // Extract the variable name from [let, name]
        let args = lhs_apply.all_arguments();
        if args.is_empty() {
            return Err("let requires a variable name".to_string());
        }

        let var_name = match &args[0] {
            Expr::Ident(ident) => {
                let text = ident.syntax().text().to_string();
                InternedString::new(&text)
            }
            _ => return Err("let requires an identifier as variable name".to_string()),
        };

        // Generate IR for the value expression
        let value_id = self.gen_expr(value_expr, block, ctx)?;

        // Infer the type of the value
        // Keep as InferType to preserve type variables for polymorphism
        let inferred_ty = self
            .type_inferencer
            .infer_expr(value_expr, ctx.type_env())
            .unwrap_or(InferType::Concrete(Type::Unknown));

        // Bind the variable in the context with its type
        ctx.bind_var(var_name, value_id, &inferred_ty);

        // Return the value (let expressions evaluate to their value)
        Ok(value_id)
    }

    /// Generate IR for a block expression
    fn gen_block(
        &mut self,
        apply: &cadenza_syntax::ast::Apply,
        block: &mut BlockBuilder,
        ctx: &mut IrGenContext,
    ) -> Result<ValueId, String> {
        let args = apply.all_arguments();

        if args.is_empty() {
            // Empty block returns nil
            return Ok(block.const_val(IrConst::Nil, Type::Nil, self.dummy_source()));
        }

        // Generate IR for each expression in the block, returning the last value
        args.iter()
            .try_fold(None, |_, expr| self.gen_expr(expr, block, ctx).map(Some))?
            .ok_or_else(|| "Block should have at least one expression".to_string())
    }

    /// Generate IR for a list construction
    fn gen_list(
        &mut self,
        _apply: &cadenza_syntax::ast::Apply,
        _block: &mut BlockBuilder,
        _ctx: &mut IrGenContext,
        _source: SourceLocation,
    ) -> Result<ValueId, String> {
        // TODO: Add list construction instruction to IR
        // For now, return an error as lists aren't supported in IR yet
        Err("List construction not yet supported in IR".to_string())
    }

    /// Map operator string to IR binary operator.
    fn map_operator(&self, op: &str) -> Result<IrBinOp, String> {
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
            _ => Err(format!("Unsupported operator: {}", op)),
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
            .gen_function(&func)
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
            .gen_function(&func)
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
            .gen_function(&func)
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
            .gen_function(&double_func)
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
            .gen_function(&quadruple_func)
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
            .gen_function(&add_func)
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
            .gen_function(&compute_func)
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
            .gen_function(&countdown_func)
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
}
