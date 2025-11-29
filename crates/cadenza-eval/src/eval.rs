//! Tree-walk evaluator for the Cadenza language.
//!
//! The evaluator interprets the AST produced by cadenza-syntax.
//! It handles:
//! - Literals (integers, floats, strings)
//! - Identifiers (variable lookup)
//! - Lists/vectors
//! - Function application
//! - Macro expansion

use crate::compiler::Compiler;
use crate::env::Env;
use crate::error::{Error, Result};
use crate::interner::{InternedId, Interner};
use crate::value::Value;
use cadenza_syntax::ast::{Apply, Attr, Expr, Ident, Literal, LiteralValue, Root, Synthetic};

/// Evaluates a complete source file (Root node).
///
/// Each top-level expression is evaluated in order. The results are
/// collected into a vector, though most top-level expressions will
/// return `Value::Nil` as side effects on the `Compiler` are the
/// primary purpose.
pub fn eval(root: &Root, interner: &mut Interner, env: &mut Env, compiler: &mut Compiler) -> Result<Vec<Value>> {
    let mut results = Vec::new();
    for expr in root.items() {
        let value = eval_expr(&expr, interner, env, compiler)?;
        results.push(value);
    }
    Ok(results)
}

/// Evaluates a single expression.
pub fn eval_expr(
    expr: &Expr,
    interner: &mut Interner,
    env: &mut Env,
    compiler: &mut Compiler,
) -> Result<Value> {
    match expr {
        Expr::Literal(lit) => eval_literal(lit),
        Expr::Ident(ident) => eval_ident(ident, interner, env, compiler),
        Expr::Apply(apply) => eval_apply(apply, interner, env, compiler),
        Expr::Attr(attr) => eval_attr(attr, interner, env, compiler),
        Expr::Op(op) => {
            // Operators as values (for higher-order usage)
            let text = op.syntax().text().to_string();
            let id = interner.intern(&text);
            Ok(Value::Symbol(id))
        }
        Expr::Synthetic(syn) => eval_synthetic(syn, interner, env, compiler),
        Expr::Error(_) => Err(Error::syntax("encountered error node in AST")),
    }
}

