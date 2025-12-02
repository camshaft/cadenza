//! Tree-walk evaluator for the Cadenza language.
//!
//! The evaluator interprets the AST produced by cadenza-syntax.
//! It handles:
//! - Literals (integers, floats, strings)
//! - Identifiers (variable lookup)
//! - Lists/vectors
//! - Function application
//! - Macro expansion

use crate::{
    compiler::Compiler,
    context::{Eval, EvalContext},
    diagnostic::{Diagnostic, Result},
    env::Env,
    interner::InternedString,
    value::{BuiltinMacro, Type, Value},
};
use cadenza_syntax::ast::{Apply, Attr, Expr, Ident, Literal, LiteralValue, Root, Synthetic};

/// Evaluates a complete source file (Root node).
///
/// Each top-level expression is evaluated in order. The results are
/// collected into a vector, though most top-level expressions will
/// return `Value::Nil` as side effects on the `Compiler` are the
/// primary purpose.
///
/// This function implements function hoisting by performing two passes:
/// 1. First pass: scan for function definitions and register them in the compiler
/// 2. Second pass: evaluate all expressions normally
///
/// This continues evaluation even when expressions fail, recording
/// errors in the compiler. On error, `Value::Nil` is used as the result for
/// that expression. Check `compiler.has_errors()` after calling to see if
/// any errors occurred.
pub fn eval(root: &Root, env: &mut Env, compiler: &mut Compiler) -> Vec<Value> {
    // First pass: hoist function definitions
    hoist_functions(root, env, compiler);

    // Second pass: evaluate all expressions
    let mut ctx = EvalContext::new(env, compiler);
    let mut results = Vec::new();
    for expr in root.items() {
        match expr.eval(&mut ctx) {
            Ok(value) => results.push(value),
            Err(diagnostic) => {
                ctx.compiler.record_diagnostic(*diagnostic);
                results.push(Value::Nil);
            }
        }
    }
    results
}

