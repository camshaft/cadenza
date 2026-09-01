//! The lexer — a simple, total, streaming tokenizer shared by both surfaces. A char-by-char
//! `Iterator<Item = Token>` over `Peek2<Chars>` (no allocation, no collecting), extended from the
//! earlier `cadenza-syntax` scanner with the ML rules the surface needs (kebab-case `-`,
//! radix/exponent numbers, backtick names, wrapping operators, the quasiquote sigils).
//!
//! It is deliberately **keyword-free**: every word is an [`Kind::Ident`], and the parser decides
//! whether a given identifier is a keyword from its text and position (see [`crate::token`]). This
//! keeps the lexer dumb and lets one tokenizer serve the ML grammar and the s-expression grammar.
//!
//! It yields trivia (whitespace/comments) too, for losslessness, then ends the stream (no explicit
//! `Eof` token — the iterator simply finishes). Literal *values* are decoded later from the spanned
//! text by [`crate::literal`], so the integer/float representation is identical on every surface.

use crate::iter::{Char, Chars, Peek2};
use crate::span::Span;
use crate::token::Kind;

/// A lexed token: its kind and the byte span of its source text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: Kind,
    pub span: Span,
}

/// A streaming tokenizer. Iterate it to get `Token`s; it yields trivia and ends without an `Eof`.
pub struct Lexer<'a> {
    chars: Peek2<Chars<'a>>,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Lexer<'a> {
        Lexer {
            chars: Peek2::new(Chars::new(src)),
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.chars.peek().map(|c| c.value)
    }
    fn peek2(&mut self) -> Option<char> {
        self.chars.peek2().map(|c| c.value)
    }
    fn bump(&mut self) -> Option<Char> {
        self.chars.next()
    }

    /// Consume chars while `pred` holds, starting from `start`; return the span from `start`'s
    /// beginning through the last consumed char's end.
    fn span_while(&mut self, start: Char, mut pred: impl FnMut(char) -> bool) -> Span {
        let mut end = start.span;
        while let Some(c) = self.chars.next_if(|c| pred(c.value)) {
            end = c.span;
        }
        start.span.merge(end)
    }

    fn next_token(&mut self) -> Option<Token> {
        let a = self.bump()?;
        let kind = match a.value {
            ' ' | '\t' | '\r' | '\n' => {
                let span = self.span_while(a, |c| matches!(c, ' ' | '\t' | '\r' | '\n'));
                return Some(Token {
                    kind: Kind::Whitespace,
                    span,
                });
            }
            '(' => Kind::LParen,
            ')' => Kind::RParen,
            '{' => Kind::LBrace,
            '}' => Kind::RBrace,
            '[' => Kind::LBracket,
            ']' => Kind::RBracket,
            // `#\…` is a char literal (one Unicode scalar); `#"…"` is a symbol literal (an interned name
            // value, reusing string lexing); `#name` is the UNQUOTED symbol sugar (the quotes are only
            // needed when the content is not a bare identifier); `#{`/`#[`/bare `#` is the map/raw-list
            // sigil (`{`/`[` are not ident-starts, so the sugar never swallows them).
            '#' if self.peek() == Some('\\') => return Some(self.char_lit(a)),
            '#' if self.peek() == Some('"') => {
                let quote = self.bump().unwrap(); // the opening `"`
                let str_tok = self.read_string(quote);
                return Some(Token {
                    kind: if str_tok.kind == Kind::Str {
                        Kind::SymLit
                    } else {
                        Kind::Error // unterminated
                    },
                    span: a.span.merge(str_tok.span),
                });
            }
            // `#name` — the unquoted symbol sugar. The `#` glued to an identifier-start char lexes as
            // one `SymLit` spanning `#` through the identifier; the identifier body follows the same
            // kebab-case rule as a bare `Ident` (so `#map-insert` is one symbol). `unescape_sym_token`
            // decodes both `#"…"` and this bare form to the same `Leaf::Sym`.
            '#' if self.peek().is_some_and(is_ident_start) => {
                let first = self.bump().unwrap(); // the identifier's first char
                let ident = self.ident(first);
                return Some(Token {
                    kind: Kind::SymLit,
                    span: a.span.merge(ident.span),
                });
            }
            '#' => Kind::Hash,
            // `@!` is the PRAGMA sugar (`@!default-float Float32` -> `(pragma default-float Float32)`) —
            // the inner-attribute twin of `@` (an annotation applies to the item BELOW it, a pragma to the
            // enclosing MODULE, mirroring Rust's `#[…]` vs `#![…]`). Glued: `@` immediately followed by `!`
            // is ONE token, checked before the bare `@` below.
            '@' if self.peek() == Some('!') => {
                let b = self.bump().unwrap(); // the `!`
                return Some(Token {
                    kind: Kind::AtBang,
                    span: a.span.merge(b.span),
                });
            }
            // A bare `@` is the ANNOTATION sigil (`@inline-never def …`); the parser wraps the
            // following form as `(@ name form)`. (The `,@` splice is a `,`-led token, handled above,
            // so a lone `@` only ever reaches here as an annotation prefix.)
            '@' => Kind::At,
            // `..` is the rest/spread marker (one token); a lone `.` is member access. A float's
            // fractional `.` is consumed inside `number` (it needs a digit after the `.`), so it never
            // reaches here — `1..n` therefore lexes `1` `..` `n`, not `1.` `.n`.
            '.' if self.peek() == Some('.') => {
                let b = self.bump().unwrap(); // the second `.`
                // `..=` (the closed-range operator `lo..=hi`) lexes to its OWN token, distinct from `..`.
                // A `..=` is only ever this operator (there is no `..=` rest-marker), so greedily gluing
                // the `=` onto `..` is unambiguous — the lexer commits the token; the grammar is elsewhere.
                if self.peek() == Some('=') {
                    let c = self.bump().unwrap();
                    return Some(Token {
                        kind: Kind::DotDotEq,
                        span: a.span.merge(c.span),
                    });
                }
                return Some(Token {
                    kind: Kind::DotDot,
                    span: a.span.merge(b.span),
                });
            }
            '.' => Kind::Dot,
            ':' => Kind::Colon,
            ';' => Kind::Semi,
            ',' => match self.peek() {
                Some('@') => {
                    let b = self.bump().unwrap();
                    return Some(Token {
                        kind: Kind::UnquoteSplice,
                        span: a.span.merge(b.span),
                    });
                }
                _ => Kind::Comma,
            },
            // `` `{ `` opens a quasiquote block; `` `name` `` is a symbolic-name escape. `a` (the
            // backtick) is already consumed, so the block's `{` is the current `peek`.
            '`' if self.peek() == Some('{') => Kind::Backtick,
            '`' => return Some(self.read_backtick_name(a)),
            '"' => return Some(self.read_string(a)),
            '=' => match self.peek() {
                Some('>') => {
                    let b = self.bump().unwrap();
                    return Some(Token {
                        kind: Kind::FatArrow,
                        span: a.span.merge(b.span),
                    });
                }
                // `==` is equality (its arena head is `=`); a lone `=` is the binding separator.
                Some('=') => {
                    let b = self.bump().unwrap();
                    return Some(Token {
                        kind: Kind::EqEq,
                        span: a.span.merge(b.span),
                    });
                }
                _ => Kind::Eq,
            },
            '<' => return Some(self.two(a, Kind::Lt, &[('=', Kind::Le), ('<', Kind::Shl)])),
            '>' => return Some(self.two(a, Kind::Gt, &[('=', Kind::Ge), ('>', Kind::Shr)])),
            '+' => return Some(self.wrapping(a, Kind::Plus, Kind::PlusPct, Kind::PlusDot)),
            '*' => return Some(self.wrapping(a, Kind::Star, Kind::StarPct, Kind::StarDot)),
            '%' => Kind::Percent,
            '&' => Kind::Amp,
            '^' => Kind::Caret,
            '|' => return Some(self.two(a, Kind::Pipe, &[('>', Kind::PipeGt)])),
            '/' => return Some(self.slash(a)),
            '-' => return Some(self.minus(a)),
            c if c.is_ascii_digit() => return Some(self.number(a)),
            // `b"…"` is a byte-string literal (the surface form of a `Bytes`), NOT the identifier `b`
            // followed by a string. Must precede the ident arm since `b` is an ident-start. The parser
            // builds `Leaf::Bytes` from it, mirroring the sexpr reader's `read_byte_string`.
            'b' if self.peek() == Some('"') => {
                let quote = self.bump().unwrap(); // the opening `"`
                let str_tok = self.read_string(quote);
                return Some(Token {
                    kind: if str_tok.kind == Kind::Str {
                        Kind::ByteStr
                    } else {
                        Kind::Error // unterminated
                    },
                    span: a.span.merge(str_tok.span),
                });
            }
            // `b[` opens a binary literal `b[<segment>, …]` (the structured sibling of the `b"…"` byte
            // string), NOT the identifier `b` followed by a list `[…]`. Glued only when `[` immediately
            // follows `b` (no whitespace), exactly like `b"`; `b [0]` stays the name `b` then a list.
            // Must precede the ident arm since `b` is an ident-start. `]` closes it (an ordinary
            // `RBracket`); the parser desugars the segment list to `(bin …)`.
            'b' if self.peek() == Some('[') => {
                let bracket = self.bump().unwrap(); // the `[`
                return Some(Token {
                    kind: Kind::BinOpen,
                    span: a.span.merge(bracket.span),
                });
            }
            c if is_ident_start(c) => return Some(self.ident(a)),
            _ => Kind::Error,
        };
        Some(Token { kind, span: a.span })
    }

    /// A one-or-two-char operator: if the next char matches one of `alts`, consume it and use that
    /// kind; else `base`.
    fn two(&mut self, a: Char, base: Kind, alts: &[(char, Kind)]) -> Token {
        if let Some(next) = self.peek() {
            for &(ch, kind) in alts {
                if next == ch {
                    let b = self.bump().unwrap();
                    return Token {
                        kind,
                        span: a.span.merge(b.span),
                    };
                }
            }
        }
        Token {
            kind: base,
            span: a.span,
        }
    }

    /// `+`/`*` with an optional `%` wrapping suffix OR a `.` float suffix (`+.`/`*.` — the OCaml-style
    /// floating-point operators, distinct from the integer `+`/`*`). The two suffixes are mutually
    /// exclusive glyphs, so a single peek decides: `%` → wrapping, `.` → float, else plain.
    fn wrapping(&mut self, a: Char, plain: Kind, wrapping: Kind, float: Kind) -> Token {
        match self.peek() {
            Some('%') => {
                let b = self.bump().unwrap();
                Token {
                    kind: wrapping,
                    span: a.span.merge(b.span),
                }
            }
            Some('.') => {
                let b = self.bump().unwrap();
                Token {
                    kind: float,
                    span: a.span.merge(b.span),
                }
            }
            _ => Token {
                kind: plain,
                span: a.span,
            },
        }
    }

    /// `/` — a `//` line comment, `///` doc comment, `/.` float division, or the division operator.
    fn slash(&mut self, a: Char) -> Token {
        if self.peek() == Some('/') {
            // The `//` (and `///`) prefix chars are consumed here, so the comment span must extend from
            // `a` through the LAST prefix `/` (`span_while`'s `end` starts at its `start` arg, which
            // would omit these already-bumped chars when the comment body is empty — e.g. `//` at EOF or
            // immediately before a `\n`, which left byte-2 of the `//` uncovered: a span gap).
            let mut last = self.bump().unwrap(); // second '/'
            let doc = self.peek() == Some('/');
            if doc {
                last = self.bump().unwrap(); // third '/'
            }
            let body = self.span_while(last, |c| c != '\n');
            let span = a.span.merge(body);
            let kind = if doc {
                Kind::DocComment
            } else {
                Kind::LineComment
            };
            Token { kind, span }
        } else if self.peek() == Some('.') {
            // `/.` — floating-point division (distinct from the integer `/`). Checked AFTER the
            // comment forms so `//`/`///` still win.
            let b = self.bump().unwrap();
            Token {
                kind: Kind::SlashDot,
                span: a.span.merge(b.span),
            }
        } else {
            Token {
                kind: Kind::Slash,
                span: a.span,
            }
        }
    }

    /// `-` — kebab rule: a `-` glued between word chars is part of an identifier and never reaches
    /// here (see `ident`). Here, a `-` with a non-word char to its left resolves to: `->` arrow,
    /// `-%` wrapping-sub, `-<digit>` negative number, or bare `-` subtraction.
    fn minus(&mut self, a: Char) -> Token {
        match self.peek() {
            Some('>') => {
                let b = self.bump().unwrap();
                Token {
                    kind: Kind::Arrow,
                    span: a.span.merge(b.span),
                }
            }
            Some(d) if d.is_ascii_digit() => self.number(a),
            Some('%') => {
                let b = self.bump().unwrap();
                Token {
                    kind: Kind::MinusPct,
                    span: a.span.merge(b.span),
                }
            }
            // `-.` — floating-point subtraction. A float LITERAL needs a leading digit (`.5` is not a
            // number), so a `-` followed by `.` is unambiguously the float operator, not a negative
            // fraction. (Checked after the `-<digit>` negative-number arm above.)
            Some('.') => {
                let b = self.bump().unwrap();
                Token {
                    kind: Kind::MinusDot,
                    span: a.span.merge(b.span),
                }
            }
            _ => Token {
                kind: Kind::Minus,
                span: a.span,
            },
        }
    }

    /// A char literal `#\…`. `a` is the `#` (already consumed); the next char is `\` (confirmed by the
    /// caller). The FIRST char after `\` is taken verbatim — even a delimiter (`#\(`, `#\ `) — then any
    /// further non-delimiter chars complete a NAMED (`newline`) or code-point (`u+00E9`) spelling. The
    /// parser turns the token text into a `Leaf::Char` / `Leaf::BadChar` via `literal::char_leaf`.
    fn char_lit(&mut self, a: Char) -> Token {
        let bs = self.bump().unwrap(); // the `\`
        let mut end = bs.span;
        // The mandatory first scalar (any char, delimiter or not).
        match self.bump() {
            Some(c) => {
                end = c.span;
                // Trailing word chars (letters/digits/`+`) complete `newline` / `u+00E9`; a delimiter or
                // any punctuation stops the literal so `#\a` is exactly one scalar.
                while let Some(nc) = self
                    .chars
                    .next_if(|c| c.value.is_alphanumeric() || c.value == '+')
                {
                    end = nc.span;
                }
            }
            None => {
                return Token {
                    kind: Kind::Error,
                    span: a.span.merge(end),
                };
            }
        }
        Token {
            kind: Kind::CharLit,
            span: a.span.merge(end),
        }
    }

    /// The shared delimited-scan loop for a `close`-terminated token: scan from `a` to the next
    /// unescaped `close`, returning a `kind` token spanning `a..=close`; a `\`-escape consumes the
    /// next char verbatim; EOF before `close` yields an `Error` token (unterminated).
    fn read_delimited(&mut self, a: Char, close: char, kind: Kind) -> Token {
        let mut end = a.span;
        loop {
            match self.bump() {
                None => {
                    return Token {
                        kind: Kind::Error,
                        span: a.span.merge(end),
                    };
                } // unterminated
                Some(c) if c.value == close => {
                    return Token {
                        kind,
                        span: a.span.merge(c.span),
                    };
                }
                Some(c) if c.value == '\\' => match self.bump() {
                    None => {
                        return Token {
                            kind: Kind::Error,
                            span: a.span.merge(c.span),
                        };
                    }
                    Some(d) => end = d.span,
                },
                Some(c) => end = c.span,
            }
        }
    }

    fn read_backtick_name(&mut self, a: Char) -> Token {
        self.read_delimited(a, '`', Kind::BacktickName)
    }

    fn read_string(&mut self, a: Char) -> Token {
        self.read_delimited(a, '"', Kind::Str)
    }

    /// A numeric literal: an optional leading `-` (already consumed as `a` on the `minus` path),
    /// radix prefix (`0x`/`0b`), digits + `_` separators, optional `.frac` and `e`-exponent. The
    /// lexer is permissive on shape; [`crate::literal`] validates strictly and decides Int vs Float
    /// vs Name from the exact text.
    fn number(&mut self, a: Char) -> Token {
        let mut end = a.span;
        // For a `-0x…`/`-0b…` (the `minus` path, `a` is the `-`), consume the `0` so the radix
        // prefix `x`/`b` becomes the current char, exactly as when `a` is the bare `0`.
        if a.value == '-'
            && self.peek() == Some('0')
            && matches!(self.peek2(), Some('x' | 'X' | 'b' | 'B'))
        {
            end = self.bump().unwrap().span; // the `0`
        }
        // radix prefix: current char is x/X/b/B, and the char before it was `0` (either `a` itself,
        // or the `0` just consumed on the `-0` path).
        let after_zero = a.value == '0' || (a.value == '-' && end != a.span);
        if after_zero && matches!(self.peek(), Some('x' | 'X' | 'b' | 'B')) {
            end = self.bump().unwrap().span; // x/b
            while matches!(self.peek(), Some(c) if c.is_ascii_hexdigit() || c == '_') {
                end = self.bump().unwrap().span;
            }
            // A glued TYPE SUFFIX (`0xFFN`, `0b1010R`) on a RADIX literal — the same single-`N`/`R` peel the
            // decimal path does below. Without it a glued `N`/`R` (never a hex digit, so the loop above
            // stops before it) re-lexes as a bare word, which the ML quantity sugar reads as `(Qty.of 0xFF
            // (Unit.of "N"))` → CDZ0201 "unknown unit N", while the equivalent `255N` suffixes correctly.
            // Take it only when what follows cannot CONTINUE an identifier, exactly as the decimal peel does
            // (`0xFFNx` is not a suffix — the whole token then fails the numeric parse and falls to a Name).
            if matches!(self.peek(), Some('N' | 'R'))
                && !self.peek2().is_some_and(is_ident_continue)
            {
                end = self.bump().unwrap().span; // consume the N/R suffix
            }
            return Token {
                kind: Kind::Int,
                span: a.span.merge(end),
            };
        }
        let mut is_float = false;
        // integer part (a may itself be a digit, or the leading `-`).
        while matches!(self.peek(), Some(c) if c.is_ascii_digit() || c == '_') {
            end = self.bump().unwrap().span;
        }
        // There is NO native rational LITERAL on the ML surface (seq-204): the operator dropped the `r`
        // glyph (`3r2`), and unspaced `3/2` is Int64 integer division, so a bare literal cannot spell a
        // rational unambiguously. A rational reaches ML source via `(/ n d)`-style construction (which the
        // compiler grounds to a normalized `Rational`), never a scalar literal; the printer still RENDERS a
        // native rational VALUE node as `num/den` (a value surface, not a source round-trip).
        // fractional part: `.` followed by a digit.
        if self.peek() == Some('.') && self.peek2().is_some_and(|c| c.is_ascii_digit()) {
            is_float = true;
            end = self.bump().unwrap().span; // .
            while matches!(self.peek(), Some(c) if c.is_ascii_digit() || c == '_') {
                end = self.bump().unwrap().span;
            }
        }
        // exponent: e/E [+/-] digits.
        if matches!(self.peek(), Some('e' | 'E')) {
            // Look ahead: an exponent needs at least one digit (after an optional sign). We can only
            // peek two chars, so accept `e<digit>` or `e<sign>` here and let the digit loop confirm;
            // if no digit follows, the token still ends at the last digit consumed (the `e`… stays
            // for the next token). Since a bare `e` is rare after a number, we take the simple rule:
            // consume `e` + optional sign + digits only when a digit is visible within reach.
            let after = self.peek2();
            let exp_ok = matches!(after, Some(c) if c.is_ascii_digit())
                || (matches!(after, Some('+' | '-')));
            if exp_ok {
                end = self.bump().unwrap().span; // e
                if matches!(self.peek(), Some('+' | '-')) {
                    end = self.bump().unwrap().span;
                }
                if self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    is_float = true;
                    while matches!(self.peek(), Some(c) if c.is_ascii_digit() || c == '_') {
                        end = self.bump().unwrap().span;
                    }
                }
            }
        }
        // A glued TYPE SUFFIX (`100N`, `0.5R`): a single `N`/`R` letter immediately after the number,
        // with NO intervening space, is part of THIS token so `classify_word` sees `100N` whole and
        // builds a `Suffixed` leaf. A SPACE before the letter (`5 R`) is a separate token — on the ML
        // surface that stays quantity-literal sugar (a unit named `R`). Only take the suffix when what
        // follows it cannot CONTINUE an identifier, so a bare `100N` suffixes but a `100Nx` does not
        // (the whole token then fails the numeric parse and falls through to a `Name`, rejected — never
        // a silent mis-read). The token keeps its `Int`/`Float` kind; `classify_word` re-parses the
        // body together with the suffix letter.
        if matches!(self.peek(), Some('N' | 'R')) && !self.peek2().is_some_and(is_ident_continue) {
            end = self.bump().unwrap().span; // consume the N/R suffix
        }
        let kind = if is_float { Kind::Float } else { Kind::Int };
        Token {
            kind,
            span: a.span.merge(end),
        }
    }

    /// An identifier word (`a` is its first char): alphanumerics/`_`/non-ASCII, with kebab `-`
    /// glued between word chars. Always `Ident`; `crate::literal` decides if the text is numeric.
    fn ident(&mut self, a: Char) -> Token {
        let mut end = a.span;
        loop {
            match self.peek() {
                Some(c) if is_ident_continue(c) => {
                    end = self.bump().unwrap().span;
                }
                // kebab: a `-` glued between two word chars is part of the identifier.
                Some('-') if self.peek2().is_some_and(is_ident_continue) => {
                    end = self.bump().unwrap().span;
                }
                _ => break,
            }
        }
        // A `"` GLUED to the identifier (no whitespace) is a TAGGED TEMPLATE `tag"…"` — the ident is the
        // tag, the string its body. One token spanning `tag` through the closing `"`, exactly like `b"`
        // is one token (not the name `b` then a string). A SPACE between (`tag "s"`) stays a bare ident
        // then a separate string. (`b"`/`#"` are handled earlier by their own arms; any OTHER ident glued
        // to `"` reaches here.) The parser splits the span into tag + chunks + holes. Unterminated → Error.
        //
        //= spec/capabilities/metaprogramming.md#a-tagged-template-is-a-binding-dispatched-compile-time-macro-over-literal-chunks-and-holes
        //# A tagged template — an identifier written immediately before a string literal, with no intervening whitespace, such as `tag"…text…{expr}…"` — MUST lex to a single canonical abstract-syntax-tree node carrying the tag name, the literal string chunks between the interpolation holes, and the holes, so that an embedded foreign syntax is captured as ordinary program data.
        if self.peek() == Some('"') {
            let quote = self.bump().unwrap(); // the opening `"`
            let body = self.read_template_body(quote);
            return Token {
                kind: body.kind, // TaggedTemplate on success, Error if unterminated
                span: a.span.merge(body.span),
            };
        }
        Token {
            kind: Kind::Ident,
            span: a.span.merge(end),
        }
    }

    /// Scan a tagged-template body from the opening `"` to the matching closing `"`, TRACKING `{…}`
    /// HOLE nesting so a `"` INSIDE a hole (a string literal in the interpolated expression, e.g.
    /// `jsx"a{f("x")}b"`) does NOT close the template. This is why a template body cannot reuse
    /// `read_string` (which stops at the first unescaped `"`). Rules:
    ///   * at brace-depth 0, `{{`/`}}` are ESCAPES (literal braces, not a hole) — consume both chars;
    ///   * at brace-depth 0, a lone `{` OPENS a hole (depth→1); `}` at depth 0 outside an escape is a
    ///     stray literal (kept — the parser decides), `"` CLOSES the template;
    ///   * inside a hole (depth>0), `{`/`}` adjust depth and a `"` opens/closes a nested string literal
    ///     (so braces/quotes inside the hole's own strings don't miscount); depth returns to 0 to end
    ///     the hole. A `\`-escape consumes the next char verbatim anywhere (string escapes).
    ///
    /// Returns a `TaggedTemplate` token on a clean close, or `Error` if the body/hole is unterminated.
    /// The parser re-scans this same body text (via `literal::split_template_body`) to build the node.
    //
    // This scan is a pure lexical brace/quote walk: it runs no program code and knows no embedded
    // grammar — it only tracks `{…}` hole nesting so the body can later be split into chunks + holes.
    //
    //= spec/capabilities/metaprogramming.md#a-tagged-template-is-a-binding-dispatched-compile-time-macro-over-literal-chunks-and-holes
    //# The reader MUST NOT run any program code or learn any grammar when lexing a tagged template, so that the reader stays outside the compiler's trusted path exactly as it does for every other form.
    fn read_template_body(&mut self, open: Char) -> Token {
        let mut end = open.span;
        let mut depth: u32 = 0; // `{…}` hole nesting
        let mut in_hole_string = false; // inside a `"…"` within a hole
        loop {
            let Some(c) = self.bump() else {
                return Token {
                    kind: Kind::Error,
                    span: open.span.merge(end),
                }; // unterminated
            };
            end = c.span;
            match c.value {
                '\\' => {
                    // A backslash escapes the next char (string escapes, in body text or a hole string).
                    if let Some(d) = self.bump() {
                        end = d.span;
                    } else {
                        return Token {
                            kind: Kind::Error,
                            span: open.span.merge(end),
                        };
                    }
                }
                '"' if depth == 0 => {
                    // The closing quote of the template (only at depth 0, outside any hole).
                    return Token {
                        kind: Kind::TaggedTemplate,
                        span: open.span.merge(end),
                    };
                }
                '"' if depth > 0 => {
                    // A `"` inside a hole toggles a nested string literal so its braces don't miscount.
                    in_hole_string = !in_hole_string;
                }
                '{' if depth == 0 && !in_hole_string => {
                    if self.peek() == Some('{') {
                        end = self.bump().unwrap().span; // `{{` escape — consume the second `{`
                    } else {
                        depth = 1; // open a hole
                    }
                }
                '}' if depth == 0 && !in_hole_string && self.peek() == Some('}') => {
                    end = self.bump().unwrap().span; // `}}` escape — consume the second `}`
                }
                '{' if depth > 0 && !in_hole_string => depth += 1,
                '}' if depth > 0 && !in_hole_string => depth -= 1,
                _ => {}
            }
        }
    }
}

