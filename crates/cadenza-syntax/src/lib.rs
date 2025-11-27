pub mod ast;
mod generated;
mod iter;
pub mod lexer;
pub mod parse;
pub mod span;
pub mod token;

#[cfg(test)]
mod testing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Lang;

impl rowan::Language for Lang {
    type Kind = token::Kind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> Self::Kind {
        assert!(raw.0 <= token::Kind::Eof as u16);
        unsafe { std::mem::transmute::<u16, token::Kind>(raw.0) }
    }

    fn kind_to_raw(kind: Self::Kind) -> rowan::SyntaxKind {
        kind.into()
    }
}

type SyntaxNode = rowan::SyntaxNode<Lang>;
type SyntaxToken = rowan::SyntaxToken<Lang>;
type SyntaxElement = rowan::NodeOrToken<SyntaxNode, Lang>;
type SyntaxElementChildren = rowan::SyntaxElementChildren<Lang>;
type SyntaxNodeChildren = rowan::SyntaxNodeChildren<Lang>;
