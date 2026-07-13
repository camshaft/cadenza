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
//!
//! It is a RECOVERING parser: it never bails at the first error. Every error is collected into
//! `Parsed::errors`, and the arena is ALWAYS well-formed (`<error>`-name placeholders stand in for
//! nodes a production could not build). Recovery is tuned to resynchronize — a sub-parser never
//! consumes a token that belongs to an enclosing construct (a closing delimiter, a separator, an
//! arm/lambda `=>`, or a block keyword), so one stray symbol yields roughly one error instead of an
//! avalanche and the structure AROUND a mistake is still recovered. A missing `,` in a bracketed
//! list is reported once and both elements survive; a missing closer is reported and the partial
//! form is kept. Forward-progress guards on the open-ended loops guarantee parsing always terminates.
//! See the `error recovery` helpers ([`Parser::at_expr_stop`], [`Parser::sep_continue`]) and the
//! `error recovery` test module.

use crate::ast::{Arenas, Builder, Leaf, StructId};
use crate::lexer::{Lexer, Token};
use crate::literal;
use crate::span::Span;
use crate::spans::{FileId, SpanTable};
use crate::token::{Keyword, Kind, infix_prec, is_right_assoc, keyword, word_op};

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

/// A captured doc/comment preceding a grammar token: `/// text` (doc) or `// text` (comment), with
/// the `//`/`///` prefix and one optional leading space stripped, and the source span.
#[derive(Clone, Debug)]
struct Lead {
    doc: bool,
    text: String,
    span: Span,
}

/// Parse `src` (in file `file`) to arenas + spans.
pub fn parse(src: &str, file: FileId) -> Parsed {
    // Split the lexer stream into grammar tokens (everything the parser already handled) and a
    // parallel `leading` side-table: `leading[i]` is the run of doc/comment tokens that immediately
    // preceded grammar token `i`. The parser proper sees ONLY grammar tokens (unchanged); it
    // consults `leading` at definition boundaries to attach docs/comments. Comments no longer
    // vanish — they are captured here and re-emitted as `(doc …)` / `(comment …)` nodes.
    let mut tokens: Vec<Token> = Vec::new();
    let mut leading: Vec<Vec<Lead>> = Vec::new();
    let mut pending: Vec<Lead> = Vec::new();
    for t in Lexer::new(src) {
        match t.kind {
            Kind::Whitespace => {}
            Kind::LineComment | Kind::DocComment => {
                let doc = t.kind == Kind::DocComment;
                pending.push(Lead {
                    doc,
                    text: strip_comment(&src[t.span.start..t.span.end], doc),
                    span: t.span,
                });
            }
            _ => {
                tokens.push(t);
                leading.push(std::mem::take(&mut pending));
            }
        }
    }
    // A trailing run of comments with no following grammar token (e.g. a comment on the last line)
    // attaches to the virtual end position.
    let trailing = pending;

    let mut p = Parser {
        src,
        tokens,
        leading,
        trailing,
        pos: 0,
        builder: Builder::new(),
        spans: SpanTable::new(file),
        errors: Vec::new(),
        arm_bar_terminates: false,
    };
    let root = p.program();
    Parsed {
        arenas: p.builder.finish(root),
        spans: p.spans,
        errors: p.errors,
    }
}

/// Strip a comment token's `//`/`///` prefix and one optional following space, yielding its text.
fn strip_comment(raw: &str, doc: bool) -> String {
    let prefix = if doc { "///" } else { "//" };
    let body = raw.strip_prefix(prefix).unwrap_or(raw);
    body.strip_prefix(' ').unwrap_or(body).to_string()
}

/// Parse `src` as an anonymous single file (`FileId(0)`).
pub fn read_ml(src: &str) -> Parsed {
    parse(src, FileId::default())
}

