use crate::{
    iter::{Char, Chars, Peek2},
    token::{Kind, Token},
};

#[derive(Clone, Copy, Debug, Default)]
enum Mode {
    #[default]
    Normal,
    StringContent,
    StringEnd,
    Comment,
    DocComment,
}

/// Returns true if the character can be part of an identifier.
///
/// Identifiers include:
/// - Alphanumeric characters (letters and digits in any script)
/// - Underscores (`_`)
/// - Any non-ASCII, non-whitespace character (including emoji, symbols, etc.)
///
/// This permissive approach allows for expressive identifiers like:
/// - `hello_world` (snake_case)
/// - `αβγ` (Greek letters)
/// - `hello🎉world` (emoji mixed with text)
/// - `你好` (Chinese characters)
fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_alphanumeric() || (!c.is_ascii() && !c.is_whitespace())
}

pub struct Lexer<'a> {
    chars: Peek2<Chars<'a>>,
    mode: Mode,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Self {
            chars: Peek2::new(Chars::new(src)),
            mode: Mode::default(),
        }
    }

    fn read_while(&mut self, start: Char, mut pred: impl FnMut(&Char) -> bool) -> Char {
        let mut end = start;
        while let Some(v) = self.chars.next_if(&mut pred) {
            end = v;
        }
        end
    }

    fn read_until_newline(&mut self, start: Char) -> Char {
        self.read_while(start, |v| !['\r', '\n'].contains(&v.value))
    }

    fn next_token(&mut self) -> Option<Token> {
        if matches!(self.mode, Mode::StringContent) {
            // check if the string is empty
            if let Some(a) = self.chars.peek().filter(|v| *v == '"') {
                self.mode = Mode::StringEnd;
                let mut span = a.span;
                span.end = span.start;
                return Some(Kind::StringContent.spanned(span));
            }
        }

        let a = self.chars.next()?;

        match self.mode {
            Mode::Normal => {}
            Mode::StringContent => {
                self.mode = Mode::StringEnd;
                let mut escape = false;
                let mut has_escape = false;
                let end = self.read_while(a, |v| {
                    // if we're escaping then always return
                    if core::mem::take(&mut escape) {
                        return true;
                    }
                    if v.value == '\\' {
                        escape = true;
                        has_escape = true;
                    }
                    v.value != '"'
                });

                let kind = if has_escape {
                    Kind::StringContentWithEscape
                } else {
                    Kind::StringContent
                };

                return Some(kind.spanned((a, end)));
            }
            Mode::StringEnd => {
                self.mode = Mode::Normal;
                debug_assert!(a.value == '"');
                return Some(Kind::StringEnd.spanned(a));
            }
            Mode::Comment => {
                self.mode = Mode::Normal;
                let end = self.read_until_newline(a);
                return Some(Kind::CommentContent.spanned((a, end)));
            }
            Mode::DocComment => {
                self.mode = Mode::Normal;
                let end = self.read_until_newline(a);
                return Some(Kind::DocCommentContent.spanned((a, end)));
            }
        }

        Some(match a.value {
            '!' => match self.chars.next_if_eq('=') {
                Some(b) => Kind::BangEqual.spanned((a, b)),
                _ => Kind::Bang.spanned(a),
            },
            '=' => {
                if let Some(b) = self.chars.next_if_eq('=') {
                    Kind::EqualEqual.spanned((a, b))
                } else if let Some(b) = self.chars.next_if_eq('>') {
                    Kind::FatArrow.spanned((a, b))
                } else {
                    Kind::Equal.spanned(a)
                }
            }
            '<' => {
                if let Some(b) = self.chars.next_if_eq('=') {
                    Kind::LessEqual.spanned((a, b))
                } else if let Some(b) = self.chars.next_if_eq('<') {
                    // Check for <<=
                    if let Some(c) = self.chars.next_if_eq('=') {
                        Kind::LessLessEqual.spanned((a, c))
                    } else {
                        Kind::LessLess.spanned((a, b))
                    }
                } else if let Some(b) = self.chars.next_if_eq('-') {
                    Kind::LeftArrow.spanned((a, b))
                } else {
                    Kind::Less.spanned(a)
                }
            }
            '>' => {
                if let Some(b) = self.chars.next_if_eq('=') {
                    Kind::GreaterEqual.spanned((a, b))
                } else if let Some(b) = self.chars.next_if_eq('>') {
                    // Check for >>=
                    if let Some(c) = self.chars.next_if_eq('=') {
                        Kind::GreaterGreaterEqual.spanned((a, c))
                    } else {
                        Kind::GreaterGreater.spanned((a, b))
                    }
                } else {
                    Kind::Greater.spanned(a)
                }
            }
            '+' => match self.chars.next_if_eq('=') {
                Some(b) => Kind::PlusEqual.spanned((a, b)),
                None => Kind::Plus.spanned(a),
            },
            '-' => {
                if let Some(b) = self.chars.next_if_eq('=') {
                    Kind::MinusEqual.spanned((a, b))
                } else if let Some(b) = self.chars.next_if_eq('>') {
                    Kind::RightArrow.spanned((a, b))
                } else {
                    Kind::Minus.spanned(a)
                }
            }
            '*' => {
                if let Some(b) = self.chars.next_if_eq('=') {
                    Kind::StarEqual.spanned((a, b))
                } else if let Some(b) = self.chars.next_if_eq('*') {
                    Kind::StarStar.spanned((a, b))
                } else {
                    Kind::Star.spanned(a)
                }
            }
            '/' => {
                if let Some(b) = self.chars.next_if_eq('=') {
                    Kind::SlashEqual.spanned((a, b))
                } else {
                    Kind::Slash.spanned(a)
                }
            }
            '"' => {
                self.mode = Mode::StringContent;
                Kind::StringStart.spanned(a)
            }
            '&' => {
                if let Some(b) = self.chars.next_if_eq('=') {
                    Kind::AmpersandEqual.spanned((a, b))
                } else if let Some(b) = self.chars.next_if_eq('&') {
                    Kind::AmpersandAmpersand.spanned((a, b))
                } else {
                    Kind::Ampersand.spanned(a)
                }
            }
            '%' => match self.chars.next_if_eq('=') {
                Some(b) => Kind::PercentEqual.spanned((a, b)),
                None => Kind::Percent.spanned(a),
            },
            '^' => match self.chars.next_if_eq('=') {
                Some(b) => Kind::CaretEqual.spanned((a, b)),
                None => Kind::Caret.spanned(a),
            },
            '$' => {
                if let Some(b) = self.chars.next_if_eq('{') {
                    Kind::LDollarBrace.spanned((a, b))
                } else {
                    Kind::Dollar.spanned(a)
                }
            }
            '?' => Kind::Question.spanned(a),
            '\\' => Kind::Backslash.spanned(a),
            '(' => Kind::LParen.spanned(a),
            ')' => Kind::RParen.spanned(a),
            '{' => Kind::LBrace.spanned(a),
            '}' => Kind::RBrace.spanned(a),
            '[' => Kind::LBracket.spanned(a),
            ']' => Kind::RBracket.spanned(a),
            ';' => Kind::Semicolon.spanned(a),
            ',' => Kind::Comma.spanned(a),
            '.' => {
                if let Some(b) = self.chars.next_if_eq('.') {
                    // Check for ... or ..=
                    if let Some(c) = self.chars.next_if_eq('.') {
                        Kind::DotDotDot.spanned((a, c))
                    } else if let Some(c) = self.chars.next_if_eq('=') {
                        Kind::DotDotEqual.spanned((a, c))
                    } else {
                        Kind::DotDot.spanned((a, b))
                    }
                } else {
                    Kind::Dot.spanned(a)
                }
            }
            ':' => match self.chars.next_if_eq(':') {
                Some(b) => Kind::ColonColon.spanned((a, b)),
                None => Kind::Colon.spanned(a),
            },
            '@' => Kind::At.spanned(a),
            '`' => Kind::Backtick.spanned(a),
            '#' => {
                if let Some(b) = self.chars.next_if_eq('#') {
                    let token = Kind::DocCommentStart;
                    self.mode = Mode::DocComment;
                    token.spanned((a, b))
                } else {
                    self.mode = Mode::Comment;
                    Kind::CommentStart.spanned(a)
                }
            }
            '~' => Kind::Tilde.spanned(a),
            '\'' => Kind::SingleQuote.spanned(a),
            '|' => {
                if let Some(b) = self.chars.next_if_eq('=') {
                    Kind::PipeEqual.spanned((a, b))
                } else if let Some(b) = self.chars.next_if_eq('|') {
                    Kind::PipePipe.spanned((a, b))
                } else if let Some(b) = self.chars.next_if_eq('>') {
                    Kind::PipeGreater.spanned((a, b))
                } else if let Some(b) = self.chars.next_if_eq('?') {
                    Kind::PipeQuestion.spanned((a, b))
                } else {
                    Kind::Pipe.spanned(a)
                }
            }
            ' ' => {
                let end = self.read_while(a, |v| v == ' ');
                Kind::Space.spanned((a, end))
            }
            '\t' => {
                let end = self.read_while(a, |v| v == '\t');
                Kind::Tab.spanned((a, end))
            }
            '\n' => Kind::Newline.spanned(a),
            '\r' => match self.chars.next_if(|c| c.value == '\n') {
                Some(b) => Kind::Newline.spanned((a, b)),
                _ => Kind::Newline.spanned(a),
            },
            c if c.is_ascii_digit() => {
                let mut token = Kind::Integer;
                let mut seen_dot = false;

                // TODO support E-notation
                let end = self.read_while(a, |v| {
                    if v.value == '.' {
                        if seen_dot {
                            return false;
                        }
                        seen_dot = true;
                        token = Kind::Float;
                        return true;
                    }
                    v.value.is_ascii_digit() || v.value == '_'
                });

                token.spanned((a, end))
            }
            _ => {
                let end = self.read_while(a, |v| is_ident_continue(v.value));
                let span = a.span.merge(end.span);
                Kind::Identifier.spanned(span)
            }
        })
    }
}

