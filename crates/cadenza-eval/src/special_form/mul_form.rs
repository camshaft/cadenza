//! The `*` special form for multiplication.

use crate::{
    Eval,
    context::EvalContext,
    diagnostic::{Diagnostic, Result},
    ir::{BinOp, BlockBuilder, IrGenContext, SourceLocation, ValueId},
    special_form::BuiltinSpecialForm,
    value::{Type, Value},
};
use cadenza_syntax::ast::Expr;
use std::sync::OnceLock;

/// Returns the `*` special form for multiplication.
///
/// # Evaluation
/// - Takes exactly 2 arguments
/// - Evaluates both arguments
/// - Multiplies them together (integers or floats)
///
/// # IR Generation
/// - Generates IR for both operands
/// - Emits a binary mul instruction
///
/// # Examples
/// ```cadenza
/// 2 * 3        # returns 6
/// 2.5 * 4.0    # returns 10.0
/// ```
pub fn get() -> &'static BuiltinSpecialForm {
    static FORM: OnceLock<BuiltinSpecialForm> = OnceLock::new();
    FORM.get_or_init(|| BuiltinSpecialForm {
        name: "*",
        signature: Type::union(vec![
            Type::function(vec![Type::Integer, Type::Integer], Type::Integer),
            Type::function(vec![Type::Float, Type::Float], Type::Float),
        ]),
        eval_fn: eval_mul,
        ir_fn: ir_mul,
    })
}

fn eval_mul(args: &[Expr], ctx: &mut EvalContext<'_>) -> Result<Value> {
    if args.len() != 2 {
        return Err(Diagnostic::arity(2, args.len()));
    }

    // Evaluate both arguments
    let lhs = args[0].eval(ctx)?;
    let rhs = args[1].eval(ctx)?;

    match (&lhs, &rhs) {
        (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(a * b)),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
        // Type mismatch - reject mixed integer/float operations
        (Value::Integer(_), Value::Float(_)) | (Value::Float(_), Value::Integer(_)) => {
            Err(Diagnostic::type_error(lhs.type_of(), rhs.type_of()))
        }
        // For non-numeric types, report type error
        (Value::Integer(_), b) | (Value::Float(_), b) => Err(Diagnostic::type_error(
            Type::union(vec![Type::Integer, Type::Float]),
            b.type_of(),
        )),
        (a, _) => Err(Diagnostic::type_error(
            Type::union(vec![Type::Integer, Type::Float]),
            a.type_of(),
        )),
    }
}

fn ir_mul(
    args: &[Expr],
    block: &mut BlockBuilder,
    ctx: &mut IrGenContext,
    source: SourceLocation,
    gen_expr: &mut dyn FnMut(&Expr, &mut BlockBuilder, &mut IrGenContext) -> Result<ValueId>,
) -> Result<ValueId> {
    if args.len() != 2 {
        return Err(Diagnostic::syntax(format!(
            "Binary operator * expects 2 arguments, got {}",
            args.len()
        )));
    }

    // Generate IR for both operands
    let lhs = gen_expr(&args[0], block, ctx)?;
    let rhs = gen_expr(&args[1], block, ctx)?;

    // Infer the result type
    // For now, use Unknown - proper type inference would go here
    let ty = Type::Unknown;

    // Emit binary mul instruction
    Ok(block.binop(BinOp::Mul, lhs, rhs, ty, source))
}
