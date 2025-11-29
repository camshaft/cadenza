//! Integration tests for the Cadenza evaluator.

use crate::{
    compiler::Compiler,
    env::Env,
    error::Error,
    interner::Interner,
    value::{BuiltinFn, Value},
};
use cadenza_syntax::parse::parse;

/// Helper to evaluate a source string and return all values.
fn eval_all(src: &str) -> Result<Vec<Value>, Error> {
    let parsed = parse(src);
    if !parsed.errors.is_empty() {
        return Err(Error::syntax(format!("parse errors: {:?}", parsed.errors)));
    }
    let root = parsed.ast();
    let mut interner = Interner::new();
    let mut env = Env::new();
    let mut compiler = Compiler::new();
    crate::eval(&root, &mut interner, &mut env, &mut compiler)
}

/// Helper to evaluate a single expression.
fn eval_one(src: &str) -> Result<Value, Error> {
    eval_all(src)?
        .into_iter()
        .next()
        .ok_or_else(|| Error::syntax("no expressions"))
}

#[test]
fn test_basic_arithmetic() {
    assert_eq!(eval_one("1 + 2").unwrap(), Value::Integer(3));
    assert_eq!(eval_one("10 - 4").unwrap(), Value::Integer(6));
    assert_eq!(eval_one("3 * 7").unwrap(), Value::Integer(21));
    assert_eq!(eval_one("20 / 4").unwrap(), Value::Integer(5));
}

#[test]
fn test_operator_precedence() {
    // * has higher precedence than +
    assert_eq!(eval_one("2 + 3 * 4").unwrap(), Value::Integer(14));
    // Left-to-right for same precedence
    assert_eq!(eval_one("10 - 5 - 2").unwrap(), Value::Integer(3));
}

#[test]
fn test_float_arithmetic() {
    assert_eq!(eval_one("1.5 + 2.5").unwrap(), Value::Float(4.0));
    assert_eq!(eval_one("3.0 * 2.0").unwrap(), Value::Float(6.0));
}

#[test]
fn test_mixed_arithmetic() {
    // Integer + Float = Float
    assert_eq!(eval_one("1 + 2.5").unwrap(), Value::Float(3.5));
    assert_eq!(eval_one("2.5 + 1").unwrap(), Value::Float(3.5));
}

#[test]
fn test_comparison_operators() {
    assert_eq!(eval_one("1 < 2").unwrap(), Value::Bool(true));
    assert_eq!(eval_one("2 < 1").unwrap(), Value::Bool(false));
    assert_eq!(eval_one("1 <= 1").unwrap(), Value::Bool(true));
    assert_eq!(eval_one("2 > 1").unwrap(), Value::Bool(true));
    assert_eq!(eval_one("1 >= 1").unwrap(), Value::Bool(true));
    assert_eq!(eval_one("1 == 1").unwrap(), Value::Bool(true));
    assert_eq!(eval_one("1 != 2").unwrap(), Value::Bool(true));
}

#[test]
fn test_string_literal() {
    assert_eq!(
        eval_one("\"hello world\"").unwrap(),
        Value::String("hello world".to_string())
    );
}

#[test]
fn test_multiple_expressions() {
    let results = eval_all("1\n2\n3").unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(results[0], Value::Integer(1));
    assert_eq!(results[1], Value::Integer(2));
    assert_eq!(results[2], Value::Integer(3));
}

#[test]
fn test_undefined_variable_error() {
    let result = eval_one("undefined_variable");
    assert!(result.is_err());
    match result.unwrap_err().kind {
        crate::error::ErrorKind::UndefinedVariable(name) => {
            assert_eq!(name, "undefined_variable")
        }
        e => panic!("unexpected error: {e}"),
    }
}

#[test]
fn test_builtin_function() {
    // Note: "add 1 2" is parsed as ((add 1) 2), i.e., nested applies.
    // For simple testing, we use a single-argument function.
    let parsed = parse("inc 5");
    let root = parsed.ast();
    let mut interner = Interner::new();
    let mut env = Env::new();
    let mut compiler = Compiler::new();

    // Define a builtin inc function (increment by 1)
    let inc_id = interner.intern("inc");
    env.define(
        inc_id,
        Value::BuiltinFn(BuiltinFn {
            name: "inc",
            func: |args| {
                if args.len() != 1 {
                    return Err(Error::arity(1, args.len()));
                }
                match &args[0] {
                    Value::Integer(a) => Ok(Value::Integer(a + 1)),
                    _ => Err(Error::type_error("integer", "other")),
                }
            },
        }),
    );

    let results = crate::eval(&root, &mut interner, &mut env, &mut compiler).unwrap();
    assert_eq!(results[0], Value::Integer(6));
}

#[test]
fn test_variable_from_environment() {
    let parsed = parse("x + y");
    let root = parsed.ast();
    let mut interner = Interner::new();
    let mut env = Env::new();
    let mut compiler = Compiler::new();

    let x_id = interner.intern("x");
    let y_id = interner.intern("y");
    env.define(x_id, Value::Integer(10));
    env.define(y_id, Value::Integer(20));

    let results = crate::eval(&root, &mut interner, &mut env, &mut compiler).unwrap();
    assert_eq!(results[0], Value::Integer(30));
}

#[test]
fn test_variable_from_compiler() {
    let parsed = parse("global_var");
    let root = parsed.ast();
    let mut interner = Interner::new();
    let mut env = Env::new();
    let mut compiler = Compiler::new();

    let id = interner.intern("global_var");
    compiler.define_var(id, Value::Integer(42));

    let results = crate::eval(&root, &mut interner, &mut env, &mut compiler).unwrap();
    assert_eq!(results[0], Value::Integer(42));
}

#[test]
fn test_division_by_zero() {
    let result = eval_one("1 / 0");
    assert!(result.is_err());
}

#[test]
fn test_interner_consistency() {
    let mut interner = Interner::new();
    let id1 = interner.intern("test");
    let id2 = interner.intern("test");
    assert_eq!(id1, id2);
    assert_eq!(interner.resolve(id1), "test");
}

#[test]
fn test_env_scoping() {
    let mut interner = Interner::new();
    let x_id = interner.intern("x");
    let mut env = Env::new();

    env.define(x_id, Value::Integer(1));
    env.push_scope();
    env.define(x_id, Value::Integer(2));
    assert_eq!(env.get(x_id), Some(&Value::Integer(2)));

    env.pop_scope();
    assert_eq!(env.get(x_id), Some(&Value::Integer(1)));
}

#[test]
fn test_compiler_definitions() {
    let mut interner = Interner::new();
    let x_id = interner.intern("x");
    let y_id = interner.intern("y");
    let mut compiler = Compiler::new();

    compiler.define_var(x_id, Value::Integer(42));
    compiler.define_var(y_id, Value::Integer(100));

    assert_eq!(compiler.get_var(x_id), Some(&Value::Integer(42)));
    assert_eq!(compiler.get_var(y_id), Some(&Value::Integer(100)));
    assert_eq!(compiler.num_defs(), 2);
}
