//! Token kinds and the precedence table
//!
//! `Kind` is both the lexer's token kind AND the rowan `SyntaxKind` space (node kinds live above the
//! token kinds, see the `NODE_*` block). `infix_prec` is the single source of truth the parser's
//! Pratt loop and the printer's minimal-paren split both read — sharing it is what guarantees the
//! text round-trip.

/// A lexical token kind and, above `EOF`, the parser's syntax-node kinds. `#[repr(u16)]` so it maps
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
    True,
    False,

    // ---- identifiers ----
    Ident,        // includes kebab-case (`byte-at`) and dotted positional (`tuple.0`)
    BacktickName, // `` `|` ``, `` `->` `` — the lossless escape for symbolic/keyword names

    // ---- keywords ----
    Let,
    In,
    If,
    Then,
    Else,
    Fn,
    Match,

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
