//! The `>` special form for greater-than comparison.

use crate::{
    context::EvalContext,
    diagnostic::{Diagnostic, Result},
    ir::{BinOp, BlockBuilder, IrGenContext, SourceLocation, ValueId},
    special_form::BuiltinSpecialForm,
    value::{Type, Value},
};
use cadenza_syntax::ast::Expr;
use std::sync::OnceLock;

/// Returns the `>` special form for greater-than comparison.
pub fn get() -> &'static BuiltinSpecialForm {
    static FORM: OnceLock<BuiltinSpecialForm> = OnceLock::new();
    FORM.get_or_init(|| BuiltinSpecialForm {
        name: ">",
        signature: Type::function(vec![Type::Unknown, Type::Unknown], Type::Bool),
        eval_fn: eval_gt,
        ir_fn: ir_gt,
    })
}

fn eval_gt(args: &[Expr], ctx: &mut EvalContext<'_>) -> Result<Value> {
    if args.len() != 2 {
        return Err(Diagnostic::arity(2, args.len()));
    }

    let lhs = ctx.eval_child(&args[0])?;
    let rhs = ctx.eval_child(&args[1])?;

    // Require exact type match for comparison
    if lhs.type_of() != rhs.type_of() {
        return Err(Diagnostic::type_error(lhs.type_of(), rhs.type_of()));
    }

    match (&lhs, &rhs) {
        (Value::Integer(a), Value::Integer(b)) => Ok(Value::Bool(a > b)),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a > b)),
        _ => Err(Diagnostic::syntax(format!(
            "Cannot compare {} values with >",
            lhs.type_of()
        ))),
    }
}

fn ir_gt(
    args: &[Expr],
    block: &mut BlockBuilder,
    ctx: &mut IrGenContext,
    source: SourceLocation,
    gen_expr: &mut dyn FnMut(&Expr, &mut BlockBuilder, &mut IrGenContext) -> Result<ValueId>,
) -> Result<ValueId> {
    if args.len() != 2 {
        return Err(Diagnostic::syntax(format!(
            "Binary operator > expects 2 arguments, got {}",
            args.len()
        )));
    }

    let lhs = gen_expr(&args[0], block, ctx)?;
    let rhs = gen_expr(&args[1], block, ctx)?;
    let result = block.binop(BinOp::Gt, lhs, rhs, Type::Bool, source);
    ctx.set_value_type(result, Type::Bool);
    Ok(result)
}
