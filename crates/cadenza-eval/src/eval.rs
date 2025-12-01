//! Tree-walk evaluator for the Cadenza language.
//!
//! The evaluator interprets the AST produced by cadenza-syntax.
//! It handles:
//! - Literals (integers, floats, strings)
//! - Identifiers (variable lookup)
//! - Lists/vectors
//! - Function application
//! - Macro expansion

use crate::{
    compiler::Compiler,
    context::{Eval, EvalContext},
    diagnostic::{Diagnostic, Result},
    env::Env,
    interner::InternedString,
    value::{Type, Value},
};
use cadenza_syntax::ast::{Apply, Attr, Expr, Ident, Literal, LiteralValue, Root, Synthetic};

/// Evaluates a complete source file (Root node).
///
/// Each top-level expression is evaluated in order. The results are
/// collected into a vector, though most top-level expressions will
/// return `Value::Nil` as side effects on the `Compiler` are the
/// primary purpose.
///
/// This function continues evaluation even when expressions fail, recording
/// errors in the compiler. On error, `Value::Nil` is used as the result for
/// that expression. Check `compiler.has_errors()` after calling to see if
/// any errors occurred.
pub fn eval(root: &Root, env: &mut Env, compiler: &mut Compiler) -> Vec<Value> {
    let mut ctx = EvalContext::new(env, compiler);
    let mut results = Vec::new();
    for expr in root.items() {
        match expr.eval(&mut ctx) {
            Ok(value) => results.push(value),
            Err(diagnostic) => {
                ctx.compiler.record_diagnostic(*diagnostic);
                results.push(Value::Nil);
            }
        }
    }
    results
}

// =============================================================================
// Eval trait implementations
// =============================================================================

impl Eval for Expr {
    fn eval(&self, ctx: &mut EvalContext<'_>) -> Result<Value> {
        match self {
            Expr::Literal(lit) => lit.eval(ctx),
            Expr::Ident(ident) => ident.eval(ctx),
            Expr::Apply(apply) => apply.eval(ctx),
            Expr::Attr(attr) => attr.eval(ctx),
            Expr::Op(op) => {
                // Operators as values (for higher-order usage)
                // Use SyntaxText directly without allocating a String
                let text = op.syntax().text();
                let id: InternedString = text.to_string().as_str().into();
                Ok(Value::Symbol(id))
            }
            Expr::Synthetic(syn) => syn.eval(ctx),
            Expr::Error(_) => Err(Diagnostic::syntax("encountered error node in AST")),
        }
    }
}

impl Eval for Literal {
    fn eval(&self, _ctx: &mut EvalContext<'_>) -> Result<Value> {
        let value = self
            .value()
            .ok_or_else(|| Diagnostic::syntax("missing literal value"))?;

        match value {
            LiteralValue::Integer(int_val) => {
                let text = int_val.syntax().text();
                let text_str = text.to_string();
                // Remove underscores for parsing
                let clean = text_str.replace('_', "");
                let n: i64 = clean
                    .parse()
                    .map_err(|_| Diagnostic::syntax(format!("invalid integer: {text_str}")))?;
                Ok(Value::Integer(n))
            }
            LiteralValue::Float(float_val) => {
                let text = float_val.syntax().text();
                let text_str = text.to_string();
                let clean = text_str.replace('_', "");
                let n: f64 = clean
                    .parse()
                    .map_err(|_| Diagnostic::syntax(format!("invalid float: {text_str}")))?;
                Ok(Value::Float(n))
            }
            LiteralValue::String(str_val) => {
                let text = str_val.syntax().text().to_string();
                Ok(Value::String(text))
            }
            LiteralValue::StringWithEscape(str_val) => {
                let text = str_val.syntax().text().to_string();
                // TODO: Process escape sequences
                Ok(Value::String(text))
            }
        }
    }
}

