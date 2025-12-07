//! The `typeof` special form for type queries.

use crate::{
    context::EvalContext,
    diagnostic::{Diagnostic, Result},
    ir::{BlockBuilder, IrGenContext, SourceLocation, ValueId},
    special_form::BuiltinSpecialForm,
    value::{Type, Value},
};
use cadenza_syntax::ast::Expr;
use std::sync::OnceLock;

/// Returns the `typeof` special form for querying expression types.
///
/// The `typeof` special form performs type inference on an expression and
/// returns the inferred type as a runtime value.
///
/// # Evaluation
/// - Takes 1 argument: an expression
/// - Infers the type of the expression using the type inferencer
/// - Returns the type as a Value::Type
///
/// # IR Generation
/// - Not yet supported for typeof
///
/// # Examples
/// ```cadenza
/// typeof 42         // Returns Type(Integer)
/// typeof "hello"    // Returns Type(String)
/// ```
pub fn get() -> &'static BuiltinSpecialForm {
    static TYPEOF_FORM: OnceLock<BuiltinSpecialForm> = OnceLock::new();
    TYPEOF_FORM.get_or_init(|| BuiltinSpecialForm {
        name: "typeof",
        signature: Type::function(vec![Type::Unknown], Type::Type),
        eval_fn: eval_typeof,
        ir_fn: ir_typeof,
    })
}

fn eval_typeof(args: &[Expr], ctx: &mut EvalContext<'_>) -> Result<Value> {
    // Validate argument count
    if args.len() != 1 {
        return Err(Diagnostic::syntax("typeof expects 1 argument: expression"));
    }

    let expr = &args[0];

    // Build type environment from current runtime environment and compiler
    let type_env = crate::typeinfer::TypeEnv::from_context(ctx.env, ctx.compiler);

    // Infer the type of the expression
    let inferred_type = ctx
        .compiler
        .type_inferencer_mut()
        .infer_expr(expr, &type_env)
        .map_err(|e| {
            Diagnostic::syntax(format!("Type inference failed for expression: {}", e))
                .with_span(expr.span())
        })?;

    // Convert to concrete type, or use Unknown if it has type variables
    let concrete_type = inferred_type.to_concrete().unwrap_or(Type::Unknown);
    Ok(Value::Type(concrete_type))
}

fn ir_typeof(
    _args: &[Expr],
    _block: &mut BlockBuilder,
    _ctx: &mut IrGenContext,
    _source: SourceLocation,
    _gen_expr: &mut dyn FnMut(&Expr, &mut BlockBuilder, &mut IrGenContext) -> Result<ValueId>,
) -> Result<ValueId> {
    // TODO: Add typeof instruction to IR or compile-time evaluate
    Err(Diagnostic::syntax(
        "typeof not yet supported in IR generation",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Compiler, Env};
    use cadenza_syntax::parse::parse;

    #[test]
    fn test_typeof_integer() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = "typeof 42";
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert!(!results.is_empty(), "Expected at least one result");
        let value = &results[0];
        assert_eq!(*value, Value::Type(Type::Integer));
    }

    #[test]
    fn test_typeof_string() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = "typeof \"hello\"";
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert!(!results.is_empty(), "Expected at least one result");
        let value = &results[0];
        assert_eq!(*value, Value::Type(Type::String));
    }
}
