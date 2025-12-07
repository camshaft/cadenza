//! The `__record__` special form for record literals.

use crate::{
    context::EvalContext,
    diagnostic::{BoxedDiagnosticExt, Diagnostic, Result},
    interner::InternedString,
    ir::{BlockBuilder, IrGenContext, SourceLocation, ValueId},
    special_form::BuiltinSpecialForm,
    value::{Type, Value},
    Eval,
};
use cadenza_syntax::ast::Expr;
use std::sync::OnceLock;

/// Returns the `__record__` special form for record literals.
///
/// The `__record__` special form creates a record value from field assignments or shorthand syntax.
///
/// # Evaluation
/// - Takes variable number of arguments (field assignments or identifiers)
/// - Each argument can be:
///   1. An assignment expression: `[=, field_name, value_expr]`
///   2. A shorthand identifier: just the field name (expands to `field = field`)
/// - Returns a Record value with evaluated fields
///
/// # IR Generation
/// - Not yet implemented (returns error)
///
/// # Examples
/// ```cadenza
/// { a = 1, b = 2 }  // Full syntax
/// { x, y }          // Shorthand syntax (uses x and y from environment)
/// {}                // Empty record
/// ```
pub fn get() -> &'static BuiltinSpecialForm {
    static RECORD_FORM: OnceLock<BuiltinSpecialForm> = OnceLock::new();
    RECORD_FORM.get_or_init(|| BuiltinSpecialForm {
        name: "__record__",
        signature: Type::function(vec![], Type::Record(vec![])),
        eval_fn: eval_record,
        ir_fn: ir_record,
    })
}

fn eval_record(args: &[Expr], ctx: &mut EvalContext<'_>) -> Result<Value> {
    // Each argument can be either:
    // 1. An assignment expression: [=, field_name, value_expr]
    // 2. A shorthand identifier: just the field name (expands to field = field)
    let mut fields = Vec::with_capacity(args.len());

    for arg in args {
        match arg {
            // Shorthand syntax: { x, y } where x and y are identifiers
            Expr::Ident(ident) => {
                let text = ident.syntax().text();
                let field_name = InternedString::new(&text.to_string());

                // Look up the variable in the environment
                let value = ctx.env.get(field_name).cloned().ok_or_else(|| {
                    Diagnostic::undefined_variable(field_name).with_span(ident.span())
                })?;

                fields.push((field_name, value));
            }
            // Full syntax: { a = 1, b = 2 }
            Expr::Apply(apply) => {
                // Get all arguments once to avoid duplicate calls
                let all_args = apply.all_arguments();
                if all_args.len() != 2 {
                    return Err(Diagnostic::syntax(
                        "record field assignment must have exactly 2 arguments",
                    ));
                }

                // Extract the field name (should be an identifier)
                let field_name = match &all_args[0] {
                    Expr::Ident(ident) => {
                        let text = ident.syntax().text();
                        InternedString::new(&text.to_string())
                    }
                    _ => {
                        return Err(Diagnostic::syntax(
                            "record field name must be an identifier",
                        ));
                    }
                };

                // Evaluate the field value (second arg)
                let value = all_args[1].eval(ctx)?;

                fields.push((field_name, value));
            }
            _ => {
                return Err(Diagnostic::syntax(
                    "record field must be an identifier or assignment expression",
                ));
            }
        }
    }

    // Return the record value
    Ok(Value::Record(fields))
}

fn ir_record(
    _args: &[Expr],
    _block: &mut BlockBuilder,
    _ctx: &mut IrGenContext,
    _source: SourceLocation,
    _gen_expr: &mut dyn FnMut(&Expr, &mut BlockBuilder, &mut IrGenContext) -> Result<ValueId>,
) -> Result<ValueId> {
    Err(Diagnostic::syntax(
        "__record__ special form IR generation not yet implemented",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Compiler, Env};
    use cadenza_syntax::parse::parse;

    #[test]
    fn test_record_full_syntax() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = "{ a = 1, b = 2 }";
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert_eq!(results.len(), 1);
        match &results[0] {
            Value::Record(fields) => {
                assert_eq!(fields.len(), 2);
                assert_eq!(fields[0].0, InternedString::new("a"));
                assert_eq!(fields[0].1, Value::Integer(1));
                assert_eq!(fields[1].0, InternedString::new("b"));
                assert_eq!(fields[1].1, Value::Integer(2));
            }
            _ => panic!("Expected Record value"),
        }
    }

    #[test]
    fn test_record_shorthand_syntax() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = r#"
let x = 10
let y = 20
{ x, y }
"#;
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert_eq!(results.len(), 3);
        match &results[2] {
            Value::Record(fields) => {
                assert_eq!(fields.len(), 2);
                assert_eq!(fields[0].0, InternedString::new("x"));
                assert_eq!(fields[0].1, Value::Integer(10));
                assert_eq!(fields[1].0, InternedString::new("y"));
                assert_eq!(fields[1].1, Value::Integer(20));
            }
            _ => panic!("Expected Record value"),
        }
    }

    #[test]
    fn test_empty_record() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = "{}";
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert_eq!(results.len(), 1);
        match &results[0] {
            Value::Record(fields) => {
                assert_eq!(fields.len(), 0);
            }
            _ => panic!("Expected Record value"),
        }
    }
}
