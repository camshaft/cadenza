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
    Rational, // `3r2` — a native rational literal `<int>r<int>` (seq-204); the parser builds a rational node
    Str,
    ByteStr, // `b"…"` — a byte-string literal (arbitrary bytes), the surface form of a `Bytes` value
    CharLit, // `#\a` / `#\newline` / `#\u+00E9` — a char literal (one Unicode scalar)
    SymLit,  // `#"meter"` — a symbol literal (an interned name value); reuses string lexing
    // `tag"…"` — a TAGGED TEMPLATE: an identifier GLUED to a string (`jsx"…"`, `id"…"`), the surface for
    // a binding-dispatched compile-time macro over literal chunks + `{expr}` holes (tagged-template
    // macros design). Glued like `b"`/`#"`; the tag ident and the string body are one token, split by
    // the parser into `(tagged-template <tag> (chunks …) (holes …))`. (B1: hole-free body; holes follow.)
    TaggedTemplate,

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
    PipeGt,   // `|>` — the pipeline operator: `x |> f(a)` threads `x` as `f`'s FIRST argument
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
    PlusDot,  // `+.` — floating-point addition (the OCaml-style FP operators)
    MinusDot, // `-.` — floating-point subtraction
    StarDot,  // `*.` — floating-point multiplication
    SlashDot, // `/.` — floating-point division

    // ---- delimiters / punctuation ----
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    BinOpen, // `b[` — opens a binary literal `b[<segment>, …]` (desugars to `(bin …)`); glued like `b"`
    Comma,   // arg/binding/arm separator, and the `,` unquote prefix
    Dot,     // `.`
    DotDot, // `..` — the rest/spread marker in a collection literal or pattern (`[x, .. rest]`); also
    // the half-open range operator `lo..hi` (the grammar for that reading is a separate, later slice).
    DotDotEq, // `..=` — the closed range operator `lo..=hi`; a token distinct from `..` so the two range
    // forms stay separable (the parse/desugar is a later slice; this only lexes the token).
    Colon,         // `:`
    Semi,          // `;` — the sequence separator (`a; b; c` -> `(do a b c)`)
    FatArrow,      // `=>`
    Arrow,         // `->`
    Hash,          // `#`
    At, // `@` — the ANNOTATION sigil (`@name form` -> `(@ name form)`); distinct from `,@` below
    AtBang, // `@!` — the PRAGMA sugar (`@!key arg` -> `(pragma key arg)`); the inner-attribute twin of `@`
    Backtick, // `` ` `` beginning a `` `{ … } `` quasiquote
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
            Kind::EqEq => "=",   // equality: surface `==`, arena head `=`
            Kind::Colon => ":",  // type ascription: `e : T` -> `(: e T)`
            Kind::Arrow => "->", // function type: `A -> B` -> `(-> A B)` (right-associative)
            Kind::Lt => "<",
            Kind::Gt => ">",
            Kind::Le => "<=",
            Kind::Ge => ">=",
            Kind::PipeGt => "|>",
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
            Kind::PlusDot => "+.",
            Kind::MinusDot => "-.",
            Kind::StarDot => "*.",
            Kind::SlashDot => "/.",
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
        // Unit COMPOSITION heads render as their mathematical infix glyph — `(Unit.* a b)` → `a * b`,
        // `(Unit./ a b)` → `a / b`, `(Unit.^ u n)` → `u ^ n` — so a unit reads like the math it denotes
        // (`meter / second`, `meter ^ 2`) instead of the backtick-escaped `` `Unit.*` `` call. Re-reading
        // `a * b` yields the ordinary `*`/`/`/`^` arena head, which the units layer treats as unit
        // composition (`eval::unit_of`) — an idempotent ML round-trip (`ml(ml(x)) == ml(x)`), the surface
        // canonicalizing the legacy `Unit.*` spelling to `*` exactly as it canonicalizes name-alias ctors.
        "Unit.*" => "*",
        "Unit./" => "/",
        "Unit.^" => "^",
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
    Def,
    Type,
    Match,
    With,
    Module,
    Import,
    Export,
    Effect,
    Handle,
    Host,
    /// The unit-conversion operator `q as meter`. Contextual: it is only meaningful in infix position
    /// after an expression (the parser's `expr` loop), so a bare `as` in prefix position is the usual
    /// "keyword outside its form" error. Reserved so it can never be a bare name (the printer
    /// backtick-escapes a name `as`).
    As,
    /// The explicit generic binder `forall a b. TYPE` in a TYPE position — an explicit way to write a
    /// generic signature so a lowercase type var BINDS instead of erroring CDZ0101. Contextual: it is
    /// only meaningful at the start of a type (the parser's `type_ref`); a bare `forall` elsewhere is an
    /// ordinary name. Sugar for the pinned "generics are type-valued parameters" model — a `forall a.`
    /// desugars to an implicit `(: a Type)` binding (v-inference's I2), so it introduces no new ∀ engine.
    Forall,
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
        "def" => Keyword::Def,
        "type" => Keyword::Type,
        "match" => Keyword::Match,
        "with" => Keyword::With,
        "module" => Keyword::Module,
        "import" => Keyword::Import,
        "export" => Keyword::Export,
        "effect" => Keyword::Effect,
        "handle" => Keyword::Handle,
        "host" => Keyword::Host,
        "as" => Keyword::As,
        "forall" => Keyword::Forall,
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
pub const PREC_MEMBER: u8 = 12;