impl Eval for Ident {
    fn eval(&self, ctx: &mut EvalContext<'_>) -> Result<Value> {
        let text = self.syntax().text();
        // TODO: Investigate rowan API to avoid allocation. SyntaxText doesn't implement
        // AsRef<str> directly. See STATUS.md for tracking.
        let id: InternedString = text.to_string().as_str().into();

        // First check the local environment
        if let Some(value) = ctx.env.get(id) {
            return Ok(value.clone());
        }

        // Then check compiler definitions
        if let Some(value) = ctx.compiler.get_var(id) {
            return Ok(value.clone());
        }

        Err(Diagnostic::undefined_variable(id))
    }
}

impl Eval for Apply {
    fn eval(&self, ctx: &mut EvalContext<'_>) -> Result<Value> {
        let receiver = self
            .receiver()
            .ok_or_else(|| Diagnostic::syntax("missing receiver in application"))?;

        let receiver_expr = receiver
            .value()
            .ok_or_else(|| Diagnostic::syntax("missing receiver expression"))?;

        // Check if the receiver is an identifier that names a macro
        if let Expr::Ident(ref ident) = receiver_expr {
            let text = ident.syntax().text();
            let id: InternedString = text.to_string().as_str().into();

            // Check for macro in compiler
            if let Some(macro_value) = ctx.compiler.get_macro(id) {
                return expand_and_eval_macro(macro_value.clone(), self, ctx);
            }

            // Check for macro in environment
            if let Some(Value::BuiltinMacro(_)) = ctx.env.get(id) {
                let macro_value = ctx.env.get(id).unwrap().clone();
                return expand_and_eval_macro(macro_value, self, ctx);
            }
        }

        // Not a macro call - evaluate normally
        let callee = receiver_expr.eval(ctx)?;

        // Collect and evaluate arguments
        let mut args = Vec::new();
        for arg in self.arguments() {
            if let Some(arg_expr) = arg.value() {
                let value = arg_expr.eval(ctx)?;
                args.push(value);
            }
        }

        apply_value(callee, args, ctx)
    }
}

/// Expands a macro and evaluates the result.
fn expand_and_eval_macro(
    macro_value: Value,
    apply: &Apply,
    ctx: &mut EvalContext<'_>,
) -> Result<Value> {
    match macro_value {
        Value::BuiltinMacro(builtin) => {
            // Collect unevaluated argument syntax nodes
            let arg_nodes: Vec<rowan::GreenNode> = apply
                .arguments()
                .filter_map(|arg| arg.value())
                .filter_map(|expr| match expr {
                    Expr::Apply(a) => Some(a.syntax().green().into_owned()),
                    Expr::Ident(i) => Some(i.syntax().green().into_owned()),
                    Expr::Literal(l) => Some(l.syntax().green().into_owned()),
                    Expr::Op(o) => Some(o.syntax().green().into_owned()),
                    Expr::Attr(a) => Some(a.syntax().green().into_owned()),
                    Expr::Synthetic(s) => Some(s.syntax().green().into_owned()),
                    Expr::Error(_) => None,
                })
                .collect();

            // Call the builtin macro with unevaluated syntax nodes and context
            let expanded_node = (builtin.func)(&arg_nodes, ctx)?;

            // Create a SyntaxNode from the GreenNode and evaluate it
            let syntax_node = cadenza_syntax::Lang::parse_node(expanded_node);
            if let Some(expr) = Expr::cast_syntax_node(&syntax_node) {
                expr.eval(ctx)
            } else {
                Err(Diagnostic::internal(
                    "macro expansion produced invalid syntax",
                ))
            }
        }
        _ => Err(Diagnostic::internal("expected macro value")),
    }
}

/// Applies a callable value to arguments.
fn apply_value(callee: Value, args: Vec<Value>, ctx: &mut EvalContext<'_>) -> Result<Value> {
    match callee {
        Value::BuiltinFn(builtin) => (builtin.func)(&args, ctx),
        Value::Symbol(id) => {
            // Operator application
            apply_operator(id, args)
        }
        _ => Err(Diagnostic::not_callable(callee.type_of())),
    }
}

