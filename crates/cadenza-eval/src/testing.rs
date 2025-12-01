//! Testing utilities for the Cadenza evaluator.
//!
//! This module provides helper functions for testing evaluation, including
//! evaluating source strings and collecting results and diagnostics.

use crate::{
    compiler::Compiler, diagnostic::Diagnostic, env::Env, interner::InternedString, value::Value,
};
use cadenza_syntax::parse::parse;

/// The result of evaluating a source string, including both values and diagnostics.
#[derive(Debug)]
#[allow(dead_code)] // Fields are accessed via Debug derive for snapshot testing
pub struct EvalResult {
    /// The evaluated values for each top-level expression.
    pub values: Vec<Value>,
    /// Any diagnostics (errors, warnings) that were collected during evaluation.
    pub diagnostics: Vec<Diagnostic>,
}

/// Evaluate a source string and return all values and diagnostics.
///
/// This function evaluates the source and collects both results and diagnostics,
/// making it suitable for snapshot testing.
pub fn eval_all(src: &str) -> EvalResult {
    let parsed = parse(src);

    // Check for parse errors first
    if !parsed.errors.is_empty() {
        let diagnostics: Vec<Diagnostic> = parsed
            .errors
            .iter()
            .map(|err| *Diagnostic::parse_error(&err.message, err.span))
            .collect();
        return EvalResult {
            values: vec![],
            diagnostics,
        };
    }

    let root = parsed.ast();
    let mut env = Env::new();
    let mut compiler = Compiler::new();

    // Register the let and = special forms for variable declaration and assignment
    let let_id: InternedString = "let".into();
    let assign_id: InternedString = "=".into();
    env.define(
        let_id,
        Value::BuiltinSpecialForm(crate::eval::builtin_let()),
    );
    env.define(
        assign_id,
        Value::BuiltinSpecialForm(crate::eval::builtin_assign()),
    );

    let values = crate::eval(&root, &mut env, &mut compiler);
    let diagnostics = compiler.take_diagnostics();

    EvalResult {
        values,
        diagnostics,
    }
}
