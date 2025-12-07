//! The `let` special form for variable declarations.

use crate::{
    Eval,
    context::EvalContext,
    diagnostic::{Diagnostic, Result},
    interner::InternedString,
    ir::{BlockBuilder, IrGenContext, SourceLocation, ValueId},
    special_form::BuiltinSpecialForm,
    value::{Type, Value},
};
use cadenza_syntax::ast::Expr;
use std::sync::OnceLock;

/// Returns the `let` special form for variable declarations.
///
/// The `let` special form binds a name to a value in the current scope.
///
/// # Evaluation
/// - Takes 2 arguments: identifier and value expression
/// - Evaluates the value expression
/// - Binds the identifier to the evaluated value in the environment
/// - Returns the evaluated value
///
/// # IR Generation
/// - Generates IR for the value expression
/// - Binds the identifier to the resulting ValueId
/// - Returns the ValueId
///
/// # Examples
/// ```cadenza
/// let x = 42
/// let y = x + 1
/// ```
pub fn get() -> &'static BuiltinSpecialForm {
    static LET_FORM: OnceLock<BuiltinSpecialForm> = OnceLock::new();
    LET_FORM.get_or_init(|| BuiltinSpecialForm {
        name: "let",
        signature: Type::function(vec![Type::Symbol, Type::Unknown], Type::Unknown),
        eval_fn: eval_let,
        ir_fn: ir_let,
    })
}

fn eval_let(args: &[Expr], ctx: &mut EvalContext<'_>) -> Result<Value> {
    // If called with 0 arguments, return Nil
    if args.is_empty() {
        return Ok(Value::Nil);
    }

    // If called with 1 argument, return Nil (needs delegation)
    if args.len() == 1 {
        return Ok(Value::Nil);
    }

    // Called with 2 arguments: [name, value]
    if args.len() != 2 {
        return Err(Diagnostic::syntax(
            "let expects 1 or 2 arguments (e.g., let x, or let x = 42)",
        ));
    }

    // First argument is the identifier
    let ident = match &args[0] {
        Expr::Ident(i) => i,
        _ => {
            return Err(Diagnostic::syntax(
                "let requires an identifier as the variable name",
            ));
        }
    };

    // Get the identifier name
    let text = ident.syntax().text();
    let name: InternedString = text.to_string().as_str().into();

    // Second argument is the value expression
    let value_expr = &args[1];
    let value = value_expr.eval(ctx)?;

    // Define the variable in the environment with the evaluated value
    ctx.env.define(name, value.clone());

    // Return the value
    Ok(value)
}

fn ir_let(
    args: &[Expr],
    block: &mut BlockBuilder,
    ctx: &mut IrGenContext,
    _source: SourceLocation,
    gen_expr: &mut dyn FnMut(&Expr, &mut BlockBuilder, &mut IrGenContext) -> Result<ValueId>,
) -> Result<ValueId> {
    // Validate argument count
    if args.len() != 2 {
        return Err(Diagnostic::syntax(
            "let requires exactly 2 arguments in IR generation",
        ));
    }

    // Extract the variable name from the identifier
    let var_name = match &args[0] {
        Expr::Ident(ident) => {
            ident.syntax().text().interned()
        }
        _ => {
            return Err(Diagnostic::syntax(
                "let requires an identifier as variable name",
            ));
        }
    };

    // Generate IR for the value expression using the provided gen_expr callback
    let value_id = gen_expr(&args[1], block, ctx)?;

    // TODO(type-inference): Infer the type of the value expression.
    // Currently we use Unknown type because the special form doesn't have access to
    // the TypeInferencer (which is inside IrGenerator). This should be addressed by:
    // 1. Passing type information via ctx (preferred), OR
    // 2. Adding type inferencer as a parameter to build_ir, OR
    // 3. Having special forms integrate directly into IrGenerator methods
    // For now, this matches the approach used before type inference was added to let bindings.
    let inferred_ty = crate::InferType::Concrete(crate::Type::Unknown);

    // Bind the variable name to the value
    ctx.bind_var(var_name, value_id, &inferred_ty);

    // Return the value ID
    Ok(value_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Compiler, Env};
    use cadenza_syntax::parse::parse;

    #[test]
    fn test_let_special_form_eval() {
        // Create an environment with the special form AND the = operator
        let mut env = Env::with_standard_builtins(); // This registers all builtins including =

        // Replace the let macro with our special form
        let let_id: InternedString = "let".into();
        env.define(let_id, Value::SpecialForm(get()));

        // Create a compiler
        let mut compiler = Compiler::new();

        // Parse and evaluate "let x = 42"
        let input = "let x = 42";
        let parsed = parse(input);
        let root = parsed.ast();

        let results = crate::eval(&root, &mut env, &mut compiler);

        assert!(!results.is_empty(), "Expected at least one result");
        let value = &results[0];
        assert_eq!(*value, Value::Integer(42));

        // Verify the variable was bound
        let x_id: InternedString = "x".into();
        assert_eq!(env.get(x_id), Some(&Value::Integer(42)));
    }
}
