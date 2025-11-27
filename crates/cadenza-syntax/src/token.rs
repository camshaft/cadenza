use crate::span::Span;
use serde::{Deserialize, Serialize};

pub use crate::generated::token::{Kind, Op};

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct Token {
    pub span: Span,
    pub kind: Kind,
}

impl PartialEq<Kind> for Token {
    fn eq(&self, other: &Kind) -> bool {
        self.kind == *other
    }
}

impl PartialEq<Token> for Kind {
    fn eq(&self, other: &Token) -> bool {
        *self == other.kind
    }
}

impl Kind {
    pub fn spanned<S: Into<Span>>(self, span: S) -> Token {
        let span = span.into();
        Token { span, kind: self }
    }
}

impl From<Kind> for rowan::SyntaxKind {
    fn from(kind: Kind) -> Self {
        Self(kind as u16)
    }
}
