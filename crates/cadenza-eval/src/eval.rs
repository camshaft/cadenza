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
    diagnostic::{BoxedDiagnosticExt, Diagnostic, Result},
    env::Env,
    interner::InternedString,
    value::{BuiltinFn, Type, Value},
};
use cadenza_syntax::{
    ast::{Apply, Attr, Expr, Ident, Literal, LiteralValue, Root, Synthetic},
    span::Span,
};

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
                                                Some(
                                                    Value::BuiltinMacro(_) | Value::SpecialForm(_)
                                                )
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
                                            } else if let Some(Value::SpecialForm(sf)) = macro_value
                                            {
                                                let _ = sf.eval(&new_args, &mut ctx);
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
                let id: InternedString = text.interned();
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
                // Remove underscores for parsing
                let clean = text.as_str().replace('_', "");
                let n: i64 = clean.parse().map_err(|_| {
                    Diagnostic::syntax(format!("invalid integer: {}", text.as_str()))
                })?;
                Ok(Value::Integer(n))
            }
            LiteralValue::Float(float_val) => {
                let text = float_val.syntax().text();
                let clean = text.as_str().replace('_', "");
                let n: f64 = clean
                    .parse()
                    .map_err(|_| Diagnostic::syntax(format!("invalid float: {}", text.as_str())))?;
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
pub fn eval_ident_no_auto_apply(ident: &Ident, ctx: &mut EvalContext<'_>) -> Result<Value> {
    let text = ident.syntax().text();
    let id: InternedString = text.interned();

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

    Err(Diagnostic::undefined_variable(id).with_span(ident.span()))
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

            // Check for macro or special form in environment
            if let Some(Value::BuiltinMacro(_) | Value::SpecialForm(_)) = ctx.env.get(id) {
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
                let id: InternedString = text.interned();
                let range = op.syntax().text_range();
                let span = Span::new(range.start().into(), range.end().into());
                ctx.env
                    .get(id)
                    .cloned()
                    .ok_or_else(|| Diagnostic::undefined_variable(id).with_span(span))?
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
pub fn extract_identifier(expr: &Expr) -> Option<InternedString> {
    match expr {
        Expr::Ident(ident) => {
            let text = ident.syntax().text();
            Some(text.interned())
        }
        Expr::Op(op) => {
            let text = op.syntax().text();
            Some(text.interned())
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
            // Collect unevaluated argument expressions (use all_arguments to get flattened args)
            let arg_exprs: Vec<Expr> = apply.all_arguments();

            // Call the builtin macro with unevaluated expressions
            (builtin.func)(&arg_exprs, ctx)
        }
        Value::SpecialForm(special_form) => {
            // Collect unevaluated argument expressions (use all_arguments to get flattened args)
            let arg_exprs: Vec<Expr> = apply.all_arguments();

            // Call the special form's eval method with unevaluated expressions
            special_form.eval(&arg_exprs, ctx)
        }
        _ => Err(Diagnostic::internal("expected macro value")),
    }
}

/// Applies a callable value to arguments.
pub fn apply_value(callee: Value, args: Vec<Value>, ctx: &mut EvalContext<'_>) -> Result<Value> {
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
        Value::StructConstructor { name, field_types } => {
            // Struct constructors create struct instances from field assignments
            // Expect exactly one argument: a record with field values
            if args.len() != 1 {
                return Err(Diagnostic::arity(1, args.len()));
            }

            // The argument should be a record with field values
            match &args[0] {
                Value::Record {
                    type_name: _,
                    fields: field_values,
                } => {
                    // Collect all validation errors instead of bailing at first error
                    let mut errors = Vec::new();
                    let mut validated_fields = Vec::with_capacity(field_types.len());

                    // Track which fields were found
                    let mut found_fields = std::collections::HashSet::new();

                    // Check that all required fields are present and types match
                    for (expected_name, expected_type) in &field_types {
                        let mut found = false;
                        for (field_name, field_value) in field_values {
                            if field_name == expected_name {
                                found = true;
                                found_fields.insert(*field_name);

                                // Check that the type matches
                                let actual_type = field_value.type_of();
                                if !types_compatible(expected_type, &actual_type) {
                                    errors.push(format!(
                                        "field '{}': expected type {}, got {}",
                                        &*expected_name, expected_type, actual_type
                                    ));
                                } else {
                                    validated_fields.push((*field_name, field_value.clone()));
                                }
                                break;
                            }
                        }
                        if !found {
                            errors.push(format!("missing required field '{}'", &*expected_name));
                        }
                    }

                    // Check for extra fields that weren't expected
                    for (field_name, _) in field_values {
                        if !found_fields.contains(field_name) {
                            errors.push(format!("unexpected field '{}'", &*field_name));
                        }
                    }

                    // If there are any errors, return them all at once
                    if !errors.is_empty() {
                        return Err(Diagnostic::syntax(&format!(
                            "struct {} field validation failed:\n  - {}",
                            &*name,
                            errors.join("\n  - ")
                        )));
                    }

                    // Create the struct instance
                    Ok(Value::Record {
                        type_name: Some(name),
                        fields: validated_fields,
                    })
                }
                _ => Err(Diagnostic::syntax(&format!(
                    "struct constructor {} expects a record argument",
                    &*name
                ))),
            }
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

/// Helper function to check if two types are compatible.
///
/// This performs basic type compatibility checking. Unknown types are treated as
/// compatible with anything since full type information may not be available at runtime.
/// Struct types use nominal equality - they must have the same name to be compatible.
///
/// TODO: Replace with proper type unification when the type system is more complete.
fn types_compatible(expected: &Type, actual: &Type) -> bool {
    // For struct types, enforce nominal typing - names must match
    if let (Type::Struct { name: n1, .. }, Type::Struct { name: n2, .. }) = (expected, actual) {
        return n1 == n2;
    }

    // Unknown types are compatible with anything (temporary until type system is complete)
    if matches!(expected, Type::Unknown) || matches!(actual, Type::Unknown) {
        return true;
    }

    // Otherwise, types must be equal
    expected == actual
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
            "__list__" | "__record__" | "__block__" | "__index__" => {
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
