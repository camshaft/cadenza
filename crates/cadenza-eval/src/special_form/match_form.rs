//! The `match` special form for pattern matching.

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

/// Returns the `match` special form for pattern matching.
///
/// The `match` special form implements pattern matching on boolean values.
///
/// # Evaluation
/// - Takes at least 2 arguments: match expression and pattern arms
/// - Evaluates the match expression (must be boolean)
/// - Checks each pattern arm in order
/// - Pattern arms have syntax: `pattern -> result`
/// - Patterns must be `true` or `false`
/// - Returns the result of the first matching arm
///
/// # IR Generation
/// - Not yet implemented (returns error)
///
/// # Examples
/// ```cadenza
/// match x > 0
///     true -> "positive"
///     false -> "negative or zero"
/// ```
pub fn get() -> &'static BuiltinSpecialForm {
    static MATCH_FORM: OnceLock<BuiltinSpecialForm> = OnceLock::new();
    MATCH_FORM.get_or_init(|| BuiltinSpecialForm {
        name: "match",
        signature: Type::function(vec![Type::Unknown], Type::Unknown),
        eval_fn: eval_match,
        ir_fn: ir_match,
    })
}

fn eval_match(args: &[Expr], ctx: &mut EvalContext<'_>) -> Result<Value> {
    // Validate argument count: need match expression and at least one arm
    if args.len() < 2 {
        return Err(Diagnostic::syntax(
            "match expects at least 2 arguments: match_expr and pattern arms",
        ));
    }

    // First argument is the expression to match on
    let match_expr = &args[0];
    let match_value = match_expr.eval(ctx)?;

    // Check that the match value is a boolean
    let match_bool = match match_value {
        Value::Bool(b) => b,
        _ => {
            return Err(Diagnostic::type_error(Type::Bool, match_value.type_of())
                .with_span(match_expr.span()));
        }
    };

    // Remaining arguments are pattern arms
    // Each arm should be an arrow expression: pattern -> result
    for arm in &args[1..] {
        // Each arm should be an arrow expression: pattern -> result
        if let Expr::Apply(apply) = arm {
            // Check if the callee is the -> operator
            if let Some(Expr::Op(op)) = apply.callee() {
                if op.syntax().text() == "->" {
                    let arm_args = apply.all_arguments();
                    if arm_args.len() == 2 {
                        let pattern = &arm_args[0];
                        let result_expr = &arm_args[1];

                        // Check if pattern is a boolean literal
                        if let Expr::Ident(ident) = pattern {
                            let pattern_text = ident.syntax().text();
                            let matches = match pattern_text.as_str() {
                                "true" => match_bool,
                                "false" => !match_bool,
                                _ => {
                                    return Err(Diagnostic::syntax(format!(
                                        "match pattern must be 'true' or 'false', got '{}'",
                                        pattern_text.as_str()
                                    ))
                                    .with_span(pattern.span()));
                                }
                            };

                            if matches {
                                return result_expr.eval(ctx);
                            }
                        }
                    }
                }
            }
        }
    }

    // No pattern matched
    Err(
        Diagnostic::syntax("match expression did not match any pattern")
            .with_span(match_expr.span()),
    )
}

fn ir_match(
    _args: &[Expr],
    _block: &mut BlockBuilder,
    _ctx: &mut IrGenContext,
    _source: SourceLocation,
    _gen_expr: &mut dyn FnMut(&Expr, &mut BlockBuilder, &mut IrGenContext) -> Result<ValueId>,
) -> Result<ValueId> {
    Err(Diagnostic::syntax(
        "match special form IR generation not yet implemented (use ir_match_with_state instead)",
    ))
}