/// Sequencing `a ; b` binds LOOSEST of all — looser even than ascription — so a statement in a
/// sequence is a whole expression (`a : T ; b` groups as `(do (: a T) b)`, not `(: a (do T b))`).
/// It is right-associative and folds a run into a single flat `(do a b c)` (see [`is_right_assoc`]
/// and the parser's Pratt loop): `a; b; c` is `(do a b c)`, the last element the sequence's value.
/// Modelled as `let _ = a in b` — evaluate `a` for effect, then `b`.
pub const PREC_SEQ: u8 = 0;

/// Type ascription `e : T` binds looser than every arithmetic/logical operator (but tighter than
/// sequencing) — so `2 + 2 : Int64` groups as `(: (+ 2 2) Int64)`: the ascription wraps the whole
/// expression.
pub const PREC_ASCRIPTION: u8 = 1;

/// The function-type arrow `A -> B` -> `(-> A B)`. The loosest TYPE constructor, just above ascription,
/// so an ascribed/annotated type captures the whole arrow (`e : A -> B` is `(: e (-> A B))`, and a
/// return type `-> A -> B` is the curried arrow). It is the one RIGHT-associative operator (see
/// [`is_right_assoc`]): `A -> B -> C` groups as `A -> (B -> C)`, the standard curried reading.
pub const PREC_ARROW: u8 = 2;

/// The pipeline operator `|>` binds looser than every binary operator EXCEPT ascription and the type
/// arrow, so `total + tax |> round` groups as `(|> (+ total tax) round)` — the whole left expression is
/// the value threaded into the right — matching the F#/Elm/OCaml convention. Left-associative like
/// every operator, so `a |> f |> g` is `(|> (|> a f) g)` — a left-to-right pipeline.
pub const PREC_PIPELINE: u8 = 3;

/// The unit-conversion operator `q as meter` -> `(Unit.in meter q)`. It binds just above the pipeline
/// and below every arithmetic operator, so a whole arithmetic expression converts as a unit
/// (`240 meter / 8 second as (meter / hour)` groups as `((240 meter / 8 second) as (meter / hour))`)
/// while it still threads into a pipeline as one value (`q as meter |> f`). The parser handles its
/// right operand specially (a UNIT denotation, not an ordinary expression), so it is not in
/// [`infix_prec`]; this constant only names the binding power the special arm compares against.
pub const PREC_AS: u8 = 4;

/// True for the one RIGHT-associative infix operator, the type arrow `->`: `A -> B -> C` groups as
/// `A -> (B -> C)`. Every other operator is left-associative. The parser recurses at `prec` (not
/// `prec + 1`) for a right-associative operator, and the printer descends the RIGHT spine.
pub fn is_right_assoc(op: &str) -> bool {
    op == "->"
}

