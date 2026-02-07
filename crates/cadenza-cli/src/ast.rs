//! AST output module - Convert Cadenza source to S-expression format for K framework.
//!
//! This module provides functionality to parse Cadenza source code and output
//! the Abstract Syntax Tree in a parenthesized S-expression format that can be
//! easily parsed by the K framework without ambiguity.

use anyhow::{Context, Result};
use cadenza_syntax::{ast::*, parse::parse};
use std::{fmt::Write, fs, path::Path};

/// Convert a Cadenza source file to an S-expression AST representation.
pub fn convert_file(path: &Path) -> Result<String> {
    let source = fs::read_to_string(path)
        .with_context(|| format!("Failed to read file: {}", path.display()))?;
    let path_str = path.display().to_string();
    convert_source_with_path(&source, &path_str)
}

/// Convert Cadenza source code to an S-expression AST representation without file path.
#[allow(dead_code)] // Used in tests
pub fn convert_source(source: &str) -> Result<String> {
    convert_source_with_path(source, "<unknown>")
}

/// Convert Cadenza source code to an S-expression AST representation with file path.
fn convert_source_with_path(source: &str, file_path: &str) -> Result<String> {
    let parsed = parse(source);
    let root = parsed.ast();
    let mut output = String::new();

    // Start with File node
    write!(output, "(File")?;
    write_char_sequence(&mut output, file_path)?;

    // Output all top-level expressions with spans
    for expr in root.items() {
        write!(output, " ")?;
        write_expr_with_span(&mut output, &expr)?;
    }

    write!(output, ")")?;

    Ok(output)
}

/// Write a string as a char sequence (u32 values in decimal)
fn write_char_sequence(out: &mut String, s: &str) -> Result<()> {
    for ch in s.chars() {
        write!(out, " {}", ch as u32)?;
    }
    Ok(())
}

/// Write an expression with span information as an S-expression.
fn write_expr_with_span(out: &mut String, expr: &Expr) -> Result<()> {
    let span = expr.span();
    write!(out, "(Span {} {} ", span.start, span.end)?;
    write_expr(out, expr)?;
    write!(out, ")")?;
    Ok(())
}

/// Write an expression as an S-expression.
fn write_expr(out: &mut String, expr: &Expr) -> Result<()> {
    match expr {
        Expr::Literal(lit) => write_literal(out, lit)?,
        Expr::Ident(ident) => {
            write!(out, "(Ident")?;
            let text = ident.syntax().text();
            write_char_sequence(out, &text)?;
            write!(out, ")")?;
        }
        Expr::Apply(apply) => write_apply(out, apply)?,
        Expr::Op(op) => {
            write!(out, "(Op")?;
            let text = op.syntax().text();
            write_char_sequence(out, &text)?;
            write!(out, ")")?;
        }
        Expr::Synthetic(syn) => {
            write!(out, "(Synthetic")?;
            write_char_sequence(out, syn.identifier())?;
            write!(out, ")")?;
        }
        Expr::Error(_) => {
            // Emit error node in AST for error modeling in semantics
            write!(out, "(Error)")?;
        }
    }
    Ok(())
}

/// Write a literal as an S-expression.
fn write_literal(out: &mut String, lit: &Literal) -> Result<()> {
    if let Some(value) = lit.value() {
        match value {
            LiteralValue::Integer(int_val) => {
                // Output as (Integer 123) with the value directly, not as a string
                write!(out, "(Integer {})", int_val.syntax().text())?;
            }
            LiteralValue::Float(float_val) => {
                // Output as (Float 3.14) with the value directly, not as a string
                write!(out, "(Float {})", float_val.syntax().text())?;
            }
            LiteralValue::String(str_val) => {
                // Encode string as char list (u32 values in hex): (String 68 65 6c 6c 6f)
                let text = str_val.syntax().text();
                write!(out, "(String")?;
                write_char_sequence(out, &text)?;
                write!(out, ")")?;
            }
            LiteralValue::StringWithEscape(str_val) => {
                // For strings with escapes, we need to unescape first then encode
                match str_val.unescaped() {
                    Ok(unescaped) => {
                        write!(out, "(String")?;
                        write_char_sequence(out, &unescaped)?;
                        write!(out, ")")?;
                    }
                    Err(_) => {
                        // Emit error for invalid escape sequence - modeling in semantics
                        write!(out, "(Error)")?;
                    }
                }
            }
        }
    } else {
        // Emit error for missing value - should be modeled in semantics
        write!(out, "(Error)")?;
    }
    Ok(())
}

