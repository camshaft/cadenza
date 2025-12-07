//! WebAssembly bindings for the Cadenza compiler.
//!
//! This crate exposes the Cadenza compiler stages as WASM-callable functions
//! for use in web-based development tools like the Compiler Explorer.
//!
//! # Exported Functions
//!
//! - [`lex`]: Tokenizes source code, returns token information
//! - [`parse`]: Parses source into a concrete syntax tree (CST)
//! - [`ast`]: Converts to abstract syntax tree (AST)
//! - [`eval`]: Evaluates the source code
//! - LSP functions for language server protocol support

use cadenza_eval::{Compiler, Env, Value};
use cadenza_lsp::{core as lsp_core, lsp_types};
use cadenza_syntax::{lexer::Lexer, parse, token::Kind};
use serde::Serialize;
use wasm_bindgen::prelude::*;

/// Token information returned from lexing.
#[derive(Serialize)]
pub struct Token {
    /// The token kind as a string.
    pub kind: String,
    /// Start byte offset in the source.
    pub start: usize,
    /// End byte offset in the source.
    pub end: usize,
    /// The token text.
    pub text: String,
}

/// Result from the lexer.
#[derive(Serialize)]
pub struct LexResult {
    /// List of tokens.
    pub tokens: Vec<Token>,
    /// Whether lexing succeeded (always true for lexer).
    pub success: bool,
}

/// Node in the concrete syntax tree.
#[derive(Serialize)]
pub struct CstNode {
    /// The node kind.
    pub kind: String,
    /// Start byte offset.
    pub start: usize,
    /// End byte offset.
    pub end: usize,
    /// The node text (for leaf nodes).
    pub text: Option<String>,
    /// Child nodes.
    pub children: Vec<CstNode>,
}

/// Result from parsing.
#[derive(Serialize)]
pub struct ParseResult {
    /// The root CST node.
    pub tree: CstNode,
    /// Parse errors, if any.
    pub errors: Vec<ParseError>,
    /// Whether parsing succeeded without errors.
    pub success: bool,
}

/// A parse error.
#[derive(Serialize)]
pub struct ParseError {
    /// Start byte offset.
    pub start: usize,
    /// End byte offset.
    pub end: usize,
    /// Error message.
    pub message: String,
}

/// Node in the abstract syntax tree.
#[derive(Serialize)]
pub struct AstNode {
    /// The node type.
    #[serde(rename = "type")]
    pub node_type: String,
    /// Start byte offset.
    pub start: usize,
    /// End byte offset.
    pub end: usize,
    /// Optional value (for literals).
    pub value: Option<String>,
    /// Child nodes.
    pub children: Vec<AstNode>,
}

/// Result from AST conversion.
#[derive(Serialize)]
pub struct AstResult {
    /// The AST nodes.
    pub nodes: Vec<AstNode>,
    /// Parse errors, if any.
    pub errors: Vec<ParseError>,
    /// Whether conversion succeeded without errors.
    pub success: bool,
}

/// A value produced by evaluation.
#[derive(Serialize)]
pub struct EvalValue {
    /// The type of the value.
    #[serde(rename = "type")]
    pub value_type: String,
    /// String representation of the value.
    pub display: String,
}

/// A diagnostic from evaluation.
#[derive(Serialize)]
pub struct EvalDiagnostic {
    /// Diagnostic level (error, warning, hint).
    pub level: String,
    /// Error message.
    pub message: String,
    /// Start byte offset (if known).
    pub start: Option<usize>,
    /// End byte offset (if known).
    pub end: Option<usize>,
}

/// Result from evaluation.
#[derive(Serialize)]
pub struct EvalResult {
    /// Values produced by evaluation.
    pub values: Vec<EvalValue>,
    /// Diagnostics (errors, warnings, hints).
    pub diagnostics: Vec<EvalDiagnostic>,
    /// Whether evaluation succeeded without errors.
    pub success: bool,
}

/// Tokenizes source code and returns token information.
///
/// Returns a JSON object with:
/// - `tokens`: Array of token objects with `kind`, `start`, `end`, `text`
/// - `success`: Always true (lexer doesn't produce errors)
#[wasm_bindgen]
pub fn lex(source: &str) -> JsValue {
    let lexer = Lexer::new(source);
    let tokens: Vec<Token> = lexer
        .map(|tok| Token {
            kind: format!("{:?}", tok.kind),
            start: tok.span.start,
            end: tok.span.end,
            text: source[tok.span.start..tok.span.end].to_string(),
        })
        .collect();

    let result = LexResult {
        tokens,
        success: true,
    };

    serde_wasm_bindgen::to_value(&result).expect("Failed to serialize LexResult")
}