/// Applies a built-in operator.
fn apply_operator(op_id: InternedString, args: Vec<Value>) -> Result<Value> {
    let op_name: &str = &op_id;

    /// Creates the "number" type (integer | float) lazily only when needed for errors.
    fn number_type() -> Type {
        Type::union(vec![Type::Integer, Type::Float])
    }

    match op_name {
        "+" => match args.as_slice() {
            [Value::Integer(a), Value::Integer(b)] => Ok(Value::Integer(a + b)),
            [Value::Float(a), Value::Float(b)] => Ok(Value::Float(a + b)),
            [Value::Integer(a), Value::Float(b)] => Ok(Value::Float(*a as f64 + b)),
            [Value::Float(a), Value::Integer(b)] => Ok(Value::Float(a + *b as f64)),
            // For binary operators, report the first non-number type as the actual type
            [Value::Integer(_), b] | [Value::Float(_), b] => {
                Err(Diagnostic::type_error(number_type(), b.type_of()))
            }
            [a, _] => Err(Diagnostic::type_error(number_type(), a.type_of())),
            _ => Err(Diagnostic::arity(2, args.len())),
        },
        "-" => match args.as_slice() {
            [Value::Integer(a)] => Ok(Value::Integer(-a)),
            [Value::Float(a)] => Ok(Value::Float(-a)),
            [Value::Integer(a), Value::Integer(b)] => Ok(Value::Integer(a - b)),
            [Value::Float(a), Value::Float(b)] => Ok(Value::Float(a - b)),
            [Value::Integer(a), Value::Float(b)] => Ok(Value::Float(*a as f64 - b)),
            [Value::Float(a), Value::Integer(b)] => Ok(Value::Float(a - *b as f64)),
            // For binary operators, report the first non-number type as the actual type
            [Value::Integer(_), b] | [Value::Float(_), b] => {
                Err(Diagnostic::type_error(number_type(), b.type_of()))
            }
            [a, _] => Err(Diagnostic::type_error(number_type(), a.type_of())),
            [] => Err(Diagnostic::arity(1, 0)),
            _ => Err(Diagnostic::arity(2, args.len())),
        },
        "*" => match args.as_slice() {
            [Value::Integer(a), Value::Integer(b)] => Ok(Value::Integer(a * b)),
            [Value::Float(a), Value::Float(b)] => Ok(Value::Float(a * b)),
            [Value::Integer(a), Value::Float(b)] => Ok(Value::Float(*a as f64 * b)),
            [Value::Float(a), Value::Integer(b)] => Ok(Value::Float(a * *b as f64)),
            // For binary operators, report the first non-number type as the actual type
            [Value::Integer(_), b] | [Value::Float(_), b] => {
                Err(Diagnostic::type_error(number_type(), b.type_of()))
            }
            [a, _] => Err(Diagnostic::type_error(number_type(), a.type_of())),
            _ => Err(Diagnostic::arity(2, args.len())),
        },
        "/" => match args.as_slice() {
            [Value::Integer(a), Value::Integer(b)] => {
                if *b == 0 {
                    Err(Diagnostic::syntax("division by zero"))
                } else {
                    Ok(Value::Integer(a / b))
                }
            }
            [Value::Float(a), Value::Float(b)] => Ok(Value::Float(a / b)),
            [Value::Integer(a), Value::Float(b)] => Ok(Value::Float(*a as f64 / b)),
            [Value::Float(a), Value::Integer(b)] => Ok(Value::Float(a / *b as f64)),
            // For binary operators, report the first non-number type as the actual type
            [Value::Integer(_), b] | [Value::Float(_), b] => {
                Err(Diagnostic::type_error(number_type(), b.type_of()))
            }
            [a, _] => Err(Diagnostic::type_error(number_type(), a.type_of())),
            _ => Err(Diagnostic::arity(2, args.len())),
        },
        "==" => match args.as_slice() {
            [a, b] => Ok(Value::Bool(a == b)),
            _ => Err(Diagnostic::arity(2, args.len())),
        },
        "!=" => match args.as_slice() {
            [a, b] => Ok(Value::Bool(a != b)),
            _ => Err(Diagnostic::arity(2, args.len())),
        },
        "<" => match args.as_slice() {
            [Value::Integer(a), Value::Integer(b)] => Ok(Value::Bool(a < b)),
            [Value::Float(a), Value::Float(b)] => Ok(Value::Bool(a < b)),
            _ => Err(Diagnostic::arity(2, args.len())),
        },
        "<=" => match args.as_slice() {
            [Value::Integer(a), Value::Integer(b)] => Ok(Value::Bool(a <= b)),
            [Value::Float(a), Value::Float(b)] => Ok(Value::Bool(a <= b)),
            _ => Err(Diagnostic::arity(2, args.len())),
        },
        ">" => match args.as_slice() {
            [Value::Integer(a), Value::Integer(b)] => Ok(Value::Bool(a > b)),
            [Value::Float(a), Value::Float(b)] => Ok(Value::Bool(a > b)),
            _ => Err(Diagnostic::arity(2, args.len())),
        },
        ">=" => match args.as_slice() {
            [Value::Integer(a), Value::Integer(b)] => Ok(Value::Bool(a >= b)),
            [Value::Float(a), Value::Float(b)] => Ok(Value::Bool(a >= b)),
            _ => Err(Diagnostic::arity(2, args.len())),
        },
        "=" => {
            // Assignment operator - for now just return the right-hand side
            // In a full implementation, this would modify the environment
            match args.as_slice() {
                [_, value] => Ok(value.clone()),
                _ => Err(Diagnostic::arity(2, args.len())),
            }
        }
        _ => Err(Diagnostic::undefined_variable(op_id)),
    }
}