impl Iterator for Lexer<'_> {
    type Item = Token;
    fn next(&mut self) -> Option<Token> {
        self.next_token()
    }
}

/// A char that can start an identifier: a letter, `_`, or any non-ASCII non-whitespace char.
fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic() || (!c.is_ascii() && !c.is_whitespace())
}

/// A char that can continue an identifier (adds digits to `is_ident_start`).
fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_alphanumeric() || (!c.is_ascii() && !c.is_whitespace())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<Kind> {
        Lexer::new(src)
            .map(|t| t.kind)
            .filter(|k| !k.is_trivia())
            .collect()
    }

    fn spanned_text(src: &str) -> Vec<(&str, Kind)> {
        Lexer::new(src)
            .filter(|t| !t.kind.is_trivia())
            .map(|t| (&src[t.span.start..t.span.end], t.kind))
            .collect()
    }

    #[test]
    fn spans_cover_source_exactly() {
        // Concatenating every token's span text (trivia included) reproduces the source.
        let src = "let x = f(a, b-c) // note\n  1.5 + `|`";
        let mut rebuilt = String::new();
        for t in Lexer::new(src) {
            rebuilt.push_str(&src[t.span.start..t.span.end]);
        }
        assert_eq!(rebuilt, src);
    }

    #[test]
    fn comment_span_covers_all_slashes_when_body_is_empty() {
        // Regression: an EMPTY `//`/`///` comment (at EOF or immediately before `\n`) must span every
        // prefix `/`, not just the first. `span_while` started its end at the first `/`, so with no
        // body char to extend it, the second/third `/` were left UNCOVERED — a gap in the span table
        // (which must be total: concatenated spans == source). Found by the arbitrary-input sweep.
        for src in ["//", "///", "//\n", "///\n", "a//", "b///\nc"] {
            let cov: String = Lexer::new(src)
                .map(|t| src[t.span.start..t.span.end].to_string())
                .collect();
            assert_eq!(cov, src, "spans must cover the whole source for {src:?}");
        }
        // The `//` still lexes as ONE line comment (trivia), `///` as one doc comment — the fix is
        // span-only, not a retokenization.
        assert_eq!(kinds("//"), Vec::<Kind>::new()); // pure trivia, filtered
        assert_eq!(
            Lexer::new("//").map(|t| t.kind).collect::<Vec<_>>(),
            vec![Kind::LineComment]
        );
        assert_eq!(
            Lexer::new("///").map(|t| t.kind).collect::<Vec<_>>(),
            vec![Kind::DocComment]
        );
    }

    #[test]
    fn words_are_ident_not_keywords() {
        assert_eq!(
            kinds("let if match true false and or else"),
            vec![Kind::Ident; 8]
        );
    }

    #[test]
    fn operators_maximal_munch() {
        assert_eq!(
            kinds("a <= b << c >= d >> e => f -> g"),
            vec![
                Kind::Ident,
                Kind::Le,
                Kind::Ident,
                Kind::Shl,
                Kind::Ident,
                Kind::Ge,
                Kind::Ident,
                Kind::Shr,
                Kind::Ident,
                Kind::FatArrow,
                Kind::Ident,
                Kind::Arrow,
                Kind::Ident,
            ]
        );
    }

    #[test]
    fn wrapping_operators() {
        assert_eq!(
            kinds("a +% b -% c *% d"),
            vec![
                Kind::Ident,
                Kind::PlusPct,
                Kind::Ident,
                Kind::MinusPct,
                Kind::Ident,
                Kind::StarPct,
                Kind::Ident,
            ]
        );
    }

    #[test]
    fn float_operators() {
        // The OCaml-style FP operators `+.`/`-.`/`*.`/`/.` lex as one token each — distinct from the
        // integer `+`/`-`/`*`/`/` and from member access / a float literal.
        assert_eq!(
            kinds("a +. b -. c *. d /. e"),
            vec![
                Kind::Ident,
                Kind::PlusDot,
                Kind::Ident,
                Kind::MinusDot,
                Kind::Ident,
                Kind::StarDot,
                Kind::Ident,
                Kind::SlashDot,
                Kind::Ident,
            ]
        );
        // A float operator does NOT swallow a following float LITERAL — `+.3.5` is `+.` then `3.5`.
        assert_eq!(
            kinds("a +.3.5"),
            vec![Kind::Ident, Kind::PlusDot, Kind::Float]
        );
        // The integer operators are UNCHANGED (no `.` suffix): `+`/`-`/`*`/`/` still tokenize plainly.
        assert_eq!(
            kinds("a + b - c * d / e"),
            vec![
                Kind::Ident,
                Kind::Plus,
                Kind::Ident,
                Kind::Minus,
                Kind::Ident,
                Kind::Star,
                Kind::Ident,
                Kind::Slash,
                Kind::Ident,
            ]
        );
        // Member access and a float literal are UNAFFECTED (the `.` there is not an operator suffix).
        assert_eq!(kinds("r.x"), vec![Kind::Ident, Kind::Dot, Kind::Ident]);
        assert_eq!(kinds("3.5"), vec![Kind::Float]);
        // `//` comments still win over `/.` (the comment check precedes the float-suffix check): a
        // trailing `// c` lexes as one comment (trivia, filtered by `kinds`), NOT `/` `/` or `/.` `c`
        // — so only the leading `a` remains.
        assert_eq!(kinds("a // c"), vec![Kind::Ident]);
    }

    #[test]
    fn kebab_vs_subtraction() {
        assert_eq!(spanned_text("byte-at"), vec![("byte-at", Kind::Ident)]);
        assert_eq!(kinds("a - b"), vec![Kind::Ident, Kind::Minus, Kind::Ident]);
        assert_eq!(spanned_text("a-b"), vec![("a-b", Kind::Ident)]);
    }

    #[test]
    fn numbers() {
        assert_eq!(spanned_text("42"), vec![("42", Kind::Int)]);
        assert_eq!(spanned_text("0x2A"), vec![("0x2A", Kind::Int)]);
        assert_eq!(spanned_text("0b1010"), vec![("0b1010", Kind::Int)]);
        assert_eq!(spanned_text("1_000_000"), vec![("1_000_000", Kind::Int)]);
        assert_eq!(spanned_text("1.5"), vec![("1.5", Kind::Float)]);
        assert_eq!(spanned_text("1.5e10"), vec![("1.5e10", Kind::Float)]);
        assert_eq!(spanned_text("1e-9"), vec![("1e-9", Kind::Float)]);
        assert_eq!(spanned_text("-42"), vec![("-42", Kind::Int)]);
    }

    #[test]
    fn type_suffix_spans_over_the_number_on_every_radix() {
        // A glued `N`/`R` TYPE SUFFIX is part of the number token so `classify_word` sees the whole
        // `<body><suffix>` and builds a `Suffixed` leaf. This must fire on the RADIX path (`0x…`/`0b…`) too,
        // not just decimal — the radix scan used to `return` before the suffix peel, so `0xFFN` lexed as
        // `0xFF` + a bare `N` word, which the ML quantity sugar mis-read as `(Qty.of 0xFF (Unit.of "N"))`
        // → CDZ0201, while `255N` suffixed correctly (a surface inconsistency; `f91a9001` advertised `0xFFN`).
        // DECIMAL (already worked):
        assert_eq!(spanned_text("255N"), vec![("255N", Kind::Int)]);
        assert_eq!(spanned_text("1.25R"), vec![("1.25R", Kind::Float)]);
        // RADIX (the fix): the suffix is glued into the one token.
        assert_eq!(spanned_text("0xFFN"), vec![("0xFFN", Kind::Int)]);
        assert_eq!(spanned_text("0b1010N"), vec![("0b1010N", Kind::Int)]);
        assert_eq!(spanned_text("0xFFR"), vec![("0xFFR", Kind::Int)]);
        assert_eq!(spanned_text("0xFF_FFN"), vec![("0xFF_FFN", Kind::Int)]);
        // A trailing letter that CONTINUES an identifier is NOT a suffix — `0xFFNx` keeps the whole
        // ident-continuation glued to the number so the token fails the numeric parse and is rejected
        // downstream (never silently a suffix + a stray `x`). The radix body absorbs the hex `F`s; the
        // `Nx` follows. `0xFFN` alone suffixes, but `0xFFNx` must not peel just the `N`.
        assert_eq!(
            spanned_text("0xFFNx"),
            vec![("0xFF", Kind::Int), ("Nx", Kind::Ident)]
        );
        // A SPACE before the letter stays a separate token (the ML `5 R` quantity-unit sugar), unaffected.
        assert_eq!(
            spanned_text("0xFF N"),
            vec![("0xFF", Kind::Int), ("N", Kind::Ident)]
        );
    }

    #[test]
    fn negative_vs_binary_minus() {
        assert_eq!(spanned_text("-42"), vec![("-42", Kind::Int)]);
        assert_eq!(
            spanned_text("x - 42"),
            vec![("x", Kind::Ident), ("-", Kind::Minus), ("42", Kind::Int)]
        );
    }

    #[test]
    fn comments_and_docs() {
        assert_eq!(
            spanned_text("a // c\nb"),
            vec![("a", Kind::Ident), ("b", Kind::Ident)]
        );
        let toks: Vec<_> = Lexer::new("// hello\n").collect();
        assert_eq!(toks[0].kind, Kind::LineComment);
        assert_eq!(
            &"// hello\n"[toks[0].span.start..toks[0].span.end],
            "// hello"
        );
        assert_eq!(
            Lexer::new("/// doc\n").next().unwrap().kind,
            Kind::DocComment
        );
    }

    #[test]
    fn backtick_name_and_string() {
        assert_eq!(spanned_text("`|`"), vec![("`|`", Kind::BacktickName)]);
        assert_eq!(spanned_text("\"hi\""), vec![("\"hi\"", Kind::Str)]);
        assert_eq!(spanned_text("\"a\\\"b\""), vec![("\"a\\\"b\"", Kind::Str)]);
    }

    #[test]
    fn quasiquote_sigils() {
        assert_eq!(
            kinds("`{ x }"),
            vec![Kind::Backtick, Kind::LBrace, Kind::Ident, Kind::RBrace]
        );
        assert_eq!(kinds(",x"), vec![Kind::Comma, Kind::Ident]);
        assert_eq!(kinds(",@xs"), vec![Kind::UnquoteSplice, Kind::Ident]);
    }

    #[test]
    fn annotation_and_pragma_sigils() {
        // A bare `@` is the annotation sigil; `@!` is the pragma sugar — glued, maximal-munch (the `@!` is
        // ONE token, not `@` then `!`), and distinct from the `,@` splice above.
        assert_eq!(kinds("@inline-never"), vec![Kind::At, Kind::Ident]);
        assert_eq!(kinds("@!default-float"), vec![Kind::AtBang, Kind::Ident]);
        // The `@` and `@!` spans are exact (one/two chars), so the printer's sigil round-trips.
        assert_eq!(
            spanned_text("@!key"),
            vec![("@!", Kind::AtBang), ("key", Kind::Ident)]
        );
    }

    #[test]
    fn hash_sigils_and_symbol_sugar() {
        // `#"…"` is a quoted symbol; `#name` is the unquoted symbol sugar (one `SymLit` spanning
        // `#` through the identifier, kebab included). `#{`/`#[`/bare `#` stay the sigil (`{`/`[`
        // are not ident-starts); a `#` before a non-ident (`#1`, `#+`) is a bare `Hash`.
        assert_eq!(
            spanned_text("#\"meter\""),
            vec![("#\"meter\"", Kind::SymLit)]
        );
        assert_eq!(spanned_text("#meter"), vec![("#meter", Kind::SymLit)]);
        assert_eq!(
            spanned_text("#map-insert"),
            vec![("#map-insert", Kind::SymLit)]
        );
        assert_eq!(
            spanned_text("#{"),
            vec![("#", Kind::Hash), ("{", Kind::LBrace)]
        );
        assert_eq!(
            spanned_text("#[]"),
            vec![
                ("#", Kind::Hash),
                ("[", Kind::LBracket),
                ("]", Kind::RBracket)
            ]
        );
        assert_eq!(
            spanned_text("#1"),
            vec![("#", Kind::Hash), ("1", Kind::Int)]
        );
        assert_eq!(
            spanned_text("#+"),
            vec![("#", Kind::Hash), ("+", Kind::Plus)]
        );
        // The sugar does NOT cross whitespace — `# x` is a bare `Hash` then the ident.
        assert_eq!(
            spanned_text("# x"),
            vec![("#", Kind::Hash), ("x", Kind::Ident)]
        );
    }

    #[test]
    fn dotted_member_is_separate_tokens() {
        assert_eq!(kinds("Sign.Neg"), vec![Kind::Ident, Kind::Dot, Kind::Ident]);
    }

    #[test]
    fn dotdot_is_one_token_a_lone_dot_is_member() {
        // `..` is the rest/spread marker (a single `DotDot`); a lone `.` is member access.
        assert_eq!(kinds(".."), vec![Kind::DotDot]);
        assert_eq!(kinds("."), vec![Kind::Dot]);
        assert_eq!(
            kinds("[x, .. rest]"),
            vec![
                Kind::LBracket,
                Kind::Ident,
                Kind::Comma,
                Kind::DotDot,
                Kind::Ident,
                Kind::RBracket
            ]
        );
        // A float's fractional `.` is consumed inside `number` (needs a digit after), so `1..n`
        // lexes `1` `..` `n` — the range/rest reading — not `1.` `.n`.
        assert_eq!(kinds("1..n"), vec![Kind::Int, Kind::DotDot, Kind::Ident]);
        // `...` is `..` then `.` (greedy two-char), harmless — no collection uses it.
        assert_eq!(kinds("..."), vec![Kind::DotDot, Kind::Dot]);
    }

    #[test]
    fn dotdoteq_is_the_closed_range_operator() {
        // `..=` is its OWN token (the closed-range operator `lo..=hi`), glued greedily from `..` + `=`.
        assert_eq!(kinds("..="), vec![Kind::DotDotEq]);
        // `1..=n` lexes `Int DotDotEq Ident` (the token boundary a closed-range grammar would consume).
        assert_eq!(kinds("1..=n"), vec![Kind::Int, Kind::DotDotEq, Kind::Ident]);
        // A `..` NOT followed by `=` stays the plain `DotDot` (the `=` glue is only after exactly `..`).
        assert_eq!(kinds("1..n"), vec![Kind::Int, Kind::DotDot, Kind::Ident]);
        // `..==` is `..=` then a lone `=` (greedy: `..=` wins, the trailing `=` is its own token).
        assert_eq!(kinds("..=="), vec![Kind::DotDotEq, Kind::Eq]);
        // `.. =` (a space between) is NOT `..=` — the `=` must be glued.
        assert_eq!(kinds(".. ="), vec![Kind::DotDot, Kind::Eq]);
        // `..=>` — the `=` glues to `..` first (`..=`), leaving `>` a lone `Gt`; `=>` does NOT re-form
        // across the token boundary (documents the greedy-`..=` precedence over a would-be `=>`).
        assert_eq!(kinds("..=>"), vec![Kind::DotDotEq, Kind::Gt]);
    }

    #[test]
    fn unterminated_string_is_error_not_panic() {
        assert_eq!(Lexer::new("\"oops").next().unwrap().kind, Kind::Error);
        assert_eq!(Lexer::new("`oops").next().unwrap().kind, Kind::Error);
    }

    #[test]
    fn bin_open_glues_only_without_whitespace() {
        // `b[` glued (no space) opens a binary literal — one `BinOpen` token, like `b"…"` beats `b` + string.
        assert_eq!(
            kinds("b[u8(1)]"),
            vec![
                Kind::BinOpen,
                Kind::Ident,
                Kind::LParen,
                Kind::Int,
                Kind::RParen,
                Kind::RBracket
            ]
        );
        assert_eq!(spanned_text("b[")[0], ("b[", Kind::BinOpen));
        // `b []` with a space is the ordinary name `b` then a list `[…]` — the glue does not cross space.
        assert_eq!(
            kinds("b [0]"),
            vec![Kind::Ident, Kind::LBracket, Kind::Int, Kind::RBracket]
        );
        // Only the identifier `b` triggers it: `ab[` is one ident then a bare `[`, and `b"` stays a byte str.
        assert_eq!(
            kinds("ab[0]"),
            vec![Kind::Ident, Kind::LBracket, Kind::Int, Kind::RBracket]
        );
        assert_eq!(kinds("b\"x\"")[0], Kind::ByteStr);
    }

    #[test]
    fn tagged_template_lexes_a_glued_ident_string() {
        // `tag"…"` — an identifier GLUED to a string — is one TaggedTemplate token (like `b"…"`).
        assert_eq!(kinds("jsx\"hi\""), vec![Kind::TaggedTemplate]);
        assert_eq!(kinds("id\"a b\""), vec![Kind::TaggedTemplate]);
        // A SPACE between the ident and the string is NOT a tagged template — a bare ident then a string.
        assert_eq!(kinds("jsx \"hi\""), vec![Kind::Ident, Kind::Str]);
        // `b"…"`/`#"…"` keep their own kinds (their arms precede the general ident arm).
        assert_eq!(kinds("b\"x\"")[0], Kind::ByteStr);
        assert_eq!(kinds("#\"m\"")[0], Kind::SymLit);
        // An unterminated body is an Error, not a TaggedTemplate.
        assert_eq!(kinds("jsx\"oops")[0], Kind::Error);
        // A hole `{…}` keeps the template as ONE token — including a `"` INSIDE the hole (a string
        // literal in the interpolated expression) which must NOT close the template early.
        assert_eq!(kinds("jsx\"a{x}b\""), vec![Kind::TaggedTemplate]);
        assert_eq!(kinds("t\"x{g(\"}\")}y\""), vec![Kind::TaggedTemplate]);
        // `{{`/`}}` brace escapes stay in the body (one token).
        assert_eq!(kinds("t\"a {{b}} c\""), vec![Kind::TaggedTemplate]);
        // An unterminated HOLE (open `{`, no close before the end) is an Error, not a TaggedTemplate.
        assert_eq!(kinds("t\"a{x")[0], Kind::Error);
    }

    #[test]
    fn never_panics_on_arbitrary_input() {
        // Hand-picked odd inputs (a quick smoke; the systematic sweep is below).
        for s in ["", "\0", "🎉", "\\", "```", "0x", "1e", "..", "@~$"] {
            let _ = Lexer::new(s).count();
        }
    }

    /// A tiny deterministic PRNG (SplitMix64) — reproducible fuzz without a dependency, matching the
    /// codec's house style (the crate stays "plain"; see `Cargo.toml`).
    struct SplitMix64(u64);
    impl SplitMix64 {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }
    }

    /// Drive the lexer over `src`, asserting the tokenizer's structural invariants hold on ANY input —
    /// the lexer must NEVER PANIC (it returns tokens, some `Kind::Error`, never crashes), and every
    /// token span must be a VALID slice of the source: in-bounds, on UTF-8 char boundaries, and the
    /// concatenation of all spans (trivia included) reproduces the source exactly (the span table's
    /// load-bearing totality invariant, `spans_cover_source_exactly` generalized to arbitrary input).
    /// Also feeds each token's text through the literal classification/unescape path a parser would
    /// call, so a panic there (bad number/escape/char) is caught too.
    fn assert_lex_invariants(src: &str) {
        let mut rebuilt = String::new();
        let mut prev_end = 0usize;
        for t in Lexer::new(src) {
            let (s, e) = (t.span.start, t.span.end);
            assert!(
                s <= e && e <= src.len(),
                "span {s}..{e} out of bounds for {src:?}"
            );
            assert!(
                src.is_char_boundary(s) && src.is_char_boundary(e),
                "span {s}..{e} not on a char boundary for {src:?}"
            );
            assert_eq!(
                s, prev_end,
                "spans must be contiguous (gap/overlap) for {src:?}"
            );
            prev_end = e;
            let text = &src[s..e];
            rebuilt.push_str(text);
            // Exercise the classification path a parser takes for this token kind — none may panic.
            match t.kind {
                Kind::Int | Kind::Float => {
                    let _ = crate::literal::classify_word(text);
                }
                Kind::Str => {
                    let _ = crate::literal::unescape_string_token(text);
                }
                Kind::ByteStr => {
                    let _ = crate::literal::unescape_byte_string_token(text);
                }
                Kind::SymLit => {
                    let _ = crate::literal::unescape_sym_token(text);
                }
                Kind::CharLit => {
                    let _ = crate::literal::char_leaf(text);
                }
                Kind::BacktickName => {
                    let _ = crate::literal::unescape_backtick_name(text);
                }
                _ => {}
            }
        }
        assert_eq!(
            prev_end,
            src.len(),
            "spans must cover to end of source for {src:?}"
        );
        assert_eq!(
            rebuilt, src,
            "concatenated spans must reproduce the source for {src:?}"
        );
    }

    #[test]
    fn lexer_invariants_hold_on_arbitrary_input() {
        // (a) Exhaustive over EVERY single byte 0..=255 that is valid UTF-8 on its own, plus a
        // representative multi-byte scalar from each UTF-8 length class — unicode in any position.
        for b in 0u8..=255 {
            let s = (b as char).to_string(); // a `char` is always a valid scalar; covers 0..=255
            assert_lex_invariants(&s);
        }
        for c in [
            'é', 'λ', '中', '🎉', '\u{200d}', '\u{feff}', '\u{0}', '\u{7f}',
        ] {
            assert_lex_invariants(&c.to_string());
            assert_lex_invariants(&format!("a{c}b"));
            assert_lex_invariants(&format!("\"{c}\""));
            assert_lex_invariants(&format!("#\"{c}\""));
        }
        // (b) Random strings drawn from an alphabet that stresses every lexer branch — the sigils that
        // start multi-char tokens, quote/escape/comment openers, digits + numeric affixes, and unicode.
        let alphabet: Vec<char> = "0123456789abcxEeNR._+-*/<>=|&^%@#!:;,()[]{}`\"\\\n \tλ中🎉"
            .chars()
            .collect();
        let mut rng = SplitMix64(0x0bad_c0de_dead_beef);
        for len in 0..=24usize {
            for _ in 0..200 {
                let s: String = (0..len)
                    .map(|_| alphabet[(rng.next() as usize) % alphabet.len()])
                    .collect();
                assert_lex_invariants(&s);
                // The full lex→parse pipeline must also never panic (the parser is the lexer's only
                // consumer; a diagnostic, never a crash, on arbitrary input).
                let _ = crate::parser::read_ml(&s);
            }
        }
        // (c) Deliberately truncated/odd literal openers — the classic panic bait (unterminated string,
        // a lone backslash escape at EOF, an incomplete numeric affix, a bare `#`/backtick).
        for s in [
            "\"", "\"\\", "\"\\x", "b\"", "b\"\\", "#\"", "#\"\\", "`", "``", "0x", "0b", "1e",
            "1e+", "1.", ".1", "1_", "0xZZ", "1N", "0.5R", "'", "'\\", "\\", "//", "/*",
        ] {
            assert_lex_invariants(s);
            let _ = crate::parser::read_ml(s);
        }
    }

    #[test]
    fn glued_literal_forms_lex_as_one_token_over_generated_bodies() {
        // The GLUED literal forms — `b"…"` (ByteStr), `#"…"` (SymLit), `<tag>"…"` (TaggedTemplate) — must
        // each lex as EXACTLY ONE token spanning the whole construct, over bodies rich in the boundary
        // chars that stress the escape-/brace-aware body scan: an escaped quote `\"` must NOT close the
        // literal early, a `\\` must consume its pair, and (tagged-template only) a `"` inside a `{…}` hole
        // must NOT close it. The arbitrary-input sweep hits these forms only rarely/degenerately (the
        // random alphabet seldom emits a full `b"…"` with a well-formed escaped body); this constructs
        // them densely. `assert_lex_invariants` already pins span reconstruction + no-panic; here we ALSO
        // assert the single-token-of-the-expected-KIND property (an early split/over-consume shows as a
        // wrong token count or kind).
        // A body alphabet weighted to the escape-significant chars + unicode. `\"` and `\\` are emitted as
        // 2-char escape UNITS so the generated body is always well-formed (even count of backslashes).
        let units: &[&str] = &[
            "a", "b", " ", "\t", "x", "1", "λ", "中", // ordinary body chars
            "\\\"", "\\\\",
            "\\n", // escape units (backslash-quote, backslash-backslash, backslash-n)
            "{{", "}}", // template brace escapes (harmless in b"/#" too — just chars there)
        ];
        let mut r = SplitMix64(0x91ed_c0de_1a7e_5eed);
        let gen_body = |r: &mut SplitMix64, n: usize| -> String {
            (0..n)
                .map(|_| units[(r.next() as usize) % units.len()])
                .collect()
        };
        for _ in 0..4000 {
            let n = (r.next() % 6) as usize;
            let body = gen_body(&mut r, n);
            // b"<body>" → one ByteStr; #"<body>" → one SymLit; id"<body>" → one TaggedTemplate.
            for (src, want) in [
                (format!("b\"{body}\""), Kind::ByteStr),
                (format!("#\"{body}\""), Kind::SymLit),
                (format!("id\"{body}\""), Kind::TaggedTemplate),
            ] {
                let ks = kinds(&src);
                assert_eq!(
                    ks,
                    vec![want],
                    "glued form {src:?} must be exactly one {want:?} token, got {ks:?}"
                );
                // The single token spans the ENTIRE source (no early split / over-consume).
                let st = spanned_text(&src);
                assert_eq!(
                    st,
                    vec![(src.as_str(), want)],
                    "glued form {src:?} token must span all of it"
                );
            }
        }
        // Tagged-template HOLES with an embedded `"` (a string literal in the interpolated expr) must NOT
        // close the template early — construct `tag"…{ <hole-with-a-quoted-string> }…"` and assert one token.
        for hole in [
            "x",
            "g(\"}\")",     // a `}` inside a string inside the hole
            "f(\"a{b}c\")", // braces + quotes inside the hole
            "h(\"\\\"\")",  // an escaped quote inside the hole's string
        ] {
            let src = format!("t\"pre{{{hole}}}post\"");
            assert_eq!(
                kinds(&src),
                vec![Kind::TaggedTemplate],
                "a hole with an embedded string must keep the template one token: {src:?}"
            );
        }
    }
}
