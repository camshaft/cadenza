//! The `__list__` special form for list literals.

use crate::{
    context::EvalContext,
    diagnostic::Result,
    ir::{BlockBuilder, IrGenContext, SourceLocation, ValueId},
    special_form::BuiltinSpecialForm,
    value::{Type, Value},
    Eval,
};
use cadenza_syntax::ast::Expr;
use std::sync::OnceLock;

/// Returns the `__list__` special form for list literals.
///
/// The `__list__` special form evaluates its arguments and constructs a list value.
/// It is automatically used by the parser when encountering list literal syntax `[...]`.
///
/// # Evaluation
/// - Evaluates each argument expression
/// - Returns a list containing all evaluated values
///
/// # IR Generation
/// - Generates IR for each element
/// - Creates a list construction instruction
///
/// # Examples
/// ```cadenza
/// [1, 2, 3]         // Creates Value::List([Integer(1), Integer(2), Integer(3)])
/// []                // Creates Value::List([])
/// [x, y + 1, f z]   // Evaluates each element expression
/// ```
pub fn get() -> &'static BuiltinSpecialForm {
    static LIST_FORM: OnceLock<BuiltinSpecialForm> = OnceLock::new();
    LIST_FORM.get_or_init(|| BuiltinSpecialForm {
        name: "__list__",
        signature: Type::function(vec![], Type::list(Type::Unknown)),
        eval_fn: eval_list,
        ir_fn: ir_list,
    })
}

fn eval_list(args: &[Expr], ctx: &mut EvalContext<'_>) -> Result<Value> {
    // Evaluate each argument expression
    let mut elements = Vec::with_capacity(args.len());
    for expr in args {
        let value = expr.eval(ctx)?;
        elements.push(value);
    }

    // Return the list value
    Ok(Value::List(elements))
}

fn ir_list(
    _args: &[Expr],
    _block: &mut BlockBuilder,
    _ctx: &mut IrGenContext,
    _source: SourceLocation,
    _gen_expr: &mut dyn FnMut(&Expr, &mut BlockBuilder, &mut IrGenContext) -> Result<ValueId>,
) -> Result<ValueId> {
    // TODO: Add list construction instruction to IR
    // For now, return an error as lists aren't fully supported in IR yet
    Err(crate::diagnostic::Diagnostic::syntax(
        "List construction not yet supported in IR",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Compiler, Env, Value};
    use cadenza_syntax::parse::parse;

    #[test]
    fn test_list_special_form_eval() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        // Test list literal
        let input = "[1, 2, 3]";
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert!(!results.is_empty(), "Expected at least one result");
        let value = &results[0];
        assert_eq!(
            *value,
            Value::List(vec![Value::Integer(1), Value::Integer(2), Value::Integer(3)])
        );
    }

    #[test]
    fn test_empty_list() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = "[]";
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert!(!results.is_empty(), "Expected at least one result");
        let value = &results[0];
        assert_eq!(*value, Value::List(vec![]));
    }
}
