//! Integration tests for the Cadenza evaluator.
//!
//! Most basic evaluation tests have been moved to snapshot-based tests in
//! the test-data/ directory. This file contains tests that require special
//! setup (custom builtins, pre-populated environments) or test internal
//! behavior that is not suitable for snapshot testing.

use crate::{
    compiler::Compiler,
    diagnostic::Diagnostic,
    env::Env,
    interner::InternedString,
    value::{BuiltinFn, Type, Value},
};
use cadenza_syntax::parse::parse;

#[test]
fn test_builtin_function() {
    let parsed = parse("inc 5");
    let root = parsed.ast();
    let mut env = Env::new();
    let mut compiler = Compiler::new();

    let inc_id: InternedString = "inc".into();
    env.define(
        inc_id,
        Value::BuiltinFn(BuiltinFn {
            name: "inc",
            signature: Type::function(vec![Type::Integer], Type::Integer),
            func: |args, _ctx| {
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
fn test_list_parsing_and_eval() {
    let src = "let l = [1, 2, 3]\nl";
    let parsed = parse(src);
    
    println!("Parse errors: {:?}", parsed.errors);
    println!("AST: {:#?}", parsed.ast());
    
    let mut env = Env::with_standard_builtins();
    let mut compiler = Compiler::new();
    
    let results = crate::eval(&parsed.ast(), &mut env, &mut compiler);
    
    println!("\nResults:");
    for (i, result) in results.iter().enumerate() {
        println!("  [{}] {result:?}", i);
    }
    
    println!("\nDiagnostics:");
    for diag in compiler.diagnostics() {
        println!("  {:?}", diag);
    }
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
    let parsed = parse("x + y");
    let root = parsed.ast();
    let mut env = Env::new();
    let mut compiler = Compiler::new();

    let x_id: InternedString = "x".into();
    let y_id: InternedString = "y".into();
    compiler.define_var(x_id, Value::Integer(5));
    compiler.define_var(y_id, Value::Integer(15));

    let results = crate::eval(&root, &mut env, &mut compiler);
    assert!(!compiler.has_errors());
    assert_eq!(results[0], Value::Integer(20));
}

#[test]
fn test_compiler_definitions() {
    let mut compiler = Compiler::new();
    let name: InternedString = "foo".into();

    compiler.define_var(name, Value::Integer(42));
    assert_eq!(compiler.num_defs(), 1);
    assert_eq!(compiler.get_var(name), Some(&Value::Integer(42)));
}

#[test]
fn test_env_scoping() {
    let mut env = Env::new();
    let name: InternedString = "x".into();

    env.define(name, Value::Integer(1));
    env.push_scope();
    env.define(name, Value::Integer(2));

    assert_eq!(env.get(name), Some(&Value::Integer(2)));
    env.pop_scope();
    assert_eq!(env.get(name), Some(&Value::Integer(1)));
}

#[test]
fn test_interner_consistency() {
    let s1: InternedString = "hello".into();
    let s2: InternedString = "hello".into();
    let s3: InternedString = "world".into();

    assert_eq!(s1, s2);
    assert_ne!(s1, s3);
}

#[test]
fn test_eval_collecting_integration() {
    let parsed = parse("let x = 1\nlet y = 2\nx + y");
    let root = parsed.ast();
    let mut env = Env::with_standard_builtins();
    let mut compiler = Compiler::new();

    let results = crate::eval(&root, &mut env, &mut compiler);
    assert!(!compiler.has_errors());
    assert_eq!(results.len(), 3);
    assert_eq!(results[0], Value::Integer(1));
    assert_eq!(results[1], Value::Integer(2));
    assert_eq!(results[2], Value::Integer(3));
}

#[test]
fn test_eval_collecting_with_defined_variables() {
    let parsed = parse("x\ny");
    let root = parsed.ast();
    let mut env = Env::new();
    let mut compiler = Compiler::new();

    let x_id: InternedString = "x".into();
    let y_id: InternedString = "y".into();
    env.define(x_id, Value::Integer(42));
    compiler.define_var(y_id, Value::Integer(100));

    let results = crate::eval(&root, &mut env, &mut compiler);
    assert!(!compiler.has_errors());
    assert_eq!(results[0], Value::Integer(42));
    assert_eq!(results[1], Value::Integer(100));
}

#[test]
fn test_diagnostic_with_span() {
    use cadenza_syntax::span::Span;

    let span = Span::new(10, 20);
    let err = Diagnostic::syntax("test error").with_span(span);

    assert!(err.span.is_some());
}

#[test]
fn test_fn_multi_arg() {
    let src = "fn add x y = x + y\nadd 3 5";
    let parsed = parse(src);

    let root = parsed.ast();
    let mut env = Env::with_standard_builtins();
    let mut compiler = Compiler::new();

    let results = crate::eval(&root, &mut env, &mut compiler);

    assert!(!compiler.has_errors());
    assert_eq!(results[1], Value::Integer(8));
}