/// First pass: scan for function definitions and register them (hoisting).
///
/// This scans top-level expressions looking for function definitions of the form
/// `fn name params... = body` and registers them in the compiler without evaluating them fully.
fn hoist_functions(root: &Root, env: &mut Env, compiler: &mut Compiler) {
    let mut ctx = EvalContext::new(env, compiler);

    for expr in root.items() {
        // Check if this is a function definition (= with fn pattern on LHS)
        if let Expr::Apply(apply) = expr {
            // Check if the callee is the = operator
            if let Some(callee) = apply.callee() {
                if let Expr::Op(op) = &callee {
                    if op.syntax().text().to_string() == "=" {
                        // This is an = application, get all arguments
                        let args = apply.all_arguments();
                        if args.len() == 2 {
                            let lhs = &args[0];
                            let rhs = &args[1];

                            // Check if LHS is fn pattern
                            if let Expr::Apply(lhs_apply) = lhs {
                                if let Some(fn_callee) = lhs_apply.callee() {
                                    if let Expr::Ident(ident) = &fn_callee {
                                        if ident.syntax().text().to_string() == "fn" {
                                            // This is a function definition - handle it
                                            let fn_args = lhs_apply.all_arguments();
                                            let _ = handle_function_definition(&fn_args, rhs, &mut ctx);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// =============================================================================
// Eval trait implementations
// =============================================================================

impl Eval for Expr {
    fn eval(&self, ctx: &mut EvalContext<'_>) -> Result<Value> {
        match self {
            Expr::Literal(lit) => lit.eval(ctx),
            Expr::Ident(ident) => ident.eval(ctx),
            Expr::Apply(apply) => apply.eval(ctx),
            Expr::Attr(attr) => attr.eval(ctx),
            Expr::Op(op) => {
                // Operators as values (for higher-order usage)
                // Use SyntaxText directly without allocating a String
                let text = op.syntax().text();
                let id: InternedString = text.to_string().as_str().into();
                Ok(Value::Symbol(id))
            }
            Expr::Synthetic(syn) => syn.eval(ctx),
            Expr::Error(_) => Err(Diagnostic::syntax("encountered error node in AST")),
        }
    }
}

impl Eval for Literal {
    fn eval(&self, _ctx: &mut EvalContext<'_>) -> Result<Value> {
        let value = self
            .value()
            .ok_or_else(|| Diagnostic::syntax("missing literal value"))?;

        match value {
            LiteralValue::Integer(int_val) => {
                let text = int_val.syntax().text();
                let text_str = text.to_string();
                // Remove underscores for parsing
                let clean = text_str.replace('_', "");
                let n: i64 = clean
                    .parse()
                    .map_err(|_| Diagnostic::syntax(format!("invalid integer: {text_str}")))?;
                Ok(Value::Integer(n))
            }
            LiteralValue::Float(float_val) => {
                let text = float_val.syntax().text();
                let text_str = text.to_string();
                let clean = text_str.replace('_', "");
                let n: f64 = clean
                    .parse()
                    .map_err(|_| Diagnostic::syntax(format!("invalid float: {text_str}")))?;
                Ok(Value::Float(n))
            }
            LiteralValue::String(str_val) => {
                let text = str_val.syntax().text().to_string();
                Ok(Value::String(text))
            }
            LiteralValue::StringWithEscape(str_val) => {
                let text = str_val.syntax().text().to_string();
                // TODO: Process escape sequences
                Ok(Value::String(text))
            }
        }
    }
}

impl Eval for Ident {
    fn eval(&self, ctx: &mut EvalContext<'_>) -> Result<Value> {
        let text = self.syntax().text();
        // TODO: Investigate rowan API to avoid allocation. SyntaxText doesn't implement
        // AsRef<str> directly. See STATUS.md for tracking.
        let id: InternedString = text.to_string().as_str().into();

        // First check the local environment
        if let Some(value) = ctx.env.get(id) {
            return Ok(value.clone());
        }

        // Then check compiler definitions
        if let Some(value) = ctx.compiler.get_var(id) {
            return Ok(value.clone());
        }

        Err(Diagnostic::undefined_variable(id))
    }
}

impl Eval for Apply {
    fn eval(&self, ctx: &mut EvalContext<'_>) -> Result<Value> {
        // Get the callee (innermost identifier in nested applications)
        let callee_expr = self
            .callee()
            .ok_or_else(|| Diagnostic::syntax("missing callee in application"))?;

        // Try to extract an identifier/operator name from the callee.
        // If successful, check if it names a macro before evaluating.
        if let Some(id) = extract_identifier(&callee_expr) {
            // Check for macro in compiler
            if let Some(macro_value) = ctx.compiler.get_macro(id) {
                return apply_macro(macro_value.clone(), self, ctx);
            }

            // Check for macro in environment
            if let Some(Value::BuiltinMacro(_)) = ctx.env.get(id) {
                let macro_value = ctx.env.get(id).unwrap().clone();
                return apply_macro(macro_value, self, ctx);
            }
        }

        // Not a macro call - evaluate normally with flattened arguments
        let callee = callee_expr.eval(ctx)?;

        // Get all arguments (flattened from nested Apply nodes)
        let all_arg_exprs = self.all_arguments();
        
        // Evaluate all arguments
        let mut args = Vec::new();
        for arg_expr in all_arg_exprs {
            let value = arg_expr.eval(ctx)?;
            args.push(value);
        }

        apply_value(callee, args, ctx)
    }
}

/// Extracts an identifier from an expression if it is an Ident or Op node.
/// Returns None for other expression types.
///
/// TODO: Investigate rowan API to avoid allocation. SyntaxText doesn't implement
/// AsRef<str> directly, so we need to allocate a String. See STATUS.md item #12.
fn extract_identifier(expr: &Expr) -> Option<InternedString> {
    match expr {
        Expr::Ident(ident) => {
            let text = ident.syntax().text();
            Some(text.to_string().as_str().into())
        }
        Expr::Op(op) => {
            let text = op.syntax().text();
            Some(text.to_string().as_str().into())
        }
        _ => None,
    }
}

/// Applies a macro to unevaluated arguments.
///
/// Macros receive unevaluated AST expressions and return values directly.
/// This unified approach replaces the separate handling of macros (which returned GreenNode)
/// and special forms (which returned Value).
fn apply_macro(macro_value: Value, apply: &Apply, ctx: &mut EvalContext<'_>) -> Result<Value> {
    match macro_value {
        Value::BuiltinMacro(builtin) => {
            // Collect unevaluated argument expressions
            let arg_exprs: Vec<Expr> = apply.arguments().filter_map(|arg| arg.value()).collect();

            // Call the builtin macro with unevaluated expressions
            (builtin.func)(&arg_exprs, ctx)
        }
        _ => Err(Diagnostic::internal("expected macro value")),
    }
}

/// Applies a callable value to arguments.
fn apply_value(callee: Value, args: Vec<Value>, ctx: &mut EvalContext<'_>) -> Result<Value> {
    match callee {
        Value::BuiltinFn(builtin) => (builtin.func)(&args, ctx),
        Value::UserFunction(user_fn) => {
            // Check arity
            if args.len() != user_fn.params.len() {
                return Err(Diagnostic::arity(user_fn.params.len(), args.len()));
            }

            // Create a new environment extending the captured environment
            let mut call_env = user_fn.captured_env.clone();
            call_env.push_scope();

            // Bind parameters to arguments
            for (param, arg) in user_fn.params.iter().zip(args.iter()) {
                call_env.define(*param, arg.clone());
            }

            // Evaluate the body in the new environment
            let mut call_ctx = EvalContext::new(&mut call_env, ctx.compiler);
            let result = user_fn.body.eval(&mut call_ctx)?;

            Ok(result)
        }
        Value::Symbol(id) => {
            // Operator application
            apply_operator(id, args)
        }
        _ => Err(Diagnostic::not_callable(callee.type_of())),
    }
}

/// Applies a built-in operator.
fn apply_operator(op_id: InternedString, args: Vec<Value>) -> Result<Value> {
    let op_name: &str = &op_id;

    /// Creates the "number" type (integer | float) lazily only when needed for errors.
    fn number_type() -> Type {
        Type::union(vec![Type::Integer, Type::Float])
    }

    match op_name {
        "+" => match args.as_slice() {
            [Value::Integer(a), Value::Integer(b)] => Ok(Value::Integer(a + b)),
            [Value::Float(a), Value::Float(b)] => Ok(Value::Float(a + b)),
            [Value::Integer(a), Value::Float(b)] => Ok(Value::Float(*a as f64 + b)),
            [Value::Float(a), Value::Integer(b)] => Ok(Value::Float(a + *b as f64)),
            // For binary operators, report the first non-number type as the actual type
            [Value::Integer(_), b] | [Value::Float(_), b] => {
                Err(Diagnostic::type_error(number_type(), b.type_of()))
            }
            [a, _] => Err(Diagnostic::type_error(number_type(), a.type_of())),
            _ => Err(Diagnostic::arity(2, args.len())),
        },
        "-" => match args.as_slice() {
            [Value::Integer(a)] => Ok(Value::Integer(-a)),
            [Value::Float(a)] => Ok(Value::Float(-a)),
            [Value::Integer(a), Value::Integer(b)] => Ok(Value::Integer(a - b)),
            [Value::Float(a), Value::Float(b)] => Ok(Value::Float(a - b)),
            [Value::Integer(a), Value::Float(b)] => Ok(Value::Float(*a as f64 - b)),
            [Value::Float(a), Value::Integer(b)] => Ok(Value::Float(a - *b as f64)),
            // For binary operators, report the first non-number type as the actual type
            [Value::Integer(_), b] | [Value::Float(_), b] => {
                Err(Diagnostic::type_error(number_type(), b.type_of()))
            }
            [a, _] => Err(Diagnostic::type_error(number_type(), a.type_of())),
            [] => Err(Diagnostic::arity(1, 0)),
            _ => Err(Diagnostic::arity(2, args.len())),
        },
        "*" => match args.as_slice() {
            [Value::Integer(a), Value::Integer(b)] => Ok(Value::Integer(a * b)),
            [Value::Float(a), Value::Float(b)] => Ok(Value::Float(a * b)),
            [Value::Integer(a), Value::Float(b)] => Ok(Value::Float(*a as f64 * b)),
            [Value::Float(a), Value::Integer(b)] => Ok(Value::Float(a * *b as f64)),
            // For binary operators, report the first non-number type as the actual type
            [Value::Integer(_), b] | [Value::Float(_), b] => {
                Err(Diagnostic::type_error(number_type(), b.type_of()))
            }
            [a, _] => Err(Diagnostic::type_error(number_type(), a.type_of())),
            _ => Err(Diagnostic::arity(2, args.len())),
        },
        "/" => match args.as_slice() {
            [Value::Integer(a), Value::Integer(b)] => {
                if *b == 0 {
                    Err(Diagnostic::syntax("division by zero"))
                } else {
                    Ok(Value::Integer(a / b))
                }
            }
            [Value::Float(a), Value::Float(b)] => Ok(Value::Float(a / b)),
            [Value::Integer(a), Value::Float(b)] => Ok(Value::Float(*a as f64 / b)),
            [Value::Float(a), Value::Integer(b)] => Ok(Value::Float(a / *b as f64)),
            // For binary operators, report the first non-number type as the actual type
            [Value::Integer(_), b] | [Value::Float(_), b] => {
                Err(Diagnostic::type_error(number_type(), b.type_of()))
            }
            [a, _] => Err(Diagnostic::type_error(number_type(), a.type_of())),
            _ => Err(Diagnostic::arity(2, args.len())),
        },
        "==" => match args.as_slice() {
            [a, b] => Ok(Value::Bool(a == b)),
            _ => Err(Diagnostic::arity(2, args.len())),
        },
        "!=" => match args.as_slice() {
            [a, b] => Ok(Value::Bool(a != b)),
            _ => Err(Diagnostic::arity(2, args.len())),
        },
        "<" => match args.as_slice() {
            [Value::Integer(a), Value::Integer(b)] => Ok(Value::Bool(a < b)),
            [Value::Float(a), Value::Float(b)] => Ok(Value::Bool(a < b)),
            _ => Err(Diagnostic::arity(2, args.len())),
        },
        "<=" => match args.as_slice() {
            [Value::Integer(a), Value::Integer(b)] => Ok(Value::Bool(a <= b)),
            [Value::Float(a), Value::Float(b)] => Ok(Value::Bool(a <= b)),
            _ => Err(Diagnostic::arity(2, args.len())),
        },
        ">" => match args.as_slice() {
            [Value::Integer(a), Value::Integer(b)] => Ok(Value::Bool(a > b)),
            [Value::Float(a), Value::Float(b)] => Ok(Value::Bool(a > b)),
            _ => Err(Diagnostic::arity(2, args.len())),
        },
        ">=" => match args.as_slice() {
            [Value::Integer(a), Value::Integer(b)] => Ok(Value::Bool(a >= b)),
            [Value::Float(a), Value::Float(b)] => Ok(Value::Bool(a >= b)),
            _ => Err(Diagnostic::arity(2, args.len())),
        },
        // Note: The `=` operator is handled as a special form (builtin_assign) for proper
        // variable assignment semantics. If `=` appears here, it means it wasn't registered
        // as a special form in the environment.
        _ => Err(Diagnostic::undefined_variable(op_id)),
    }
}

impl Eval for Attr {
    fn eval(&self, ctx: &mut EvalContext<'_>) -> Result<Value> {
        // For now, attributes are evaluated and returned as-is
        // In the future, they might have special semantics
        if let Some(value_expr) = self.value() {
            value_expr.eval(ctx)
        } else {
            Ok(Value::Nil)
        }
    }
}

impl Eval for Synthetic {
    fn eval(&self, _ctx: &mut EvalContext<'_>) -> Result<Value> {
        let ident = self.identifier();

        match ident {
            "__list__" | "__record__" => {
                // These should not appear as standalone expressions
                // They're always wrapped in Apply nodes
                Ok(Value::Nil)
            }
            _ => Err(Diagnostic::syntax(format!(
                "unknown synthetic node: {ident}"
            ))),
        }
    }
}

// =============================================================================
// Built-in special forms for variable declaration and assignment
// =============================================================================

/// Creates the `let` special form that declares a variable.
///
/// The `let` special form takes an identifier and declares it in the environment
/// with an initial value of `Nil`. It returns the symbol so it can be used in
/// assignment expressions.
///
/// # Parsing
///
/// The expression `let x = 42` is parsed as `[[=, [let, x], 42]]`, which is:
/// - An `=` operator applied to `[let, x]` (the `let` special form applied to `x`) and `42`
///
/// This allows `let` to declare the variable and return a symbol, which `=` then uses
/// to assign the value.
pub fn builtin_let() -> BuiltinMacro {
    BuiltinMacro {
        name: "let",
        signature: Type::function(vec![Type::Symbol], Type::Symbol),
        func: |args, ctx| {
            // Expect exactly one argument (the identifier)
            if args.len() != 1 {
                return Err(Diagnostic::arity(1, args.len()));
            }

            // Validate that the expression is an identifier
            let ident = match &args[0] {
                Expr::Ident(i) => i,
                _ => {
                    return Err(Diagnostic::syntax(
                        "let requires an identifier, not an expression",
                    ));
                }
            };

            // Get the identifier name
            let text = ident.syntax().text();
            let name: InternedString = text.to_string().as_str().into();

            // Define the variable in the environment with initial value Nil
            ctx.env.define(name, Value::Nil);

            // Return the symbol
            Ok(Value::Symbol(name))
        },
    }
}

/// Creates the `=` special form that assigns a value to a variable or defines a function.
///
/// The `=` special form takes two arguments:
/// 1. The left-hand side (which should be a symbol from `let`, or a `fn` application)
/// 2. The right-hand side (which is evaluated for variables, or used as the function body)
///
/// Examples:
/// - `let x = 42` - The `let x` returns a symbol, then `=` assigns 42 to it.
/// - `fn add x y = x + y` - The `fn add x y` is recognized as a function definition pattern
pub fn builtin_assign() -> BuiltinMacro {
    BuiltinMacro {
        name: "=",
        signature: Type::function(vec![Type::Unknown, Type::Unknown], Type::Unknown),
        func: |args, ctx| {
            // Expect exactly two arguments
            if args.len() != 2 {
                return Err(Diagnostic::arity(2, args.len()));
            }

            // Get the LHS and RHS expressions
            let lhs_expr = &args[0];
            let rhs_expr = &args[1];

            // Check if LHS is a function definition pattern (fn name params...)
            if let Expr::Apply(apply) = lhs_expr {
                // Get the callee and all arguments
                if let Some(callee) = apply.callee() {
                    // Check if the callee is 'fn'
                    if let Expr::Ident(ident) = &callee {
                        let text = ident.syntax().text();
                        if text.to_string() == "fn" {
                            // This is a function definition: fn name params... = body
                            let fn_args = apply.all_arguments();
                            return handle_function_definition(&fn_args, rhs_expr, ctx);
                        }
                    }
                }
            }

            // Not a function definition - handle as normal assignment
            // Evaluate the RHS to get the value
            let rhs_value = rhs_expr.eval(ctx)?;

            // Determine the variable name based on LHS
            match lhs_expr {
                // If LHS is a plain identifier, get the name directly (for reassignment)
                Expr::Ident(ident) => {
                    let text = ident.syntax().text();
                    let name: InternedString = text.to_string().as_str().into();

                    // Check if the variable exists (must be declared with `let` first)
                    if let Some(var) = ctx.env.get_mut(name) {
                        *var = rhs_value.clone();
                        Ok(rhs_value)
                    } else {
                        Err(Diagnostic::undefined_variable(name))
                    }
                }
                // Otherwise, evaluate the LHS (e.g., from `let x`)
                _ => {
                    let lhs_value = lhs_expr.eval(ctx)?;

                    match lhs_value {
                        Value::Symbol(name) => {
                            // Update the variable in the environment
                            if let Some(var) = ctx.env.get_mut(name) {
                                *var = rhs_value.clone();
                                Ok(rhs_value)
                            } else {
                                Err(Diagnostic::undefined_variable(name))
                            }
                        }
                        _ => Err(Diagnostic::type_error(Type::Symbol, lhs_value.type_of())),
                    }
                }
            }
        },
    }
}

/// Handles function definitions of the form: fn name param1 param2... = body
///
/// The fn_args slice contains the arguments after 'fn' (i.e., `name param1 param2...`),
/// and body_expr is the function body (the RHS of the `=`).
fn handle_function_definition(
    fn_args: &[Expr],
    body_expr: &Expr,
    ctx: &mut EvalContext<'_>,
) -> Result<Value> {
    if fn_args.is_empty() {
        return Err(Diagnostic::syntax(
            "fn requires at least a function name",
        ));
    }

    // First argument is the function name
    let name_ident = match &fn_args[0] {
        Expr::Ident(i) => i,
        _ => {
            return Err(Diagnostic::syntax(
                "fn requires an identifier as the function name",
            ));
        }
    };
    let name_text = name_ident.syntax().text();
    let name: InternedString = name_text.to_string().as_str().into();

    // Remaining arguments are parameters
    let mut params = Vec::new();
    for arg in &fn_args[1..] {
        match arg {
            Expr::Ident(ident) => {
                let param_text = ident.syntax().text();
                let param_name: InternedString = param_text.to_string().as_str().into();
                params.push(param_name);
            }
            _ => {
                return Err(Diagnostic::syntax(
                    "fn parameters must be identifiers",
                ));
            }
        }
    }

    // Clone the body expression
    let body = body_expr.clone();

    // Capture the current environment for closure semantics
    let captured_env = ctx.env.clone();

    // Create the user function value
    let user_fn = Value::UserFunction(crate::value::UserFunction {
        name,
        params,
        body,
        captured_env,
    });

    // Register the function in the compiler (hoisting)
    ctx.compiler.define_var(name, user_fn);

    // Return nil
    Ok(Value::Nil)
}

/// Creates the `fn` identifier for use in function definitions.
///
/// The `fn` identifier is used in conjunction with the `=` operator to define functions.
/// When `=` sees a pattern like `fn name params... = body`, it handles the function definition.
///
/// This standalone macro just returns Nil if called directly (which shouldn't happen in normal use).
pub fn builtin_fn() -> BuiltinMacro {
    BuiltinMacro {
        name: "fn",
        signature: Type::function(vec![Type::Unknown], Type::Nil),
        func: |_args, _ctx| {
            // This should not be called directly in normal usage.
            // Function definitions are handled by the `=` macro when it sees `fn` patterns.
            Ok(Value::Nil)
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadenza_syntax::parse::parse;

    fn eval_str(src: &str) -> Result<Vec<Value>> {
        let parsed = parse(src);
        if let Some(err) = parsed.errors.first() {
            return Err(Diagnostic::parse_error(&err.message, err.span));
        }
        let root = parsed.ast();
        let mut env = Env::new();
        let mut compiler = Compiler::new();
        let results = eval(&root, &mut env, &mut compiler);
        if compiler.has_errors() {
            return Err(Box::new(
                compiler
                    .take_diagnostics()
                    .into_iter()
                    .next()
                    .expect("has_errors() returned true but no diagnostics found"),
            ));
        }
        Ok(results)
    }

    fn eval_single(src: &str) -> Result<Value> {
        let values = eval_str(src)?;
        values
            .into_iter()
            .next()
            .ok_or_else(|| Diagnostic::syntax("no expressions"))
    }

    #[test]
    fn eval_integer() {
        assert_eq!(eval_single("42").unwrap(), Value::Integer(42));
    }

    #[test]
    fn eval_float() {
        assert_eq!(eval_single("2.5").unwrap(), Value::Float(2.5));
    }

    #[test]
    fn eval_integer_with_underscores() {
        assert_eq!(eval_single("1_000_000").unwrap(), Value::Integer(1000000));
    }

    #[test]
    fn eval_string() {
        assert_eq!(
            eval_single("\"hello\"").unwrap(),
            Value::String("hello".to_string())
        );
    }

    #[test]
    fn eval_addition() {
        assert_eq!(eval_single("1 + 2").unwrap(), Value::Integer(3));
    }

    #[test]
    fn eval_subtraction() {
        assert_eq!(eval_single("5 - 3").unwrap(), Value::Integer(2));
    }

    #[test]
    fn eval_multiplication() {
        assert_eq!(eval_single("4 * 5").unwrap(), Value::Integer(20));
    }

    #[test]
    fn eval_division() {
        assert_eq!(eval_single("10 / 2").unwrap(), Value::Integer(5));
    }

    #[test]
    fn eval_complex_arithmetic() {
        // Due to operator precedence, this should be parsed as 1 + (2 * 3)
        assert_eq!(eval_single("1 + 2 * 3").unwrap(), Value::Integer(7));
    }

    #[test]
    fn eval_comparison() {
        assert_eq!(eval_single("1 < 2").unwrap(), Value::Bool(true));
        assert_eq!(eval_single("2 > 1").unwrap(), Value::Bool(true));
        assert_eq!(eval_single("1 == 1").unwrap(), Value::Bool(true));
        assert_eq!(eval_single("1 != 2").unwrap(), Value::Bool(true));
    }

    #[test]
    fn eval_undefined_variable() {
        let result = eval_single("undefined_var");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().kind,
            crate::diagnostic::DiagnosticKind::UndefinedVariable(_)
        ));
    }

    #[test]
    fn eval_with_environment() {
        let parsed = parse("x");
        let root = parsed.ast();
        let mut env = Env::new();
        let mut compiler = Compiler::new();

        // Define x in environment
        let x_id: InternedString = "x".into();
        env.define(x_id, Value::Integer(42));

        let result = eval(&root, &mut env, &mut compiler);
        assert!(!compiler.has_errors());
        assert_eq!(result[0], Value::Integer(42));
    }

    #[test]
    fn eval_with_compiler_var() {
        let parsed = parse("y");
        let root = parsed.ast();
        let mut env = Env::new();
        let mut compiler = Compiler::new();

        // Define y in compiler
        let y_id: InternedString = "y".into();
        compiler.define_var(y_id, Value::Integer(100));

        let result = eval(&root, &mut env, &mut compiler);
        assert!(!compiler.has_errors());
        assert_eq!(result[0], Value::Integer(100));
    }

    #[test]
    fn eval_continues_on_error() {
        // This source has multiple expressions, some of which will fail
        let src = "1\nundefined_var\n3";
        let parsed = parse(src);
        let root = parsed.ast();
        let mut env = Env::new();
        let mut compiler = Compiler::new();

        let results = eval(&root, &mut env, &mut compiler);

        // Should have 3 results (1, Nil for error, 3)
        assert_eq!(results.len(), 3);
        assert_eq!(results[0], Value::Integer(1));
        assert_eq!(results[1], Value::Nil); // error becomes Nil
        assert_eq!(results[2], Value::Integer(3));

        // Compiler should have recorded 1 error
        assert_eq!(compiler.num_diagnostics(), 1);
        assert!(compiler.has_errors());
    }

    #[test]
    fn eval_collects_multiple_errors() {
        // Multiple undefined variables
        let src = "undefined_a\n1\nundefined_b\n2\nundefined_c";
        let parsed = parse(src);
        let root = parsed.ast();
        let mut env = Env::new();
        let mut compiler = Compiler::new();

        let results = eval(&root, &mut env, &mut compiler);

        // Should have 5 results
        assert_eq!(results.len(), 5);
        assert_eq!(results[0], Value::Nil); // undefined_a error
        assert_eq!(results[1], Value::Integer(1));
        assert_eq!(results[2], Value::Nil); // undefined_b error
        assert_eq!(results[3], Value::Integer(2));
        assert_eq!(results[4], Value::Nil); // undefined_c error

        // Compiler should have recorded 3 errors
        assert_eq!(compiler.num_diagnostics(), 3);
        assert!(compiler.has_errors());

        // All should be UndefinedVariable errors
        for diag in compiler.diagnostics() {
            assert!(matches!(
                &diag.kind,
                crate::diagnostic::DiagnosticKind::UndefinedVariable(_)
            ));
        }
    }

    #[test]
    fn eval_no_errors() {
        let src = "1\n2\n3";
        let parsed = parse(src);
        let root = parsed.ast();
        let mut env = Env::new();
        let mut compiler = Compiler::new();

        let results = eval(&root, &mut env, &mut compiler);

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], Value::Integer(1));
        assert_eq!(results[1], Value::Integer(2));
        assert_eq!(results[2], Value::Integer(3));

        // No errors recorded
        assert_eq!(compiler.num_diagnostics(), 0);
        assert!(!compiler.has_errors());
    }
}
