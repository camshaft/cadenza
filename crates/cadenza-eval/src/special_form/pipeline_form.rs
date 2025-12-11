//! The `|>` special form for pipeline operator.

use crate::{
    Eval,
    context::EvalContext,
    diagnostic::{Diagnostic, Result},
    eval::{apply_value, eval_ident_no_auto_apply, extract_identifier},
    ir::{BlockBuilder, IrGenContext, SourceLocation, ValueId},
    special_form::BuiltinSpecialForm,
    value::{Type, Value},
};
use cadenza_syntax::ast::Expr;
use std::sync::OnceLock;

/// Returns the `|>` special form for pipeline operator.
///
/// The `|>` special form pipes a value through a function, injecting it as the first argument.
///
/// # Evaluation
/// - Takes exactly 2 arguments: LHS value expression and RHS function application
/// - Evaluates the LHS to get the value to pipe
/// - The RHS can be:
///   1. A function identifier (e.g., `|> f` means `f lhs_value`)
///   2. A function application (e.g., `|> f x y` means `f lhs_value x y`)
/// - Returns the result of the function application
///
/// # IR Generation
/// - Not yet implemented (returns error)
///
/// # Examples
/// ```cadenza
/// 5 |> add 3           // Equivalent to: add 5 3
/// 10 |> sub 2 |> mul 3 // Equivalent to: mul (sub 10 2) 3
/// x |> f |> g          // Equivalent to: g (f x)
/// ```
pub fn get() -> &'static BuiltinSpecialForm {
    static PIPELINE_FORM: OnceLock<BuiltinSpecialForm> = OnceLock::new();
    PIPELINE_FORM.get_or_init(|| BuiltinSpecialForm {
        name: "|>",
        signature: Type::function(vec![Type::Unknown, Type::Unknown], Type::Unknown),
        eval_fn: eval_pipeline,
        ir_fn: ir_pipeline,
    })
}

fn eval_pipeline(args: &[Expr], ctx: &mut EvalContext<'_>) -> Result<Value> {
    // Pipeline requires exactly 2 arguments: LHS value and RHS function application
    if args.len() != 2 {
        return Err(Diagnostic::arity(2, args.len()));
    }

    // Evaluate the LHS to get the value to pipe
    let lhs_value = ctx.eval_child(&args[0])?;

    // The RHS should be either:
    // 1. A function identifier (e.g., `|> f` means `f lhs_value`)
    // 2. A function application (e.g., `|> f x y` means `f lhs_value x y`)
    match &args[1] {
        // Case 1: RHS is just an identifier - apply it to the LHS value
        Expr::Ident(ident) => {
            // Look up the identifier without auto-applying
            let func = eval_ident_no_auto_apply(ident, ctx)?;
            // Apply the function to the LHS value
            apply_value(func, vec![lhs_value], ctx)
        }
        // Case 2: RHS is an application - inject LHS as first argument
        Expr::Apply(apply) => {
            // Get the callee
            let callee_expr = apply
                .callee()
                .ok_or_else(|| Diagnostic::syntax("missing callee in pipeline"))?;

            // Try to extract an identifier/operator name from the callee.
            // If successful, check if it names a macro before evaluating.
            if let Some(id) = extract_identifier(&callee_expr) {
                // Check for macro in compiler
                if ctx.compiler.get_macro(id).is_some() {
                    // Macros expect unevaluated AST expressions to enable compile-time
                    // transformations and syntax manipulation. The pipeline operator
                    // fundamentally conflicts with this because it must evaluate the LHS
                    // value before piping it. Since we can't "un-evaluate" a value back
                    // into an AST expression, piping into macros is not supported.
                    return Err(Diagnostic::syntax(
                        "cannot use pipeline operator with macros",
                    ));
                }

                // Check for macro in environment
                if let Some(Value::BuiltinMacro(_) | Value::SpecialForm(_)) = ctx.env.get(id) {
                    return Err(Diagnostic::syntax(
                        "cannot use pipeline operator with macros or special forms",
                    ));
                }
            }

            // Not a macro - evaluate the callee normally
            let callee = ctx.with_attribute_scope(Vec::new(), |ctx| match &callee_expr {
                Expr::Ident(ident) => eval_ident_no_auto_apply(ident, ctx),
                Expr::Op(op) => {
                    // Use extract_identifier to get the operator name
                    let id = extract_identifier(&callee_expr)
                        .ok_or_else(|| Diagnostic::syntax("invalid operator"))?;
                    let span = op.span();
                    let value = ctx
                        .env
                        .get(id)
                        .cloned()
                        .ok_or_else(|| Diagnostic::undefined_variable(id).with_span(span))?;
                    Ok(value)
                }
                _ => ctx.eval_child(&callee_expr),
            })?;

            // Get all RHS arguments and evaluate them
            let rhs_arg_exprs = apply.all_arguments();
            let mut all_args = Vec::with_capacity(1 + rhs_arg_exprs.len());

            // Inject the LHS value as the first argument
            all_args.push(lhs_value);

            // Then add the evaluated RHS arguments
            for arg_expr in rhs_arg_exprs {
                let value = ctx.eval_child(&arg_expr)?;
                all_args.push(value);
            }

            // Apply the function with the combined arguments
            apply_value(callee, all_args, ctx)
        }
        _ => {
            // RHS must be either an identifier or an application
            Err(Diagnostic::syntax(
                "right side of pipeline must be a function or function application",
            ))
        }
    }
}

fn ir_pipeline(
    _args: &[Expr],
    _block: &mut BlockBuilder,
    _ctx: &mut IrGenContext,
    _source: SourceLocation,
    _gen_expr: &mut dyn FnMut(&Expr, &mut BlockBuilder, &mut IrGenContext) -> Result<ValueId>,
) -> Result<ValueId> {
    Err(Diagnostic::syntax(
        "|> special form IR generation not yet implemented",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Compiler, Env};
    use cadenza_syntax::parse::parse;

    #[test]
    fn test_pipeline_simple() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        // Define helper function
        let input = r#"
fn add_three x = x + 3
5 |> add_three
"#;
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert_eq!(results.len(), 2);
        assert_eq!(results[1], Value::Integer(8));
    }

    #[test]
    fn test_pipeline_with_args() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = "5 |> + 3";
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0], Value::Integer(8));
    }

    #[test]
    fn test_pipeline_chained() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = "10 |> - 2 |> * 3";
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0], Value::Integer(24));
    }
}