/// Write an application as an S-expression.
fn write_apply(out: &mut String, apply: &Apply) -> Result<()> {
    write!(out, "(Apply ")?;

    // Write receiver
    if let Some(receiver) = apply.receiver() {
        if let Some(receiver_expr) = receiver.value() {
            write_expr(out, &receiver_expr)?;
        } else {
            write!(out, "(Error)")?;
        }
    } else {
        write!(out, "(Error)")?;
    }

    // Write arguments
    for arg in apply.arguments() {
        write!(out, " ")?;
        if let Some(arg_expr) = arg.value() {
            write_expr(out, &arg_expr)?;
        } else {
            write!(out, "(Error)")?;
        }
    }

    write!(out, ")")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_integer_literal() {
        let result = convert_source("42").unwrap();
        // <unknown> = 60 117 110 107 110 111 119 110 62
        assert_eq!(
            result,
            "(File 60 117 110 107 110 111 119 110 62 (Span 0 2 (Integer 42)))"
        );
    }

    #[test]
    fn test_float_literal() {
        let result = convert_source("3.14").unwrap();
        assert_eq!(
            result,
            "(File 60 117 110 107 110 111 119 110 62 (Span 0 4 (Float 3.14)))"
        );
    }

    #[test]
    fn test_string_literal() {
        let result = convert_source("\"hello\"").unwrap();
        // "hello" = 104 101 108 108 111, <unknown> = 60 117 110 107 110 111 119 110 62
        assert_eq!(
            result,
            "(File 60 117 110 107 110 111 119 110 62 (Span 0 7 (String 104 101 108 108 111)))"
        );
    }

    #[test]
    fn test_identifier() {
        let result = convert_source("foo").unwrap();
        // "foo" = 102 111 111
        assert_eq!(
            result,
            "(File 60 117 110 107 110 111 119 110 62 (Span 0 3 (Ident 102 111 111)))"
        );
    }

    #[test]
    fn test_simple_list() {
        let result = convert_source("[f, x]").unwrap();
        // __list__ = 95 95 108 105 115 116 95 95, f=102, x=120
        assert_eq!(
            result,
            "(File 60 117 110 107 110 111 119 110 62 (Span 0 6 (Apply (Synthetic 95 95 108 105 115 116 95 95) (Ident 102) (Ident 120))))"
        );
    }

    #[test]
    fn test_multiple_args_list() {
        let result = convert_source("[add, 1, 2]").unwrap();
        // add = 97 100 100
        assert_eq!(
            result,
            "(File 60 117 110 107 110 111 119 110 62 (Span 0 11 (Apply (Synthetic 95 95 108 105 115 116 95 95) (Ident 97 100 100) (Integer 1) (Integer 2))))"
        );
    }

    #[test]
    fn test_nested_list() {
        let result = convert_source("[[f, x], y]").unwrap();
        // f=102, x=120, y=121
        assert_eq!(
            result,
            "(File 60 117 110 107 110 111 119 110 62 (Span 0 11 (Apply (Synthetic 95 95 108 105 115 116 95 95) (Apply (Synthetic 95 95 108 105 115 116 95 95) (Ident 102) (Ident 120)) (Ident 121))))"
        );
    }

    #[test]
    fn test_multiple_expressions() {
        let result = convert_source("42\n3.14\n\"hello\"").unwrap();
        // "hello" = 104 101 108 108 111
        assert_eq!(
            result,
            "(File 60 117 110 107 110 111 119 110 62 (Span 0 2 (Integer 42)) (Span 3 7 (Float 3.14)) (Span 8 15 (String 104 101 108 108 111)))"
        );
    }
}
