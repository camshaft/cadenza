use crate::{Lang, token::Kind};

type SyntaxNode = rowan::SyntaxNode<Lang>;
type SyntaxToken = rowan::SyntaxToken<Lang>;
type SyntaxElement = rowan::NodeOrToken<SyntaxNode, SyntaxToken>;
type SyntaxElementChildren = rowan::SyntaxElementChildren<SyntaxNode>;
type SyntaxNodeChildren = rowan::SyntaxNodeChildren<SyntaxNode>;

macro_rules! ast_node {
    ($name:ident, $kind:ident) => {
        #[derive(Clone, PartialEq, Eq, Hash)]
        #[repr(transparent)]
        struct $name(SyntaxNode);

        impl $name {
            fn cast(node: SyntaxNode) -> Option<Self> {
                if node.kind() == Kind::$kind {
                    Some(Self(node))
                } else {
                    None
                }
            }

            fn syntax(&self) -> &SyntaxNode {
                &self.0
            }

            fn into_syntax(self) -> SyntaxNode {
                self.0
            }
        }
    };
}

ast_node!(Root, NodeRoot);
// ast_node!();
