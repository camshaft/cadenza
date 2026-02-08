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

/// Convert Cadenza source code to an S-expression AST representation with file path.
fn convert_source_with_path(source: &str, file_path: &str) -> Result<String> {
    let parsed = parse(source);
    let root = parsed.ast();
    let mut output = String::new();

    // Start with File node
    write!(output, "(File (Path")?;
    write_char_sequence(&mut output, file_path)?;
    write!(output, ")")?;

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
    write!(out, "(Span ({} {}) ", span.start, span.end)?;
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
            LiteralValue::Integer(int_val) => match int_val.parse() {
                Ok(value) => {
                    write!(out, "(Integer {value})")?;
                }
                Err(_err) => {
                    write!(out, "(Error)")?;
                }
            },
            LiteralValue::Float(float_val) => match float_val.parse() {
                Ok(value) => {
                    // Emit float as its bit representation to avoid the semantic layer parsing floats
                    write!(out, "(Float {})", value.to_bits())?;
                }
                Err(_err) => {
                    write!(out, "(Error)")?;
                }
            },
            LiteralValue::String(str_val) => {
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

    /// Convert Cadenza source code to an S-expression AST representation without file path.
    fn convert_source(source: &str) -> Result<String> {
        convert_source_with_path(source, "<test>")
    }

    #[test]
    fn test_integer_literal() {
        let result = convert_source("42").unwrap();
        insta::assert_snapshot!(result);
    }

    #[test]
    fn test_float_literal() {
        let result = convert_source("3.14").unwrap();
        insta::assert_snapshot!(result);
    }

    #[test]
    fn test_string_literal() {
        let result = convert_source("\"hello\"").unwrap();
        insta::assert_snapshot!(result);
    }

    #[test]
    fn test_identifier() {
        let result = convert_source("foo").unwrap();
        insta::assert_snapshot!(result);
    }

    #[test]
    fn test_simple_list() {
        let result = convert_source("[f, x]").unwrap();
        insta::assert_snapshot!(result);
    }

    #[test]
    fn test_multiple_args_list() {
        let result = convert_source("[add, 1, 2]").unwrap();
        insta::assert_snapshot!(result);
    }

    #[test]
    fn test_nested_list() {
        let result = convert_source("[[f, x], y]").unwrap();
        insta::assert_snapshot!(result);
    }

    #[test]
    fn test_multiple_expressions() {
        let result = convert_source("42\n3.14\n\"hello\"").unwrap();
        insta::assert_snapshot!(result);
    }
}
