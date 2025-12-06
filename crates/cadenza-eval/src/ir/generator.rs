//! IR Generator - converts evaluated values to IR.
//!
//! This module generates target-independent IR from Cadenza's evaluated values.
//! The IR is in SSA form and can be used for optimization passes and code generation.
//!
//! Note: IR generation happens after evaluation, not directly from AST. This means
//! we work with Values rather than AST nodes, making the transformation simpler.

use super::{
    BinOp as IrBinOp, BlockBuilder, FunctionId, IrBuilder, IrConst, SourceLocation, ValueId,
};
use crate::{
    interner::InternedString,
    value::{Type, UserFunction, Value},
};
use cadenza_syntax::ast::Expr;
use std::collections::HashMap;

/// Context for IR generation from AST.
///
/// Tracks SSA values and variable bindings during IR generation.
struct IrGenContext {
    /// Maps variable names to their SSA value IDs.
    variables: HashMap<InternedString, ValueId>,
}

impl IrGenContext {
    /// Create a new IR generation context.
    fn new() -> Self {
        Self {
            variables: HashMap::new(),
        }
    }

    /// Bind a variable name to an SSA value.
    fn bind_var(&mut self, name: InternedString, value: ValueId) {
        self.variables.insert(name, value);
    }

    /// Look up a variable binding.
    fn lookup_var(&self, name: InternedString) -> Option<ValueId> {
        self.variables.get(&name).copied()
    }
}

/// IR Generator - converts evaluated values to IR.
pub struct IrGenerator {
    builder: IrBuilder,
}

impl IrGenerator {
    /// Create a new IR generator.
    pub fn new() -> Self {
        Self {
            builder: IrBuilder::new(),
        }
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
                        dimension: dim.clone(),
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

        // Create parameter types from the function's parameters
        let param_types: Vec<(InternedString, Type)> = func
            .params
            .iter()
            .map(|p| (*p, Type::Unknown)) // TODO: Get actual types from type inference
            .collect();

        // For now, assume return type is Unknown
        // TODO: Get actual return type from type inference
        let return_ty = Type::Unknown;

        let mut func_builder = self.builder.function(name, param_types.clone(), return_ty);

        // Create the entry block
        let mut block = func_builder.block();

        // Create a context for SSA variable tracking
        let mut ctx = IrGenContext::new();

        // Bind parameters to their SSA values (%0, %1, ...)
        for (i, param_name) in func.params.iter().enumerate() {
            ctx.bind_var(*param_name, ValueId(i as u32));
        }

        // Generate IR for the function body
        let result = self.gen_expr(&func.body, &mut block, &mut ctx)?;

        // Return the result
        let (block_inst, next_val) = block.ret(Some(result), self.dummy_source());
        func_builder.add_block(block_inst, next_val);

        // Build the function
        let ir_func = func_builder.build();
        let func_id = self.builder.add_function(ir_func);

        Ok(func_id)
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

        let const_val = match value {
            LiteralValue::Integer(i) => {
                let text = i.syntax().text();
                let value = text.to_string().parse::<i64>().map_err(|e| e.to_string())?;
                IrConst::Integer(value)
            }
            LiteralValue::Float(f) => {
                let text = f.syntax().text();
                let value = text.to_string().parse::<f64>().map_err(|e| e.to_string())?;
                IrConst::Float(value)
            }
            LiteralValue::String(s) => {
                let text = s.syntax().text().to_string();
                IrConst::String(InternedString::new(&text))
            }
            LiteralValue::StringWithEscape(_) => {
                // For now, treat escaped strings as regular strings
                // TODO: Properly handle escape sequences
                return Err("String with escapes not yet supported in IR generation".to_string());
            }
        };

        Ok(block.const_val(const_val, source))
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

            // Emit binary operation
            return Ok(block.binop(ir_op, lhs, rhs, source));
        }

        // TODO: Handle function calls
        Err("Function calls not yet supported in IR generation".to_string())
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

    /// Get a mutable reference to the underlying IrBuilder.
    pub fn builder_mut(&mut self) -> &mut IrBuilder {
        &mut self.builder
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
            generator.value_to_const(&Value::Float(3.14)),
            Some(IrConst::Float(3.14))
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

        let func_id = generator.gen_function(&func).expect("Failed to generate IR");

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
        assert!(ir_text.contains("function add"));
        assert!(ir_text.contains("add %0 %1"));
        assert!(ir_text.contains("ret"));
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

        let _func_id = generator.gen_function(&func).expect("Failed to generate IR");

        let module = generator.build();
        let ir_text = module.to_string();

        println!("Generated IR:\n{}", ir_text);

        // Verify the function was generated
        assert_eq!(module.functions.len(), 1);
        assert_eq!(module.functions[0].name, InternedString::new("get_answer"));
        assert_eq!(module.functions[0].params.len(), 0);

        // Verify the IR contains expected elements
        assert!(ir_text.contains("function get_answer"));
        assert!(ir_text.contains("const 42"));
        assert!(ir_text.contains("ret"));
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

        let _func_id = generator.gen_function(&func).expect("Failed to generate IR");

        let module = generator.build();
        let ir_text = module.to_string();

        println!("Generated IR:\n{}", ir_text);

        // Verify the function was generated
        assert_eq!(module.functions.len(), 1);
        assert_eq!(module.functions[0].name, InternedString::new("calc"));

        // Verify the IR contains expected operations
        assert!(ir_text.contains("function calc"));
        assert!(ir_text.contains("mul"));
        assert!(ir_text.contains("add"));
        assert!(ir_text.contains("ret"));
    }
}
