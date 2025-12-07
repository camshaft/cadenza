//! The `=` special form for variable assignment and macro delegation.

use crate::{
    context::EvalContext,
    diagnostic::{BoxedDiagnosticExt, Diagnostic, Result},
    interner::InternedString,
    ir::{BlockBuilder, IrGenContext, SourceLocation, ValueId},
    special_form::BuiltinSpecialForm,
    value::{Type, Value},
    Eval,
};
use cadenza_syntax::ast::{Apply, Expr};
use std::sync::OnceLock;

/// Returns the `=` special form for assignment and macro delegation.
///
/// # Evaluation
/// The `=` operator takes two arguments:
/// 1. The left-hand side (which can be a macro application or an identifier)
/// 2. The right-hand side (the value to assign or pass to the macro)
///
/// When the LHS is a macro application (e.g., `let x`, `fn name params...`, `measure name`),
/// the `=` operator delegates to that macro by calling it with `[lhs_args..., rhs]`.
///
/// When the LHS is a plain identifier, `=` performs a direct reassignment to that variable.
///
/// # IR Generation
/// Not yet supported in IR generation.
///
/// # Examples
/// ```cadenza
/// let x = 42          # Delegates to `let` with `[x, 42]`
/// x = 50              # Direct reassignment to existing variable `x`
/// record.field = 100  # Field assignment
/// ```
pub fn get() -> &'static BuiltinSpecialForm {
    static FORM: OnceLock<BuiltinSpecialForm> = OnceLock::new();
    FORM.get_or_init(|| BuiltinSpecialForm {
        name: "=",
        signature: Type::function(vec![Type::Unknown, Type::Unknown], Type::Unknown),
        eval_fn: eval_assign,
        ir_fn: ir_assign,
    })
}

fn eval_assign(args: &[Expr], ctx: &mut EvalContext<'_>) -> Result<Value> {
    // Expect exactly two arguments
    if args.len() != 2 {
        return Err(Diagnostic::arity(2, args.len()));
    }

    // Get the LHS and RHS expressions
    let lhs_expr = &args[0];
    let rhs_expr = &args[1];

    // Check if LHS is a macro application - delegate if so
    // EXCEPT for field access (.) which should be handled as field assignment
    if let Expr::Apply(apply) = lhs_expr {
        if let Some(callee_expr) = apply.callee() {
            // Check if this is field access - handle separately
            if let Expr::Op(op) = &callee_expr {
                if op.syntax().text() == "." {
                    // This is field assignment: record.field = value
                    return handle_field_assignment(apply, rhs_expr, ctx);
                }
            }

            // Try to extract an identifier from the callee
            if let Some(id) = extract_identifier(&callee_expr) {
                // Check if this identifier refers to a macro
                let macro_value = if let Some(value) = ctx.compiler.get_macro(id) {
                    Some(value.clone())
                } else {
                    ctx.env.get(id).and_then(|v| match v {
                        Value::BuiltinMacro(_) | Value::SpecialForm(_) => Some(v.clone()),
                        _ => None,
                    })
                };

                match macro_value {
                    Some(Value::BuiltinMacro(builtin)) => {
                        // This is a macro! Delegate to it with [lhs_args..., rhs]
                        let lhs_args = apply.all_arguments();
                        let mut new_args = Vec::with_capacity(lhs_args.len() + 1);
                        new_args.extend(lhs_args);
                        new_args.push(rhs_expr.clone());

                        // Call the macro directly
                        return (builtin.func)(&new_args, ctx);
                    }
                    Some(Value::SpecialForm(sf)) => {
                        // This is a special form! Delegate to it with [lhs_args..., rhs]
                        let lhs_args = apply.all_arguments();
                        let mut new_args = Vec::with_capacity(lhs_args.len() + 1);
                        new_args.extend(lhs_args);
                        new_args.push(rhs_expr.clone());

                        // Call the special form directly
                        return sf.eval(&new_args, ctx);
                    }
                    _ => {}
                }
            }
        }
    }

    // LHS is not a macro application - handle as direct identifier reassignment
    match lhs_expr {
        Expr::Ident(ident) => {
            let text = ident.syntax().text();
            let name: InternedString = text.to_string().as_str().into();

            let rhs_value = rhs_expr.eval(ctx)?;

            // Check if the variable exists (must be declared with `let` first)
            if let Some(var) = ctx.env.get_mut(name) {
                *var = rhs_value.clone();
                Ok(rhs_value)
            } else {
                Err(Diagnostic::undefined_variable(name).with_span(ident.span()))
            }
        }
        _ => Err(Diagnostic::syntax(
            "left side of = must be an identifier, field access (e.g., record.field), or a macro application (e.g., let x, fn name, measure unit)",
        )),
    }
}

