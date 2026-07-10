//! The parser — a Pratt parser that builds the canonical [`Arenas`] AST DIRECTLY (no intermediate
//! syntax tree), plus a [`SpanTable`] mapping each structure occurrence to its source span.
//!
//! Parsing *is* lowering: each grammar production emits arena node-ids in the same `(head child…)`
//! shape the s-expression surface produces, so the two surfaces yield structurally-equal arenas.
//! It is NON-whitespace-significant. Keywords are contextual: the lexer emits `Ident`, and this
//! parser decides via [`crate::token::keyword`] whether an identifier begins a `let`/`if`/`fn`/
//! `match` form; `and`/`or` are word-spelled infix operators. A backtick name escapes any reserved
//! word so it can still be an ordinary name.
//!
//! Every structure node built here pushes exactly one span (in id order) into the `SpanTable`, so
//! the table stays total and 1:1 with occurrences.

use crate::ast::{Arenas, Builder, Leaf, StructId};
use crate::lexer::{Lexer, Token};
use crate::literal;
use crate::span::Span;
use crate::spans::{FileId, SpanTable};
use crate::token::{infix_prec, keyword, word_op, Keyword, Kind};

/// A parse error: a message anchored to a source span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub span: Span,
    pub message: String,
}

/// The result of parsing: the canonical arenas, the span table, and any recovered errors. The
/// arenas are always well-formed (`root` is valid) even when `errors` is non-empty — error recovery
/// substitutes `Name` placeholders rather than leaving holes.
pub struct Parsed {
    pub arenas: Arenas,
    pub spans: SpanTable,
    pub errors: Vec<ParseError>,
}

impl Parsed {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Parse `src` (in file `file`) to arenas + spans.
pub fn parse(src: &str, file: FileId) -> Parsed {
    let tokens: Vec<Token> = Lexer::new(src).filter(|t| !t.kind.is_trivia()).collect();
    let mut p = Parser {
        src,
        tokens,
        pos: 0,
        builder: Builder::new(),
        spans: SpanTable::new(file),
        errors: Vec::new(),
    };
    let root = p.program();
    Parsed { arenas: p.builder.finish(root), spans: p.spans, errors: p.errors }
}

/// Parse `src` as an anonymous single file (`FileId(0)`).
pub fn read_ml(src: &str) -> Parsed {
    parse(src, FileId::default())
}

struct Parser<'a> {
    src: &'a str,
    tokens: Vec<Token>,
    pos: usize,
    builder: Builder,
    spans: SpanTable,
    errors: Vec<ParseError>,
}

impl<'a> Parser<'a> {
    // ---- arena helpers (each records a span, keeping SpanTable 1:1 with structure ids) ----

    /// Push an `Atom` occurrence of `leaf` with source `span`.
    fn atom(&mut self, leaf: Leaf, span: Span) -> StructId {
        let id = self.builder.atom_leaf(leaf);
        self.spans.push(span);
        id
    }

    /// Push a `List` occurrence spanning `span`.
    fn list(&mut self, children: Vec<StructId>, span: Span) -> StructId {
        let id = self.builder.list(children);
        self.spans.push(span);
        id
    }

    /// An `Atom` of a `Name` with source `span`.
    fn name(&mut self, name: impl Into<String>, span: Span) -> StructId {
        self.atom(Leaf::Name(name.into()), span)
    }

    // ---- token cursor ----

