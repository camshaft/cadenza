//! The `@` special form for registering attributes.
use crate::{
    context::EvalContext,
    diagnostic::{Diagnostic, Result},
    ir::{BlockBuilder, IrGenContext, SourceLocation, ValueId},
    special_form::BuiltinSpecialForm,
    value::{Type, Value},
};
use cadenza_syntax::ast::Expr;
use std::sync::OnceLock;

pub fn get() -> &'static BuiltinSpecialForm {
    static ATTR_FORM: OnceLock<BuiltinSpecialForm> = OnceLock::new();
    ATTR_FORM.get_or_init(|| BuiltinSpecialForm {
        name: "@",
        signature: Type::function(vec![Type::Unknown], Type::Nil),
        eval_fn: eval_attr,
        ir_fn: ir_attr,
    })
}

fn eval_attr(args: &[Expr], ctx: &mut EvalContext<'_>) -> Result<Value> {
    if args.len() != 1 {
        return Err(Diagnostic::arity(1, args.len()));
    }
    ctx.add_attribute(args[0].clone());
    Ok(Value::Nil)
}

fn ir_attr(
    _args: &[Expr],
    _block: &mut BlockBuilder,
    _ctx: &mut IrGenContext,
    _source: SourceLocation,
    _gen_expr: &mut dyn FnMut(&Expr, &mut BlockBuilder, &mut IrGenContext) -> Result<ValueId>,
) -> Result<ValueId> {
    Err(Diagnostic::syntax("@ attributes are handled in evaluation only"))
}
