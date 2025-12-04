use serde::{Deserialize, Serialize};
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u16)]
pub enum Kind {
    /// "@"
    At,
    /// "!"
    Bang,
    /// "~"
    Tilde,
    /// "$"
    Dollar,
    /// "?"
    Question,
    /// "|?"
    PipeQuestion,
    /// "|>"
    PipeGreater,
    /// ".."
    DotDot,
    /// "..="
    DotDotEqual,
    /// "="
    Equal,
    /// "->"
    RightArrow,
    /// "<-"
    LeftArrow,
    /// "+="
    PlusEqual,
    /// "-="
    MinusEqual,
    /// "*="
    StarEqual,
    /// "/="
    SlashEqual,
    /// "%="
    PercentEqual,
    /// "&="
    AmpersandEqual,
    /// "|="
    PipeEqual,
    /// "^="
    CaretEqual,
    /// "<<="
    LessLessEqual,
    /// ">>="
    GreaterGreaterEqual,
    /// "||"
    PipePipe,
    /// "&&"
    AmpersandAmpersand,
    /// "=="
    EqualEqual,
    /// "!="
    BangEqual,
    /// "<"
    Less,
    /// "<="
    LessEqual,
    /// ">"
    Greater,
    /// ">="
    GreaterEqual,
    /// "|"
    Pipe,
    /// "^"
    Caret,
    /// "&"
    Ampersand,
    /// "<<"
    LessLess,
    /// ">>"
    GreaterGreater,
    /// "+"
    Plus,
    /// "-"
    Minus,
    /// "*"
    Star,
    /// "/"
    Slash,
    /// "%"
    Percent,
    /// "**"
    StarStar,
    /// "."
    Dot,
    /// "::"
    ColonColon,
    /// "\\"
    Backslash,
    /// "`"
    Backtick,
    /// "'"
    SingleQuote,
    /// ","
    Comma,
    /// ":"
    Colon,
    /// ";"
    Semicolon,
    /// "("
    LParen,
    /// ")"
    RParen,
    /// "${"
    LDollarBrace,
    /// "{"
    LBrace,
    /// "}"
    RBrace,
    /// "["
    LBracket,
    /// "]"
    RBracket,
    /// "\""
    StringStart,
    StringEnd,
    /// "#"
    CommentStart,
    /// "##"
    DocCommentStart,
    /// " "
    Space,
    /// "\t"
    Tab,
    /// "\n"
    Newline,
    StringContent,
    StringContentWithEscape,
    CommentContent,
    DocCommentContent,
    Integer,
    Float,
    Identifier,
    Root,
    Apply,
    ApplyArgument,
    ApplyReceiver,
    Attr,
    Literal,
    Error,
    SyntheticList,
    SyntheticRecord,
    SyntheticBlock,
    Eof,
}
impl Kind {
    pub const ALL: &'static [Self] = &[
        Self::At,
        Self::Bang,
        Self::Tilde,
        Self::Dollar,
        Self::Question,
        Self::PipeQuestion,
        Self::PipeGreater,
        Self::DotDot,
        Self::DotDotEqual,
        Self::Equal,
        Self::RightArrow,
        Self::LeftArrow,
        Self::PlusEqual,
        Self::MinusEqual,
        Self::StarEqual,
        Self::SlashEqual,
        Self::PercentEqual,
        Self::AmpersandEqual,
        Self::PipeEqual,
        Self::CaretEqual,
        Self::LessLessEqual,
        Self::GreaterGreaterEqual,
        Self::PipePipe,
        Self::AmpersandAmpersand,
        Self::EqualEqual,
        Self::BangEqual,
        Self::Less,
        Self::LessEqual,
        Self::Greater,
        Self::GreaterEqual,
        Self::Pipe,
        Self::Caret,
        Self::Ampersand,
        Self::LessLess,
        Self::GreaterGreater,
        Self::Plus,
        Self::Minus,
        Self::Star,
        Self::Slash,
        Self::Percent,
        Self::StarStar,
        Self::Dot,
        Self::ColonColon,
        Self::Backslash,
        Self::Backtick,
        Self::SingleQuote,
        Self::Comma,
        Self::Colon,
        Self::Semicolon,
        Self::LParen,
        Self::RParen,
        Self::LDollarBrace,
        Self::LBrace,
        Self::RBrace,
        Self::LBracket,
        Self::RBracket,
        Self::StringStart,
        Self::StringEnd,
        Self::CommentStart,
        Self::DocCommentStart,
        Self::Space,
        Self::Tab,
        Self::Newline,
        Self::StringContent,
        Self::StringContentWithEscape,
        Self::CommentContent,
        Self::DocCommentContent,
        Self::Integer,
        Self::Float,
        Self::Identifier,
        Self::Root,
        Self::Apply,
        Self::ApplyArgument,
        Self::ApplyReceiver,
        Self::Attr,
        Self::Literal,
        Self::Error,
        Self::SyntheticList,
        Self::SyntheticRecord,
        Self::SyntheticBlock,
        Self::Eof,
    ];

    pub const WHITESPACE: &'static [Self] = &[Self::Space, Self::Tab, Self::Newline];

    pub const fn is_whitespace(self) -> bool {
        matches!(self, Self::Space | Self::Tab | Self::Newline)
    }

    pub const TRIVIA: &'static [Self] = &[
        Self::CommentStart,
        Self::Space,
        Self::Tab,
        Self::Newline,
        Self::CommentContent,
    ];

    pub const fn is_trivia(self) -> bool {
        matches!(
            self,
            Self::CommentStart | Self::Space | Self::Tab | Self::Newline | Self::CommentContent
        )
    }

    pub const NODE: &'static [Self] = &[
        Self::At,
        Self::Bang,
        Self::Tilde,
        Self::Dollar,
        Self::Question,
        Self::PipeQuestion,
        Self::PipeGreater,
        Self::DotDot,
        Self::DotDotEqual,
        Self::Equal,
        Self::RightArrow,
        Self::LeftArrow,
        Self::PlusEqual,
        Self::MinusEqual,
        Self::StarEqual,
        Self::SlashEqual,
        Self::PercentEqual,
        Self::AmpersandEqual,
        Self::PipeEqual,
        Self::CaretEqual,
        Self::LessLessEqual,
        Self::GreaterGreaterEqual,
        Self::PipePipe,
        Self::AmpersandAmpersand,
        Self::EqualEqual,
        Self::BangEqual,
        Self::Less,
        Self::LessEqual,
        Self::Greater,
        Self::GreaterEqual,
        Self::Pipe,
        Self::Caret,
        Self::Ampersand,
        Self::LessLess,
        Self::GreaterGreater,
        Self::Plus,
        Self::Minus,
        Self::Star,
        Self::Slash,
        Self::Percent,
        Self::StarStar,
        Self::Dot,
        Self::ColonColon,
        Self::StringContent,
        Self::StringContentWithEscape,
        Self::DocCommentContent,
        Self::Integer,
        Self::Float,
        Self::Identifier,
        Self::Root,
        Self::Apply,
        Self::ApplyArgument,
        Self::ApplyReceiver,
        Self::Attr,
        Self::Literal,
        Self::Error,
        Self::SyntheticList,
        Self::SyntheticRecord,
        Self::SyntheticBlock,
    ];

    pub const fn is_node(self) -> bool {
        matches!(
            self,
            Self::At
                | Self::Bang
                | Self::Tilde
                | Self::Dollar
                | Self::Question
                | Self::PipeQuestion
                | Self::PipeGreater
                | Self::DotDot
                | Self::DotDotEqual
                | Self::Equal
                | Self::RightArrow
                | Self::LeftArrow
                | Self::PlusEqual
                | Self::MinusEqual
                | Self::StarEqual
                | Self::SlashEqual
                | Self::PercentEqual
                | Self::AmpersandEqual
                | Self::PipeEqual
                | Self::CaretEqual
                | Self::LessLessEqual
                | Self::GreaterGreaterEqual
                | Self::PipePipe
                | Self::AmpersandAmpersand
                | Self::EqualEqual
                | Self::BangEqual
                | Self::Less
                | Self::LessEqual
                | Self::Greater
                | Self::GreaterEqual
                | Self::Pipe
                | Self::Caret
                | Self::Ampersand
                | Self::LessLess
                | Self::GreaterGreater
                | Self::Plus
                | Self::Minus
                | Self::Star
                | Self::Slash
                | Self::Percent
                | Self::StarStar
                | Self::Dot
                | Self::ColonColon
                | Self::StringContent
                | Self::StringContentWithEscape
                | Self::DocCommentContent
                | Self::Integer
                | Self::Float
                | Self::Identifier
                | Self::Root
                | Self::Apply
                | Self::ApplyArgument
                | Self::ApplyReceiver
                | Self::Attr
                | Self::Literal
                | Self::Error
                | Self::SyntheticList
                | Self::SyntheticRecord
                | Self::SyntheticBlock
        )
    }

    pub const fn as_str(self) -> Option<&'static str> {
        match self {
            Self::At => Some("@"),
            Self::Bang => Some("!"),
            Self::Tilde => Some("~"),
            Self::Dollar => Some("$"),
            Self::Question => Some("?"),
            Self::PipeQuestion => Some("|?"),
            Self::PipeGreater => Some("|>"),
            Self::DotDot => Some(".."),
            Self::DotDotEqual => Some("..="),
            Self::Equal => Some("="),
            Self::RightArrow => Some("->"),
            Self::LeftArrow => Some("<-"),
            Self::PlusEqual => Some("+="),
            Self::MinusEqual => Some("-="),
            Self::StarEqual => Some("*="),
            Self::SlashEqual => Some("/="),
            Self::PercentEqual => Some("%="),
            Self::AmpersandEqual => Some("&="),
            Self::PipeEqual => Some("|="),
            Self::CaretEqual => Some("^="),
            Self::LessLessEqual => Some("<<="),
            Self::GreaterGreaterEqual => Some(">>="),
            Self::PipePipe => Some("||"),
            Self::AmpersandAmpersand => Some("&&"),
            Self::EqualEqual => Some("=="),
            Self::BangEqual => Some("!="),
            Self::Less => Some("<"),
            Self::LessEqual => Some("<="),
            Self::Greater => Some(">"),
            Self::GreaterEqual => Some(">="),
            Self::Pipe => Some("|"),
            Self::Caret => Some("^"),
            Self::Ampersand => Some("&"),
            Self::LessLess => Some("<<"),
            Self::GreaterGreater => Some(">>"),
            Self::Plus => Some("+"),
            Self::Minus => Some("-"),
            Self::Star => Some("*"),
            Self::Slash => Some("/"),
            Self::Percent => Some("%"),
            Self::StarStar => Some("**"),
            Self::Dot => Some("."),
            Self::ColonColon => Some("::"),
            Self::Backslash => Some("\\"),
            Self::Backtick => Some("`"),
            Self::SingleQuote => Some("'"),
            Self::Comma => Some(","),
            Self::Colon => Some(":"),
            Self::Semicolon => Some(";"),
            Self::LParen => Some("("),
            Self::RParen => Some(")"),
            Self::LDollarBrace => Some("${"),
            Self::LBrace => Some("{"),
            Self::RBrace => Some("}"),
            Self::LBracket => Some("["),
            Self::RBracket => Some("]"),
            Self::StringStart => Some("\""),
            Self::CommentStart => Some("#"),
            Self::DocCommentStart => Some("##"),
            Self::Space => Some(" "),
            Self::Tab => Some("\t"),
            Self::Newline => Some("\n"),
            _ => None,
        }
    }

    /// Try to convert a u16 discriminant to a Kind
    pub const fn try_from_u16(value: u16) -> Option<Self> {
        if value < 81 {
            // SAFETY: value is within valid discriminant range
            Some(unsafe { core::mem::transmute::<u16, Kind>(value) })
        } else {
            None
        }
    }

    /// Returns a human-readable name for this token kind
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::At => "@",
            Self::Bang => "!",
            Self::Tilde => "~",
            Self::Dollar => "$",
            Self::Question => "?",
            Self::PipeQuestion => "|?",
            Self::PipeGreater => "|>",
            Self::DotDot => "..",
            Self::DotDotEqual => "..=",
            Self::Equal => "=",
            Self::RightArrow => "->",
            Self::LeftArrow => "<-",
            Self::PlusEqual => "+=",
            Self::MinusEqual => "-=",
            Self::StarEqual => "*=",
            Self::SlashEqual => "/=",
            Self::PercentEqual => "%=",
            Self::AmpersandEqual => "&=",
            Self::PipeEqual => "|=",
            Self::CaretEqual => "^=",
            Self::LessLessEqual => "<<=",
            Self::GreaterGreaterEqual => ">>=",
            Self::PipePipe => "||",
            Self::AmpersandAmpersand => "&&",
            Self::EqualEqual => "==",
            Self::BangEqual => "!=",
            Self::Less => "<",
            Self::LessEqual => "<=",
            Self::Greater => ">",
            Self::GreaterEqual => ">=",
            Self::Pipe => "|",
            Self::Caret => "^",
            Self::Ampersand => "&",
            Self::LessLess => "<<",
            Self::GreaterGreater => ">>",
            Self::Plus => "+",
            Self::Minus => "-",
            Self::Star => "*",
            Self::Slash => "/",
            Self::Percent => "%",
            Self::StarStar => "**",
            Self::Dot => ".",
            Self::ColonColon => "::",
            Self::Backslash => "\\",
            Self::Backtick => "`",
            Self::SingleQuote => "'",
            Self::Comma => ",",
            Self::Colon => ":",
            Self::Semicolon => ";",
            Self::LParen => "(",
            Self::RParen => ")",
            Self::LDollarBrace => "${",
            Self::LBrace => "{",
            Self::RBrace => "}",
            Self::LBracket => "[",
            Self::RBracket => "]",
            Self::StringStart => "\"",
            Self::StringEnd => "string end",
            Self::CommentStart => "#",
            Self::DocCommentStart => "##",
            Self::Space => " ",
            Self::Tab => "\t",
            Self::Newline => "\n",
            Self::StringContent => "string content",
            Self::StringContentWithEscape => "string content with escape",
            Self::CommentContent => "comment content",
            Self::DocCommentContent => "doc comment content",
            Self::Integer => "integer",
            Self::Float => "float",
            Self::Identifier => "identifier",
            Self::Root => "root",
            Self::Apply => "apply",
            Self::ApplyArgument => "apply argument",
            Self::ApplyReceiver => "apply receiver",
            Self::Attr => "attr",
            Self::Literal => "literal",
            Self::Error => "error",
            Self::SyntheticList => "synthetic list",
            Self::SyntheticRecord => "synthetic record",
            Self::SyntheticBlock => "synthetic block",
            Self::Eof => "eof",
        }
    }

    pub const fn as_op(self) -> Option<Op> {
        match self {
            Self::At => Some(Op::At),
            Self::Bang => Some(Op::Bang),
            Self::Tilde => Some(Op::Tilde),
            Self::Dollar => Some(Op::Dollar),
            Self::Question => Some(Op::Question),
            Self::PipeQuestion => Some(Op::PipeQuestion),
            Self::PipeGreater => Some(Op::PipeGreater),
            Self::DotDot => Some(Op::DotDot),
            Self::DotDotEqual => Some(Op::DotDotEqual),
            Self::Equal => Some(Op::Equal),
            Self::RightArrow => Some(Op::RightArrow),
            Self::LeftArrow => Some(Op::LeftArrow),
            Self::PlusEqual => Some(Op::PlusEqual),
            Self::MinusEqual => Some(Op::MinusEqual),
            Self::StarEqual => Some(Op::StarEqual),
            Self::SlashEqual => Some(Op::SlashEqual),
            Self::PercentEqual => Some(Op::PercentEqual),
            Self::AmpersandEqual => Some(Op::AmpersandEqual),
            Self::PipeEqual => Some(Op::PipeEqual),
            Self::CaretEqual => Some(Op::CaretEqual),
            Self::LessLessEqual => Some(Op::LessLessEqual),
            Self::GreaterGreaterEqual => Some(Op::GreaterGreaterEqual),
            Self::PipePipe => Some(Op::PipePipe),
            Self::AmpersandAmpersand => Some(Op::AmpersandAmpersand),
            Self::EqualEqual => Some(Op::EqualEqual),
            Self::BangEqual => Some(Op::BangEqual),
            Self::Less => Some(Op::Less),
            Self::LessEqual => Some(Op::LessEqual),
            Self::Greater => Some(Op::Greater),
            Self::GreaterEqual => Some(Op::GreaterEqual),
            Self::Pipe => Some(Op::Pipe),
            Self::Caret => Some(Op::Caret),
            Self::Ampersand => Some(Op::Ampersand),
            Self::LessLess => Some(Op::LessLess),
            Self::GreaterGreater => Some(Op::GreaterGreater),
            Self::Plus => Some(Op::Plus),
            Self::Minus => Some(Op::Minus),
            Self::Star => Some(Op::Star),
            Self::Slash => Some(Op::Slash),
            Self::Percent => Some(Op::Percent),
            Self::StarStar => Some(Op::StarStar),
            Self::Dot => Some(Op::Dot),
            Self::ColonColon => Some(Op::ColonColon),
            _ => None,
        }
    }

    pub const fn prefix_binding_power(self) -> Option<u8> {
        match self {
            Self::At => Some(0),
            Self::Bang => Some(26),
            Self::Tilde => Some(26),
            Self::Dollar => Some(26),
            _ => None,
        }
    }

    pub const fn infix_binding_power(self) -> Option<(u8, u8)> {
        match self {
            Self::PipeGreater => Some((0, 1)),
            Self::DotDot => Some((2, 3)),
            Self::DotDotEqual => Some((2, 3)),
            Self::Equal => Some((5, 4)),
            Self::RightArrow => Some((5, 4)),
            Self::LeftArrow => Some((5, 4)),
            Self::PlusEqual => Some((5, 4)),
            Self::MinusEqual => Some((5, 4)),
            Self::StarEqual => Some((5, 4)),
            Self::SlashEqual => Some((5, 4)),
            Self::PercentEqual => Some((5, 4)),
            Self::AmpersandEqual => Some((5, 4)),
            Self::PipeEqual => Some((5, 4)),
            Self::CaretEqual => Some((5, 4)),
            Self::LessLessEqual => Some((5, 4)),
            Self::GreaterGreaterEqual => Some((5, 4)),
            Self::PipePipe => Some((8, 9)),
            Self::AmpersandAmpersand => Some((10, 11)),
            Self::EqualEqual => Some((12, 13)),
            Self::BangEqual => Some((12, 13)),
            Self::Less => Some((14, 15)),
            Self::LessEqual => Some((14, 15)),
            Self::Greater => Some((14, 15)),
            Self::GreaterEqual => Some((14, 15)),
            Self::Pipe => Some((16, 17)),
            Self::Caret => Some((18, 19)),
            Self::Ampersand => Some((20, 21)),
            Self::LessLess => Some((22, 23)),
            Self::GreaterGreater => Some((22, 23)),
            Self::Plus => Some((24, 25)),
            Self::Minus => Some((24, 25)),
            Self::Star => Some((26, 27)),
            Self::Slash => Some((26, 27)),
            Self::Percent => Some((26, 27)),
            Self::StarStar => Some((29, 28)),
            Self::Dot => Some((30, 31)),
            Self::ColonColon => Some((32, 33)),
            _ => None,
        }
    }

    pub const fn postfix_binding_power(self) -> Option<u8> {
        match self {
            Self::Question => Some(29),
            Self::PipeQuestion => Some(0),
            _ => None,
        }
    }

    /// Returns the binding power for juxtaposition (function application)
    /// This is calculated from the Juxtaposition infix binding power group
    pub const fn juxtaposition_binding_power() -> (u8, u8) {
        (6, 7)
    }

    /// Returns true if this token kind has infix binding power
    pub const fn is_infix(self) -> bool {
        self.infix_binding_power().is_some()
    }

    /// Returns true if this token kind has postfix binding power
    pub const fn is_postfix(self) -> bool {
        self.postfix_binding_power().is_some()
    }

    /// Returns true if this token kind has prefix binding power
    pub const fn is_prefix(self) -> bool {
        self.prefix_binding_power().is_some()
    }

    /// Returns the identifier for synthetic nodes
    pub const fn synthetic_identifier(self) -> Option<&'static str> {
        match self {
            Self::SyntheticList => Some("__list__"),
            Self::SyntheticRecord => Some("__record__"),
            Self::SyntheticBlock => Some("__block__"),
            _ => None,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u16)]
