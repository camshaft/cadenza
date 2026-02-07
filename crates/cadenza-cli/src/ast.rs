//! AST output module - Convert Cadenza source to S-expression format for K framework.
//!
//! This module provides functionality to parse Cadenza source code and output
//! the Abstract Syntax Tree in a parenthesized S-expression format that can be
//! easily parsed by the K framework without ambiguity.

use anyhow::{Context, Result};
use cadenza_syntax::{ast::*, parse::parse};
use std::fmt::Write;
use std::fs;
use std::path::Path;

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

/// Write an expression as an S-expression.
fn write_expr(out: &mut String, expr: &Expr) -> Result<()> {
    match expr {
        Expr::Literal(lit) => write_literal(out, lit)?,
        Expr::Ident(ident) => write!(out, "(Ident \"{}\")", ident.syntax().text())?,
        Expr::Apply(apply) => write_apply(out, apply)?,
        Expr::Op(op) => write!(out, "(Op \"{}\")", op.syntax().text())?,
        Expr::Synthetic(syn) => write!(out, "(Synthetic \"{}\")", syn.identifier())?,
        Expr::Error(_) => write!(out, "(Error)")?,
    }
    Ok(())
}

/// Write a literal as an S-expression.
fn write_literal(out: &mut String, lit: &Literal) -> Result<()> {
    if let Some(value) = lit.value() {
        match value {
            LiteralValue::Integer(int_val) => {
                write!(out, "(Int \"{}\")", int_val.syntax().text())?;
            }
            LiteralValue::Float(float_val) => {
                write!(out, "(Float \"{}\")", float_val.syntax().text())?;
            }
            LiteralValue::String(str_val) => {
                // Escape the string content for S-expression output
                let text = str_val.syntax().text();
                let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
                write!(out, "(String \"{}\")", escaped)?;
            }
            LiteralValue::StringWithEscape(str_val) => {
                // For strings with escapes, output the raw text
                let text = str_val.syntax().text();
                let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
                write!(out, "(String \"{}\")", escaped)?;
            }
        }
    } else {
        write!(out, "(Literal)")?;
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
        assert_eq!(result, "(Int \"42\")");
    }

    #[test]
    fn test_float_literal() {
        let result = convert_source("3.14").unwrap();
        assert_eq!(result, "(Float \"3.14\")");
    }

    #[test]
    fn test_string_literal() {
        let result = convert_source("\"hello\"").unwrap();
        assert_eq!(result, "(String \"hello\")");
    }

    #[test]
    fn test_identifier() {
        let result = convert_source("foo").unwrap();
        assert_eq!(result, "(Ident \"foo\")");
    }

    #[test]
    fn test_simple_apply() {
        let result = convert_source("[f, x]").unwrap();
        assert_eq!(result, "(Apply (Ident \"f\") (Ident \"x\"))");
    }

    #[test]
    fn test_multiple_args_apply() {
        let result = convert_source("[add, 1, 2]").unwrap();
        assert_eq!(result, "(Apply (Ident \"add\") (Int \"1\") (Int \"2\"))");
    }

    #[test]
    fn test_nested_apply() {
        let result = convert_source("[[f, x], y]").unwrap();
        assert_eq!(
            result,
            "(Apply (Apply (Ident \"f\") (Ident \"x\")) (Ident \"y\"))"
        );
    }

    #[test]
    fn test_multiple_expressions() {
        let result = convert_source("42\n3.14\n\"hello\"").unwrap();
        assert_eq!(result, "(Int \"42\")\n(Float \"3.14\")\n(String \"hello\")");
    }
}
