use crate::span::Span;
use serde::{Deserialize, Serialize};

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

macro_rules! kind_value {
    () => {
        None
    };
    ($value:literal) => {
        Some($value)
    };
}

macro_rules! impl_kind {
    (pub enum Kind { $($ident:ident $(= $value:literal)?),* $(,)? }) => {
        #[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[repr(u16)]
        pub enum Kind {
            $(
                $(#[doc = $value])?
                $ident,
            )*
        }

        impl Kind {
            pub const ALL: &'static [Kind] = &[
                $(
                    Kind::$ident,
                )*
            ];

            pub const fn as_str(&self) -> Option<&'static str> {
                match self {
                    $(
                        Kind::$ident => kind_value!($($value)?),
                    )*
                }
            }

            pub const fn is_node(&self) -> bool {
                match self {
                    $(
                        Kind::$ident => const {
                            let name = stringify!($ident).as_bytes();
                            name[0] == b'N' && name[1] == b'o' && name[2] == b'd' && name[3] == b'e'
                        },
                    )*
                }
            }
        }
    };
}

impl_kind!(
    pub enum Kind {
        At = "@",
        Equal = "=",
        EqualEqual = "==",
        Less = "<",
        LessEqual = "<=",
        LessLess = "<<",
        Greater = ">",
        GreaterEqual = ">=",
        GreaterGreater = ">>",
        Plus = "+",
        PlusEqual = "+=",
        Minus = "-",
        MinusEqual = "-=",
        Arrow = "->",
        Star = "*",
        StarEqual = "*=",
        Slash = "/",
        SlashEqual = "/=",
        Backslash = "\\",
        Percent = "%",
        PercentEqual = "%=",
        Bang = "!",
        BangEqual = "!=",
        Ampersand = "&",
        AmpersandAmpersand = "&&",
        AmpersandEqual = "&=",
        Backtick = "`",
        SingleQuote = "'",
        Pipe = "|",
        PipePipe = "||",
        PipeEqual = "|=",
        PipeGreater = "|>",
        Caret = "^",
        CaretEqual = "^=",
        Tilde = "~",
        Dot = ".",
        DotDot = "..",
        DotEqual = ".=",
        Dollar = "$",
        Question = "?",
        Comma = ",",
        Colon = ":",
        ColonColon = "::",
        Semicolon = ";",
        LParen = "(",
        RParen = ")",
        LBrace = "{",
        RBrace = "}",
        LBracket = "[",
        RBracket = "]",
        Identifier,
        Integer,
        Float,
        StringStart = "\"",
        StringContent,
        StringContentWithEscape,
        StringEnd = "\"",
        CommentStart = "#",
        CommentContent,
        DocCommentStart = "##",
        DocCommentContent,
        Space = " ",
        Tab = "\t",
        Newline = "\n",
        // Nodes
        NodeRoot,
        NodeApply,
        NodeApplyArgument,
        NodeApplyReceiver,
        NodeAttr,
        NodeLiteral,
        NodeLiteralValue,
        NodeError,
        // Must come last
        Eof,
    }
);

impl Kind {
    pub const WS: &[Self] = &[Self::Space, Self::Tab, Self::Newline];
    pub const TRIVIA: &[Self] = &[
        Self::Space,
        Self::Tab,
        Self::Newline,
        Self::CommentStart,
        Self::CommentContent,
    ];
    pub const NODES: &[Self] = &[
        Self::NodeRoot,
        Self::NodeApply,
        Self::NodeApplyArgument,
        Self::NodeApplyReceiver,
        Self::NodeAttr,
        Self::NodeLiteral,
        Self::NodeLiteralValue,
        Self::NodeError,
    ];

    pub const fn is_ws(&self) -> bool {
        matches!(self, Self::Space | Self::Tab | Self::Newline)
    }

    pub const fn is_trivia(&self) -> bool {
        matches!(
            self,
            Self::Space | Self::Tab | Self::Newline | Self::CommentStart | Self::CommentContent
        )
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nodes_list() {
        let expected = Kind::ALL
            .iter()
            .copied()
            .filter(|kind| format!("{kind:?}").starts_with("Node"))
            .collect::<Vec<_>>();
        assert_eq!(Kind::NODES, expected);
    }
}
