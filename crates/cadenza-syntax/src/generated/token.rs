use serde::{Serialize, Deserialize};
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u16)]
pub enum Kind {
    /// "@"
    At,
    /// "="
    Equal,
    /// "=="
    EqualEqual,
    /// "<"
    Less,
    /// "<="
    LessEqual,
    /// "<<"
    LessLess,
    /// ">"
    Greater,
    /// ">="
    GreaterEqual,
    /// ">>"
    GreaterGreater,
    /// "+"
    Plus,
    /// "+="
    PlusEqual,
    /// "-"
    Minus,
    /// "-="
    MinusEqual,
    /// "->"
    Arrow,
    /// "*"
    Star,
    /// "*="
    StarEqual,
    /// "/"
    Slash,
    /// "/="
    SlashEqual,
    /// "\\"
    Backslash,
    /// "%"
    Percent,
    /// "%="
    PercentEqual,
    /// "!"
    Bang,
    /// "!="
    BangEqual,
    /// "&"
    Ampersand,
    /// "&&"
    AmpersandAmpersand,
    /// "&="
    AmpersandEqual,
    /// "`"
    Backtick,
    /// "'"
    SingleQuote,
    /// "|"
    Pipe,
    /// "||"
    PipePipe,
    /// "|="
    PipeEqual,
    /// "|>"
    PipeGreater,
    /// "^"
    Caret,
    /// "^="
    CaretEqual,
    /// "~"
    Tilde,
    /// "."
    Dot,
    /// ".."
    DotDot,
    /// ".="
    DotEqual,
    /// "$"
    Dollar,
    /// "?"
    Question,
    /// ","
    Comma,
    /// ":"
    Colon,
    /// "::"
    ColonColon,
    /// ";"
    Semicolon,
    /// "("
    LParen,
    /// ")"
    RParen,
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
    Eof,
}
impl Kind {
    pub const ALL: &'static [Self] = &[
        Self::At,
        Self::Equal,
        Self::EqualEqual,
        Self::Less,
        Self::LessEqual,
        Self::LessLess,
        Self::Greater,
        Self::GreaterEqual,
        Self::GreaterGreater,
        Self::Plus,
        Self::PlusEqual,
        Self::Minus,
        Self::MinusEqual,
        Self::Arrow,
        Self::Star,
        Self::StarEqual,
        Self::Slash,
        Self::SlashEqual,
        Self::Backslash,
        Self::Percent,
        Self::PercentEqual,
        Self::Bang,
        Self::BangEqual,
        Self::Ampersand,
        Self::AmpersandAmpersand,
        Self::AmpersandEqual,
        Self::Backtick,
        Self::SingleQuote,
        Self::Pipe,
        Self::PipePipe,
        Self::PipeEqual,
        Self::PipeGreater,
        Self::Caret,
        Self::CaretEqual,
        Self::Tilde,
        Self::Dot,
        Self::DotDot,
        Self::DotEqual,
        Self::Dollar,
        Self::Question,
        Self::Comma,
        Self::Colon,
        Self::ColonColon,
        Self::Semicolon,
        Self::LParen,
        Self::RParen,
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
        Self::Eof,
    ];

    pub const WHITESPACE: &'static [Self] = &[
        Self::Space, 
        Self::Tab, 
        Self::Newline, 
    ];

    pub const fn is_whitespace(self) -> bool {
        match self {
            Self::Space => true,
            Self::Tab => true,
            Self::Newline => true,
            _ => false, 
        }
    }

    pub const TRIVIA: &'static [Self] = &[
        Self::CommentStart, 
        Self::Space, 
        Self::Tab, 
        Self::Newline, 
        Self::CommentContent, 
    ];

    pub const fn is_trivia(self) -> bool {
        match self {
            Self::CommentStart => true,
            Self::Space => true,
            Self::Tab => true,
            Self::Newline => true,
            Self::CommentContent => true,
            _ => false, 
        }
    }

    pub const NODE: &'static [Self] = &[
        Self::At, 
        Self::Equal, 
        Self::EqualEqual, 
        Self::Less, 
        Self::LessEqual, 
        Self::LessLess, 
        Self::Greater, 
        Self::GreaterEqual, 
        Self::GreaterGreater, 
        Self::Plus, 
        Self::PlusEqual, 
        Self::Minus, 
        Self::MinusEqual, 
        Self::Arrow, 
        Self::Star, 
        Self::StarEqual, 
        Self::Slash, 
        Self::SlashEqual, 
        Self::Backslash, 
        Self::Percent, 
        Self::PercentEqual, 
        Self::Bang, 
        Self::BangEqual, 
        Self::Ampersand, 
        Self::AmpersandAmpersand, 
        Self::AmpersandEqual, 
        Self::Backtick, 
        Self::SingleQuote, 
        Self::Pipe, 
        Self::PipePipe, 
        Self::PipeEqual, 
        Self::PipeGreater, 
        Self::Caret, 
        Self::CaretEqual, 
        Self::Tilde, 
        Self::Dot, 
        Self::DotDot, 
        Self::DotEqual, 
        Self::Dollar, 
        Self::Question, 
        Self::Comma, 
        Self::Colon, 
        Self::ColonColon, 
        Self::Semicolon, 
        Self::LBrace, 
        Self::LBracket, 
        Self::DocCommentStart, 
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
    ];

    pub const fn is_node(self) -> bool {
        match self {
            Self::At => true,
            Self::Equal => true,
            Self::EqualEqual => true,
            Self::Less => true,
            Self::LessEqual => true,
            Self::LessLess => true,
            Self::Greater => true,
            Self::GreaterEqual => true,
            Self::GreaterGreater => true,
            Self::Plus => true,
            Self::PlusEqual => true,
            Self::Minus => true,
            Self::MinusEqual => true,
            Self::Arrow => true,
            Self::Star => true,
            Self::StarEqual => true,
            Self::Slash => true,
            Self::SlashEqual => true,
            Self::Backslash => true,
            Self::Percent => true,
            Self::PercentEqual => true,
            Self::Bang => true,
            Self::BangEqual => true,
            Self::Ampersand => true,
            Self::AmpersandAmpersand => true,
            Self::AmpersandEqual => true,
            Self::Backtick => true,
            Self::SingleQuote => true,
            Self::Pipe => true,
            Self::PipePipe => true,
            Self::PipeEqual => true,
            Self::PipeGreater => true,
            Self::Caret => true,
            Self::CaretEqual => true,
            Self::Tilde => true,
            Self::Dot => true,
            Self::DotDot => true,
            Self::DotEqual => true,
            Self::Dollar => true,
            Self::Question => true,
            Self::Comma => true,
            Self::Colon => true,
            Self::ColonColon => true,
            Self::Semicolon => true,
            Self::LBrace => true,
            Self::LBracket => true,
            Self::DocCommentStart => true,
            Self::StringContent => true,
            Self::StringContentWithEscape => true,
            Self::DocCommentContent => true,
            Self::Integer => true,
            Self::Float => true,
            Self::Identifier => true,
            Self::Root => true,
            Self::Apply => true,
            Self::ApplyArgument => true,
            Self::ApplyReceiver => true,
            Self::Attr => true,
            Self::Literal => true,
            Self::Error => true,
            _ => false, 
        }
    }

    pub const fn as_str(self) -> Option<&'static str> {
        match self {
            Self::At => Some("@"),
            Self::Equal => Some("="),
            Self::EqualEqual => Some("=="),
            Self::Less => Some("<"),
            Self::LessEqual => Some("<="),
            Self::LessLess => Some("<<"),
            Self::Greater => Some(">"),
            Self::GreaterEqual => Some(">="),
            Self::GreaterGreater => Some(">>"),
            Self::Plus => Some("+"),
            Self::PlusEqual => Some("+="),
            Self::Minus => Some("-"),
            Self::MinusEqual => Some("-="),
            Self::Arrow => Some("->"),
            Self::Star => Some("*"),
            Self::StarEqual => Some("*="),
            Self::Slash => Some("/"),
            Self::SlashEqual => Some("/="),
            Self::Backslash => Some("\\"),
            Self::Percent => Some("%"),
            Self::PercentEqual => Some("%="),
            Self::Bang => Some("!"),
            Self::BangEqual => Some("!="),
            Self::Ampersand => Some("&"),
            Self::AmpersandAmpersand => Some("&&"),
            Self::AmpersandEqual => Some("&="),
            Self::Backtick => Some("`"),
            Self::SingleQuote => Some("'"),
            Self::Pipe => Some("|"),
            Self::PipePipe => Some("||"),
            Self::PipeEqual => Some("|="),
            Self::PipeGreater => Some("|>"),
            Self::Caret => Some("^"),
            Self::CaretEqual => Some("^="),
            Self::Tilde => Some("~"),
            Self::Dot => Some("."),
            Self::DotDot => Some(".."),
            Self::DotEqual => Some(".="),
            Self::Dollar => Some("$"),
            Self::Question => Some("?"),
            Self::Comma => Some(","),
            Self::Colon => Some(":"),
            Self::ColonColon => Some("::"),
            Self::Semicolon => Some(";"),
            Self::LParen => Some("("),
            Self::RParen => Some(")"),
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

    pub const fn as_op(self) -> Option<Op> {
        match self {
            Self::At => Some(Op::At), 
            Self::Equal => Some(Op::Equal), 
            Self::EqualEqual => Some(Op::EqualEqual), 
            Self::Less => Some(Op::Less), 
            Self::LessEqual => Some(Op::LessEqual), 
            Self::LessLess => Some(Op::LessLess), 
            Self::Greater => Some(Op::Greater), 
            Self::GreaterEqual => Some(Op::GreaterEqual), 
            Self::GreaterGreater => Some(Op::GreaterGreater), 
            Self::Plus => Some(Op::Plus), 
            Self::PlusEqual => Some(Op::PlusEqual), 
            Self::Minus => Some(Op::Minus), 
            Self::MinusEqual => Some(Op::MinusEqual), 
            Self::Arrow => Some(Op::Arrow), 
            Self::Star => Some(Op::Star), 
            Self::StarEqual => Some(Op::StarEqual), 
            Self::Slash => Some(Op::Slash), 
            Self::SlashEqual => Some(Op::SlashEqual), 
            Self::Backslash => Some(Op::Backslash), 
            Self::Percent => Some(Op::Percent), 
            Self::PercentEqual => Some(Op::PercentEqual), 
            Self::Bang => Some(Op::Bang), 
            Self::BangEqual => Some(Op::BangEqual), 
            Self::Ampersand => Some(Op::Ampersand), 
            Self::AmpersandAmpersand => Some(Op::AmpersandAmpersand), 
            Self::AmpersandEqual => Some(Op::AmpersandEqual), 
            Self::Backtick => Some(Op::Backtick), 
            Self::SingleQuote => Some(Op::SingleQuote), 
            Self::Pipe => Some(Op::Pipe), 
            Self::PipePipe => Some(Op::PipePipe), 
            Self::PipeEqual => Some(Op::PipeEqual), 
            Self::PipeGreater => Some(Op::PipeGreater), 
            Self::Caret => Some(Op::Caret), 
            Self::CaretEqual => Some(Op::CaretEqual), 
            Self::Tilde => Some(Op::Tilde), 
            Self::Dot => Some(Op::Dot), 
            Self::DotDot => Some(Op::DotDot), 
            Self::DotEqual => Some(Op::DotEqual), 
            Self::Dollar => Some(Op::Dollar), 
            Self::Question => Some(Op::Question), 
            Self::Comma => Some(Op::Comma), 
            Self::Colon => Some(Op::Colon), 
            Self::ColonColon => Some(Op::ColonColon), 
            Self::Semicolon => Some(Op::Semicolon), 
            Self::LBrace => Some(Op::LBrace), 
            Self::LBracket => Some(Op::LBracket), 
            Self::DocCommentStart => Some(Op::DocCommentStart), 
            _ => None, 
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u16)]
pub enum Op {
    /// "@"
    At, 
    /// "="
    Equal, 
    /// "=="
    EqualEqual, 
    /// "<"
    Less, 
    /// "<="
    LessEqual, 
    /// "<<"
    LessLess, 
    /// ">"
    Greater, 
    /// ">="
    GreaterEqual, 
    /// ">>"
    GreaterGreater, 
    /// "+"
    Plus, 
    /// "+="
    PlusEqual, 
    /// "-"
    Minus, 
    /// "-="
    MinusEqual, 
    /// "->"
    Arrow, 
    /// "*"
    Star, 
    /// "*="
    StarEqual, 
    /// "/"
    Slash, 
    /// "/="
    SlashEqual, 
    /// "\\"
    Backslash, 
    /// "%"
    Percent, 
    /// "%="
    PercentEqual, 
    /// "!"
    Bang, 
    /// "!="
    BangEqual, 
    /// "&"
    Ampersand, 
    /// "&&"
    AmpersandAmpersand, 
    /// "&="
    AmpersandEqual, 
    /// "`"
    Backtick, 
    /// "'"
    SingleQuote, 
    /// "|"
    Pipe, 
    /// "||"
    PipePipe, 
    /// "|="
    PipeEqual, 
    /// "|>"
    PipeGreater, 
    /// "^"
    Caret, 
    /// "^="
    CaretEqual, 
    /// "~"
    Tilde, 
    /// "."
    Dot, 
    /// ".."
    DotDot, 
    /// ".="
    DotEqual, 
    /// "$"
    Dollar, 
    /// "?"
    Question, 
    /// ","
    Comma, 
    /// ":"
    Colon, 
    /// "::"
    ColonColon, 
    /// ";"
    Semicolon, 
    /// "{"
    LBrace, 
    /// "["
    LBracket, 
    /// "##"
    DocCommentStart, 
}
