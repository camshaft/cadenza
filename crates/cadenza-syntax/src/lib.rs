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

impl cadenza_tree::Language for Lang {
    type Kind = token::Kind;

    fn kind_from_raw(raw: cadenza_tree::SyntaxKind) -> Self::Kind {
        assert!(raw.into_raw() <= token::Kind::Eof as u16);
        unsafe { std::mem::transmute::<u16, token::Kind>(raw.into_raw()) }
    }

    fn kind_to_raw(kind: Self::Kind) -> cadenza_tree::SyntaxKind {
        kind.into()
    }
}

impl Lang {
    /// Creates a SyntaxNode from a GreenNode.
    pub fn parse_node(green: cadenza_tree::GreenNode) -> SyntaxNode {
        cadenza_tree::SyntaxNode::new_root(green)
    }
}

/// Re-export SyntaxNode for external use.
pub type SyntaxNode = cadenza_tree::SyntaxNode<Lang>;

#[expect(dead_code)]
type SyntaxToken = cadenza_tree::SyntaxToken<Lang>;
#[expect(dead_code)]
type SyntaxElement = cadenza_tree::SyntaxElement<Lang>;
