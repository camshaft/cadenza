//! The `__block__` special form for block expressions.

use crate::{
    Eval,
    context::EvalContext,
    diagnostic::Result,
    ir::{BlockBuilder, IrGenContext, SourceLocation, ValueId},
    special_form::BuiltinSpecialForm,
    value::{Type, Value},
};
use cadenza_syntax::ast::Expr;
use std::sync::OnceLock;

/// Returns the `__block__` special form for block expressions.
///
/// The `__block__` special form creates a new lexical scope, evaluates each
/// expression in sequence, and returns the value of the last expression.
///
/// # Evaluation
/// - Creates a new scope
/// - Evaluates each expression in sequence
/// - Returns the last expression's value
/// - Pops the scope on exit
///
/// # IR Generation
/// - Generates IR for each expression in sequence
/// - Returns the last expression's ValueId
///
/// # Examples
/// ```cadenza
/// let foo =
///     let bar = 1
///     let baz = 2
///     bar + baz
/// ```
pub fn get() -> &'static BuiltinSpecialForm {
    static BLOCK_FORM: OnceLock<BuiltinSpecialForm> = OnceLock::new();
    BLOCK_FORM.get_or_init(|| BuiltinSpecialForm {
        name: "__block__",
        signature: Type::function(vec![Type::Unknown], Type::Unknown),
        eval_fn: eval_block,
        ir_fn: ir_block,
    })
}

fn eval_block(args: &[Expr], ctx: &mut EvalContext<'_>) -> Result<Value> {
    if args.is_empty() {
        return Ok(Value::Nil);
    }

    // Push a new scope for the block
    ctx.env.push_scope();

    // Evaluate each expression in sequence
    let mut result = Value::Nil;
    let mut pending_attrs: Vec<cadenza_syntax::ast::Attr> = Vec::new();
    for expr in args {
        if let Expr::Attr(attr) = expr {
            pending_attrs.push(attr.clone());
            continue;
        }
        let attrs = std::mem::take(&mut pending_attrs);
        result = crate::eval::evaluate_with_attributes(expr, attrs, ctx)?;
    }

    if !pending_attrs.is_empty() {
        ctx.compiler
            .record_diagnostic(*crate::eval::dangling_attributes_error(&pending_attrs));
    }

    // Pop the scope when exiting the block
    ctx.env.pop_scope();

    // Return the last expression's value
    Ok(result)
}

fn ir_block(
    args: &[Expr],
    block: &mut BlockBuilder,
    ctx: &mut IrGenContext,
    source: SourceLocation,
    gen_expr: &mut dyn FnMut(&Expr, &mut BlockBuilder, &mut IrGenContext) -> Result<ValueId>,
) -> Result<ValueId> {
    if args.is_empty() {
        // Return nil for empty blocks
        return Ok(block.const_val(crate::ir::IrConst::Nil, Type::Nil, source));
    }

    // Generate IR for each expression in sequence, returning the last value
    // Note: IR doesn't have lexical scoping - all variables are in the same SSA namespace
    let mut result = None;
    for expr in args {
        result = Some(gen_expr(expr, block, ctx)?);
    }

    // Return the last expression's value ID
    Ok(result.unwrap())
}

#[cfg(test)]
mod tests {
    use crate::{Compiler, Env};
    use cadenza_syntax::parse::parse;

    #[test]
    fn test_block_special_form_eval() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        // Test block with multiple expressions
        let input = r#"
let x =
    let y = 1
    let z = 2
    y + z
"#;
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert!(!results.is_empty(), "Expected at least one result");
        let value = &results[0];
        assert_eq!(*value, crate::Value::Integer(3));

        // Verify that y and z are not in scope outside the block
        let y_id: crate::interner::InternedString = "y".into();
        let z_id: crate::interner::InternedString = "z".into();
        assert_eq!(env.get(y_id), None);
        assert_eq!(env.get(z_id), None);
    }
}