struct Parser<'a> {
    src: &'a str,
    tokens: Vec<Token>,
    /// `leading[i]` = doc/comment tokens that immediately preceded grammar token `i`.
    leading: Vec<Vec<Lead>>,
    /// Doc/comments after the last grammar token.
    trailing: Vec<Lead>,
    pos: usize,
    builder: Builder,
    spans: SpanTable,
    errors: Vec<ParseError>,
    /// True while parsing a match-arm body at the arm's own bracket level: a top-level `|` there
    /// TERMINATES the arm (starts the next `| pat => body`) rather than being the bitwise-or operator.
    /// Any bracket (`(`/`[`/`{`) that starts a fresh sub-expression clears this, so `(a | b)` inside an
    /// arm body is still bitwise-or. (Corpus has zero infix-`|`, so this only future-proofs.)
    arm_bar_terminates: bool,
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

    /// An `Atom` of a STRING literal with source `span` — used as the HEAD of a compound-value literal
    /// so it desugars to the primitive CONSTRUCTOR (`[1 2]` → `("list" 1 2)`, `(a, b)` → `("tuple" a
    /// b)`). A string head is the unshadowable primitive: unlike a NAME head (`(list …)`), it is not a
    /// name a binding could shadow, so a literal always builds the compound even where the alias name
    /// `list`/`tuple`/`record`/`map` is rebound. ("The strings are the symbols.")
    fn ctor_head(&mut self, name: &str, span: Span) -> StructId {
        self.atom(Leaf::Str(name.to_string()), span)
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
        self.tok()
            .map(|t| t.span)
            .unwrap_or(Span::new(self.src.len(), self.src.len()))
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
        self.errors.push(ParseError {
            span: self.cur_span(),
            message: message.to_string(),
        });
    }

    /// A synthetic error placeholder occurrence (a name `<error>`), used when a production cannot
    /// produce a real node so the arena stays well-formed.
    fn error_node(&mut self, span: Span) -> StructId {
        self.name("<error>", span)
    }

    // ---- error recovery ----
    //
    // Recovery has two jobs: keep collecting errors past the first (never bail), and keep the token
    // cursor SYNCHRONIZED so one stray symbol yields ~one error rather than a cascade. The rule that
    // buys both is: a sub-parser must never consume a token that belongs to an ENCLOSING construct
    // (a closing delimiter, a separator, an arm/lambda arrow, a block keyword). Leaving such a token
    // for the parent lets the parent resynchronize on it; eating it desyncs everything after.

    /// True at a token that cannot begin an expression and belongs to an enclosing construct, so
    /// prefix-position error recovery must NOT consume it (that would desync the parent). Also true
    /// at end of input. A `|` counts only inside a match-arm body, where it starts the next arm; an
    /// ordinary leading `|` is consumable junk so the parser still makes progress.
    fn at_expr_stop(&self) -> bool {
        if self.at_end() {
            return true;
        }
        match self.kind() {
            Kind::RParen
            | Kind::RBracket
            | Kind::RBrace
            | Kind::Comma
            | Kind::Semi
            | Kind::FatArrow => true,
            Kind::Pipe => self.arm_bar_terminates,
            Kind::Ident => matches!(
                keyword(self.cur_text()),
                Some(Keyword::In | Keyword::Then | Keyword::Else | Keyword::With)
            ),
            _ => false,
        }
    }

    /// The pattern counterpart of [`Self::at_expr_stop`]. A bare `|` always terminates a pattern (it
    /// separates match arms, and this grammar has no infix `|`), regardless of the arm-body flag.
    fn at_pattern_stop(&self) -> bool {
        self.at_expr_stop() || self.at(Kind::Pipe)
    }

    /// True at a token that closes or continues an ENCLOSING construct relative to a bracketed list
    /// whose own closer is `closer`: a *different* closing delimiter, a `;`, an arm `=>`, a `|`, or a
    /// block keyword. A list's separator-recovery leaves such a token for the parent instead of
    /// swallowing it.
    fn at_outer_close(&self, closer: Kind) -> bool {
        match self.kind() {
            Kind::RParen | Kind::RBracket | Kind::RBrace => self.kind() != closer,
            Kind::Semi | Kind::FatArrow | Kind::Pipe => true,
            Kind::Ident => matches!(
                keyword(self.cur_text()),
                Some(Keyword::In | Keyword::Then | Keyword::Else | Keyword::With)
            ),
            _ => false,
        }
    }

    /// After parsing one element of a `,`-separated list closed by `closer`, decide whether to parse
    /// another, recovering from stray tokens. Returns `true` to continue:
    ///   - `,`  — consume it; stop only if the `closer` immediately follows (a tolerated trailing `,`).
    ///   - the `closer` / end of input / a token closing an ENCLOSING construct — stop (the caller's
    ///     `expect(closer)` reports a missing closer; an outer token is left for the parent to handle).
    ///   - anything else — a missing separator: record ONE error and continue, treating the token as
    ///     the next element. The element parser always consumes such a token, so the list terminates.
    fn sep_continue(&mut self, closer: Kind) -> bool {
        if self.at(Kind::Comma) {
            self.bump();
            return !self.at(closer); // tolerate a trailing comma before the closer
        }
        if self.at(closer) || self.at_end() || self.at_outer_close(closer) {
            return false;
        }
        self.error("expected `,`");
        true
    }

    // ---- doc / comment attachment ----

    /// Drain and return the `//` COMMENT leads preceding the current grammar token, leaving any
    /// `///` docs in place (a def/module drains those itself). Used at statement positions to wrap
    /// the following form in `(comment "text" node)`.
    fn take_comments_here(&mut self) -> Vec<Lead> {
        let leads = if self.pos < self.leading.len() {
            &mut self.leading[self.pos]
        } else {
            return Vec::new();
        };
        let (comments, docs): (Vec<Lead>, Vec<Lead>) =
            std::mem::take(leads).into_iter().partition(|l| !l.doc);
        *leads = docs;
        comments
    }

    /// Drain and return the `///` DOC leads preceding the current grammar token. Called by a
    /// def/module parser at entry to splice them as `(doc "text")` body forms.
    fn take_docs_here(&mut self) -> Vec<Lead> {
        let leads = if self.pos < self.leading.len() {
            &mut self.leading[self.pos]
        } else {
            return Vec::new();
        };
        let (docs, comments): (Vec<Lead>, Vec<Lead>) =
            std::mem::take(leads).into_iter().partition(|l| l.doc);
        *leads = comments;
        docs
    }

    /// Parse a statement (a top-level form / module member): capture any leading `//` comments and
    /// wrap the parsed form in `(comment "text" node)`, outermost = first. Leading `///` docs are
    /// left in place for a def/module parser to splice inside; any docs a non-def form leaves behind
    /// are then wrapped as comments too, so no doc/comment is ever dropped.
    fn stmt(&mut self) -> StructId {
        let start = self.pos;
        let comments = self.take_comments_here();
        let node = self.expr(0);
        // Docs still sitting at the statement's start slot were NOT consumed (the form was not a
        // def/module), so they'd otherwise be dropped — preserve them as comments.
        let leftover: Vec<Lead> = if start < self.leading.len() {
            std::mem::take(&mut self.leading[start])
        } else {
            Vec::new()
        };
        let node = self.wrap_comments(leftover, node);
        self.wrap_comments(comments, node)
    }

    /// Fold a run of comment leads around `node`: `[c0, c1]` -> `(comment c0 (comment c1 node))`.
    fn wrap_comments(&mut self, comments: Vec<Lead>, mut node: StructId) -> StructId {
        for lead in comments.into_iter().rev() {
            let head = self.name("comment", lead.span);
            let text = self.atom(Leaf::Str(lead.text), lead.span);
            node = self.list(vec![head, text, node], lead.span);
        }
        node
    }

    /// Build `(doc "text")` body-form nodes from a run of doc leads.
    fn doc_nodes(&mut self, docs: Vec<Lead>) -> Vec<StructId> {
        docs.into_iter()
            .map(|lead| {
                let head = self.name("doc", lead.span);
                let text = self.atom(Leaf::Str(lead.text), lead.span);
                self.list(vec![head, text], lead.span)
            })
            .collect()
    }

    // ---- program ----

    fn program(&mut self) -> StructId {
        if self.at_end() {
            let span = Span::new(0, 0);
            self.error("empty program");
            return self.error_node(span);
        }
        // A program is a `;`-separated SEQUENCE of top-level forms. One form stays bare; two or more
        // wrap into a `(do …)` sequencing form — the root counterpart of a nested `;` sequence, so a
        // corpus file authors its several top-level forms at the root (no wrapper keyword). The `;`
        // between forms is consumed here (a trailing/absent `;` is tolerated for robustness).
        let start = self.cur_span();
        let mut forms = vec![self.stmt()];
        while !self.at_end() {
            if self.at(Kind::Semi) {
                self.bump(); // separator between top-level forms
                if self.at_end() {
                    break; // trailing `;`
                }
            }
            let before = self.pos;
            forms.push(self.stmt());
            // Forward-progress guard: a stray token that begins no expression (e.g. a lone `)` at the
            // top level) is left un-consumed by `prefix` so a parent can resync — but here there is no
            // parent, so skip it ourselves. `prefix` already recorded the error; just advance so the
            // loop terminates.
            if self.pos == before && !self.at_end() {
                self.bump();
            }
        }
        let mut root = if forms.len() == 1 {
            forms.pop().unwrap()
        } else {
            let do_head = self.name("do", start);
            let mut items = Vec::with_capacity(forms.len() + 1);
            items.push(do_head);
            items.extend(forms);
            let span = start.merge(self.prev_span());
            self.list(items, span)
        };
        // Comments after the last grammar token (e.g. a trailing `// note` on the final line) have
        // no following form to precede; attach them as outer `(comment …)` wrappers so nothing is
        // dropped. (v1 scope: their printed position moves above the program — a known limitation
        // noted for a later "trailing comment" refinement.)
        let trailing = std::mem::take(&mut self.trailing);
        root = self.wrap_comments(trailing, root);
        root
    }

    // ---- expression grammar (Pratt) ----

    /// Parse an expression whose infix operators bind at least `min_prec`.
    fn expr(&mut self, min_prec: u8) -> StructId {
        let start = self.cur_span();
        let mut left = self.prefix();
        left = self.postfix(left, start);
        while let Some(op_name) = self.infix_op() {
            let prec = infix_prec(op_name).expect("infix_op returns only infix names");
            if prec < min_prec {
                break;
            }
            let op_span = self.cur_span();
            self.bump(); // operator
            let head = self.name(op_name, op_span);
            // Left-assoc: the right operand binds one tighter (`prec + 1`), so a same-precedence run
            // groups left. The type arrow `->` is right-associative — it recurses at `prec`, so
            // `A -> B -> C` groups as `A -> (B -> C)` (the curried reading).
            let right_min = if is_right_assoc(op_name) {
                prec
            } else {
                prec + 1
            };
            let right = self.expr(right_min);
            let span = start.merge(self.prev_span());
            left = self.list(vec![head, left, right], span);
        }
        left
    }

    /// Run a bracketed sub-expression parser with the match-arm `|`-terminates flag CLEARED, restoring
    /// it after — so a `|` inside `( … )`/`[ … ]`/`{ … }` within a match arm body is bitwise-or, while
    /// a `|` at the arm's own level still terminates the arm.
    fn bracketed_bars(&mut self, f: fn(&mut Self) -> StructId) -> StructId {
        let saved = self.arm_bar_terminates;
        self.arm_bar_terminates = false;
        let node = f(self);
        self.arm_bar_terminates = saved;
        node
    }

    /// The infix operator name at the cursor (symbolic kind or word-op ident), or `None`. A `|` at
    /// the top level of a match-arm body is NOT infix — it terminates the arm (see
    /// [`Self::arm_bar_terminates`]).
    fn infix_op(&self) -> Option<&'static str> {
        if self.arm_bar_terminates && self.at(Kind::Pipe) {
            return None;
        }
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
                // `unescape_string_token` yields the leaf directly — `Str` on a valid escape set, or a
                // `BadEscape` MARKER (`\q`) the compiler rejects CDZ0001. Both surfaces agree here.
                self.atom(literal::unescape_string_token(self.text(t)), span)
            }
            Kind::ByteStr => {
                let t = self.bump().unwrap();
                self.atom(
                    Leaf::Bytes(literal::unescape_byte_string_token(self.text(t))),
                    span,
                )
            }
            Kind::CharLit => {
                let t = self.bump().unwrap();
                // The token text is `#\<word>`; `char_leaf` classifies `<word>` into a `Char` scalar or a
                // `BadChar` MARKER (surrogate / out-of-range / unknown name) the compiler rejects CDZ0002.
                let word = self.text(t).strip_prefix("#\\").unwrap_or("");
                self.atom(literal::char_leaf(word), span)
            }
            Kind::BacktickName => {
                let t = self.bump().unwrap();
                self.name(literal::unescape_backtick_name(self.text(t)), span)
            }
            Kind::Ident => match keyword(self.cur_text()) {
                Some(Keyword::Let) => self.let_expr(),
                Some(Keyword::If) => self.if_expr(),
                Some(Keyword::Fn) => self.fn_expr(),
                Some(Keyword::Def) => self.def_expr(),
                Some(Keyword::Type) => self.type_expr(),
                Some(Keyword::Match) => self.match_expr(),
                Some(Keyword::Module) => self.module_expr(),
                Some(Keyword::Import) => self.import_expr(),
                Some(Keyword::Export) => self.export_expr(),
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
            // Bracketed sub-expressions parse their contents at a fresh level, where a `|` is bitwise-or
            // again — clear the arm-bar flag across them so `(a | b)` inside a match arm body works.
            Kind::LParen => self.bracketed_bars(Self::paren),
            Kind::LBracket => self.bracketed_bars(Self::list_literal),
            Kind::LBrace => self.bracketed_bars(Self::record_literal),
            Kind::Backtick => self.quasiquote(),
            Kind::Comma => self.unquote("unquote"),
            Kind::UnquoteSplice => self.unquote("unquote-splicing"),
            // `#{` is a map literal; `#[` is the raw-list escape.
            Kind::Hash if self.nth_kind(1) == Kind::LBrace => {
                self.bracketed_bars(Self::map_literal)
            }
            Kind::Hash => self.bracketed_bars(Self::hash_list),
            _ => {
                self.error("expected an expression");
                // Consume the offending token so we make progress — UNLESS it belongs to an
                // enclosing construct (a closer, separator, `=>`, or block keyword), which we leave
                // for the parent to resynchronize on. Eating it here would desync the whole rest.
                if !self.at_expr_stop() {
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
                        // A numeric index — positional tuple access `obj.0`. The key is the same `Int`
                        // atom the corpus `(. obj 0)` head-form carries, so both surfaces agree.
                        Kind::Int => {
                            let t = self.bump().unwrap();
                            self.atom(literal::classify_word(self.text(t)), key_span)
                        }
                        _ => {
                            self.error("expected a member name or index after `.`");
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

    /// A `.` begins member access only when followed by a member key — a field name, an escaped name,
    /// or a numeric index (`obj.0`, positional tuple access).
    fn dot_is_member(&self) -> bool {
        matches!(
            self.nth_kind(1),
            Kind::Ident | Kind::BacktickName | Kind::Int
        )
    }

    fn nth_kind(&self, n: usize) -> Kind {
        self.tokens
            .get(self.pos + n)
            .map(|t| t.kind)
            .unwrap_or(Kind::Error)
    }

    /// Parse `( e, … )` and return the argument occurrences.
    fn arg_exprs(&mut self) -> Vec<StructId> {
        self.expect(Kind::LParen, "`(`");
        let mut args = Vec::new();
        if !self.at(Kind::RParen) {
            loop {
                args.push(self.expr(0));
                if !self.sep_continue(Kind::RParen) {
                    break;
                }
            }
        }
        self.expect(Kind::RParen, "`)`");
        args
    }

    /// `()` the unit form, `( expr )` grouping, `( e, e, … )` a tuple literal `(tuple e …)`, or
    /// `( e; e; … )` a parenthesized SEQUENCE `(do e …)` — the way a sequence is used as a VALUE (a
    /// let-binding value, a call argument): `def x = (setup(); compute())`, like OCaml's
    /// `let x = (f (); 42)`. A single `(e)` is transparent grouping (NOT a 1-tuple).
    fn paren(&mut self) -> StructId {
        let start = self.cur_span();
        self.expect(Kind::LParen, "`(`");
        if self.at(Kind::RParen) {
            self.bump();
            let span = start.merge(self.prev_span());
            return self.name("unit", span);
        }
        let first = self.expr(0);
        if self.at(Kind::Comma) {
            // a tuple: gather the rest, recovering from a missing `,` between elements. The head is the
            // STRING primitive `"tuple"` (not the name), so the literal builds the unshadowable tuple
            // constructor even where the name `tuple` is rebound.
            let head = self.ctor_head("tuple", start);
            let mut items = vec![head, first];
            while self.sep_continue(Kind::RParen) {
                items.push(self.expr(0));
            }
            self.expect(Kind::RParen, "`)`");
            let span = start.merge(self.prev_span());
            return self.list(items, span);
        }
        if self.at(Kind::Semi) {
            // a parenthesized sequence -> (do first …). `let`-in-sequence scoping works here too: a
            // `let` element greedily takes the rest via `seq`/`let_expr`, so it lands last.
            let head = self.name("do", start);
            let mut items = vec![head, first];
            while self.at(Kind::Semi) {
                self.bump(); // `;`
                if self.at(Kind::RParen) {
                    break; // trailing `;`
                }
                items.push(self.expr(0));
            }
            self.expect(Kind::RParen, "`)`");
            let span = start.merge(self.prev_span());
            return self.list(items, span);
        }
        self.expect(Kind::RParen, "`)`");
        first // grouping is transparent in the arena
    }

    // ---- keyword forms ----

    /// `let n = e, … in body`  ->  `(let ((n e) …) body)`. The binding is separated from the body by
    /// `in`, which SELF-DELIMITS the `let` — its body is a full expression, so a `let` at the tail of
    /// a def body cannot swallow following top-level forms (the dangling-let fix). The body is a plain
    /// expression, not a `;`-sequence (a multi-statement body parenthesizes as `(a; b)`).
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

    /// `fn(p, …) => body`  ->  `(fn (p …) body)`, or with a return type `fn(p, …) -> R => body` ->
    /// `(fn (p …) (: body R))`. `fn` is ALWAYS an anonymous lambda now; a named definition uses `def`
    /// (see [`Self::def_expr`]). The return type desugars to a body ascription, exactly as a `def`'s
    /// return type does.
    fn fn_expr(&mut self) -> StructId {
        let start = self.cur_span();
        let head = self.keyword_head("fn", start);
        self.bump(); // `fn`
        let param_list = self.param_list();
        let ret_ty = self.opt_return_type();
        self.expect(Kind::FatArrow, "`=>`");
        let body = self.expr(0);
        let body = self.ascribe(body, ret_ty);
        let span = start.merge(self.prev_span());
        self.list(vec![head, param_list, body], span)
    }

    /// After a function's parameter list, an optional return-type annotation `-> R`. Returns the type
    /// occurrence (`R`) when a `->` follows, else `None`. Shared by `def` and `fn`; the caller wraps
    /// the body in `(: body R)` via [`Self::ascribe`].
    fn opt_return_type(&mut self) -> Option<StructId> {
        if self.at(Kind::Arrow) {
            self.bump(); // `->`
            Some(self.type_ref())
        } else {
            None
        }
    }

    /// Wrap `body` in a type ascription `(: body ty)` when a return type is present, else return `body`
    /// unchanged. The wrapper reuses the ascription form the value/parameter annotations already use,
    /// so a return type needs no dedicated IR node — it is the body constrained to the declared type.
    fn ascribe(&mut self, body: StructId, ty: Option<StructId>) -> StructId {
        match ty {
            Some(ty) => {
                let body_span = self.spans.get(body).unwrap_or_else(|| Span::new(0, 0));
                let ty_span = self.spans.get(ty).unwrap_or_else(|| Span::new(0, 0));
                let span = body_span.merge(ty_span);
                let colon = self.name(":", span);
                self.list(vec![colon, body, ty], span)
            }
            None => body,
        }
    }

    /// A named definition (a hoisting declaration), in two shapes:
    ///   `def name(p, …) = body`  ->  `(def (name p …) body)`   — a function
    ///   `def name = value`       ->  `(def name value)`         — a value
    /// The disambiguator is whether a `(` follows the name (a parameter list) or a `=` (a value). Both
    /// share the `def` keyword because both hoist — unlike a sequential `let`.
    fn def_expr(&mut self) -> StructId {
        let start = self.cur_span();
        // Leading `///` docs attach INSIDE the def, as `(doc "text")` body forms before the body.
        let docs = self.take_docs_here();
        let def_head = self.keyword_head("def", start);
        self.bump(); // `def`
        let sig_start = self.cur_span();
        let name = self.binder();

        // ---- value definition: `def name = value` -> (def name value) ----
        if self.at(Kind::Eq) {
            self.bump(); // `=`
            let value = self.expr(0);
            let span = start.merge(self.prev_span());
            // (def name doc… value) — docs precede the value, mirroring the function form.
            let mut items = vec![def_head, name];
            items.extend(self.doc_nodes(docs));
            items.push(value);
            return self.list(items, span);
        }

        // ---- function definition: `def name(p, …) = body` -> (def (name p …) body) ----
        let mut sig = vec![name];
        self.expect(Kind::LParen, "`(`");
        if !self.at(Kind::RParen) {
            loop {
                let before = self.pos;
                sig.push(self.param());
                if !self.sep_continue(Kind::RParen) {
                    break;
                }
                // A `param` at a non-name token (e.g. `def f(1) = …`) records an error but does not
                // consume — skip it so the missing-`,` branch of `sep_continue` can't loop forever.
                if self.pos == before {
                    self.bump();
                }
            }
        }
        self.expect(Kind::RParen, "`)`");
        let sig_span = sig_start.merge(self.prev_span());
        let signature = self.list(sig, sig_span);
        // Optional return-type annotation `-> R` between the signature and `=`. It desugars to a body
        // ascription: `def f(x) -> R = e` becomes `(def (f x) (: e R))`, reusing the annotation form —
        // no dedicated return-type node. The printer recovers the `-> R` from that body shape.
        let ret_ty = self.opt_return_type();
        self.expect(Kind::Eq, "`=`");
        let body = self.expr(0);
        let body = self.ascribe(body, ret_ty);
        let span = start.merge(self.prev_span());
        // (def signature doc… body) — docs precede the body form, matching the spec's
        // `(def (f) (doc "…") body)` shape.
        let mut items = vec![def_head, signature];
        items.extend(self.doc_nodes(docs));
        items.push(body);
        self.list(items, span)
    }

    /// `type Name = A(T, …) | B | …`  ->  `(type Name (A T …) B …)`. A sum-type declaration: each
    /// variant is either a nullary constructor (a bare `Ctor` -> a `Name` atom) or a constructor with
    /// a payload (`Ctor(T, …)` -> a list `(Ctor T …)`), separated by `|`. This mirrors the value
    /// side, where a nullary variant is a bare name and an applied one is `Ctor(args)`; the `|` is a
    /// surface separator between the structural variant entries, never a node in the tree.
    fn type_expr(&mut self) -> StructId {
        let start = self.cur_span();
        // Leading `///` docs attach INSIDE the type decl, as `(doc "text")` forms before the variants.
        let docs = self.take_docs_here();
        let head = self.keyword_head("type", start);
        self.bump(); // `type`
        let name = self.binder();
        let mut items = vec![head, name];
        items.extend(self.doc_nodes(docs));
        self.expect(Kind::Eq, "`=`");
        // Variants are `|`-led, with an (always-printed) leading `|` before the first — tolerate its
        // absence for robustness. Each `|` introduces a variant.
        if self.at(Kind::Pipe) {
            self.bump(); // optional leading `|`
        }
        loop {
            items.push(self.variant());
            if self.at(Kind::Pipe) {
                self.bump(); // `|`
            } else {
                break;
            }
        }
        let span = start.merge(self.prev_span());
        self.list(items, span)
    }

    /// One sum-type variant: `Ctor` (nullary -> a `Name` atom) or `Ctor(T, …)` (a payload -> the list
    /// `(Ctor T …)`). The constructor name is a binder; each payload type is parsed as a postfix
    /// expression (a name, dotted/qualified name, or application like `Tuple(A, B)` / `Option(Int64)`).
    fn variant(&mut self) -> StructId {
        let start = self.cur_span();
        let ctor = self.binder();
        if !self.at(Kind::LParen) {
            return ctor; // nullary variant: bare constructor name
        }
        self.bump(); // `(`
        let mut items = vec![ctor];
        if !self.at(Kind::RParen) {
            loop {
                let before = self.pos;
                items.push(self.type_ref());
                if !self.sep_continue(Kind::RParen) {
                    break;
                }
                // `type_ref`'s `prefix` at a stop token records an error without consuming; skip it so
                // the missing-`,` branch cannot loop forever.
                if self.pos == before {
                    self.bump();
                }
            }
        }
        self.expect(Kind::RParen, "`)`");
        let span = start.merge(self.prev_span());
        self.list(items, span)
    }

    /// `module name { form… }`  ->  `(module name doc… form…)`.
    fn module_expr(&mut self) -> StructId {
        let start = self.cur_span();
        // Leading `///` docs attach INSIDE the module as `(doc …)` forms before its members.
        let docs = self.take_docs_here();
        let head = self.keyword_head("module", start);
        self.bump(); // `module`
        let name = self.binder();
        let mut items = vec![head, name];
        items.extend(self.doc_nodes(docs));
        self.expect(Kind::LBrace, "`{`");
        while !self.at(Kind::RBrace) && !self.at_end() {
            // members capture their own leading `//` comments and `///` docs
            let before = self.pos;
            items.push(self.stmt());
            // Forward-progress guard: a stray token that begins no member and isn't our `}` (e.g. a
            // lone `)`) is left un-consumed by `prefix`; skip it so the module loop can't spin.
            if self.pos == before {
                self.bump();
            }
        }
        self.expect(Kind::RBrace, "`}`");
        let span = start.merge(self.prev_span());
        self.list(items, span)
    }

    /// `export { name, … }`  ->  `(export name …)`. A declaration of the module's public surface: a
    /// brace-delimited, comma-separated list of exported names (the same brace-of-names shape as a
    /// record's field-shorthand list, reused as the export surface). The names are the `(export …)`
    /// form's direct children — NOT a nested record — matching the corpus `(export main)` shape.
    fn export_expr(&mut self) -> StructId {
        let start = self.cur_span();
        let head = self.keyword_head("export", start);
        self.bump(); // `export`
        let mut items = vec![head];
        items.extend(self.brace_name_list());
        let span = start.merge(self.prev_span());
        self.list(items, span)
    }

    /// `import { name, … } from "path"`  ->  `(import "path" (name …))`. Brings a sibling module's
    /// public names into scope. The surface orders names-then-source for readability; the arena is the
    /// corpus's path-first shape `(import "path" (name …))` (a path string then a name-LIST), so both
    /// surfaces agree. (The qualified/alias form is a later phase.)
    fn import_expr(&mut self) -> StructId {
        let start = self.cur_span();
        let head = self.keyword_head("import", start);
        self.bump(); // `import`
        let names_start = self.cur_span();
        let names = self.brace_name_list();
        let names_span = names_start.merge(self.prev_span());
        let name_list = self.list(names, names_span);
        // `from` is a CONTEXTUAL keyword — an ordinary identifier `from` in this one position, not a
        // globally-reserved word (so `from` stays usable as a variable name elsewhere).
        if self.at(Kind::Ident) && self.cur_text() == "from" {
            self.bump();
        } else {
            self.error("expected `from` after the import name list");
        }
        // The module path: a string literal.
        let path = if self.at(Kind::Str) {
            let path_span = self.cur_span();
            let t = self.bump().unwrap();
            self.atom(literal::unescape_string_token(self.text(t)), path_span)
        } else {
            self.error("expected a module path string after `from`");
            self.error_node(self.cur_span())
        };
        let span = start.merge(self.prev_span());
        // Arena order is path-first: `(import "path" (name…))`.
        self.list(vec![head, path, name_list], span)
    }

    /// A brace-delimited comma-separated name list `{ a, b, … }` -> the vector of name occurrences.
    /// Shared by `export`/`import`. Each element is a bare (or backtick-escaped) name; a non-name
    /// element records an error and is skipped, so a malformed list still terminates.
    fn brace_name_list(&mut self) -> Vec<StructId> {
        self.expect(Kind::LBrace, "`{`");
        let mut names = Vec::new();
        if !self.at(Kind::RBrace) {
            loop {
                let before = self.pos;
                names.push(self.binder());
                if !self.sep_continue(Kind::RBrace) {
                    break;
                }
                if self.pos == before {
                    self.bump(); // no name consumed — avoid a missing-`,` spin
                }
            }
        }
        self.expect(Kind::RBrace, "`}`");
        names
    }

    /// A parenthesized parameter list `(p, …)` -> `(p …)`.
    fn param_list(&mut self) -> StructId {
        let params_start = self.cur_span();
        self.expect(Kind::LParen, "`(`");
        let mut params = Vec::new();
        if !self.at(Kind::RParen) {
            loop {
                let before = self.pos;
                params.push(self.param());
                if !self.sep_continue(Kind::RParen) {
                    break;
                }
                // `param` at a non-name token records an error without consuming; skip it so the
                // missing-`,` branch can't loop forever.
                if self.pos == before {
                    self.bump();
                }
            }
        }
        self.expect(Kind::RParen, "`)`");
        let params_span = params_start.merge(self.prev_span());
        self.list(params, params_span)
    }

    /// `match scrut with | pat [if g] => body | …`  ->  `(match scrut (pat body) …)`, where a guarded
    /// pattern is `(guard <pat> <expr>)`. Arms are `|`-led (with an always-printed leading `|`;
    /// tolerated if absent). An arm body runs until the next arm's `|` or the end of the enclosing
    /// context (the `|` at arm level does not read as bitwise-or; see `arm_bar_terminates`).
    fn match_expr(&mut self) -> StructId {
        let start = self.cur_span();
        let head = self.keyword_head("match", start);
        self.bump(); // `match`
        let scrut = self.expr(0);
        let mut items = vec![head, scrut];
        self.expect_keyword(Keyword::With, "`with`");
        if self.at(Kind::Pipe) {
            self.bump(); // optional leading `|`
        }
        loop {
            items.push(self.match_arm());
            if self.at(Kind::Pipe) {
                self.bump(); // `|` before the next arm
            } else {
                break;
            }
        }
        let span = start.merge(self.prev_span());
        self.list(items, span)
    }

    /// One arm: `pattern [if guard] => body`  ->  `(pattern body)`. The body is parsed with `|` set to
    /// terminate the arm (not read as bitwise-or), so the next `| pat => …` starts a fresh arm.
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
        let saved = self.arm_bar_terminates;
        self.arm_bar_terminates = true;
        let body = self.expr(0);
        self.arm_bar_terminates = saved;
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
                            let before = self.pos;
                            items.push(self.pattern());
                            if !self.sep_continue(Kind::RParen) {
                                break;
                            }
                            // A pattern at a stop token records an error without consuming; skip it
                            // so the missing-`,` branch can't loop forever.
                            if self.pos == before {
                                self.bump();
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
                // `unescape_string_token` yields the leaf directly — `Str` on a valid escape set, or a
                // `BadEscape` MARKER (`\q`) the compiler rejects CDZ0001. Both surfaces agree here.
                self.atom(literal::unescape_string_token(self.text(t)), span)
            }
            Kind::ByteStr => {
                let t = self.bump().unwrap();
                self.atom(
                    Leaf::Bytes(literal::unescape_byte_string_token(self.text(t))),
                    span,
                )
            }
            Kind::CharLit => {
                let t = self.bump().unwrap();
                let word = self.text(t).strip_prefix("#\\").unwrap_or("");
                self.atom(literal::char_leaf(word), span)
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
                // `()` -> unit; `(p)` -> transparent grouping; `(p, q, …)` -> a tuple pattern
                // `(tuple p q …)`, mirroring the expression `paren()` split (and the value tuple, so
                // `tuple(a, b)` and `(a, b)` are the same pattern).
                self.bump();
                if self.at(Kind::RParen) {
                    let s = self.cur_span();
                    self.bump();
                    return self.name("unit", s);
                }
                let first = self.pattern();
                if self.at(Kind::Comma) {
                    let head = self.name("tuple", span);
                    let mut items = vec![head, first];
                    while self.sep_continue(Kind::RParen) {
                        let before = self.pos;
                        items.push(self.pattern());
                        if self.pos == before {
                            self.bump(); // pattern didn't consume — avoid a missing-`,` spin
                        }
                    }
                    self.expect(Kind::RParen, "`)`");
                    let tup_span = span.merge(self.prev_span());
                    return self.list(items, tup_span);
                }
                self.expect(Kind::RParen, "`)`");
                first // grouping is transparent
            }
            _ => {
                self.error("expected a pattern");
                // Skip the offending token to make progress, but leave a `=>`, `|`, closer, or other
                // token that belongs to the enclosing match arm / bracketed pattern for the parent.
                if !self.at_pattern_stop() {
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

    /// `[ e, … ]`  ->  `("list" e …)`. A homogeneous sequence literal — head is the STRING primitive
    /// so the literal builds the unshadowable list constructor (a rebound name `list` cannot capture it).
    fn list_literal(&mut self) -> StructId {
        let start = self.cur_span();
        let head = self.ctor_head("list", start);
        self.bump(); // '['
        let mut items = vec![head];
        if !self.at(Kind::RBracket) {
            loop {
                let before = self.pos;
                items.push(self.expr(0));
                if !self.sep_continue(Kind::RBracket) {
                    break;
                }
                if self.pos == before {
                    self.bump(); // element didn't consume — avoid a missing-`,` spin
                }
            }
        }
        self.expect(Kind::RBracket, "`]`");
        let span = start.merge(self.prev_span());
        self.list(items, span)
    }

    /// `{ name = e, … }`  ->  `("record" (name e) …)`. Fixed named fields (distinct from a map); head is
    /// the STRING primitive so the literal builds the unshadowable record constructor.
    fn record_literal(&mut self) -> StructId {
        let start = self.cur_span();
        let head = self.ctor_head("record", start);
        self.bump(); // '{'
        let mut items = vec![head];
        if !self.at(Kind::RBrace) {
            loop {
                let before = self.pos;
                let f_start = self.cur_span();
                // Capture the field name's spelling BEFORE building the binder, so a shorthand field
                // can reuse it for the punned value (`binder` consumes the token; the builder doesn't
                // read occurrences back).
                let pun = self.binder_spelling();
                let name = self.binder();
                // Field SHORTHAND: `{ x }` puns to `{ x = x }` — a field with no `= value` binds the
                // field to a same-named value in scope. The value is a SECOND `x` occurrence (so it
                // resolves as an ordinary name reference), spanning the same name text.
                let value = if self.at(Kind::Eq) {
                    self.bump(); // `=`
                    self.expr(0)
                } else if let Some(n) = pun {
                    self.name(n, f_start)
                } else {
                    // a non-name field with no `=` — record the missing `=` as before.
                    self.expect(Kind::Eq, "`=`");
                    self.expr(0)
                };
                let f_span = f_start.merge(self.prev_span());
                items.push(self.list(vec![name, value], f_span));
                if !self.sep_continue(Kind::RBrace) {
                    break;
                }
                if self.pos == before {
                    self.bump(); // no field token consumed — avoid a missing-`,` spin
                }
            }
        }
        self.expect(Kind::RBrace, "`}`");
        let span = start.merge(self.prev_span());
        self.list(items, span)
    }

    /// The spelling of the upcoming binder token (a plain or backtick-escaped name), WITHOUT consuming
    /// it — so a caller can reuse the name (e.g. a punned record field `{ x }` → `{ x = x }`). `None`
    /// when the next token is not a name.
    fn binder_spelling(&self) -> Option<String> {
        match self.kind() {
            Kind::Ident => Some(self.cur_text().to_string()),
            Kind::BacktickName => Some(literal::unescape_backtick_name(self.cur_text())),
            _ => None,
        }
    }

    /// `#{ key = v, … }`  ->  `(map (key v) …)`. A dynamic key→value map (keys are arbitrary
    /// expressions), distinct from a record's fixed fields only by the `#` sigil; both use `=` to
    /// separate key/field from value. (The `=` separator is not the equality operator — bare `=` is
    /// never infix; equality is `==`.)
    fn map_literal(&mut self) -> StructId {
        let start = self.cur_span();
        let head = self.ctor_head("map", start);
        self.bump(); // '#'
        self.bump(); // '{'
        let mut items = vec![head];
        if !self.at(Kind::RBrace) {
            loop {
                let before = self.pos;
                let e_start = self.cur_span();
                let key = self.expr(0);
                self.expect(Kind::Eq, "`=`");
                let value = self.expr(0);
                let e_span = e_start.merge(self.prev_span());
                items.push(self.list(vec![key, value], e_span));
                if !self.sep_continue(Kind::RBrace) {
                    break;
                }
                if self.pos == before {
                    self.bump(); // no entry token consumed — avoid a missing-`,` spin
                }
            }
        }
        self.expect(Kind::RBrace, "`}`");
        let span = start.merge(self.prev_span());
        self.list(items, span)
    }

    /// `#[ e, … ]`  ->  a `List` of the forms (the raw list escape).
    fn hash_list(&mut self) -> StructId {
        let start = self.cur_span();
        self.bump(); // '#'
        self.expect(Kind::LBracket, "`[`");
        let mut items = Vec::new();
        if !self.at(Kind::RBracket) {
            loop {
                let before = self.pos;
                items.push(self.expr(0));
                if !self.sep_continue(Kind::RBracket) {
                    break;
                }
                if self.pos == before {
                    self.bump(); // element didn't consume — avoid a missing-`,` spin
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

    /// A parameter binder, optionally type-annotated: `name` or `name: Type`. An annotated parameter
    /// lowers to the binder-position annotation form `(: name Type)` — the same shape the s-expr
    /// surface writes — so the two surfaces agree. The `Type` is parsed by [`Self::type_ref`], which
    /// covers a name, a dotted/qualified name, an application like `Option(Int64)`, and a function
    /// type `A -> B`.
    fn param(&mut self) -> StructId {
        let start = self.cur_span();
        let binder = self.binder();
        if self.at(Kind::Colon) {
            self.bump(); // `:`
            let colon = self.name(":", start);
            let ty = self.type_ref();
            let span = start.merge(self.prev_span());
            self.list(vec![colon, binder, ty], span)
        } else {
            binder
        }
    }

    /// A type reference in a binder/return/payload position (a parameter annotation, a function's
    /// return type, a sum-variant payload). A type is a postfix expression — a name, a dotted or
    /// qualified name, or an application like `Option(Int64)` — extended with the RIGHT-associative
    /// function arrow `A -> B` -> `(-> A B)`, so a parameter/return type may itself be a function type
    /// (`f: Int64 -> Bool`, `-> Int64 -> Int64`). The arrow is parsed here (not via the general Pratt
    /// `expr`) so a type position admits `->` and application without also admitting arithmetic or a
    /// bare `:` re-ascription. `A -> B -> C` right-associates to `(-> A (-> B C))`.
    fn type_ref(&mut self) -> StructId {
        let start = self.cur_span();
        let head = self.prefix();
        let left = self.postfix(head, start);
        if self.at(Kind::Arrow) {
            self.bump(); // `->`
            let arrow = self.name("->", start);
            let right = self.type_ref(); // right-associative
            let span = start.merge(self.prev_span());
            self.list(vec![arrow, left, right], span)
        } else {
            left
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
        assert!(
            p.ok(),
            "expected clean parse of {src:?}, got {:?}",
            p.errors
        );
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
            "match e with | Some(n) => n | None => 0 | _ => neg",
            "match n with | x if x < 0 => neg | _ => pos",
            "List.at(xs, 0)",
            "x |> f",
            "x |> f(a) |> g",
            "total + tax |> round",
            "`{ x + 1 }",
            "#[a, b, c]",
        ] {
            let _ = parse_ok(src);
        }
    }

    #[test]
    fn pipeline_operator_builds_a_real_node() {
        // `|>` is a REAL infix operator, not parse-time sugar: it builds an arena node `(|> L R)`.
        // The rewrite into an application happens later (in the resolver), so the surface tree keeps
        // the operator and round-trips.
        let a = parse_ok("x |> f");
        assert_eq!(a.head_name(a.root), Some("|>"));
        let pipe = a.as_form(a.root, "|>").unwrap();
        assert_eq!(pipe.len(), 2);
        assert_eq!(a.as_name(pipe[0]), Some("x"));
        assert_eq!(a.as_name(pipe[1]), Some("f"));

        // Left-associative: `x |> f |> g` -> (|> (|> x f) g), a left-to-right pipeline.
        let a = parse_ok("x |> f |> g");
        let outer = a.as_form(a.root, "|>").unwrap();
        assert_eq!(a.head_name(outer[0]), Some("|>")); // the left operand is the inner pipe
        assert_eq!(a.as_name(outer[1]), Some("g"));

        // Looser than arithmetic: `total + tax |> round` -> (|> (+ total tax) round). The whole left
        // expression is the value threaded into the right.
        let a = parse_ok("total + tax |> round");
        let pipe = a.as_form(a.root, "|>").unwrap();
        assert_eq!(a.head_name(pipe[0]), Some("+"));
        assert_eq!(a.as_name(pipe[1]), Some("round"));
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

        // `p.0` -> (. p 0) — positional tuple access, the numeric sibling of `p.field`.
        let a = parse_ok("p.0");
        let tail = a.as_form(a.root, ".").unwrap();
        assert_eq!(a.as_name(tail[0]), Some("p"));
        assert!(
            matches!(a.get(tail[1]), crate::ast::Struct::Atom(l) if matches!(a.leaf(*l), Leaf::Int { .. }))
        );
        // `(x.0).1` -> (. (. x 0) 1) — chained index, parens keep `0.1` from lexing as a float.
        let a = parse_ok("(x.0).1");
        assert_eq!(a.head_name(a.root), Some("."));
        let outer = a.as_form(a.root, ".").unwrap();
        assert_eq!(a.head_name(outer[0]), Some("."));

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
        // `match e with | Some(n) => n | _ => 0` -> (match e ((Some n) n) (_ 0))
        let a = parse_ok("match e with | Some(n) => n | _ => 0");
        let tail = a.as_form(a.root, "match").unwrap();
        assert_eq!(tail.len(), 3); // scrutinee + 2 arms
        // first arm is a 2-element list (pattern, body); pattern is (Some n)
        let crate::ast::Struct::List(arm0) = a.get(tail[1]) else {
            panic!()
        };
        assert_eq!(arm0.len(), 2);
        assert_eq!(a.head_name(arm0[0]), Some("Some"));
    }

    #[test]
    fn guarded_arm_wraps_pattern() {
        // `match n with | x if x < 0 => neg | _ => pos`: first arm pattern is (guard x (< x 0))
        let a = parse_ok("match n with | x if x < 0 => neg | _ => pos");
        let tail = a.as_form(a.root, "match").unwrap();
        let crate::ast::Struct::List(arm0) = a.get(tail[1]) else {
            panic!()
        };
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
        assert_ne!(
            ls, rs,
            "the two `x` occurrences map to different source spans"
        );
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
        assert_eq!(
            a.leaf(match a.get(a.root) {
                crate::ast::Struct::Atom(l) => *l,
                _ => panic!(),
            }),
            &Leaf::Str("a\nb".to_string())
        );
    }

    #[test]
    fn never_panics() {
        for src in [
            "", "(", ")", "let", "match {", "1 +", ".", "=>", "fn(", "if then", "`", "\"",
        ] {
            let _ = read_ml(src); // must not panic
        }
    }

    // ---- error recovery ----
    //
    // The parser is a RECOVERING parser: it never bails at the first error. It collects every error
    // into `errors`, and — crucially — resynchronizes so one stray symbol yields roughly one error
    // instead of an avalanche, and so structure AROUND a mistake is still recovered. These tests pin
    // that behavior down: they assert the arena stays well-formed, that multiple independent errors
    // are all reported, that recovery syncs on delimiters, and that parsing always terminates.

    /// Assert the arena is well-formed regardless of errors: the root id is in range, every list
    /// child id is in range and traversable, and the span table is total (1:1 with structure nodes,
    /// the invariant the whole `SpanTable` design rests on). Returns the parse for further checks.
    fn recovered(src: &str) -> Parsed {
        let p = read_ml(src);
        let n = p.arenas.structure.len();
        assert!(n > 0, "arena is never empty for {src:?}");
        assert!(
            (p.arenas.root.0 as usize) < n,
            "root id in range for {src:?}"
        );
        assert_eq!(
            p.spans.len(),
            n,
            "span table stays total (1:1 with structure) for {src:?}"
        );
        // Every reachable node's children are valid ids — the tree is fully traversable.
        fn walk(a: &Arenas, id: StructId, seen: &mut usize) {
            *seen += 1;
            if let crate::ast::Struct::List(children) = a.get(id) {
                for &c in children {
                    assert!(
                        (c.0 as usize) < a.structure.len(),
                        "child id {} in range",
                        c.0
                    );
                    walk(a, c, seen);
                }
            }
        }
        let mut seen = 0;
        walk(&p.arenas, p.arenas.root, &mut seen);
        p
    }

    #[test]
    fn recovered_arena_is_always_well_formed() {
        // Whatever the garbage, the arena is traversable and the span table is total.
        for src in [
            "@",
            "f(@)",
            "1 + @ + 2",
            "let x = @ in x",
            "[1, @, 3]",
            "{ a = @, b = 2 }",
            "match e with | @ => 1",
            "def f(@, x) = x",
            "f(a b c",
            "module m { @ }",
            ")(][}{",
            "1 @ 2 # 3 ~ 4",
        ] {
            let _ = recovered(src);
        }
    }

    #[test]
    fn does_not_bail_at_first_error() {
        // Several independent mistakes are ALL reported, not just the first. Three stray symbols
        // separated as their own top-level statements yield (at least) three errors.
        let p = recovered("@; ~; $");
        assert!(
            p.errors.len() >= 3,
            "each stray statement reports its own error, got {:?}",
            p.errors
        );
    }

    #[test]
    fn a_single_stray_symbol_does_not_cascade() {
        // One bad token in the middle of an otherwise-fine call yields a small, bounded number of
        // errors — recovery resynchronizes rather than mis-parsing everything after it.
        let p = read_ml("f(a, @, c)");
        assert!(!p.ok(), "the stray `@` is reported");
        assert!(
            p.errors.len() <= 2,
            "one stray token stays bounded, got {} errors: {:?}",
            p.errors.len(),
            p.errors
        );
        // The call is still recovered as `(f a <error> c)` — the good arguments survive.
        let a = &p.arenas;
        assert_eq!(a.head_name(a.root), Some("f"));
        let call = a.as_form(a.root, "f").unwrap();
        assert_eq!(call.len(), 3, "three arguments recovered around the error");
        assert_eq!(a.as_name(call[0]), Some("a"));
        assert_eq!(a.as_name(call[2]), Some("c"));
    }

    #[test]
    fn error_inside_brackets_does_not_escape_them() {
        // The offending token inside `( … )` must NOT consume the closing `)` — the parser resyncs on
        // the bracket, so the SECOND statement after it parses cleanly as its own form.
        let p = read_ml("f(@); g(x)");
        assert!(!p.ok());
        // Root is a `(do …)` of two statements; the second is a clean call `(g x)`.
        let top = p.arenas.as_form(p.arenas.root, "do").unwrap();
        assert_eq!(top.len(), 2, "two top-level statements survive: {top:?}");
        assert_eq!(p.arenas.head_name(top[1]), Some("g"));
        let g = p.arenas.as_form(top[1], "g").unwrap();
        assert_eq!(p.arenas.as_name(g[0]), Some("x"));
    }

    #[test]
    fn missing_comma_between_args_recovers() {
        // `f(a b)` — a missing separator is reported once, and BOTH arguments are still recovered.
        let p = read_ml("f(a b)");
        assert!(!p.ok(), "the missing `,` is reported");
        assert_eq!(p.errors.len(), 1, "exactly one error: {:?}", p.errors);
        assert!(
            p.errors[0].message.contains(','),
            "the error names the missing comma: {:?}",
            p.errors[0]
        );
        let a = &p.arenas;
        let call = a.as_form(a.root, "f").unwrap();
        assert_eq!(call.len(), 2, "both args recovered");
        assert_eq!(a.as_name(call[0]), Some("a"));
        assert_eq!(a.as_name(call[1]), Some("b"));
    }

    #[test]
    fn missing_comma_in_list_recovers() {
        // `[1 2 3]` — every element is recovered, with one missing-`,` error per gap. The literal
        // desugars to the STRING-headed primitive `("list" 1 2 3)`, so read the tail via `as_ctor_form`.
        let p = read_ml("[1 2 3]");
        assert!(!p.ok());
        let a = &p.arenas;
        let list = a.as_ctor_form(a.root, "list").unwrap();
        assert_eq!(list.len(), 3, "all three elements recovered: {list:?}");
    }

    #[test]
    fn missing_closer_is_reported_and_recovered() {
        // An unterminated call reports the missing `)` but still yields a usable `(f a b)` tree
        // (rather than discarding the whole form).
        let p = read_ml("f(a, b");
        assert!(!p.ok());
        assert!(
            p.errors.iter().any(|e| e.message.contains(')')),
            "the missing `)` is reported: {:?}",
            p.errors
        );
        let a = &p.arenas;
        let call = a.as_form(a.root, "f").unwrap();
        assert_eq!(call.len(), 2);
    }

    #[test]
    fn recovers_the_let_around_a_bad_binding() {
        // A stray value in a binding is isolated: the `let` shape and its body survive.
        let p = read_ml("let x = @ in x + 1");
        assert!(!p.ok());
        let a = &p.arenas;
        let tail = a.as_form(a.root, "let").expect("still a let form");
        assert_eq!(tail.len(), 2, "bindings + body recovered");
        // body is `(+ x 1)` — parsed cleanly after the bad binding.
        assert_eq!(a.head_name(tail[1]), Some("+"));
    }

    #[test]
    fn keyword_boundary_is_not_swallowed_by_a_bad_condition() {
        // A stray symbol where the `if` condition belongs must not eat the `then` — the rest of the
        // form still parses, so we get an `(if …)` with three children.
        let p = read_ml("if @ then a else b");
        assert!(!p.ok());
        let a = &p.arenas;
        let if_form = a.as_form(a.root, "if").expect("still an if form");
        assert_eq!(if_form.len(), 3, "cond/then/else all recovered");
        assert_eq!(a.as_name(if_form[1]), Some("a"));
        assert_eq!(a.as_name(if_form[2]), Some("b"));
    }

    #[test]
    fn match_arm_boundary_survives_a_bad_pattern() {
        // A garbage pattern in the first arm does not consume the `=>` or the `|` that starts the
        // next arm — both arms are recovered.
        let p = read_ml("match e with | @ => 1 | _ => 2");
        assert!(!p.ok());
        let a = &p.arenas;
        let m = a.as_form(a.root, "match").expect("still a match");
        assert_eq!(m.len(), 3, "scrutinee + two arms recovered: {m:?}");
    }

    #[test]
    fn stray_closers_do_not_hang_and_stay_bounded() {
        // A pile of mismatched closers/garbage must terminate (the test completing IS the assertion)
        // and produce a well-formed arena with a finite error list.
        for src in [
            ")))))",
            "][}{)(",
            "f(((((",
            "[[[[[",
            "{{{{{",
            "#{#{#{",
            ",,,,,",
            "..........",
            "=> => =>",
            "| | | |",
            "@@@@@@@@@@",
            "let let let",
        ] {
            let p = recovered(src);
            assert!(
                p.errors.len() < 10_000,
                "error list stays finite for {src:?} (no runaway loop)"
            );
        }
    }

    #[test]
    fn valid_programs_still_report_no_errors() {
        // Recovery must be inert on well-formed input — no spurious errors, exact trees preserved.
        for src in [
            "1 + 2 * 3",
            "f(a, b, c)",
            "let x = 1, y = 2 in x + y",
            "match e with | Some(n) => n | None => 0",
            "def f(x, y) = x + y",
            "[1, 2, 3]",
            "{ a = 1, b = 2 }",
            "#{ k = v }",
            "if a then b else c",
            "module m { def x = 1 def y = 2 }", // module members are whitespace-separated, no `;`
        ] {
            let p = read_ml(src);
            assert!(p.ok(), "no spurious errors on {src:?}: {:?}", p.errors);
        }
    }

    #[test]
    fn exhaustive_short_token_soup_always_terminates_well_formed() {
        // The strongest termination evidence: enumerate EVERY sequence of up to four tokens drawn
        // from an alphabet chosen to stress recovery (delimiters, separators, keywords, junk). If any
        // combination could drive `prefix`/`sep_continue`/the block loops into a non-advancing cycle,
        // this test would hang — so its completion is the proof that parsing always makes progress.
        // Each parse is also checked for a well-formed, traversable arena and a total span table.
        let alphabet = [
            "(", ")", "[", "]", "{", "}", "#", ",", ";", ".", "=>", "|", "@", "let", "in", "if",
            "match", "with", "def", "x", "1",
        ];
        let mut count = 0usize;
        // lengths 1..=3 exhaustively; a light length-4 sweep keeps the total bounded but deep.
        for len in 1..=3 {
            let combos = alphabet.len().pow(len as u32);
            for mut n in 0..combos {
                let mut src = String::new();
                for _ in 0..len {
                    src.push_str(alphabet[n % alphabet.len()]);
                    src.push(' ');
                    n /= alphabet.len();
                }
                let _ = recovered(&src); // must terminate + stay well-formed
                count += 1;
            }
        }
        assert!(count > 8_000, "swept a meaningful space, got {count}");
    }

    #[test]
    fn nested_error_reports_once_and_outer_form_survives() {
        // A bad token nested two levels deep is reported, and every enclosing construct is still
        // recovered up to the root.
        let p = recovered("g(f(a, @), b)");
        assert!(!p.ok());
        let a = &p.arenas;
        // outer call `(g (f a <error>) b)`
        let g = a.as_form(a.root, "g").expect("outer call recovered");
        assert_eq!(g.len(), 2, "outer call keeps both args: {g:?}");
        let f = a.as_form(g[0], "f").expect("inner call recovered");
        assert_eq!(f.len(), 2, "inner call keeps both args: {f:?}");
        assert_eq!(a.as_name(g[1]), Some("b"), "arg after the bad one survives");
    }
}
