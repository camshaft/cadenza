//! The `fn` special form for function definitions.

use crate::{
    context::EvalContext,
    diagnostic::{Diagnostic, Result},
    interner::InternedString,
    ir::{BlockBuilder, IrGenContext, SourceLocation, ValueId},
    special_form::BuiltinSpecialForm,
    value::{Type, UserFunction, Value},
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

    // Capture the current environment for closure semantics
    let captured_env = ctx.env.clone();

    // Create the user function value
    let user_fn_value = UserFunction {
        name,
        params,
        body,
        captured_env,
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
