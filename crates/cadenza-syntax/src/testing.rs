use crate::token::Token;

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
