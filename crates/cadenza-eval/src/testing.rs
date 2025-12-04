//! Testing utilities for the Cadenza evaluator.
//!
//! This module provides helper functions for testing evaluation, including
//! evaluating source strings and collecting results and diagnostics.

use crate::{compiler::Compiler, diagnostic::Diagnostic, env::Env, value::Value};
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
            .into_iter()
            .map(|err| *Box::<Diagnostic>::from(err))
            .collect();
        return EvalResult {
            values: vec![],
            diagnostics,
        };
    }

    let root = parsed.ast();
    let mut env = Env::with_standard_builtins();
    let mut compiler = Compiler::new();

    let values = crate::eval(&root, &mut env, &mut compiler);
    let diagnostics = compiler.take_diagnostics();

    EvalResult {
        values,
        diagnostics,
    }
}

/// Parse a source string and return the AST root.
///
/// This function parses the source and returns the AST, making it suitable
/// for snapshot testing the parse tree structure.
pub fn ast(src: &str) -> cadenza_syntax::ast::Root {
    let parsed = parse(src);
    parsed.ast()
}
