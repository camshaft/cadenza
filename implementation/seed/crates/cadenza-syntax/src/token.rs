//! Token kinds and the precedence table
//!
//! `Kind` is both the lexer's token kind AND the rowan `SyntaxKind` space (node kinds live above the
//! token kinds, see the `Node*` block). `infix_prec` is the single source of truth the parser's
//! Pratt loop and the printer's minimal-paren split both read — sharing it is what guarantees the
//! text round-trip.
//!
//! The lexer is deliberately KEYWORD-FREE: every word lexes to `Ident`, and the PARSER decides
//! whether a given `Ident` is a keyword from its text and grammatical position (contextual
//! keywords). See [`keyword`] and [`is_reserved`]. This keeps the lexer a simple total tokenizer
//! shared by both the ML and s-expression surfaces.

/// A lexical token kind and, above `Eof`, the parser's syntax-node kinds. `#[repr(u16)]` so it maps
/// to rowan's `SyntaxKind(u16)` by transmute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum Kind {
    // ---- trivia (lossless; skipped by the parser grammar) ----
    Whitespace = 0,
    LineComment, // `// …`
    DocComment,  // `/// …`

    // ---- literals ----
    Int,
    Float,
    Str,

    // ---- identifiers (keywords are NOT lexed; the parser recognizes them from Ident text) ----
    Ident,        // words: kebab-case (`byte-at`), `true`/`false`, `let`/`if`/… — all Ident
    BacktickName, // `` `|` ``, `` `->` `` — the lossless escape for symbolic/keyword names

    // ---- operators (each has a binding power in `infix_prec`) ----
    Or,       // `or`
    And,      // `and`
    Eq,       // `=`
    Lt,       // `<`
    Gt,       // `>`
    Le,       // `<=`
    Ge,       // `>=`
    Pipe,     // `|`
    Caret,    // `^`
    Amp,      // `&`
    Shl,      // `<<`
    Shr,      // `>>`
    Plus,     // `+`
    Minus,    // `-`
    PlusPct,  // `+%`
    MinusPct, // `-%`
    Star,     // `*`
    Slash,    // `/`
    Percent,  // `%`
    StarPct,  // `*%`

    // ---- delimiters / punctuation ----
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,         // arg/binding/arm separator, and the `,` unquote prefix
    Dot,           // `.`
    Colon,         // `:`
    FatArrow,      // `=>`
    Arrow,         // `->`
    Hash,          // `#`
    Backtick,      // `` ` `` beginning a `` `{ … } `` quasiquote
    UnquoteSplice, // `,@`

    Error,
    Eof,

    // ---- syntax-node kinds (parser output; never produced by the lexer) ----
    NodeRoot,
    NodeLiteral,
    NodeName,
    NodeParen,
    NodeBinary,
    NodePrefix,
    NodeMember,
    NodeCall,
    NodeArgList,
    NodeLet,
    NodeBinding,
    NodeIf,
    NodeFn,
    NodeParamList,
    NodeMatch,
    NodeMatchArm,
    NodePattern,
    NodeGuard,
    NodeQuasiquote,
    NodeUnquote,
    NodeUnquoteSplice,
    NodeHashList,
    NodeComment,
    NodeDoc,
    NodeError,
}

impl Kind {
    /// True for lexer trivia (whitespace / comments) — carried into the green tree for
    /// losslessness but skipped when reading the grammar. NOTE comments are trivia *tokens* but the
    /// lower step turns them into real `(comment …)`/`(doc …)` nodes, so they still survive.
    pub fn is_trivia(self) -> bool {
        matches!(
            self,
            Kind::Whitespace | Kind::LineComment | Kind::DocComment
        )
    }

