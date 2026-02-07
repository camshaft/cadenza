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
    convert_source(&source)
}

/// Convert Cadenza source code to an S-expression AST representation.
pub fn convert_source(source: &str) -> Result<String> {
    let parsed = parse(source);
    let root = parsed.ast();
    let mut output = String::new();

    // Output all top-level expressions
    for expr in root.items() {
        if !output.is_empty() {
            writeln!(&mut output)?;
        }
        write_expr(&mut output, &expr)?;
    }

    Ok(output)
}

/// Write a string as a char sequence (u32 values in hex)
fn write_char_sequence(out: &mut String, s: &str) -> Result<()> {
    for ch in s.chars() {
        write!(out, " {:x}", ch as u32)?;
    }
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
            // Write error message so it shows what went wrong
            anyhow::bail!("Encountered error node in AST - parsing failed");
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
                        anyhow::bail!("Invalid escape sequence in string literal");
                    }
                }
            }
        }
    } else {
        // This is a parser bug - literals should always have values
        anyhow::bail!("Literal node missing value - this is a parser bug");
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
        assert_eq!(result, "(Integer 42)");
    }

    #[test]
    fn test_float_literal() {
        let result = convert_source("3.14").unwrap();
        assert_eq!(result, "(Float 3.14)");
    }

    #[test]
    fn test_string_literal() {
        let result = convert_source("\"hello\"").unwrap();
        // "hello" = 68 65 6c 6c 6f in hex
        assert_eq!(result, "(String 68 65 6c 6c 6f)");
    }

    #[test]
    fn test_identifier() {
        let result = convert_source("foo").unwrap();
        // "foo" chars as u32 in hex: f=66, o=6f, o=6f
        assert_eq!(result, "(Ident 66 6f 6f)");
    }

    #[test]
    fn test_simple_list() {
        let result = convert_source("[f, x]").unwrap();
        // __list__ = 5f 5f 6c 69 73 74 5f 5f, f=66, x=78
        assert_eq!(
            result,
            "(Apply (Synthetic 5f 5f 6c 69 73 74 5f 5f) (Ident 66) (Ident 78))"
        );
    }

    #[test]
    fn test_multiple_args_list() {
        let result = convert_source("[add, 1, 2]").unwrap();
        // add = 61 64 64
        assert_eq!(
            result,
            "(Apply (Synthetic 5f 5f 6c 69 73 74 5f 5f) (Ident 61 64 64) (Integer 1) (Integer 2))"
        );
    }

    #[test]
    fn test_nested_list() {
        let result = convert_source("[[f, x], y]").unwrap();
        // f=66, x=78, y=79
        assert_eq!(
            result,
            "(Apply (Synthetic 5f 5f 6c 69 73 74 5f 5f) (Apply (Synthetic 5f 5f 6c 69 73 74 5f 5f) (Ident 66) (Ident 78)) (Ident 79))"
        );
    }

    #[test]
    fn test_multiple_expressions() {
        let result = convert_source("42\n3.14\n\"hello\"").unwrap();
        // "hello" = 68 65 6c 6c 6f in hex
        assert_eq!(
            result,
            "(Integer 42)\n(Float 3.14)\n(String 68 65 6c 6c 6f)"
        );
    }
}