/// Converts a rowan SyntaxNode to our CstNode format.
fn syntax_node_to_cst(node: &cadenza_syntax::SyntaxNode) -> CstNode {
    use rowan::NodeOrToken;

    let kind = format!("{:?}", node.kind());
    let range = node.text_range();
    let start = range.start().into();
    let end = range.end().into();

    let mut children = Vec::new();

    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Node(n) => {
                children.push(syntax_node_to_cst(&n));
            }
            NodeOrToken::Token(t) => {
                children.push(CstNode {
                    kind: format!("{:?}", t.kind()),
                    start: t.text_range().start().into(),
                    end: t.text_range().end().into(),
                    text: Some(t.text().to_string()),
                    children: Vec::new(),
                });
            }
        }
    }

    // For leaf nodes (no children), include the text
    let text = if children.is_empty() {
        Some(node.text().to_string())
    } else {
        None
    };

    CstNode {
        kind,
        start,
        end,
        text,
        children,
    }
}

/// Parses source code and returns the concrete syntax tree (CST).
///
/// Returns a JSON object with:
/// - `tree`: The CST as a nested object with `kind`, `start`, `end`, `text`, `children`
/// - `errors`: Array of parse errors with `start`, `end`, `message`
/// - `success`: true if no errors
#[wasm_bindgen]
pub fn parse_source(source: &str) -> JsValue {
    let parsed = parse::parse(source);

    let tree = syntax_node_to_cst(&parsed.syntax());
    let errors: Vec<ParseError> = parsed
        .errors
        .iter()
        .map(|e| ParseError {
            start: e.span.start,
            end: e.span.end,
            message: e.message.clone(),
        })
        .collect();

    let success = errors.is_empty();

    let result = ParseResult {
        tree,
        errors,
        success,
    };

    serde_wasm_bindgen::to_value(&result).expect("Failed to serialize ParseResult")
}

/// Converts an AST expression to our AstNode format.
fn expr_to_ast(expr: &cadenza_syntax::ast::Expr) -> AstNode {
    use cadenza_syntax::ast::Expr;

    match expr {
        Expr::Apply(apply) => {
            let syntax = apply.syntax();
            let range = syntax.text_range();

            let mut children = Vec::new();

            // Add receiver as first child
            if let Some(expr) = apply.receiver().and_then(|r| r.value()) {
                children.push(expr_to_ast(&expr));
            }

            // Add arguments
            for arg in apply.arguments() {
                if let Some(expr) = arg.value() {
                    children.push(expr_to_ast(&expr));
                }
            }

            AstNode {
                node_type: "Apply".to_string(),
                start: range.start().into(),
                end: range.end().into(),
                value: None,
                children,
            }
        }
        Expr::Ident(ident) => {
            let syntax = ident.syntax();
            let range = syntax.text_range();
            AstNode {
                node_type: "Ident".to_string(),
                start: range.start().into(),
                end: range.end().into(),
                value: Some(syntax.text().to_string()),
                children: Vec::new(),
            }
        }
        Expr::Literal(lit) => {
            let syntax = lit.syntax();
            let range = syntax.text_range();
            AstNode {
                node_type: "Literal".to_string(),
                start: range.start().into(),
                end: range.end().into(),
                value: Some(syntax.text().to_string()),
                children: Vec::new(),
            }
        }
        Expr::Op(op) => {
            let syntax = op.syntax();
            let range = syntax.text_range();
            AstNode {
                node_type: "Op".to_string(),
                start: range.start().into(),
                end: range.end().into(),
                value: Some(syntax.text().to_string()),
                children: Vec::new(),
            }
        }
        Expr::Attr(attr) => {
            let syntax = attr.syntax();
            let range = syntax.text_range();

            let children = if let Some(expr) = attr.value() {
                vec![expr_to_ast(&expr)]
            } else {
                Vec::new()
            };

            AstNode {
                node_type: "Attr".to_string(),
                start: range.start().into(),
                end: range.end().into(),
                value: None,
                children,
            }
        }
        Expr::Synthetic(syn) => {
            let syntax = syn.syntax();
            let range = syntax.text_range();
            AstNode {
                node_type: "Synthetic".to_string(),
                start: range.start().into(),
                end: range.end().into(),
                value: Some(syn.identifier().to_string()),
                children: Vec::new(),
            }
        }
        Expr::Error(err) => {
            let syntax = err.syntax();
            let range = syntax.text_range();
            AstNode {
                node_type: "Error".to_string(),
                start: range.start().into(),
                end: range.end().into(),
                value: Some(syntax.text().to_string()),
                children: Vec::new(),
            }
        }
    }
}