    /// The operator symbol a `Kind` denotes, for building the head `Name` of an infix node and for
    /// the printer. `None` for non-operator kinds.
    pub fn op_str(self) -> Option<&'static str> {
        Some(match self {
            Kind::Or => "or",
            Kind::And => "and",
            Kind::Eq => "=",
            Kind::Lt => "<",
            Kind::Gt => ">",
            Kind::Le => "<=",
            Kind::Ge => ">=",
            Kind::Pipe => "|",
            Kind::Caret => "^",
            Kind::Amp => "&",
            Kind::Shl => "<<",
            Kind::Shr => ">>",
            Kind::Plus => "+",
            Kind::Minus => "-",
            Kind::PlusPct => "+%",
            Kind::MinusPct => "-%",
            Kind::Star => "*",
            Kind::Slash => "/",
            Kind::Percent => "%",
            Kind::StarPct => "*%",
            _ => return None,
        })
    }
}

/// A contextual keyword the parser recognizes. The lexer emits `Ident`; the parser calls
/// [`keyword`] on the text to decide. `and`/`or` are NOT keywords — they are word-spelled infix
/// operators (see [`word_op`]). `else` is a keyword only inside `if`/`match`; it is still reserved
/// so it can never be a bare name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyword {
    Let,
    In,
    If,
    Then,
    Else,
    Fn,
    Match,
}

/// The keyword an identifier's text denotes, if any.
pub fn keyword(text: &str) -> Option<Keyword> {
    Some(match text {
        "let" => Keyword::Let,
        "in" => Keyword::In,
        "if" => Keyword::If,
        "then" => Keyword::Then,
        "else" => Keyword::Else,
        "fn" => Keyword::Fn,
        "match" => Keyword::Match,
        _ => return None,
    })
}

/// The infix operator an identifier's text denotes, if it is a word-spelled operator (`and`/`or`).
pub fn word_op(text: &str) -> Option<&'static str> {
    match text {
        "and" => Some("and"),
        "or" => Some("or"),
        _ => None,
    }
}

/// True if a bare name with this text would be misread by the parser — a keyword or a word-operator.
/// The printer backtick-escapes such a name so it round-trips as a name, not the reserved word.
pub fn is_reserved(text: &str) -> bool {
    keyword(text).is_some() || word_op(text).is_some()
}

/// Member access and application bind tightest.
pub const PREC_MEMBER: u8 = 10;

/// The precedence (binding power) of a binary operator NAME, or `None` if it is not infix. This is
/// the single source of truth shared by the parser (Pratt `min_prec`) and printer (minimal-paren
/// `prec`/`prec+1` split). All operators are left-associative.
///
/// Ordering (low→high): `or < and < comparisons < (| ^) < & < (<< >>) < (+ -) < (* / %) < member/app`.
/// Values match the ml-spike table that round-trips the whole corpus.
pub fn infix_prec(op: &str) -> Option<u8> {
    Some(match op {
        "or" => 1,
        "and" => 2,
        "=" | "<" | ">" | "<=" | ">=" => 3,
        "|" | "^" => 4,
        "&" => 5,
        "<<" | ">>" => 6,
        "+" | "-" | "+%" | "-%" => 7,
        "*" | "/" | "%" | "*%" => 8,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_str_and_infix_prec_agree() {
        // Every operator Kind must have a name, and that name must be infix.
        for k in [
            Kind::Or,
            Kind::And,
            Kind::Eq,
            Kind::Lt,
            Kind::Gt,
            Kind::Le,
            Kind::Ge,
            Kind::Pipe,
            Kind::Caret,
            Kind::Amp,
            Kind::Shl,
            Kind::Shr,
            Kind::Plus,
            Kind::Minus,
            Kind::PlusPct,
            Kind::MinusPct,
            Kind::Star,
            Kind::Slash,
            Kind::Percent,
            Kind::StarPct,
        ] {
            let s = k.op_str().expect("operator kind has a name");
            assert!(infix_prec(s).is_some(), "operator {s} has a precedence");
        }
    }

    #[test]
    fn precedence_orders_as_documented() {
        assert!(infix_prec("or") < infix_prec("and"));
        assert!(infix_prec("and") < infix_prec("="));
        assert!(infix_prec("=") < infix_prec("|"));
        assert!(infix_prec("|") < infix_prec("&"));
        assert!(infix_prec("&") < infix_prec("<<"));
        assert!(infix_prec("<<") < infix_prec("+"));
        assert!(infix_prec("+") < infix_prec("*"));
        assert!(infix_prec("*").unwrap() < PREC_MEMBER);
    }
}
