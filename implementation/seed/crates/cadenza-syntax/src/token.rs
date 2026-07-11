//! Token kinds and the precedence table.
//!
//! `Kind` is the lexer's token kind. `infix_prec` is the single source of truth the parser's Pratt
//! loop and the printer's minimal-paren split both read — sharing it is what guarantees the text
//! round-trip.
//!
//! The lexer is deliberately KEYWORD-FREE: every word lexes to `Ident`, and the PARSER decides
//! whether a given `Ident` is a keyword from its text and grammatical position (contextual
//! keywords). See [`keyword`] and [`is_reserved`]. This keeps the lexer a simple total tokenizer
//! shared by both the ML and s-expression surfaces. `and`/`or` are word-spelled infix operators
//! (also `Ident`; see [`word_op`]), not distinct token kinds.

/// A lexical token kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    // ---- trivia (lossless; skipped by the parser grammar) ----
    Whitespace,
    LineComment, // `// …`
    DocComment,  // `/// …`

    // ---- literals ----
    Int,
    Float,
    Str,

    // ---- identifiers (keywords are NOT lexed; the parser recognizes them from Ident text) ----
    Ident, // words: kebab-case (`byte-at`), `true`/`false`, `let`/`if`/…, `and`/`or` — all Ident
    BacktickName, // `` `|` ``, `` `->` `` — the lossless escape for symbolic/keyword names

    // ---- operators (each has a binding power in `infix_prec`) ----
    Eq,       // `=` — the BINDING separator (let/fn/record/map); NOT an infix operator
    EqEq,     // `==` — equality; its arena head is `=` (see `op_str`), spelled `==` on the surface
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
}

impl Kind {
    /// True for lexer trivia (whitespace / comments). Skipped by the grammar; comment tokens still
    /// become real `(comment …)`/`(doc …)` arena nodes at parse time, so they survive.
    pub fn is_trivia(self) -> bool {
        matches!(
            self,
            Kind::Whitespace | Kind::LineComment | Kind::DocComment
        )
    }

    /// The operator NAME a `Kind` denotes — the head `Name` of the infix form it builds, shared by
    /// the parser and printer. `None` for non-operator kinds. This is the ARENA head, which can
    /// differ from the surface glyph: `==` (`EqEq`) and `:` (`Colon`) build heads `=` and `:`, and
    /// the printer maps back to the glyph via [`infix_glyph`]. (`and`/`or` are `Ident`; see
    /// [`word_op`].) A bare `=` (`Kind::Eq`) is the binding separator, NOT an operator, so it has
    /// no op name.
    pub fn op_str(self) -> Option<&'static str> {
        Some(match self {
            Kind::EqEq => "=",  // equality: surface `==`, arena head `=`
            Kind::Colon => ":", // type ascription: `e : T` -> `(: e T)`
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

/// The surface GLYPH the printer emits for an infix operator arena head — the inverse of the head
/// mapping in [`Kind::op_str`]. Identity for every operator except equality, whose arena head `=` is
/// spelled `==` on the ML surface (a bare `=` is the binding separator). Used only by the printer.
pub fn infix_glyph(op: &str) -> &str {
    match op {
        "=" => "==",
        other => other,
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
    Module,
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
        "module" => Keyword::Module,
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

/// Type ascription `e : T` binds loosest of all — looser than every arithmetic/logical operator — so
/// `2 + 2 : Int64` groups as `(: (+ 2 2) Int64)`: the ascription wraps the whole expression.
pub const PREC_ASCRIPTION: u8 = 1;

/// The precedence (binding power) of a binary operator NAME (the ARENA head — `=` is equality here,
/// spelled `==` on the surface; `:` is ascription), or `None` if it is not infix. The single source
/// of truth shared by the parser (Pratt `min_prec`) and printer (minimal-paren `prec`/`prec+1`
/// split). All operators are left-associative.
///
/// Ordering (low→high): `: < or < and < (== < > <= >=) < (| ^) < & < (<< >>) < (+ -) < (* / %) <
/// member/app`. Ascription is loosest; equality (`==`, arena head `=`) sits with the comparisons.
pub fn infix_prec(op: &str) -> Option<u8> {
    Some(match op {
        ":" => PREC_ASCRIPTION, // 1 — loosest
        "or" => 2,
        "and" => 3,
        "=" | "<" | ">" | "<=" | ">=" => 4, // `=` = equality (surface `==`)
        "|" | "^" => 5,
        "&" => 6,
        "<<" | ">>" => 7,
        "+" | "-" | "+%" | "-%" => 8,
        "*" | "/" | "%" | "*%" => 9,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_str_and_infix_prec_agree() {
        // Every operator Kind that names an infix op must map to a precedence. `EqEq` (`==`) and
        // `Colon` (`:`) are included; a bare `Eq` (`=`) is the binding separator, NOT an operator.
        for k in [
            Kind::EqEq,
            Kind::Colon,
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
        // A bare `=` is the binding separator, not an infix operator.
        assert_eq!(Kind::Eq.op_str(), None);
        // The word-spelled operators live in `word_op`, not `Kind`.
        for w in ["and", "or"] {
            assert!(infix_prec(word_op(w).unwrap()).is_some());
        }
    }

    #[test]
    fn precedence_orders_as_documented() {
        // Ascription is loosest of all.
        assert!(infix_prec(":") < infix_prec("or"));
        assert_eq!(infix_prec(":"), Some(PREC_ASCRIPTION));
        assert!(infix_prec("or") < infix_prec("and"));
        assert!(infix_prec("and") < infix_prec("=")); // `=` here = equality (surface `==`)
        assert!(infix_prec("=") < infix_prec("|"));
        assert!(infix_prec("|") < infix_prec("&"));
        assert!(infix_prec("&") < infix_prec("<<"));
        assert!(infix_prec("<<") < infix_prec("+"));
        assert!(infix_prec("+") < infix_prec("*"));
        assert!(infix_prec("*").unwrap() < PREC_MEMBER);
    }

    #[test]
    fn equality_glyph_and_head() {
        // `==` on the surface builds arena head `=`; the printer maps it back to `==`.
        assert_eq!(Kind::EqEq.op_str(), Some("="));
        assert_eq!(infix_glyph("="), "==");
        // every other operator glyph is identity.
        assert_eq!(infix_glyph("+"), "+");
        assert_eq!(infix_glyph(":"), ":");
    }
}