/// Evaluates a literal value.
fn eval_literal(lit: &Literal) -> Result<Value> {
    let value = lit.value().ok_or_else(|| Error::syntax("missing literal value"))?;

    match value {
        LiteralValue::Integer(int_val) => {
            let text = int_val.syntax().text().to_string();
            // Remove underscores for parsing
            let clean = text.replace('_', "");
            let n: i64 = clean
                .parse()
                .map_err(|_| Error::syntax(format!("invalid integer: {text}")))?;
            Ok(Value::Integer(n))
        }
        LiteralValue::Float(float_val) => {
            let text = float_val.syntax().text().to_string();
            let clean = text.replace('_', "");
            let n: f64 = clean
                .parse()
                .map_err(|_| Error::syntax(format!("invalid float: {text}")))?;
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

/// Evaluates an identifier by looking it up in the environment.
fn eval_ident(
    ident: &Ident,
    interner: &mut Interner,
    env: &Env,
    compiler: &Compiler,
) -> Result<Value> {
    let text = ident.syntax().text().to_string();
    let id = interner.intern(&text);

    // First check the local environment
    if let Some(value) = env.get(id) {
        return Ok(value.clone());
    }

    // Then check compiler definitions
    if let Some(value) = compiler.get_var(id) {
        return Ok(value.clone());
    }

    Err(Error::undefined_variable(&text))
}

/// Evaluates a function/macro application.
fn eval_apply(
    apply: &Apply,
    interner: &mut Interner,
    env: &mut Env,
    compiler: &mut Compiler,
) -> Result<Value> {
    let receiver = apply
        .receiver()
        .ok_or_else(|| Error::syntax("missing receiver in application"))?;

    let receiver_expr = receiver
        .value()
        .ok_or_else(|| Error::syntax("missing receiver expression"))?;

    // Check if the receiver is an identifier that names a macro
    if let Expr::Ident(ref ident) = receiver_expr {
        let text = ident.syntax().text().to_string();
        let id = interner.intern(&text);

        // Check for macro in compiler
        if let Some(macro_value) = compiler.get_macro(id) {
            return expand_and_eval_macro(macro_value.clone(), apply, interner, env, compiler);
        }

        // Check for macro in environment
        if let Some(Value::BuiltinMacro(_)) = env.get(id) {
            let macro_value = env.get(id).unwrap().clone();
            return expand_and_eval_macro(macro_value, apply, interner, env, compiler);
        }
    }

    // Not a macro call - evaluate normally
    let callee = eval_expr(&receiver_expr, interner, env, compiler)?;

    // Collect and evaluate arguments
    let mut args = Vec::new();
    for arg in apply.arguments() {
        if let Some(arg_expr) = arg.value() {
            let value = eval_expr(&arg_expr, interner, env, compiler)?;
            args.push(value);
        }
    }

    apply_value(callee, args, interner, env, compiler)
}

/// Expands a macro and evaluates the result.
fn expand_and_eval_macro(
    macro_value: Value,
    apply: &Apply,
    interner: &mut Interner,
    env: &mut Env,
    compiler: &mut Compiler,
) -> Result<Value> {
    match macro_value {
        Value::BuiltinMacro(builtin) => {
            // Collect unevaluated argument syntax nodes
            let arg_nodes: Vec<rowan::GreenNode> = apply
                .arguments()
                .filter_map(|arg| arg.value())
                .filter_map(|expr| {
                    match expr {
                        Expr::Apply(a) => Some(a.syntax().green().into_owned()),
                        Expr::Ident(i) => Some(i.syntax().green().into_owned()),
                        Expr::Literal(l) => Some(l.syntax().green().into_owned()),
                        Expr::Op(o) => Some(o.syntax().green().into_owned()),
                        Expr::Attr(a) => Some(a.syntax().green().into_owned()),
                        Expr::Synthetic(s) => Some(s.syntax().green().into_owned()),
                        Expr::Error(_) => None,
                    }
                })
                .collect();

            // Call the builtin macro with unevaluated syntax nodes
            let expanded_node = (builtin.func)(&arg_nodes)?;

            // Create a SyntaxNode from the GreenNode and evaluate it
            let syntax_node = cadenza_syntax::Lang::parse_node(expanded_node);
            if let Some(expr) = Expr::cast_syntax_node(&syntax_node) {
                eval_expr(&expr, interner, env, compiler)
            } else {
                Err(Error::internal("macro expansion produced invalid syntax"))
            }
        }
        _ => Err(Error::internal("expected macro value")),
    }
}

/// Applies a callable value to arguments.
fn apply_value(
    callee: Value,
    args: Vec<Value>,
    interner: &Interner,
    _env: &mut Env,
    _compiler: &mut Compiler,
) -> Result<Value> {
    match callee {
        Value::BuiltinFn(builtin) => (builtin.func)(&args),
        Value::Symbol(id) => {
            // Operator application
            apply_operator(id, args, interner)
        }
        _ => Err(Error::not_callable(callee.type_name())),
    }
}

/// Applies a built-in operator.
fn apply_operator(op_id: InternedId, args: Vec<Value>, interner: &Interner) -> Result<Value> {
    let op_name = interner.resolve(op_id);

    match op_name {
        "+" => match args.as_slice() {
            [Value::Integer(a), Value::Integer(b)] => Ok(Value::Integer(a + b)),
            [Value::Float(a), Value::Float(b)] => Ok(Value::Float(a + b)),
            [Value::Integer(a), Value::Float(b)] => Ok(Value::Float(*a as f64 + b)),
            [Value::Float(a), Value::Integer(b)] => Ok(Value::Float(a + *b as f64)),
            [a, b] => Err(Error::type_error("number", format!("{} and {}", a.type_name(), b.type_name()))),
            _ => Err(Error::arity(2, args.len())),
        },
        "-" => match args.as_slice() {
            [Value::Integer(a)] => Ok(Value::Integer(-a)),
            [Value::Float(a)] => Ok(Value::Float(-a)),
            [Value::Integer(a), Value::Integer(b)] => Ok(Value::Integer(a - b)),
            [Value::Float(a), Value::Float(b)] => Ok(Value::Float(a - b)),
            [Value::Integer(a), Value::Float(b)] => Ok(Value::Float(*a as f64 - b)),
            [Value::Float(a), Value::Integer(b)] => Ok(Value::Float(a - *b as f64)),
            [a, b] => Err(Error::type_error("number", format!("{} and {}", a.type_name(), b.type_name()))),
            _ => Err(Error::arity(2, args.len())),
        },
        "*" => match args.as_slice() {
            [Value::Integer(a), Value::Integer(b)] => Ok(Value::Integer(a * b)),
            [Value::Float(a), Value::Float(b)] => Ok(Value::Float(a * b)),
            [Value::Integer(a), Value::Float(b)] => Ok(Value::Float(*a as f64 * b)),
            [Value::Float(a), Value::Integer(b)] => Ok(Value::Float(a * *b as f64)),
            [a, b] => Err(Error::type_error("number", format!("{} and {}", a.type_name(), b.type_name()))),
            _ => Err(Error::arity(2, args.len())),
        },
        "/" => match args.as_slice() {
            [Value::Integer(a), Value::Integer(b)] => {
                if *b == 0 {
                    Err(Error::syntax("division by zero"))
                } else {
                    Ok(Value::Integer(a / b))
                }
            }
            [Value::Float(a), Value::Float(b)] => Ok(Value::Float(a / b)),
            [Value::Integer(a), Value::Float(b)] => Ok(Value::Float(*a as f64 / b)),
            [Value::Float(a), Value::Integer(b)] => Ok(Value::Float(a / *b as f64)),
            [a, b] => Err(Error::type_error("number", format!("{} and {}", a.type_name(), b.type_name()))),
            _ => Err(Error::arity(2, args.len())),
        },
        "==" => match args.as_slice() {
            [a, b] => Ok(Value::Bool(a == b)),
            _ => Err(Error::arity(2, args.len())),
        },
        "!=" => match args.as_slice() {
            [a, b] => Ok(Value::Bool(a != b)),
            _ => Err(Error::arity(2, args.len())),
        },
        "<" => match args.as_slice() {
            [Value::Integer(a), Value::Integer(b)] => Ok(Value::Bool(a < b)),
            [Value::Float(a), Value::Float(b)] => Ok(Value::Bool(a < b)),
            _ => Err(Error::arity(2, args.len())),
        },
        "<=" => match args.as_slice() {
            [Value::Integer(a), Value::Integer(b)] => Ok(Value::Bool(a <= b)),
            [Value::Float(a), Value::Float(b)] => Ok(Value::Bool(a <= b)),
            _ => Err(Error::arity(2, args.len())),
        },
        ">" => match args.as_slice() {
            [Value::Integer(a), Value::Integer(b)] => Ok(Value::Bool(a > b)),
            [Value::Float(a), Value::Float(b)] => Ok(Value::Bool(a > b)),
            _ => Err(Error::arity(2, args.len())),
        },
        ">=" => match args.as_slice() {
            [Value::Integer(a), Value::Integer(b)] => Ok(Value::Bool(a >= b)),
            [Value::Float(a), Value::Float(b)] => Ok(Value::Bool(a >= b)),
            _ => Err(Error::arity(2, args.len())),
        },
        "=" => {
            // Assignment operator - for now just return the right-hand side
            // In a full implementation, this would modify the environment
            match args.as_slice() {
                [_, value] => Ok(value.clone()),
                _ => Err(Error::arity(2, args.len())),
            }
        },
        _ => Err(Error::undefined_variable(op_name)),
    }
}

/// Evaluates an attribute (@decorator).
fn eval_attr(
    attr: &Attr,
    interner: &mut Interner,
    env: &mut Env,
    compiler: &mut Compiler,
) -> Result<Value> {
    // For now, attributes are evaluated and returned as-is
    // In the future, they might have special semantics
    if let Some(value_expr) = attr.value() {
        eval_expr(&value_expr, interner, env, compiler)
    } else {
        Ok(Value::Nil)
    }
}

/// Evaluates a synthetic node (list, record, etc.).
fn eval_synthetic(
    syn: &Synthetic,
    _interner: &mut Interner,
    _env: &mut Env,
    _compiler: &mut Compiler,
) -> Result<Value> {
    let ident = syn.identifier();

    match ident {
        "__list__" | "__record__" => {
            // These should not appear as standalone expressions
            // They're always wrapped in Apply nodes
            Ok(Value::Nil)
        }
        _ => Err(Error::syntax(format!("unknown synthetic node: {ident}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadenza_syntax::parse::parse;

    fn eval_str(src: &str) -> Result<Vec<Value>> {
        let parsed = parse(src);
        if !parsed.errors.is_empty() {
            return Err(Error::syntax(format!("parse errors: {:?}", parsed.errors)));
        }
        let root = parsed.ast();
        let mut interner = Interner::new();
        let mut env = Env::new();
        let mut compiler = Compiler::new();
        eval(&root, &mut interner, &mut env, &mut compiler)
    }

    fn eval_single(src: &str) -> Result<Value> {
        let values = eval_str(src)?;
        values.into_iter().next().ok_or_else(|| Error::syntax("no expressions"))
    }

    #[test]
    fn eval_integer() {
        assert_eq!(eval_single("42").unwrap(), Value::Integer(42));
    }

    #[test]
    fn eval_float() {
        assert_eq!(eval_single("3.14").unwrap(), Value::Float(3.14));
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
        assert!(matches!(result.unwrap_err(), Error::UndefinedVariable(_)));
    }

    #[test]
    fn eval_with_environment() {
        let parsed = parse("x");
        let root = parsed.ast();
        let mut interner = Interner::new();
        let mut env = Env::new();
        let mut compiler = Compiler::new();

        // Define x in environment
        let x_id = interner.intern("x");
        env.define(x_id, Value::Integer(42));

        let result = eval(&root, &mut interner, &mut env, &mut compiler).unwrap();
        assert_eq!(result[0], Value::Integer(42));
    }

    #[test]
    fn eval_with_compiler_var() {
        let parsed = parse("y");
        let root = parsed.ast();
        let mut interner = Interner::new();
        let mut env = Env::new();
        let mut compiler = Compiler::new();

        // Define y in compiler
        let y_id = interner.intern("y");
        compiler.define_var(y_id, Value::Integer(100));

        let result = eval(&root, &mut interner, &mut env, &mut compiler).unwrap();
        assert_eq!(result[0], Value::Integer(100));
    }
}