impl Iterator for Lexer<'_> {
    type Item = Token;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_token()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::lex;
    use bolero::check;

    #[test]
    fn all_tokens_test() {
        for kind in Kind::ALL {
            // This will be the same character as StringStart so skip in this test
            if matches!(kind, Kind::StringEnd) {
                continue;
            }

            let Some(s) = kind.as_str() else {
                continue;
            };
            let mut tokens = lex(s);
            assert_eq!(tokens.len(), 1);
            let token = tokens.pop().unwrap();
            assert_eq!(token.kind, *kind);
        }
    }

    /// Shows that any punctuation finishes the identifier
    #[test]
    fn ident_punct() {
        for kind in Kind::ALL {
            // This will be the same character as StringStart so skip in this test
            if matches!(kind, Kind::StringEnd) {
                continue;
            }

            let Some(s) = kind.as_str() else {
                continue;
            };

            let s = format!("a{s}");
            let tokens = lex(&s);
            assert_eq!(tokens.len(), 2, "{s:?} -> {tokens:?}");
            assert_eq!(tokens[0].kind, Kind::Identifier);
            assert_eq!(tokens[1].kind, *kind);
        }
    }

    #[test]
    fn fuzz_test() {
        check!().for_each(|bytes| {
            let Some(s) = std::str::from_utf8(bytes).ok() else {
                return;
            };
            let mut tokens = Lexer::new(s);
            while tokens.next().is_some() {}
        });
    }
}
