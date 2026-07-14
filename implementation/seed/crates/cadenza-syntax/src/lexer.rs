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
            // `..` is the rest/spread marker (one token); a lone `.` is member access. A float's
            // fractional `.` is consumed inside `number` (it needs a digit after the `.`), so it never
            // reaches here — `1..n` therefore lexes `1` `..` `n`, not `1.` `.n`.
            '.' if self.peek() == Some('.') => {
                let b = self.bump().unwrap();
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
            self.bump(); // second '/'
            let doc = self.peek() == Some('/');
            if doc {
                self.bump(); // third '/'
            }
            let span = self.span_while(a, |c| c != '\n');
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

    fn read_backtick_name(&mut self, a: Char) -> Token {
        let mut end = a.span;
        loop {
            match self.bump() {
                None => {
                    return Token {
                        kind: Kind::Error,
                        span: a.span.merge(end),
                    };
                } // unterminated
                Some(c) if c.value == '`' => {
                    return Token {
                        kind: Kind::BacktickName,
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

    fn read_string(&mut self, a: Char) -> Token {
        let mut end = a.span;
        loop {
            match self.bump() {
                None => {
                    return Token {
                        kind: Kind::Error,
                        span: a.span.merge(end),
                    };
                } // unterminated
                Some(c) if c.value == '"' => {
                    return Token {
                        kind: Kind::Str,
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
        Token {
            kind: Kind::Ident,
            span: a.span.merge(end),
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
    fn never_panics_on_arbitrary_input() {
        // A cheap stand-in for the fuzz test: drive the lexer over odd byte sequences.
        for s in ["", "\0", "🎉", "\\", "```", "0x", "1e", "..", "@~$"] {
            let _ = Lexer::new(s).count();
        }
    }
}
