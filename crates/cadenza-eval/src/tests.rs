//! Integration tests for the Cadenza evaluator.

use crate::{
    compiler::Compiler,
    diagnostic::Diagnostic,
    env::Env,
    interner::InternedString,
    value::{BuiltinFn, Type, Value},
};
use cadenza_syntax::parse::parse;

/// Helper to evaluate a source string and return all values.
fn eval_all(src: &str) -> Result<Vec<Value>, Box<Diagnostic>> {
    let parsed = parse(src);
    if let Some(err) = parsed.errors.first() {
        return Err(Diagnostic::parse_error(&err.message, err.span));
    }
    let root = parsed.ast();
    let mut env = Env::new();
    let mut compiler = Compiler::new();
    let results = crate::eval(&root, &mut env, &mut compiler);
    if compiler.has_errors() {
        // Return the first error for backwards compatibility in tests
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

/// Helper to evaluate a single expression.
fn eval_one(src: &str) -> Result<Value, Box<Diagnostic>> {
    eval_all(src)?
        .into_iter()
        .next()
        .ok_or_else(|| Diagnostic::syntax("no expressions"))
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
    assert!(matches!(
        result.unwrap_err().kind,
        crate::diagnostic::DiagnosticKind::UndefinedVariable(_)
    ));
}

#[test]
fn test_builtin_function() {
    // Note: "add 1 2" is parsed as ((add 1) 2), i.e., nested applies.
    // For simple testing, we use a single-argument function.
    let parsed = parse("inc 5");
    let root = parsed.ast();
    let mut env = Env::new();
    let mut compiler = Compiler::new();

    // Define a builtin inc function (increment by 1)
    let inc_id: InternedString = "inc".into();
    env.define(
        inc_id,
        Value::BuiltinFn(BuiltinFn {
            name: "inc",
            signature: Type::function(vec![Type::Integer], Type::Integer),
            func: |args| {
                if args.len() != 1 {
                    return Err(Diagnostic::arity(1, args.len()));
                }
                match &args[0] {
                    Value::Integer(a) => Ok(Value::Integer(a + 1)),
                    _ => Err(Diagnostic::type_error(Type::Integer, args[0].type_of())),
                }
            },
        }),
    );

    let results = crate::eval(&root, &mut env, &mut compiler);
    assert!(!compiler.has_errors());
    assert_eq!(results[0], Value::Integer(6));
}

#[test]
fn test_variable_from_environment() {
    let parsed = parse("x + y");
    let root = parsed.ast();
    let mut env = Env::new();
    let mut compiler = Compiler::new();

    let x_id: InternedString = "x".into();
    let y_id: InternedString = "y".into();
    env.define(x_id, Value::Integer(10));
    env.define(y_id, Value::Integer(20));

    let results = crate::eval(&root, &mut env, &mut compiler);
    assert!(!compiler.has_errors());
    assert_eq!(results[0], Value::Integer(30));
}

#[test]
fn test_variable_from_compiler() {
    let parsed = parse("global_var");
    let root = parsed.ast();
    let mut env = Env::new();
    let mut compiler = Compiler::new();

    let id: InternedString = "global_var".into();
    compiler.define_var(id, Value::Integer(42));

    let results = crate::eval(&root, &mut env, &mut compiler);
    assert!(!compiler.has_errors());
    assert_eq!(results[0], Value::Integer(42));
}

#[test]
fn test_division_by_zero() {
    let result = eval_one("1 / 0");
    assert!(result.is_err());
}

#[test]
fn test_interner_consistency() {
    let s1: InternedString = "test".into();
    let s2: InternedString = "test".into();
    assert_eq!(s1, s2);
    assert_eq!(&*s1, "test");
}

#[test]
fn test_env_scoping() {
    let x_id: InternedString = "x".into();
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
    let x_id: InternedString = "x".into();
    let y_id: InternedString = "y".into();
    let mut compiler = Compiler::new();

    compiler.define_var(x_id, Value::Integer(42));
    compiler.define_var(y_id, Value::Integer(100));

    assert_eq!(compiler.get_var(x_id), Some(&Value::Integer(42)));
    assert_eq!(compiler.get_var(y_id), Some(&Value::Integer(100)));
    assert_eq!(compiler.num_defs(), 2);
}

#[test]
fn test_eval_collecting_integration() {
    // Test that eval properly collects errors during evaluation
    // while still producing results for valid expressions
    let src = "x + 1\n2 + 3\ny + 4";
    let parsed = parse(src);
    let root = parsed.ast();
    let mut env = Env::new();
    let mut compiler = Compiler::new();

    let results = crate::eval(&root, &mut env, &mut compiler);

    // Should get 3 results
    assert_eq!(results.len(), 3);
    // First and third are errors (undefined x, y) -> Nil
    assert_eq!(results[0], Value::Nil);
    assert_eq!(results[1], Value::Integer(5)); // 2 + 3 = 5
    assert_eq!(results[2], Value::Nil);

    // Check diagnostics
    assert_eq!(compiler.num_diagnostics(), 2);
    assert!(compiler.has_errors());
}

#[test]
fn test_eval_collecting_with_defined_variables() {
    // Test that defined variables work in eval mode
    let src = "x\ny + 1\nz";
    let parsed = parse(src);
    let root = parsed.ast();
    let mut env = Env::new();
    let mut compiler = Compiler::new();

    // Define x in env
    let x_id: InternedString = "x".into();
    env.define(x_id, Value::Integer(42));

    // Define z in compiler
    let z_id: InternedString = "z".into();
    compiler.define_var(z_id, Value::Integer(100));

    let results = crate::eval(&root, &mut env, &mut compiler);

    assert_eq!(results.len(), 3);
    assert_eq!(results[0], Value::Integer(42)); // x from env
    assert_eq!(results[1], Value::Nil); // y + 1 fails (undefined y)
    assert_eq!(results[2], Value::Integer(100)); // z from compiler

    // Only one error (undefined y)
    assert_eq!(compiler.num_diagnostics(), 1);
}

#[test]
fn test_parse_error_message() {
    // Test that parse errors return actual error messages instead of generic "parse errors: [...]"
    let result = eval_one("[1, , 2]"); // Array with missing element
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(
        &err.kind,
        crate::diagnostic::DiagnosticKind::ParseError(_)
    ));
    // The error message should contain the actual parse error, not "parse errors: [...]"
    let msg = format!("{}", err);
    assert!(
        msg.contains("expected expression before comma"),
        "Expected actual parse error message, got: {}",
        msg
    );
    // The error should have a span
    assert!(err.span.is_some());
}
