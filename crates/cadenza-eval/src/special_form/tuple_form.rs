//! The `__tuple__` special form for tuple literals.

use crate::{
    Eval,
    context::EvalContext,
    diagnostic::{Diagnostic, Result},
    ir::{BlockBuilder, IrGenContext, SourceLocation, ValueId},
    special_form::BuiltinSpecialForm,
    value::{Type, Value},
};
use cadenza_syntax::ast::Expr;
use std::sync::OnceLock;

/// Returns the `__tuple__` special form for tuple literals.
///
/// The `__tuple__` special form creates a tuple value from positional arguments.
///
/// # Evaluation
/// - Takes variable number of arguments (tuple elements)
/// - Each argument is evaluated and added to the tuple in order
/// - Returns a Tuple value with evaluated elements
///
/// # IR Generation
/// - Not yet implemented (returns error)
///
/// # Examples
/// ```cadenza
/// (1, 2, 3)     // Tuple with three elements
/// (x, y)        // Tuple with two elements
/// ()            // Empty tuple
/// (42,)         // Single-element tuple
/// ```
pub fn get() -> &'static BuiltinSpecialForm {
    static TUPLE_FORM: OnceLock<BuiltinSpecialForm> = OnceLock::new();
    TUPLE_FORM.get_or_init(|| BuiltinSpecialForm {
        name: "__tuple__",
        signature: Type::function(vec![], Type::Tuple(vec![])),
        eval_fn: eval_tuple,
        ir_fn: ir_tuple,
    })
}

fn eval_tuple(args: &[Expr], ctx: &mut EvalContext<'_>) -> Result<Value> {
    // Each argument is evaluated and added to the tuple
    let mut elements = Vec::with_capacity(args.len());

    for arg in args {
        let value = arg.eval(ctx)?;
        elements.push(value);
    }

    Ok(Value::Tuple {
        type_name: None,
        elements,
    })
}

fn ir_tuple(
    _args: &[Expr],
    _block: &mut BlockBuilder,
    _ctx: &mut IrGenContext,
    _source: SourceLocation,
    _gen_expr: &mut dyn FnMut(&Expr, &mut BlockBuilder, &mut IrGenContext) -> Result<ValueId>,
) -> Result<ValueId> {
    Err(Diagnostic::syntax(
        "tuple special form IR generation not yet implemented",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Compiler, Env};
    use cadenza_syntax::parse::parse;

    #[test]
    fn test_empty_tuple() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = "()";
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert_eq!(results.len(), 1);
        match &results[0] {
            Value::Tuple {
                type_name,
                elements,
            } => {
                assert!(type_name.is_none());
                assert_eq!(elements.len(), 0);
            }
            _ => panic!("Expected tuple, got {:?}", results[0]),
        }
    }

    #[test]
    fn test_pair_tuple() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = "(1, 2)";
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert_eq!(results.len(), 1);
        match &results[0] {
            Value::Tuple {
                type_name,
                elements,
            } => {
                assert!(type_name.is_none());
                assert_eq!(elements.len(), 2);
                assert_eq!(elements[0], Value::Integer(1));
                assert_eq!(elements[1], Value::Integer(2));
            }
            _ => panic!("Expected tuple, got {:?}", results[0]),
        }
    }

    #[test]
    fn test_triple_tuple() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = "(1, 2, 3)";
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert_eq!(results.len(), 1);
        match &results[0] {
            Value::Tuple {
                type_name,
                elements,
            } => {
                assert!(type_name.is_none());
                assert_eq!(elements.len(), 3);
                assert_eq!(elements[0], Value::Integer(1));
                assert_eq!(elements[1], Value::Integer(2));
                assert_eq!(elements[2], Value::Integer(3));
            }
            _ => panic!("Expected tuple, got {:?}", results[0]),
        }
    }

    #[test]
    fn test_nested_tuples() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = "((1, 2), (3, 4))";
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert_eq!(results.len(), 1);
        match &results[0] {
            Value::Tuple {
                type_name,
                elements,
            } => {
                assert!(type_name.is_none());
                assert_eq!(elements.len(), 2);

                // Check first nested tuple
                match &elements[0] {
                    Value::Tuple {
                        type_name: tn1,
                        elements: e1,
                    } => {
                        assert!(tn1.is_none());
                        assert_eq!(e1.len(), 2);
                        assert_eq!(e1[0], Value::Integer(1));
                        assert_eq!(e1[1], Value::Integer(2));
                    }
                    _ => panic!("Expected tuple in first element"),
                }

                // Check second nested tuple
                match &elements[1] {
                    Value::Tuple {
                        type_name: tn2,
                        elements: e2,
                    } => {
                        assert!(tn2.is_none());
                        assert_eq!(e2.len(), 2);
                        assert_eq!(e2[0], Value::Integer(3));
                        assert_eq!(e2[1], Value::Integer(4));
                    }
                    _ => panic!("Expected tuple in second element"),
                }
            }
            _ => panic!("Expected tuple, got {:?}", results[0]),
        }
    }
}
