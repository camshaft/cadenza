use crate::{parse::ParseError, token::Token};

pub fn lex(s: &str) -> Vec<Token> {
    crate::lexer::Lexer::new(s).collect()
}

pub fn cst(s: &str) -> crate::SyntaxNode {
    let v = crate::parse::parse(s);
    assert!(v.errors.is_empty(), "{:?}", v.errors);
    v.syntax()
}

pub fn ast(s: &str) -> crate::ast::Root {
    let v = crate::parse::parse(s);
    assert!(v.errors.is_empty(), "{:?}", v.errors);
    v.ast()
}

pub fn parse_errors(s: &str) -> Vec<ParseError> {
    let v = crate::parse::parse(s);
    v.errors
}

/// Parse and return CST without asserting on errors (for invalid input tests)
pub fn cst_no_assert(s: &str) -> crate::SyntaxNode {
    let v = crate::parse::parse(s);
    v.syntax()
}

/// Parse and return AST without asserting on errors (for invalid input tests)
pub fn ast_no_assert(s: &str) -> crate::ast::Root {
    let v = crate::parse::parse(s);
    v.ast()
}