/// Parses source code and returns the abstract syntax tree (AST).
///
/// Returns a JSON object with:
/// - `nodes`: Array of top-level AST nodes
/// - `errors`: Array of parse errors
/// - `success`: true if no errors
#[wasm_bindgen]
pub fn ast(source: &str) -> JsValue {
    let parsed = parse::parse(source);

    let errors: Vec<ParseError> = parsed
        .errors
        .iter()
        .map(|e| ParseError {
            start: e.span.start,
            end: e.span.end,
            message: e.message.clone(),
        })
        .collect();

    let root = parsed.ast();
    let nodes: Vec<AstNode> = root.items().map(|expr| expr_to_ast(&expr)).collect();

    let success = errors.is_empty();

    let result = AstResult {
        nodes,
        errors,
        success,
    };

    serde_wasm_bindgen::to_value(&result).expect("Failed to serialize AstResult")
}

/// Converts a Value to an EvalValue for serialization.
fn value_to_eval_value(value: &Value) -> EvalValue {
    EvalValue {
        value_type: value.type_of().to_string(),
        display: format!("{}", value),
    }
}

/// Evaluates source code and returns the results.
///
/// Returns a JSON object with:
/// - `values`: Array of evaluation results with `type` and `display`
/// - `diagnostics`: Array of diagnostics with `level`, `message`, `start`, `end`
/// - `success`: true if no errors
#[wasm_bindgen]
pub fn eval_source(source: &str) -> JsValue {
    let parsed = parse::parse(source);

    // Collect parse errors as diagnostics
    let mut diagnostics: Vec<EvalDiagnostic> = parsed
        .errors
        .iter()
        .map(|e| EvalDiagnostic {
            level: "error".to_string(),
            message: e.message.clone(),
            start: Some(e.span.start),
            end: Some(e.span.end),
        })
        .collect();

    let root = parsed.ast();
    let mut env = Env::with_standard_builtins();
    let mut compiler = Compiler::new();

    let results = cadenza_eval::eval(&root, &mut env, &mut compiler);

    // Add evaluation diagnostics
    for diag in compiler.take_diagnostics() {
        let level = match diag.level {
            cadenza_eval::DiagnosticLevel::Error => "error",
            cadenza_eval::DiagnosticLevel::Warning => "warning",
            cadenza_eval::DiagnosticLevel::Hint => "hint",
        };

        diagnostics.push(EvalDiagnostic {
            level: level.to_string(),
            message: diag.to_string(),
            start: diag.span.map(|s| s.start),
            end: diag.span.map(|s| s.end),
        });
    }

    let values: Vec<EvalValue> = results.iter().map(value_to_eval_value).collect();
    let success = diagnostics.iter().all(|d| d.level != "error");

    let result = EvalResult {
        values,
        diagnostics,
        success,
    };

    serde_wasm_bindgen::to_value(&result).expect("Failed to serialize EvalResult")
}

/// Returns the list of all token kinds for syntax highlighting.
#[wasm_bindgen]
pub fn get_token_kinds() -> JsValue {
    let kinds: Vec<String> = Kind::ALL.iter().map(|k| format!("{:?}", k)).collect();
    serde_wasm_bindgen::to_value(&kinds).expect("Failed to serialize token kinds")
}

/// LSP diagnostic information.
#[derive(Serialize)]
pub struct LspDiagnostic {
    /// Start line number (0-based).
    pub start_line: u32,
    /// Start character position (0-based).
    pub start_character: u32,
    /// End line number (0-based).
    pub end_line: u32,
    /// End character position (0-based).
    pub end_character: u32,
    /// Diagnostic message.
    pub message: String,
    /// Severity: "error", "warning", "info", or "hint".
    pub severity: String,
}

/// Get diagnostics for the given source code.
///
/// Returns an array of diagnostic objects with position information and messages.
#[wasm_bindgen]
pub fn lsp_diagnostics(source: &str) -> JsValue {
    let diagnostics = lsp_core::parse_to_diagnostics(source);

    let lsp_diagnostics: Vec<LspDiagnostic> = diagnostics
        .into_iter()
        .map(|d| LspDiagnostic {
            start_line: d.range.start.line,
            start_character: d.range.start.character,
            end_line: d.range.end.line,
            end_character: d.range.end.character,
            message: d.message,
            severity: match d.severity {
                Some(s) => match s {
                    lsp_types::DiagnosticSeverity::ERROR => "error",
                    lsp_types::DiagnosticSeverity::WARNING => "warning",
                    lsp_types::DiagnosticSeverity::INFORMATION => "info",
                    lsp_types::DiagnosticSeverity::HINT => "hint",
                    _ => "error",
                }
                .to_string(),
                None => "error".to_string(),
            },
        })
        .collect();

    serde_wasm_bindgen::to_value(&lsp_diagnostics).expect("Failed to serialize diagnostics")
}