    fn tok(&self) -> Option<Token> {
        self.tokens.get(self.pos).copied()
    }
    fn kind(&self) -> Kind {
        self.tok().map(|t| t.kind).unwrap_or(Kind::Error)
    }
    fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }
    fn text(&self, t: Token) -> &'a str {
        &self.src[t.span.start..t.span.end]
    }
    fn cur_text(&self) -> &'a str {
        self.tok().map(|t| self.text(t)).unwrap_or("")
    }
    fn cur_span(&self) -> Span {
        self.tok().map(|t| t.span).unwrap_or(Span::new(self.src.len(), self.src.len()))
    }
    fn bump(&mut self) -> Option<Token> {
        let t = self.tok();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }
    fn at(&self, k: Kind) -> bool {
        self.kind() == k
    }
    fn at_keyword(&self, kw: Keyword) -> bool {
        self.at(Kind::Ident) && keyword(self.cur_text()) == Some(kw)
    }
    /// Consume `k` if present; otherwise record an error and do not advance.
    fn expect(&mut self, k: Kind, what: &str) {
        if self.at(k) {
            self.bump();
        } else {
            self.error(&format!("expected {what}"));
        }
    }
    fn expect_keyword(&mut self, kw: Keyword, what: &str) {
        if self.at_keyword(kw) {
            self.bump();
        } else {
            self.error(&format!("expected {what}"));
        }
    }
    fn error(&mut self, message: &str) {
        self.errors.push(ParseError { span: self.cur_span(), message: message.to_string() });
    }

    /// A synthetic error placeholder occurrence (a name `<error>`), used when a production cannot
    /// produce a real node so the arena stays well-formed.
    fn error_node(&mut self, span: Span) -> StructId {
        self.name("<error>", span)
    }

    // ---- program ----

    fn program(&mut self) -> StructId {
        if self.at_end() {
            let span = Span::new(0, 0);
            self.error("empty program");
            return self.error_node(span);
        }
        let root = self.expr(0);
        if !self.at_end() {
            self.error("unexpected trailing input");
            // Consume the rest so we don't loop; the already-built root is returned.
            while !self.at_end() {
                self.bump();
            }
        }
        root
    }

    // ---- expression grammar (Pratt) ----

    /// Parse an expression whose infix operators bind at least `min_prec`.
    fn expr(&mut self, min_prec: u8) -> StructId {
        let start = self.cur_span();
        let mut left = self.prefix();
        left = self.postfix(left, start);
        loop {
            let op_name = match self.infix_op() {
                Some(name) => name,
                None => break,
            };
            let prec = infix_prec(op_name).expect("infix_op returns only infix names");
            if prec < min_prec {
                break;
            }
            let op_span = self.cur_span();
            self.bump(); // operator
            let head = self.name(op_name, op_span);
            let right = self.expr(prec + 1); // left-assoc: right binds one tighter
            let span = start.merge(self.prev_span());
            left = self.list(vec![head, left, right], span);
        }
        left
    }

    /// The infix operator name at the cursor (symbolic kind or word-op ident), or `None`.
    fn infix_op(&self) -> Option<&'static str> {
        match self.kind() {
            Kind::Ident => word_op(self.cur_text()),
            k => k.op_str(),
        }
    }

    /// Prefix position.
    fn prefix(&mut self) -> StructId {
        let span = self.cur_span();
        match self.kind() {
            Kind::Int | Kind::Float => {
                let t = self.bump().unwrap();
                self.atom(literal::classify_word(self.text(t)), span)
            }
            Kind::Str => {
                let t = self.bump().unwrap();
                self.atom(Leaf::Str(literal::unescape_string_token(self.text(t))), span)
            }
            Kind::BacktickName => {
                let t = self.bump().unwrap();
                self.name(literal::unescape_backtick_name(self.text(t)), span)
            }
            Kind::Ident => match keyword(self.cur_text()) {
                Some(Keyword::Let) => self.let_expr(),
                Some(Keyword::If) => self.if_expr(),
                Some(Keyword::Fn) => self.fn_expr(),
                Some(Keyword::Match) => self.match_expr(),
                Some(_) => {
                    // `in`/`then`/`else` bare in prefix position is an error; keep the ident as a
                    // name so we make progress and the arena stays well-formed.
                    self.error("keyword used outside its form");
                    let t = self.bump().unwrap();
                    self.name(self.text(t), span)
                }
                None => {
                    // A plain name (may be a bool/number-shaped word — classify_word decides).
                    let t = self.bump().unwrap();
                    self.atom(literal::classify_word(self.text(t)), span)
                }
            },
            Kind::LParen => self.paren(),
            Kind::Backtick => self.quasiquote(),
            Kind::Comma => self.unquote("unquote"),
            Kind::UnquoteSplice => self.unquote("unquote-splicing"),
            Kind::Hash => self.hash_list(),
            _ => {
                self.error("expected an expression");
                if !self.at_end() {
                    self.bump();
                }
                self.error_node(span)
            }
        }
    }

    /// Postfix chain: `.member` and `(args…)` application, tightest, left-nested.
    fn postfix(&mut self, mut node: StructId, start: Span) -> StructId {
        loop {
            match self.kind() {
                Kind::Dot if self.dot_is_member() => {
                    self.bump(); // '.'
                    let key_span = self.cur_span();
                    let key = match self.kind() {
                        Kind::Ident => {
                            let t = self.bump().unwrap();
                            self.name(self.text(t), key_span)
                        }
                        Kind::BacktickName => {
                            let t = self.bump().unwrap();
                            self.name(literal::unescape_backtick_name(self.text(t)), key_span)
                        }
                        _ => {
                            self.error("expected a member name after `.`");
                            self.error_node(key_span)
                        }
                    };
                    let dot_span = start.merge(self.prev_span());
                    let dot = self.name(".", dot_span);
                    node = self.list(vec![dot, node, key], dot_span);
                }
                Kind::LParen => {
                    let args = self.arg_exprs();
                    let span = start.merge(self.prev_span());
                    let mut items = Vec::with_capacity(args.len() + 1);
                    items.push(node);
                    items.extend(args);
                    node = self.list(items, span);
                }
                _ => break,
            }
        }
        node
    }

    /// A `.` begins member access only when followed by a member key.
    fn dot_is_member(&self) -> bool {
        matches!(self.nth_kind(1), Kind::Ident | Kind::BacktickName)
    }

    fn nth_kind(&self, n: usize) -> Kind {
        self.tokens.get(self.pos + n).map(|t| t.kind).unwrap_or(Kind::Error)
    }

    /// Parse `( e, … )` and return the argument occurrences.
    fn arg_exprs(&mut self) -> Vec<StructId> {
        self.expect(Kind::LParen, "`(`");
        let mut args = Vec::new();
        if !self.at(Kind::RParen) {
            loop {
                args.push(self.expr(0));
                if self.at(Kind::Comma) {
                    self.bump();
                } else {
                    break;
                }
            }
        }
        self.expect(Kind::RParen, "`)`");
        args
    }

    /// `( expr )` grouping, or `()` the unit form.
    fn paren(&mut self) -> StructId {
        let start = self.cur_span();
        self.expect(Kind::LParen, "`(`");
        if self.at(Kind::RParen) {
            self.bump();
            let span = start.merge(self.prev_span());
            return self.name("unit", span);
        }
        let inner = self.expr(0);
        self.expect(Kind::RParen, "`)`");
        inner // grouping is transparent in the arena
    }

    // ---- keyword forms ----

    /// `let n = e, … in body`  ->  `(let ((n e) …) body)`
    fn let_expr(&mut self) -> StructId {
        let start = self.cur_span();
        let let_head = self.keyword_head("let", start);
        self.bump(); // `let`
        let mut bindings = Vec::new();
        loop {
            let b_start = self.cur_span();
            let n = self.binder();
            self.expect(Kind::Eq, "`=`");
            let e = self.expr(0);
            let b_span = b_start.merge(self.prev_span());
            bindings.push(self.list(vec![n, e], b_span));
            if self.at(Kind::Comma) {
                self.bump();
            } else {
                break;
            }
        }
        let binds_span = start.merge(self.prev_span());
        let binds = self.list(bindings, binds_span);
        self.expect_keyword(Keyword::In, "`in`");
        let body = self.expr(0);
        let span = start.merge(self.prev_span());
        self.list(vec![let_head, binds, body], span)
    }

    /// `if c then t else e`  ->  `(if c t e)`
    fn if_expr(&mut self) -> StructId {
        let start = self.cur_span();
        let head = self.keyword_head("if", start);
        self.bump(); // `if`
        let c = self.expr(0);
        self.expect_keyword(Keyword::Then, "`then`");
        let t = self.expr(0);
        self.expect_keyword(Keyword::Else, "`else`");
        let e = self.expr(0);
        let span = start.merge(self.prev_span());
        self.list(vec![head, c, t, e], span)
    }

    /// `fn(p, …) => body`  ->  `(fn (p …) body)`
    fn fn_expr(&mut self) -> StructId {
        let start = self.cur_span();
        let head = self.keyword_head("fn", start);
        self.bump(); // `fn`
        let params_start = self.cur_span();
        self.expect(Kind::LParen, "`(`");
        let mut params = Vec::new();
        if !self.at(Kind::RParen) {
            loop {
                params.push(self.binder());
                if self.at(Kind::Comma) {
                    self.bump();
                } else {
                    break;
                }
            }
        }
        self.expect(Kind::RParen, "`)`");
        let params_span = params_start.merge(self.prev_span());
        let param_list = self.list(params, params_span);
        self.expect(Kind::FatArrow, "`=>`");
        let body = self.expr(0);
        let span = start.merge(self.prev_span());
        self.list(vec![head, param_list, body], span)
    }

    /// `match scrut { pat [if g] => body, … }`  ->  `(match scrut (pat body) …)`, where a guarded
    /// pattern is `(guard <pat> <expr>)`.
    fn match_expr(&mut self) -> StructId {
        let start = self.cur_span();
        let head = self.keyword_head("match", start);
        self.bump(); // `match`
        let scrut = self.expr(0);
        let mut items = vec![head, scrut];
        self.expect(Kind::LBrace, "`{`");
        while !self.at(Kind::RBrace) && !self.at_end() {
            items.push(self.match_arm());
            if self.at(Kind::Comma) {
                self.bump();
            } else {
                break;
            }
        }
        self.expect(Kind::RBrace, "`}`");
        let span = start.merge(self.prev_span());
        self.list(items, span)
    }

    /// One arm: `pattern [if guard] => body`  ->  `(pattern body)`.
    fn match_arm(&mut self) -> StructId {
        let start = self.cur_span();
        let mut pat = self.pattern();
        if self.at_keyword(Keyword::If) {
            let g_start = self.cur_span();
            let guard_head = self.keyword_head("guard", g_start);
            self.bump(); // `if`
            let g = self.expr(0);
            let g_span = g_start.merge(self.prev_span());
            // (guard <pat> <expr>) — keeps the arm head a single pattern occurrence.
            pat = self.list(vec![guard_head, pat, g], g_span);
        }
        self.expect(Kind::FatArrow, "`=>`");
        let body = self.expr(0);
        let span = start.merge(self.prev_span());
        self.list(vec![pat, body], span)
    }

    // ---- structural pattern grammar ----

    /// A structural pattern occurrence. A pattern's tree is a plain `(head child…)` form (the same
    /// shape the pattern printer emits): a head atom — a literal, a binding/wildcard name, a
    /// backtick name, or a grouped sub-pattern — followed by an optional `.member` chain and/or a
    /// `(sub-pattern, …)` application, left-nested. It is never an infix expression. This mirrors
    /// the printer exactly, so constructor patterns (`Some(x)`), dotted constructors (`Sign.Neg`),
    /// literal-headed forms (`1(v)`), and quoted patterns (`quasiquote(…)`) all parse uniformly.
    fn pattern(&mut self) -> StructId {
        let start = self.cur_span();
        let mut node = self.pattern_atom();
        loop {
            match self.kind() {
                Kind::Dot if matches!(self.nth_kind(1), Kind::Ident | Kind::BacktickName) => {
                    self.bump(); // '.'
                    let seg_span = self.cur_span();
                    let seg_t = self.bump().unwrap();
                    let seg = match seg_t.kind {
                        Kind::BacktickName => {
                            self.name(literal::unescape_backtick_name(self.text(seg_t)), seg_span)
                        }
                        _ => self.name(self.text(seg_t), seg_span),
                    };
                    let dot_span = start.merge(self.prev_span());
                    let dot = self.name(".", dot_span);
                    node = self.list(vec![dot, node, seg], dot_span);
                }
                Kind::LParen => {
                    self.bump();
                    let mut items = vec![node];
                    if !self.at(Kind::RParen) {
                        loop {
                            items.push(self.pattern());
                            if self.at(Kind::Comma) {
                                self.bump();
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(Kind::RParen, "`)`");
                    let span = start.merge(self.prev_span());
                    node = self.list(items, span);
                }
                _ => break,
            }
        }
        node
    }

    /// The head atom of a pattern (before any `.member` / application postfix).
    fn pattern_atom(&mut self) -> StructId {
        let span = self.cur_span();
        match self.kind() {
            Kind::Int | Kind::Float => {
                let t = self.bump().unwrap();
                self.atom(literal::classify_word(self.text(t)), span)
            }
            Kind::Str => {
                let t = self.bump().unwrap();
                self.atom(Leaf::Str(literal::unescape_string_token(self.text(t))), span)
            }
            Kind::BacktickName => {
                let t = self.bump().unwrap();
                self.name(literal::unescape_backtick_name(self.text(t)), span)
            }
            Kind::Ident => {
                let t = self.bump().unwrap();
                let word = self.text(t);
                // A word that heads a `.member` chain or an application is a constructor NAME; a
                // bare word that is a literal in shape (`true`/`false`/number) is a LITERAL pattern
                // (matching the oracle); any other bare word is a binding/wildcard name.
                if self.at(Kind::Dot) || self.at(Kind::LParen) {
                    self.name(word, span)
                } else {
                    self.atom(literal::classify_word(word), span)
                }
            }
            Kind::LParen => {
                self.bump();
                let inner = if self.at(Kind::RParen) {
                    let s = self.cur_span();
                    self.name("unit", s)
                } else {
                    self.pattern()
                };
                self.expect(Kind::RParen, "`)`");
                inner
            }
            _ => {
                self.error("expected a pattern");
                if !matches!(self.kind(), Kind::FatArrow | Kind::RBrace) && !self.at_end() {
                    self.bump();
                }
                self.error_node(span)
            }
        }
    }

    // ---- quasiquote / unquote sigils ----

    /// `` `{ expr } ``  ->  `(quasiquote expr)`
    fn quasiquote(&mut self) -> StructId {
        let start = self.cur_span();
        let head = self.name("quasiquote", start);
        self.bump(); // backtick
        self.expect(Kind::LBrace, "`{`");
        let inner = self.expr(0);
        self.expect(Kind::RBrace, "`}`");
        let span = start.merge(self.prev_span());
        self.list(vec![head, inner], span)
    }

    /// `,e` / `,{ e }` (unquote) or `,@e` / `,@{ e }` (unquote-splicing).
    fn unquote(&mut self, head_name: &str) -> StructId {
        let start = self.cur_span();
        let head = self.name(head_name, start);
        self.bump(); // `,` or `,@`
        let inner = if self.at(Kind::LBrace) {
            self.bump();
            let e = self.expr(0);
            self.expect(Kind::RBrace, "`}`");
            e
        } else {
            // a tight prefix (member/call chain), no trailing infix
            let s = self.cur_span();
            let p = self.prefix();
            self.postfix(p, s)
        };
        let span = start.merge(self.prev_span());
        self.list(vec![head, inner], span)
    }

    /// `#[ e, … ]`  ->  a `List` of the forms (the raw list escape).
    fn hash_list(&mut self) -> StructId {
        let start = self.cur_span();
        self.bump(); // '#'
        self.expect(Kind::LBracket, "`[`");
        let mut items = Vec::new();
        if !self.at(Kind::RBracket) {
            loop {
                items.push(self.expr(0));
                if self.at(Kind::Comma) {
                    self.bump();
                } else {
                    break;
                }
            }
        }
        self.expect(Kind::RBracket, "`]`");
        let span = start.merge(self.prev_span());
        self.list(items, span)
    }

    // ---- misc helpers ----

    /// A binder position: an identifier or backtick name; error placeholder otherwise.
    fn binder(&mut self) -> StructId {
        let span = self.cur_span();
        match self.kind() {
            Kind::Ident => {
                let t = self.bump().unwrap();
                self.name(self.text(t), span)
            }
            Kind::BacktickName => {
                let t = self.bump().unwrap();
                self.name(literal::unescape_backtick_name(self.text(t)), span)
            }
            _ => {
                self.error("expected a name");
                self.error_node(span)
            }
        }
    }

    /// A `Name` atom for a construct keyword head, at `span`.
    fn keyword_head(&mut self, name: &str, span: Span) -> StructId {
        self.name(name, span)
    }

    /// The span of the most recently consumed token (for closing a node's span).
    fn prev_span(&self) -> Span {
        if self.pos == 0 {
            Span::new(0, 0)
        } else {
            self.tokens[self.pos - 1].span
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(src: &str) -> Arenas {
        let p = read_ml(src);
        assert!(p.ok(), "expected clean parse of {src:?}, got {:?}", p.errors);
        p.arenas
    }

    #[test]
    fn parses_clean() {
        for src in [
            "42",
            "1 + 2 * 3",
            "f(a, b)",
            "a.b.c",
            "let x = 1, y = 2 in x + y",
            "if a then b else c",
            "fn(x, y) => x + y",
            "match e { Some(n) => n, None => 0, _ => neg }",
            "match n { x if x < 0 => neg, _ => pos }",
            "List.at(xs, 0)",
            "`{ x + 1 }",
            "#[a, b, c]",
        ] {
            let _ = parse_ok(src);
        }
    }

    #[test]
    fn arena_shapes() {
        // `1 + 2 * 3` -> (+ 1 (* 2 3))
        let a = parse_ok("1 + 2 * 3");
        assert_eq!(a.head_name(a.root), Some("+"));
        let plus = a.as_form(a.root, "+").unwrap();
        assert_eq!(a.head_name(plus[1]), Some("*"));

        // `f(a, b)` -> (f a b)
        let a = parse_ok("f(a, b)");
        assert_eq!(a.head_name(a.root), Some("f"));
        assert_eq!(a.as_form(a.root, "f").unwrap().len(), 2);

        // `a.b` -> (. a b)
        let a = parse_ok("a.b");
        assert_eq!(a.head_name(a.root), Some("."));

        // `if a then b else c` -> (if a b c)
        let a = parse_ok("if a then b else c");
        assert_eq!(a.as_form(a.root, "if").map(|t| t.len()), Some(3));

        // `let x = 1 in x` -> (let ((x 1)) x)
        let a = parse_ok("let x = 1 in x");
        let tail = a.as_form(a.root, "let").unwrap();
        assert_eq!(tail.len(), 2); // bindings + body
    }

    #[test]
    fn match_arm_is_pattern_body_pair() {
        // `match e { Some(n) => n, _ => 0 }` -> (match e ((Some n) n) (_ 0))
        let a = parse_ok("match e { Some(n) => n, _ => 0 }");
        let tail = a.as_form(a.root, "match").unwrap();
        assert_eq!(tail.len(), 3); // scrutinee + 2 arms
        // first arm is a 2-element list (pattern, body); pattern is (Some n)
        let crate::ast::Struct::List(arm0) = a.get(tail[1]) else { panic!() };
        assert_eq!(arm0.len(), 2);
        assert_eq!(a.head_name(arm0[0]), Some("Some"));
    }

    #[test]
    fn guarded_arm_wraps_pattern() {
        // `match n { x if x < 0 => neg, _ => pos }`: first arm pattern is (guard x (< x 0))
        let a = parse_ok("match n { x if x < 0 => neg, _ => pos }");
        let tail = a.as_form(a.root, "match").unwrap();
        let crate::ast::Struct::List(arm0) = a.get(tail[1]) else { panic!() };
        assert_eq!(a.head_name(arm0[0]), Some("guard"));
    }

    #[test]
    fn spans_are_total_and_distinct_for_occurrences() {
        // `x + x`: two x occurrences share one leaf but have distinct ids and distinct spans.
        let p = read_ml("x + x");
        assert!(p.ok());
        let a = &p.arenas;
        // span table has one entry per structure node
        assert_eq!(p.spans.len(), a.structure.len());
        let plus = a.as_form(a.root, "+").unwrap();
        let (l, r) = (plus[0], plus[1]);
        assert_ne!(l, r);
        let ls = p.spans.get(l).unwrap();
        let rs = p.spans.get(r).unwrap();
        assert_ne!(ls, rs, "the two `x` occurrences map to different source spans");
        // both are the text "x"
        assert_eq!(&"x + x"[ls.start..ls.end], "x");
        assert_eq!(&"x + x"[rs.start..rs.end], "x");
    }

    #[test]
    fn one_leaf_for_repeated_name() {
        let a = parse_ok("f(f, f)");
        // "f" interned once (+ nothing else); 3 occurrences.
        assert_eq!(a.leaves.len(), 1);
    }

    #[test]
    fn backtick_escapes_reserved_word() {
        // `` `let` `` is the name "let", not a let-form.
        let a = parse_ok("`let`");
        assert_eq!(a.as_name(a.root), Some("let"));
    }

    #[test]
    fn string_unescape_and_nfc() {
        let a = parse_ok(r#" "a\nb" "#);
        assert_eq!(a.leaf(match a.get(a.root) { crate::ast::Struct::Atom(l) => *l, _ => panic!() }),
                   &Leaf::Str("a\nb".to_string()));
    }

    #[test]
    fn never_panics() {
        for src in ["", "(", ")", "let", "match {", "1 +", ".", "=>", "fn(", "if then", "`", "\""] {
            let _ = read_ml(src); // must not panic
        }
    }
}