pub enum Op {
    /// "@"
    At,
    /// "!"
    Bang,
    /// "~"
    Tilde,
    /// "$"
    Dollar,
    /// "?"
    Question,
    /// "|?"
    PipeQuestion,
    /// "|>"
    PipeGreater,
    /// ".."
    DotDot,
    /// "..="
    DotDotEqual,
    /// "="
    Equal,
    /// "->"
    RightArrow,
    /// "<-"
    LeftArrow,
    /// "+="
    PlusEqual,
    /// "-="
    MinusEqual,
    /// "*="
    StarEqual,
    /// "/="
    SlashEqual,
    /// "%="
    PercentEqual,
    /// "&="
    AmpersandEqual,
    /// "|="
    PipeEqual,
    /// "^="
    CaretEqual,
    /// "<<="
    LessLessEqual,
    /// ">>="
    GreaterGreaterEqual,
    /// "||"
    PipePipe,
    /// "&&"
    AmpersandAmpersand,
    /// "=="
    EqualEqual,
    /// "!="
    BangEqual,
    /// "<"
    Less,
    /// "<="
    LessEqual,
    /// ">"
    Greater,
    /// ">="
    GreaterEqual,
    /// "|"
    Pipe,
    /// "^"
    Caret,
    /// "&"
    Ampersand,
    /// "<<"
    LessLess,
    /// ">>"
    GreaterGreater,
    /// "+"
    Plus,
    /// "-"
    Minus,
    /// "*"
    Star,
    /// "/"
    Slash,
    /// "%"
    Percent,
    /// "**"
    StarStar,
    /// "."
    Dot,
    /// "::"
    ColonColon,
}