/// Hover information.
#[derive(Serialize)]
pub struct LspHoverInfo {
    /// The hover content/text.
    pub content: String,
    /// Whether hover info was found.
    pub found: bool,
}

/// Get hover information for a position in the source code.
///
/// Returns hover information if available at the given position.
#[wasm_bindgen]
pub fn lsp_hover(source: &str, line: u32, character: u32) -> JsValue {
    let position = lsp_types::Position::new(line, character);
    let offset = lsp_core::position_to_offset(source, position);

    // Find the word boundaries around the offset
    let start = source[..offset]
        .rfind(|c: char| !c.is_alphanumeric() && c != '_')
        .map(|i| i + 1)
        .unwrap_or(0);

    let end = source[offset..]
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .map(|i| offset + i)
        .unwrap_or(source.len());

    let hover_info = if start < end {
        let word = &source[start..end];
        if !word.is_empty() {
            LspHoverInfo {
                content: format!("Symbol: `{}`\n\nType information coming soon!", word),
                found: true,
            }
        } else {
            LspHoverInfo {
                content: String::new(),
                found: false,
            }
        }
    } else {
        LspHoverInfo {
            content: String::new(),
            found: false,
        }
    };

    serde_wasm_bindgen::to_value(&hover_info).expect("Failed to serialize hover info")
}

/// Completion item.
#[derive(Serialize)]
pub struct LspCompletionItem {
    /// The label of this completion item.
    pub label: String,
    /// The kind of this completion item.
    pub kind: String,
    /// A human-readable string with additional information.
    pub detail: Option<String>,
}

/// Get completion items for a position in the source code.
///
/// Returns an array of completion items.
#[wasm_bindgen]
pub fn lsp_completions(source: &str, line: u32, character: u32) -> JsValue {
    // For now, return basic keyword completions
    let _position = lsp_types::Position::new(line, character);
    let _offset = lsp_core::position_to_offset(source, _position);

    let items = vec![
        LspCompletionItem {
            label: "let".to_string(),
            kind: "keyword".to_string(),
            detail: Some("Variable binding".to_string()),
        },
        LspCompletionItem {
            label: "fn".to_string(),
            kind: "keyword".to_string(),
            detail: Some("Function definition".to_string()),
        },
    ];

    serde_wasm_bindgen::to_value(&items).expect("Failed to serialize completion items")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lex_basic() {
        let source = "1 + 2";
        let lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.collect();
        assert!(!tokens.is_empty());
    }

    #[test]
    fn test_parse_basic() {
        let source = "1 + 2";
        let parsed = parse::parse(source);
        assert!(parsed.errors.is_empty());
    }

    #[test]
    fn test_eval_basic() {
        let source = "1 + 2";
        let parsed = parse::parse(source);
        let root = parsed.ast();
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let results = cadenza_eval::eval(&root, &mut env, &mut compiler);
        assert_eq!(results.len(), 1);
        assert!(!compiler.has_errors());
    }

    #[test]
    fn test_eval_with_standard_builtins() {
        // Test that Env::with_standard_builtins() works correctly
        let source = "let x = 42\nx";
        let parsed = parse::parse(source);
        let root = parsed.ast();
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let results = cadenza_eval::eval(&root, &mut env, &mut compiler);
        assert_eq!(results.len(), 2);
        assert!(!compiler.has_errors());
        // First result is the assignment value (42)
        assert_eq!(results[0], Value::Integer(42));
        // Second result is the value of x (42)
        assert_eq!(results[1], Value::Integer(42));
    }

    #[test]
    fn test_eval_let_with_expression() {
        // Test let with a complex expression on RHS
        let source = "let x = 1 + 2\nx";
        let parsed = parse::parse(source);
        let root = parsed.ast();
        let mut env = Env::with_standard_builtins();
        let mut compiler = Compiler::new();

        let results = cadenza_eval::eval(&root, &mut env, &mut compiler);
        assert_eq!(results.len(), 2);
        assert!(!compiler.has_errors());
        assert_eq!(results[0], Value::Integer(3));
        assert_eq!(results[1], Value::Integer(3));
    }
}
