//! The `fn` special form for function definitions.

use crate::{
    context::EvalContext,
    diagnostic::{Diagnostic, Result},
    eval,
    interner::InternedString,
    ir::{BlockBuilder, IrGenContext, SourceLocation, ValueId},
    special_form::BuiltinSpecialForm,
    value::{FunctionTypeAnnotation, Type, UserFunction, Value},
};
use cadenza_syntax::ast::Expr;
use std::sync::OnceLock;

/// Returns the `fn` special form for function definitions.
///
/// The `fn` special form defines a named function with parameters and a body.
///
/// # Evaluation
/// - Takes at least 2 arguments: function name, parameters..., and body
/// - Creates a UserFunction value capturing the environment
/// - Registers the function in the compiler (hoisting)
/// - Returns Nil
///
/// # IR Generation
/// - Not yet implemented (returns error)
///
/// # Examples
/// ```cadenza
/// fn add a b = a + b
/// fn zero_arity = 42
/// ```
pub fn get() -> &'static BuiltinSpecialForm {
    static FN_FORM: OnceLock<BuiltinSpecialForm> = OnceLock::new();
    FN_FORM.get_or_init(|| BuiltinSpecialForm {
        name: "fn",
        signature: Type::function(vec![Type::Unknown], Type::Nil),
        eval_fn,
        ir_fn,
    })
}

fn eval_fn(args: &[Expr], ctx: &mut EvalContext<'_>) -> Result<Value> {
    // If called with 0 or 1 arguments, return Nil (needs delegation)
    if args.len() < 2 {
        return Ok(Value::Nil);
    }

    // When called with 2+ arguments, treat last as body and rest as [name, params...]
    let (fn_args, body_slice) = args.split_at(args.len() - 1);
    let body_expr = &body_slice[0];

    // Call the existing helper function
    handle_function_definition(fn_args, body_expr, ctx)
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

    // Consume and interpret @t attributes for type annotations
    let type_annotation = parse_type_attribute(ctx)?;
    if let Some(annotation) = &type_annotation {
        if annotation.params.len() != params.len() {
            return Err(Box::new(
                Diagnostic::syntax(format!(
                    "type annotation parameter count ({}) does not match function parameters ({})",
                    annotation.params.len(),
                    params.len()
                ))
                .with_span(body.span()),
            ));
        }
    }

    // Capture the current environment for closure semantics
    let captured_env = ctx.env.clone();

    // Create the user function value
    let user_fn_value = UserFunction {
        name,
        params,
        body,
        captured_env,
        type_annotation,
    };

    // Generate IR for the function if IR generation is enabled and it hasn't been generated already
    // This check prevents duplicate IR generation during hoisting and regular evaluation
    // Do this before moving the value into the compiler
    if let Some(ir_gen) = ctx.compiler.ir_generator()
        && !ir_gen.has_function(name)
        && let Some(Err(err)) = ctx
            .compiler
            .generate_ir_for_function(&user_fn_value, ctx.env)
    {
        // Record as a warning diagnostic instead of printing to stderr
        let warning = Diagnostic::syntax(format!(
            "Failed to generate IR for function {}: {}",
            name, err
        ))
        .set_level(crate::diagnostic::DiagnosticLevel::Warning);
        ctx.compiler.record_diagnostic(warning);
    }

    // Register the function in the compiler (hoisting)
    ctx.compiler
        .define_var(name, Value::UserFunction(user_fn_value));

    // Return nil
    Ok(Value::Nil)
}

fn ir_fn(
    _args: &[Expr],
    _block: &mut BlockBuilder,
    _ctx: &mut IrGenContext,
    _source: SourceLocation,
    _gen_expr: &mut dyn FnMut(&Expr, &mut BlockBuilder, &mut IrGenContext) -> Result<ValueId>,
) -> Result<ValueId> {
    Err(Diagnostic::syntax(
        "fn special form IR generation not yet implemented",
    ))
}

/// Consume current attributes and parse @t type annotations, if present.
fn parse_type_attribute(ctx: &mut EvalContext<'_>) -> Result<Option<FunctionTypeAnnotation>> {
    let attrs = ctx.take_attributes();
    if attrs.is_empty() {
        return Ok(None);
    }

    let mut type_annotation = None;
    let mut unconsumed = Vec::new();

    for attr in attrs {
        let text = attr.syntax().text().to_string();
        let trimmed = text.trim_start();
        let is_t_attr = if let Some(rest) = trimmed.strip_prefix("@t") {
            rest.is_empty() || rest.starts_with(char::is_whitespace)
        } else {
            false
        };

        if is_t_attr && type_annotation.is_none() {
            let parsed = parse_type_annotation_text(&text, attr.span())?;
            type_annotation = Some(parsed);
        } else {
            unconsumed.push(attr);
        }
    }

    if !unconsumed.is_empty() {
        return Err(crate::eval::unconsumed_attributes_error(
            &unconsumed,
            unconsumed[0].span(),
        ));
    }

    Ok(type_annotation)
}

fn parse_type_annotation_text(text: &str, span: cadenza_syntax::span::Span) -> Result<FunctionTypeAnnotation> {
    // Strip leading "@t"
    let trimmed = text.trim();
    let without_prefix = trimmed.trim_start_matches("@t").trim();

    if without_prefix.is_empty() {
        return Err(Box::new(
            Diagnostic::syntax("type annotation requires types after @t").with_span(span),
        ));
    }

    let parts: Vec<&str> = without_prefix.split("->").collect();
    if parts.len() != 2 {
        let msg = if parts.len() > 2 {
            "type annotation must contain exactly one '->' separating parameters and return type"
        } else {
            "type annotation must use '->' to separate parameters and return type"
        };
        return Err(Box::new(
            Diagnostic::syntax(msg).with_span(span),
        ));
    }

    let params_part = parts[0].trim();
    let return_part = parts[1].trim();

    let mut params = Vec::new();
    if !params_part.is_empty() {
        for name in params_part.split_whitespace() {
            params.push(map_type_name(name, span)?);
        }
    }

    let return_type = if return_part.is_empty() {
        Type::Unknown
    } else {
        map_type_name(return_part, span)?
    };

    Ok(FunctionTypeAnnotation {
        params,
        return_type,
    })
}

fn map_type_name(name: &str, span: cadenza_syntax::span::Span) -> Result<Type> {
    let lower = name.trim().to_ascii_lowercase();
    let ty = match lower.as_str() {
        "integer" => Type::Integer,
        "float" => Type::Float,
        "string" => Type::String,
        "bool" | "boolean" => Type::Bool,
        "nil" => Type::Nil,
        "unknown" => Type::Unknown,
        _ => {
            return Err(Box::new(
                Diagnostic::syntax(format!("unknown type name in @t annotation: {}", name))
                    .with_span(span),
            ));
        }
    };
    Ok(ty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Compiler, Env};
    use cadenza_syntax::parse::parse;

    #[test]
    fn test_fn_special_form_eval() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        // Test function definition
        let input = r#"
fn add a b = a + b
add 1 2
"#;
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert_eq!(results.len(), 2);
        // First result is Nil from function definition
        assert_eq!(results[0], Value::Nil);
        // Second result is the function call result
        assert_eq!(results[1], Value::Integer(3));
    }

    #[test]
    fn test_fn_zero_arity() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = r#"
fn get_value = 42
get_value
"#;
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0], Value::Nil);
        assert_eq!(results[1], Value::Integer(42));
    }
}