fn ir_assign(
    _args: &[Expr],
    _block: &mut BlockBuilder,
    _ctx: &mut IrGenContext,
    _source: SourceLocation,
    _gen_expr: &mut dyn FnMut(&Expr, &mut BlockBuilder, &mut IrGenContext) -> Result<ValueId>,
) -> Result<ValueId> {
    Err(Diagnostic::syntax(
        "assignment operator not yet supported in IR generation",
    ))
}

/// Extracts an identifier from an expression if possible.
fn extract_identifier(expr: &Expr) -> Option<InternedString> {
    match expr {
        Expr::Ident(ident) => {
            let text = ident.syntax().text();
            Some(text.to_string().as_str().into())
        }
        Expr::Apply(apply) => {
            // Recursive case: try to extract from the callee
            apply.callee().and_then(|callee| extract_identifier(&callee))
        }
        _ => None,
    }
}

/// Handles field assignment of the form: record.field = value
///
/// The apply expression represents the field access (e.g., `record.field`),
/// and rhs_expr is the value to assign.
fn handle_field_assignment(
    apply: &Apply,
    rhs_expr: &Expr,
    ctx: &mut EvalContext<'_>,
) -> Result<Value> {
    // Field assignment requires exactly 2 arguments in the apply: record and field
    let args = apply.all_arguments();
    if args.len() != 2 {
        return Err(Diagnostic::syntax(
            "field assignment requires exactly record and field name",
        ));
    }

    // Get the record identifier (first argument)
    let (record_name, record_span) = match &args[0] {
        Expr::Ident(ident) => {
            let text = ident.syntax().text();
            let id: InternedString = text.to_string().as_str().into();
            (id, ident.span())
        }
        _ => {
            return Err(Diagnostic::syntax(
                "field assignment requires a variable name for the record",
            ));
        }
    };

    // Get the field name (second argument)
    let field_name = match &args[1] {
        Expr::Ident(ident) => {
            let text = ident.syntax().text();
            let id: InternedString = text.to_string().as_str().into();
            id
        }
        _ => return Err(Diagnostic::syntax("field name must be an identifier")),
    };

    // Evaluate the RHS value
    let new_value = rhs_expr.eval(ctx)?;

    // Get a mutable reference to the record from the environment
    let record = ctx
        .env
        .get_mut(record_name)
        .ok_or_else(|| Diagnostic::undefined_variable(record_name).with_span(record_span))?;

    // Update the field in the record
    match record {
        Value::Record(fields) => {
            // Find and update the field
            let mut found = false;
            for (name, value) in fields.iter_mut() {
                if *name == field_name {
                    // Check that the new value's type matches the old value's type
                    let old_type = value.type_of();
                    let new_type = new_value.type_of();
                    if old_type != new_type {
                        return Err(Diagnostic::type_error(old_type, new_type));
                    }
                    *value = new_value.clone();
                    found = true;
                    break;
                }
            }

            if found {
                Ok(new_value)
            } else {
                Err(Diagnostic::syntax(format!(
                    "field '{}' not found in record",
                    &*field_name
                )))
            }
        }
        _ => Err(Diagnostic::type_error(
            Type::Record(vec![]),
            record.type_of(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Compiler, Env};
    use cadenza_syntax::parse::parse;

    #[test]
    fn test_direct_assignment() {
        let src = "let x = 1\nx = 2";
        let parsed = parse(src);
        assert!(parsed.errors.is_empty());

        let root = parsed.ast();
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();
        let mut ctx = EvalContext::new(&mut env, &mut compiler);

        let results: Vec<_> = root
            .items()
            .map(|expr| expr.eval(&mut ctx))
            .collect::<Result<_>>()
            .unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[1], Value::Integer(2));

        // Verify the variable was updated
        let x_id: InternedString = "x".into();
        assert_eq!(env.get(x_id), Some(&Value::Integer(2)));
    }

    #[test]
    fn test_field_assignment() {
        let src = "let rec = { x = 1 }\nrec.x = 42";
        let parsed = parse(src);
        assert!(parsed.errors.is_empty());

        let root = parsed.ast();
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();
        let mut ctx = EvalContext::new(&mut env, &mut compiler);

        let results: Vec<_> = root
            .items()
            .map(|expr| expr.eval(&mut ctx))
            .collect::<Result<_>>()
            .unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[1], Value::Integer(42));

        // Verify the record field was updated
        let rec_id: InternedString = "rec".into();
        let x_id: InternedString = "x".into();
        
        if let Some(Value::Record(fields)) = env.get(rec_id) {
            let x_field = fields.iter().find(|(k, _)| *k == x_id);
            assert!(x_field.is_some(), "Field 'x' not found in record");
            assert_eq!(x_field.unwrap().1, Value::Integer(42), "Field 'x' has wrong value");
        } else {
            panic!("Expected record value");
        }
    }
}
