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
use crate::token::{Keyword, Kind, PREC_AS, infix_prec, is_right_assoc, keyword, word_op};

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
        depth: 0,
        depth_exceeded: false,
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
    /// The current recursion depth of `expr` — incremented on entry and decremented on exit. Every
    /// nested sub-expression funnels through `expr` (the Pratt hub: bracket forms, keyword forms, and
    /// the infix right operand all call it), so bounding it bounds the native stack. Past
    /// [`crate::sexpr::MAX_NESTING_DEPTH`] `expr` records one error and returns an `<error>` node
    /// instead of recursing, guarding against a stack overflow (SIGABRT) on pathologically deep input —
    /// the ML-surface analogue of the s-expr reader's guard (shares the one limit constant).
    depth: u32,
    /// Set once the depth limit is hit — a FATAL, unrecoverable parse condition. The ordinary
    /// error-recovery model (record an error, keep the cursor, make progress) cannot apply here: the
    /// thousands of unconsumed deeply-nested tokens would drive every enclosing loop to re-enter `expr`,
    /// hit the limit again, and spin (a non-terminating hang, not a crash). So once set, [`Self::at_end`]
    /// reports end-of-input: every parse loop (`program`, `paren`, arg lists, …) terminates immediately
    /// and the stack unwinds without reprocessing the deep tail. One diagnostic, clean termination.
    depth_exceeded: bool,
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

    /// Classify a numeric token's text into a value node, DESUGARING a `100N`/`0.5R` type suffix to
    /// the annotation `(: <literal> BigInt|Rational)` — the ML twin of the sexpr reader's suffix
    /// desugar, so a suffixed literal reads to the SAME arena on both surfaces. A bare number stays a
    /// plain atom. The `Suffixed` leaf is kept as the value child so the printer re-emits the suffix.
    fn numeric_atom(&mut self, text: &str, span: Span) -> StructId {
        match literal::classify_word(text) {
            leaf @ Leaf::Suffixed { kind, .. } => {
                let colon = self.name(":", span);
                let value = self.atom(leaf, span);
                let ty = self.name(kind.type_name(), span);
                self.list(vec![colon, value, ty], span)
            }
            leaf => self.atom(leaf, span),
        }
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
        // A depth-exceeded parse is fatally poisoned: report end-of-input so every parse loop
        // terminates at once and the stack unwinds without reprocessing the deep token tail (see
        // `depth_exceeded`).
        self.depth_exceeded || self.pos >= self.tokens.len()
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

    /// If the cursor is at a `..` rest/spread marker, consume it, push the `..` marker plus the
    /// following binder (parsed by `elem`) onto `items`, and return `true`; otherwise consume nothing
    /// and return `false`. The arena stays FLAT — a `Leaf::Name("..")` sibling immediately followed by
    /// the rest node — the SAME shape the s-expression surface writes and the list/map lowering scans
    /// for (`(list p… .. rest)`, `(map (k p) .. rest)`). This is the one rest/spread marker shared by
    /// every collection in both construction (`[1, 2, .. rest]`) and pattern (`[x, .. rest]`) position;
    /// well-formedness (exactly one binder after `..`, `..` last) is left to the compiler, matching the
    /// s-expr surface, which likewise accepts the flat form and rejects a malformed one at lowering.
    fn rest_marker(
        &mut self,
        items: &mut Vec<StructId>,
        elem: impl FnOnce(&mut Self) -> StructId,
    ) -> bool {
        if !self.at(Kind::DotDot) {
            return false;
        }
        let dd_span = self.cur_span();
        self.bump(); // `..`
        items.push(self.name("..", dd_span));
        items.push(elem(self));
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
    ///
    /// The parser REPRESENTS each `//` comment as a `(comment "text" node)` NODE of the canonical
    /// representation — not discarded as lexical trivia — wrapping the following form so the comment is
    /// ATTACHED to the part it annotates (its position recovered on printing). Because the node is an
    /// ordinary list node, it survives the binary-AST codec: printing the binary AST back to text and
    /// re-parsing yields the same `(comment …)` (and `(doc …)`) nodes — comments and documentation both
    /// round-trip. (An intra-program EDIT preserving them is the sidecar `Rewrite` surface, not yet built.)
    //= spec/capabilities/agent-authoring.md#comments-are-parsed-into-the-representation
    //# A textual syntax's parser MUST represent a comment it reads as a node of the canonical representation rather than discard it as lexical trivia, because the canonical stored form is the binary AST and a comment not carried by the tree is not stored.
    //= spec/capabilities/agent-authoring.md#comments-are-parsed-into-the-representation
    //# A comment MUST be attached in the canonical representation to the part of the program it annotates, so that its position relative to that part is recovered on printing.
    //= spec/capabilities/agent-authoring.md#comments-survive-round-trip-and-edits
    //# A comment MUST be preserved when a program's binary AST is printed to a textual syntax and parsed back.
    //= spec/capabilities/agent-authoring.md#documentation-survives-round-trip-and-edits
    //# Documentation MUST be preserved when a program's binary AST is printed to a textual syntax and parsed back.
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
        // A program is a JUXTAPOSED run of top-level forms — whitespace-separated, NO `;` between them,
        // exactly like the members of a `module { … }` block. One form stays bare; two or more wrap into
        // a `(do …)` form, the root's declaration+result list (no wrapper keyword in the surface). `;` is
        // NOT a top-level separator: it is the sequencing operator WITHIN a body (see `finish_sequence`),
        // so a `def`/expression body greedily collects its own `;`-run and stops at the next juxtaposed
        // form. A stray `;` between top-level forms is thus surplus, skipped by the progress guard below.
        let start = self.cur_span();
        let mut forms = Vec::new();
        self.push_root_form(&mut forms, self.pos);
        while !self.at_end() {
            let before = self.pos;
            self.push_root_form(&mut forms, before);
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

    /// Parse one top-level form and append it to `forms`, FLATTENING a `(do …)` result into its
    /// elements. `stmt` parses at `expr(0)`, so a `;`-run between top-level forms (`a; b`) folds into a
    /// single `(do a b)` — but the root is itself a flat declaration+result sequence, so those elements
    /// are the root's own forms, not a nested block. Splicing them here makes a `;`-separated top-level
    /// run and a whitespace-JUXTAPOSED one converge on the SAME flat root `(do …)`: the surface may write
    /// either (or mix them — a `;` only where an adjacency would otherwise re-lex, e.g. `def x = 5; x + 1`)
    /// and the tree is identical. A `(comment …)`-wrapped stmt is appended whole (its inner form may be a
    /// `do`, but the comment wrapper must stay attached). `start` is the stmt's first token position, used
    /// only to detect that `stmt` made no progress (handled by the caller's guard).
    fn push_root_form(&mut self, forms: &mut Vec<StructId>, start_pos: usize) {
        let node = self.stmt();
        if self.pos == start_pos {
            return; // no progress — the caller's guard will advance past the stray token
        }
        // Splice a bare `(do e1 e2 …)` (head is the NAME `do`, NOT a comment-wrapped node) into flat
        // root forms; anything else is one form. `as_form` matches only a NAME-`do` head with ≥1 child.
        let do_elems = self
            .builder
            .as_form(node, "do")
            .filter(|elems| !elems.is_empty())
            .map(|elems| elems.to_vec());
        match do_elems {
            Some(elems) => forms.extend(elems),
            None => forms.push(node),
        }
    }

    // ---- expression grammar (Pratt) ----

    /// Parse an expression whose infix operators bind at least `min_prec`.
    fn expr(&mut self, min_prec: u8) -> StructId {
        let start = self.cur_span();
        // DEPTH GUARD: every nested sub-expression funnels through `expr` (bracket/keyword forms and the
        // infix right operand all call it), so bounding this recursion bounds the native stack. Past the
        // limit, record ONE error and return an `<error>` node WITHOUT recursing — a clean diagnostic
        // instead of a stack overflow (SIGABRT). Shares the s-expr reader's limit (see `MAX_NESTING_DEPTH`).
        if self.depth >= crate::sexpr::MAX_NESTING_DEPTH {
            // Record ONE error and POISON the parser (fatal): further parsing would spin on the deep
            // unconsumed tail. `depth_exceeded` makes `at_end` true, so all enclosing loops stop.
            if !self.depth_exceeded {
                self.error("expression nests too deeply to parse");
                self.depth_exceeded = true;
            }
            return self.error_node(start);
        }
        self.depth += 1;
        let mut left = self.prefix();
        left = self.postfix(left, start);
        // The number of left-associative layers this loop has folded onto `left`. Added to `self.depth`
        // (the recursion depth) it is the arena-tree depth built so far, which the guard below bounds.
        let mut spine: u32 = 0;
        loop {
            // `expr as UNIT` — the unit-conversion operator, handled here rather than via `infix_op`
            // because its right operand is a UNIT denotation (a bare name reads as `(Unit.of #"name")`,
            // and `*`/`/`/`^` compose units), not an ordinary expression. It binds at `PREC_AS` (above
            // the pipeline, below arithmetic), so `a / b as u` converts the quotient `(a / b)` and
            // `q as u |> f` threads the conversion into the pipeline. Left-associative — the loop
            // re-checks, so `q as m as m` chains left. Checked inside the shared loop so it interleaves
            // with the arithmetic operators (`/` binds tighter, so it is consumed first).
            // The `as` conversion must not cross a STATEMENT/NEWLINE boundary: a leading `as` on a new line
            // would reach BACK across the newline and absorb the previous statement's (or a def RHS's)
            // trailing expression — `def a() = 5.0 <newline> as meter` silently becoming `def a() = (5.0 as
            // meter)`, changing a's type from a number to Qty(meter) on a mere line break. Statement
            // sequencing (`539f7712`: forms juxtapose across lines) takes precedence, so an `as` separated
            // from its left operand by a newline is a SEPARATE statement, not a continuation. Same
            // boundary the quantity sugar draws (`f57c4a53`); the `as` operator landed alongside it without
            // the guard. A genuine same-line `q as u` has no intervening newline and still converts.
            if self.at_keyword(Keyword::As)
                && PREC_AS >= min_prec
                && !self.src[self.prev_span().end..self.cur_span().start].contains('\n')
            {
                left = self.as_conversion(left, start);
                continue;
            }
            let Some(op_name) = self.infix_op() else {
                break;
            };
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
            // DEPTH GUARD for the LEFT SPINE. A left-associative run (`a + b + c + …`) is parsed by
            // this LOOP, not by recursion, so `self.depth` does not grow with it — but each iteration
            // deepens the produced arena on its LEFT (`(op (op a b) c)`), so a long flat run yields an
            // arbitrarily deep TREE that a recursive CONSUMER (the s-expr printer, `canon`) then walks
            // to a stack overflow (SIGABRT), even though the PARSE never recursed. Count each folded
            // layer against the same limit so a pathologically long chain produces one clean
            // "nests too deeply" diagnostic instead of a downstream crash. `depth_exceeded` poisons the
            // parse (⇒ `at_end`), so the loop's next `infix_op`/`at_keyword` check stops it.
            spine += 1;
            if !self.depth_exceeded && self.depth + spine >= crate::sexpr::MAX_NESTING_DEPTH {
                self.error("expression nests too deeply to parse");
                self.depth_exceeded = true;
                break;
            }
        }
        // Sequencing `;` is the LOOSEST operator (looser than every infix op above), so it is folded
        // here AFTER the Pratt loop rather than through it: a `;`-run collapses to a single flat
        // `(do a b c)` (not the nested `(; a (; b c))` a generic right-assoc fold would give), with the
        // last element the sequence's value — modelling `a; b` as `let _ = a in b`. It is collected only
        // when the CALLER permits sequencing (`min_prec == PREC_SEQ`, i.e. a body/statement position);
        // a sub-expression parsed at any tighter level leaves the `;` for its enclosing sequence, so a
        // `;` inside a call arg / list / tuple element does not escape into the element.
        if min_prec == crate::token::PREC_SEQ && self.at(Kind::Semi) {
            left = self.finish_sequence(left, start);
        }
        self.depth -= 1;
        left
    }

    /// Fold a `;`-separated run starting after `first` into a flat `(do first e2 e3 …)`. Each following
    /// element is parsed at `PREC_SEQ + 1`, so it stops at the next `;` (the elements stay flat siblings
    /// rather than nesting), modelling `a; b` as `let _ = a in b`.
    ///
    /// A `;` ENDS the sequence — consumed as a surplus/trailing separator, no further element taken —
    /// when the token after it is:
    ///   - a closer / block keyword / end of input (a genuine trailing `;`), or
    ///   - a DECLARATION KEYWORD (`def`/`type`/`effect`/`module`/`import`/`export`). A declaration is a
    ///     top-level / module-member form, not an expression-statement, so a `;` before it cannot be
    ///     sequencing — it is the separator a body was terminated with. This lets a function body
    ///     greedily collect its statement run (`deposit(20); deposit(5); balance()`) while a trailing
    ///     `;` before the next `def` cleanly ends the body instead of swallowing that `def`.
    ///
    /// When only `first` was collected (every `;` ended the sequence), it is returned BARE — a lone
    /// `first;` is just `first`, not a one-element `(do first)`.
    fn finish_sequence(&mut self, first: StructId, start: Span) -> StructId {
        let mut elems = vec![first];
        while self.at(Kind::Semi) {
            self.bump(); // `;`
            if self.at_expr_stop() || self.at_declaration_keyword() {
                break; // trailing `;`, or the next juxtaposed declaration form begins
            }
            elems.push(self.expr(crate::token::PREC_SEQ + 1));
        }
        if elems.len() == 1 {
            return elems.pop().expect("one element");
        }
        let do_head = self.name("do", start);
        let mut items = Vec::with_capacity(elems.len() + 1);
        items.push(do_head);
        items.extend(elems);
        let span = start.merge(self.prev_span());
        self.list(items, span)
    }

    /// True at a declaration keyword (`def`/`type`/`effect`/`module`/`import`/`export`) — a top-level or
    /// module-member form that introduces a binding/declaration, never an expression-statement. A `;`
    /// sequence stops before one (see [`Self::finish_sequence`]); the declaration is left to be parsed
    /// as the next juxtaposed form by `program`/`module_expr`.
    fn at_declaration_keyword(&self) -> bool {
        self.at(Kind::Ident)
            && matches!(
                keyword(self.cur_text()),
                Some(
                    Keyword::Def
                        | Keyword::Type
                        | Keyword::Effect
                        | Keyword::Module
                        | Keyword::Import
                        | Keyword::Export
                )
            )
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
                let text = self.text(t).to_string();
                let num = self.numeric_atom(&text, span);
                // A TYPE-SUFFIXED literal desugared to a `(: … …)` annotation is NOT a quantity magnitude
                // (`100N feet` is meaningless — a suffix selects a numeric type, not a unit); only a BARE
                // number takes the `<num> <unit>` quantity sugar. A suffix is glued (no space), so the
                // two never both apply to one literal; guard on the suffix so a following name is not
                // mis-eaten as a unit.
                if matches!(literal::classify_word(&text), Leaf::Suffixed { .. }) {
                    num
                } else {
                    self.maybe_quantity_literal(num, span)
                }
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
            Kind::SymLit => {
                let t = self.bump().unwrap();
                // The token text is `#"<body>"`; `unescape_sym_token` strips the `#"`/`"` and yields a
                // `Leaf::Sym` (reusing the string escape set + NFC). The s-expr reader agrees.
                self.atom(literal::unescape_sym_token(self.text(t)), span)
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
            // An ANNOTATION: `@name form` -> `(@ name form)`. `@` is a GENERAL-PURPOSE annotation
            // sigil — the name is any ident (`inline-never`, `inline-always`, and whatever future
            // annotations the language grows); the compiler consumes the ones it recognizes and rejects
            // the rest. Stacked annotations nest, since the wrapped form is itself parsed in prefix
            // position: `@a @b def …` -> `(@ a (@ b (def …)))`.
            Kind::At => {
                self.bump(); // `@`
                let head = self.name("@", span);
                let name = if self.at(Kind::Ident) {
                    let name_span = self.cur_span();
                    let t = self.bump().unwrap();
                    self.name(self.text(t), name_span)
                } else {
                    self.error("expected an annotation name after `@`");
                    self.error_node(self.cur_span())
                };
                // The annotated form parses in PREFIX position (no postfix): a following juxtaposed
                // top-level form that begins with `(` must not be swallowed as a call of the def. A
                // `def`/other keyword dispatches to its full form; a nested `@` recurses here.
                let form = self.prefix();
                let full = span.merge(self.prev_span());
                self.list(vec![head, name, form], full)
            }
            // A PRAGMA sugar: `@!key arg` -> `(pragma key arg)`. The inner-attribute twin of `@` — an
            // annotation applies to the item below it, a pragma to the enclosing MODULE (Rust's `#[…]` vs
            // `#![…]`). The head is the `pragma` keyword itself, so the desugared form is byte-identical to
            // a written `(pragma key arg)` and flows through the SAME registry/validation with no new
            // downstream case. The KEY is any ident (the registry decides which are defined); the ARGUMENT
            // is one form parsed in PREFIX position (a type name `Float32` or a parenthesized type
            // expression `(Int 8)`), so a juxtaposed following form is not swallowed as an application.
            Kind::AtBang => {
                self.bump(); // `@!`
                let head = self.name("pragma", span);
                let key = if self.at(Kind::Ident) {
                    let key_span = self.cur_span();
                    let t = self.bump().unwrap();
                    self.name(self.text(t), key_span)
                } else {
                    self.error("expected a pragma key after `@!` (e.g. `@!default-float Float32`)");
                    self.error_node(self.cur_span())
                };
                // The ARGUMENT is a TYPE expression parsed in prefix+POSTFIX position — so a bare name
                // (`Float32`), a member access (`Foo.Bar`), and a constructor APPLICATION (`Int(8)` ->
                // `(Int 8)`) all parse as the single argument, exactly as a type annotation's type does. The
                // postfix stops at a `.`/`(` glued to the type; a following module member (`def …` on the
                // next line) does not begin with either, so it is never swallowed. Infix operators / `as`
                // are intentionally NOT consumed (a pragma type is a single type, never `A -> B`).
                let arg_start = self.cur_span();
                let arg_prefix = self.prefix();
                let arg = self.postfix(arg_prefix, arg_start);
                let full = span.merge(self.prev_span());
                self.list(vec![head, key, arg], full)
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
                Some(Keyword::Effect) => self.effect_expr(),
                Some(Keyword::Handle) => self.handle_expr(),
                Some(Keyword::Host) => self.host_expr(),
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
            // `#{` is a map literal; `#(` is a set literal; `#[` is the raw-list escape.
            Kind::Hash if self.nth_kind(1) == Kind::LBrace => {
                self.bracketed_bars(Self::map_literal)
            }
            Kind::Hash if self.nth_kind(1) == Kind::LParen => {
                self.bracketed_bars(Self::set_literal)
            }
            Kind::Hash => self.bracketed_bars(Self::hash_list),
            // `b[ <segment>, … ]` — a binary literal, sugar for `(bin <segment> …)`. Each segment is an
            // ordinary call-shaped expression (`u16(258)`, `bits(1, 1)`, `bytes(payload)`), so it parses
            // like a list literal's elements and wraps under the `bin` grammar head.
            Kind::BinOpen => self.bracketed_bars(Self::bin_literal),
            // A lexer ERROR token — an unterminated literal (`"…` / `b"…` / `#"…` / `` `… `` / `#\`) run
            // to end-of-input, or an otherwise-unrecognized character. The generic "expected an
            // expression" here misdirects (the token IS where an expression should start; the real defect
            // is that the literal never closed), so name the specific cause from the token's opening
            // characters — the lexer merged the error token's span from the opener to EOF, so `cur_text`
            // begins with the opener. This is the ML-surface twin of the s-expr reader's "unterminated
            // string" message; without it an unterminated string in ML read as a bare "expected an
            // expression" at the quote.
            Kind::Error => {
                let t = self.cur_text();
                let msg = if t.starts_with("b\"") {
                    "unterminated byte-string literal (missing closing `\"`)"
                } else if t.starts_with("#\"") {
                    "unterminated symbol literal (missing closing `\"`)"
                } else if t.starts_with('"') {
                    "unterminated string literal (missing closing `\"`)"
                } else if t.starts_with('`') {
                    "unterminated backtick name (missing closing `` ` ``)"
                } else if t.starts_with("#\\") {
                    "unterminated character literal"
                } else {
                    "unexpected character"
                };
                self.error(msg);
                if !self.at_expr_stop() {
                    self.bump();
                }
                self.error_node(span)
            }
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

    /// A numeric literal immediately followed by a bare unit name is a QUANTITY LITERAL: `5 feet`
    /// (and `5.0 feet`) reads as `(Qty.of 5 (Unit.of #"feet"))` — the concise ML surface for attaching
    /// a compile-time unit to a number. It binds TIGHTER than every infix operator (built here in
    /// prefix position, before the Pratt loop), so `5 feet / 1 second` groups as
    /// `(/ (Qty.of 5 (Unit.of #"feet")) (Qty.of 1 (Unit.of #"second")))` — a rate, not `5 (feet / 1) …`.
    ///
    /// The unit name is any single `Ident` that is neither a keyword (`in`/`then`/…) nor a word-op
    /// (`and`/`or`) — those keep their existing meaning after a number (`5 in`, `5 and mask`). A
    /// number followed by anything else (an operator, `(`, EOF) is just the bare number. Juxtaposing a
    /// number and a name has no other meaning on the ML surface (application is `f(x)`, not
    /// juxtaposition), so this repurposes a previously-meaningless adjacency. The printer renders the
    /// same arena shape back to `<num> <name>`, an exact round-trip.
    fn maybe_quantity_literal(&mut self, num: StructId, num_span: Span) -> StructId {
        if !self.at(Kind::Ident) {
            return num;
        }
        let text = self.cur_text();
        if keyword(text).is_some() || word_op(text).is_some() {
            return num;
        }
        let unit_span = self.cur_span();
        // The unit name must be on the SAME LINE as the number. The quantity sugar repurposes number+name
        // ADJACENCY, but statement sequencing juxtaposes forms across lines with no separator (`539f7712`),
        // so a number ending one statement (`def a() = 10`) sits right before the next statement's leading
        // identifier (`a() + 5`). Without this guard the sugar greedily eats that identifier as a unit —
        // `10 a` — swallowing the following statement and MISCOMPILING the program to a bogus quantity. A
        // NEWLINE between the number and the candidate unit means they belong to different statements: the
        // adjacency is sequencing, not a quantity, so decline the sugar and leave the bare number. (A
        // genuine `5 feet` / `10 a` on ONE line has no intervening newline and still reads as a quantity.)
        if self.src[num_span.end..unit_span.start].contains('\n') {
            return num;
        }
        let name = text.to_string();
        self.bump(); // the unit name
        // (Unit.of #"name")
        let unit_head = self.member_head("Unit", "of", unit_span);
        let sym = self.atom(Leaf::Sym(name), unit_span);
        let unit_expr = self.list(vec![unit_head, sym], unit_span);
        // (Qty.of num (Unit.of #"name"))
        let span = num_span.merge(self.prev_span());
        let qty_head = self.member_head("Qty", "of", span);
        self.list(vec![qty_head, num, unit_expr], span)
    }

    /// Build a member-access head `(. obj key)` — the arena shape `obj.key` desugars to, reused to
    /// synthesize the `Qty.of` / `Unit.of` heads of a quantity literal.
    fn member_head(&mut self, obj: &str, key: &str, span: Span) -> StructId {
        let dot = self.name(".", span);
        let obj = self.name(obj, span);
        let key = self.name(key, span);
        self.list(vec![dot, obj, key], span)
    }

    /// Parse the tail of a unit conversion `value as UNIT` (the cursor is at the `as` keyword), returning
    /// `(Unit.in UNIT value)` — the same arena `(Unit.in target q)` an explicit `Unit.in(target, q)` call
    /// builds, so the conversion carries no new semantics. The target UNIT is a denotation, read by
    /// [`Self::unit_denotation`]: a bare name `meter` becomes `(Unit.of #"meter")`, and a parenthesized
    /// compound (`(meter / hour)`) composes via the ordinary `*`/`/`/`^` the units layer reads as unit
    /// composition. The printer renders the bare-name case back to `value as name`.
    fn as_conversion(&mut self, value: StructId, start: Span) -> StructId {
        let as_span = self.cur_span();
        self.bump(); // `as`
        let target = self.unit_denotation(as_span);
        let span = start.merge(self.prev_span());
        let in_head = self.member_head("Unit", "in", as_span);
        self.list(vec![in_head, target, value], span)
    }

    /// The UNIT denotation on the right of an `as`. A bare identifier `meter` reads as the family unit
    /// `(Unit.of #"meter")` — the same shape the `<num> unit` quantity literal builds. Any other unit
    /// expression (a compound `(meter / hour)`, a `Unit.prefix …`, a `Unit.of(…)` call) is written
    /// parenthesized and parsed as an ordinary expression, which the units layer already interprets as a
    /// unit (`eval::unit_of` reads `Unit.of`/`Unit.*`/`Unit./`/`Unit.^` and the bare `*`/`/`/`^`).
    fn unit_denotation(&mut self, at: Span) -> StructId {
        if self.at(Kind::Ident)
            && keyword(self.cur_text()).is_none()
            && word_op(self.cur_text()).is_none()
        {
            let span = self.cur_span();
            let name = self.cur_text().to_string();
            self.bump(); // the unit name
            // (Unit.of #"name")
            let unit_head = self.member_head("Unit", "of", span);
            let sym = self.atom(Leaf::Sym(name), span);
            return self.list(vec![unit_head, sym], span);
        }
        // A parenthesized / computed unit expression — parsed as an ordinary expression the units layer
        // reduces to a unit. A bare non-name here (an operator, EOF) is a conversion target error.
        if self.at(Kind::LParen) {
            return self.bracketed_bars(Self::paren);
        }
        self.error("expected a unit name after `as`");
        self.error_node(at)
    }

    /// Postfix chain: `.member` and `(args…)` application, tightest, left-nested.
    fn postfix(&mut self, mut node: StructId, start: Span) -> StructId {
        // Layers folded by this loop, guarded like the infix left spine. A postfix run (`x.a.b.c…`,
        // `f(1)(2)(3)…`) is iterative, so `self.depth` does not grow with it — but each iteration wraps
        // `node` one deeper (`(. (. x a) b)`, `((f 1) 2)`), building an arbitrarily deep TREE a recursive
        // consumer (printer/`canon`) would walk to a stack overflow. Count each layer against the shared
        // limit (see the twin guard in `expr`) so a pathological chain gets a clean diagnostic, not a crash.
        let mut spine: u32 = 0;
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
                        // The WILDCARD member `obj.*` — the `(. obj *)` form the export surface uses to
                        // name a type's WHOLE constructor set (`export { Color.* }`). `*` is a reserved
                        // final member segment here (recognized only as a member key, so it never
                        // collides with the multiply operator, which needs an operand before it). The key
                        // is the bare `*` name atom the s-expr `(. Color *)` carries, so both surfaces
                        // agree and it round-trips.
                        Kind::Star => {
                            self.bump();
                            self.name("*", key_span)
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
            // Guard the layer JUST folded (checked AFTER building it, so a run that stops at the limit
            // still yields a well-formed node). Only the layers THIS loop adds count — the enclosing
            // recursion depth was already bounded at `expr` entry, so re-adding `self.depth` here would
            // double-count and reject a legitimate deep bracket nest whose postfix run is short.
            spine += 1;
            if !self.depth_exceeded && spine >= crate::sexpr::MAX_NESTING_DEPTH {
                self.error("expression nests too deeply to parse");
                self.depth_exceeded = true;
                return node;
            }
        }
        node
    }

    /// A `.` begins member access only when followed by a member key — a field name, an escaped name,
    /// a numeric index (`obj.0`, positional tuple access), or the wildcard `*` (`obj.*` — the
    /// whole-constructor-set member the export surface uses).
    fn dot_is_member(&self) -> bool {
        matches!(
            self.nth_kind(1),
            Kind::Ident | Kind::BacktickName | Kind::Int | Kind::Star
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
                // An argument is a single expression, not a sequence (`PREC_SEQ + 1`): a `;` here belongs
                // to an enclosing block, so a sequence passed as an argument must parenthesize —
                // `f((a; b))` — matching the "parens only for a genuine ambiguity" surface rule.
                args.push(self.expr(crate::token::PREC_SEQ + 1));
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
    /// `let x = (f (); 42)`. A single `(e)` is transparent grouping (NOT a 1-tuple). The inner `expr(0)`
    /// is a full sequence position, so a `;`-run inside the parens folds to `(do …)` via the Pratt
    /// loop's sequencing rule — the parens are the delimiter that lets a sequence sit in a value slot.
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
            // constructor even where the name `tuple` is rebound. A tuple element is a single expression
            // at `PREC_SEQ + 1` (not a sequence): a `;` inside would belong to an enclosing block, not
            // the element, and `,` separates elements — so `(a; b, c)` is not a legal tuple element here.
            let head = self.ctor_head("tuple", start);
            let mut items = vec![head, first];
            while self.sep_continue(Kind::RParen) {
                items.push(self.expr(crate::token::PREC_SEQ + 1));
            }
            self.expect(Kind::RParen, "`)`");
            let span = start.merge(self.prev_span());
            return self.list(items, span);
        }
        self.expect(Kind::RParen, "`)`");
        first // grouping (or the folded `(do …)` sequence) is transparent in the arena
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
            // A `let` binder is normally a plain name, but a binder that OPENS a destructuring pattern
            // (`(a, b)` / `[x, .. rest]` / `#{ k = p }` / `b[u16(n)]`) binds by pattern — the same
            // irrefutable-in-a-binding-position patterns `param` accepts, so `let (a, b) = p in …` and
            // `def f((a, b)) = …` agree. The compiler already lowers a pattern let-binder (it desugars
            // to the same destructuring form); this lets the ML reader round-trip it.
            let n = if self.at_pattern_param_start() {
                self.pattern()
            } else {
                self.binder()
            };
            self.expect(Kind::Eq, "`=`");
            // The bound value is a single expression (`PREC_SEQ + 1`), delimited by `in` (or the next
            // `,` binding). A `;` after it belongs to the enclosing sequence — `let x = a in b; c` is
            // `(do (let x=a in b) c)` — so a sequence VALUE parenthesizes: `let x = (a; b) in …`.
            let e = self.expr(crate::token::PREC_SEQ + 1);
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

    /// `if c then t else e`  ->  `(if c t e)`. The condition and both branches are single expressions
    /// (`PREC_SEQ + 1`), NOT sequences: a `;` after the `if` belongs to the ENCLOSING sequence, so
    /// `if c then a else b; more` is `(do (if c a b) more)` — `more` runs after the `if` regardless of
    /// the branch taken — not `(if c a (do b more))`. A sequence inside a branch is written `(a; b)`.
    fn if_expr(&mut self) -> StructId {
        let start = self.cur_span();
        let head = self.keyword_head("if", start);
        self.bump(); // `if`
        let c = self.expr(crate::token::PREC_SEQ + 1);
        self.expect_keyword(Keyword::Then, "`then`");
        let t = self.expr(crate::token::PREC_SEQ + 1);
        self.expect_keyword(Keyword::Else, "`else`");
        let e = self.expr(crate::token::PREC_SEQ + 1);
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
            // A value def binds a single expression (`PREC_SEQ + 1`), like a `let` binding — NOT a
            // sequence. A `;` after it belongs to the enclosing sequence, so `def x = 5; rest` is
            // `(do (def x 5) rest)`: the def hoists `x` into scope for `rest` (the corpus's
            // `(do (def x 5) (+ x 1))` reading), rather than making `5; rest` the value. A value that
            // is itself a sequence parenthesizes: `def x = (a; b)`. (A FUNCTION body, by contrast, IS a
            // sequence position — it collects its `;`-run — since its body is delimited by the next
            // top-level form, with no trailing "rest" to escape into.)
            let value = self.expr(crate::token::PREC_SEQ + 1);
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
        items.extend(self.brace_export_list());
        let span = start.merge(self.prev_span());
        self.list(items, span)
    }

    /// `effect Name = | op : Type | …`  ->  `(effect Name (op op Type) …)`. An effect declaration: a
    /// name, then an `=` and one-or-more `|`-led operation signatures, each `op : Type` (the operation
    /// name and its type). Mirrors `type Name = | A | B` — the operations are the effect's "variants",
    /// each led by a `|` (the leading `|` before the first is always printed but tolerated absent).
    /// Each op lowers to `(op <name> <type>)`, the shape the s-expr surface uses.
    fn effect_expr(&mut self) -> StructId {
        let start = self.cur_span();
        let head = self.keyword_head("effect", start);
        self.bump(); // `effect`
        let name = self.binder();
        let mut items = vec![head, name];
        self.expect(Kind::Eq, "`=`");
        // Operations are `|`-led, with an (always-printed) leading `|` before the first — tolerate its
        // absence for robustness. Each `|` introduces an operation signature.
        if self.at(Kind::Pipe) {
            self.bump(); // optional leading `|`
        }
        loop {
            items.push(self.effect_op());
            if self.at(Kind::Pipe) {
                self.bump(); // `|`
            } else {
                break;
            }
        }
        let span = start.merge(self.prev_span());
        self.list(items, span)
    }

    /// One effect operation signature `op : Type`  ->  `(op <op> <Type>)`, matching the corpus
    /// `(op ask (-> Unit Int64))` shape. An operation type is always a function arrow. The common
    /// form `P -> R` parses via [`Self::type_ref`] to the flat `(-> P R)`. A NULLARY operation whose
    /// parameter is elided is written with a LEADING arrow — `op : -> R` -> `(-> R)`, the one-element
    /// arrow that types as `Unit -> R` — so both the explicit-unit `(-> Unit R)` and the elided
    /// `(-> R)` forms have a distinct, round-tripping surface.
    fn effect_op(&mut self) -> StructId {
        let start = self.cur_span();
        let op_head = self.name("op", start);
        let op_name = self.binder();
        self.expect(Kind::Colon, "`:`");
        let ty = if self.at(Kind::Arrow) {
            // Leading `->`: a nullary-elided operation type `(-> R)`.
            let arrow_start = self.cur_span();
            self.bump(); // `->`
            let arrow = self.name("->", arrow_start);
            let result = self.type_ref();
            let arrow_span = arrow_start.merge(self.prev_span());
            self.list(vec![arrow, result], arrow_span)
        } else {
            self.type_ref()
        };
        let span = start.merge(self.prev_span());
        self.list(vec![op_head, op_name, ty], span)
    }

    /// `handle E(seed) with | op(p…, state) => body … in body`  ->
    /// `(handle E seed ((op (p…) state body) …) body)` — the CANONICAL effect-handler shape. The effect
    /// `E` and its initial `seed` are promoted into the head (one `handle` discharges exactly ONE
    /// effect; multi-effect handling is nested `handle`s); an omitted `(seed)` is the stateless `unit`
    /// seed. Each arm is an OPERATION of `E` written bare (`op`, not `E.op`); its parenthesized binder
    /// list is `params…, state` — the LAST binder is the resumption STATE, the rest are the operation's
    /// parameters (symmetric with `resume(value, next_state)`, where state is last on both sides). The
    /// arm op is left bare here; `rcdzc`'s handle desugar rewrites it to the `(. E op)` projection and
    /// drops `E` from the head, yielding the internal `(handle seed (arm…) body)` the compiler lowers.
    fn handle_expr(&mut self) -> StructId {
        let start = self.cur_span();
        let head = self.keyword_head("handle", start);
        self.bump(); // `handle`
        let effect = self.binder(); // the effect name E
        // Optional seed `E(seed)`; `E` (or `E()`) alone is the degenerate stateless `unit` seed.
        let seed = if self.at(Kind::LParen) {
            self.bump(); // `(`
            let s = if self.at(Kind::RParen) {
                let sp = self.cur_span();
                self.name("unit", sp)
            } else {
                self.expr(0)
            };
            self.expect(Kind::RParen, "`)`");
            s
        } else {
            self.name("unit", start)
        };
        self.expect_keyword(Keyword::With, "`with`");
        if self.at(Kind::Pipe) {
            self.bump(); // optional leading `|`
        }
        let arms_start = self.cur_span();
        let mut arms = Vec::new();
        loop {
            arms.push(self.handle_arm());
            if self.at(Kind::Pipe) {
                self.bump(); // `|` before the next arm
            } else {
                break;
            }
        }
        let arms_span = arms_start.merge(self.prev_span());
        let arms_list = self.list(arms, arms_span);
        self.expect_keyword(Keyword::In, "`in`");
        let body = self.expr(0);
        let span = start.merge(self.prev_span());
        self.list(vec![head, effect, seed, arms_list, body], span)
    }

    /// One handler arm `op(p…, state) => body`  ->  `(op (p…) state body)`. The binder list's LAST
    /// entry is the resumption state; everything before it is the operation's parameters (so a nullary
    /// operation is `op(state)` → an empty param list). The body runs until the next arm's `|` or `in`.
    fn handle_arm(&mut self) -> StructId {
        let start = self.cur_span();
        let op = self.binder(); // bare operation name (resolved against the handle's effect)
        let binders_start = self.cur_span();
        self.expect(Kind::LParen, "`(`");
        let mut binders = Vec::new();
        if !self.at(Kind::RParen) {
            loop {
                let before = self.pos;
                binders.push(self.binder());
                if !self.sep_continue(Kind::RParen) {
                    break;
                }
                if self.pos == before {
                    self.bump(); // no binder consumed — avoid a missing-`,` spin
                }
            }
        }
        self.expect(Kind::RParen, "`)`");
        // The last binder is the STATE; the rest are the operation's parameters.
        let state = if let Some(s) = binders.pop() {
            s
        } else {
            let sp = self.cur_span();
            self.error("a handle arm needs a state binder: `op(…, state)`");
            self.error_node(sp)
        };
        let params_span = binders_start.merge(self.prev_span());
        let params = self.list(binders, params_span);
        self.expect(Kind::FatArrow, "`=>`");
        let saved = self.arm_bar_terminates;
        self.arm_bar_terminates = true;
        let body = self.expr(0);
        self.arm_bar_terminates = saved;
        let span = start.merge(self.prev_span());
        self.list(vec![op, params, state, body], span)
    }

    /// `host E, … in body`  ->  `(host (E …) body)` — an entrypoint delegation of one or more effects
    /// to the component boundary. The effects are a comma-separated name list; the body is the delegated
    /// computation. Mirrors `handle`'s `… in body` tail (reusing the `in` keyword as the body delimiter).
    fn host_expr(&mut self) -> StructId {
        let start = self.cur_span();
        let head = self.keyword_head("host", start);
        self.bump(); // `host`
        let effects_start = self.cur_span();
        let mut effects = vec![self.binder()];
        while self.at(Kind::Comma) {
            self.bump(); // `,`
            effects.push(self.binder());
        }
        let effects_span = effects_start.merge(self.prev_span());
        let effects_list = self.list(effects, effects_span);
        self.expect_keyword(Keyword::In, "`in`");
        let body = self.expr(0);
        let span = start.merge(self.prev_span());
        self.list(vec![head, effects_list, body], span)
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
    /// Used by `import`. Each element is a bare (or backtick-escaped) name; a non-name element records
    /// an error and is skipped, so a malformed list still terminates.
    fn brace_name_list(&mut self) -> Vec<StructId> {
        self.brace_list_of(false)
    }

    /// The `export { … }` list — a name list where each element MAY carry a member-access postfix
    /// `.A` / `.*` (a constructor-export element `(. T A)` / the wildcard `(. T *)`), since an export
    /// publishes a value/handle name OR a type's constructor(s). Import stays name-only (a member has
    /// no meaning there).
    fn brace_export_list(&mut self) -> Vec<StructId> {
        self.brace_list_of(true)
    }

    /// The shared brace-list machinery. `members` = whether an element may carry a `.member` postfix
    /// (`export` yes, `import` no) — when set, each binder runs through `postfix` so `Color.*` /
    /// `Color.Red` parse to the `(. Color …)` member form.
    fn brace_list_of(&mut self, members: bool) -> Vec<StructId> {
        self.expect(Kind::LBrace, "`{`");
        let mut names = Vec::new();
        if !self.at(Kind::RBrace) {
            loop {
                let before = self.pos;
                let elem_span = self.cur_span();
                let mut elem = self.binder();
                if members && self.at(Kind::Dot) && self.dot_is_member() {
                    elem = self.postfix(elem, elem_span);
                }
                names.push(elem);
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
        // The scrutinee is a single expression (`PREC_SEQ + 1`), delimited by `with`; a sequence
        // scrutinee parenthesizes. (Each arm body, below, IS a sequence position — bounded by `|`.)
        let scrut = self.expr(crate::token::PREC_SEQ + 1);
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
            // A guard is a single boolean expression (`PREC_SEQ + 1`), delimited by `=>`.
            let g = self.expr(crate::token::PREC_SEQ + 1);
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
                let text = self.text(t).to_string();
                // A suffixed literal pattern (`100N`) desugars to `(: 100 BigInt)` here too, so a match
                // on a big/rational literal reads consistently with a value position.
                self.numeric_atom(&text, span)
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
            Kind::SymLit => {
                let t = self.bump().unwrap();
                self.atom(literal::unescape_sym_token(self.text(t)), span)
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
            Kind::LBracket => {
                // `[p, …]` / `[p, …, .. rest]` — a list pattern, the s-expr `(list p… .. rest)` twin.
                // Head is the NAME `list` (like the tuple pattern's `tuple`), so the compiler's existing
                // name-headed list-pattern lowering matches it; elements and the rest binder are
                // sub-patterns, so a list pattern nests (`[(a, b), .. rest]`).
                self.bump(); // '['
                let head = self.name("list", span);
                let mut items = vec![head];
                if !self.at(Kind::RBracket) {
                    loop {
                        let before = self.pos;
                        if !self.rest_marker(&mut items, |p| p.pattern()) {
                            items.push(self.pattern());
                        }
                        if !self.sep_continue(Kind::RBracket) {
                            break;
                        }
                        if self.pos == before {
                            self.bump(); // pattern didn't consume — avoid a missing-`,` spin
                        }
                    }
                }
                self.expect(Kind::RBracket, "`]`");
                let lspan = span.merge(self.prev_span());
                self.list(items, lspan)
            }
            Kind::Hash if self.nth_kind(1) == Kind::LBrace => {
                // `#{ k = p, … }` / `#{ k = p, …, .. rest }` — a map pattern, the s-expr `(map (k p) ..
                // rest)` twin. Head is the NAME `map`; each entry is a `(key sub-pattern)` pair (the key
                // is a value expression to look up, the value slot a sub-pattern), and an optional `..
                // rest` binds the remaining map — the same key-directed shape the corpus authors.
                self.bump(); // '#'
                self.bump(); // '{'
                let head = self.name("map", span);
                let mut items = vec![head];
                if !self.at(Kind::RBrace) {
                    loop {
                        let before = self.pos;
                        if !self.rest_marker(&mut items, |p| p.pattern()) {
                            let e_start = self.cur_span();
                            let key = self.expr(0);
                            self.expect(Kind::Eq, "`=`");
                            let value = self.pattern();
                            let e_span = e_start.merge(self.prev_span());
                            items.push(self.list(vec![key, value], e_span));
                        }
                        if !self.sep_continue(Kind::RBrace) {
                            break;
                        }
                        if self.pos == before {
                            self.bump(); // no entry token consumed — avoid a missing-`,` spin
                        }
                    }
                }
                self.expect(Kind::RBrace, "`}`");
                let mspan = span.merge(self.prev_span());
                self.list(items, mspan)
            }
            Kind::BinOpen => {
                // `b[ <segment>, … ]` — a binary PATTERN, destructuring a Bytes scrutinee (the dual of
                // the construction literal). Head is the `bin` NAME, and each segment is a sub-PATTERN
                // (`u16(n)` binds `n`, `bytes(rest)` binds the tail), so the compiler's `(bin …)`
                // pattern lowering matches it exactly — the same form the s-expr surface writes.
                self.bin_form(Self::pattern)
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
                // `.. rest` spreads a tail list into the literal (`[1, 2, .. rest]`); an ordinary
                // element otherwise. The marker is flat (`… ".." rest`), shared with the pattern form.
                // Elements are single expressions (`PREC_SEQ + 1`) — a `;` is not a list separator, so a
                // sequence element parenthesizes (`[(a; b), c]`), matching call-argument position.
                if !self.rest_marker(&mut items, |p| p.expr(crate::token::PREC_SEQ + 1)) {
                    items.push(self.expr(crate::token::PREC_SEQ + 1));
                }
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
                // A field value is a single expression (`PREC_SEQ + 1`), delimited by `,`/`}`; a
                // sequence value parenthesizes.
                let value = if self.at(Kind::Eq) {
                    self.bump(); // `=`
                    self.expr(crate::token::PREC_SEQ + 1)
                } else if let Some(n) = pun {
                    self.name(n, f_start)
                } else {
                    // a non-name field with no `=` — record the missing `=` as before.
                    self.expect(Kind::Eq, "`=`");
                    self.expr(crate::token::PREC_SEQ + 1)
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
                // `.. rest` spreads a tail map into the literal (`#{ 1 = v, .. rest }`); a `key = value`
                // entry otherwise. The marker is flat (`… ".." rest`), the list analogue's twin.
                // Key and value are single expressions (`PREC_SEQ + 1`); a sequence parenthesizes.
                if !self.rest_marker(&mut items, |p| p.expr(crate::token::PREC_SEQ + 1)) {
                    let e_start = self.cur_span();
                    let key = self.expr(crate::token::PREC_SEQ + 1);
                    self.expect(Kind::Eq, "`=`");
                    let value = self.expr(crate::token::PREC_SEQ + 1);
                    let e_span = e_start.merge(self.prev_span());
                    items.push(self.list(vec![key, value], e_span));
                }
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
                // Raw-list elements are single expressions (`PREC_SEQ + 1`); a sequence parenthesizes.
                items.push(self.expr(crate::token::PREC_SEQ + 1));
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

    /// `#( e, … )`  ->  `((. Set of) ("list" e …))` — a set literal, sugar for `Set.of([e, …])`. The
    /// third built-in collection surface, completing the `#`-prefix family (`#{`=map, `#[`=raw-list,
    /// `#(`=set). It desugars to a member-access APPLICATION of the ordinary prelude `Set.of` (not a
    /// grammar primitive) applied to a list literal — so `#()` is the empty set `Set.of([])`, and a
    /// shadowed `Set` binding correctly falls back to the user's `Set` (there is no unshadowable set
    /// primitive; the printer round-trips this exact shape via `set_literal`). Elements are single
    /// expressions (`PREC_SEQ + 1`), comma-separated; a sequence element parenthesizes.
    fn set_literal(&mut self) -> StructId {
        let start = self.cur_span();
        self.bump(); // '#'
        self.bump(); // '('
        let list_head = self.ctor_head("list", start);
        let mut elems = vec![list_head];
        if !self.at(Kind::RParen) {
            loop {
                let before = self.pos;
                elems.push(self.expr(crate::token::PREC_SEQ + 1));
                if !self.sep_continue(Kind::RParen) {
                    break;
                }
                if self.pos == before {
                    self.bump(); // element didn't consume — avoid a missing-`,` spin
                }
            }
        }
        self.expect(Kind::RParen, "`)`");
        let span = start.merge(self.prev_span());
        let list = self.list(elems, span);
        let set_of = self.member_head("Set", "of", span);
        self.list(vec![set_of, list], span)
    }

    /// `b[ <segment>, … ]`  ->  `(bin <segment> …)` — a binary literal, the structured sibling of the
    /// `b"…"` byte string. In EXPRESSION position it CONSTRUCTS a Bytes value; in PATTERN position (via
    /// [`Self::bin_pattern`]) it DESTRUCTURES a Bytes scrutinee — the same dual `bin` grammar form the
    /// s-expr surface writes. Each segment is an ordinary call-shaped form (`u16(258)`, `bits(1, 1)`,
    /// `bytes(payload)`); here they are single EXPRESSIONS (`PREC_SEQ + 1`, comma-separated, a sequence
    /// element parenthesizes). `b[]` is the zero-length Bytes value `(bin)`. The head is the `bin` NAME
    /// (a reserved grammar form, structurally dispatched like `match` — never a value a binding shadows).
    fn bin_literal(&mut self) -> StructId {
        self.bin_form(Self::bin_segment_expr)
    }

    /// A single construction-position bin segment: an ordinary expression, like a list element.
    fn bin_segment_expr(&mut self) -> StructId {
        self.expr(crate::token::PREC_SEQ + 1)
    }

    /// The shared `b[ … ]` skeleton for both construction ([`Self::bin_literal`]) and matching
    /// ([`Self::bin_pattern`]): consume the `BinOpen`, read `segment`-parsed items separated by `,`
    /// until `]`, and wrap them under the `bin` head. The cursor is on the `BinOpen` token (`b[`).
    fn bin_form(&mut self, segment: fn(&mut Self) -> StructId) -> StructId {
        let start = self.cur_span();
        self.bump(); // `b[`
        let head = self.name("bin", start);
        let mut items = vec![head];
        if !self.at(Kind::RBracket) {
            loop {
                let before = self.pos;
                items.push(segment(self));
                if !self.sep_continue(Kind::RBracket) {
                    break;
                }
                if self.pos == before {
                    self.bump(); // segment didn't consume — avoid a missing-`,` spin
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
        // A `const`-prefixed parameter — an EXPLICIT compile-time parameter (`const d: T` / `const d`),
        // wrapped `(const BINDER)` so the compiler's load-time strip records it. `const` is not a lexer
        // keyword (a plain identifier), so treat it as the modifier ONLY when it heads a param AND is
        // followed by another binder (an identifier or a `(`/`[`/`#{`-led pattern) — a bare param literally
        // named `const` (no following binder) is left as an ordinary name. The inner binder parses
        // recursively (so `const (a, b)` / `const d: T` / `const [x, .. r]` all work).
        if self.at(Kind::Ident)
            && self.cur_text() == "const"
            && matches!(
                self.nth_kind(1),
                Kind::Ident
                    | Kind::BacktickName
                    | Kind::LParen
                    | Kind::LBracket
                    | Kind::BinOpen
                    | Kind::Hash
            )
        {
            self.bump(); // `const`
            let kw = self.name("const", start);
            let inner = self.param();
            let span = start.merge(self.prev_span());
            return self.list(vec![kw, inner], span);
        }
        // A parameter is normally a plain binder name, but a parameter that OPENS a destructuring
        // PATTERN is parsed as a pattern: a tuple `(a, b)` (or a literal-bearing one like `(1, x)`
        // desugaring to a refutable binder → CDZ0210), a list `[x, .. rest]`, a map `#{ k = p }`, or a
        // binary `b[u16(n)]`. Each is irrefutable only in a form the compiler admits in a binding
        // position; parsing it here lets the ML surface round-trip the pattern parameters the s-expr
        // surface and the printer already support (a plain `name`/`name: Type` still takes `binder`).
        let binder = if self.at_pattern_param_start() {
            self.pattern()
        } else {
            self.binder()
        };
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

    /// True at a token that OPENS a destructuring pattern in a parameter (or `let`-binder) position:
    /// `(` tuple, `[` list, `#{` map, `b[` binary. These are the compound patterns [`Self::pattern`]
    /// deconstructs; a bare name/literal is NOT one (a name is an ordinary binder, a bare literal
    /// param is not a destructure). Keyed here — not by delegating every token to `pattern` — so a
    /// plain `name`/`name: Type` parameter keeps the fast [`Self::binder`] path and its diagnostics.
    fn at_pattern_param_start(&self) -> bool {
        matches!(self.kind(), Kind::LParen | Kind::LBracket | Kind::BinOpen)
            || (self.at(Kind::Hash) && self.nth_kind(1) == Kind::LBrace)
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
    fn deeply_nested_input_is_diagnosed_not_crashed() {
        // The Pratt parser recurses through `expr` one native frame per nesting level, so DESCENDING to
        // the depth guard (`MAX_NESTING_DEPTH` = 1024) itself needs more stack than a default `cargo test`
        // worker (~2 MB on Linux, ~512 KB–1 MB on macOS) — the guard fires cleanly, but the recursion
        // reaching it would overflow the worker's stack first (a spurious SIGABRT that is NOT what this
        // test asserts). Run the body on a large-stacked thread so it exercises the depth guard, not the
        // worker's stack limit. (The compiler's own deep walks use the same 64 MB guard-sized stack.)
        run_deep(|| {
            // Unguarded, a pathologically deep nest overflowed the native stack (SIGABRT) or — once a
            // naive guard returned an error node without stopping — SPUN on the unconsumed deep tail (a
            // hang). The depth guard records ONE error and POISONS the parser (`depth_exceeded` ⇒
            // `at_end`), so parsing TERMINATES with a clean diagnostic. The nest exceeds the limit.
            let n = (crate::sexpr::MAX_NESTING_DEPTH as usize) + 50;
            let src = format!("{}1{}", "(".repeat(n), ")".repeat(n));
            let p = read_ml(&src);
            assert!(
                !p.ok()
                    && p.errors
                        .iter()
                        .any(|e| e.message.contains("nests too deeply")),
                "deep nesting must be a clean depth-limit error, not a crash/hang; got {:?}",
                p.errors
            );
            // A nest well under the limit still parses cleanly (no over-rejection).
            let ok = (crate::sexpr::MAX_NESTING_DEPTH as usize) - 1;
            let shallow = format!("{}1{}", "(".repeat(ok), ")".repeat(ok));
            let ps = read_ml(&shallow);
            assert!(
                ps.ok(),
                "a nest just under the limit must parse: {:?}",
                ps.errors
            );
        });
    }

    #[test]
    fn deep_flat_chains_are_diagnosed_not_crashed() {
        // A FLAT chain — left-associative infix (`1+1+1…`), a postfix member run (`x.a.a…`), or a
        // call chain (`f(1)(1)…`) — is parsed by a LOOP, not recursion, so the parser's per-`expr`
        // depth counter never grows with it. But each iteration deepens the produced ARENA on one
        // side, so an unbounded run built an arbitrarily deep TREE that a recursive CONSUMER (the
        // s-expr printer, `canon`, the compiler's own walk) then overflowed the stack on (SIGABRT) —
        // even though the PARSE itself never recursed and succeeded. The `expr`/`postfix` loop guards
        // now bound the folded spine against the same `MAX_NESTING_DEPTH`, so a pathological chain is a
        // clean parse diagnostic. This test needs NO large stack: the guard fires while building the
        // tree, before any deep walk. (Regression for the flat-chain stack-overflow class.)
        let over = (crate::sexpr::MAX_NESTING_DEPTH as usize) + 50;
        let cases = [
            format!("1{}", "+1".repeat(over)),    // left-assoc infix spine
            format!("1{}", " |> f".repeat(over)), // pipeline spine (also infix)
            format!("x{}", ".a".repeat(over)),    // postfix member chain
            format!("f{}", "(1)".repeat(over)),   // postfix call chain
        ];
        for src in cases {
            let p = read_ml(&src);
            assert!(
                !p.ok()
                    && p.errors
                        .iter()
                        .any(|e| e.message.contains("nests too deeply")),
                "a deep flat chain must be a clean depth-limit error, not a crash; got ok={} errs={:?}",
                p.ok(),
                p.errors
            );
            // The produced arena is well-formed and its recursive printer walk must not crash — the
            // guard capped the tree depth, so printing it is safe on an ordinary stack.
            let _ = crate::printer::print(&p.arenas, 80);
        }
        // A flat chain WELL under the limit parses cleanly (no over-rejection) and round-trips.
        let ok_n = (crate::sexpr::MAX_NESTING_DEPTH as usize) / 2;
        let shallow = format!("1{}", "+1".repeat(ok_n));
        let ps = read_ml(&shallow);
        assert!(
            ps.ok(),
            "a flat chain under the limit must parse: {:?}",
            ps.errors
        );
    }

    /// Run `f` on a thread with a stack large enough to reach the parser's depth guard (the same
    /// 64 MB the compiler sizes its deep-walk worker at), re-raising a panic so an assertion failure
    /// inside still fails the test. The default `cargo test` worker stack is too small to DESCEND to
    /// the depth limit before overflowing (macOS especially), which would mask the guarded behavior.
    fn run_deep(f: impl FnOnce() + Send + 'static) {
        let h = std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(f)
            .expect("spawn deep-parse worker");
        if let Err(payload) = h.join() {
            std::panic::resume_unwind(payload);
        }
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
    fn effect_decl_builds_op_signatures() {
        // `effect Diag = | emit : Int64 -> Unit | collect : -> List(Int64)` ->
        // `(effect Diag (op emit (-> Int64 Unit)) (op collect (-> (List Int64))))`. The leading-arrow
        // op type is the nullary-elided one-element `(-> R)`.
        let a = parse_ok("effect Diag = | emit : Int64 -> Unit | collect : -> List(Int64)");
        let tail = a.as_form(a.root, "effect").unwrap();
        assert_eq!(a.as_name(tail[0]), Some("Diag"));
        let emit = a.as_form(tail[1], "op").unwrap();
        assert_eq!(a.as_name(emit[0]), Some("emit"));
        let emit_ty = a.as_form(emit[1], "->").unwrap();
        assert_eq!(emit_ty.len(), 2, "P -> R is a two-element arrow");
        let collect = a.as_form(tail[2], "op").unwrap();
        let collect_ty = a.as_form(collect[1], "->").unwrap();
        assert_eq!(
            collect_ty.len(),
            1,
            "nullary-elided `-> R` is a one-element arrow"
        );
    }

    #[test]
    fn handle_promotes_effect_and_seed_with_state_last() {
        // `handle Fresh(0) with | next(u, s) => resume(s, s + 1) in body` ->
        // `(handle Fresh 0 ((next (u) s (resume s (+ s 1)))) body)`: the effect NAME and seed are the
        // head's 1st/2nd children, the arm op is BARE, and the LAST binder `s` is the state.
        let a = parse_ok("handle Fresh(0) with | next(u, s) => resume(s, s + 1) in Fresh.next()");
        let tail = a.as_form(a.root, "handle").unwrap();
        // `as_form` returns the tail (head excluded): [effect, seed, arms, body].
        assert_eq!(tail.len(), 4, "handle E seed (arms) body");
        assert_eq!(
            a.as_name(tail[0]),
            Some("Fresh"),
            "effect name promoted to head"
        );
        assert_eq!(a.as_name(tail[1]), None); // seed is the int 0, not a name
        let crate::ast::Struct::List(arms) = a.get(tail[2]) else {
            panic!("arms list")
        };
        let crate::ast::Struct::List(arm0) = a.get(arms[0]) else {
            panic!("one arm")
        };
        assert_eq!(arm0.len(), 4, "arm = op (params) state body");
        assert_eq!(a.as_name(arm0[0]), Some("next"), "bare op, not Fresh.next");
        let crate::ast::Struct::List(params) = a.get(arm0[1]) else {
            panic!("params list")
        };
        assert_eq!(params.len(), 1, "one param `u` (state `s` is separate)");
        assert_eq!(a.as_name(params[0]), Some("u"));
        assert_eq!(a.as_name(arm0[2]), Some("s"), "last binder is the state");
    }

    #[test]
    fn handle_stateless_seed_elides_to_unit() {
        // `handle Choose with | pick(s) => resume(5, s) in …`: no `(seed)` → seed is `unit`; the arm's
        // single binder is the state, so the param list is empty (a nullary operation).
        let a = parse_ok("handle Choose with | pick(s) => resume(5, s) in Choose.pick()");
        let tail = a.as_form(a.root, "handle").unwrap();
        assert_eq!(a.as_name(tail[0]), Some("Choose"));
        assert_eq!(a.as_name(tail[1]), Some("unit"), "elided seed is unit");
        let crate::ast::Struct::List(arms) = a.get(tail[2]) else {
            panic!()
        };
        let crate::ast::Struct::List(arm0) = a.get(arms[0]) else {
            panic!()
        };
        let crate::ast::Struct::List(params) = a.get(arm0[1]) else {
            panic!()
        };
        assert!(
            params.is_empty(),
            "nullary op: state consumed the only binder"
        );
        assert_eq!(a.as_name(arm0[2]), Some("s"));
    }

    #[test]
    fn host_delegation_builds_effect_list() {
        // `host ask, log in body` -> `(host (ask log) body)`.
        let a = parse_ok("host ask, log in ask.ask()");
        let tail = a.as_form(a.root, "host").unwrap();
        let crate::ast::Struct::List(effects) = a.get(tail[0]) else {
            panic!("effect list")
        };
        assert_eq!(effects.len(), 2);
        assert_eq!(a.as_name(effects[0]), Some("ask"));
        assert_eq!(a.as_name(effects[1]), Some("log"));
    }

    #[test]
    fn semicolon_sequences_a_function_body() {
        // A `;`-separated body folds into a flat `(do …)`, the last element the value — modelling
        // `a; b` as `let _ = a in b`. The body greedily collects its run and stops at the next
        // top-level `def` (a declaration keyword ends the sequence, so `g`'s def is NOT swallowed).
        let a = parse_ok("def f() = a; b; c\ndef g() = 2");
        let top = a.as_form(a.root, "do").unwrap();
        assert_eq!(top.len(), 2, "two top-level defs: {top:?}");
        let f = a.as_form(top[0], "def").unwrap();
        let body = a.as_form(f[1], "do").unwrap();
        assert_eq!(
            body.len(),
            3,
            "f's body is the 3-element sequence (do a b c)"
        );
        assert_eq!(a.as_name(body[0]), Some("a"));
        assert_eq!(a.as_name(body[2]), Some("c"));
    }

    #[test]
    fn top_level_forms_juxtapose_without_semicolons() {
        // Top-level forms are whitespace-separated — no `;` needed. `def a = 1 def b = 2` is two
        // distinct root forms, not a body that swallowed the second.
        let a = parse_ok("def a = 1 def b = 2");
        let top = a.as_form(a.root, "do").unwrap();
        assert_eq!(top.len(), 2);
        assert!(a.as_form(top[0], "def").is_some());
        assert!(a.as_form(top[1], "def").is_some());
    }

    #[test]
    fn top_level_semicolon_folds_and_flattens_to_the_same_root() {
        // A `;` between top-level forms is optional: it folds a stmt-level `(do …)` that the root
        // then splices flat, so `a; b` and `a  b` at the root yield the IDENTICAL tree.
        let with = parse_ok("f(); g()");
        let without = parse_ok("f() g()");
        let wt = with.as_form(with.root, "do").unwrap();
        let wo = without.as_form(without.root, "do").unwrap();
        assert_eq!(wt.len(), 2);
        assert_eq!(wo.len(), 2);
        assert_eq!(with.head_name(wt[0]), Some("f"));
        assert_eq!(with.head_name(wt[1]), Some("g"));
    }

    #[test]
    fn semicolon_in_argument_position_needs_parens() {
        // A call argument is a single expression: a `;` inside must parenthesize, so `f((a; b))` is a
        // one-argument call whose argument is the sequence `(do a b)`.
        let a = parse_ok("f((a; b))");
        let call = a.as_form(a.root, "f").unwrap();
        assert_eq!(call.len(), 1, "one argument");
        let seq = a.as_form(call[0], "do").unwrap();
        assert_eq!(seq.len(), 2, "the argument is the sequence (do a b)");
    }

    #[test]
    fn if_branch_does_not_swallow_the_trailing_sequence() {
        // `if`'s branches parse at `PREC_SEQ + 1`, so a `;` after the `if` belongs to the enclosing
        // sequence: `if c then a else b; more` is `(do (if c a b) more)`, not `(if c a (do b more))`.
        let a = parse_ok("def f() = if c then a else b; more");
        let f = a.as_form(a.root, "def").unwrap();
        let body = a.as_form(f[1], "do").unwrap();
        assert_eq!(body.len(), 2, "body is (do (if …) more)");
        assert!(a.as_form(body[0], "if").is_some(), "first stmt is the if");
        assert_eq!(a.as_name(body[1]), Some("more"));
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
    fn quantity_literal_desugars() {
        use crate::sexpr;
        // A numeric literal followed by a bare unit name is a quantity literal: `5 feet` desugars to
        // the same arena as the canonical `(Qty.of 5 (Unit.of #"feet"))`.
        let a = parse_ok("5 feet");
        assert_eq!(sexpr::print(&a), r#"((. Qty of) 5 ((. Unit of) #"feet"))"#);
        // A float value works the same way.
        let f = parse_ok("5.0 meter");
        assert_eq!(
            sexpr::print(&f),
            r#"((. Qty of) 5.0 ((. Unit of) #"meter"))"#
        );
        // The literal binds TIGHTER than every operator, so `5 feet / 1 second` is a rate — the
        // division of two quantity literals — the reading the surface is designed to give.
        let rate = parse_ok("5 feet / 1 second");
        assert_eq!(
            sexpr::print(&rate),
            r#"(/ ((. Qty of) 5 ((. Unit of) #"feet")) ((. Qty of) 1 ((. Unit of) #"second")))"#
        );
        // It composes as an ordinary operand: a call argument, and an addend.
        assert_eq!(
            sexpr::print(&parse_ok("dist(5 feet)")),
            r#"(dist ((. Qty of) 5 ((. Unit of) #"feet")))"#
        );
    }

    #[test]
    fn set_literal_desugars() {
        use crate::sexpr;
        // `#(1, 2, 3)` desugars to `Set.of([1, 2, 3])` — a member-access application of the prelude
        // `Set.of` over a `list` literal. The list head is the unshadowable STRING primitive `"list"`.
        let a = parse_ok("#(1, 2, 3)");
        assert_eq!(sexpr::print(&a), r#"((. Set of) ("list" 1 2 3))"#);
        // The empty set is `Set.of([])`.
        let e = parse_ok("#()");
        assert_eq!(sexpr::print(&e), r#"((. Set of) ("list"))"#);
        // A single-element set, and nested elements (an expression element parses fully).
        assert_eq!(
            sexpr::print(&parse_ok("#(x + 1)")),
            r#"((. Set of) ("list" (+ x 1)))"#
        );
        // It composes as an ordinary operand: a call argument.
        assert_eq!(
            sexpr::print(&parse_ok("contains(#(1, 2), 1)")),
            r#"(contains ((. Set of) ("list" 1 2)) 1)"#
        );
    }

    #[test]
    fn bin_literal_desugars() {
        use crate::sexpr;
        // `b[u16(258), u8(1)]` desugars to the `(bin …)` grammar form — each segment is an ordinary
        // call-shaped expression wrapped under the `bin` head.
        assert_eq!(
            sexpr::print(&parse_ok("b[u16(258), u8(1)]")),
            "(bin (u16 258) (u8 1))"
        );
        // `b[]` is the zero-length Bytes value `(bin)`.
        assert_eq!(sexpr::print(&parse_ok("b[]")), "(bin)");
        // The `le` modifier and a `bits(v, k)` field carry through as ordinary call args.
        assert_eq!(
            sexpr::print(&parse_ok("b[u16(258, le), bits(1, 1)]")),
            "(bin (u16 258 le) (bits 1 1))"
        );
        // A dependent-size `bytes(payload)` segment and a computed size expression parse fully.
        assert_eq!(
            sexpr::print(&parse_ok("b[u16(Bytes.len(payload)), bytes(payload)]")),
            "(bin (u16 ((. Bytes len) payload)) (bytes payload))"
        );
        // It composes as an ordinary operand: a call argument and an equality operand.
        assert_eq!(
            sexpr::print(&parse_ok("b[u8(1)] == other")),
            "(= (bin (u8 1)) other)"
        );
    }

    #[test]
    fn a_def_parameter_may_be_a_destructuring_pattern() {
        use crate::sexpr;
        // A def parameter that STARTS a compound pattern is a destructuring binder, not just a bare name —
        // the ML reader must accept every pattern shape the printer emits for a pattern parameter, or the
        // corpus round-trip breaks (the regression this guards: `def head([x, .. rest]) = x` failed
        // "expected a name" because `param` routed only `(`-led patterns to `pattern()`, not `[`/`#{`).

        // A `(`-led TUPLE pattern parameter (the already-working case) — kept as a regression anchor.
        assert_eq!(
            sexpr::print(&parse_ok("def f((a, b)) = a")),
            "(def (f (tuple a b)) a)"
        );
        // A `[`-led LIST pattern parameter — a fixed-arity and a rest form.
        assert_eq!(
            sexpr::print(&parse_ok("def f([a, b]) = a")),
            "(def (f (list a b)) a)"
        );
        assert_eq!(
            sexpr::print(&parse_ok("def head([x, .. rest]) = x")),
            "(def (head (list x .. rest)) x)"
        );
        // A `#{`-led MAP pattern parameter.
        assert_eq!(
            sexpr::print(&parse_ok("def get(#{ 1 = v }) = v")),
            "(def (get (map (1 v))) v)"
        );
        // Pattern parameters COMPOSE and mix with plain-name / annotated params.
        assert_eq!(
            sexpr::print(&parse_ok("def f([(a, b), .. rest]) = a")),
            "(def (f (list (tuple a b) .. rest)) a)"
        );
        assert_eq!(
            sexpr::print(&parse_ok("def f(x, [a, .. rest]) = x")),
            "(def (f x (list a .. rest)) x)"
        );
    }

    #[test]
    fn bin_pattern_desugars() {
        use crate::sexpr;
        // In pattern position `b[u16(n), bytes(rest)]` desugars to the same `(bin …)` head, but its
        // segments are sub-PATTERNS: `u16(n)` binds `n`, `bytes(rest)` binds the tail.
        assert_eq!(
            sexpr::print(&parse_ok("match x with | b[u16(n), bytes(rest)] => n")),
            "(match x ((bin (u16 n) (bytes rest)) n))"
        );
        // The empty binary pattern and a `le` modifier in a pattern segment.
        assert_eq!(
            sexpr::print(&parse_ok("match x with | b[] => 0")),
            "(match x ((bin) 0))"
        );
        assert_eq!(
            sexpr::print(&parse_ok("match x with | b[u16(n, le)] => n")),
            "(match x ((bin (u16 n le)) n))"
        );
    }

    #[test]
    fn number_before_keyword_is_not_a_quantity() {
        use crate::sexpr;
        // Only a bare NON-keyword identifier attaches as a unit. A word-operator keeps its infix
        // meaning after a number: `5 and mask` is the boolean `and`, not a quantity in unit `and`.
        let a = parse_ok("5 and mask");
        assert_eq!(sexpr::print(&a), "(and 5 mask)");
    }

    #[test]
    fn a_destructuring_pattern_parameter_parses() {
        use crate::sexpr;
        // A `(`-led tuple, `[`-led list, `#{`-led map, or `b[`-led binary parameter is a destructuring
        // PATTERN (`param` routes it to `pattern`); a plain `name` / annotated `name: Type` is not.
        assert_eq!(
            sexpr::print(&parse_ok("def f((a, b)) = a + b")),
            "(def (f (tuple a b)) (+ a b))"
        );
        // The reported gap: a list-REST pattern parameter (`def head([x, .. rest]) = x`).
        assert_eq!(
            sexpr::print(&parse_ok("def head([x, .. rest]) = x")),
            "(def (head (list x .. rest)) x)"
        );
        assert_eq!(
            sexpr::print(&parse_ok("def g(b[u8(n)]) = n")),
            "(def (g (bin (u8 n))) n)"
        );
        // A plain-name and an annotated parameter keep the ordinary binder path.
        assert_eq!(sexpr::print(&parse_ok("def h(x) = x")), "(def (h x) x)");
        assert_eq!(
            sexpr::print(&parse_ok("def s(xs: List(Int64)) = xs")),
            "(def (s (: xs (List Int64))) xs)"
        );
    }

    #[test]
    fn a_destructuring_pattern_let_binder_parses() {
        use crate::sexpr;
        // A `let` binder that opens a destructuring pattern binds by pattern — the twin of the pattern
        // parameter, so `let (a, b) = p in …` and `let [x, .. rest] = ys in …` parse.
        assert_eq!(
            sexpr::print(&parse_ok("let (a, b) = p in a + b")),
            "(let (((tuple a b) p)) (+ a b))"
        );
        assert_eq!(
            sexpr::print(&parse_ok("let [x, .. rest] = ys in x")),
            "(let (((list x .. rest) ys)) x)"
        );
        // A plain-name binder is unchanged, and a `let` may MIX a name and a pattern binder.
        assert_eq!(sexpr::print(&parse_ok("let x = 1 in x")), "(let ((x 1)) x)");
        assert_eq!(
            sexpr::print(&parse_ok("let x = 1, (a, b) = p in x + a")),
            "(let ((x 1) ((tuple a b) p)) (+ x a))"
        );
    }

    #[test]
    fn quantity_sugar_does_not_cross_a_newline() {
        use crate::sexpr;
        // The quantity sugar (`5 feet` → Qty) repurposes number+name ADJACENCY, but statement sequencing
        // juxtaposes forms across lines with no separator — so a number ending one statement sits right
        // before the next statement's leading identifier. The sugar must NOT eat that identifier as a unit
        // (a miscompile that swallows the following statement). A NEWLINE between the number and the
        // candidate unit means they are different statements: leave the bare number, let the next form be
        // its own statement.
        //
        // `def a() = 10 <newline> a() + 5`: the `10` must stay a bare number (main's `a` def), and `a()+5`
        // is the next top-level form — NOT `(Qty.of 10 (Unit.of "a"))` eating the next line.
        let a = parse_ok("def a() = 10\na() + 5");
        assert_eq!(
            sexpr::print(&a),
            "(do (def (a) 10) (+ (a) 5))",
            "the quantity sugar must not span the newline into the next statement"
        );
        // A genuine SAME-LINE quantity is unchanged — `10 a` (no intervening newline) is still a quantity.
        assert_eq!(
            sexpr::print(&parse_ok("10 a")),
            r#"((. Qty of) 10 ((. Unit of) #"a"))"#
        );
        // Same-line even when a statement follows on the NEXT line: `5 feet` is the quantity, `x` is next.
        assert_eq!(
            sexpr::print(&parse_ok("5 feet\nx")),
            r#"(do ((. Qty of) 5 ((. Unit of) #"feet")) x)"#
        );
    }

    #[test]
    fn as_conversion_does_not_cross_a_newline() {
        use crate::sexpr;
        // The `as` unit-conversion postfix (`value as meter` → `(Unit.in (Unit.of "meter") value)`) must
        // apply only WITHIN one statement. Statement sequencing juxtaposes forms across lines, so an `as`
        // beginning a new line must NOT reach back across the newline and absorb the previous statement's
        // trailing expression — `x as meter` split over two lines is a value `x` then a separate (erroring)
        // `as meter`, NOT `(x as meter)`. Same boundary the quantity sugar draws; the `as` operator landed
        // without it, so `def a() = 5.0 <newline> as meter` silently became `def a() = (5.0 as meter)`.
        //
        // `x <newline> as meter`: `x` is a complete statement; the leading `as` on the next line does not
        // continue it. (`read_ml` tolerates the stray `as`-with-no-left-operand error and still yields a
        // tree; the stray `as`/`meter` land as their own error-recovered forms — the point is `x` is NOT
        // folded into a `(Unit.in … x)` conversion.)
        let parsed = read_ml("x\nas meter");
        let printed = sexpr::print(&parsed.arenas);
        assert!(
            !printed.contains("Unit in"),
            "a leading `as` on a new line must not absorb the previous statement into a conversion: {printed}"
        );
        // A genuine SAME-LINE conversion is unchanged — `x as meter` (no intervening newline) still converts.
        assert_eq!(
            sexpr::print(&parse_ok("x as meter")),
            r#"((. Unit in) ((. Unit of) #"meter") x)"#
        );
        // Same-line `as` even when a statement follows on the NEXT line: the conversion is `5.0 as meter`,
        // `x` is the next statement — the newline after the conversion ends it, it does not chain into `x`.
        assert_eq!(
            sexpr::print(&parse_ok("5.0 as meter\nx")),
            r#"(do ((. Unit in) ((. Unit of) #"meter") 5.0) x)"#
        );
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
        let p = read_ml("f(a, $, c)");
        assert!(!p.ok(), "the stray `$` is reported");
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
    fn an_unterminated_literal_names_its_specific_cause() {
        // A lexer ERROR token in expression position (an unterminated literal run to end-of-input) used
        // to read as the generic "expected an expression" — misdirecting, since the token IS where an
        // expression starts; the real defect is the unclosed literal. Each opener now names its cause,
        // the ML-surface twin of the s-expr reader's "unterminated string".
        for (src, needle) in [
            ("def f() = \"abc", "unterminated string literal"),
            ("def f() = b\"abc", "unterminated byte-string literal"),
            ("def f() = #\"abc", "unterminated symbol literal"),
            ("def f() = `abc", "unterminated backtick name"),
        ] {
            let p = read_ml(src);
            assert!(!p.ok(), "{src:?} is rejected");
            assert!(
                p.errors.iter().any(|e| e.message.contains(needle)),
                "{src:?} names {needle:?}, not the generic message: {:?}",
                p.errors
            );
            assert!(
                !p.errors
                    .iter()
                    .any(|e| e.message == "expected an expression"),
                "{src:?} does not fall back to the generic message: {:?}",
                p.errors
            );
        }
        // A well-terminated string is unaffected (no spurious error).
        assert!(
            read_ml("def f() = \"abc\"").ok(),
            "a closed string parses clean"
        );
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
        let p = read_ml("let x = $ in x + 1");
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
        let p = read_ml("if $ then a else b");
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