impl Eval for Attr {
    fn eval(&self, ctx: &mut EvalContext<'_>) -> Result<Value> {
        // For now, attributes are evaluated and returned as-is
        // In the future, they might have special semantics
        if let Some(value_expr) = self.value() {
            value_expr.eval(ctx)
        } else {
            Ok(Value::Nil)
        }
    }
}

impl Eval for Synthetic {
    fn eval(&self, _ctx: &mut EvalContext<'_>) -> Result<Value> {
        let ident = self.identifier();

        match ident {
            "__list__" | "__record__" => {
                // These should not appear as standalone expressions
                // They're always wrapped in Apply nodes
                Ok(Value::Nil)
            }
            _ => Err(Diagnostic::syntax(format!(
                "unknown synthetic node: {ident}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadenza_syntax::parse::parse;

    fn eval_str(src: &str) -> Result<Vec<Value>> {
        let parsed = parse(src);
        if let Some(err) = parsed.errors.first() {
            return Err(Diagnostic::parse_error(&err.message, err.span));
        }
        let root = parsed.ast();
        let mut env = Env::new();
        let mut compiler = Compiler::new();
        let results = eval(&root, &mut env, &mut compiler);
        if compiler.has_errors() {
            return Err(Box::new(
                compiler
                    .take_diagnostics()
                    .into_iter()
                    .next()
                    .expect("has_errors() returned true but no diagnostics found"),
            ));
        }
        Ok(results)
    }

    fn eval_single(src: &str) -> Result<Value> {
        let values = eval_str(src)?;
        values
            .into_iter()
            .next()
            .ok_or_else(|| Diagnostic::syntax("no expressions"))
    }

    #[test]
    fn eval_integer() {
        assert_eq!(eval_single("42").unwrap(), Value::Integer(42));
    }

    #[test]
    fn eval_float() {
        assert_eq!(eval_single("2.5").unwrap(), Value::Float(2.5));
    }

    #[test]
    fn eval_integer_with_underscores() {
        assert_eq!(eval_single("1_000_000").unwrap(), Value::Integer(1000000));
    }

    #[test]
    fn eval_string() {
        assert_eq!(
            eval_single("\"hello\"").unwrap(),
            Value::String("hello".to_string())
        );
    }

    #[test]
    fn eval_addition() {
        assert_eq!(eval_single("1 + 2").unwrap(), Value::Integer(3));
    }

    #[test]
    fn eval_subtraction() {
        assert_eq!(eval_single("5 - 3").unwrap(), Value::Integer(2));
    }

    #[test]
    fn eval_multiplication() {
        assert_eq!(eval_single("4 * 5").unwrap(), Value::Integer(20));
    }

    #[test]
    fn eval_division() {
        assert_eq!(eval_single("10 / 2").unwrap(), Value::Integer(5));
    }

    #[test]
    fn eval_complex_arithmetic() {
        // Due to operator precedence, this should be parsed as 1 + (2 * 3)
        assert_eq!(eval_single("1 + 2 * 3").unwrap(), Value::Integer(7));
    }

    #[test]
    fn eval_comparison() {
        assert_eq!(eval_single("1 < 2").unwrap(), Value::Bool(true));
        assert_eq!(eval_single("2 > 1").unwrap(), Value::Bool(true));
        assert_eq!(eval_single("1 == 1").unwrap(), Value::Bool(true));
        assert_eq!(eval_single("1 != 2").unwrap(), Value::Bool(true));
    }

    #[test]
    fn eval_undefined_variable() {
        let result = eval_single("undefined_var");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().kind,
            crate::diagnostic::DiagnosticKind::UndefinedVariable(_)
        ));
    }

    #[test]
    fn eval_with_environment() {
        let parsed = parse("x");
        let root = parsed.ast();
        let mut env = Env::new();
        let mut compiler = Compiler::new();

        // Define x in environment
        let x_id: InternedString = "x".into();
        env.define(x_id, Value::Integer(42));

        let result = eval(&root, &mut env, &mut compiler);
        assert!(!compiler.has_errors());
        assert_eq!(result[0], Value::Integer(42));
    }

    #[test]
    fn eval_with_compiler_var() {
        let parsed = parse("y");
        let root = parsed.ast();
        let mut env = Env::new();
        let mut compiler = Compiler::new();

        // Define y in compiler
        let y_id: InternedString = "y".into();
        compiler.define_var(y_id, Value::Integer(100));

        let result = eval(&root, &mut env, &mut compiler);
        assert!(!compiler.has_errors());
        assert_eq!(result[0], Value::Integer(100));
    }

    #[test]
    fn eval_continues_on_error() {
        // This source has multiple expressions, some of which will fail
        let src = "1\nundefined_var\n3";
        let parsed = parse(src);
        let root = parsed.ast();
        let mut env = Env::new();
        let mut compiler = Compiler::new();

        let results = eval(&root, &mut env, &mut compiler);

        // Should have 3 results (1, Nil for error, 3)
        assert_eq!(results.len(), 3);
        assert_eq!(results[0], Value::Integer(1));
        assert_eq!(results[1], Value::Nil); // error becomes Nil
        assert_eq!(results[2], Value::Integer(3));

        // Compiler should have recorded 1 error
        assert_eq!(compiler.num_diagnostics(), 1);
        assert!(compiler.has_errors());
    }

    #[test]
    fn eval_collects_multiple_errors() {
        // Multiple undefined variables
        let src = "undefined_a\n1\nundefined_b\n2\nundefined_c";
        let parsed = parse(src);
        let root = parsed.ast();
        let mut env = Env::new();
        let mut compiler = Compiler::new();

        let results = eval(&root, &mut env, &mut compiler);

        // Should have 5 results
        assert_eq!(results.len(), 5);
        assert_eq!(results[0], Value::Nil); // undefined_a error
        assert_eq!(results[1], Value::Integer(1));
        assert_eq!(results[2], Value::Nil); // undefined_b error
        assert_eq!(results[3], Value::Integer(2));
        assert_eq!(results[4], Value::Nil); // undefined_c error

        // Compiler should have recorded 3 errors
        assert_eq!(compiler.num_diagnostics(), 3);
        assert!(compiler.has_errors());

        // All should be UndefinedVariable errors
        for diag in compiler.diagnostics() {
            assert!(matches!(
                &diag.kind,
                crate::diagnostic::DiagnosticKind::UndefinedVariable(_)
            ));
        }
    }

    #[test]
    fn eval_no_errors() {
        let src = "1\n2\n3";
        let parsed = parse(src);
        let root = parsed.ast();
        let mut env = Env::new();
        let mut compiler = Compiler::new();

        let results = eval(&root, &mut env, &mut compiler);

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], Value::Integer(1));
        assert_eq!(results[1], Value::Integer(2));
        assert_eq!(results[2], Value::Integer(3));

        // No errors recorded
        assert_eq!(compiler.num_diagnostics(), 0);
        assert!(!compiler.has_errors());
    }
}
