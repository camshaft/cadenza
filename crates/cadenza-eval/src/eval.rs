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
    value::{BuiltinFn, BuiltinMacro, Type, Value},
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
/// This now uses the same delegation pattern as the `=` operator: if LHS is a macro call,
/// we delegate to that macro.
#[allow(clippy::collapsible_if)]
fn hoist_functions(root: &Root, env: &mut Env, compiler: &mut Compiler) {
    let mut ctx = EvalContext::new(env, compiler);

    for expr in root.items() {
        // Check if this is a function definition (= with macro pattern on LHS)
        if let Expr::Apply(apply) = expr {
            // Check if the callee is the = operator
            if let Some(Expr::Op(op)) = apply.callee() {
                if op.syntax().text() == "=" {
                    // This is an = application, get all arguments
                    let args = apply.all_arguments();
                    if args.len() == 2 {
                        let lhs = &args[0];
                        let rhs = &args[1];

                        // Check if LHS is a macro application (e.g., fn)
                        if let Expr::Apply(lhs_apply) = lhs {
                            if let Some(callee_expr) = lhs_apply.callee() {
                                // Try to get the macro identifier
                                if let Some(id) = extract_identifier(&callee_expr) {
                                    // Check if this is a macro (specifically, check for 'fn' for hoisting)
                                    // We only hoist functions, not other macros
                                    let id_str: &str = &id;
                                    if id_str == "fn" {
                                        // Check if this is actually registered as a macro
                                        if ctx.compiler.get_macro(id).is_some()
                                            || matches!(
                                                ctx.env.get(id),
                                                Some(Value::BuiltinMacro(_))
                                            )
                                        {
                                            // This is a function definition - delegate to fn macro
                                            let lhs_args = lhs_apply.all_arguments();
                                            let mut new_args =
                                                Vec::with_capacity(lhs_args.len() + 1);
                                            new_args.extend(lhs_args);
                                            new_args.push(rhs.clone());

                                            // Get the fn macro and call it
                                            let macro_value =
                                                if let Some(value) = ctx.compiler.get_macro(id) {
                                                    Some(value.clone())
                                                } else {
                                                    ctx.env.get(id).cloned()
                                                };

                                            if let Some(Value::BuiltinMacro(builtin)) = macro_value
                                            {
                                                let _ = (builtin.func)(&new_args, &mut ctx);
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

/// Helper to auto-apply functions when referenced as standalone identifiers.
///
/// If the value is a user function, it is automatically invoked with no arguments.
/// This will succeed for zero-parameter functions and produce an arity error for
/// functions that require parameters.
/// Otherwise, the value is returned as-is.
fn maybe_auto_apply(value: Value, ctx: &mut EvalContext<'_>) -> Result<Value> {
    if let Value::UserFunction(_) = &value {
        return apply_value(value, vec![], ctx);
    }
    Ok(value)
}

/// Helper to look up an identifier value without auto-applying.
/// Used when the identifier is being used as a callee in an application.
fn eval_ident_no_auto_apply(ident: &Ident, ctx: &mut EvalContext<'_>) -> Result<Value> {
    let text = ident.syntax().text();
    let id: InternedString = text.to_string().as_str().into();

    // First check the local environment
    if let Some(value) = ctx.env.get(id) {
        return Ok(value.clone());
    }

    // Then check compiler definitions
    if let Some(value) = ctx.compiler.get_var(id) {
        return Ok(value.clone());
    }

    // Check if it's a registered unit name
    // If so, return a unit constructor value
    if let Some(unit) = ctx.compiler.units().get(id) {
        return Ok(Value::UnitConstructor(unit.clone()));
    }

    Err(Diagnostic::undefined_variable(id))
}

impl Eval for Ident {
    fn eval(&self, ctx: &mut EvalContext<'_>) -> Result<Value> {
        let value = eval_ident_no_auto_apply(self, ctx)?;
        maybe_auto_apply(value, ctx)
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

        // Not a macro call - evaluate the callee
        // For identifiers and operators, we must NOT auto-apply since this is an application context
        let callee = match &callee_expr {
            Expr::Ident(ident) => eval_ident_no_auto_apply(ident, ctx)?,
            Expr::Op(op) => {
                // Look up operator in environment
                let text = op.syntax().text();
                let id: InternedString = text.to_string().as_str().into();
                ctx.env
                    .get(id)
                    .cloned()
                    .ok_or_else(|| Diagnostic::undefined_variable(id))?
            }
            _ => callee_expr.eval(ctx)?,
        };

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
        Expr::Synthetic(syn) => {
            // Synthetic nodes provide their identifier directly
            Some(syn.identifier().into())
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
        Value::Symbol(id) => {
            // Look up the symbol in the environment and try to call it
            // This handles operators and other functions stored in variables
            let actual_value = ctx
                .env
                .get(id)
                .cloned()
                .ok_or_else(|| Diagnostic::undefined_variable(id))?;
            // Recursively apply the looked-up value
            apply_value(actual_value, args, ctx)
        }
        Value::UnitConstructor(unit) => {
            // Unit constructors create quantities from numbers
            if args.len() != 1 {
                return Err(Diagnostic::arity(1, args.len()));
            }

            let value = match &args[0] {
                Value::Integer(n) => *n as f64,
                Value::Float(f) => *f,
                _ => {
                    return Err(Diagnostic::type_error(
                        Type::union(vec![Type::Integer, Type::Float]),
                        args[0].type_of(),
                    ));
                }
            };

            // Create a derived dimension from this unit's dimension
            use crate::unit::DerivedDimension;
            let dimension = DerivedDimension::from_dimension(unit.dimension);

            Ok(Value::Quantity {
                value,
                unit,
                dimension,
            })
        }
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
        _ => Err(Diagnostic::not_callable(callee.type_of())),
    }
}

/// Helper function to create a Value from a numeric result and optional dimension.
///
/// Returns Quantity if dimension is Some, otherwise returns Float.
fn create_numeric_value(
    value: f64,
    dimension: Option<crate::unit::DerivedDimension>,
    unit: Option<crate::unit::Unit>,
) -> Value {
    match dimension {
        Some(dim) if !dim.is_dimensionless() => {
            // Create a quantity with the dimension
            // For now, use a synthesized unit name based on the dimension
            let unit = unit.unwrap_or_else(|| {
                use crate::unit::Unit;
                // Create a temporary unit for the dimension
                let unit_name: InternedString = format!("{}", dim).as_str().into();
                Unit::base(unit_name)
            });
            Value::Quantity {
                value,
                unit,
                dimension: dim,
            }
        }
        _ => {
            // Dimensionless or no dimension - return plain float
            Value::Float(value)
        }
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
            "__list__" | "__record__" | "__block__" => {
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

/// Creates the `let` macro for declaring and initializing variables.
///
/// The `let` macro can be used in two forms:
///
/// 1. With `=` operator (delegated from `=`): `let x = 42`
///    - Called by `=` with arguments: `[x, 42]`
///    - Declares the variable and assigns the value
///
/// 2. Standalone (returns Nil): `let x` (when used before `=`, returns Nil to signal delegation)
///
/// When called with 2 arguments, it declares the variable and assigns the value.
/// When called with 1 argument, it returns Nil to signal that delegation is needed.
///
/// Examples:
/// - `let x = 42` - Called with `[x, 42]`, declares `x` and assigns 42
/// - `let y = x + 10` - Called with `[y, x + 10]`, declares `y` and assigns the result
pub fn builtin_let() -> BuiltinMacro {
    BuiltinMacro {
        name: "let",
        signature: Type::function(vec![Type::Symbol], Type::Nil),
        func: |args, ctx| {
            // If called with 0 arguments, return Nil
            if args.is_empty() {
                return Ok(Value::Nil);
            }

            // If called with 1 argument, return Nil (needs delegation)
            if args.len() == 1 {
                return Ok(Value::Nil);
            }

            // Called with 2 arguments: [name, value]
            if args.len() != 2 {
                return Err(Diagnostic::syntax(
                    "let expects 1 or 2 arguments (e.g., let x, or let x = 42)",
                ));
            }

            // First argument is the identifier
            let ident = match &args[0] {
                Expr::Ident(i) => i,
                _ => {
                    return Err(Diagnostic::syntax(
                        "let requires an identifier as the variable name",
                    ));
                }
            };

            // Get the identifier name
            let text = ident.syntax().text();
            let name: InternedString = text.to_string().as_str().into();

            // Second argument is the value expression
            let value_expr = &args[1];
            let value = value_expr.eval(ctx)?;

            // Define the variable in the environment with the evaluated value
            ctx.env.define(name, value.clone());

            // Return the value
            Ok(value)
        },
    }
}

/// Creates the `=` operator that assigns values or delegates to macros.
///
/// The `=` operator takes two arguments:
/// 1. The left-hand side (which can be a macro application or an identifier)
/// 2. The right-hand side (the value to assign or pass to the macro)
///
/// When the LHS is a macro application (e.g., `let x`, `fn name params...`, `measure name`),
/// the `=` operator delegates to that macro by calling it with `[lhs_args..., rhs]`.
///
/// When the LHS is a plain identifier, `=` performs a direct reassignment to that variable.
///
/// Examples:
/// - `let x = 42` - Delegates to `let` with `[x, 42]`
/// - `fn add x y = x + y` - Delegates to `fn` with `[add, x, y, x + y]`
/// - `measure inch = millimeter 25.4` - Delegates to `measure` with `[inch, millimeter 25.4]`
/// - `x = 50` - Direct reassignment to existing variable `x`
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

            // Check if LHS is a macro application - delegate if so
            if let Expr::Apply(apply) = lhs_expr {
                if let Some(callee_expr) = apply.callee() {
                    // Try to extract an identifier from the callee
                    if let Some(id) = extract_identifier(&callee_expr) {
                        // Check if this identifier refers to a macro
                        let macro_value = if let Some(value) = ctx.compiler.get_macro(id) {
                            Some(value.clone())
                        } else {
                            ctx.env.get(id).and_then(|v| match v {
                                Value::BuiltinMacro(_) => Some(v.clone()),
                                _ => None,
                            })
                        };

                        if let Some(Value::BuiltinMacro(builtin)) = macro_value {
                            // This is a macro! Delegate to it with [lhs_args..., rhs]
                            let lhs_args = apply.all_arguments();
                            let mut new_args = Vec::with_capacity(lhs_args.len() + 1);
                            new_args.extend(lhs_args);
                            new_args.push(rhs_expr.clone());

                            // Call the macro directly
                            return (builtin.func)(&new_args, ctx);
                        }
                    }
                }
            }

            // LHS is not a macro application - handle as direct identifier reassignment
            match lhs_expr {
                Expr::Ident(ident) => {
                    let text = ident.syntax().text();
                    let name: InternedString = text.to_string().as_str().into();

                    let rhs_value = rhs_expr.eval(ctx)?;

                    // Check if the variable exists (must be declared with `let` first)
                    if let Some(var) = ctx.env.get_mut(name) {
                        *var = rhs_value.clone();
                        Ok(rhs_value)
                    } else {
                        Err(Diagnostic::undefined_variable(name))
                    }
                }
                _ => {
                    // LHS is neither a macro application nor an identifier
                    Err(Diagnostic::syntax(
                        "left side of = must be an identifier or a macro application (e.g., let x, fn name, measure unit)",
                    ))
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
        return Err(Diagnostic::syntax("fn requires at least a function name"));
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
                return Err(Diagnostic::syntax("fn parameters must be identifiers"));
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

/// Creates the `__block__` synthetic macro for handling block expressions.
///
/// The `__block__` macro is automatically emitted by the parser when multiple
/// expressions are at the same indentation level. It creates a new scope,
/// evaluates each expression in sequence, and returns the value of the last
/// expression.
///
/// Example:
/// ```cadenza
/// let result =
///     let x = 10
///     let y = 20
///     x + y
/// ```
///
/// The parser transforms this into: `[=, [let, result], [__block__, [=, [let, x], 10], [=, [let, y], 20], [+, x, y]]]`
pub fn builtin_block() -> BuiltinMacro {
    BuiltinMacro {
        name: "__block__",
        signature: Type::function(vec![Type::Unknown], Type::Unknown),
        func: |args, ctx| {
            if args.is_empty() {
                return Ok(Value::Nil);
            }

            // Push a new scope for the block
            ctx.env.push_scope();

            // Evaluate each expression in sequence
            let mut result = Value::Nil;
            for expr in args {
                result = expr.eval(ctx)?;
                // Continue evaluation even if one expression returns Nil
            }

            // Pop the scope when exiting the block
            ctx.env.pop_scope();

            // Return the last expression's value
            Ok(result)
        },
    }
}

/// Creates the `__list__` builtin macro for list literals.
///
/// The `__list__` macro evaluates its arguments and constructs a list value.
/// It is automatically used by the parser when encountering list literal syntax `[...]`.
///
/// Examples:
/// ```ignore
/// [1, 2, 3]         // Creates Value::List([Integer(1), Integer(2), Integer(3)])
/// []                // Creates Value::List([])
/// [x, y + 1, f z]   // Evaluates each element expression
/// ```
///
/// The parser transforms `[1, 2, 3]` into: `[__list__, 1, 2, 3]`
pub fn builtin_list() -> BuiltinMacro {
    BuiltinMacro {
        name: "__list__",
        signature: Type::function(vec![], Type::list(Type::Unknown)),
        func: |args, ctx| {
            // Evaluate each argument expression
            let mut elements = Vec::with_capacity(args.len());
            for expr in args {
                let value = expr.eval(ctx)?;
                elements.push(value);
            }

            // Return the list value
            Ok(Value::List(elements))
        },
    }
}

/// Creates the `fn` macro for defining functions.
///
/// The `fn` macro is used to define functions. It can be used in two forms:
///
/// 1. With `=` operator (delegated from `=`): `fn name params... = body`
///    - Called by `=` with arguments: `[name, params..., body]`
///    - Defines a function with the given name, parameters, and body
///
/// 2. Standalone (returns Nil): `fn name params...` (when used before `=`, returns Nil to signal delegation)
///
/// When called with at least 2 arguments, the last argument is treated as the body,
/// and all previous arguments are treated as [name, params...].
///
/// Examples:
/// - `fn add x y = x + y` - Called with `[add, x, y, x + y]`, defines a 2-param function
/// - `fn square x = x * x` - Called with `[square, x, x * x]`, defines a 1-param function
/// - `fn const = 42` - Called with `[const, 42]`, defines a 0-param function
pub fn builtin_fn() -> BuiltinMacro {
    BuiltinMacro {
        name: "fn",
        signature: Type::function(vec![Type::Unknown], Type::Nil),
        func: |args, ctx| {
            // If called with 0 or 1 arguments, return Nil (needs delegation)
            if args.len() < 2 {
                return Ok(Value::Nil);
            }

            // When called with 2+ arguments, treat last as body and rest as [name, params...]
            let (fn_args, body_slice) = args.split_at(args.len() - 1);
            let body_expr = &body_slice[0];

            // Use the existing helper function
            handle_function_definition(fn_args, body_expr, ctx)
        },
    }
}

/// Creates the `measure` builtin macro for defining units and conversions.
///
/// The `measure` macro defines a unit for dimensional analysis. It can be used in two forms:
///
/// 1. Base unit definition: `measure meter`
///    - Called with 1 argument: `[meter]`
///    - Defines a new base unit with no conversion
///
/// 2. Derived unit with conversion: `measure inch = millimeter 25.4`
///    - Called by `=` with 2 arguments: `[inch, millimeter 25.4]`
///    - Defines a new unit where 1 inch = 25.4 millimeters
///    - The syntax `measure inch = base scale` defines: 1 inch = scale * base
///    - Creates a bidirectional link (both units can be converted to each other)
///
/// Examples:
/// ```ignore
/// measure meter               // Base unit
/// measure inch = millimeter 25.4   // 1 inch = 25.4 millimeters
/// measure foot = inch 12           // 1 foot = 12 inches
/// ```
pub fn builtin_measure() -> BuiltinMacro {
    use crate::unit::Unit;

    BuiltinMacro {
        name: "measure",
        signature: Type::function(vec![Type::Symbol], Type::Nil),
        func: |args, ctx| {
            if args.is_empty() {
                return Err(Diagnostic::syntax("measure requires at least one argument"));
            }

            // Case 1: Base unit definition - measure name
            if args.len() == 1 {
                match &args[0] {
                    Expr::Ident(ident) => {
                        let text = ident.syntax().text();
                        let name: InternedString = text.to_string().as_str().into();
                        let unit = Unit::base(name);
                        ctx.compiler.units_mut().register(unit);
                        return Ok(Value::Nil);
                    }
                    _ => {
                        return Err(Diagnostic::syntax(
                            "measure requires an identifier as unit name",
                        ));
                    }
                }
            }

            // Case 2: Derived unit definition - measure name = base scale
            // When called from `=`, we receive [name, rhs] where rhs is `base scale`
            if args.len() == 2 {
                // Get the unit name
                let name = match &args[0] {
                    Expr::Ident(ident) => {
                        let text = ident.syntax().text();
                        let name: InternedString = text.to_string().as_str().into();
                        name
                    }
                    _ => {
                        return Err(Diagnostic::syntax(
                            "measure unit name must be an identifier",
                        ));
                    }
                };

                // The RHS should be: base scale (an Apply node)
                let rhs_expr = &args[1];
                match rhs_expr {
                    Expr::Apply(rhs_apply) => {
                        let base_expr = rhs_apply.callee();
                        let scale_args: Vec<Expr> = rhs_apply
                            .arguments()
                            .filter_map(|arg| arg.value())
                            .collect();

                        if scale_args.len() != 1 {
                            return Err(Diagnostic::syntax(
                                "measure conversion requires: base scale (e.g., millimeter 25.4)",
                            ));
                        }

                        // Get base unit name
                        let base_unit_name = match base_expr {
                            Some(Expr::Ident(ident)) => {
                                let text = ident.syntax().text();
                                let name: InternedString = text.to_string().as_str().into();
                                name
                            }
                            _ => {
                                return Err(Diagnostic::syntax(
                                    "measure conversion base must be a unit name",
                                ));
                            }
                        };

                        // Get scale
                        let scale_value = scale_args[0].eval(ctx)?;
                        let scale = match scale_value {
                            Value::Integer(n) => n as f64,
                            Value::Float(f) => f,
                            _ => {
                                return Err(Diagnostic::syntax(
                                    "measure conversion scale must be a number",
                                ));
                            }
                        };

                        // Look up the base unit
                        let base_unit =
                            ctx.compiler.units().get(base_unit_name).ok_or_else(|| {
                                Diagnostic::syntax(format!("undefined unit '{}'", &*base_unit_name))
                            })?;

                        // Create derived unit: 1 new_unit = scale base_units
                        let derived_unit = Unit::derived(name, base_unit.dimension, scale, 0.0);

                        ctx.compiler.units_mut().register(derived_unit);
                        Ok(Value::Nil)
                    }
                    _ => Err(Diagnostic::syntax(
                        "measure conversion requires: base scale (e.g., millimeter 25.4)",
                    )),
                }
            } else {
                Err(Diagnostic::syntax(
                    "measure expects 1 or 2 arguments (e.g., measure meter, or measure inch = millimeter 25.4)",
                ))
            }
        },
    }
}

// =============================================================================
// Built-in operators (registered as functions in the standard environment)
// =============================================================================

/// Creates the `+` addition operator.
pub fn builtin_add() -> BuiltinFn {
    BuiltinFn {
        name: "+",
        signature: Type::union(vec![
            Type::function(vec![Type::Integer, Type::Integer], Type::Integer),
            Type::function(vec![Type::Float, Type::Float], Type::Float),
        ]),
        func: |args, _ctx| {
            if args.len() != 2 {
                return Err(Diagnostic::arity(2, args.len()));
            }

            match (&args[0], &args[1]) {
                (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(a + b)),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
                // Handle quantity addition - dimensions must match
                (
                    Value::Quantity {
                        value: v1,
                        unit: u1,
                        dimension: d1,
                    },
                    Value::Quantity {
                        value: v2,
                        unit: u2,
                        dimension: _d2,
                    },
                ) => {
                    // Check if dimensions are compatible
                    if u1.dimension != u2.dimension {
                        return Err(Diagnostic::syntax(format!(
                            "cannot add quantities with incompatible dimensions: {} and {}",
                            u1.name, u2.name
                        )));
                    }

                    // Convert second quantity to first unit's scale
                    let scale_ratio = u2.scale / u1.scale;
                    let v2_converted = v2 * scale_ratio;

                    let result = v1 + v2_converted;
                    Ok(Value::Quantity {
                        value: result,
                        unit: u1.clone(),
                        dimension: d1.clone(),
                    })
                }
                // Allow adding plain numbers to quantities
                (
                    Value::Quantity {
                        value,
                        unit,
                        dimension,
                    },
                    Value::Integer(n),
                )
                | (
                    Value::Integer(n),
                    Value::Quantity {
                        value,
                        unit,
                        dimension,
                    },
                ) => {
                    let result = value + (*n as f64);
                    Ok(Value::Quantity {
                        value: result,
                        unit: unit.clone(),
                        dimension: dimension.clone(),
                    })
                }
                (
                    Value::Quantity {
                        value,
                        unit,
                        dimension,
                    },
                    Value::Float(f),
                )
                | (
                    Value::Float(f),
                    Value::Quantity {
                        value,
                        unit,
                        dimension,
                    },
                ) => {
                    let result = value + f;
                    Ok(Value::Quantity {
                        value: result,
                        unit: unit.clone(),
                        dimension: dimension.clone(),
                    })
                }
                // Type mismatch - reject mixed integer/float operations
                (Value::Integer(_), Value::Float(_)) | (Value::Float(_), Value::Integer(_)) => {
                    Err(Diagnostic::type_error(args[0].type_of(), args[1].type_of()))
                }
                // For non-numeric types, report type error
                (Value::Integer(_), b) | (Value::Float(_), b) => Err(Diagnostic::type_error(
                    Type::union(vec![Type::Integer, Type::Float]),
                    b.type_of(),
                )),
                (a, _) => Err(Diagnostic::type_error(
                    Type::union(vec![Type::Integer, Type::Float]),
                    a.type_of(),
                )),
            }
        },
    }
}

/// Creates the `-` subtraction/negation operator.
pub fn builtin_sub() -> BuiltinFn {
    BuiltinFn {
        name: "-",
        signature: Type::union(vec![
            Type::function(vec![Type::Integer], Type::Integer),
            Type::function(vec![Type::Float], Type::Float),
            Type::function(vec![Type::Integer, Type::Integer], Type::Integer),
            Type::function(vec![Type::Float, Type::Float], Type::Float),
        ]),
        func: |args, _ctx| {
            match args.len() {
                1 => {
                    // Unary negation
                    match &args[0] {
                        Value::Integer(a) => Ok(Value::Integer(-a)),
                        Value::Float(a) => Ok(Value::Float(-a)),
                        Value::Quantity {
                            value,
                            unit,
                            dimension,
                        } => Ok(Value::Quantity {
                            value: -value,
                            unit: unit.clone(),
                            dimension: dimension.clone(),
                        }),
                        a => Err(Diagnostic::type_error(
                            Type::union(vec![Type::Integer, Type::Float]),
                            a.type_of(),
                        )),
                    }
                }
                2 => {
                    // Binary subtraction
                    match (&args[0], &args[1]) {
                        (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(a - b)),
                        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
                        // Handle quantity subtraction - dimensions must match
                        (
                            Value::Quantity {
                                value: v1,
                                unit: u1,
                                dimension: d1,
                            },
                            Value::Quantity {
                                value: v2,
                                unit: u2,
                                dimension: _d2,
                            },
                        ) => {
                            // Check if dimensions are compatible
                            if u1.dimension != u2.dimension {
                                return Err(Diagnostic::syntax(format!(
                                    "cannot subtract quantities with incompatible dimensions: {} and {}",
                                    u1.name, u2.name
                                )));
                            }

                            // Convert second quantity to first unit's scale
                            let scale_ratio = u2.scale / u1.scale;
                            let v2_converted = v2 * scale_ratio;

                            let result = v1 - v2_converted;
                            Ok(Value::Quantity {
                                value: result,
                                unit: u1.clone(),
                                dimension: d1.clone(),
                            })
                        }
                        // Allow subtracting plain numbers from quantities
                        (
                            Value::Quantity {
                                value,
                                unit,
                                dimension,
                            },
                            Value::Integer(n),
                        ) => {
                            let result = value - (*n as f64);
                            Ok(Value::Quantity {
                                value: result,
                                unit: unit.clone(),
                                dimension: dimension.clone(),
                            })
                        }
                        (
                            Value::Integer(n),
                            Value::Quantity {
                                value,
                                unit,
                                dimension,
                            },
                        ) => {
                            let result = (*n as f64) - value;
                            Ok(Value::Quantity {
                                value: result,
                                unit: unit.clone(),
                                dimension: dimension.clone(),
                            })
                        }
                        (
                            Value::Quantity {
                                value,
                                unit,
                                dimension,
                            },
                            Value::Float(f),
                        ) => {
                            let result = value - f;
                            Ok(Value::Quantity {
                                value: result,
                                unit: unit.clone(),
                                dimension: dimension.clone(),
                            })
                        }
                        (
                            Value::Float(f),
                            Value::Quantity {
                                value,
                                unit,
                                dimension,
                            },
                        ) => {
                            let result = f - value;
                            Ok(Value::Quantity {
                                value: result,
                                unit: unit.clone(),
                                dimension: dimension.clone(),
                            })
                        }
                        // Type mismatch - reject mixed integer/float operations
                        (Value::Integer(_), Value::Float(_))
                        | (Value::Float(_), Value::Integer(_)) => {
                            Err(Diagnostic::type_error(args[0].type_of(), args[1].type_of()))
                        }
                        // For non-numeric types, report type error
                        (Value::Integer(_), b) | (Value::Float(_), b) => {
                            Err(Diagnostic::type_error(
                                Type::union(vec![Type::Integer, Type::Float]),
                                b.type_of(),
                            ))
                        }
                        (a, _) => Err(Diagnostic::type_error(
                            Type::union(vec![Type::Integer, Type::Float]),
                            a.type_of(),
                        )),
                    }
                }
                0 => Err(Diagnostic::arity(1, 0)),
                _ => Err(Diagnostic::arity(2, args.len())),
            }
        },
    }
}

/// Creates the `*` multiplication operator.
pub fn builtin_mul() -> BuiltinFn {
    BuiltinFn {
        name: "*",
        signature: Type::union(vec![
            Type::function(vec![Type::Integer, Type::Integer], Type::Integer),
            Type::function(vec![Type::Float, Type::Float], Type::Float),
        ]),
        func: |args, _ctx| {
            if args.len() != 2 {
                return Err(Diagnostic::arity(2, args.len()));
            }

            match (&args[0], &args[1]) {
                // Integer multiplication
                (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(a * b)),
                // Float multiplication
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
                // Quantity multiplication with dimensions
                (
                    Value::Quantity {
                        value: v1,
                        unit: _,
                        dimension: d1,
                    },
                    Value::Quantity {
                        value: v2,
                        unit: _,
                        dimension: d2,
                    },
                ) => {
                    let result_val = v1 * v2;
                    let result_dim = d1.multiply(d2);
                    Ok(create_numeric_value(result_val, Some(result_dim), None))
                }
                // Quantity * scalar
                (
                    Value::Quantity {
                        value,
                        unit,
                        dimension,
                    },
                    Value::Float(f),
                )
                | (
                    Value::Float(f),
                    Value::Quantity {
                        value,
                        unit,
                        dimension,
                    },
                ) => {
                    let result = value * f;
                    Ok(Value::Quantity {
                        value: result,
                        unit: unit.clone(),
                        dimension: dimension.clone(),
                    })
                }
                (
                    Value::Quantity {
                        value,
                        unit,
                        dimension,
                    },
                    Value::Integer(n),
                )
                | (
                    Value::Integer(n),
                    Value::Quantity {
                        value,
                        unit,
                        dimension,
                    },
                ) => {
                    let result = value * (*n as f64);
                    Ok(Value::Quantity {
                        value: result,
                        unit: unit.clone(),
                        dimension: dimension.clone(),
                    })
                }
                // Type mismatch - reject mixed integer/float operations
                (Value::Integer(_), Value::Float(_)) | (Value::Float(_), Value::Integer(_)) => {
                    Err(Diagnostic::type_error(args[0].type_of(), args[1].type_of()))
                }
                // For non-numeric types, report type error
                (Value::Integer(_), b) | (Value::Float(_), b) => Err(Diagnostic::type_error(
                    Type::union(vec![Type::Integer, Type::Float]),
                    b.type_of(),
                )),
                (a, _) => Err(Diagnostic::type_error(
                    Type::union(vec![Type::Integer, Type::Float]),
                    a.type_of(),
                )),
            }
        },
    }
}

/// Creates the `/` division operator.
pub fn builtin_div() -> BuiltinFn {
    BuiltinFn {
        name: "/",
        signature: Type::union(vec![
            Type::function(vec![Type::Integer, Type::Integer], Type::Integer),
            Type::function(vec![Type::Float, Type::Float], Type::Float),
        ]),
        func: |args, _ctx| {
            if args.len() != 2 {
                return Err(Diagnostic::arity(2, args.len()));
            }

            match (&args[0], &args[1]) {
                // Integer division
                (Value::Integer(a), Value::Integer(b)) => {
                    if *b == 0 {
                        return Err(Diagnostic::syntax("division by zero"));
                    }
                    Ok(Value::Integer(a / b))
                }
                // Float division
                (Value::Float(a), Value::Float(b)) => {
                    if *b == 0.0 {
                        return Err(Diagnostic::syntax("division by zero"));
                    }
                    Ok(Value::Float(a / b))
                }
                // Quantity division with dimensions
                (
                    Value::Quantity {
                        value: v1,
                        unit: _,
                        dimension: d1,
                    },
                    Value::Quantity {
                        value: v2,
                        unit: _,
                        dimension: d2,
                    },
                ) => {
                    if *v2 == 0.0 {
                        return Err(Diagnostic::syntax("division by zero"));
                    }
                    let result_val = v1 / v2;
                    let result_dim = d1.divide(d2);
                    Ok(create_numeric_value(result_val, Some(result_dim), None))
                }
                // Quantity / scalar
                (
                    Value::Quantity {
                        value,
                        unit,
                        dimension,
                    },
                    Value::Float(f),
                ) => {
                    if *f == 0.0 {
                        return Err(Diagnostic::syntax("division by zero"));
                    }
                    let result = value / f;
                    Ok(Value::Quantity {
                        value: result,
                        unit: unit.clone(),
                        dimension: dimension.clone(),
                    })
                }
                (
                    Value::Quantity {
                        value,
                        unit,
                        dimension,
                    },
                    Value::Integer(n),
                ) => {
                    if *n == 0 {
                        return Err(Diagnostic::syntax("division by zero"));
                    }
                    let result = value / (*n as f64);
                    Ok(Value::Quantity {
                        value: result,
                        unit: unit.clone(),
                        dimension: dimension.clone(),
                    })
                }
                // scalar / Quantity - inverts dimension
                (
                    Value::Float(f),
                    Value::Quantity {
                        value, dimension, ..
                    },
                ) => {
                    if *value == 0.0 {
                        return Err(Diagnostic::syntax("division by zero"));
                    }
                    let result_val = f / value;
                    // Invert the dimension
                    use crate::unit::DerivedDimension;
                    let inverted_dim = DerivedDimension {
                        numerator: dimension.denominator.clone(),
                        denominator: dimension.numerator.clone(),
                    };
                    Ok(create_numeric_value(result_val, Some(inverted_dim), None))
                }
                (
                    Value::Integer(n),
                    Value::Quantity {
                        value, dimension, ..
                    },
                ) => {
                    if *value == 0.0 {
                        return Err(Diagnostic::syntax("division by zero"));
                    }
                    let result_val = (*n as f64) / value;
                    // Invert the dimension
                    use crate::unit::DerivedDimension;
                    let inverted_dim = DerivedDimension {
                        numerator: dimension.denominator.clone(),
                        denominator: dimension.numerator.clone(),
                    };
                    Ok(create_numeric_value(result_val, Some(inverted_dim), None))
                }
                // Type mismatch - reject mixed integer/float operations
                (Value::Integer(_), Value::Float(_)) | (Value::Float(_), Value::Integer(_)) => {
                    Err(Diagnostic::type_error(args[0].type_of(), args[1].type_of()))
                }
                // For non-numeric types, report type error
                (Value::Integer(_), b) | (Value::Float(_), b) => Err(Diagnostic::type_error(
                    Type::union(vec![Type::Integer, Type::Float]),
                    b.type_of(),
                )),
                (a, _) => Err(Diagnostic::type_error(
                    Type::union(vec![Type::Integer, Type::Float]),
                    a.type_of(),
                )),
            }
        },
    }
}

/// Creates the `==` equality operator.
pub fn builtin_eq() -> BuiltinFn {
    BuiltinFn {
        name: "==",
        signature: Type::function(vec![Type::Unknown, Type::Unknown], Type::Bool),
        func: |args, _ctx| {
            if args.len() != 2 {
                return Err(Diagnostic::arity(2, args.len()));
            }

            let a = &args[0];
            let b = &args[1];

            // Check that types match
            let type_a = a.type_of();
            let type_b = b.type_of();
            if type_a != type_b {
                return Err(Diagnostic::type_error(type_a, type_b));
            }

            Ok(Value::Bool(a == b))
        },
    }
}

/// Creates the `!=` inequality operator.
pub fn builtin_ne() -> BuiltinFn {
    BuiltinFn {
        name: "!=",
        signature: Type::function(vec![Type::Unknown, Type::Unknown], Type::Bool),
        func: |args, _ctx| {
            if args.len() != 2 {
                return Err(Diagnostic::arity(2, args.len()));
            }

            let a = &args[0];
            let b = &args[1];

            // Check that types match
            let type_a = a.type_of();
            let type_b = b.type_of();
            if type_a != type_b {
                return Err(Diagnostic::type_error(type_a, type_b));
            }

            Ok(Value::Bool(a != b))
        },
    }
}

/// Helper function to compare two values using their PartialOrd implementation.
/// Returns a type error if the values have different types or are not comparable.
fn compare_ordered<F>(a: &Value, b: &Value, check_ordering: F) -> Result<Value>
where
    F: FnOnce(std::cmp::Ordering) -> bool,
{
    // Check that types match - strongly typed, no implicit conversions
    let type_a = a.type_of();
    let type_b = b.type_of();
    if type_a != type_b {
        return Err(Diagnostic::type_error(type_a, type_b));
    }

    match (a, b) {
        (Value::Integer(a), Value::Integer(b)) => {
            let ordering = a.cmp(b);
            Ok(Value::Bool(check_ordering(ordering)))
        }
        (Value::Float(a), Value::Float(b)) => {
            // For floats, use partial_cmp since they may be NaN
            match a.partial_cmp(b) {
                Some(ordering) => Ok(Value::Bool(check_ordering(ordering))),
                None => Err(Diagnostic::syntax("cannot compare NaN values".to_string())),
            }
        }
        _ => Err(Diagnostic::syntax(format!(
            "cannot compare values of type {}",
            a.type_of()
        ))),
    }
}

/// Creates the `<` less-than operator.
pub fn builtin_lt() -> BuiltinFn {
    BuiltinFn {
        name: "<",
        signature: Type::union(vec![
            Type::function(vec![Type::Integer, Type::Integer], Type::Bool),
            Type::function(vec![Type::Float, Type::Float], Type::Bool),
        ]),
        func: |args, _ctx| {
            if args.len() != 2 {
                return Err(Diagnostic::arity(2, args.len()));
            }
            compare_ordered(&args[0], &args[1], |ord| ord == std::cmp::Ordering::Less)
        },
    }
}

/// Creates the `<=` less-than-or-equal operator.
pub fn builtin_lte() -> BuiltinFn {
    BuiltinFn {
        name: "<=",
        signature: Type::union(vec![
            Type::function(vec![Type::Integer, Type::Integer], Type::Bool),
            Type::function(vec![Type::Float, Type::Float], Type::Bool),
        ]),
        func: |args, _ctx| {
            if args.len() != 2 {
                return Err(Diagnostic::arity(2, args.len()));
            }
            compare_ordered(&args[0], &args[1], |ord| ord != std::cmp::Ordering::Greater)
        },
    }
}

/// Creates the `>` greater-than operator.
pub fn builtin_gt() -> BuiltinFn {
    BuiltinFn {
        name: ">",
        signature: Type::union(vec![
            Type::function(vec![Type::Integer, Type::Integer], Type::Bool),
            Type::function(vec![Type::Float, Type::Float], Type::Bool),
        ]),
        func: |args, _ctx| {
            if args.len() != 2 {
                return Err(Diagnostic::arity(2, args.len()));
            }
            compare_ordered(&args[0], &args[1], |ord| ord == std::cmp::Ordering::Greater)
        },
    }
}

/// Creates the `>=` greater-than-or-equal operator.
pub fn builtin_gte() -> BuiltinFn {
    BuiltinFn {
        name: ">=",
        signature: Type::union(vec![
            Type::function(vec![Type::Integer, Type::Integer], Type::Bool),
            Type::function(vec![Type::Float, Type::Float], Type::Bool),
        ]),
        func: |args, _ctx| {
            if args.len() != 2 {
                return Err(Diagnostic::arity(2, args.len()));
            }
            compare_ordered(&args[0], &args[1], |ord| ord != std::cmp::Ordering::Less)
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
        let mut env = Env::with_standard_builtins();
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
