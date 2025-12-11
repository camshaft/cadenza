//! The `assert` special form for runtime assertions.

use crate::{
    Eval,
    context::EvalContext,
    diagnostic::{BoxedDiagnosticExt, Diagnostic, Result},
    ir::{BlockBuilder, IrGenContext, SourceLocation, ValueId},
    special_form::BuiltinSpecialForm,
    value::{Type, Value},
};
use cadenza_syntax::ast::Expr;
use std::sync::OnceLock;

/// Returns the `assert` special form for runtime assertions.
///
/// The `assert` special form evaluates a condition and optionally a message.
/// If the condition is false, it returns an assertion failure diagnostic.
///
/// # Evaluation
/// - Takes 1 or 2 arguments: condition expression and optional message
/// - Evaluates the condition (must be a boolean)
/// - If false, returns an assertion failure with the message
/// - If true, returns Nil
///
/// # IR Generation
/// - Not yet supported for assertions
///
/// # Examples
/// ```cadenza
/// assert v == 1
/// assert v == 1 "expected v to be one"
/// ```
pub fn get() -> &'static BuiltinSpecialForm {
    static ASSERT_FORM: OnceLock<BuiltinSpecialForm> = OnceLock::new();
    ASSERT_FORM.get_or_init(|| BuiltinSpecialForm {
        name: "assert",
        signature: Type::function(vec![Type::Bool], Type::Nil),
        eval_fn: eval_assert,
        ir_fn: ir_assert,
    })
}

fn eval_assert(args: &[Expr], ctx: &mut EvalContext<'_>) -> Result<Value> {
    // Validate argument count (1 or 2)
    if args.is_empty() || args.len() > 2 {
        return Err(Diagnostic::syntax(
            "assert expects 1 or 2 arguments: condition [message]",
        ));
    }

    // Get the condition expression
    let condition_expr = &args[0];

    // Evaluate the condition
    let condition_value = ctx.eval_child(condition_expr)?;

    // Check that condition is a boolean
    let condition_result = match condition_value {
        Value::Bool(b) => b,
        _ => {
            return Err(
                Diagnostic::type_error(Type::Bool, condition_value.type_of())
                    .with_span(condition_expr.span()),
            );
        }
    };

    // If condition is false, create assertion failure
    if !condition_result {
        // Get the condition expression text for error message
        let condition_text = condition_expr.syntax().text();

        // Build the error message
        let message = if args.len() == 2 {
            // Custom message provided
            let msg_expr = &args[1];
            let msg_value = ctx.eval_child(msg_expr)?;
            match msg_value {
                Value::String(s) => format!("{}\n  condition: {}", s, condition_text.as_str()),
                _ => {
                    return Err(Diagnostic::type_error(Type::String, msg_value.type_of())
                        .with_span(msg_expr.span()));
                }
            }
        } else {
            // No custom message, add descriptive prefix
            format!("Assertion failed: {}", condition_text.as_str())
        };

        return Err(Diagnostic::assertion_failed(message).with_span(condition_expr.span()));
    }

    // Assertion passed
    Ok(Value::Nil)
}

fn ir_assert(
    _args: &[Expr],
    _block: &mut BlockBuilder,
    _ctx: &mut IrGenContext,
    _source: SourceLocation,
    _gen_expr: &mut dyn FnMut(&Expr, &mut BlockBuilder, &mut IrGenContext) -> Result<ValueId>,
) -> Result<ValueId> {
    // TODO: Add assertion instruction to IR
    Err(Diagnostic::syntax(
        "assert not yet supported in IR generation",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Compiler, Env};
    use cadenza_syntax::parse::parse;

    #[test]
    fn test_assert_passes() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = "assert true";
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert!(!results.is_empty(), "Expected at least one result");
        let value = &results[0];
        assert_eq!(*value, Value::Nil);
    }

    #[test]
    fn test_assert_fails() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = "assert false";
        let parsed = parse(input);
        let root = parsed.ast();

        let _results = crate::eval(&root, &mut env, &mut compiler);

        // Should have an error diagnostic
        assert!(!compiler.diagnostics().is_empty());
    }
}
