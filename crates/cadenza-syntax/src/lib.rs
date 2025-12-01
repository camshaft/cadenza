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

impl Lang {
    /// Creates a SyntaxNode from a GreenNode.
    pub fn parse_node(green: rowan::GreenNode) -> SyntaxNode {
        rowan::SyntaxNode::new_root(green)
    }
}

/// Re-export SyntaxNode for external use.
pub type SyntaxNode = rowan::SyntaxNode<Lang>;

#[expect(dead_code)]
type SyntaxToken = rowan::SyntaxToken<Lang>;
#[expect(dead_code)]
type SyntaxElement = rowan::NodeOrToken<SyntaxNode, Lang>;
#[expect(dead_code)]
type SyntaxElementChildren = rowan::SyntaxElementChildren<Lang>;
#[expect(dead_code)]
type SyntaxNodeChildren = rowan::SyntaxNodeChildren<Lang>;