/// IR generation for match with multi-block support.
///
/// Generates control flow with branches for boolean pattern matching.
pub fn ir_match_with_state(
    args: &[Expr],
    state: &mut crate::ir::IrGenState,
    ctx: &mut IrGenContext,
    source: SourceLocation,
    gen_expr: &mut dyn FnMut(
        &Expr,
        &mut crate::ir::IrGenState,
        &mut IrGenContext,
    ) -> Result<ValueId>,
) -> Result<ValueId> {
    // Validate argument count: need match expression and at least one arm
    if args.len() < 2 {
        return Err(Diagnostic::syntax(
            "match expects at least 2 arguments: match_expr and pattern arms",
        ));
    }

    // First argument is the expression to match on (must be boolean)
    let match_expr = &args[0];

    // Parse the arms to find the then and else branches
    // Each arm should be: pattern -> result
    let mut then_expr = None;
    let mut else_expr = None;

    for arm in &args[1..] {
        if let Expr::Apply(apply) = arm {
            if let Some(Expr::Op(op)) = apply.callee() {
                if op.syntax().text() == "->" {
                    let arm_args = apply.all_arguments();
                    if arm_args.len() == 2 {
                        let pattern = &arm_args[0];
                        let result_expr = &arm_args[1];

                        if let Expr::Ident(ident) = pattern {
                            let pattern_text = ident.syntax().text();
                            match pattern_text.as_str() {
                                "true" => then_expr = Some(result_expr.clone()),
                                "false" => else_expr = Some(result_expr.clone()),
                                _ => {
                                    return Err(Diagnostic::syntax(format!(
                                        "match pattern must be 'true' or 'false', got '{}'",
                                        pattern_text.as_str()
                                    ))
                                    .with_span(pattern.span()));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let then_expr =
        then_expr.ok_or_else(|| Diagnostic::syntax("match expression missing 'true' pattern"))?;
    let else_expr =
        else_expr.ok_or_else(|| Diagnostic::syntax("match expression missing 'false' pattern"))?;

    // Generate the condition in the entry block
    let cond = gen_expr(match_expr, state, ctx)?;

    // Allocate block IDs for then, else, and merge blocks
    let then_block_id = state.alloc_block_id();
    let else_block_id = state.alloc_block_id();
    let merge_block_id = state.alloc_block_id();

    // Complete the entry block with a branch instruction
    let current = state
        .current_block
        .take()
        .expect("No entry block available for branch instruction");
    let (entry_block, next_val) = current.branch(cond, then_block_id, else_block_id, source);
    state.complete_current_block(entry_block, next_val);

    // Now create the then block (gets fresh value IDs after entry block)
    let then_block = state.create_block_with_id(then_block_id);
    state.current_block = Some(then_block);
    let then_value = gen_expr(&then_expr, state, ctx)?;
    let then_block = state
        .current_block
        .take()
        .expect("Current block missing after generating then branch");
    let (then_block_complete, then_next_val) = then_block.jump(merge_block_id, source);
    state.complete_current_block(then_block_complete, then_next_val);

    // Create the else block (gets fresh value IDs after then block)
    let else_block = state.create_block_with_id(else_block_id);
    state.current_block = Some(else_block);
    let else_value = gen_expr(&else_expr, state, ctx)?;
    let else_block = state
        .current_block
        .take()
        .expect("Current block missing after generating else branch");
    let (else_block_complete, else_next_val) = else_block.jump(merge_block_id, source);
    state.complete_current_block(else_block_complete, else_next_val);

    // Create the merge block with phi node
    let merge_block = state.create_block_with_id(merge_block_id);
    let mut merge = merge_block;
    let incoming = vec![(then_value, then_block_id), (else_value, else_block_id)];

    // Infer the type from one of the branches (they should have the same type)
    // For now, use Unknown - proper type inference would check both branches
    let result_ty = Type::Unknown;

    let result = merge.phi(incoming, result_ty, source);

    // Set merge block as current block
    state.current_block = Some(merge);

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Compiler, Env};
    use cadenza_syntax::parse::parse;

    #[test]
    fn test_match_true_branch() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = "(match true (true -> 1) (false -> 2))";
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0], Value::Integer(1));
    }

    #[test]
    fn test_match_false_branch() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = "(match false (true -> 1) (false -> 2))";
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0], Value::Integer(2));
    }

    #[test]
    fn test_match_with_expression() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        // Define the variable first
        let input1 = "let x = 5";
        let parsed1 = parse(input1);
        let root1 = parsed1.ast();
        let _results1 = crate::eval(&root1, &mut env, &mut compiler);

        // Then do the match
        let input2 = "(match (x > 0) (true -> \"positive\") (false -> \"not positive\"))";
        let parsed2 = parse(input2);
        let root2 = parsed2.ast();
        let results2 = crate::eval(&root2, &mut env, &mut compiler);

        assert_eq!(results2.len(), 1);
        assert_eq!(results2[0], Value::String("positive".into()));
    }
}
