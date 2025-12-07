//! The `__index__` special form for array indexing.

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

/// Returns the `__index__` special form for array indexing.
///
/// The `__index__` special form performs array indexing operations.
///
/// # Evaluation
/// - Takes exactly 2 arguments: array expression and index expression
/// - Evaluates both arguments
/// - Returns the element at the given index
/// - Supports negative indexing (from the end)
///
/// # IR Generation
/// - Not yet implemented (returns error)
///
/// # Examples
/// ```cadenza
/// arr[0]           // Gets the first element of arr
/// matrix[1][2]     // Gets element at row 1, column 2
/// [1, 2, 3][0]     // Gets the first element: 1
/// [1, 2, 3][-1]    // Gets the last element: 3
/// ```
pub fn get() -> &'static BuiltinSpecialForm {
    static INDEX_FORM: OnceLock<BuiltinSpecialForm> = OnceLock::new();
    INDEX_FORM.get_or_init(|| BuiltinSpecialForm {
        name: "__index__",
        signature: Type::function(
            vec![Type::list(Type::Unknown), Type::Integer],
            Type::Unknown,
        ),
        eval_fn: eval_index,
        ir_fn: ir_index,
    })
}

fn eval_index(args: &[Expr], ctx: &mut EvalContext<'_>) -> Result<Value> {
    if args.len() != 2 {
        return Err(Diagnostic::syntax(
            "index operation requires exactly 2 arguments (array and index)",
        ));
    }

    // Evaluate the array expression
    let array_value = args[0].eval(ctx)?;

    // Evaluate the index expression
    let index_value = args[1].eval(ctx)?;

    // Extract the index as an integer
    let index = match index_value {
        Value::Integer(i) => i,
        _ => {
            return Err(Diagnostic::type_error(Type::Integer, index_value.type_of()));
        }
    };

    // Extract the array
    match array_value {
        Value::List(ref elements) => {
            // Handle negative indexing (from the end)
            let len = elements.len() as i64;
            let actual_index = if index < 0 { len + index } else { index };

            // Check bounds
            if actual_index < 0 || actual_index >= len {
                return Err(Diagnostic::syntax(format!(
                    "index out of bounds: index {} is out of range for list of length {}",
                    index, len
                )));
            }

            Ok(elements[actual_index as usize].clone())
        }
        _ => Err(Diagnostic::type_error(
            Type::list(Type::Unknown),
            array_value.type_of(),
        )),
    }
}

fn ir_index(
    _args: &[Expr],
    _block: &mut BlockBuilder,
    _ctx: &mut IrGenContext,
    _source: SourceLocation,
    _gen_expr: &mut dyn FnMut(&Expr, &mut BlockBuilder, &mut IrGenContext) -> Result<ValueId>,
) -> Result<ValueId> {
    Err(Diagnostic::syntax(
        "__index__ special form IR generation not yet implemented",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Compiler, Env};
    use cadenza_syntax::parse::parse;

    #[test]
    fn test_index_basic() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = "[1, 2, 3][0]";
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0], Value::Integer(1));
    }

    #[test]
    fn test_index_negative() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let input = "[1, 2, 3][-1]";
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0], Value::Integer(3));
    }

    #[test]
    fn test_index_on_variable() {
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        // First, define the variable
        let input1 = "let arr = [10, 20, 30]";
        let parsed1 = parse(input1);
        let root1 = parsed1.ast();
        let _results1 = crate::eval(&root1, &mut env, &mut compiler);

        // Then access it with indexing
        let input2 = "arr[1]";
        let parsed2 = parse(input2);
        let root2 = parsed2.ast();
        let results2 = crate::eval(&root2, &mut env, &mut compiler);

        assert_eq!(results2.len(), 1);
        assert_eq!(results2[0], Value::Integer(20));
    }
}