/// The precedence (binding power) of a binary operator NAME (the ARENA head — `=` is equality here,
/// spelled `==` on the surface; `:` is ascription, `->` the type arrow), or `None` if it is not infix.
/// The single source of truth shared by the parser (Pratt `min_prec`) and printer (minimal-paren
/// `prec`/`prec+1` split). All operators are left-associative EXCEPT the type arrow (see
/// [`is_right_assoc`]).
///
/// Ordering (low→high): `: < -> < |> < or < and < (== < > <= >=) < (| ^) < & < (<< >>) < (+ -) < (* / %)
/// < member/app`. Ascription is loosest; the type arrow sits just above it; the pipeline just above
/// that; equality (`==`, arena head `=`) sits with the comparisons.
pub fn infix_prec(op: &str) -> Option<u8> {
    Some(match op {
        ":" => PREC_ASCRIPTION, // 1 — loosest
        "->" => PREC_ARROW,     // 2 — the loosest type constructor
        "|>" => PREC_PIPELINE,  // 3 — looser than every operator but ascription and `->`
        "or" => 4,
        "and" => 5,
        "=" | "<" | ">" | "<=" | ">=" => 6, // `=` = equality (surface `==`)
        // `Unit.^` renders as the glyph `^` (arena `BitXor`, tier 7), so it MUST share that tier — the
        // printer's parenthesization and the parser's binding power agree on the round-trip.
        "|" | "^" | "Unit.^" => 7,
        "&" => 8,
        "<<" | ">>" => 9,
        "+" | "-" | "+%" | "-%" | "+." | "-." => 10,
        // `Unit.*`/`Unit./` render as `*`/`/` (tier 11), sharing the multiplicative tier for the same
        // round-trip agreement.
        "*" | "/" | "%" | "*%" | "*." | "/." | "Unit.*" | "Unit./" => 11,
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
            Kind::Arrow,
            Kind::Lt,
            Kind::Gt,
            Kind::Le,
            Kind::Ge,
            Kind::PipeGt,
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
            Kind::PlusDot,
            Kind::MinusDot,
            Kind::StarDot,
            Kind::SlashDot,
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
        assert!(infix_prec(":") < infix_prec("->"));
        assert_eq!(infix_prec(":"), Some(PREC_ASCRIPTION));
        // The type arrow sits just above ascription — the loosest type constructor — and is the one
        // right-associative operator.
        assert_eq!(infix_prec("->"), Some(PREC_ARROW));
        assert!(infix_prec("->") < infix_prec("|>"));
        assert!(is_right_assoc("->"));
        assert!(!is_right_assoc("+"));
        // The pipeline sits just above the type arrow — looser than every non-type operator.
        assert_eq!(infix_prec("|>"), Some(PREC_PIPELINE));
        assert!(infix_prec("|>") < infix_prec("or"));
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
        assert_eq!(infix_glyph("->"), "->");
    }

    // Every infix operator ARENA head — the exact domain of `infix_prec`, which is precisely the set of
    // heads the printer ever hands to `infix_glyph` (it glyph-maps the head of an infix form). Kept in
    // lockstep with the `infix_prec` match by `every_infix_head_has_a_precedence` (each must have a prec,
    // so a head deleted from `infix_prec` is caught) — the anchor for the glyph round-trip sweep below.
    const INFIX_HEADS: &[&str] = &[
        ":", "->", "|>", "or", "and", "=", "<", ">", "<=", ">=", "|", "^", "Unit.^", "&", "<<",
        ">>", "+", "-", "+%", "-%", "+.", "-.", "*", "/", "%", "*%", "*.", "/.", "Unit.*",
        "Unit./",
    ];

    #[test]
    fn every_infix_head_has_a_precedence() {
        // Ties INFIX_HEADS to the `infix_prec` table: every listed head is a real infix operator. (A head
        // REMOVED from `infix_prec` but left here fails; the glyph sweep below relies on this list being
        // the true head domain.)
        for h in INFIX_HEADS {
            assert!(
                infix_prec(h).is_some(),
                "{h:?} listed as an infix head but has no precedence"
            );
        }
    }

    #[test]
    fn infix_glyph_is_idempotent_and_identity_except_the_canonicalizing_heads() {
        // The docstring's contract: `infix_glyph` gives an IDEMPOTENT surface round-trip — printing a head
        // as its glyph, re-reading, and re-printing is stable (`ml(ml(x)) == ml(x)`). Nothing pinned that.
        // Over the ENTIRE infix-head domain: (a) `infix_glyph` is idempotent (glyph of a glyph is itself),
        // and (b) it is the IDENTITY except for exactly the four documented CANONICALIZING heads — equality
        // (`=`→`==`) and the three unit-composition heads (`Unit.*`/`Unit./`/`Unit.^`→`*`/`/`/`^`). A stray
        // new non-identity entry (which would need matching parser support to round-trip) or a
        // non-idempotent one (which would make `ml` diverge under re-rendering) is caught here.
        let canonicalizing: &[(&str, &str)] = &[
            ("=", "=="),
            ("Unit.*", "*"),
            ("Unit./", "/"),
            ("Unit.^", "^"),
        ];
        for h in INFIX_HEADS {
            let g = infix_glyph(h);
            // (a) idempotent: the glyph is a fixpoint, so re-rendering never diverges.
            assert_eq!(
                infix_glyph(g),
                g,
                "infix_glyph not idempotent at {h:?} (glyph {g:?})"
            );
            // (b) identity unless `h` is one of the four canonicalizing heads, in which case it maps to
            // exactly the documented glyph.
            match canonicalizing.iter().find(|&&(head, _)| head == *h) {
                Some(&(_, want)) => assert_eq!(g, want, "{h:?} must render as {want:?}"),
                None => assert_eq!(g, *h, "{h:?} must be identity (not a canonicalizing head)"),
            }
        }
        // The canonicalizing set is EXACTLY these four — no head outside it is non-identity (the `None`
        // arm above), and each listed one is present in the head domain (so the table can't go stale).
        for &(head, _) in canonicalizing {
            assert!(
                INFIX_HEADS.contains(&head),
                "canonicalizing head {head:?} missing from INFIX_HEADS"
            );
        }
    }

    #[test]
    fn unit_composition_heads_render_infix() {
        // Unit composition renders as its math glyph so a unit reads like `meter / second` / `meter ^ 2`
        // instead of the backtick-escaped `` `Unit.*` `` call. The glyph re-reads to the ordinary numeric
        // head (the units layer treats it as composition), an idempotent ML round-trip.
        assert_eq!(infix_glyph("Unit.*"), "*");
        assert_eq!(infix_glyph("Unit./"), "/");
        assert_eq!(infix_glyph("Unit.^"), "^");
        // Each shares the tier of the glyph it renders as, so parser binding power and printer
        // parenthesization agree on the round-trip.
        assert_eq!(infix_prec("Unit.*"), infix_prec("*"));
        assert_eq!(infix_prec("Unit./"), infix_prec("/"));
        assert_eq!(infix_prec("Unit.^"), infix_prec("^"));
    }

    // The full set of keyword spellings the parser recognizes — kept in sync with the `keyword` match
    // and the `Keyword` enum. `all_keyword_spellings_classify` pins that each spelling maps to a
    // keyword; `every_keyword_variant_has_a_spelling` pins the reverse (no enum variant is unreachable).
    const KEYWORD_SPELLINGS: &[(&str, Keyword)] = &[
        ("let", Keyword::Let),
        ("in", Keyword::In),
        ("if", Keyword::If),
        ("then", Keyword::Then),
        ("else", Keyword::Else),
        ("fn", Keyword::Fn),
        ("def", Keyword::Def),
        ("type", Keyword::Type),
        ("match", Keyword::Match),
        ("with", Keyword::With),
        ("module", Keyword::Module),
        ("import", Keyword::Import),
        ("export", Keyword::Export),
        ("effect", Keyword::Effect),
        ("handle", Keyword::Handle),
        ("host", Keyword::Host),
        ("as", Keyword::As),
        ("forall", Keyword::Forall),
    ];

    #[test]
    fn all_keyword_spellings_classify() {
        // Each listed spelling classifies as its keyword; a non-keyword word (and an ordinary
        // kebab-case identifier) classifies as None.
        for &(text, kw) in KEYWORD_SPELLINGS {
            assert_eq!(keyword(text), Some(kw), "{text:?} is a keyword");
        }
        for text in [
            "and", "or", "byte-at", "letx", "iff", "As", "LET", "", "let ",
        ] {
            assert_eq!(keyword(text), None, "{text:?} is not a keyword");
        }
    }

    #[test]
    fn keywords_are_case_sensitive_and_exact() {
        // Keyword classification is exact-match: no case-folding, no prefix match. A capitalized or
        // substring'd spelling is an ordinary identifier (so `Let`, `Type`-prefixed names are free).
        assert_eq!(keyword("As"), None);
        assert_eq!(keyword("Type"), None);
        assert_eq!(keyword("matches"), None);
        assert_eq!(keyword("in-scope"), None);
    }

    #[test]
    fn keyword_and_word_op_are_disjoint() {
        // A spelling is at most ONE of {keyword, word-operator} — the parser's contextual decision
        // relies on these two classifiers never both firing for the same text. `and`/`or` are the two
        // word operators and must NOT also be keywords.
        for &(text, _) in KEYWORD_SPELLINGS {
            assert!(
                word_op(text).is_none(),
                "{text:?} is a keyword and must not also be a word-operator"
            );
        }
        for w in ["and", "or"] {
            assert_eq!(keyword(w), None, "{w:?} is a word-operator, not a keyword");
            assert_eq!(word_op(w), Some(w));
        }
        assert_eq!(word_op("nand"), None); // not a word operator
    }

    #[test]
    fn is_reserved_is_exactly_keyword_union_word_op() {
        // `is_reserved` (what the printer backtick-escapes so a reserved word survives as a bare name)
        // MUST be exactly keyword ∪ word_op — no more, no less. Every keyword and word-op is reserved;
        // an ordinary identifier is not.
        for &(text, _) in KEYWORD_SPELLINGS {
            assert!(is_reserved(text), "{text:?} (keyword) is reserved");
        }
        for w in ["and", "or"] {
            assert!(is_reserved(w), "{w:?} (word-op) is reserved");
        }
        for text in ["byte-at", "foo", "let-binding", "and-then", "x"] {
            assert!(
                !is_reserved(text),
                "{text:?} is an ordinary name, not reserved"
            );
        }
    }

    /// The canonical spelling of every `Keyword` variant. This match is EXHAUSTIVE, so adding a variant
    /// to the enum is a COMPILE ERROR here until it is given a spelling — this is the compile-time half
    /// of the "no dead/unspelled keyword variant" guard. `every_keyword_variant_has_a_spelling` is the
    /// runtime half: it drives assertions from [`ALL_KEYWORDS`] (the full variant set), not from the
    /// `KEYWORD_SPELLINGS` table, so a variant MISSING from that table is caught (the original test
    /// iterated the table and could not — PR #420 Copilot finding).
    fn canonical_spelling(kw: Keyword) -> &'static str {
        match kw {
            Keyword::Let => "let",
            Keyword::In => "in",
            Keyword::If => "if",
            Keyword::Then => "then",
            Keyword::Else => "else",
            Keyword::Fn => "fn",
            Keyword::Def => "def",
            Keyword::Type => "type",
            Keyword::Match => "match",
            Keyword::With => "with",
            Keyword::Module => "module",
            Keyword::Import => "import",
            Keyword::Export => "export",
            Keyword::Effect => "effect",
            Keyword::Handle => "handle",
            Keyword::Host => "host",
            Keyword::As => "as",
            Keyword::Forall => "forall",
        }
    }

    // Every `Keyword` variant, independent of the `KEYWORD_SPELLINGS` table — a source of "all variants"
    // the tests iterate. Its length is tied to the variant count at COMPILE time by the `const` assertion
    // below (`ALL_KEYWORDS.len() == KEYWORD_COUNT`), so a `Keyword` variant added to the enum forces this
    // array to grow: the variant makes `keyword_ordinal`'s exhaustive match fail to compile until given
    // an ordinal (which bumps `KEYWORD_COUNT`), and the assertion then fails until the entry is added
    // here. (Content — that the entries are the RIGHT, distinct variants — is checked at test time by
    // `every_keyword_variant_has_a_spelling`.)
    const ALL_KEYWORDS: &[Keyword] = &[
        Keyword::Let,
        Keyword::In,
        Keyword::If,
        Keyword::Then,
        Keyword::Else,
        Keyword::Fn,
        Keyword::Def,
        Keyword::Type,
        Keyword::Match,
        Keyword::With,
        Keyword::Module,
        Keyword::Import,
        Keyword::Export,
        Keyword::Effect,
        Keyword::Handle,
        Keyword::Host,
        Keyword::As,
        Keyword::Forall,
    ];

    // The number of `Keyword` variants, defined by an EXHAUSTIVE match: adding a variant to the enum
    // fails to compile here until it is given a distinct ordinal, and `KEYWORD_COUNT` is one past the
    // last. This is the compile-time anchor `ALL_KEYWORDS`'s length is checked against.
    const fn keyword_ordinal(kw: Keyword) -> usize {
        match kw {
            Keyword::Let => 0,
            Keyword::In => 1,
            Keyword::If => 2,
            Keyword::Then => 3,
            Keyword::Else => 4,
            Keyword::Fn => 5,
            Keyword::Def => 6,
            Keyword::Type => 7,
            Keyword::Match => 8,
            Keyword::With => 9,
            Keyword::Module => 10,
            Keyword::Import => 11,
            Keyword::Export => 12,
            Keyword::Effect => 13,
            Keyword::Handle => 14,
            Keyword::Host => 15,
            Keyword::As => 16,
            Keyword::Forall => 17,
        }
    }
    const KEYWORD_COUNT: usize = keyword_ordinal(Keyword::Forall) + 1;

    // COMPILE-TIME completeness: `ALL_KEYWORDS` must list exactly one entry per variant. A new variant
    // (once given a `keyword_ordinal`, which raises `KEYWORD_COUNT`) makes this fail to compile until it
    // is added to `ALL_KEYWORDS`. This is the real tie the old `assert_all_keywords_listed` comment
    // CLAIMED but did not deliver (its `matches!` forced naming a variant, but never referenced
    // `ALL_KEYWORDS`, so a variant omitted from the array compiled fine — PR #423 Copilot finding).
    const _: () = assert!(
        ALL_KEYWORDS.len() == KEYWORD_COUNT,
        "ALL_KEYWORDS must list every Keyword variant exactly once"
    );

    #[test]
    fn every_keyword_variant_has_a_spelling() {
        // Drive from ALL_KEYWORDS (the full variant set), NOT KEYWORD_SPELLINGS — so a variant that is
        // missing from the table is caught (the original test iterated the table and so could not; PR
        // #420 Copilot finding). For every variant: (1) its canonical spelling classifies back to it,
        // and (2) it appears in the KEYWORD_SPELLINGS table (which the other tests iterate).
        for &kw in ALL_KEYWORDS {
            assert_eq!(
                keyword(canonical_spelling(kw)),
                Some(kw),
                "{kw:?} round-trips through its spelling"
            );
            assert!(
                KEYWORD_SPELLINGS.iter().any(|&(_, k)| k == kw),
                "{kw:?} is missing from KEYWORD_SPELLINGS (the table the other tests iterate)"
            );
        }
        // The reverse direction: the table has no stale/extra entry. `ALL_KEYWORDS` completeness is
        // already enforced at COMPILE time (`ALL_KEYWORDS.len() == KEYWORD_COUNT`), so equal lengths here
        // means `KEYWORD_SPELLINGS` also has exactly one entry per variant (the loop above proved each
        // variant IS present, and no duplicates exist because the spellings are distinct keywords).
        assert_eq!(
            KEYWORD_SPELLINGS.len(),
            ALL_KEYWORDS.len(),
            "KEYWORD_SPELLINGS must have exactly one entry per Keyword variant"
        );
    }
}
