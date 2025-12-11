//! The `||` special form for logical OR.

use crate::{
    context::EvalContext,
    diagnostic::{Diagnostic, Result},
    ir::{BinOp, BlockBuilder, IrGenContext, SourceLocation, ValueId},
    special_form::BuiltinSpecialForm,
    value::{Type, Value},
};
use cadenza_syntax::ast::Expr;
use std::sync::OnceLock;

/// Returns the `||` special form for logical OR.
pub fn get() -> &'static BuiltinSpecialForm {
    static FORM: OnceLock<BuiltinSpecialForm> = OnceLock::new();
    FORM.get_or_init(|| BuiltinSpecialForm {
        name: "||",
        signature: Type::function(vec![Type::Bool, Type::Bool], Type::Bool),
        eval_fn: eval_or,
        ir_fn: ir_or,
    })
}

fn eval_or(args: &[Expr], ctx: &mut EvalContext<'_>) -> Result<Value> {
    if args.len() != 2 {
        return Err(Diagnostic::arity(2, args.len()));
    }

    // Evaluate left operand first
    let lhs = ctx.eval_child(&args[0])?;

    match lhs {
        Value::Bool(true) => {
            // Short-circuit: if left is true, don't evaluate right
            Ok(Value::Bool(true))
        }
        Value::Bool(false) => {
            // Left is false, need to evaluate right
            let rhs = ctx.eval_child(&args[1])?;
            match rhs {
                Value::Bool(b) => Ok(Value::Bool(b)),
                _ => Err(Diagnostic::type_error(Type::Bool, rhs.type_of())),
            }
        }
        _ => Err(Diagnostic::type_error(Type::Bool, lhs.type_of())),
    }
}

fn ir_or(
    args: &[Expr],
    block: &mut BlockBuilder,
    ctx: &mut IrGenContext,
    source: SourceLocation,
    gen_expr: &mut dyn FnMut(&Expr, &mut BlockBuilder, &mut IrGenContext) -> Result<ValueId>,
) -> Result<ValueId> {
    if args.len() != 2 {
        return Err(Diagnostic::syntax(format!(
            "Binary operator || expects 2 arguments, got {}",
            args.len()
        )));
    }

    let lhs = gen_expr(&args[0], block, ctx)?;
    let rhs = gen_expr(&args[1], block, ctx)?;
    Ok(block.binop(BinOp::Or, lhs, rhs, Type::Bool, source))
}
