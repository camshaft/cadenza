//! The `.` special form for field access.

use crate::{
    Eval,
    context::EvalContext,
    diagnostic::{BoxedDiagnosticExt, Diagnostic, Result},
    interner::InternedString,
    ir::{BlockBuilder, IrGenContext, SourceLocation, ValueId},
    special_form::BuiltinSpecialForm,
    value::{Type, Value},
};
use cadenza_syntax::ast::Expr;
use std::sync::OnceLock;

/// Returns the `.` special form for field access.
///
/// The `.` special form accesses a field from a record value.
///
/// # Evaluation
/// - Takes exactly 2 arguments: record expression and field name identifier
/// - Evaluates the record expression
/// - Extracts the field name (unevaluated identifier)
/// - Returns the field value from the record
///
/// # IR Generation
/// - Not yet implemented (returns error)
///
/// # Examples
/// ```cadenza
/// let point = { x = 10, y = 20 }
/// point.x  # returns 10
/// point.y  # returns 20
/// ```
pub fn get() -> &'static BuiltinSpecialForm {
    static FIELD_ACCESS_FORM: OnceLock<BuiltinSpecialForm> = OnceLock::new();
    FIELD_ACCESS_FORM.get_or_init(|| BuiltinSpecialForm {
        name: ".",
        signature: Type::function(vec![Type::Unknown, Type::Symbol], Type::Unknown),
        eval_fn: eval_field_access,
        ir_fn: ir_field_access,
    })
}

fn eval_field_access(args: &[Expr], ctx: &mut EvalContext<'_>) -> Result<Value> {
    // Field access requires exactly 2 arguments: record and field name
    if args.len() != 2 {
        return Err(Diagnostic::arity(2, args.len()));
    }

    // Evaluate the record (first argument)
    let record_value = args[0].eval(ctx)?;

    // Extract the field name from the second argument (must be an identifier)
    let (field_name, field_span) = match &args[1] {
        Expr::Ident(ident) => {
            let text = ident.syntax().text();
            let id: InternedString = text.to_string().as_str().into();
            (id, ident.span())
        }
        _ => return Err(Diagnostic::syntax("field name must be an identifier")),
    };

    // Extract the record fields
    match record_value {
        Value::Record { type_name, fields } => {
            // Look up the field in the record (works for both structural and nominal)
            for (name, value) in fields {
                if name == field_name {
                    return Ok(value);
                }
            }
            // Field not found - include type name in error if it's a struct
            let type_description = match type_name {
                Some(name) => format!("struct {}", &*name),
                None => "record".to_string(),
            };
            Err(Diagnostic::syntax(format!(
                "field '{}' not found in {}",
                &*field_name, type_description
            ))
            .with_span(field_span))
        }
        other => Err(Diagnostic::type_error(
            Type::Record(vec![]),
            other.type_of(),
        )),
    }
}

fn ir_field_access(
    _args: &[Expr],
    _block: &mut BlockBuilder,
    _ctx: &mut IrGenContext,
    _source: SourceLocation,
    _gen_expr: &mut dyn FnMut(&Expr, &mut BlockBuilder, &mut IrGenContext) -> Result<ValueId>,
) -> Result<ValueId> {
    Err(Diagnostic::syntax(
        ". special form IR generation not yet implemented",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Compiler, Env};
    use cadenza_syntax::parse::parse;

    #[test]
    fn test_field_access_basic() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = r#"
let point = { x = 10, y = 20 }
point.x
"#;
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert_eq!(results.len(), 2);
        assert_eq!(results[1], Value::Integer(10));
    }

    #[test]
    fn test_field_access_multiple_fields() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = r#"
let point = { x = 10, y = 20 }
point.y
"#;
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert_eq!(results.len(), 2);
        assert_eq!(results[1], Value::Integer(20));
    }

    #[test]
    fn test_field_access_on_expression() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = r#"
fn make_point x y = { x = x, y = y }
(make_point 5 10).x
"#;
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert_eq!(results.len(), 2);
        assert_eq!(results[1], Value::Integer(5));
    }
}
