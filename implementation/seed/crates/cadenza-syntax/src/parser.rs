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

use crate::ast::{Arenas, Builder, CompoundCtor, Leaf, StructId};
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

/// The RESERVED set of first-class embedded-syntax grammar tags. A `Ident` matching one of these,
/// GLUED to a `{` (see the `prefix` guard), switches the parser into that sub-grammar; the region is
/// parsed by the named surface's own reader and grafted as `(embedded <tag> <subtree>)`. Returning the
/// canonical grammar name (a `#tag` symbol leaf's word) keeps the set the ONE place a tag is decided —
/// everything NOT in this set stays an ordinary name / a v-metaprogramming template tag, so the
/// front-end switch and library-level tagged templates never collide (the reserved-set boundary the
/// design locks with v-metaprogramming). JSON + TOML first (operator sequencing); markdown/jsx follow.
fn embedded_grammar(tag: &str) -> Option<&'static str> {
    match tag {
        "json" => Some("json"),
        "toml" => Some("toml"),
        _ => None,
    }
}

/// The WIT primitive type names an inline world member's type may name directly (lowered to a
/// `(name)` primitive descriptor via [`crate::ast::Builder::wit_type_prim`]). The closed set the
/// kernel's `build_type` produces primitives for (component-model scalars + `string`); a bare name
/// NOT in this set is left as an ordinary type node (not a WIT descriptor). Kept in sync with
/// `build_type`'s `prim(...)` arms.
fn is_wit_primitive(name: &str) -> bool {
    matches!(
        name,
        "bool"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "s8"
            | "s16"
            | "s32"
            | "s64"
            | "char"
            | "string"
            | "f32"
            | "f64"
    )
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
    /// TRAILING = the comment began on the SAME source line as the PREVIOUS grammar token (code before
    /// it: `x // note`), as opposed to LEADING = on its own line above the next token. A trailing comment
    /// documents the thing it FOLLOWS, so a drain site can attach it to the preceding node (as
    /// `(comment-after "text" node)`) and the printer re-emits it same-line — instead of stranding it at
    /// the next token's leading slot (where an interior position drops it). `///` docs are never trailing
    /// (a doc leads its item); only `//` comments carry this.
    trailing: bool,
    text: String,
    span: Span,
}

/// Build a [`Parser`] over `src` — tokenize + split out the `leading` doc/comment side-table — WITHOUT
/// running the grammar. Extracted from [`parse`] so the recursive [`parse`] and the iterative rewrite
/// (`parse_iterative`, and the `expr`-vs-`expr_iter` differential unit tests) share the identical
/// lexing/leading setup and differ ONLY in which grammar driver they run.
fn build_parser(src: &str, file: FileId) -> Parser<'_> {
    // Split the lexer stream into grammar tokens (everything the parser already handled) and a
    // parallel `leading` side-table: `leading[i]` is the run of doc/comment tokens that immediately
    // preceded grammar token `i`. The parser proper sees ONLY grammar tokens (unchanged); it
    // consults `leading` at definition boundaries to attach docs/comments. Comments no longer
    // vanish — they are captured here and re-emitted as `(doc …)` / `(comment …)` nodes.
    let mut tokens: Vec<Token> = Vec::new();
    let mut leading: Vec<Vec<Lead>> = Vec::new();
    let mut pending: Vec<Lead> = Vec::new();
    // The END offset of the most recent GRAMMAR token — to classify a following comment as TRAILING
    // (same source line as that token, no newline between) vs LEADING (its own line). `None` before the
    // first grammar token (a file-leading comment is never trailing).
    let mut prev_grammar_end: Option<usize> = None;
    for t in Lexer::new(src) {
        match t.kind {
            Kind::Whitespace => {}
            Kind::LineComment | Kind::DocComment => {
                let doc = t.kind == Kind::DocComment;
                // Trailing iff a grammar token precedes it on the SAME line: no `\n` in the source gap
                // between that token's end and this comment's start. A `///` doc is never trailing (a doc
                // leads its item, even if written after code — treat it as leading for attachment).
                let trailing = !doc
                    && match prev_grammar_end {
                        Some(end) if end <= t.span.start => !src[end..t.span.start].contains('\n'),
                        _ => false,
                    };
                pending.push(Lead {
                    doc,
                    trailing,
                    text: strip_comment(&src[t.span.start..t.span.end], doc),
                    span: t.span,
                });
            }
            _ => {
                tokens.push(t);
                prev_grammar_end = Some(t.span.end);
                leading.push(std::mem::take(&mut pending));
            }
        }
    }
    // A trailing run of comments with no following grammar token (e.g. a comment on the last line)
    // attaches to the virtual end position.
    let trailing = pending;

    Parser {
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
        iterative: false,
    }
}

/// Parse `src` (in file `file`) to arenas + spans — the RECURSIVE-descent parser. Kept as the frozen
/// reference the differential oracle diffs the iterative rewrite against (see [`read_ml_recursive`]);
/// deleted once the rewrite is complete.
pub fn parse(src: &str, file: FileId) -> Parsed {
    let mut p = build_parser(src, file);
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

/// The ITERATIVE parse entry point (v-syntax-nonrec-reader I3) — the explicit-worklist replacement for
/// the recursive-descent parser, being built up construct-by-construct. [`read_ml`] routes here; the
/// frozen recursive [`parse`] stays as the differential-oracle reference ([`read_ml_recursive`]) until
/// the rewrite is complete. WHILE the iterative engine is incomplete this delegates to [`parse`] so its
/// output is byte-identical (the oracle + corpus stay green); each increment replaces more of the
/// delegation with explicit-stack control flow, verified against the recursive reference over the
/// differential oracle + `corpus_roundtrip`. When complete, `parse` and its recursive methods are
/// deleted and this becomes the sole parser.
pub fn parse_iterative(src: &str, file: FileId) -> Parsed {
    let mut p = build_parser(src, file);
    // Route every `expr` through the iterative `expr_iter` shunting-yard. Operand reads still go through
    // the (recursive) prefix/postfix, so bracket/keyword operand NESTING still recurses until those forms
    // are pulled onto the worklist in a later increment — but the infix / right-operand / right-assoc
    // chains are now iterative, and expr_iter is exercised end-to-end by the differential oracle + corpus.
    p.iterative = true;
    let root = p.program();
    Parsed {
        arenas: p.builder.finish(root),
        spans: p.spans,
        errors: p.errors,
    }
}

/// Parse `src` as an anonymous single file (`FileId(0)`).
pub fn read_ml(src: &str) -> Parsed {
    parse_iterative(src, FileId::default())
}

/// The FROZEN RECURSIVE-parser reference for the differential oracle (see
/// `roundtrip_tests::generative_roundtrip`), used while the ML parser is converted from recursive
/// descent to an explicit worklist. At present it is identical to [`read_ml`] (both run the recursive
/// `parse`), so the oracle is a green passthrough. When a later increment makes [`read_ml`]/`parse`
/// ITERATIVE (adding iterative methods ALONGSIDE the recursive ones, not rewriting in place), this is
/// repointed to the preserved recursive entry so it stays the recursive baseline the oracle diffs the
/// new iterative output against byte-for-byte (arenas + span table + errors). Removed once the rewrite
/// is complete and soaked. Test-only — it never ships in a non-test build.
#[cfg(test)]
pub(crate) fn read_ml_recursive(src: &str) -> Parsed {
    parse(src, FileId::default())
}

/// A record/map field-pair the iterative `expr_iter` has started reading but must SUSPEND on to descend
/// into a sub-expr (the worklist twin of `record_literal`/`map_literal`'s inline field loop). It records
/// which sub-expr the pending descent will deliver, plus the partial field state to reassemble once it
/// does — the `(= name value)` `FieldPair` node is built on DELIVER (matching the recursive struct-id
/// order: name/key then value subtree, THEN the `=` atom, then the field `list`). A record SHORTHAND
/// field (`{ x }` → `(= x x)`) needs NO descent, so `advance_fields` completes it inline and never yields
/// a `FieldPhase` for it.
enum FieldPhase {
    /// A `.. rest` spread: `head` is the pre-created `..` name; the pending descent is its operand.
    RestOperand { dd_span: Span, rest_head: StructId },
    /// A map `key = value` entry whose KEY is being read; on deliver, expect `=` then descend the value.
    MapKey { leading: Vec<Lead>, e_start: Span },
    /// A map entry whose VALUE is being read; `key` is the already-read key node.
    MapValue {
        leading: Vec<Lead>,
        e_start: Span,
        key: StructId,
    },
    /// A record `name = value` field whose VALUE is being read; `name` is the already-read binder node.
    RecordValue {
        leading: Vec<Lead>,
        f_start: Span,
        name: StructId,
    },
}

/// What a `{ … }` record PATTERN field needs to DESCEND (a sub-pattern) on the pattern worklist, once its
/// inline preamble is read (via `advance_record_pat`). A `.. rest` spread descends its rest operand; a
/// `field = <pat>` descends its value; a shorthand `{ x }` (`= x x`) needs NO descent and is completed
/// inline. `before` is the field-loop missing-`,` progress guard.
enum RecordPatDescend {
    Rest {
        dd_span: Span,
        rest_head: StructId,
        before: usize,
    },
    Value {
        f_start: Span,
        field: StructId,
        before: usize,
    },
}

/// The sub-expr an `if c then t else e` form is reading when it SUSPENDS on the worklist (the twin of
/// `if_expr`'s three sequential `expr` calls). Each phase carries the already-built branches plus the
/// own-line leading comments captured for the branch currently being read (`expr` does not drain a
/// sub-expr's own leading slot). On deliver, `expr_iter` wraps the branch (leading + a same-line trailing
/// comment on then/else), then either advances to the next `then`/`else` keyword + descends, or (after
/// `else`) assembles `(if c t e)`. Byte-identical to `if_expr` (node order: head, then cond/then/else
/// subtrees + their comment wrappers in turn, then the `list`).
enum IfPhase {
    /// Reading the condition; `c_lead` is its own-line leading comment run (captured before the descent).
    Cond { c_lead: Vec<Lead> },
    /// Reading the then-branch; `c` is the built condition, `t_lead` the then-branch's leading comments.
    Then { c: StructId, t_lead: Vec<Lead> },
    /// Reading the else-branch; `c`/`t` are the built condition/then, `e_lead` the else leading comments.
    Else {
        c: StructId,
        t: StructId,
        e_lead: Vec<Lead>,
    },
}

/// The sub-expr a `let b = v, … in body` form is reading when it SUSPENDS on the worklist (the twin of
/// `let_expr`'s per-binding value `expr` + the body `expr`). Every binding's VALUE is a descent (no
/// inline-completable binding, unlike a record shorthand); the pattern/name binder is read INLINE (a flat
/// leaf, or the recursive `pattern()` for a destructuring binder — its own separate depth guard, I4). On
/// deliver, `expr_iter` builds the `(binder value)` pair, then either starts the next `,`-binding (inline
/// preamble + descend) or finishes the binding list, consumes `in`, and descends the body. Byte-identical
/// to `let_expr` (node order: head, per-binding binder then value subtree then the pair, the `binds` list,
/// then the body subtree, then the outer `list`).
enum LetPhase {
    /// Reading a binding's value; `bindings` are the built pairs so far, `n` the just-read binder, and
    /// `leading`/`b_start`/`e_lead` the binding's own-line leading comments / start span / value-leading
    /// comments captured before the descent.
    BindingValue {
        bindings: Vec<StructId>,
        n: StructId,
        leading: Vec<Lead>,
        b_start: Span,
        e_lead: Vec<Lead>,
    },
    /// Reading the body; `binds` is the assembled `(… bindings)` list, `body_lead` its leading comments.
    Body {
        binds: StructId,
        body_lead: Vec<Lead>,
    },
}

/// The sub-expr a `match scrut with | pat [if g] => body | …` form is reading when it SUSPENDS on the
/// worklist (the twin of `match_expr`/`match_arm`'s `expr` calls — scrutinee, each arm's optional guard,
/// each arm's body). The pattern is read INLINE via `match_arm_pat` (recursive `pattern()`, I4's separate
/// guard). Arms are sibling iteration (a `Vec` in the cont's `items`), not recursion. Byte-identical to
/// `match_expr` node order + comment handling (`arm_bar_terminates` forced true only around each body).
enum MatchPhase {
    /// Reading the scrutinee; on deliver: push it, consume `with`, drain the first arm's leading comments
    /// + optional leading `|`, then start the first arm.
    Scrut,
    /// Reading an arm's guard expr; on deliver: fold `(guard pat g)`, then read the body preamble + descend
    /// the body. `arm_leading` is the arm's own-line leading comment run.
    ArmGuard {
        arm_start: Span,
        arm_leading: Vec<Lead>,
        pat: StructId,
        guard_head: StructId,
        g_start: Span,
    },
    /// Reading an arm's body expr (`arm_bar_terminates` forced true; `saved_arm_bar` restores it on
    /// deliver). On deliver: assemble `(pat body)`, wrap comments, append; then either start the next arm
    /// (on `|`) or finish the match.
    ArmBody {
        arm_start: Span,
        arm_leading: Vec<Lead>,
        pat: StructId,
        body_lead: Vec<Lead>,
        saved_arm_bar: bool,
    },
}

/// The sub-expr a `handle E[(seed)] with | op(…, s) => body | … in body` form is reading when it SUSPENDS
/// on the worklist (the twin of `handle_expr`/`handle_arm`'s `expr` calls — the optional seed, each arm's
/// body, the final `in` body). The effect name + each arm's `op(binder…, state)` header are read INLINE
/// (binders only, no expr descent); arms are sibling iteration (a `Vec` in the phase). Byte-identical to
/// `handle_expr` node order + the `arm_bar_terminates`-true-around-each-body discipline. `head`/`effect`
/// live in the `Cont::Handle` (known at dispatch); `seed`/`arms`/`arms_start` flow through the phases.
enum HandlePhase {
    /// Reading the seed expr of `E(seed)`; on deliver: consume `)`, then `with` + first arm.
    Seed,
    /// Reading an arm's body (`arm_bar_terminates` forced true; `saved_arm_bar` restores it). On deliver:
    /// assemble `(op params state body)`, append; then start the next arm (on `|`) or descend the `in` body.
    ArmBody {
        seed: StructId,
        arms_start: Span,
        arms: Vec<StructId>,
        arm_start: Span,
        op: StructId,
        params: StructId,
        state: StructId,
        saved_arm_bar: bool,
    },
    /// Reading the final `in` body; `arms_list` is the assembled `(arm…)`. On deliver: assemble
    /// `(handle effect seed (arm…) body)`.
    Body { seed: StructId, arms_list: StructId },
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
    /// I3 (v-syntax-nonrec-reader): when true, [`Self::expr`] dispatches to the iterative
    /// [`Self::expr_iter`] (the explicit-stack shunting-yard) instead of recursing. [`parse`] (=
    /// `read_ml_recursive`, the frozen oracle reference) leaves it `false`; [`parse_iterative`] (=
    /// `read_ml`) sets it `true`. Every OTHER method is shared, so the differential oracle diffs the two
    /// expr strategies over the whole corpus + generated sweep. Removed once the rewrite is complete.
    iterative: bool,
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
    fn name(&mut self, name: impl Into<std::sync::Arc<str>>, span: Span) -> StructId {
        self.atom(Leaf::Name(name.into()), span)
    }

    /// Parse a tagged-template HOLE's source text as an ordinary ML expression and GRAFT the result
    /// into this builder, returning the grafted root id. A hole `{expr}` holds any expression, so it is
    /// parsed by the full ML reader (`read_ml`) in its own arena, then its root subtree is copied here
    /// (leaves + structure) so it lives in the one arena this parse produces. Every grafted node is
    /// given the template token's `span` (a hole has no independent source span in this token — the
    /// whole `tag"…"` is one lexed token; a finer per-hole span is a future refinement). A hole that
    /// fails to parse contributes its recovered/error arena (the reader never panics), and any parse
    /// errors are surfaced by lifting them into this parser's error list.
    //
    // The grafted subtree IS the hole's parsed expression, placed directly under the node's `holes`
    // list (see the `Kind::TaggedTemplate` arm) — so each hole appears in the node as an ordinary
    // expression of the language.
    //
    //= spec/capabilities/metaprogramming.md#a-tagged-template-is-a-binding-dispatched-compile-time-macro-over-literal-chunks-and-holes
    //# Each interpolation hole `{expr}` MUST be parsed as an ordinary expression of the language.
    fn graft_ml_expr(&mut self, src: &str, span: Span) -> StructId {
        let parsed = read_ml(src);
        for e in &parsed.errors {
            self.errors.push(ParseError {
                span,
                message: format!("in a tagged-template hole: {}", e.message),
            });
        }
        self.graft_subtree(&parsed.arenas, parsed.arenas.root, span)
    }

    /// Recursively copy the subtree rooted at `id` in `src` into this builder, giving each copied node
    /// `span`. Returns the new root id. (An arena is append-only, so a post-order copy is valid.)
    fn graft_subtree(&mut self, src: &Arenas, id: StructId, span: Span) -> StructId {
        match src.get(id) {
            crate::ast::Struct::Atom(leaf) => self.atom(src.leaf(*leaf).clone(), span),
            crate::ast::Struct::List(children) => {
                let children = children.clone();
                let kids: Vec<StructId> = children
                    .iter()
                    .map(|&c| self.graft_subtree(src, c, span))
                    .collect();
                self.list(kids, span)
            }
        }
    }

    /// Like [`graft_subtree`], but each copied node keeps its OWN source span from `src`'s span table,
    /// SHIFTED by `offset` into this document's coordinate system. This is what makes an embedded region
    /// LSP-transparent: a cursor inside a `json{ … }` body resolves to the exact JSON node, because the
    /// grafted node's span is the sub-grammar's real span (relative to the body) plus the body's start
    /// offset in the outer source. A node the sub-grammar left un-spanned (should not happen — the
    /// surfaces keep a total table) falls back to `whole` (the region span), so the graft stays total.
    fn graft_subtree_spanned(
        &mut self,
        src: &Arenas,
        id: StructId,
        spans: &SpanTable,
        offset: usize,
        whole: Span,
    ) -> StructId {
        let span = spans
            .get(id)
            .map(|s| Span::new(s.start + offset, s.end + offset))
            .unwrap_or(whole);
        match src.get(id) {
            crate::ast::Struct::Atom(leaf) => self.atom(src.leaf(*leaf).clone(), span),
            crate::ast::Struct::List(children) => {
                let children = children.clone();
                let kids: Vec<StructId> = children
                    .iter()
                    .map(|&c| self.graft_subtree_spanned(src, c, spans, offset, whole))
                    .collect();
                self.list(kids, span)
            }
        }
    }

    /// Parse a FIRST-CLASS EMBEDDED-SYNTAX region — `<grammar>{ …raw… }` — into an
    /// `(embedded <grammar> <subtree>)` node. The cursor is on the grammar-tag `Ident`; the next token
    /// is the opening `{` (the `prefix` guard checked the tag is reserved + glued to the `{`).
    ///
    /// The body is handed VERBATIM to the sub-grammar's own reader: we do NOT trust the ML tokenization
    /// of the body (JSON/TOML have their own lexis), so we find the balanced closing `}` by scanning the
    /// RAW SOURCE from just after the `{`, tracking brace depth while skipping over string literals (a
    /// `}` inside `"…"` does not close the region). The raw slice between the braces is parsed by the
    /// grammar's `read`; on success the returned arena's root subtree is grafted here under
    /// `(embedded <#grammar> …)`; on a sub-grammar error the diagnostic is lifted into this parse and an
    /// `<error>` placeholder keeps the arena well-formed (the never-panic contract). Finally the token
    /// cursor is advanced past every ML token whose span lies within the consumed region.
    ///
    /// (Spans: the whole grafted subtree currently shares the region span — same best-effort granularity
    /// as a tagged-template hole; per-node span remapping from the sub-grammar's own span table is a
    /// later refinement, tracked for the LSP-transparency goal.)
    fn embedded_syntax(&mut self) -> StructId {
        let tag_tok = self.bump().expect("on the grammar-tag ident");
        let grammar = embedded_grammar(self.text(tag_tok)).expect("guard checked a reserved tag");
        let brace_tok = self.bump().expect("guard checked the `{`");
        let open = brace_tok.span; // the `{`
        let body_start = open.end; // first byte after `{`

        // Scan raw source for the matching `}` (depth 1 at body_start), skipping string literals.
        let bytes = self.src.as_bytes();
        let mut i = body_start;
        let mut depth = 1usize;
        let mut in_str = false;
        let mut escaped = false;
        let mut close: Option<usize> = None;
        while i < bytes.len() {
            let c = bytes[i];
            if in_str {
                if escaped {
                    escaped = false;
                } else if c == b'\\' {
                    escaped = true;
                } else if c == b'"' {
                    in_str = false;
                }
            } else {
                match c {
                    b'"' => in_str = true,
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            close = Some(i);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            i += 1;
        }

        let region_span = match close {
            Some(end) => Span::new(open.start, end + 1),
            None => {
                // Unbalanced — no closing `}`. Record an error, consume to end, emit a placeholder.
                self.errors.push(ParseError {
                    span: open,
                    message: format!("unterminated `{grammar}{{ … }}` embedded-syntax region"),
                });
                self.pos = self.tokens.len();
                let head = self.name("embedded", open);
                let g = self.atom(Leaf::Sym(grammar.into()), open);
                let err = self.error_node(open);
                return self.list(vec![head, g, err], open);
            }
        };
        let body = &self.src[body_start..close.expect("Some in this arm")];

        // Advance the ML token cursor past every token inside the consumed region (up to and including
        // the closing `}`), so the parser resumes AFTER the region regardless of how ML tokenized it.
        let region_end = region_span.end;
        while self.pos < self.tokens.len() && self.tokens[self.pos].span.start < region_end {
            self.pos += 1;
        }

        // Parse the raw body with the sub-grammar's own reader and graft the result. `body_start` is the
        // body's offset in the outer source, so each grafted node's span lands in document coordinates.
        let head = self.name("embedded", region_span);
        let g = self.atom(Leaf::Sym(grammar.into()), region_span);
        let sub = self.read_embedded(grammar, body, body_start, region_span);
        self.list(vec![head, g, sub], region_span)
    }

    /// Dispatch a raw embedded-syntax body to the named sub-grammar's SPANNED reader and graft the result
    /// into this arena, returning the grafted subtree root. Each grafted node keeps its own source span
    /// (the sub-grammar's span shifted by `body_start` into document coordinates) so the region is
    /// LSP-transparent — a cursor inside resolves to the exact embedded node, not the whole region. A
    /// reader error is lifted into this parse (anchored to the region span) and an `<error>` placeholder
    /// is returned — never a panic (the surface contract).
    fn read_embedded(
        &mut self,
        grammar: &str,
        body: &str,
        body_start: usize,
        span: Span,
    ) -> StructId {
        let parsed = match grammar {
            "json" => crate::json::read_spanned(body).map_err(|e| e.0),
            "toml" => crate::toml_surface::read_spanned(body).map_err(|e| e.0),
            _ => Err(format!("unknown embedded grammar `{grammar}`")),
        };
        match parsed {
            Ok((arena, spans)) => {
                self.graft_subtree_spanned(&arena, arena.root, &spans, body_start, span)
            }
            Err(message) => {
                self.errors.push(ParseError {
                    span,
                    message: format!("in a `{grammar}{{ … }}` region: {message}"),
                });
                self.error_node(span)
            }
        }
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
        // M2: the compound constructor is a native ctor LEAF KIND (recognized by kind identity, not head
        // text), the ML-reader counterpart of the s-expr `#word(` flip. Unshadowable like the old string
        // primitive; `read_ml` is native end-to-end (ruling ii).
        let ctor = match name {
            "record" => CompoundCtor::Record,
            "tuple" => CompoundCtor::Tuple,
            "list" => CompoundCtor::List,
            "map" => CompoundCtor::Map,
            "set" => CompoundCtor::Set,
            _ => unreachable!("ctor_head is only called with the five compound ctor words"),
        };
        self.atom(Leaf::Ctor(ctor), span)
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

    /// If the cursor is at a `..` rest/spread marker, consume it + the following binder (parsed by
    /// `elem`) and push a single WRAPPED node `(.. <binder>)` onto `items`, returning `true`; otherwise
    /// consume nothing and return `false`. The rest/spread is a self-contained `(.. operand)` form — a
    /// `List` whose head is the `..` `Name` — in BOTH construction (`[1, 2, ..rest]` → `(list 1 2 (.. rest))`)
    /// and pattern (`[x, ..rest]` → `(list x (.. rest))`) position (operator 2026-08-29: `(.. v)` everywhere,
    /// "a lot more consistent... otherwise we're putting an infix operator in sexpr"). Supersedes the legacy
    /// FLAT `Name("..")` + next-sibling marker; the compiler + every surface recognize both via
    /// [`Arenas::rest_marker`] (Phase 1, #5890), and the s-expr reader normalizes the legacy flat form to
    /// this wrapped shape. Well-formedness (one binder, position) is left to the compiler, as before.
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
        let head = self.name("..", dd_span);
        let operand = elem(self);
        let span = dd_span.merge(self.prev_span());
        items.push(self.list(vec![head, operand], span));
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

    /// If the current grammar token's leading run begins with TRAILING comment lead(s) — comments that
    /// sat on the SAME source line as the PREVIOUS token (`Ctor(T)  // note`) — drain and return them,
    /// leaving any leading (own-line) comments/docs in place. A drain site calls this RIGHT AFTER parsing
    /// the element the comment trails, then wraps that element in `(comment-after "text" node)` so the
    /// comment attaches to what it FOLLOWS and re-prints same-line — instead of being stranded at this
    /// token's leading slot (where an interior parser drops it). Only the LEADING PREFIX of trailing
    /// leads is taken (a trailing comment is contiguous with the prior line's code; any own-line comment
    /// after it is a genuine leading comment of the next element and stays). Returns `[]` if none.
    fn take_trailing_comment_here(&mut self) -> Vec<Lead> {
        let leads = if self.pos < self.leading.len() {
            &mut self.leading[self.pos]
        } else {
            return Vec::new();
        };
        // Count the leading prefix that is trailing (`//` on the previous line). Stop at the first
        // non-trailing lead (an own-line comment/doc that belongs to the NEXT element).
        let n = leads.iter().take_while(|l| l.trailing).count();
        if n == 0 {
            return Vec::new();
        }
        leads.drain(..n).collect()
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

    /// Move the `///` DOC leads at leading slot `from` to the FRONT of slot `to` (leaving `//`
    /// comments at `from` untouched). Used by the `@` annotation arm to carry a doc that preceded
    /// the annotation down to the annotated FORM's slot, so a documentable inner form
    /// (`def`/`type`/`effect`/`module`) drains it as a `(doc …)` — matching a doc written directly
    /// before an UNannotated def. Without this the docs sit at the `@` slot, never seen by the inner
    /// def parser (which runs after `@name`), and `stmt` downgrades them to a `(comment …)` (`//`).
    /// A no-op when `from`/`to` are out of range or equal. Composes through stacked `@a @b def …`:
    /// each arm carries the docs one slot inward until the def drains them.
    fn carry_docs(&mut self, from: usize, to: usize) {
        if from == to || from >= self.leading.len() || to >= self.leading.len() {
            return;
        }
        let (docs, comments): (Vec<Lead>, Vec<Lead>) = std::mem::take(&mut self.leading[from])
            .into_iter()
            .partition(|l| l.doc);
        self.leading[from] = comments;
        if docs.is_empty() {
            return;
        }
        let mut merged = docs;
        merged.append(&mut self.leading[to]);
        self.leading[to] = merged;
    }

    /// Parse an expression BODY (a def value/function body) that may be preceded by leading `//`
    /// comment or `///` doc lines on their own line(s) — the interior body-leading-trivia position
    /// (`def f() =` newline `// note` newline `body`). `expr` itself does NOT drain trivia (only
    /// `stmt` does, at statement positions), so a comment leading a body expression would otherwise
    /// be STRANDED at the body's first-token slot and DROPPED entirely (not even a `(comment …)`
    /// node — a genuine comment LOSS, worse than a downgrade). Capture the trivia at the body's first
    /// token, parse the body, and wrap it in `(comment "text" body)` nodes (outermost = first) so it
    /// round-trips like a statement comment. A leftover `///` here downgrades to `//` (there is no
    /// body-doc concept) — still strictly better than dropping it. Only the body's FIRST-token slot
    /// is drained (a mid-body comment is a separate, harder position, out of scope here).
    fn body_expr(&mut self, min_prec: u8) -> StructId {
        let leading: Vec<Lead> = if self.pos < self.leading.len() {
            std::mem::take(&mut self.leading[self.pos])
        } else {
            Vec::new()
        };
        let body = self.expr(min_prec);
        self.wrap_comments(leading, body)
    }

    /// Parse a statement (a top-level form / module member): capture any leading `//` comments and
    /// wrap the parsed form in `(comment "text" node)`, outermost = first. Leading `///` docs are
    /// left in place for a def/module parser to splice inside; any docs a non-def form leaves behind
    /// become `(module-doc "text")` SIBLING nodes before the form (preserving the `///` marker) rather
    /// than being downgraded to `(comment …)` (`//`). The module-doc siblings are returned spliced into
    /// a bare `(do …)` — which `push_root_form` flattens into top-level siblings (the file-header case:
    /// `/// header` before the first `import`), and which a `module { … }` body accepts as a member
    /// sequence — so a leading `///` on a NON-documentable form is preserved AS documentation.
    fn stmt(&mut self) -> StructId {
        let start = self.pos;
        let comments = self.take_comments_here();
        let node = self.expr(0);
        self.finish_stmt(node, start, comments)
    }

    /// The post-expr half of [`Self::stmt`]: given the parsed form `node`, the statement's START token
    /// index, and the leading `//` `comments` captured before it, attach any leftover trivia and return
    /// the statement node. Shared by the recursive `stmt` and the iterative `Cont::Module` member loop so
    /// the two never drift. `start` is a TOKEN INDEX (into `self.leading`), not a span.
    fn finish_stmt(&mut self, node: StructId, start: usize, comments: Vec<Lead>) -> StructId {
        // Docs still sitting at the statement's start slot were NOT consumed (the form was not a
        // def/type/effect/module, which drain their own docs) — so they document the FILE/MODULE, not a
        // definition. Emit them as `(module-doc …)` siblings so they re-print as `///` (a
        // `wrap_comments` here would downgrade them to `//` — the file-header doc-loss bug).
        let leftover: Vec<Lead> = if start < self.leading.len() {
            std::mem::take(&mut self.leading[start])
        } else {
            Vec::new()
        };
        let (docs, comments_left): (Vec<Lead>, Vec<Lead>) =
            leftover.into_iter().partition(|l| l.doc);
        // Any stray `//` still here (shouldn't normally happen — `take_comments_here` took them — but be
        // total) stays a comment wrapper, as before.
        let node = self.wrap_comments(comments_left, node);
        let node = self.wrap_comments(comments, node);
        if docs.is_empty() {
            return node;
        }
        // Prepend `(module-doc "text")` siblings, then the form, spliced into a `(do …)`.
        let mut items = Vec::with_capacity(docs.len() + 2);
        let do_span = docs[0].span;
        items.push(self.name("do", do_span));
        for lead in docs {
            let head = self.name("module-doc", lead.span);
            let text = self.atom(Leaf::Str(lead.text.into()), lead.span);
            items.push(self.list(vec![head, text], lead.span));
        }
        items.push(node);
        self.list(items, do_span)
    }

    /// Fold a run of comment leads around `node`: `[c0, c1]` -> `(comment c0 (comment c1 node))`.
    ///
    /// The parser REPRESENTS each `//` comment as a `(comment "text" node)` NODE of the canonical
    /// representation — not discarded as lexical trivia — wrapping the following form so the comment is
    /// ATTACHED to the part it annotates (its position recovered on printing). Because the node is an
    /// ordinary list node, it survives the binary-AST codec: printing the binary AST back to text and
    /// re-parsing yields the same `(comment …)` (and `(doc …)`) nodes — comments and documentation both
    /// round-trip. (An intra-program EDIT preserving them is the sidecar `Rewrite` surface — now built;
    /// see `query::driver::apply_rewrite` + its `a_rewrite_preserves_untouched_comment_and_doc_nodes` test.)
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
            let text = self.atom(Leaf::Str(lead.text.into()), lead.span);
            node = self.list(vec![head, text, node], lead.span);
        }
        node
    }

    /// Wrap `node` in `(comment-after "text" node)` for each TRAILING comment lead (a `//` that followed
    /// `node` on the same source line), innermost = first so multiple trail in source order. Distinct
    /// head from the leading `(comment …)` so the printer knows to re-emit it SAME-LINE (`node // text`),
    /// and so `strip_comments` (which peels both heads) keeps it transparent to the compiler. A no-op for
    /// an empty run (returns `node` unchanged).
    fn wrap_comment_after(&mut self, comments: Vec<Lead>, mut node: StructId) -> StructId {
        for lead in comments.into_iter() {
            let head = self.name("comment-after", lead.span);
            let text = self.atom(Leaf::Str(lead.text.into()), lead.span);
            // Same `(head "text" node)` shape as a leading `(comment …)` — so `strip_comments` peels
            // BOTH heads by the identical `tail = [text, form]` rule (the compiler never sees either).
            node = self.list(vec![head, text, node], lead.span);
        }
        node
    }

    /// Drain any OWN-LINE `//` comment(s) sitting in the CLOSER token's leading slot (a trailing comment
    /// on its own line before `]`/`}`/`)` after the last element, e.g. `[1, 2\n // note\n]`) and attach
    /// them to the LAST element of `items` as leading `(comment …)` wrappers. Without this the comment is
    /// stranded in the closer's slot and dropped (the comment-drop guard then refuses to format the file).
    /// Mirrors the module-body trailing-comment fix (attach-to-last): the comment's PRINTED position moves
    /// ABOVE the last element rather than staying just before the closer — the same accepted v1 limitation
    /// the module body has — but the comment is PRESERVED (round-trip no longer drops it). `head_len` is
    /// the count of non-element leading items (1 for a name-headed `("list" …)`, 0 for a bare list). DOC
    /// (`///`) leads are left in place by `take_comments_here` (the `///`-in-collection case is separately
    /// operator-gated), so this only moves ordinary `//` comments.
    ///
    /// GATED to skip when the last element is ALREADY comment-wrapped (`(comment …)`/`(comment-after …)`):
    /// prepending the closer comment there would REORDER it above the element's own earlier comment
    /// (`[1, // mid\n 2\n // last\n]` → `last` printed above `mid`), an unfaithful round-trip. In that
    /// collision the closer comment is left stranded (the drop-guard refuses — no corruption), exactly the
    /// pre-fix behavior; only the clean single-comment case is captured. A no-op too when the closer has no
    /// leading comment or `items` has no element (only the head, or empty).
    fn drain_closer_comment_onto_last(&mut self, items: &mut [StructId], head_len: usize) {
        if items.len() <= head_len {
            return; // no element to attach to (empty collection)
        }
        let last_ix = items.len() - 1;
        // If the last element already carries a comment wrapper, attaching here would reorder — leave the
        // closer comment for the drop-guard (no corruption) rather than emit an out-of-order round-trip.
        if self.node_is_comment_wrapped(items[last_ix]) {
            return;
        }
        let trailing = self.take_comments_here();
        if trailing.is_empty() {
            return;
        }
        items[last_ix] = self.wrap_comments(trailing, items[last_ix]);
    }

    /// Is `node` already wrapped in a leading `(comment …)` or trailing `(comment-after …)`? Used to gate
    /// [`Self::drain_closer_comment_onto_last`] off a reordering collision.
    fn node_is_comment_wrapped(&self, node: StructId) -> bool {
        matches!(self.builder.get(node), crate::ast::Struct::List(kids)
            if kids.first().is_some_and(|h|
                matches!(self.builder.as_name(*h), Some("comment") | Some("comment-after"))))
    }

    /// Build `(doc "text")` body-form nodes from a run of doc leads.
    fn doc_nodes(&mut self, docs: Vec<Lead>) -> Vec<StructId> {
        docs.into_iter()
            .map(|lead| {
                let head = self.name("doc", lead.span);
                let text = self.atom(Leaf::Str(lead.text.into()), lead.span);
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
        // Comments after the last grammar token (e.g. a trailing `// note` on the final line) have no
        // FOLLOWING form to precede. Attach them to the LAST form instead — the same `(comment "text"
        // node)` wrapper a mid/leading comment gets, just around the final form rather than the next one.
        // This preserves the TOP-LEVEL FORM SET: a trailing comment must NOT wrap the whole root, because
        // when the root is a multi-form `(do …)` that buries every top-level def inside the comment's
        // child, so a top-level walk (`cdz metadata`/`exports`/manifest parse) sees ZERO defs though a
        // leading comment parses fine (bug from v-cdz-tooling: a `Project.cdz` ending in `//` read as
        // name:null deps:[]). Wrapping the last form keeps each def a direct root child. (v1 scope: the
        // comment's PRINTED position moves ABOVE the last form — the same known limitation a mid-comment
        // already has — but the def set is now correct, which is what walkers depend on.) When there are
        // no forms at all (a comment-only program) `program` has already errored above, so `forms` is
        // non-empty here whenever `trailing` is.
        let trailing = std::mem::take(&mut self.trailing);
        if !trailing.is_empty() && !forms.is_empty() {
            let last = forms.pop().unwrap();
            let wrapped = self.wrap_comments(trailing, last);
            forms.push(wrapped);
        }
        if forms.len() == 1 {
            forms.pop().unwrap()
        } else {
            let do_head = self.name("do", start);
            let mut items = Vec::with_capacity(forms.len() + 1);
            items.push(do_head);
            items.extend(forms);
            let span = start.merge(self.prev_span());
            self.list(items, span)
        }
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

    /// Shared DEPTH GUARD for a recursion that re-enters `prefix` WITHOUT going through `expr` (the
    /// unary-minus operand, the bare-form unquote operand) — those paths bypass `expr`'s own guard, so a
    /// pathologically deep `-----…1` / `,,,,,x` would overflow the native stack (SIGABRT). If the depth
    /// limit is hit, poison the parse (one error, `depth_exceeded` so all loops stop) and return an
    /// `<error>` node for the caller to graft; otherwise bump `self.depth` and return `None`. The caller
    /// MUST `self.depth -= 1` after its recursion (mirroring `expr`'s decrement) to keep the budget
    /// balanced. Shares `MAX_NESTING_DEPTH` with `expr` and the s-expr reader.
    fn guard_prefix(&mut self, start: Span) -> Option<StructId> {
        if self.depth >= crate::sexpr::MAX_NESTING_DEPTH {
            if !self.depth_exceeded {
                self.error("expression nests too deeply to parse");
                self.depth_exceeded = true;
            }
            return Some(self.error_node(start));
        }
        self.depth += 1;
        None
    }

    /// Parse an expression whose infix operators bind at least `min_prec`.
    fn expr(&mut self, min_prec: u8) -> StructId {
        // I3: route to the iterative shunting-yard when `parse_iterative` set the flag (`read_ml`); the
        // recursive body below stays for `parse` = `read_ml_recursive` (the frozen oracle reference).
        if self.iterative {
            return self.expr_iter(min_prec);
        }
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
        // A numeric literal handles its OWN unit suffix in the prefix arm (preserving the `Suffixed`
        // exemption: `100N feet` is NOT a quantity). Every OTHER prefix gets the unit suffix generally,
        // after `postfix`, so a variable / call / parenthesized expression takes a unit too.
        let prefix_is_number = matches!(self.kind(), Kind::Int | Kind::Float);
        let mut left = self.prefix();
        left = self.postfix(left, start);
        // UNIT SUFFIX (general postfix): an adjacent same-line unit name applies to ANY non-literal
        // expression — `x meters`, `f(5) meters`, `(a + b) meters` all read as a quantity. This binds
        // TIGHTER than every infix operator (applied here, before the infix loop below), so `x + 1 meters`
        // = `x + (1 meters)` and `(x + 1) meters` needs the parens (operator-confirmed). Generalizes the
        // former literal-only sugar. Fixes a real miscompile: `x meters` previously SILENTLY parsed as a
        // two-statement sequence `(do x meters)`. (Typing — the operand must be a DIMENSIONLESS number; a
        // Quantity operand is a type error — is enforced by v-quantity/v-inference, not the parser: the
        // parse is the uniform `(Qty.of <expr> (Unit.of #name))` regardless of the operand's type.) A
        // number is EXCLUDED here — it already took (or, if suffixed, declined) its unit in the prefix arm,
        // so re-applying would double-wrap a `10 meters` or wrongly unit-suffix a `100N`.
        if !prefix_is_number {
            left = self.maybe_unit_suffix(left, start);
        }
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
            // A SAME-LINE trailing `//` comment on the LEFT operand sits at this operator token's leading
            // slot (`a  // note` newline `and b`), tagged trailing. Attach it to `left` as
            // `(comment-after …)` so it re-prints — otherwise it is stranded at the operator's slot and
            // NEVER drained (the mid-infix-chain comment-loss, seq-277/C3). Only the same-line PREFIX is
            // taken; an own-line comment before the operator stays (it leads the right operand). The infix
            // PRINTER must re-emit a comment-after-wrapped operand with a break before the operator (else
            // the `// note` would swallow the trailing ` op right`). `strip_comments` peels it.
            let left_trailing = self.take_trailing_comment_here();
            left = self.wrap_comment_after(left_trailing, left);
            // OWN-LINE `//` comment(s) sitting BEFORE this operator (`a\n  // note\n  and b`, or a block
            // between operands of a multi-line `and`/`|>` chain) lead the RIGHT operand — they remain at
            // the operator token's leading slot after the same-line trailing prefix is taken. Drain them
            // here + attach to `right` below, else they are stranded at the op slot and DROPPED (seq-277/C3:
            // sread-eval.cdz's mid-chain own-line comment blocks). The infix printer emits them own-line
            // BEFORE the operator. `strip_comments` peels them.
            let right_leading = self.take_comments_here();
            self.bump(); // operator
            let head = self.name(op_name, op_span);
            // A `:` ascription whose RHS OPENS with `forall` is a type-position `forall` (`e : forall a. T`):
            // parse it via `forall_type`, the same path the structural `:` sites (param/return/field/effect-op)
            // reach through `type_ref`. `forall` is a CONTEXTUAL keyword recognized only in type position, so
            // the general `expr` RHS below would misread it as an ordinary name and let the unit-suffix postfix
            // eat the following binder (`forall a` → `(Qty.of forall (Unit.of #"a"))`) — the printer emits
            // `e : forall a. T` (the type surface), so without this the round-trip breaks. Only `forall` needs
            // the intercept: every OTHER type form (`Int64`, `List(a)`, `a -> b`, `M.T`, `Tuple(a, b)`) already
            // round-trips through the general `expr` RHS, so the ascription's value/expression RHS is otherwise
            // unchanged (`x : a + b` stays `(: x (+ a b))`).
            let right = if op_name == ":" && self.at_keyword(Keyword::Forall) {
                self.forall_type(self.cur_span())
            } else {
                // Left-assoc: the right operand binds one tighter (`prec + 1`), so a same-precedence run
                // groups left. The type arrow `->` is right-associative — it recurses at `prec`, so
                // `A -> B -> C` groups as `A -> (B -> C)` (the curried reading).
                let right_min = if is_right_assoc(op_name) {
                    prec
                } else {
                    prec + 1
                };
                self.expr(right_min)
            };
            // Attach the own-line comment(s) that preceded the operator as leading on the right operand.
            let right = self.wrap_comments(right_leading, right);
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

    /// Iterative precedence-climbing — the explicit-stack replacement for the recursive [`Self::expr`]'s
    /// `self.expr(right_min)` right-operand recursion (v-syntax-nonrec-reader I3). Byte-identical to
    /// `expr` (verified by the differential oracle in `roundtrip_tests::generative_roundtrip`): it mirrors
    /// every arm — the `as`-conversion, the `:`+`forall` intercept, same-line + own-line comment attach,
    /// the unit suffix, the spine + depth guards, and the `min_prec == PREC_SEQ` `finish_sequence`.
    ///
    /// HYBRID STAGE: operands are still read via the (recursive) [`Self::prefix`]/[`Self::postfix`], so
    /// this de-recurses the infix / right-operand / right-assoc-arrow chains; bracket/keyword operand
    /// NESTING still recurses through `prefix` until those forms are pulled onto the worklist in a later
    /// increment. It reuses `expr`'s exact helpers, so the output arena + span table + errors match.
    fn expr_iter(&mut self, min_prec: u8) -> StructId {
        // A pending continuation on the explicit stack — the worklist that replaces native recursion.
        enum Cont {
            // A SUSPENDED expr level awaiting its infix RIGHT operand (replaces `self.expr(right_min)`);
            // on deliver, combine `(head left right)` and resume THIS level's infix loop.
            Op {
                left: StructId,
                head: StructId,
                right_leading: Vec<Lead>,
                start: Span,
                min_prec: u8,
                spine: u32,
            },
            // A quasiquote `` `{ <expr> } `` awaiting its braced inner expr (an OPERAND-position form —
            // the level that opened it was reading its operand). On deliver: consume `}`, wrap
            // `(quasiquote inner)`, then that node IS the parent level's operand (so postfix + unit-suffix
            // apply), and the parent's infix loop resumes. Carries the parent LEVEL state to restore.
            QuasiQuote {
                head: StructId,
                start: Span,
                min_prec: u8,
                spine: u32,
                entered: bool,
                prefix_is_number: bool,
            },
            // A `( … )` paren awaiting a sub-expr (an OPERAND-position form). It is a multi-phase
            // collector: the FIRST sub-expr (`self.expr(0)`) decides grouping `(e)` vs tuple `(a, b, …)`;
            // in tuple mode it then collects `,`-separated elements (`self.expr(PREC_SEQ+1)`). On the final
            // close it assembles the operand (transparent grouping = the first expr; tuple = `("tuple" …)`),
            // which becomes the parent level's operand (postfix + unit-suffix apply). `arm_bar` restores
            // `arm_bar_terminates` (mirrors `bracketed_bars`). `pending_leading` is the own-line comment run
            // captured before the sub-expr currently being read (`first_leading`, or a tuple element's).
            Paren {
                start: Span,
                min_prec: u8,
                spine: u32,
                entered: bool,
                prefix_is_number: bool,
                arm_bar: bool,
                pending_leading: Vec<Lead>,
                // `items` is empty while the FIRST sub-expr is still being read (grouping-vs-tuple undecided);
                // once tuple mode is entered it holds `[ "tuple"-head, first, … ]`. `items.is_empty()` IS the
                // mode flag on deliver — no separate bool needed. (A LEADING `.. a` spread pre-populates
                // `items` with the tuple head, so it enters tuple mode immediately — never `is_empty`.)
                items: Vec<StructId>,
                // The delivered sub-expr is a `.. a` CONSTRUCTION SPREAD operand — wrap it `(.. operand)`
                // (head = `rest_head`, pre-created before the descent, matching `rest_marker`'s order)
                // before pushing it as a tuple element. `false` for an ordinary element/grouping.
                spread: bool,
                rest_head: StructId,
            },
            // A `[ … ]` list literal awaiting an element sub-expr. `items` = `[ "list"-head, … ]`. Each
            // element is either a `.. rest` spread (`is_rest`: wrap `(.. binder)` with `dd_span`) or an
            // ordinary element (`pending_leading` own-line comments; a same-line trailing comment on the
            // LAST element). `before` is the pos at the element's loop-iteration start (the missing-`,`
            // progress guard). `arm_bar` restores `arm_bar_terminates`. On the close it assembles
            // `("list" …)` as the parent level's operand.
            List {
                start: Span,
                min_prec: u8,
                spine: u32,
                entered: bool,
                prefix_is_number: bool,
                arm_bar: bool,
                // The closer that ends this comma-list. `]` for `[…]` list / `b[…]` bin / `#[…]` raw-list;
                // `)` for a `#(…)` set. These families are the SAME skeleton (optional head in `items[0]`
                // + `,`-separated elements read at `PREC_SEQ+1`), differing only in delimiters, the head,
                // and three per-family behaviors captured by the flags below:
                //   - `allow_rest`   — a `.. binder` spread element is legal (list/set; NOT bin/hash).
                //   - `allow_comments` — own-line leading + last-element trailing comment slots are
                //                        captured+wrapped (list/set/bin; NOT hash, whose elements are bare).
                //   - `drain_closer` — an own-line `//` just before the closer attaches to the last element
                //                      (list/set; NOT bin/hash).
                closer: Kind,
                allow_rest: bool,
                allow_comments: bool,
                drain_closer: bool,
                items: Vec<StructId>,
                pending_leading: Vec<Lead>,
                is_rest: bool,
                dd_span: Span,
                // The `..` head node, created BEFORE the binder descends (matching `rest_marker`'s
                // head-then-operand order, so the span table's struct-id order is byte-identical).
                rest_head: StructId,
                before: usize,
            },
            // A `{ … }` record or `#{ … }` map awaiting a field sub-expr (an OPERAND-position form). It is
            // the field-pair analogue of `List`: `items` = `[ ctor-head, … ]`; `phase` says what the pending
            // descent delivers (rest operand / map key / map|record value) and carries the partial field
            // state to reassemble it (see `FieldPhase` + `advance_fields`). `is_map` selects record vs map
            // reassembly; `before` is the current field's start pos (the missing-`,` progress guard). On the
            // close it assembles `("record"|"map" …)` as the parent level's operand (postfix + unit-suffix).
            Fields {
                start: Span,
                min_prec: u8,
                spine: u32,
                entered: bool,
                prefix_is_number: bool,
                arm_bar: bool,
                is_map: bool,
                items: Vec<StructId>,
                phase: FieldPhase,
                before: usize,
            },
            // An `if c then t else e` keyword form awaiting one of its three branch sub-exprs (an
            // OPERAND-position form; NOT bracketed_bars — `arm_bar_terminates` is left untouched, unlike
            // the collections). `head` is the pre-created `if` keyword head; `phase` says which branch is
            // being read + carries the already-built branches. On the final (else) deliver it assembles
            // `(if c t e)` as the parent level's operand (postfix + unit-suffix apply).
            If {
                start: Span,
                min_prec: u8,
                spine: u32,
                entered: bool,
                prefix_is_number: bool,
                head: StructId,
                phase: IfPhase,
            },
            // A `let b = v, … in body` keyword form awaiting a binding value or the body (an OPERAND-position
            // form; NOT bracketed_bars — arm_bar_terminates untouched). `head` is the pre-created `let` head;
            // `phase` carries the built bindings + which sub-expr is pending. On the body deliver it assembles
            // `(let (binds) body)` as the parent level's operand (postfix + unit-suffix apply).
            Let {
                start: Span,
                min_prec: u8,
                spine: u32,
                entered: bool,
                prefix_is_number: bool,
                head: StructId,
                phase: LetPhase,
            },
            // A `match scrut with | … | …` keyword form awaiting the scrutinee, an arm guard, or an arm
            // body (an OPERAND-position form; NOT bracketed_bars). `items` = `[ "match"-head, scrut, arm… ]`
            // grows as arms complete; `phase` carries the in-progress arm state. On the final arm's body
            // deliver (no more `|`) it assembles `(match scrut arm…)` as the parent's operand.
            Match {
                start: Span,
                min_prec: u8,
                spine: u32,
                entered: bool,
                prefix_is_number: bool,
                items: Vec<StructId>,
                phase: MatchPhase,
            },
            // A `fn(p, …) [-> R] => body` lambda awaiting its body (single descent — the param list + return
            // type are read INLINE at dispatch; those are separate grammars, param/type de-recursion is
            // I4/I5). On deliver: ascribe the body with `ret_ty`, assemble `(fn (params) body)`.
            Fn {
                start: Span,
                min_prec: u8,
                spine: u32,
                entered: bool,
                prefix_is_number: bool,
                head: StructId,
                param_list: StructId,
                ret_ty: Option<StructId>,
            },
            // A `host E, … in body` delegation awaiting its body (single descent — the effect name list is
            // read INLINE at dispatch). On deliver: assemble `(host (E …) body)`.
            Host {
                start: Span,
                min_prec: u8,
                spine: u32,
                entered: bool,
                prefix_is_number: bool,
                head: StructId,
                effects_list: StructId,
            },
            // A `handle E[(seed)] with | … in body` form awaiting the seed, an arm body, or the final body
            // (an OPERAND-position form; NOT bracketed_bars). `head`/`effect` are known at dispatch; `phase`
            // carries the seed/arms/arms_start + in-progress arm state. On the final `in` body deliver it
            // assembles `(handle effect seed (arm…) body)` as the parent's operand.
            Handle {
                start: Span,
                min_prec: u8,
                spine: u32,
                entered: bool,
                prefix_is_number: bool,
                head: StructId,
                effect: StructId,
                phase: HandlePhase,
            },
            // A call `callee( arg, … )` awaiting an argument sub-expr — the worklist twin of `arg_exprs`
            // (the last recursion site). `callee` is the base being applied; `args` the built args so far;
            // `leading` the current arg's own-line leading comments. `call_start` is the base operand's
            // start (the whole `callee(…)` span). `saved_arm_bar` restores `arm_bar_terminates` (cleared
            // for the call interior, like `arg_exprs`). `pf_spine`/`pf_num` carry the postfix-layer counter
            // + unit-suffix gate so the postfix funnel RESUMES on the built call node (folding further
            // `.member`/`(args)`); `lvl_*` restore the owning expr level's infix state on completion.
            Call {
                callee: StructId,
                call_start: Span,
                args: Vec<StructId>,
                leading: Vec<Lead>,
                saved_arm_bar: bool,
                pf_spine: u32,
                pf_num: bool,
                lvl_min_prec: u8,
                lvl_spine: u32,
                lvl_entered: bool,
            },
            // A prefix UNARY MINUS `- <tight-operand>` awaiting its operand. The operand is a TIGHT unary
            // (`prefix + postfix`, NO trailing infix, NO unit suffix) — read as a fresh level descended at
            // `TIGHT_PREC` (suppresses infix; the funnel skips the unit suffix at that min_prec). On deliver
            // build `(- operand)`, which becomes the owning level's operand (so the OUTER postfix + unit
            // suffix apply, mirroring `expr`). `neg_start` is the `-`'s span (the whole negation's start);
            // `lvl_*` restore the owning level's infix state; `lvl_num` is its `prefix_is_number` (false for
            // a `-`-led operand, so the outer unit suffix applies). No `guard_prefix`/`depth` bookkeeping
            // here — the tight level's own reading-block depth guard replaces it, keeping depth identical.
            Neg {
                neg_start: Span,
                lvl_min_prec: u8,
                lvl_spine: u32,
                lvl_entered: bool,
                lvl_num: bool,
            },
            // An `,e` / `,{e}` unquote or `,@e` / `,@{e}` unquote-splicing awaiting its inner. `head` (the
            // `unquote`/`unquote-splicing` name) is created at dispatch — BEFORE the inner — matching
            // `unquote`'s struct-id order (head, then inner subtree). `braced`: the `{ … }` form reads a
            // FULL `expr(0)` inner (infix + unit suffix + sequencing) and consumes `}` on deliver; the bare
            // form reads a TIGHT operand (descended at TIGHT_PREC, no unit suffix, no infix — same depth
            // treatment as unary minus). On deliver build `(head inner)`, then the OUTER postfix/unit suffix
            // apply. `lvl_*` restore the owning level's infix state.
            Unquote {
                head: StructId,
                unq_start: Span,
                braced: bool,
                lvl_min_prec: u8,
                lvl_spine: u32,
                lvl_entered: bool,
                lvl_num: bool,
            },
            // An `@tag( … )` annotation GLUED-CALL name mid-read: the arg list of `@tag(a, b)` (whose result
            // `(tag a b)` becomes the annotation `name`) descends on the worklist, one arg per level, instead
            // of recursing `arg_exprs -> expr` — so a nested `@a(@b(@c(…)))` no longer grows the native stack.
            // `head` = the `@` name; `bare` = the `tag` name; `name_span` its span (for the `(tag …)` span);
            // `args` the collected args; `leading` the current arg's leading comments; `saved_arm_bar` the
            // arm-bar flag to restore after the `)`. `at_pos`/`at_span` + the `lvl_*` are the enclosing `@`
            // state, replayed to build the `TCont`/`Cont::At` form-descent once the name is complete.
            AtArgs {
                head: StructId,
                bare: StructId,
                name_span: Span,
                args: Vec<StructId>,
                leading: Vec<Lead>,
                saved_arm_bar: bool,
                at_pos: usize,
                at_span: Span,
                lvl_min_prec: u8,
                lvl_spine: u32,
                lvl_entered: bool,
                lvl_num: bool,
            },
            // An `@ann <form>` annotation awaiting its annotated FORM. `head` = the `@` name; `name` = the
            // annotation name (bare, or a glued call `(tag "x")` read inline at dispatch). The form is read
            // PREFIX-ONLY (descended at PREFIX_ONLY_PREC). `at_pos`/`form_pos` are the token slots the
            // `carry_docs` doc-shuffle uses (the leading `///` docs belong to the item below the `@`): the
            // at->form carry ran at dispatch; the form->at carry-back runs on deliver (only reached on the
            // NON-guard-trip path, matching `@`'s early return). On deliver build `(@ name form)` + apply the
            // OUTER postfix/unit suffix. `at_span` is the `@`'s span for the whole node.
            At {
                head: StructId,
                name: StructId,
                at_pos: usize,
                form_pos: usize,
                at_span: Span,
                lvl_min_prec: u8,
                lvl_spine: u32,
                lvl_entered: bool,
                lvl_num: bool,
            },
            // An `@!key <arg>` pragma awaiting its (non-`param`) TYPE arg — read as a TIGHT operand
            // (prefix+postfix, no unit suffix; descended at TIGHT_PREC). `head` = `pragma`, `key` = the
            // pragma key. On deliver build `(pragma key arg)` + apply the OUTER postfix/unit suffix. (The
            // `@!param` payload form is assembled inline at dispatch — it takes no descending arg.)
            Pragma {
                head: StructId,
                key: StructId,
                at_span: Span,
                lvl_min_prec: u8,
                lvl_spine: u32,
                lvl_entered: bool,
                lvl_num: bool,
            },
            // A `def` declaration awaiting its value/body. `target` is the name (value def `def x = …`) or
            // the built signature `(name p …)` (function def `def f(…) = …`); `ret_ty` the optional `-> R`
            // (None for a value def) applied to the body via `ascribe` on deliver — so both branches unify.
            // `docs` are the leading `///` docs (their `(doc …)` nodes built ON DELIVER, after the body, to
            // match struct-id order); `leading` is the body's own interior leading trivia (the `body_expr`
            // drain, captured before the descent). On deliver build `(def target doc… body)` + apply the
            // OUTER postfix/unit suffix. `def` is reachable as an expr operand, so `def a = def b = …`
            // de-recurses (the body descent reads the inner def iteratively).
            Def {
                def_head: StructId,
                target: StructId,
                ret_ty: Option<StructId>,
                docs: Vec<Lead>,
                leading: Vec<Lead>,
                start: Span,
                lvl_min_prec: u8,
                lvl_spine: u32,
                lvl_entered: bool,
                lvl_num: bool,
            },
            // A `module Name { member… }` declaration reading its member statements. `items` holds
            // `[ "module"-head, name, leading-doc…, member… ]`; `members_start` the index members begin at;
            // `stmt_start`/`comments` the current member's start token index + pre-captured leading `//`
            // comments (for `finish_stmt` on deliver). Each member's form is an expr(0) descent — so a
            // NESTED `module` is read on THIS worklist (no native recursion). On the last member (or an
            // empty body) `finish_module_body` closes `}` + assembles `(module …)`.
            Module {
                start: Span,
                items: Vec<StructId>,
                members_start: usize,
                stmt_start: usize,
                comments: Vec<Lead>,
                lvl_min_prec: u8,
                lvl_spine: u32,
                lvl_entered: bool,
                lvl_num: bool,
            },
        }
        // A sentinel precedence higher than every real infix/`as`/ascription/arrow precedence: a level
        // descended at `TIGHT_PREC` folds its operand + postfix but no infix, and the postfix funnel skips
        // the unit suffix — exactly the "tight operand" a unary minus / bare unquote reads. It is carried
        // through the operand's conts as their `min_prec`, so a paren/call tight operand suppresses its
        // OUTER unit suffix while its interior (read at a normal min_prec) is unaffected.
        const TIGHT_PREC: u8 = u8::MAX;
        // A second sentinel for a PREFIX-ONLY operand: `self.prefix()` with NO postfix, NO unit suffix, NO
        // infix — what the `@` annotation reads for its annotated form (so a juxtaposed `.member`/`(args)`
        // is NOT folded onto the form; it belongs to the enclosing `(@ …)`). The funnel is skipped ENTIRELY
        // at this min_prec (vs `TIGHT_PREC`, which still folds postfix). Same carried-through-conts scoping.
        const PREFIX_ONLY_PREC: u8 = u8::MAX - 1;
        let mut pending: Vec<Cont> = Vec::new();
        let mut cur_min_prec = min_prec;
        let mut cur_start = self.cur_span();
        let mut left: StructId = StructId(0); // placeholder; always assigned before use (reading=true first)
        let mut spine: u32 = 0;
        let mut entered = false; // did THIS level increment self.depth (mirrors `expr`'s guarded entry)?
        // `reading`: begin a FRESH level by reading its operand; else `left` already holds a completed
        // sub-level combined into the resumed parent, and we re-enter its infix loop directly.
        let mut reading = true;
        // POSTFIX FUNNEL (I3 arg_exprs de-recursion): once an operand is produced, apply its `.member` /
        // `(args)` postfix chain iteratively before the infix loop, so a call ARGUMENT is read on THIS
        // worklist (a `Cont::Call` descent) rather than recursing through `postfix -> arg_exprs -> expr`.
        // A site that has produced a bare operand sets `pf_pending = true` (with `pf_num` = whether the
        // operand began with a number, gating the unit suffix, and `pf_spine` the postfix-layer depth
        // counter) and leaves `left`/`cur_start` at the operand; the funnel below folds the chain. Sites
        // NOT yet converted still call the recursive `self.postfix` directly — both produce byte-identical
        // arenas, so the conversion is incremental + oracle-green at every step.
        let mut pf_pending = false;
        let mut pf_num = false;
        let mut pf_spine: u32 = 0;
        loop {
            if reading {
                cur_start = self.cur_span();
                // DEPTH GUARD (mirrors `expr` entry): past the limit this level's value is an `error_node`
                // WITHOUT incrementing depth (as `expr` early-returns before its `self.depth += 1`).
                if self.depth >= crate::sexpr::MAX_NESTING_DEPTH {
                    if !self.depth_exceeded {
                        self.error("expression nests too deeply to parse");
                        self.depth_exceeded = true;
                    }
                    left = self.error_node(cur_start);
                    spine = 0;
                    entered = false;
                } else {
                    self.depth += 1;
                    entered = true;
                    let prefix_is_number = matches!(self.kind(), Kind::Int | Kind::Float);
                    // OPERAND DISPATCH (I3, family-by-family): a worklist-handled operand family pushes a
                    // `Cont` and DESCENDS (read its sub-expr as a fresh level) rather than recursing through
                    // `prefix`; every OTHER operand falls back to the (recursive) `prefix` — so each stage
                    // stays byte-identical (oracle-green) as families are pulled onto the worklist one at a
                    // time. quasiquote `` `{ e } `` is the first family (a clean single braced sub-expr).
                    if self.kind() == Kind::Backtick {
                        let head = self.name("quasiquote", cur_start);
                        self.bump(); // `` ` ``
                        self.expect(Kind::LBrace, "`{`");
                        pending.push(Cont::QuasiQuote {
                            head,
                            start: cur_start,
                            min_prec: cur_min_prec,
                            spine,
                            entered,
                            prefix_is_number,
                        });
                        cur_min_prec = 0; // the braced inner is `self.expr(0)`
                        continue; // descend: read the inner as a fresh level (reading stays true)
                    } else if self.kind() == Kind::LParen {
                        // `( … )` — mirror `bracketed_bars(paren)`: clear `arm_bar_terminates` for the
                        // interior (restored on assemble). `()` is unit (inline, no sub-expr); otherwise
                        // descend to read the FIRST sub-expr (`expr(0)`) — the Paren cont then decides
                        // grouping vs tuple on deliver.
                        let arm_bar = self.arm_bar_terminates;
                        self.arm_bar_terminates = false;
                        self.expect(Kind::LParen, "`(`");
                        if self.at(Kind::RParen) {
                            self.bump();
                            let span = cur_start.merge(self.prev_span());
                            self.arm_bar_terminates = arm_bar;
                            left = self.name("unit", span);
                            spine = 0;
                            pf_pending = true;
                            pf_num = prefix_is_number;
                            pf_spine = 0;
                        } else if self.at(Kind::DotDot) {
                            // A LEADING `.. a` spread forces the tuple path (no grouping meaning): create the
                            // tuple head THEN the `..` head (matching the recursive `ctor_head` -> `rest_marker`
                            // struct-id order), then descend the spread operand (at PREC_SEQ+1, as
                            // `rest_marker` reads it). `items` starts NON-empty (tuple mode).
                            let head = self.ctor_head("tuple", cur_start);
                            let dd = self.cur_span();
                            self.bump(); // `..`
                            let rest_head = self.name("..", dd);
                            pending.push(Cont::Paren {
                                start: cur_start,
                                min_prec: cur_min_prec,
                                spine,
                                entered,
                                prefix_is_number,
                                arm_bar,
                                pending_leading: Vec::new(),
                                items: vec![head],
                                spread: true,
                                rest_head,
                            });
                            cur_min_prec = crate::token::PREC_SEQ + 1;
                            continue; // descend: read the spread operand as a fresh level
                        } else {
                            let first_leading = self.take_comments_here();
                            pending.push(Cont::Paren {
                                start: cur_start,
                                min_prec: cur_min_prec,
                                spine,
                                entered,
                                prefix_is_number,
                                arm_bar,
                                pending_leading: first_leading,
                                items: Vec::new(),
                                spread: false,
                                rest_head: StructId(0),
                            });
                            cur_min_prec = 0; // `first = self.expr(0)`
                            continue; // descend: read `first` as a fresh level
                        }
                    } else if self.kind() == Kind::LBracket
                        || self.kind() == Kind::BinOpen
                        || (self.kind() == Kind::Hash
                            && matches!(self.nth_kind(1), Kind::LParen | Kind::LBracket))
                    {
                        // A `,`-separated comma-list operand family — one iterative skeleton shared by four
                        // recursive bodies (mirror `bracketed_bars(list_literal/set_literal/bin_literal/
                        // hash_list)`): head created BEFORE elements (span-order), then elements read at
                        // `PREC_SEQ+1`. The head + closer + three behavior flags select the family:
                        //   `[…]`  list → ctor "list" head, `]`, rest + comments + drain
                        //   `b[…]` bin  → name "bin" head,  `]`, comments only (NO rest, NO drain)
                        //   `#[…]` raw  → NO head,          `]`, none (bare elements)
                        //   `#(…)` set  → ctor "set" head,  `)`, rest + comments + drain
                        let arm_bar = self.arm_bar_terminates;
                        self.arm_bar_terminates = false;
                        let (mut items, closer, allow_rest, allow_comments, drain_closer) = if self
                            .kind()
                            == Kind::LBracket
                        {
                            self.bump(); // '['
                            (
                                vec![self.ctor_head("list", cur_start)],
                                Kind::RBracket,
                                true,
                                true,
                                true,
                            )
                        } else if self.kind() == Kind::BinOpen {
                            self.bump(); // `b[`
                            (
                                vec![self.name("bin", cur_start)],
                                Kind::RBracket,
                                false,
                                true,
                                false,
                            )
                        } else if self.kind() == Kind::Hash && self.nth_kind(1) == Kind::LBracket {
                            self.bump(); // '#'
                            self.bump(); // '['
                            (Vec::new(), Kind::RBracket, false, false, false)
                        } else {
                            self.bump(); // '#'
                            self.bump(); // '('
                            (
                                vec![self.ctor_head("set", cur_start)],
                                Kind::RParen,
                                true,
                                true,
                                true,
                            )
                        };
                        if self.at(closer) {
                            if drain_closer {
                                self.drain_closer_comment_onto_last(&mut items, 1);
                            }
                            self.expect(closer, "comma-list closer");
                            let span = cur_start.merge(self.prev_span());
                            self.arm_bar_terminates = arm_bar;
                            left = self.list(items, span);
                            spine = 0;
                            pf_pending = true;
                            pf_num = prefix_is_number;
                            pf_spine = 0;
                        } else {
                            let before = self.pos;
                            // A `.. rest` spread (list/set only): create the `..` head NOW (before the binder
                            // descends, matching `rest_marker`'s head-then-operand order). Else an ordinary
                            // element, with its own-line leading comments captured when the family allows.
                            let (is_rest, dd_span, rest_head) =
                                if allow_rest && self.at(Kind::DotDot) {
                                    let dd = self.cur_span();
                                    self.bump(); // `..`
                                    (true, dd, self.name("..", dd))
                                } else {
                                    (false, cur_start, StructId(0))
                                };
                            let pending_leading = if allow_comments && !is_rest {
                                self.take_comments_here()
                            } else {
                                Vec::new()
                            };
                            pending.push(Cont::List {
                                start: cur_start,
                                min_prec: cur_min_prec,
                                spine,
                                entered,
                                prefix_is_number,
                                arm_bar,
                                closer,
                                allow_rest,
                                allow_comments,
                                drain_closer,
                                items,
                                pending_leading,
                                is_rest,
                                dd_span,
                                rest_head,
                                before,
                            });
                            cur_min_prec = crate::token::PREC_SEQ + 1;
                            continue; // descend: read the first element as a fresh level
                        }
                    } else if self.kind() == Kind::LBrace
                        || (self.kind() == Kind::Hash && self.nth_kind(1) == Kind::LBrace)
                    {
                        // `{ … }` record OR `#{ … }` map — the field-pair operand families (mirror
                        // `bracketed_bars(record_literal/map_literal)`): ctor head first, then fields via
                        // `advance_fields`. Empty is inline; a field needing a sub-expr descends (Cont::Fields);
                        // a run of record shorthands completes inline. On close, assemble as the operand.
                        let arm_bar = self.arm_bar_terminates;
                        self.arm_bar_terminates = false;
                        let is_map = self.kind() == Kind::Hash;
                        let head = if is_map {
                            let h = self.ctor_head("map", cur_start);
                            self.bump(); // '#'
                            self.bump(); // '{'
                            h
                        } else {
                            let h = self.ctor_head("record", cur_start);
                            self.bump(); // '{'
                            h
                        };
                        let mut items = vec![head];
                        // `advance_fields` returns None (closed inline: `{}`, or all-shorthand `{x, y}`) or a
                        // pending field descent. Assemble on None; push Cont::Fields + descend otherwise.
                        let step = if self.at(Kind::RBrace) {
                            self.drain_closer_comment_onto_last(&mut items, 1);
                            self.expect(Kind::RBrace, "`}`");
                            None
                        } else {
                            self.advance_fields(&mut items, is_map, Kind::RBrace)
                        };
                        match step {
                            None => {
                                let span = cur_start.merge(self.prev_span());
                                self.arm_bar_terminates = arm_bar;
                                left = self.list(items, span);
                                spine = 0;
                                pf_pending = true;
                                pf_num = prefix_is_number;
                                pf_spine = 0;
                            }
                            Some((phase, before)) => {
                                pending.push(Cont::Fields {
                                    start: cur_start,
                                    min_prec: cur_min_prec,
                                    spine,
                                    entered,
                                    prefix_is_number,
                                    arm_bar,
                                    is_map,
                                    items,
                                    phase,
                                    before,
                                });
                                cur_min_prec = crate::token::PREC_SEQ + 1;
                                continue; // descend: read the field's sub-expr as a fresh level
                            }
                        }
                    } else if self.at_keyword(Keyword::If) {
                        // `if c then t else e` — the first keyword form pulled onto the worklist. NOT
                        // bracketed_bars (arm_bar_terminates untouched). Create the head, capture the
                        // condition's own-line leading comments, then descend to read the condition; the
                        // Cont::If phase machine handles `then`/`else` + assembly on deliver.
                        let head = self.keyword_head("if", cur_start);
                        self.bump(); // `if`
                        let c_lead = self.take_comments_here();
                        pending.push(Cont::If {
                            start: cur_start,
                            min_prec: cur_min_prec,
                            spine,
                            entered,
                            prefix_is_number,
                            head,
                            phase: IfPhase::Cond { c_lead },
                        });
                        cur_min_prec = crate::token::PREC_SEQ + 1;
                        continue; // descend: read the condition as a fresh level
                    } else if self.at_keyword(Keyword::Let) {
                        // `let b = v, … in body` — NOT bracketed_bars. Create the head, then read the FIRST
                        // binding's inline preamble (leading comments + binder [flat name or recursive
                        // `pattern()`] + `=` + value-leading comments) and descend its value. The Cont::Let
                        // phase machine handles the `,`-binding loop, `in`, the body, and assembly.
                        let head = self.keyword_head("let", cur_start);
                        self.bump(); // `let`
                        let leading = self.take_comments_here();
                        let b_start = self.cur_span();
                        let n = self.read_let_binder(b_start);
                        self.expect(Kind::Eq, "`=`");
                        let e_lead = self.take_comments_here();
                        pending.push(Cont::Let {
                            start: cur_start,
                            min_prec: cur_min_prec,
                            spine,
                            entered,
                            prefix_is_number,
                            head,
                            phase: LetPhase::BindingValue {
                                bindings: Vec::new(),
                                n,
                                leading,
                                b_start,
                                e_lead,
                            },
                        });
                        cur_min_prec = crate::token::PREC_SEQ + 1;
                        continue; // descend: read the first binding's value as a fresh level
                    } else if self.at_keyword(Keyword::Match) {
                        // `match scrut with | … ` — NOT bracketed_bars. Create the head + descend the
                        // scrutinee; the Cont::Match phase machine handles `with`, the arm loop (pattern
                        // inline, guard/body descents), and assembly.
                        let head = self.keyword_head("match", cur_start);
                        self.bump(); // `match`
                        pending.push(Cont::Match {
                            start: cur_start,
                            min_prec: cur_min_prec,
                            spine,
                            entered,
                            prefix_is_number,
                            items: vec![head],
                            phase: MatchPhase::Scrut,
                        });
                        cur_min_prec = crate::token::PREC_SEQ + 1;
                        continue; // descend: read the scrutinee as a fresh level
                    } else if self.at_keyword(Keyword::Fn) {
                        // `fn(p, …) [-> R] => body` — read the param list + optional return type INLINE
                        // (separate grammars), consume `=>`, then descend the body (`expr(0)`). NOT
                        // bracketed_bars. Cont::Fn ascribes + assembles on deliver.
                        let head = self.keyword_head("fn", cur_start);
                        self.bump(); // `fn`
                        let param_list = self.param_list();
                        let ret_ty = self.opt_return_type();
                        self.expect(Kind::FatArrow, "`=>`");
                        pending.push(Cont::Fn {
                            start: cur_start,
                            min_prec: cur_min_prec,
                            spine,
                            entered,
                            prefix_is_number,
                            head,
                            param_list,
                            ret_ty,
                        });
                        cur_min_prec = 0; // the body is `expr(0)` (a sequence position)
                        continue; // descend: read the body as a fresh level
                    } else if self.at_keyword(Keyword::Host) {
                        // `host E, … in body` — read the comma-separated effect name list INLINE, consume
                        // `in`, then descend the body (`expr(0)`). Cont::Host assembles on deliver.
                        let head = self.keyword_head("host", cur_start);
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
                        pending.push(Cont::Host {
                            start: cur_start,
                            min_prec: cur_min_prec,
                            spine,
                            entered,
                            prefix_is_number,
                            head,
                            effects_list,
                        });
                        cur_min_prec = 0; // the body is `expr(0)`
                        continue; // descend: read the body as a fresh level
                    } else if self.at_keyword(Keyword::Def) {
                        // `def name = value` (value def) or `def name(p,…) [-> R] = body` (function def). The
                        // preamble (leading `///` docs, `forall` type-params, name, params, return type) is
                        // read INLINE — only the value/body is an expr descent (PREC_SEQ+1 for a value,
                        // 0 for a function body, a sequence position). Cont::Def builds `(def target doc… body)`
                        // on deliver. `def` is an expr operand, so `def a = def b = …` now de-recurses.
                        let start = cur_start;
                        let docs = self.take_docs_here();
                        let def_head = self.keyword_head("def", start);
                        self.bump(); // `def`
                        let sig_start = self.cur_span();
                        let sig_type_params = self.forall_sig_type_params();
                        let name = self.binder();
                        if sig_type_params.is_none() && self.at(Kind::Eq) {
                            // value def: `def name = value`
                            self.bump(); // `=`
                            let leading = if self.pos < self.leading.len() {
                                std::mem::take(&mut self.leading[self.pos])
                            } else {
                                Vec::new()
                            };
                            pending.push(Cont::Def {
                                def_head,
                                target: name,
                                ret_ty: None,
                                docs,
                                leading,
                                start,
                                lvl_min_prec: cur_min_prec,
                                lvl_spine: spine,
                                lvl_entered: entered,
                                lvl_num: prefix_is_number,
                            });
                            cur_min_prec = crate::token::PREC_SEQ + 1;
                            continue; // descend: read the value
                        }
                        // function def: `def name(p,…) [-> R] = body`
                        self.expect(Kind::LParen, "`(`");
                        let mut params = Vec::new();
                        if !self.at(Kind::RParen) {
                            loop {
                                let before = self.pos;
                                params.push(self.param());
                                if !self.sep_continue(Kind::RParen) {
                                    break;
                                }
                                if self.pos == before {
                                    self.bump();
                                }
                            }
                        }
                        self.expect(Kind::RParen, "`)`");
                        let sig_span = sig_start.merge(self.prev_span());
                        let params = self.hoist_forall_params(params, sig_span);
                        let mut sig = vec![name];
                        if let Some(tps) = sig_type_params {
                            sig.extend(tps);
                        }
                        sig.extend(params);
                        let signature = self.list(sig, sig_span);
                        let ret_ty = self.opt_return_type();
                        self.expect(Kind::Eq, "`=`");
                        let leading = if self.pos < self.leading.len() {
                            std::mem::take(&mut self.leading[self.pos])
                        } else {
                            Vec::new()
                        };
                        pending.push(Cont::Def {
                            def_head,
                            target: signature,
                            ret_ty,
                            docs,
                            leading,
                            start,
                            lvl_min_prec: cur_min_prec,
                            lvl_spine: spine,
                            lvl_entered: entered,
                            lvl_num: prefix_is_number,
                        });
                        cur_min_prec = 0; // the function body is `expr(0)` (a sequence position)
                        continue; // descend: read the body
                    } else if self.at_keyword(Keyword::Module) {
                        // `module Name { member… }` — read the preamble (docs, name, leading (doc…), `{`)
                        // INLINE, then read members via the worklist: each member's form is an expr(0)
                        // descent (Cont::Module), so a NESTED module de-recurses. On close, finish_module_body
                        // drains the trailing `}` slot + assembles. Empty body finishes inline.
                        let start = cur_start;
                        let docs = self.take_docs_here();
                        let head = self.keyword_head("module", start);
                        self.bump(); // `module`
                        let name = self.binder();
                        let mut items = vec![head, name];
                        items.extend(self.doc_nodes(docs));
                        self.expect(Kind::LBrace, "`{`");
                        let members_start = items.len();
                        if !self.at(Kind::RBrace) && !self.at_end() {
                            let stmt_start = self.pos;
                            let comments = self.take_comments_here();
                            pending.push(Cont::Module {
                                start,
                                items,
                                members_start,
                                stmt_start,
                                comments,
                                lvl_min_prec: cur_min_prec,
                                lvl_spine: spine,
                                lvl_entered: entered,
                                lvl_num: prefix_is_number,
                            });
                            cur_min_prec = 0; // a member form is a statement = `expr(0)`
                            continue; // descend: read the first member
                        }
                        // Empty body (`module M {}`) — no member descent; finish inline.
                        left = self.finish_module_body(items, members_start, start);
                        spine = 0;
                        pf_pending = true;
                        pf_num = prefix_is_number;
                        pf_spine = 0;
                    } else if self.at_keyword(Keyword::Handle) {
                        // `handle E[(seed)] with | op(…, s) => body | … in body` — NOT bracketed_bars. Read
                        // the effect name + seed head INLINE; the seed of `E(seed)` descends (Cont::Handle
                        // Seed), while `E`/`E()` seeds are the inline `unit`. The Cont::Handle phase machine
                        // handles `with`, the arm loop (header inline, body descents with arm_bar=true), `in`,
                        // and assembly.
                        let head = self.keyword_head("handle", cur_start);
                        self.bump(); // `handle`
                        let effect = self.binder();
                        // The seed: `E(seed)` descends; `E()` is a `unit` at the `)`'s span; bare `E` is a
                        // `unit` at the handle start. Only the descending case suspends; the others start the
                        // first arm inline (mirrors `handle_expr`).
                        let inline_seed = if self.at(Kind::LParen) {
                            self.bump(); // `(`
                            if self.at(Kind::RParen) {
                                let sp = self.cur_span();
                                let seed = self.name("unit", sp);
                                self.expect(Kind::RParen, "`)`");
                                Some(seed)
                            } else {
                                None // a real seed expr — descend it
                            }
                        } else {
                            Some(self.name("unit", cur_start))
                        };
                        match inline_seed {
                            Some(seed) => {
                                let arms_start = self.handle_after_seed();
                                let (arm_start, op, params, state, saved_arm_bar) =
                                    self.handle_arm_header();
                                pending.push(Cont::Handle {
                                    start: cur_start,
                                    min_prec: cur_min_prec,
                                    spine,
                                    entered,
                                    prefix_is_number,
                                    head,
                                    effect,
                                    phase: HandlePhase::ArmBody {
                                        seed,
                                        arms_start,
                                        arms: Vec::new(),
                                        arm_start,
                                        op,
                                        params,
                                        state,
                                        saved_arm_bar,
                                    },
                                });
                                cur_min_prec = 0; // the arm body is `expr(0)`
                                continue; // descend: read the first arm's body
                            }
                            None => {
                                pending.push(Cont::Handle {
                                    start: cur_start,
                                    min_prec: cur_min_prec,
                                    spine,
                                    entered,
                                    prefix_is_number,
                                    head,
                                    effect,
                                    phase: HandlePhase::Seed,
                                });
                                cur_min_prec = 0; // the seed is `expr(0)`
                                continue; // descend: read the seed
                            }
                        }
                    } else if self.kind() == Kind::At {
                        // `@ann <form>` annotation -> `(@ name form)`. Read the annotation name (bare, or a
                        // GLUED call `@tag("x")` -> `(tag "x")` via inline arg_exprs), shuffle any leading
                        // `///` docs down to the form slot (carry_docs), then read the FORM prefix-only
                        // (descended at PREFIX_ONLY_PREC — no postfix/unit/infix). The `@`'s OWN depth guard
                        // is handled INLINE (byte-identical to `@`'s early return) so a `@a @b @c … def`
                        // stack declines at the same point/shape; the form level's own guard covers a deep
                        // form. The glued-call name args (`@tag(a, b)`) descend on the worklist via
                        // Cont::AtArgs (below), so a nested `@a(@b(@c(…)))` no longer recurses.
                        let at_pos = self.pos;
                        self.bump(); // `@`
                        let head = self.name("@", cur_start);
                        let name = if self.at(Kind::Ident) {
                            let name_span = self.cur_span();
                            let t = self.bump().unwrap();
                            let bare = self.name(self.text(t), name_span);
                            if self.at(Kind::LParen)
                                && self.prev_span().end == self.cur_span().start
                            {
                                // GLUED call `@tag(a, b)` -> name `(tag a b)`. De-recurse the arg list onto
                                // the worklist (Cont::AtArgs, mirroring `arg_exprs`'s arm-bar + comment
                                // handling) instead of the recursive `arg_exprs` -> `expr`, so a nested
                                // `@a(@b(@c(…)))` no longer grows the native stack.
                                self.expect(Kind::LParen, "`(`");
                                let saved_arm_bar = self.arm_bar_terminates;
                                self.arm_bar_terminates = false;
                                if self.at(Kind::RParen) {
                                    // Empty glued call `@tag()` -> `(tag)` (one-element list), no descent.
                                    self.expect(Kind::RParen, "`)`");
                                    self.arm_bar_terminates = saved_arm_bar;
                                    let call_span = name_span.merge(self.prev_span());
                                    self.list(vec![bare], call_span)
                                } else {
                                    // Descend the FIRST argument as a fresh level; Cont::AtArgs collects the
                                    // rest, builds `(tag args)`, then runs the `@` form descent on resume.
                                    let leading = self.take_comments_here();
                                    pending.push(Cont::AtArgs {
                                        head,
                                        bare,
                                        name_span,
                                        args: Vec::new(),
                                        leading,
                                        saved_arm_bar,
                                        at_pos,
                                        at_span: cur_start,
                                        lvl_min_prec: cur_min_prec,
                                        lvl_spine: spine,
                                        lvl_entered: entered,
                                        lvl_num: prefix_is_number,
                                    });
                                    cur_min_prec = crate::token::PREC_SEQ + 1;
                                    continue; // descend: read the first glued-call argument
                                }
                            } else {
                                bare
                            }
                        } else {
                            self.error("expected an annotation name after `@`");
                            self.error_node(self.cur_span())
                        };
                        let form_pos = self.pos;
                        self.carry_docs(at_pos, form_pos);
                        if self.depth >= crate::sexpr::MAX_NESTING_DEPTH {
                            // `@`'s own guard trips: `(@ name <error>)` at the `@`'s span, no form, no
                            // carry-back (matches `guard_prefix`'s early return in the `@` arm).
                            if !self.depth_exceeded {
                                self.error("expression nests too deeply to parse");
                                self.depth_exceeded = true;
                            }
                            let err = self.error_node(cur_start);
                            left =
                                self.list(vec![head, name, err], cur_start.merge(self.prev_span()));
                            spine = 0;
                            pf_pending = true;
                            pf_num = prefix_is_number;
                            pf_spine = 0;
                        } else {
                            pending.push(Cont::At {
                                head,
                                name,
                                at_pos,
                                form_pos,
                                at_span: cur_start,
                                lvl_min_prec: cur_min_prec,
                                lvl_spine: spine,
                                lvl_entered: entered,
                                lvl_num: prefix_is_number,
                            });
                            cur_min_prec = PREFIX_ONLY_PREC;
                            continue; // descend: read the annotated form prefix-only
                        }
                    } else if self.kind() == Kind::AtBang {
                        // `@!key <arg>` pragma -> `(pragma key arg)`. `@!param` takes a payload (config kvs
                        // + a `name : Type` binder), assembled INLINE (it returns directly, no descending
                        // arg). Every other key takes a single TIGHT TYPE arg (prefix+postfix, no unit;
                        // descended at TIGHT_PREC). The `@!`'s own depth guard trips INLINE like `@`.
                        // (NOTE: `@!param`'s config kv VALUES still use inline recursive `self.expr` — a
                        // minor remaining vector, sibling to the `@tag(...)` glued-args, to convert before I8.)
                        self.bump(); // `@!`
                        let head = self.name("pragma", cur_start);
                        let mut key_text = String::new();
                        let key = if self.at(Kind::Ident) {
                            let key_span = self.cur_span();
                            let t = self.bump().unwrap();
                            key_text = self.text(t).to_string();
                            self.name(key_text.as_str(), key_span)
                        } else {
                            self.error(
                                "expected a pragma key after `@!` (e.g. `@!default-float Float32`)",
                            );
                            self.error_node(self.cur_span())
                        };
                        if key_text == "param" {
                            let payload = self.param_pragma_payload();
                            let full = cur_start.merge(self.prev_span());
                            let mut items = vec![head, key];
                            items.extend(payload);
                            left = self.list(items, full);
                            spine = 0;
                            pf_pending = true;
                            pf_num = prefix_is_number;
                            pf_spine = 0;
                        } else if self.depth >= crate::sexpr::MAX_NESTING_DEPTH {
                            if !self.depth_exceeded {
                                self.error("expression nests too deeply to parse");
                                self.depth_exceeded = true;
                            }
                            let err = self.error_node(cur_start);
                            left =
                                self.list(vec![head, key, err], cur_start.merge(self.prev_span()));
                            spine = 0;
                            pf_pending = true;
                            pf_num = prefix_is_number;
                            pf_spine = 0;
                        } else {
                            pending.push(Cont::Pragma {
                                head,
                                key,
                                at_span: cur_start,
                                lvl_min_prec: cur_min_prec,
                                lvl_spine: spine,
                                lvl_entered: entered,
                                lvl_num: prefix_is_number,
                            });
                            cur_min_prec = TIGHT_PREC;
                            continue; // descend: read the pragma arg as a tight operand
                        }
                    } else if self.kind() == Kind::Comma || self.kind() == Kind::UnquoteSplice {
                        // `,e`/`,{e}` unquote or `,@e`/`,@{e}` unquote-splicing (a prefix-position comma —
                        // a separator comma is consumed by `sep_continue`, never read as an operand). Head
                        // created NOW (before the inner). Braced form reads a full `expr(0)` (min_prec 0) +
                        // consumes `}`; bare form reads a TIGHT operand at TIGHT_PREC.
                        let head_name = if self.kind() == Kind::Comma {
                            "unquote"
                        } else {
                            "unquote-splicing"
                        };
                        let head = self.name(head_name, cur_start);
                        self.bump(); // `,` or `,@`
                        let braced = self.at(Kind::LBrace);
                        if braced {
                            self.bump(); // `{`
                        }
                        pending.push(Cont::Unquote {
                            head,
                            unq_start: cur_start,
                            braced,
                            lvl_min_prec: cur_min_prec,
                            lvl_spine: spine,
                            lvl_entered: entered,
                            lvl_num: prefix_is_number,
                        });
                        cur_min_prec = if braced { 0 } else { TIGHT_PREC };
                        continue; // descend: read the unquote inner as a fresh level
                    } else if self.kind() == Kind::Minus {
                        // PREFIX UNARY MINUS `- <tight-operand>` -> `(- operand)`. The operand is a TIGHT
                        // unary — descend it at TIGHT_PREC (no infix, unit suffix suppressed); Cont::Neg
                        // wraps it + applies the OUTER postfix/unit suffix on deliver. No `guard_prefix`
                        // here: the tight level's own reading-block depth guard replaces it (identical
                        // depth accounting), so a `- - - … x` run declines at the same point + shape.
                        self.bump(); // `-`
                        pending.push(Cont::Neg {
                            neg_start: cur_start,
                            lvl_min_prec: cur_min_prec,
                            lvl_spine: spine,
                            lvl_entered: entered,
                            lvl_num: prefix_is_number,
                        });
                        cur_min_prec = TIGHT_PREC;
                        continue; // descend: read the tight operand as a fresh level
                    } else {
                        left = self.prefix();
                        spine = 0;
                        // Enter the postfix funnel (iterative `.member`/`(args)` folding) instead of the
                        // recursive `self.postfix` + `maybe_unit_suffix`.
                        pf_pending = true;
                        pf_num = prefix_is_number;
                        pf_spine = 0;
                    }
                }
            }
            // POSTFIX FUNNEL: fold `.member` layers inline (no descent) and `(args)` call layers via the
            // worklist (`Cont::Call` — each argument is a fresh level, so a call ARGUMENT no longer recurses
            // `postfix -> arg_exprs -> expr`). Runs after an operand is produced (a site set `pf_pending`)
            // and before this level's infix loop. Byte-identical to `postfix` + the trailing
            // `maybe_unit_suffix` (guard, member/call order, empty-call `(callee)`, arg comment slots).
            if pf_pending && cur_min_prec == PREFIX_ONLY_PREC {
                // A PREFIX-ONLY operand (the `@` annotated form): no postfix + no unit suffix at all.
                pf_pending = false;
            }
            if pf_pending {
                pf_pending = false;
                let mut pf_descended = false; // set if a Cont::Call was pushed (an arg must be read)
                loop {
                    match self.kind() {
                        Kind::Dot if self.dot_is_member() => {
                            left = self.member_access(left, cur_start);
                            pf_spine += 1;
                            if !self.depth_exceeded
                                && self.depth + pf_spine >= crate::sexpr::MAX_NESTING_DEPTH
                            {
                                self.error("expression nests too deeply to parse");
                                self.depth_exceeded = true;
                                break; // postfix stops (mirrors `postfix`'s early return)
                            }
                        }
                        Kind::LParen => {
                            self.expect(Kind::LParen, "`(`");
                            // A call `( … )` is a bracket boundary — `|` inside an arg is bitwise-or, so
                            // clear `arm_bar_terminates` for the call interior (restored after), like
                            // `arg_exprs`/`bracketed_bars`.
                            let saved_arm_bar = self.arm_bar_terminates;
                            self.arm_bar_terminates = false;
                            if self.at(Kind::RParen) {
                                // Empty call `f()` -> `(callee)` (a one-element list), no arg descent.
                                self.expect(Kind::RParen, "`)`");
                                self.arm_bar_terminates = saved_arm_bar;
                                let span = cur_start.merge(self.prev_span());
                                left = self.list(vec![left], span);
                                pf_spine += 1;
                                if !self.depth_exceeded
                                    && self.depth + pf_spine >= crate::sexpr::MAX_NESTING_DEPTH
                                {
                                    self.error("expression nests too deeply to parse");
                                    self.depth_exceeded = true;
                                    break;
                                }
                                continue; // fold further postfix onto the call result
                            }
                            // A real argument list — descend the first arg on the worklist.
                            let leading = self.take_comments_here();
                            pending.push(Cont::Call {
                                callee: left,
                                call_start: cur_start,
                                args: Vec::new(),
                                leading,
                                saved_arm_bar,
                                pf_spine,
                                pf_num,
                                lvl_min_prec: cur_min_prec,
                                lvl_spine: spine,
                                lvl_entered: entered,
                            });
                            cur_min_prec = crate::token::PREC_SEQ + 1;
                            reading = true; // read the argument as a fresh level
                            pf_descended = true;
                            break;
                        }
                        _ => break, // no more postfix layers
                    }
                }
                if pf_descended {
                    continue; // go descend the call argument; Cont::Call resumes the funnel
                }
                // Postfix chain complete — apply the unit suffix (unless this is a TIGHT operand, which
                // omits it), then fall into the infix loop.
                if !pf_num && cur_min_prec != TIGHT_PREC {
                    left = self.maybe_unit_suffix(left, cur_start);
                }
            }
            // The infix loop for the current level (on `left` / `cur_min_prec` / `cur_start` / `spine`).
            let mut suspended = false;
            loop {
                if self.at_keyword(Keyword::As)
                    && crate::token::PREC_AS >= cur_min_prec
                    && !self.src[self.prev_span().end..self.cur_span().start].contains('\n')
                {
                    left = self.as_conversion(left, cur_start);
                    continue;
                }
                let Some(op_name) = self.infix_op() else {
                    break;
                };
                let prec = infix_prec(op_name).expect("infix_op returns only infix names");
                if prec < cur_min_prec {
                    break;
                }
                let op_span = self.cur_span();
                let left_trailing = self.take_trailing_comment_here();
                left = self.wrap_comment_after(left_trailing, left);
                let right_leading = self.take_comments_here();
                self.bump(); // operator
                let head = self.name(op_name, op_span);
                // `:`+`forall` intercept: the RIGHT operand is a `forall_type`, NOT a sub-`expr` level —
                // compute + combine it inline (no descent), exactly as `expr`.
                if op_name == ":" && self.at_keyword(Keyword::Forall) {
                    let right = self.forall_type(self.cur_span());
                    let right = self.wrap_comments(right_leading, right);
                    let span = cur_start.merge(self.prev_span());
                    left = self.list(vec![head, left, right], span);
                    spine += 1;
                    if !self.depth_exceeded && self.depth + spine >= crate::sexpr::MAX_NESTING_DEPTH
                    {
                        self.error("expression nests too deeply to parse");
                        self.depth_exceeded = true;
                        break;
                    }
                    continue;
                }
                // Otherwise SUSPEND this level and DESCEND: the right operand is `expr(right_min)`.
                let right_min = if is_right_assoc(op_name) {
                    prec
                } else {
                    prec + 1
                };
                pending.push(Cont::Op {
                    left,
                    head,
                    right_leading,
                    start: cur_start,
                    min_prec: cur_min_prec,
                    spine,
                });
                cur_min_prec = right_min;
                suspended = true;
                break;
            }
            if suspended {
                reading = true;
                continue; // read the right operand as a fresh level
            }
            // COMPLETE this level — `left` is `expr(cur_min_prec)`'s value (mirrors `expr`'s tail).
            if cur_min_prec == crate::token::PREC_SEQ && self.at(Kind::Semi) {
                left = self.finish_sequence(left, cur_start);
            }
            if entered {
                self.depth -= 1;
            }
            // REDUCE into the parent continuation, then resume the parent level's infix loop.
            match pending.pop() {
                None => return left,
                Some(Cont::Op {
                    left: parent_left,
                    head,
                    right_leading,
                    start,
                    min_prec: parent_min_prec,
                    spine: parent_spine,
                }) => {
                    let right = self.wrap_comments(right_leading, left);
                    let span = start.merge(self.prev_span());
                    left = self.list(vec![head, parent_left, right], span);
                    cur_start = start;
                    cur_min_prec = parent_min_prec;
                    spine = parent_spine + 1;
                    entered = true; // a suspended parent had read its operand, so it incremented depth
                    if !self.depth_exceeded && self.depth + spine >= crate::sexpr::MAX_NESTING_DEPTH
                    {
                        self.error("expression nests too deeply to parse");
                        self.depth_exceeded = true;
                        // The parent completes: its infix loop below breaks immediately (`at_end`).
                    }
                    reading = false; // re-enter the parent's infix loop on the combined `left`
                    continue;
                }
                Some(Cont::QuasiQuote {
                    head,
                    start,
                    min_prec: parent_min_prec,
                    spine: parent_spine,
                    entered: parent_entered,
                    prefix_is_number,
                }) => {
                    // `left` is the braced inner expr; close `}`, wrap, and it becomes the PARENT level's
                    // operand — so postfix + unit-suffix apply, then the parent's infix loop resumes.
                    self.expect(Kind::RBrace, "`}`");
                    let span = start.merge(self.prev_span());
                    left = self.list(vec![head, left], span);
                    cur_start = start;
                    cur_min_prec = parent_min_prec;
                    spine = parent_spine;
                    entered = parent_entered;
                    pf_pending = true; // postfix funnel applies the `.member`/`(args)` chain + unit suffix
                    pf_num = prefix_is_number;
                    pf_spine = 0;
                    reading = false; // re-enter the parent level's infix loop with the quasiquote operand
                    continue;
                }
                Some(Cont::Paren {
                    start,
                    min_prec: parent_min_prec,
                    spine: parent_spine,
                    entered: parent_entered,
                    prefix_is_number,
                    arm_bar,
                    pending_leading,
                    mut items,
                    spread,
                    rest_head,
                }) => {
                    // Assemble the paren operand from the delivered sub-expr, restore state, and resume the
                    // parent level's infix loop with it as the operand (postfix + unit-suffix apply).
                    // START-NEXT-ELEMENT (after a `,`): a `.. a` element descends as a spread (wrap on
                    // deliver, `finish_tuple`'s rest_marker twin); else an ordinary element with its own-line
                    // leading comments. Diverges via `continue`.
                    macro_rules! next_tuple_elem {
                        ($items:expr) => {{
                            if self.at(Kind::DotDot) {
                                let dd = self.cur_span();
                                self.bump(); // `..`
                                let rest_head = self.name("..", dd);
                                pending.push(Cont::Paren {
                                    start,
                                    min_prec: parent_min_prec,
                                    spine: parent_spine,
                                    entered: parent_entered,
                                    prefix_is_number,
                                    arm_bar,
                                    pending_leading: Vec::new(),
                                    items: $items,
                                    spread: true,
                                    rest_head,
                                });
                            } else {
                                let elem_leading = self.take_comments_here();
                                pending.push(Cont::Paren {
                                    start,
                                    min_prec: parent_min_prec,
                                    spine: parent_spine,
                                    entered: parent_entered,
                                    prefix_is_number,
                                    arm_bar,
                                    pending_leading: elem_leading,
                                    items: $items,
                                    spread: false,
                                    rest_head: StructId(0),
                                });
                            }
                            cur_min_prec = crate::token::PREC_SEQ + 1;
                            reading = true;
                            continue; // read the next tuple element as a fresh level
                        }};
                    }
                    if spread {
                        // `left` is a `.. a` spread operand — wrap `(.. operand)` (head pre-created at
                        // `rest_head`, whose span is the `..`'s) and push it as a tuple element (`items`
                        // already holds `[ "tuple"-head, … ]`), then close or read the next element.
                        let dd_span = self.spans.get(rest_head).unwrap_or_else(|| Span::new(0, 0));
                        let span = dd_span.merge(self.prev_span());
                        items.push(self.list(vec![rest_head, left], span));
                        if self.sep_continue(Kind::RParen) {
                            next_tuple_elem!(items);
                        }
                        self.drain_closer_comment_onto_last(&mut items, 1);
                        self.expect(Kind::RParen, "`)`");
                        let span = start.merge(self.prev_span());
                        self.arm_bar_terminates = arm_bar;
                        left = self.list(items, span);
                    } else if items.is_empty() {
                        // `left` is the delivered FIRST sub-expr — decide grouping `(e)` vs tuple `(a, …)`.
                        let first = self.wrap_comments(pending_leading, left);
                        if self.at(Kind::Comma) {
                            let head = self.ctor_head("tuple", start);
                            let mut items = vec![head, first];
                            if self.sep_continue(Kind::RParen) {
                                next_tuple_elem!(items);
                            }
                            // `(a,)` — trailing comma, no further element.
                            self.drain_closer_comment_onto_last(&mut items, 1);
                            self.expect(Kind::RParen, "`)`");
                            let span = start.merge(self.prev_span());
                            self.arm_bar_terminates = arm_bar;
                            left = self.list(items, span);
                        } else {
                            // Grouping: transparent — `first` IS the operand.
                            self.expect(Kind::RParen, "`)`");
                            self.arm_bar_terminates = arm_bar;
                            left = first;
                        }
                    } else {
                        // `left` is the delivered subsequent ORDINARY tuple element.
                        let mut elem = self.wrap_comments(pending_leading, left);
                        if self.at(Kind::RParen) {
                            let trailing = self.take_trailing_comment_here();
                            elem = self.wrap_comment_after(trailing, elem);
                        }
                        items.push(elem);
                        if self.sep_continue(Kind::RParen) {
                            next_tuple_elem!(items);
                        }
                        self.drain_closer_comment_onto_last(&mut items, 1);
                        self.expect(Kind::RParen, "`)`");
                        let span = start.merge(self.prev_span());
                        self.arm_bar_terminates = arm_bar;
                        left = self.list(items, span);
                    }
                    cur_start = start;
                    cur_min_prec = parent_min_prec;
                    spine = parent_spine;
                    entered = parent_entered;
                    pf_pending = true; // postfix funnel applies the `.member`/`(args)` chain + unit suffix
                    pf_num = prefix_is_number;
                    pf_spine = 0;
                    reading = false; // resume the parent level's infix loop with the paren operand
                    continue;
                }
                Some(Cont::List {
                    start,
                    min_prec: parent_min_prec,
                    spine: parent_spine,
                    entered: parent_entered,
                    prefix_is_number,
                    arm_bar,
                    closer,
                    allow_rest,
                    allow_comments,
                    drain_closer,
                    mut items,
                    pending_leading,
                    is_rest,
                    dd_span,
                    rest_head,
                    before,
                }) => {
                    // Push the delivered element: a `.. rest` spread `(.. binder)` (head pre-created), or
                    // an ordinary element with own-line leading + LAST-element same-line trailing comments
                    // (comment slots only when the family allows them — bare elements otherwise).
                    if is_rest {
                        let span = dd_span.merge(self.prev_span());
                        items.push(self.list(vec![rest_head, left], span));
                    } else if allow_comments {
                        let mut elem = self.wrap_comments(pending_leading, left);
                        if self.at(closer) {
                            let trailing = self.take_trailing_comment_here();
                            elem = self.wrap_comment_after(trailing, elem);
                        }
                        items.push(elem);
                    } else {
                        items.push(left);
                    }
                    if !self.sep_continue(closer) {
                        if drain_closer {
                            self.drain_closer_comment_onto_last(&mut items, 1);
                        }
                        self.expect(closer, "comma-list closer");
                        let span = start.merge(self.prev_span());
                        self.arm_bar_terminates = arm_bar;
                        left = self.list(items, span);
                        cur_start = start;
                        cur_min_prec = parent_min_prec;
                        spine = parent_spine;
                        entered = parent_entered;
                        pf_pending = true; // postfix funnel applies the `.member`/`(args)` chain + unit suffix
                        pf_num = prefix_is_number;
                        pf_spine = 0;
                        reading = false; // resume the parent level's infix loop with the list operand
                        continue;
                    }
                    // Missing-`,` progress guard (mirrors `list_literal`), then start the next element.
                    if self.pos == before {
                        self.bump();
                    }
                    let before = self.pos;
                    let (is_rest, dd_span, rest_head) = if allow_rest && self.at(Kind::DotDot) {
                        let dd = self.cur_span();
                        self.bump(); // `..`
                        (true, dd, self.name("..", dd))
                    } else {
                        (false, start, StructId(0))
                    };
                    let pending_leading = if allow_comments && !is_rest {
                        self.take_comments_here()
                    } else {
                        Vec::new()
                    };
                    pending.push(Cont::List {
                        start,
                        min_prec: parent_min_prec,
                        spine: parent_spine,
                        entered: parent_entered,
                        prefix_is_number,
                        arm_bar,
                        closer,
                        allow_rest,
                        allow_comments,
                        drain_closer,
                        items,
                        pending_leading,
                        is_rest,
                        dd_span,
                        rest_head,
                        before,
                    });
                    cur_min_prec = crate::token::PREC_SEQ + 1;
                    reading = true;
                    continue; // read the next element as a fresh level
                }
                Some(Cont::Fields {
                    start,
                    min_prec: parent_min_prec,
                    spine: parent_spine,
                    entered: parent_entered,
                    prefix_is_number,
                    arm_bar,
                    is_map,
                    mut items,
                    phase,
                    before,
                }) => {
                    // Reassemble the delivered sub-expr per phase. `MapKey` is mid-entry: it does NOT append
                    // a field — it expects `=` and descends the value. The others append a completed field,
                    // then fall through to the shared separator + next-field advance below. The `=` FieldPair
                    // atom is created HERE (after the value), matching the recursive struct-id order.
                    match phase {
                        FieldPhase::RestOperand { dd_span, rest_head } => {
                            let span = dd_span.merge(self.prev_span());
                            items.push(self.list(vec![rest_head, left], span));
                        }
                        FieldPhase::MapKey { leading, e_start } => {
                            self.expect(Kind::Eq, "`=`");
                            pending.push(Cont::Fields {
                                start,
                                min_prec: parent_min_prec,
                                spine: parent_spine,
                                entered: parent_entered,
                                prefix_is_number,
                                arm_bar,
                                is_map,
                                items,
                                phase: FieldPhase::MapValue {
                                    leading,
                                    e_start,
                                    key: left,
                                },
                                before,
                            });
                            cur_min_prec = crate::token::PREC_SEQ + 1;
                            reading = true;
                            continue; // read the map value as a fresh level
                        }
                        FieldPhase::MapValue {
                            leading,
                            e_start,
                            key,
                        } => {
                            let e_span = e_start.merge(self.prev_span());
                            let eq = self.atom(Leaf::FieldPair, e_start);
                            let entry = self.list(vec![eq, key, left], e_span);
                            let entry = self.wrap_comments(leading, entry);
                            if self.at(Kind::RBrace) {
                                let trailing = self.take_trailing_comment_here();
                                items.push(self.wrap_comment_after(trailing, entry));
                            } else {
                                items.push(entry);
                            }
                        }
                        FieldPhase::RecordValue {
                            leading,
                            f_start,
                            name,
                        } => {
                            let f_span = f_start.merge(self.prev_span());
                            let eq = self.atom(Leaf::FieldPair, f_start);
                            let field = self.list(vec![eq, name, left], f_span);
                            let field = self.wrap_comments(leading, field);
                            if self.at(Kind::RBrace) {
                                let trailing = self.take_trailing_comment_here();
                                items.push(self.wrap_comment_after(trailing, field));
                            } else {
                                items.push(field);
                            }
                        }
                    }
                    // A field was appended. Advance the separator; on close assemble the operand, else run
                    // the missing-`,` progress guard + start the next field(s) via `advance_fields`.
                    let closed = if !self.sep_continue(Kind::RBrace) {
                        self.drain_closer_comment_onto_last(&mut items, 1);
                        self.expect(Kind::RBrace, "`}`");
                        true
                    } else {
                        if self.pos == before {
                            self.bump(); // no field token consumed — avoid a missing-`,` spin
                        }
                        match self.advance_fields(&mut items, is_map, Kind::RBrace) {
                            None => true,
                            Some((phase, before)) => {
                                pending.push(Cont::Fields {
                                    start,
                                    min_prec: parent_min_prec,
                                    spine: parent_spine,
                                    entered: parent_entered,
                                    prefix_is_number,
                                    arm_bar,
                                    is_map,
                                    items,
                                    phase,
                                    before,
                                });
                                cur_min_prec = crate::token::PREC_SEQ + 1;
                                reading = true;
                                continue; // read the next field's sub-expr as a fresh level
                            }
                        }
                    };
                    debug_assert!(closed);
                    let span = start.merge(self.prev_span());
                    self.arm_bar_terminates = arm_bar;
                    left = self.list(items, span);
                    cur_start = start;
                    cur_min_prec = parent_min_prec;
                    spine = parent_spine;
                    entered = parent_entered;
                    pf_pending = true; // postfix funnel applies the `.member`/`(args)` chain + unit suffix
                    pf_num = prefix_is_number;
                    pf_spine = 0;
                    reading = false; // resume the parent level's infix loop with the record/map operand
                    continue;
                }
                Some(Cont::If {
                    start,
                    min_prec: parent_min_prec,
                    spine: parent_spine,
                    entered: parent_entered,
                    prefix_is_number,
                    head,
                    phase,
                }) => {
                    match phase {
                        IfPhase::Cond { c_lead } => {
                            // `left` is the condition. Wrap its leading comments; capture own-line comments
                            // before `then` (+ same after) so they print above the then-branch; descend it.
                            let c = self.wrap_comments(c_lead, left);
                            let mut t_lead = self.take_comments_here();
                            self.expect_keyword(Keyword::Then, "`then`");
                            t_lead.extend(self.take_comments_here());
                            pending.push(Cont::If {
                                start,
                                min_prec: parent_min_prec,
                                spine: parent_spine,
                                entered: parent_entered,
                                prefix_is_number,
                                head,
                                phase: IfPhase::Then { c, t_lead },
                            });
                            cur_min_prec = crate::token::PREC_SEQ + 1;
                            reading = true;
                            continue; // read the then-branch as a fresh level
                        }
                        IfPhase::Then { c, t_lead } => {
                            // `left` is the then-branch. Wrap leading + a same-line trailing `//` on it,
                            // capture own-line comments around `else`, descend the else-branch.
                            let t = self.wrap_comments(t_lead, left);
                            let t_trail = self.take_trailing_comment_here();
                            let t = self.wrap_comment_after(t_trail, t);
                            let mut e_lead = self.take_comments_here();
                            self.expect_keyword(Keyword::Else, "`else`");
                            e_lead.extend(self.take_comments_here());
                            pending.push(Cont::If {
                                start,
                                min_prec: parent_min_prec,
                                spine: parent_spine,
                                entered: parent_entered,
                                prefix_is_number,
                                head,
                                phase: IfPhase::Else { c, t, e_lead },
                            });
                            cur_min_prec = crate::token::PREC_SEQ + 1;
                            reading = true;
                            continue; // read the else-branch as a fresh level
                        }
                        IfPhase::Else { c, t, e_lead } => {
                            // `left` is the else-branch — assemble `(if c t e)` as the parent's operand.
                            let e = self.wrap_comments(e_lead, left);
                            let e_trail = self.take_trailing_comment_here();
                            let e = self.wrap_comment_after(e_trail, e);
                            let span = start.merge(self.prev_span());
                            left = self.list(vec![head, c, t, e], span);
                            cur_start = start;
                            cur_min_prec = parent_min_prec;
                            spine = parent_spine;
                            entered = parent_entered;
                            pf_pending = true; // postfix funnel: `.member`/`(args)` chain + unit suffix
                            pf_num = prefix_is_number;
                            pf_spine = 0;
                            reading = false; // resume the parent level's infix loop with the if operand
                            continue;
                        }
                    }
                }
                Some(Cont::Let {
                    start,
                    min_prec: parent_min_prec,
                    spine: parent_spine,
                    entered: parent_entered,
                    prefix_is_number,
                    head,
                    phase,
                }) => {
                    match phase {
                        LetPhase::BindingValue {
                            mut bindings,
                            n,
                            leading,
                            b_start,
                            e_lead,
                        } => {
                            // `left` is this binding's value — build `(n value)`, wrap comments, append.
                            let e = self.wrap_comments(e_lead, left);
                            let b_span = b_start.merge(self.prev_span());
                            let binding = self.list(vec![n, e], b_span);
                            bindings.push(self.wrap_comments(leading, binding));
                            if self.at(Kind::Comma) {
                                self.bump(); // `,` — another binding follows; read its inline preamble.
                                let leading = self.take_comments_here();
                                let b_start = self.cur_span();
                                let n = self.read_let_binder(b_start);
                                self.expect(Kind::Eq, "`=`");
                                let e_lead = self.take_comments_here();
                                pending.push(Cont::Let {
                                    start,
                                    min_prec: parent_min_prec,
                                    spine: parent_spine,
                                    entered: parent_entered,
                                    prefix_is_number,
                                    head,
                                    phase: LetPhase::BindingValue {
                                        bindings,
                                        n,
                                        leading,
                                        b_start,
                                        e_lead,
                                    },
                                });
                                cur_min_prec = crate::token::PREC_SEQ + 1;
                                reading = true;
                                continue; // descend the next binding's value
                            }
                            // No more bindings — assemble the `binds` list, consume `in`, capture the body's
                            // same-line-trailing + own-line-leading comments, then descend the body.
                            let binds_span = start.merge(self.prev_span());
                            let binds = self.list(bindings, binds_span);
                            self.expect_keyword(Keyword::In, "`in`");
                            let in_trail = self.take_trailing_comment_here();
                            let binds = self.wrap_comment_after(in_trail, binds);
                            let body_lead = self.take_comments_here();
                            pending.push(Cont::Let {
                                start,
                                min_prec: parent_min_prec,
                                spine: parent_spine,
                                entered: parent_entered,
                                prefix_is_number,
                                head,
                                phase: LetPhase::Body { binds, body_lead },
                            });
                            cur_min_prec = 0; // the body is `expr(0)` (a sequence position)
                            reading = true;
                            continue; // descend the body
                        }
                        LetPhase::Body { binds, body_lead } => {
                            // `left` is the body — assemble `(let binds body)` as the parent's operand.
                            let body = self.wrap_comments(body_lead, left);
                            let span = start.merge(self.prev_span());
                            left = self.list(vec![head, binds, body], span);
                            cur_start = start;
                            cur_min_prec = parent_min_prec;
                            spine = parent_spine;
                            entered = parent_entered;
                            pf_pending = true; // postfix funnel: `.member`/`(args)` chain + unit suffix
                            pf_num = prefix_is_number;
                            pf_spine = 0;
                            reading = false; // resume the parent level's infix loop with the let operand
                            continue;
                        }
                    }
                }
                Some(Cont::Match {
                    start,
                    min_prec: parent_min_prec,
                    spine: parent_spine,
                    entered: parent_entered,
                    prefix_is_number,
                    mut items,
                    phase,
                }) => {
                    // START-ARM macro-free inline (used after the scrutinee + after each completed arm on a
                    // `|`): read the pattern + optional guard head (inline), then descend the guard OR the
                    // body. `$arm_leading` is the arm's own-line leading comment run. Both sub-arms diverge
                    // via `continue`, so no path falls through.
                    macro_rules! start_arm {
                        ($arm_leading:expr) => {{
                            let arm_leading = $arm_leading;
                            let (arm_start, pat, guard) = self.match_arm_pat();
                            match guard {
                                Some((guard_head, g_start)) => {
                                    pending.push(Cont::Match {
                                        start,
                                        min_prec: parent_min_prec,
                                        spine: parent_spine,
                                        entered: parent_entered,
                                        prefix_is_number,
                                        items,
                                        phase: MatchPhase::ArmGuard {
                                            arm_start,
                                            arm_leading,
                                            pat,
                                            guard_head,
                                            g_start,
                                        },
                                    });
                                    cur_min_prec = crate::token::PREC_SEQ + 1;
                                    reading = true;
                                    continue; // descend the guard expr
                                }
                                None => {
                                    let (body_lead, saved_arm_bar) = self.match_arm_body_preamble();
                                    pending.push(Cont::Match {
                                        start,
                                        min_prec: parent_min_prec,
                                        spine: parent_spine,
                                        entered: parent_entered,
                                        prefix_is_number,
                                        items,
                                        phase: MatchPhase::ArmBody {
                                            arm_start,
                                            arm_leading,
                                            pat,
                                            body_lead,
                                            saved_arm_bar,
                                        },
                                    });
                                    cur_min_prec = 0; // the body is `expr(0)` (a sequence position)
                                    reading = true;
                                    continue; // descend the body expr
                                }
                            }
                        }};
                    }
                    match phase {
                        MatchPhase::Scrut => {
                            // `left` is the scrutinee. Append it, consume `with`, drain the first arm's
                            // own-line leading comments + optional leading `|`, then start the first arm.
                            items.push(left);
                            self.expect_keyword(Keyword::With, "`with`");
                            let arm_leading = self.take_comments_here();
                            if self.at(Kind::Pipe) {
                                self.bump(); // optional leading `|`
                            }
                            start_arm!(arm_leading);
                        }
                        MatchPhase::ArmGuard {
                            arm_start,
                            arm_leading,
                            pat,
                            guard_head,
                            g_start,
                        } => {
                            // `left` is the guard expr — fold `(guard pat g)`, then descend the body.
                            let g_span = g_start.merge(self.prev_span());
                            let pat = self.list(vec![guard_head, pat, left], g_span);
                            let (body_lead, saved_arm_bar) = self.match_arm_body_preamble();
                            pending.push(Cont::Match {
                                start,
                                min_prec: parent_min_prec,
                                spine: parent_spine,
                                entered: parent_entered,
                                prefix_is_number,
                                items,
                                phase: MatchPhase::ArmBody {
                                    arm_start,
                                    arm_leading,
                                    pat,
                                    body_lead,
                                    saved_arm_bar,
                                },
                            });
                            cur_min_prec = 0;
                            reading = true;
                            continue; // descend the body expr
                        }
                        MatchPhase::ArmBody {
                            arm_start,
                            arm_leading,
                            pat,
                            body_lead,
                            saved_arm_bar,
                        } => {
                            // `left` is the arm body. Restore arm_bar, assemble `(pat body)`, wrap the arm's
                            // own-line leading + same-line trailing comments, append.
                            self.arm_bar_terminates = saved_arm_bar;
                            let body = self.wrap_comments(body_lead, left);
                            let arm_span = arm_start.merge(self.prev_span());
                            let arm = self.list(vec![pat, body], arm_span);
                            let arm = self.wrap_comments(arm_leading, arm);
                            let trailing = self.take_trailing_comment_here();
                            items.push(self.wrap_comment_after(trailing, arm));
                            let mut pending_leading = self.take_comments_here();
                            if self.at(Kind::Pipe) {
                                self.bump(); // `|` before the next arm
                                start_arm!(pending_leading);
                            }
                            // No more arms. Own-line comment(s) we drained lead whatever FOLLOWS the match,
                            // not a (nonexistent) next arm — restore them to the current token's leading slot
                            // so the enclosing parser picks them up (the seq-277 reader-attachment gap).
                            if !pending_leading.is_empty() && self.pos < self.leading.len() {
                                let mut restored = std::mem::take(&mut pending_leading);
                                restored.append(&mut self.leading[self.pos]);
                                self.leading[self.pos] = restored;
                            }
                            let span = start.merge(self.prev_span());
                            left = self.list(items, span);
                            cur_start = start;
                            cur_min_prec = parent_min_prec;
                            spine = parent_spine;
                            entered = parent_entered;
                            pf_pending = true; // postfix funnel: `.member`/`(args)` chain + unit suffix
                            pf_num = prefix_is_number;
                            pf_spine = 0;
                            reading = false; // resume the parent level's infix loop with the match operand
                            continue;
                        }
                    }
                }
                Some(Cont::Fn {
                    start,
                    min_prec: parent_min_prec,
                    spine: parent_spine,
                    entered: parent_entered,
                    prefix_is_number,
                    head,
                    param_list,
                    ret_ty,
                }) => {
                    // `left` is the body — ascribe it with the return type, assemble `(fn (params) body)`.
                    let body = self.ascribe(left, ret_ty);
                    let span = start.merge(self.prev_span());
                    left = self.list(vec![head, param_list, body], span);
                    cur_start = start;
                    cur_min_prec = parent_min_prec;
                    spine = parent_spine;
                    entered = parent_entered;
                    pf_pending = true; // postfix funnel: `.member`/`(args)` chain + unit suffix
                    pf_num = prefix_is_number;
                    pf_spine = 0;
                    reading = false; // resume the parent level's infix loop with the fn operand
                    continue;
                }
                Some(Cont::Host {
                    start,
                    min_prec: parent_min_prec,
                    spine: parent_spine,
                    entered: parent_entered,
                    prefix_is_number,
                    head,
                    effects_list,
                }) => {
                    // `left` is the body — assemble `(host (E …) body)`.
                    let span = start.merge(self.prev_span());
                    left = self.list(vec![head, effects_list, left], span);
                    cur_start = start;
                    cur_min_prec = parent_min_prec;
                    spine = parent_spine;
                    entered = parent_entered;
                    pf_pending = true; // postfix funnel: `.member`/`(args)` chain + unit suffix
                    pf_num = prefix_is_number;
                    pf_spine = 0;
                    reading = false; // resume the parent level's infix loop with the host operand
                    continue;
                }
                Some(Cont::Handle {
                    start,
                    min_prec: parent_min_prec,
                    spine: parent_spine,
                    entered: parent_entered,
                    prefix_is_number,
                    head,
                    effect,
                    phase,
                }) => {
                    // START-ARM inline (after the seed + after each arm on `|`): read the arm header
                    // (`op(binder…, state) =>`, sets arm_bar=true) then descend the body. Diverges via
                    // `continue`.
                    macro_rules! start_handle_arm {
                        ($seed:expr, $arms_start:expr, $arms:expr) => {{
                            let (arm_start, op, params, state, saved_arm_bar) =
                                self.handle_arm_header();
                            pending.push(Cont::Handle {
                                start,
                                min_prec: parent_min_prec,
                                spine: parent_spine,
                                entered: parent_entered,
                                prefix_is_number,
                                head,
                                effect,
                                phase: HandlePhase::ArmBody {
                                    seed: $seed,
                                    arms_start: $arms_start,
                                    arms: $arms,
                                    arm_start,
                                    op,
                                    params,
                                    state,
                                    saved_arm_bar,
                                },
                            });
                            cur_min_prec = 0; // the arm body is `expr(0)`
                            reading = true;
                            continue; // descend the arm body
                        }};
                    }
                    match phase {
                        HandlePhase::Seed => {
                            // `left` is the seed expr — consume `)`, then `with` + start the first arm.
                            let seed = left;
                            self.expect(Kind::RParen, "`)`");
                            let arms_start = self.handle_after_seed();
                            start_handle_arm!(seed, arms_start, Vec::new());
                        }
                        HandlePhase::ArmBody {
                            seed,
                            arms_start,
                            mut arms,
                            arm_start,
                            op,
                            params,
                            state,
                            saved_arm_bar,
                        } => {
                            // `left` is the arm body — restore arm_bar, assemble `(op params state body)`.
                            self.arm_bar_terminates = saved_arm_bar;
                            let arm_span = arm_start.merge(self.prev_span());
                            arms.push(self.list(vec![op, params, state, left], arm_span));
                            if self.at(Kind::Pipe) {
                                self.bump(); // `|` before the next arm
                                start_handle_arm!(seed, arms_start, arms);
                            }
                            // No more arms — assemble the arm list, consume `in`, descend the final body.
                            let arms_span = arms_start.merge(self.prev_span());
                            let arms_list = self.list(arms, arms_span);
                            self.expect_keyword(Keyword::In, "`in`");
                            pending.push(Cont::Handle {
                                start,
                                min_prec: parent_min_prec,
                                spine: parent_spine,
                                entered: parent_entered,
                                prefix_is_number,
                                head,
                                effect,
                                phase: HandlePhase::Body { seed, arms_list },
                            });
                            cur_min_prec = 0; // the body is `expr(0)`
                            reading = true;
                            continue; // descend the final body
                        }
                        HandlePhase::Body { seed, arms_list } => {
                            // `left` is the final body — assemble `(handle effect seed (arm…) body)`.
                            let span = start.merge(self.prev_span());
                            left = self.list(vec![head, effect, seed, arms_list, left], span);
                            cur_start = start;
                            cur_min_prec = parent_min_prec;
                            spine = parent_spine;
                            entered = parent_entered;
                            pf_pending = true; // postfix funnel: `.member`/`(args)` chain + unit suffix
                            pf_num = prefix_is_number;
                            pf_spine = 0;
                            reading = false; // resume the parent level's infix loop with the handle operand
                            continue;
                        }
                    }
                }
                Some(Cont::Call {
                    callee,
                    call_start,
                    mut args,
                    leading,
                    saved_arm_bar,
                    pf_spine: saved_pf_spine,
                    pf_num: saved_pf_num,
                    lvl_min_prec,
                    lvl_spine,
                    lvl_entered,
                }) => {
                    // `left` is the delivered argument — wrap its leading comment + a same-line trailing
                    // comment on the LAST arg (gated on `at(RParen)`, the PR#758 rule), matching `arg_exprs`.
                    let arg = self.wrap_comments(leading, left);
                    if self.at(Kind::RParen) {
                        let trailing = self.take_trailing_comment_here();
                        args.push(self.wrap_comment_after(trailing, arg));
                    } else {
                        args.push(arg);
                    }
                    if self.sep_continue(Kind::RParen) {
                        // Another argument follows — capture its leading comments + descend it.
                        let leading = self.take_comments_here();
                        pending.push(Cont::Call {
                            callee,
                            call_start,
                            args,
                            leading,
                            saved_arm_bar,
                            pf_spine: saved_pf_spine,
                            pf_num: saved_pf_num,
                            lvl_min_prec,
                            lvl_spine,
                            lvl_entered,
                        });
                        cur_min_prec = crate::token::PREC_SEQ + 1;
                        reading = true;
                        continue; // read the next argument as a fresh level
                    }
                    // Arguments done — close `)`, restore arm_bar, build `(callee arg…)`.
                    self.expect(Kind::RParen, "`)`");
                    self.arm_bar_terminates = saved_arm_bar;
                    let span = call_start.merge(self.prev_span());
                    let mut items = Vec::with_capacity(args.len() + 1);
                    items.push(callee);
                    items.extend(args);
                    left = self.list(items, span);
                    // Restore the owning expr level; the built call is its (in-progress) operand.
                    cur_start = call_start;
                    cur_min_prec = lvl_min_prec;
                    spine = lvl_spine;
                    entered = lvl_entered;
                    reading = false;
                    // Guard the call layer just folded (mirrors `postfix`); on trip, postfix stops.
                    let new_pf_spine = saved_pf_spine + 1;
                    if !self.depth_exceeded
                        && self.depth + new_pf_spine >= crate::sexpr::MAX_NESTING_DEPTH
                    {
                        self.error("expression nests too deeply to parse");
                        self.depth_exceeded = true;
                        if !saved_pf_num && cur_min_prec != TIGHT_PREC {
                            left = self.maybe_unit_suffix(left, call_start);
                        }
                        continue; // reading=false -> next iteration runs the infix loop
                    }
                    // RE-ENTER the postfix funnel on the built call node (fold further `.member`/`(args)`).
                    pf_pending = true;
                    pf_num = saved_pf_num;
                    pf_spine = new_pf_spine;
                    continue;
                }
                Some(Cont::Neg {
                    neg_start,
                    lvl_min_prec,
                    lvl_spine,
                    lvl_entered,
                    lvl_num,
                }) => {
                    // `left` is the tight operand — build `(- operand)` (the `-` head created AFTER the
                    // operand, matching the recursive minus arm's struct-id order), then apply the OUTER
                    // postfix + unit suffix via the funnel and resume the owning level's infix loop.
                    let full = neg_start.merge(self.prev_span());
                    let head = self.name("-", neg_start);
                    left = self.list(vec![head, left], full);
                    cur_start = neg_start;
                    cur_min_prec = lvl_min_prec;
                    spine = lvl_spine;
                    entered = lvl_entered;
                    pf_pending = true; // OUTER postfix/unit suffix apply to `(- operand)` (as `expr` does)
                    pf_num = lvl_num;
                    pf_spine = 0;
                    reading = false;
                    continue;
                }
                Some(Cont::Unquote {
                    head,
                    unq_start,
                    braced,
                    lvl_min_prec,
                    lvl_spine,
                    lvl_entered,
                    lvl_num,
                }) => {
                    // `left` is the inner (braced: full expr; bare: tight operand). Close `}` for the braced
                    // form, then build `(head inner)` and apply the OUTER postfix/unit suffix via the funnel.
                    if braced {
                        self.expect(Kind::RBrace, "`}`");
                    }
                    let span = unq_start.merge(self.prev_span());
                    left = self.list(vec![head, left], span);
                    cur_start = unq_start;
                    cur_min_prec = lvl_min_prec;
                    spine = lvl_spine;
                    entered = lvl_entered;
                    pf_pending = true;
                    pf_num = lvl_num;
                    pf_spine = 0;
                    reading = false;
                    continue;
                }
                Some(Cont::AtArgs {
                    head,
                    bare,
                    name_span,
                    mut args,
                    leading,
                    saved_arm_bar,
                    at_pos,
                    at_span,
                    lvl_min_prec,
                    lvl_spine,
                    lvl_entered,
                    lvl_num,
                }) => {
                    // `left` is a delivered glued-call argument — wrap its leading comment + a same-line
                    // trailing comment on the LAST arg (gated on `at(RParen)`, the PR#758 rule), matching
                    // `arg_exprs` (and the postfix `Cont::Call`).
                    let arg = self.wrap_comments(leading, left);
                    if self.at(Kind::RParen) {
                        let trailing = self.take_trailing_comment_here();
                        args.push(self.wrap_comment_after(trailing, arg));
                    } else {
                        args.push(arg);
                    }
                    if self.sep_continue(Kind::RParen) {
                        // Another argument follows — capture its leading comments + descend it.
                        let leading = self.take_comments_here();
                        pending.push(Cont::AtArgs {
                            head,
                            bare,
                            name_span,
                            args,
                            leading,
                            saved_arm_bar,
                            at_pos,
                            at_span,
                            lvl_min_prec,
                            lvl_spine,
                            lvl_entered,
                            lvl_num,
                        });
                        cur_min_prec = crate::token::PREC_SEQ + 1;
                        reading = true;
                        continue; // read the next glued-call argument as a fresh level
                    }
                    // Arguments done — close `)`, restore arm_bar, build the annotation name `(tag arg…)`.
                    self.expect(Kind::RParen, "`)`");
                    self.arm_bar_terminates = saved_arm_bar;
                    let call_span = name_span.merge(self.prev_span());
                    let mut items = Vec::with_capacity(args.len() + 1);
                    items.push(bare);
                    items.extend(args);
                    let name = self.list(items, call_span);
                    // Now the `@` form descent — identical to the inline `@` setup, but with the enclosing
                    // level state replayed from the saved `lvl_*`: carry any docs from the `@` slot to the
                    // form slot, then either trip the `@`'s own guard inline (`(@ name <error>)`) or push
                    // Cont::At and read the annotated FORM prefix-only.
                    let form_pos = self.pos;
                    self.carry_docs(at_pos, form_pos);
                    if self.depth >= crate::sexpr::MAX_NESTING_DEPTH {
                        if !self.depth_exceeded {
                            self.error("expression nests too deeply to parse");
                            self.depth_exceeded = true;
                        }
                        let err = self.error_node(at_span);
                        left = self.list(vec![head, name, err], at_span.merge(self.prev_span()));
                        cur_start = at_span;
                        cur_min_prec = lvl_min_prec;
                        spine = lvl_spine;
                        entered = lvl_entered;
                        pf_pending = true;
                        pf_num = lvl_num;
                        pf_spine = 0;
                        reading = false;
                        continue;
                    }
                    pending.push(Cont::At {
                        head,
                        name,
                        at_pos,
                        form_pos,
                        at_span,
                        lvl_min_prec,
                        lvl_spine,
                        lvl_entered,
                        lvl_num,
                    });
                    cur_start = at_span;
                    cur_min_prec = PREFIX_ONLY_PREC;
                    reading = true;
                    continue; // descend: read the annotated form prefix-only
                }
                Some(Cont::At {
                    head,
                    name,
                    at_pos,
                    form_pos,
                    at_span,
                    lvl_min_prec,
                    lvl_spine,
                    lvl_entered,
                    lvl_num,
                }) => {
                    // `left` is the prefix-only annotated form. Carry any docs that stayed at the form slot
                    // back to the `@` slot (so an un-documentable form downgrades them, not drops), then
                    // build `(@ name form)` and apply the OUTER postfix/unit suffix.
                    self.carry_docs(form_pos, at_pos);
                    let full = at_span.merge(self.prev_span());
                    left = self.list(vec![head, name, left], full);
                    cur_start = at_span;
                    cur_min_prec = lvl_min_prec;
                    spine = lvl_spine;
                    entered = lvl_entered;
                    pf_pending = true;
                    pf_num = lvl_num;
                    pf_spine = 0;
                    reading = false;
                    continue;
                }
                Some(Cont::Pragma {
                    head,
                    key,
                    at_span,
                    lvl_min_prec,
                    lvl_spine,
                    lvl_entered,
                    lvl_num,
                }) => {
                    // `left` is the tight pragma arg — build `(pragma key arg)` + OUTER postfix/unit suffix.
                    let full = at_span.merge(self.prev_span());
                    left = self.list(vec![head, key, left], full);
                    cur_start = at_span;
                    cur_min_prec = lvl_min_prec;
                    spine = lvl_spine;
                    entered = lvl_entered;
                    pf_pending = true;
                    pf_num = lvl_num;
                    pf_spine = 0;
                    reading = false;
                    continue;
                }
                Some(Cont::Def {
                    def_head,
                    target,
                    ret_ty,
                    docs,
                    leading,
                    start,
                    lvl_min_prec,
                    lvl_spine,
                    lvl_entered,
                    lvl_num,
                }) => {
                    // `left` is the value/body — wrap its interior leading trivia, ascribe the return type
                    // (a no-op for a value def), then assemble `(def target doc… body)`. The `(doc …)` nodes
                    // are built HERE (after the body) to match the recursive struct-id order.
                    let body = self.wrap_comments(leading, left);
                    let body = self.ascribe(body, ret_ty);
                    let span = start.merge(self.prev_span());
                    let mut items = vec![def_head, target];
                    items.extend(self.doc_nodes(docs));
                    items.push(body);
                    left = self.list(items, span);
                    cur_start = start;
                    cur_min_prec = lvl_min_prec;
                    spine = lvl_spine;
                    entered = lvl_entered;
                    pf_pending = true; // OUTER postfix/unit suffix (as `expr` applies to `prefix`'s result)
                    pf_num = lvl_num;
                    pf_spine = 0;
                    reading = false;
                    continue;
                }
                Some(Cont::Module {
                    start,
                    mut items,
                    members_start,
                    stmt_start,
                    comments,
                    lvl_min_prec,
                    lvl_spine,
                    lvl_entered,
                    lvl_num,
                }) => {
                    // `left` is the delivered member FORM — finish the statement (leftover-doc/comment
                    // attach) and append it, then start the next member or close the body.
                    let member = self.finish_stmt(left, stmt_start, comments);
                    items.push(member);
                    if self.pos == stmt_start {
                        self.bump(); // forward-progress guard (a stray token that begins no member)
                    }
                    if !self.at(Kind::RBrace) && !self.at_end() {
                        let stmt_start = self.pos;
                        let comments = self.take_comments_here();
                        pending.push(Cont::Module {
                            start,
                            items,
                            members_start,
                            stmt_start,
                            comments,
                            lvl_min_prec,
                            lvl_spine,
                            lvl_entered,
                            lvl_num,
                        });
                        cur_min_prec = 0;
                        reading = true;
                        continue; // descend: read the next member
                    }
                    // No more members — close `}` + assemble `(module …)`.
                    left = self.finish_module_body(items, members_start, start);
                    cur_start = start;
                    cur_min_prec = lvl_min_prec;
                    spine = lvl_spine;
                    entered = lvl_entered;
                    pf_pending = true;
                    pf_num = lvl_num;
                    pf_spine = 0;
                    reading = false; // resume the parent level's infix loop with the module operand
                    continue;
                }
            }
        }
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
            && (matches!(
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
            // The CONTEXTUAL `world` declaration (not a reserved keyword) — recognized by the same
            // unambiguous `world <name> =` shape `prefix` uses, so a `;`-sequence stops before a
            // top-level `world` decl the way it stops before `def`/`effect`.
            || (self.cur_text() == "world"
                && self.nth_kind(1) == Kind::Ident
                && self.nth_kind(2) == Kind::Eq))
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
            // NOTE (seq-204): there is NO ML rational LITERAL — the operator dropped the `r` glyph and
            // unspaced `3/2` is Int64 division, so the lexer never emits a rational token. A native rational
            // VALUE node `(RationalTag <num> <den>)` is built by the compiler (const-fold / `(/ n d)`
            // grounding), never parsed from a scalar literal; the printer still renders such a node `num/den`.
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
                    // A bare numeric literal takes the unit suffix HERE (the literal-only fast path,
                    // preserving the exact suffix guard: `100N feet` is not a quantity). A non-literal
                    // expression gets the SAME sugar generally, applied in `expr` after `postfix` — see
                    // `maybe_unit_suffix`. Applying it here for the literal keeps the numeric path (and its
                    // `Suffixed` exemption) unchanged; the `expr` hook then sees a `(Qty.of …)` node with no
                    // trailing unit ident, so it no-ops and never double-wraps.
                    self.maybe_unit_suffix(num, span)
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
                    Leaf::Bytes(literal::unescape_byte_string_token(self.text(t)).into()),
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
            // A TAGGED TEMPLATE `tag"…{expr}…"` → `(tagged-template <tag> (chunks <str>…) (holes <expr>…))`
            // — a binding-dispatched compile-time macro over literal chunks + `{expr}` holes. The token
            // text is `<tag>"<body>"`; `split_template_body` splits it into the tag, the unescaped
            // literal chunks (`{{`/`}}` → `{`/`}`), and each hole's raw source text. Each hole is
            // RE-PARSED as an ordinary expression (via `read_ml`), so a hole can hold any expression.
            // The head is the reserved name `tagged-template`; invariant chunks.len() == holes.len() + 1.
            Kind::TaggedTemplate => {
                let t = self.bump().unwrap();
                let raw = self.text(t).to_string();
                let head = self.name("tagged-template", span);
                let body = literal::split_template_body(&raw);
                let (tag_name, chunk_strs, hole_srcs) = match body {
                    Some(b) => (b.tag, b.chunks, b.holes),
                    None => (String::new(), vec![String::new()], Vec::new()),
                };
                let tag = self.name(tag_name, span);
                // chunks: each literal piece as a Str leaf.
                let chunks_head = self.name("chunks", span);
                let mut chunks = vec![chunks_head];
                for s in chunk_strs {
                    chunks.push(self.atom(Leaf::Str(s.into()), span));
                }
                let chunks = self.list(chunks, span);
                // holes: each hole's source re-parsed as an expression and grafted in. `read_ml` returns
                // its own arena; `graft` copies the parsed root's subtree into this builder.
                let holes_head = self.name("holes", span);
                let mut holes = vec![holes_head];
                //= spec/capabilities/metaprogramming.md#a-tagged-template-is-a-binding-dispatched-compile-time-macro-over-literal-chunks-and-holes
                //# Each parsed interpolation hole MUST appear in the tagged-template node as one of its holes, so that the tag function receives the hole expressions in source order.
                for src in hole_srcs {
                    holes.push(self.graft_ml_expr(&src, span));
                }
                let holes = self.list(holes, span);
                self.list(vec![head, tag, chunks, holes], span)
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
                let at_pos = self.pos; // slot holding any `///` docs that preceded the annotation
                self.bump(); // `@`
                let head = self.name("@", span);
                let name = if self.at(Kind::Ident) {
                    let name_span = self.cur_span();
                    let t = self.bump().unwrap();
                    let bare = self.name(self.text(t), name_span);
                    // `@tag("slow")` — a call-STYLE annotation argument: an application GLUED to the name
                    // (`tag(…)`) makes the name slot the application `(tag "slow")`, so the tree is
                    // `(@ (tag "slow") form)`. A bare `@test` stays `(@ test form)`. The `(` must be GLUED
                    // (no intervening whitespace/newline) — otherwise a space-separated `@test (g)` would
                    // wrongly eat `(g)` as `test`'s call args instead of leaving it as the annotated form.
                    // Same adjacency discipline the quantity/`.member` sugars use. Take EXACTLY the one
                    // glued call: NOT the general `postfix` loop, which after the glued `("slow")` would
                    // continue and eat the FOLLOWING (space-separated) annotated form as a second call
                    // layer (`@tag("t") (a + 1)` → `(tag "t" (a + 1))`) — the round-trip bug a parenthesized
                    // annotated compound form (`@tag("t") (a + 1)`, printed by the printer) exposed.
                    if self.at(Kind::LParen) && self.prev_span().end == self.cur_span().start {
                        let call_args = self.arg_exprs();
                        let call_span = name_span.merge(self.prev_span());
                        let mut items = Vec::with_capacity(call_args.len() + 1);
                        items.push(bare);
                        items.extend(call_args);
                        self.list(items, call_span)
                    } else {
                        bare
                    }
                } else {
                    self.error("expected an annotation name after `@`");
                    self.error_node(self.cur_span())
                };
                // A `///` doc that preceded the annotation belongs to the item BELOW it (the def), not
                // the `@` sigil. Carry those docs onto the annotated form's slot so the inner
                // def/type/effect/module drains them as `(doc …)` — matching a doc before an
                // unannotated def. Without this they sit at the `@` slot, unseen by the inner def
                // parser, and `stmt` downgrades them to a `(comment …)` (`//`) — the annotated-def
                // doc-loss bug. Composes through stacked `@a @b def …`: the inner `@`'s recursion
                // carries them one more slot to the def.
                let form_pos = self.pos;
                self.carry_docs(at_pos, form_pos);
                // The annotated form parses in PREFIX position (no postfix): a following juxtaposed
                // top-level form that begins with `(` must not be swallowed as a call of the def. A
                // `def`/other keyword dispatches to its full form; a nested `@` recurses HERE via
                // `self.prefix()` — directly, NOT through `expr` — so a deep stack of annotations
                // (`@a @b @c … def`) bypassed `expr`'s depth guard and overflowed the native stack
                // (SIGABRT). Count each annotation layer against the shared depth budget via `guard_prefix`.
                if let Some(err) = self.guard_prefix(span) {
                    return self.list(vec![head, name, err], span.merge(self.prev_span()));
                }
                let form = self.prefix();
                self.depth -= 1;
                // If the form was NOT documentable (no def/type/effect/module drained the carried
                // docs), they still sit at `form_pos` and would be silently DROPPED. Move them back to
                // the `@` slot so `stmt`'s leftover-drain re-wraps them as `(comment …)` — preserving
                // the pre-fix behavior (a downgrade, not a loss) for `@ann (expr)` with no def below.
                self.carry_docs(form_pos, at_pos);
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
                let mut key_text = String::new();
                let key = if self.at(Kind::Ident) {
                    let key_span = self.cur_span();
                    let t = self.bump().unwrap();
                    key_text = self.text(t).to_string();
                    self.name(key_text.as_str(), key_span)
                } else {
                    self.error("expected a pragma key after `@!` (e.g. `@!default-float Float32`)");
                    self.error_node(self.cur_span())
                };
                // `@!param` carries a PARAM PAYLOAD, not a single type arg: a glued `(config…)` of
                // `key: value` kv pairs PLUS a following `name : Type` binder, module-attached (the
                // operator-ruled module-level `@param`). It parses to `(pragma param (param <kv>…)
                // (: name Type))` — the `pragma` head marks it module-attached (matching `@!default-fraction`);
                // the `(param …)` sublist holds the widget/range/default kvs (each a `(: key value)`
                // ascription, the same kv shape the `@param` annotation's `(param (: widget slider) …)`
                // config carries); the `(: name Type)` binder gives the param NAME + its declared TYPE.
                // Without this, the generic single-type-arg path below read `param`'s config as the arg and
                // then let the general unit-suffix postfix eat the trailing `name` as a unit on the pragma
                // node (a garbled `Qty.of` tree). v-metaprogramming's sidecar reads name/type/config from
                // the stable positions of this node. Only `param` takes the payload; every other pragma key
                // keeps the single-type-arg form.
                if key_text == "param" {
                    let payload = self.param_pragma_payload();
                    let full = span.merge(self.prev_span());
                    let mut items = vec![head, key];
                    items.extend(payload);
                    return self.list(items, full);
                }
                // The ARGUMENT is a TYPE expression parsed in prefix+POSTFIX position — so a bare name
                // (`Float32`), a member access (`Foo.Bar`), and a constructor APPLICATION (`Int(8)` ->
                // `(Int 8)`) all parse as the single argument, exactly as a type annotation's type does. The
                // postfix stops at a `.`/`(` glued to the type; a following module member (`def …` on the
                // next line) does not begin with either, so it is never swallowed. Infix operators / `as`
                // are intentionally NOT consumed (a pragma type is a single type, never `A -> B`).
                // DEPTH GUARD: a stacked pragma (`@!k @!k … def`) recurses `prefix` DIRECTLY here (like
                // the `@` annotation arm), bypassing `expr`'s guard → deep run overflowed the stack.
                if let Some(err) = self.guard_prefix(span) {
                    return self.list(vec![head, key, err], span.merge(self.prev_span()));
                }
                let arg_start = self.cur_span();
                let arg_prefix = self.prefix();
                let arg = self.postfix(arg_prefix, arg_start);
                self.depth -= 1;
                let full = span.merge(self.prev_span());
                self.list(vec![head, key, arg], full)
            }
            // FIRST-CLASS EMBEDDED SYNTAX: a reserved grammar tag GLUED to a brace-delimited region —
            // `json{ … }` — switches the parser into that sub-grammar. The raw region (the text between
            // the balanced `{`/`}`) is handed VERBATIM to the sub-grammar's own reader and grafted as an
            // `(embedded <grammar> <subtree>)` node in the shared arena, so every downstream tool (codec,
            // fmt, LSP, refactor) operates on it as ordinary nodes. This is the front-end, parser-level
            // switch (operator-greenlit) — distinct from v-metaprogramming's tagged-template macros,
            // which coexist. A tag is recognized ONLY when immediately followed by `{` (no space — the
            // `{`'s span start must equal the ident's span end), so a bare name `json` is unaffected.
            Kind::Ident
                if embedded_grammar(self.cur_text()).is_some()
                    && self.nth_kind(1) == Kind::LBrace
                    && self.tok().map(|t| t.span.end)
                        == self.tokens.get(self.pos + 1).map(|t| t.span.start) =>
            {
                self.embedded_syntax()
            }
            // CONTEXTUAL `world` declaration: `world Name = …`. `world` is NOT a reserved keyword (a
            // bare `world` stays an ordinary name everywhere else), so recognize the declaration only by
            // the unambiguous `world <name> =` shape — a bare `world` variable is never followed by
            // `<ident> =`. This keeps the common word usable as an identifier while still spelling the
            // inline WIT-world decl (operator's contextual + WIT-familiar surface ruling, 2026-08-11).
            Kind::Ident
                if self.cur_text() == "world"
                    && self.nth_kind(1) == Kind::Ident
                    && self.nth_kind(2) == Kind::Eq =>
            {
                self.world_expr()
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
            // PREFIX UNARY MINUS: `-<expr>` -> `(- <expr>)`, negation. A `Kind::Minus` in prefix
            // position is always the unary operator: a `-` GLUED to a digit lexes as a SIGNED LITERAL
            // (`-1`, `-1.5` — the lexer's `minus` calls `number`), so `Kind::Minus` reaches here only
            // when the `-` is NOT part of a literal — a `-` before a name / `(` / call. The operand is
            // a TIGHT unary (a `prefix` + `postfix` chain, no trailing infix), so negation binds TIGHTER
            // than every binary operator: `-x + 1` groups as `(+ (- x) 1)`, `-f(x)` as `(- (f x))`, and
            // `-x.field` as `(- (. x field))` (the member chain is the operand, not a projection of the
            // negation). A parenthesized operand `-(x + 1)` is one `postfix` atom, so it negates the
            // whole sum. Double negation `- -x` recurses here. Canonical arena is the arity-1
            // subtraction `(- e)` — no new prim / grammar head: `lower` reads a one-operand `Sub` as
            // type-directed negation (`0 - e` at the operand's numeric type). The printer renders it
            // back to `-e`.
            Kind::Minus => {
                self.bump(); // `-`
                // DEPTH GUARD: a run of unary minus (`- - - … x`) recurses `prefix` → `prefix` DIRECTLY,
                // not back through `expr`, so `expr`'s depth guard never fires — a pathologically deep
                // `-----…1` overflowed the native stack (SIGABRT). Each unary layer builds one arena-tree
                // level, so count it against the SAME depth budget via `guard_prefix` (clean diagnostic).
                if let Some(err) = self.guard_prefix(span) {
                    return err;
                }
                let operand_start = self.cur_span();
                let operand_prefix = self.prefix();
                let operand = self.postfix(operand_prefix, operand_start);
                self.depth -= 1;
                let full = span.merge(self.prev_span());
                let head = self.name("-", span);
                self.list(vec![head, operand], full)
            }
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
    /// Apply an adjacent same-line UNIT SUFFIX to `expr`, yielding `(Qty.of <expr> (Unit.of #name))`, or
    /// return `expr` unchanged if no unit name follows. `expr` is ANY expression (a numeric literal from
    /// the prefix arm, or a variable / call / parenthesized expression via the `expr` post-`postfix`
    /// hook) — this is the general unit-application postfix. The unit name must be an adjacent, SAME-LINE,
    /// non-keyword/non-word-op `Ident` (the same guards the literal-only sugar used: the same-line guard
    /// stops the sugar eating the next statement's leading ident across a newline — the `10 a` miscompile
    /// guard). Typing (operand must be a dimensionless number; a Quantity operand → a type error) is
    /// v-quantity/v-inference's job — the parse is uniform regardless of the operand's type.
    fn maybe_unit_suffix(&mut self, expr: StructId, expr_span: Span) -> StructId {
        let num = expr;
        let num_span = expr_span;
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
        // (Unit.of #"name"), then extend into a COMPOUND / RATE unit across glued operators.
        let unit_head = self.member_head("Unit", "of", unit_span);
        let sym = self.atom(Leaf::Sym(name.into()), unit_span);
        let atom = self.list(vec![unit_head, sym], unit_span);
        let unit_expr = self.compound_unit_tail(atom);
        // (Qty.of num <unit>)
        let span = num_span.merge(self.prev_span());
        let qty_head = self.member_head("Qty", "of", span);
        self.list(vec![qty_head, num, unit_expr], span)
    }

    /// Extend a just-read unit atom into a COMPOUND unit across GLUED `/`/`*`/`^` operators. `^` binds
    /// TIGHTER than `/`/`*` (so `m/s^2` = `m/(s^2)` = `(Unit./ m (Unit.^ s 2))`, the physical reading — NOT
    /// `(m/s)^2`); `/`/`*` are left-associative (`a/b/c` = `(a/b)/c`). "Glued" = each operator's span abuts
    /// the token before it AND its right operand abuts the operator (no whitespace) — the syntactic rule
    /// that separates a RATE unit (`GiB/s`) from arithmetic (`GiB / 2`, left to the ordinary infix loop).
    /// A non-glued operator, or a right operand that is not the expected unit-name (`/`/`*`) or integer
    /// (`^`), stops the chain. `left` already includes any `^` on the first atom (applied by `unit_factor`).
    fn compound_unit_tail(&mut self, first: StructId) -> StructId {
        let mut left = self.unit_pow(first); // a trailing `^n` on the first atom binds before `/`/`*`
        // Layers this loop has folded onto `left`. Like the `postfix`/`expr` left-spine guards: this loop
        // is iterative (`self.depth` does not grow), but each iteration deepens the produced arena by one
        // `(/ … …)` level, so an unbounded glued chain (`m/s/s/s…`) would build an arbitrarily deep tree a
        // recursive consumer (printer/`canon`) overflows on — the flat-chain DoS class (PR #383). Bound
        // `self.depth + spine` against the shared limit so a pathological chain is a clean diagnostic.
        let mut spine: u32 = 0;
        loop {
            let op_name = match self.kind() {
                Kind::Slash => "/",
                Kind::Star => "*",
                _ => break,
            };
            let (Some(op_tok), Some(rhs_tok)) =
                (self.tokens.get(self.pos), self.tokens.get(self.pos + 1))
            else {
                break;
            };
            let (op_span, rhs_span) = (op_tok.span, rhs_tok.span);
            // GLUE + a following unit NAME (not a number — `GiB/2` is arithmetic — nor a keyword).
            if self.prev_span().end != op_span.start
                || op_span.end != rhs_span.start
                || rhs_tok.kind != Kind::Ident
            {
                break;
            }
            let name = self.text(*rhs_tok);
            if keyword(name).is_some() || word_op(name).is_some() {
                break;
            }
            let name = name.to_string();
            self.bump(); // operator
            self.bump(); // unit name
            let unit_head = self.member_head("Unit", "of", rhs_span);
            let sym = self.atom(Leaf::Sym(name.into()), rhs_span);
            let rhs_atom = self.list(vec![unit_head, sym], rhs_span);
            let rhs = self.unit_pow(rhs_atom); // `^n` binds to THIS factor before the `/`/`*`
            // BARE `/` / `*` head between two unit operands — `eval::unit_of` composes a bare arithmetic
            // operator over two operands that BOTH reduce to units (v-inference confirmed), and this is the
            // shape the printer ROUND-TRIPS to (it renders both `Unit./` and bare `/` as the infix `a / b`,
            // which re-reads as the bare `/`) — so emitting bare keeps read→print→read stable, whereas a
            // `Unit./` head would print `a / b` then re-read as bare `/` (a spurious round-trip drift).
            let head = self.name(op_name, op_span);
            // Span the WHOLE composite `left <op> rhs` — from the LEFT operand's start, not the operator
            // (else `a/b` would highlight only `/b`, mis-anchoring a diagnostic; matches how the ordinary
            // infix loop spans `start.merge(prev)`). `left`'s span is in the table (it was just built).
            let left_span = self.spans.get(left).unwrap_or(op_span);
            let span = left_span.merge(self.prev_span());
            left = self.list(vec![head, left, rhs], span);
            // Guard the layer just folded (checked AFTER building it, so a chain that stops at the limit
            // still yields a well-formed node), bounding total arena depth `self.depth + spine` like the
            // `postfix` loop — a deep glued unit chain gets a clean depth diagnostic, not a downstream crash.
            spine += 1;
            if !self.depth_exceeded && self.depth + spine >= crate::sexpr::MAX_NESTING_DEPTH {
                self.error("expression nests too deeply to parse");
                self.depth_exceeded = true;
                return left;
            }
        }
        left
    }

    /// Apply a GLUED integer exponent `^n` to a unit `atom` → `(Unit.^ atom n)`, else return `atom`
    /// unchanged. Only an integer literal is a valid unit exponent; `^` binds tighter than `/`/`*`.
    fn unit_pow(&mut self, atom: StructId) -> StructId {
        if self.kind() != Kind::Caret {
            return atom;
        }
        let (Some(op_tok), Some(exp_tok)) =
            (self.tokens.get(self.pos), self.tokens.get(self.pos + 1))
        else {
            return atom;
        };
        let (op_span, exp_span) = (op_tok.span, exp_tok.span);
        if self.prev_span().end != op_span.start
            || op_span.end != exp_span.start
            || exp_tok.kind != Kind::Int
        {
            return atom;
        }
        let exp_text = self.text(*exp_tok).to_string();
        self.bump(); // `^`
        self.bump(); // integer
        let exp = self.numeric_atom(&exp_text, exp_span);
        // BARE `^` head (eval composes it over a unit operand; the printer round-trips it) — see the
        // `/`/`*` note in `compound_unit_tail` for why bare, not the `Unit.^` flat name.
        let head = self.name("^", op_span);
        // Span the WHOLE `atom ^ n` from the BASE atom's start (not the `^`), so `m^2` highlights `m^2`
        // whole rather than `^2` — a truncated span would mis-anchor a diagnostic on the unit factor.
        let atom_span = self.spans.get(atom).unwrap_or(op_span);
        let span = atom_span.merge(self.prev_span());
        self.list(vec![head, atom, exp], span)
    }

    /// Build a member-access head `(. obj key)` — the arena shape `obj.key` desugars to, reused to
    /// synthesize the `Qty.of` / `Unit.of` heads of a quantity literal.
    fn member_head(&mut self, obj: &str, key: &str, span: Span) -> StructId {
        // M2: `.` is a native Member leaf head (kind identity), the ML counterpart of the s-expr flip.
        let dot = self.atom(Leaf::Member, span);
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
            // (Unit.of #"name"), then extend across a GLUED `/`/`*`/`^` chain into a COMPOUND unit — so
            // `x as GiB/s` converts to the rate unit, matching the `<num> GiB/s` quantity-literal surface
            // (without this the bare-name case read only a SINGLE unit and a following `/s` fell to the
            // enclosing infix loop as a division of the conversion by unbound `s`).
            let unit_head = self.member_head("Unit", "of", span);
            let sym = self.atom(Leaf::Sym(name.into()), span);
            let atom = self.list(vec![unit_head, sym], span);
            return self.compound_unit_tail(atom);
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
                    node = self.member_access(node, start);
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
            // still yields a well-formed node). Bound the TOTAL arena depth `self.depth + spine`: the
            // enclosing recursion depth (`self.depth`, one frame per bracket/keyword nesting level) and
            // the postfix layers this loop folds BOTH deepen the same produced tree, and a recursive
            // consumer (printer/`canon`) walks their SUM. A deeply-parenthesized expression WITH a long
            // postfix chain (`(((x)))….a.a.a…`) builds an arena of depth `self.depth + spine`; a
            // spine-only check let that reach ~2× MAX_NESTING_DEPTH — reintroducing the stack-overflow
            // DoS the guard exists to prevent (PR #383). NOT a double-count: `self.depth` is real tree
            // depth, and the infix guard in `expr` bounds `self.depth + spine` the same way. A nest at
            // the recursion limit plus one postfix layer is legitimately past the bound, so rejecting it
            // is correct; a combined depth under the limit still parses (no over-rejection).
            spine += 1;
            if !self.depth_exceeded && self.depth + spine >= crate::sexpr::MAX_NESTING_DEPTH {
                self.error("expression nests too deeply to parse");
                self.depth_exceeded = true;
                return node;
            }
        }
        node
    }

    /// Fold ONE `.member` access onto `node`: consume the `.` and its key, returning `(. node key)`.
    /// The key is a field name, an escaped/backtick name, a numeric index (`obj.0`, positional tuple
    /// access), or the wildcard `*` (`obj.*` — the whole-constructor-set member the export surface uses).
    /// Shared by the value [`Self::postfix`] and the type [`Self::type_postfix`] (a qualified type name
    /// `M.T` is the same `.`-chain), so both surfaces build the identical `(. …)` node. `start` is the
    /// span of the base expression, so the folded node's span covers the whole `base.key`. Call only when
    /// [`Self::dot_is_member`] holds (a `.` followed by a member key).
    fn member_access(&mut self, node: StructId, start: Span) -> StructId {
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
        // M2: native Member leaf head (kind identity), the ML counterpart of the s-expr member flip.
        let dot = self.atom(Leaf::Member, dot_span);
        self.list(vec![dot, node, key], dot_span)
    }

    /// The payload of an `@!param` module pragma: a glued `(config kv…)` application followed by a
    /// `name : Type` binder. Returns `[config, binder]` to append after the `pragma param` head, giving
    /// `(pragma param (param (: widget slider) …) (: name Type))`. The config kvs are `key: value`
    /// ascriptions (`(: key value)`), the same kv shape the `@param` annotation's `(param (: widget
    /// slider) …)` config carries, so the sidecar reads them identically; the binder gives the param NAME
    /// and its DECLARED TYPE (an `@param` must be typed). A missing/empty config is `(param)`; the binder
    /// is REQUIRED (an untyped or unnamed `@!param` records an error and recovers — never-panic).
    fn param_pragma_payload(&mut self) -> Vec<StructId> {
        // The config kvs group under a `(param <kv>…)` sub-node — BYTE-SIMILAR to the config of today's
        // `@param` annotation (whose name-slot is the app `(param (: widget slider) …)`), so
        // v-metaprogramming's `scan_manifest` reads the config off this node exactly as it does today
        // (v-metaprogramming LOCKED this head — grouped config as one node, unambiguously separate from the
        // binder sibling — over config kvs as direct pragma children, which it could not tell apart from the
        // trailing `(: name Type)` binder).
        let config_start = self.cur_span();
        let config_head = self.name("param", config_start);
        let mut config = vec![config_head];
        // A GLUED `(` (no intervening space — same adjacency the `@tag("…")`/quantity sugars use) opens the
        // config kv list. Each entry is `key: value` -> `(: key value)`. No glued `(` = an empty config.
        if self.at(Kind::LParen) && self.prev_span().end == self.cur_span().start {
            self.bump(); // `(`
            if !self.at(Kind::RParen) {
                loop {
                    let kv_start = self.cur_span();
                    let label = self.binder();
                    if self.at(Kind::Colon) {
                        self.bump(); // `:`
                        let colon = self.name(":", kv_start);
                        let value = self.expr(crate::token::PREC_SEQ + 1);
                        let kv_span = kv_start.merge(self.prev_span());
                        config.push(self.list(vec![colon, label, value], kv_span));
                    } else {
                        // A bare key with no `: value` is malformed config; keep the label so the shape is
                        // visible and recover.
                        self.error("expected `key: value` in an `@!param` config");
                        config.push(label);
                    }
                    if !self.sep_continue(Kind::RParen) {
                        break;
                    }
                }
            }
            self.expect(Kind::RParen, "`)`");
        }
        let config_span = config_start.merge(self.prev_span());
        let config_node = self.list(config, config_span);
        // The `name : Type` binder — REQUIRED. `name` is a plain binder; `: Type` is a `type_ref`. Parsed
        // directly (NOT the general `expr`) so the trailing `name` is not eaten as a unit-suffix on the
        // pragma (the bug this whole branch fixes) and `: Type` is a type, not a value ascription.
        let binder_start = self.cur_span();
        let name = self.binder();
        let binder = if self.at(Kind::Colon) {
            self.bump(); // `:`
            let colon = self.name(":", binder_start);
            let ty = self.type_ref();
            let binder_span = binder_start.merge(self.prev_span());
            self.list(vec![colon, name, ty], binder_span)
        } else {
            self.error("an `@!param` needs a `name : Type` binder (e.g. `@!param(widget: slider) width : Int64`)");
            name
        };
        vec![config_node, binder]
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
        // A call's `( … )` is a bracket boundary, so a `|` inside a call ARGUMENT is bitwise-or, not a
        // match/handle-arm terminator — clear `arm_bar_terminates` for the duration (restored after),
        // exactly as `bracketed_bars` does for a parenthesized/list/record sub-expression. Without this a
        // `resume(x | 8, …)` in an arm body printed by the ML printer failed to re-parse: the reader took
        // the `|` inside the call args as the start of the next arm (breaker's pipe-in-arm round-trip bug).
        let saved_arm_bar = self.arm_bar_terminates;
        self.arm_bar_terminates = false;
        let mut args = Vec::new();
        if !self.at(Kind::RParen) {
            loop {
                // Own-line `//` comment(s) leading this argument (`g(\n // note\n 1, 2)`) sit in its
                // first-token leading slot, which `expr` does not drain — capture + wrap `(comment "text"
                // arg)` so they round-trip (the call printer renders a leading comment on its own line
                // above the arg).
                let leading = self.take_comments_here();
                // An argument is a single expression, not a sequence (`PREC_SEQ + 1`): a `;` here belongs
                // to an enclosing block, so a sequence passed as an argument must parenthesize —
                // `f((a; b))` — matching the "parens only for a genuine ambiguity" surface rule.
                let arg = self.expr(crate::token::PREC_SEQ + 1);
                let arg = self.wrap_comments(leading, arg);
                // A same-line `//` trailing the LAST argument (`g(1, 2 // note)`) sits in the `)` token's
                // leading slot; capture it as `(comment-after …)` (gated on `at(RParen)`, the PR#758 rule:
                // a non-last arg's next token is `,`, and a same-line comment there has no faithful inline
                // rendering). The call printer renders it same-line + forces `)` onto its own line.
                if self.at(Kind::RParen) {
                    let trailing = self.take_trailing_comment_here();
                    args.push(self.wrap_comment_after(trailing, arg));
                } else {
                    args.push(arg);
                }
                if !self.sep_continue(Kind::RParen) {
                    break;
                }
            }
        }
        self.expect(Kind::RParen, "`)`");
        self.arm_bar_terminates = saved_arm_bar;
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
        // A LEADING `.. a` spread forces the tuple path — a spread has no meaning as a grouping — and
        // builds the `#tuple((.. a) …)` construction-spread node (the tuple twin of the list/set/map/record
        // construction spread; the printer already renders `(.. a, …)`). The `..` head is created via
        // `rest_marker`, exactly as the other compounds do.
        if self.at(Kind::DotDot) {
            let head = self.ctor_head("tuple", start);
            let mut items = vec![head];
            self.rest_marker(&mut items, |p| p.expr(crate::token::PREC_SEQ + 1));
            return self.finish_tuple(items, start);
        }
        // Own-line `//` comment(s) leading the first element (`(\n // note\n 1, 2)` or a grouped `(\n
        // // note\n e)`) sit in its first-token leading slot, which `expr` does not drain — capture +
        // wrap `(comment "text" first)` so they round-trip (the printer renders a leading comment on its
        // own line above the expr). Applies to both the tuple and the transparent-grouping outcome.
        let first_leading = self.take_comments_here();
        let first = self.expr(0);
        let first = self.wrap_comments(first_leading, first);
        if self.at(Kind::Comma) {
            // a tuple: gather the rest. The head is the STRING primitive `"tuple"` (not the name), so the
            // literal builds the unshadowable tuple constructor even where the name `tuple` is rebound.
            let head = self.ctor_head("tuple", start);
            let items = vec![head, first];
            return self.finish_tuple(items, start);
        }
        self.expect(Kind::RParen, "`)`");
        first // grouping (or the folded `(do …)` sequence) is transparent in the arena
    }

    /// Gather the remaining `,`-separated tuple elements onto `items` (already holding `[ "tuple"-head,
    /// … ]`), recovering from a missing `,`, then close `)` and build the tuple node. Each element is a
    /// single expression at `PREC_SEQ + 1` (not a sequence — a `;` belongs to an enclosing block) OR a
    /// `.. a` CONSTRUCTION SPREAD (`(.. operand)`, via `rest_marker` — the tuple twin of the list/set/
    /// map/record spread, so `(1, .. a)` / `(.. a, 1)` round-trip). Only a LAST-element same-line comment
    /// is captured (gated on `at(RParen)`, the PR#758 rule — a non-last element's comment sits before the
    /// `,` with no faithful slot). `rest_marker` returns false on a non-`..` element, so an ORDINARY tuple
    /// is byte-identical — this is pure ADDITIVE acceptance of a previously-rejected spread. Shared by the
    /// recursive `paren` and the iterative `Cont::Paren` path (kept in sync by the differential oracle).
    fn finish_tuple(&mut self, mut items: Vec<StructId>, start: Span) -> StructId {
        while self.sep_continue(Kind::RParen) {
            if !self.rest_marker(&mut items, |p| p.expr(crate::token::PREC_SEQ + 1)) {
                // Own-line leading comment before this element (own-line has no swallow hazard — see
                // `list_literal`), then the element, then a same-line trailing comment on the LAST element.
                let leading = self.take_comments_here();
                let elem = self.expr(crate::token::PREC_SEQ + 1);
                let elem = self.wrap_comments(leading, elem);
                if self.at(Kind::RParen) {
                    let trailing = self.take_trailing_comment_here();
                    items.push(self.wrap_comment_after(trailing, elem));
                } else {
                    items.push(elem);
                }
            }
        }
        // Own-line `//` before `)` (`(1, 2\n // note\n)`) → attach to the last element (see the helper).
        self.drain_closer_comment_onto_last(&mut items, 1);
        self.expect(Kind::RParen, "`)`");
        let span = start.merge(self.prev_span());
        self.list(items, span)
    }

    // ---- keyword forms ----

    /// `let n = e, … in body`  ->  `(let ((n e) …) body)`. The binding is separated from the body by
    /// `in`, which SELF-DELIMITS the `let` — its body is a full expression, so a `let` at the tail of
    /// a def body cannot swallow following top-level forms (the dangling-let fix). The body is a plain
    /// expression, not a `;`-sequence (a multi-statement body parenthesizes as `(a; b)`).
    /// Read a single `let` binder position: a plain name, a destructuring `pattern()` (irrefutable
    /// binding-position patterns, like `param`), each with an OPTIONAL type annotation `x: T` folded to
    /// `(: binder T)` (the s-expr binder-annotation shape; closes the `let x: T = …` surface). `b_start` is
    /// the span at the binder start (drained of leading comments), used for the `:`/annotation spans.
    /// Shared by the recursive `let_expr` and the iterative `Cont::Let` path so the two never drift.
    fn read_let_binder(&mut self, b_start: Span) -> StructId {
        let n = if self.at_pattern_param_start() {
            self.pattern()
        } else {
            self.binder()
        };
        if self.at(Kind::Colon) {
            self.bump(); // `:`
            let colon = self.name(":", b_start);
            let ty = self.type_ref();
            let ann_span = b_start.merge(self.prev_span());
            self.list(vec![colon, n, ty], ann_span)
        } else {
            n
        }
    }

    fn let_expr(&mut self) -> StructId {
        let start = self.cur_span();
        let let_head = self.keyword_head("let", start);
        self.bump(); // `let`
        let mut bindings = Vec::new();
        loop {
            // Own-line `//` comment(s) leading this binding (`let\n // note\n x = 1 in …`, or before a
            // `,`-separated later binding) sit in the binder's first-token leading slot, which
            // `pattern`/`binder`/`expr` do not drain — capture + wrap the `(binder value)` pair in
            // `(comment "text" …)` so it round-trips (`is_let_shape` peels it; `print_let` renders the
            // comment on its own line above the binding + forces the bindings to break). Own-line has no
            // swallow hazard.
            let leading = self.take_comments_here();
            let b_start = self.cur_span();
            // The binder: a plain name / destructuring pattern, with an optional `: T` annotation folded to
            // `(: binder T)` — see `read_let_binder` (shared with the iterative reader).
            let n = self.read_let_binder(b_start);
            self.expect(Kind::Eq, "`=`");
            // An own-line `//` comment BETWEEN `=` and the value (`let y =<newline> // note<newline>
            // x + 1`) sits at the value's first-token leading slot, which `expr` does not drain — capture
            // + wrap `(comment "text" value)` so it round-trips (else `cdz fmt` refuses). Same own-line
            // leading-comment capture as if_expr's branches / the binding's own leading comment.
            let e_lead = self.take_comments_here();
            // The bound value is a single expression (`PREC_SEQ + 1`), delimited by `in` (or the next
            // `,` binding). A `;` after it belongs to the enclosing sequence — `let x = a in b; c` is
            // `(do (let x=a in b) c)` — so a sequence VALUE parenthesizes: `let x = (a; b) in …`.
            let e = self.expr(crate::token::PREC_SEQ + 1);
            let e = self.wrap_comments(e_lead, e);
            let b_span = b_start.merge(self.prev_span());
            let binding = self.list(vec![n, e], b_span);
            bindings.push(self.wrap_comments(leading, binding));
            if self.at(Kind::Comma) {
                self.bump();
            } else {
                break;
            }
        }
        let binds_span = start.merge(self.prev_span());
        let binds = self.list(bindings, binds_span);
        self.expect_keyword(Keyword::In, "`in`");
        // A same-line TRAILING `//` after `in` (`let x = a in // note<newline> body`) sits at the body's
        // first-token leading slot. Capture it as a TRAILING comment on the bindings (`(comment-after …
        // binds)`) so the printer re-emits it same-line after `in` (`let x = a in // note`) — it round-
        // trips (else `cdz fmt` refuses; the comment-attachment gap that blocked hm-collect.cdz). It is a
        // TRAILING (same-line) comment, distinct from an own-line comment leading the body (which the
        // body's own `expr` leading-slot would carry).
        let in_trail = self.take_trailing_comment_here();
        let binds = self.wrap_comment_after(in_trail, binds);
        // An OWN-LINE `//` comment leading the body (`let x = a in<newline> // note<newline> body`) — a
        // NON-trailing lead remaining after the same-line trailing capture above — sits at the body's
        // leading slot. Capture + wrap `(comment "text" body)` so it prints own-line above the body
        // (round-trips; else stranded → `cdz fmt` refuses).
        let body_lead = self.take_comments_here();
        let body = self.expr(0);
        let body = self.wrap_comments(body_lead, body);
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
        // Own-line `//` comment(s) leading the condition / then-branch / else-branch (`if\n // note\n c
        // then …`, `if c then\n // note\n t else …`, `else\n // note\n e`) sit in each sub-expr's
        // first-token leading slot, which `expr` does not drain — capture + wrap `(comment "text" expr)`
        // so they round-trip (the printer renders a leading comment on its own line above the expr). No
        // swallow hazard (own-line). Mirrors the collection/let leading-comment capture.
        let c_lead = self.take_comments_here();
        let c = self.expr(crate::token::PREC_SEQ + 1);
        let c = self.wrap_comments(c_lead, c);
        // Own-line `//` comment(s) BEFORE the `then` keyword (`if c<newline> // note<newline> then t`)
        // sit in the `then` token's leading slot; `expect_keyword` would consume past them, dropping
        // them. Capture before the bump + fold into the then-branch's leading comments so they print
        // own-line above the then-branch (round-trips; else stranded → `cdz fmt` refuses). Symmetric with
        // the own-line-before-`else` capture below.
        let mut t_lead = self.take_comments_here();
        self.expect_keyword(Keyword::Then, "`then`");
        t_lead.extend(self.take_comments_here());
        let t = self.expr(crate::token::PREC_SEQ + 1);
        let t = self.wrap_comments(t_lead, t);
        // A same-line TRAILING `//` after the then-branch (`if a then 1 // note<newline> else …`) sits at
        // the `else` keyword's leading slot; `expr` did not drain it. Capture + wrap it `(comment-after
        // …)` on the then-branch so it round-trips (else `cdz fmt` refuses to format the whole file —
        // the comment-attachment gap that blocked hm-collect.cdz). Mirrors the leading-comment capture
        // above + the list/tuple/record trailing capture (`take_trailing_comment_here`).
        let t_trail = self.take_trailing_comment_here();
        let t = self.wrap_comment_after(t_trail, t);
        // Own-line `//` comment(s) sitting BEFORE the `else` keyword (`if a then 1<newline> // note<newline>
        // else 2`) are in the `else` token's leading slot; `expect_keyword` would consume past them,
        // dropping them. Capture them here (before the bump) and fold them into the else-branch's leading
        // comments so they print own-line above the else-branch (round-trips; they'd otherwise strand).
        let mut e_lead = self.take_comments_here();
        self.expect_keyword(Keyword::Else, "`else`");
        e_lead.extend(self.take_comments_here());
        let e = self.expr(crate::token::PREC_SEQ + 1);
        let e = self.wrap_comments(e_lead, e);
        // A same-line trailing `//` after the else-branch — the if is at an expression tail, so this is
        // captured the same way (it re-prints on the else line). Construct-end trailing already
        // round-trips in many contexts, but capturing here makes the if uniform + robust in nested use.
        let e_trail = self.take_trailing_comment_here();
        let e = self.wrap_comment_after(e_trail, e);
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
        // A leading `forall a b.` (P1 ergonomic generic-def spelling: `def forall a. f(x: a) = …`) —
        // consumed BEFORE the name, since it precedes it. Yields synthesized `(: a Type)` params to
        // PREPEND to the signature (pure sugar; the same arena a hand-written leading `(: a Type)` param
        // or the param-annotation `forall` desugar produces). A `forall`-prefixed def is always a
        // FUNCTION def (a generic value def is meaningless), so the value-def branch below is skipped.
        let sig_type_params = self.forall_sig_type_params();
        let name = self.binder();

        // ---- value definition: `def name = value` -> (def name value) ----
        // (Not reachable when a leading `forall` was consumed — that forces the function-def form.)
        if sig_type_params.is_none() && self.at(Kind::Eq) {
            self.bump(); // `=`
            // A value def binds a single expression (`PREC_SEQ + 1`), like a `let` binding — NOT a
            // sequence. A `;` after it belongs to the enclosing sequence, so `def x = 5; rest` is
            // `(do (def x 5) rest)`: the def hoists `x` into scope for `rest` (the corpus's
            // `(do (def x 5) (+ x 1))` reading), rather than making `5; rest` the value. A value that
            // is itself a sequence parenthesizes: `def x = (a; b)`. (A FUNCTION body, by contrast, IS a
            // sequence position — it collects its `;`-run — since its body is delimited by the next
            // top-level form, with no trailing "rest" to escape into.)
            // `body_expr` drains any leading interior `//`/`///` trivia (a comment on its own line
            // after the `=`) so it isn't stranded + dropped.
            let value = self.body_expr(crate::token::PREC_SEQ + 1);
            let span = start.merge(self.prev_span());
            // (def name doc… value) — docs precede the value, mirroring the function form.
            let mut items = vec![def_head, name];
            items.extend(self.doc_nodes(docs));
            items.push(value);
            return self.list(items, span);
        }

        // ---- function definition: `def name(p, …) = body` -> (def (name p …) body) ----
        self.expect(Kind::LParen, "`(`");
        let mut params = Vec::new();
        if !self.at(Kind::RParen) {
            loop {
                let before = self.pos;
                params.push(self.param());
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
        // Desugar any `forall` binder in a parameter annotation into leading `(: a Type)` params.
        let params = self.hoist_forall_params(params, sig_span);
        let mut sig = vec![name];
        // A leading `def forall a b. …` clause prepends its `(: a Type)` params ahead of the value params
        // (source order), so both the leading-clause form and the param-annotation form desugar to the
        // SAME signature. `forall a. f(x: forall b. …)` composes: the leading `a` comes first, then the
        // annotation-hoisted `b` (in `hoist_forall_params` order).
        if let Some(tps) = sig_type_params {
            sig.extend(tps);
        }
        sig.extend(params);
        let signature = self.list(sig, sig_span);
        // Optional return-type annotation `-> R` between the signature and `=`. It desugars to a body
        // ascription: `def f(x) -> R = e` becomes `(def (f x) (: e R))`, reusing the annotation form —
        // no dedicated return-type node. The printer recovers the `-> R` from that body shape.
        let ret_ty = self.opt_return_type();
        self.expect(Kind::Eq, "`=`");
        let body = self.body_expr(0);
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
        // absence for robustness. Each `|` introduces a variant. A `..`-led body is a ZERO-named-variant
        // OPEN SUM (`type Opaque = .. r` — no variants, only a row tail): skip the variant loop so the
        // trailing-`.. r` handler below reads the tail (without it, the loop's unconditional first
        // `variant()` would try to read a name at `..` and fail — the trunk-red round-trip bug).
        // Own-line `//` comment(s) leading the FIRST variant sit in the leading `|`'s slot (only `///`
        // DOCS were drained above, as `(doc)`; a `//` comment is distinct). Drain BEFORE the `|` bump —
        // a drain after it would miss them (mirrors `match_expr`'s arm-comment capture).
        let mut pending_leading = self.take_comments_here();
        if self.at(Kind::Pipe) {
            self.bump(); // optional leading `|`
        }
        if !self.at(Kind::DotDot) {
            loop {
                // Wrap the variant in any own-line LEADING `//` comment(s) drained before its `|` — the
                // printer renders them on their own line above the variant; `is_type_shape` unwraps them.
                // Own-line has no swallow hazard.
                let leading = std::mem::take(&mut pending_leading);
                let v = self.variant();
                let v = self.wrap_comments(leading, v);
                // A `//` comment trailing this variant on the same line (`| Ctor(T)  // note`) sits at
                // the NEXT token's leading slot (the `|` or the type's end). Drain + attach it to THIS
                // variant as `(comment-after …)` so it re-prints same-line, rather than being stranded
                // at the next variant's slot (where the variant loop would drop it — the trailing-inline
                // comment-loss). `strip_comments` peels it, so the type scanner is unaffected.
                let trailing = self.take_trailing_comment_here();
                items.push(self.wrap_comment_after(trailing, v));
                // Own-line comment(s) leading the NEXT variant sit in the upcoming `|`'s slot — drain
                // before the bump (into `pending_leading` for the next iteration), like `match_expr`.
                pending_leading = self.take_comments_here();
                if self.at(Kind::Pipe) {
                    self.bump(); // `|`
                } else {
                    // No more variants: any own-line comment(s) we drained lead what FOLLOWS the type decl
                    // (the next `def`, or an own-line comment after the last variant / a multi-line trailing
                    // comment's own-line continuation), NOT a nonexistent next variant. Restore them to the
                    // current token's leading slot so the enclosing parser attaches them instead of dropping
                    // them (the seq-277 reader-attachment gap; mirrors `match_expr`'s restore-on-break).
                    if !pending_leading.is_empty() && self.pos < self.leading.len() {
                        let mut restored = std::mem::take(&mut pending_leading);
                        restored.append(&mut self.leading[self.pos]);
                        self.leading[self.pos] = restored;
                    }
                    break;
                }
            }
        }
        // OPEN SUM: an optional trailing `.. r` row-variable marker after the last variant — the sum is
        // OPEN over the variants a caller may add, `r` naming the open tail (open-sums OS1). It lowers to
        // the SAME flat two-sibling convention the collection-literal/pattern rest uses: a bare `..` Name
        // atom then a bare lowercase Name atom, appended as the type list's two final children (NOT a
        // wrapper node) — exactly what `db.rs::scan_type_decl` reads and what the s-expr corpus carries
        // (`(type Vocab (Known Unit) .. r)`). The row var is lowercase (a Capitalized trailing name is a
        // normal nullary variant, consumed by the loop above); at most one `.. r` (a sum has one tail).
        if self.at(Kind::DotDot) {
            let dd_span = self.cur_span();
            self.bump(); // `..`
            items.push(self.name("..", dd_span));
            // The row var is a lowercase name. `binder` records a clean diagnostic if it's absent.
            items.push(self.binder());
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
        // A variant's payload is the SAME paren-arg surface as a type application: each arg is either a
        // bare type OR a LABELED field `name : Type` → `(: name Type)`. The labeled form is what a
        // RECORD-payload variant uses — `(type R (record (: field Ty)))` prints as `R = | record(field :
        // Ty)`, and without accepting the `name : Ty` label here that re-parse failed at the `:` (the
        // `(type _ (record …))` type-decl surface was never round-trip-exercised — breaker's report;
        // same class as the derived-unit `type_ref` gap). Reuse `type_arg_exprs`, which parses exactly
        // this (label or bare type) and shares the missing-`,` recovery.
        if !self.at(Kind::RParen) {
            // `type_arg_exprs` consumes its own `(`/`)`; we already bumped `(`, so parse the args with a
            // helper that assumes the `(` is open. Inline the same loop to avoid a double-`(` expect.
            loop {
                let before = self.pos;
                items.push(self.type_arg());
                if !self.sep_continue(Kind::RParen) {
                    break;
                }
                // A stop token that `type_arg`/`prefix` didn't consume would spin the missing-`,` branch;
                // skip it so the loop always makes progress.
                if self.pos == before {
                    self.bump();
                }
            }
        }
        self.expect(Kind::RParen, "`)`");
        let span = start.merge(self.prev_span());
        self.list(items, span)
    }

    /// One type-application / variant-payload argument: a LABELED field `name : Type` → `(: name Type)`,
    /// or a bare [`Self::type_ref`]. A label is an `Ident`/backtick-name IMMEDIATELY followed by `:`
    /// (so a bare `M.T` / `List(a)` positional arg is unaffected). Shared by [`Self::type_arg_exprs`]
    /// (annotation position, e.g. `Record(x: Int64)`) and [`Self::variant`] (a record-payload variant,
    /// e.g. `record(field : Ty)`), so both accept the label form identically.
    fn type_arg(&mut self) -> StructId {
        if let Some((start, label)) = self.type_arg_label() {
            let ty = self.type_ref();
            let colon = self.name(":", start);
            let span = start.merge(self.prev_span());
            self.list(vec![colon, label, ty], span)
        } else {
            self.type_ref()
        }
    }

    /// If the cursor is at a LABELED type-application argument `name:` (an `Ident`/backtick-name
    /// IMMEDIATELY followed by `:`), consume the label + the `:` and return `(label-start-span, label-node)`;
    /// otherwise consume nothing and return `None` (a bare positional type argument). Shared by the
    /// recursive [`Self::type_arg`] and the iterative [`Self::type_ref_iter`] so the label-then-type node
    /// creation order can't drift between the two readers (the completed type becomes `(: label ty)` in both).
    fn type_arg_label(&mut self) -> Option<(Span, StructId)> {
        let start = self.cur_span();
        let is_label = matches!(self.kind(), Kind::Ident | Kind::BacktickName)
            && self.nth_kind(1) == Kind::Colon;
        if !is_label {
            return None;
        }
        let label = match self.kind() {
            Kind::BacktickName => {
                let t = self.bump().unwrap();
                self.name(literal::unescape_backtick_name(self.text(t)), start)
            }
            _ => {
                let t = self.bump().unwrap();
                self.name(self.text(t), start)
            }
        };
        self.expect(Kind::Colon, "`:`");
        Some((start, label))
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
        // Index in `items` where MEMBERS begin (past `head`, `name`, and any leading `(doc …)`). The
        // trailing-comment handler below must never pop below this — the module NAME is not a member.
        let members_start = items.len();
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
        self.finish_module_body(items, members_start, start)
    }

    /// Close a `module { … }` body: drain any `///` doc / `//` comment stranded in the closing `}` slot
    /// (re-attaching per the rules below), consume `}`, and build the `(module …)` node. `items` already
    /// holds `[ "module"-head, name, leading-doc…, member… ]`; `members_start` is the index where members
    /// begin (the trailing-comment wrap must never pop below it — the name is not a member); `start` is the
    /// module's span. Shared by the recursive `module_expr` and the iterative `Cont::Module` path.
    ///
    /// A `///` doc / `//` comment on the last line(s) before `}` sits in the `}` token's leading slot (the
    /// member loop exits at `}` without draining it) and would be STRANDED + DROPPED (a comment/doc LOSS →
    /// `cdz fmt` refuses the file). Re-attach, mirroring `program()`'s trailing handler: a `///` doc becomes
    /// a trailing `(doc …)` module MEMBER; a leftover `//` wraps the LAST member as `(comment …)`; a `//` in
    /// an EMPTY body is left in the slot for the drop-guard (no standalone carrier round-trips).
    fn finish_module_body(
        &mut self,
        mut items: Vec<StructId>,
        members_start: usize,
        start: Span,
    ) -> StructId {
        let (docs, comments): (Vec<Lead>, Vec<Lead>) = {
            let leads: Vec<Lead> = if self.pos < self.leading.len() {
                std::mem::take(&mut self.leading[self.pos])
            } else {
                Vec::new()
            };
            leads.into_iter().partition(|l| l.doc)
        };
        if !comments.is_empty() {
            if items.len() > members_start {
                let last = items.pop().unwrap();
                items.push(self.wrap_comments(comments, last));
            } else if self.pos < self.leading.len() {
                self.leading[self.pos] = comments;
            }
        }
        // A trailing `///` doc becomes a `(doc …)` member at the END of the module body (empty or not).
        items.extend(self.doc_nodes(docs));
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
        // Leading `///` docs attach INSIDE the effect decl, as `(doc "text")` forms before the
        // operations — mirroring `type_expr`. Without this drain the docs would be left in the
        // statement's start slot and `stmt` would wrap them as `(comment …)` (printed `//`),
        // silently downgrading `///` doc-comments before an `effect` to `//` and losing the doc
        // marker on round-trip.
        let docs = self.take_docs_here();
        let head = self.keyword_head("effect", start);
        self.bump(); // `effect`
        let name = self.binder();
        let mut items = vec![head, name];
        items.extend(self.doc_nodes(docs));
        self.expect(Kind::Eq, "`=`");
        // Operations are `|`-led, with an (always-printed) leading `|` before the first — tolerate its
        // absence for robustness. Each `|` introduces an operation signature.
        if self.at(Kind::Pipe) {
            self.bump(); // optional leading `|`
        }
        loop {
            let op = self.effect_op();
            // A same-line `//` trailing this op (`| op : Sig  // note`) sits at the next `|`/decl-end
            // token's leading slot, tagged trailing. Attach it to the op as `(comment-after …)` so it
            // re-prints same-line — else a NON-last op's trailing is DROPPED, and the LAST op's trailing
            // mis-attaches to the FOLLOWING def (seq-277 gap: db-query-perfield.cdz). Only the same-line
            // PREFIX; own-line comments + leading `///` docs are drained elsewhere, untouched.
            let trailing = self.take_trailing_comment_here();
            items.push(self.wrap_comment_after(trailing, op));
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
        // SEC-F1 RESOURCE MARKER (concierge-ruled, coordinated w/ v-agent-harness 2026-08-13): a param
        // written `@resource T` designates the SEC-F1 resource arg the kernel/executor extracts for
        // routing + Cedar authz. `@resource T` parses as an `(@ resource T)`-wrapped param in the op-type
        // arrow spine; here we LIFT it OUT — unwrap the wrapped param to its bare type (so the op TYPE the
        // schema-hash is built from carries NO marker → hash-invariant BY CONSTRUCTION) and record its
        // POSITION as a decl-level `(resource <index>)` sibling on the op node. The kernel reads that
        // sibling off the descriptor; rcdzc reify stays capability-blind. At most one resource per op.
        let span = start.merge(self.prev_span());
        let (ty, resource_idx) = self.lift_resource_marker(ty, span);
        let mut items = vec![op_head, op_name, ty];
        if let Some(idx) = resource_idx {
            let resource_head = self.name("resource", span);
            let idx_atom = self.atom(
                Leaf::Int {
                    value: crate::ast::IntValue::from_i64(idx as i64),
                    radix: crate::ast::Radix::Dec,
                },
                span,
            );
            items.push(self.list(vec![resource_head, idx_atom], span));
        }
        self.list(items, span)
    }

    /// Scan an effect-op TYPE (a curried arrow `(-> P0 (-> P1 … R))`, or a bare non-arrow) for a param
    /// written `@resource T` (an `(@ resource T)` wrapper). If found, return a marker-FREE copy of the
    /// type (the wrapped param unwrapped to its bare `T`) plus the resource param's 0-based POSITION;
    /// otherwise return the type unchanged and `None`. Only the FIRST `@resource` is honored (at most one
    /// resource per op); a `@resource` on the result position is ignored (a result is not an arg). Keeping
    /// the marker OUT of the returned op-type is what makes the schema-hash resource-marker-invariant.
    fn lift_resource_marker(&mut self, ty: StructId, span: Span) -> (StructId, Option<usize>) {
        // Collect the curried-arrow params (left of each `->`) + the final result, so we can rebuild.
        let mut params: Vec<StructId> = Vec::new();
        let mut cur = ty;
        loop {
            // An arrow node is `(-> P rest)` (2-param) or the nullary-elided `(-> R)` (1-param = result).
            let arrow = match self.builder.get(cur) {
                crate::ast::Struct::List(kids)
                    if kids.len() == 3 && self.builder.as_name(kids[0]) == Some("->") =>
                {
                    Some((kids[1], kids[2]))
                }
                _ => None,
            };
            match arrow {
                Some((p, rest)) => {
                    params.push(p);
                    cur = rest;
                }
                None => break, // `cur` is the result (a bare type or a nullary `(-> R)` handled by caller)
            }
        }
        let result = cur;
        // Find the first param that is `(@ resource T)`; unwrap it, record its index.
        let mut resource_idx = None;
        let mut new_params = Vec::with_capacity(params.len());
        for (i, &p) in params.iter().enumerate() {
            if resource_idx.is_none()
                && let Some(inner) = self.unwrap_resource_param(p)
            {
                resource_idx = Some(i);
                new_params.push(inner);
            } else {
                new_params.push(p);
            }
        }
        if resource_idx.is_none() {
            return (ty, None); // no marker — return the original type untouched
        }
        // Rebuild the curried arrow `(-> P0 (-> P1 … result))` from the marker-free params.
        let mut rebuilt = result;
        for &p in new_params.iter().rev() {
            let arrow_head = self.name("->", span);
            rebuilt = self.list(vec![arrow_head, p, rebuilt], span);
        }
        (rebuilt, resource_idx)
    }

    /// If `p` is an `(@ resource T)` param-marker wrapper, return the inner `T`; else `None`. The marker
    /// is the general `@`-annotation form `(@ <name> <form>)` with the name `resource`.
    fn unwrap_resource_param(&self, p: StructId) -> Option<StructId> {
        match self.builder.get(p) {
            crate::ast::Struct::List(kids)
                if kids.len() == 3
                    && self.builder.as_name(kids[0]) == Some("@")
                    && self.builder.as_name(kids[1]) == Some("resource") =>
            {
                Some(kids[2])
            }
            _ => None,
        }
    }

    /// `world Name = | import Iface = | member : Sig … | export Iface = … `  ->
    /// `(world Name (import Iface (member M Func)…) (export Iface …)…)` — the inline WIT-world
    /// declaration (DESIGN inline-WIT-world, converged w/ v-agent-harness 2026-08-11). Lowers to the
    /// SAME canonical node `cadenza-ast::Builder::world_schema_tree` builds, so a target world means one
    /// tree whether it comes from this inline surface, an external binary-AST artifact, or v-cml's emit.
    /// `world` is a CONTEXTUAL keyword (recognized only at a declaration head — a bare `world` elsewhere
    /// stays an ordinary name), matching the operator's "keep it WIT-familiar" surface direction. The
    /// body is `|`-led interfaces, each `import`/`export IfaceName = | member : Sig …` — reusing the
    /// reserved `import`/`export` words as the direction sub-heads so the correspondence to the WIT world
    /// it lowers to is self-evident. All structure heads are NAME atoms (head-kind-fixed, so the world's
    /// content-hash is byte-stable — see `world_schema_tree`).
    fn world_expr(&mut self) -> StructId {
        let start = self.cur_span();
        let docs = self.take_docs_here();
        let head = self.name("world", start);
        self.bump(); // `world`
        let name = self.binder();
        let mut items = vec![head, name];
        items.extend(self.doc_nodes(docs));
        self.expect(Kind::Eq, "`=`");
        // `|`-led interfaces (tolerate an absent leading `|`, mirroring `effect_expr`).
        if self.at(Kind::Pipe) {
            self.bump();
        }
        loop {
            items.push(self.world_interface());
            if self.at(Kind::Pipe) {
                self.bump();
            } else {
                break;
            }
        }
        let span = start.merge(self.prev_span());
        self.list(items, span)
    }

    /// One interface of a `world` body: `import|export IfaceName = | member : Sig …`  ->
    /// `(import|export IfaceName (member M Func)…)`. The direction word (`import`/`export`) is the NAME
    /// sub-head — structural, since the compiler emits a different value-bridge per direction. Members
    /// are `|`-led, each `member : Sig` (see [`Self::world_member`]).
    fn world_interface(&mut self) -> StructId {
        let start = self.cur_span();
        // Direction: reuse the reserved `import`/`export` keywords contextually. Anything else is an error.
        let dir = match keyword(self.cur_text()) {
            Some(Keyword::Import) => "import",
            Some(Keyword::Export) => "export",
            _ => {
                self.error("expected `import` or `export` to head a world interface");
                "import"
            }
        };
        let dir_head = self.name(dir, start);
        self.bump(); // `import`/`export`
        let iface_name = self.binder();
        let mut items = vec![dir_head, iface_name];
        self.expect(Kind::Eq, "`=`");
        if self.at(Kind::Pipe) {
            self.bump();
        }
        loop {
            items.push(self.world_member());
            // A following `|` continues the member list UNLESS it heads a new interface (`| import …` /
            // `| export …`) — that `|` belongs to the enclosing world loop, so stop and leave it.
            if self.at(Kind::Pipe) && !self.next_pipe_heads_interface() {
                self.bump();
            } else {
                break;
            }
        }
        let span = start.merge(self.prev_span());
        self.list(items, span)
    }

    /// True when the CURRENT `|` is followed by an interface-direction keyword (`import`/`export`) — i.e.
    /// the `|` opens a new world interface, not another member of the current one. Used by
    /// [`Self::world_interface`] to hand a `| import …`/`| export …` back to the world-level loop.
    fn next_pipe_heads_interface(&self) -> bool {
        debug_assert!(self.at(Kind::Pipe));
        self.nth_kind(1) == Kind::Ident
            && self
                .tokens
                .get(self.pos + 1)
                .map(|&t| {
                    matches!(
                        keyword(self.text(t)),
                        Some(Keyword::Import | Keyword::Export)
                    )
                })
                .unwrap_or(false)
    }

    /// One member of a world interface: `member_name : ( p1 : T1 , … ) -> R`  ->
    /// `(member member_name (func (param p1 T1) … (result R)))`. A WIT func is named params plus a
    /// result; the `func` wrapper groups them (a lone type descriptor encodes one type). The param list
    /// is parenthesized and comma-separated (each `name : type`); the result follows `->`. A nullary
    /// member is `member_name : () -> R` (empty parens) or `member_name : -> R`. The result is ALWAYS
    /// present (no omitted slot) — a no-return member writes `-> Unit`.
    /// Lower a parsed ML type node (from [`Self::type_ref`]) to the CANONICAL WIT type-descriptor form
    /// the kernel's `build_type` and rcdzc's `parse_wit_type` share — via the shared
    /// `Builder::wit_type_prim`/`wit_type_list`/`wit_type_option`, so the inline world surface encodes
    /// BYTE-IDENTICALLY to the external artifact + v-cml's emit (v-agent-harness ruling 2026-08-12; both
    /// route through the same builders). Recognized member types (MVP scope = the shared builders'):
    /// - a bare WIT PRIMITIVE name (`u8`/`u16`/…/`s64`/`bool`/`char`/`string`/`f32`/`f64`) -> `(name)`
    /// - `list(T)` / `List(T)` -> `("list" <T>)`
    /// - `option(T)` / `Option(T)` -> `("option" <T>)`
    ///
    /// - `tuple(A, …)` / `Tuple(A, …)` -> `("tuple" <A> …)`
    /// - a record TYPE `{f: T, …}` / `Record(f: T, …)` (canonical `(Record (: f T)…)`) -> `("record" (f
    ///   <T>)…)` via `Builder::wit_type_record`, matching rcdzc's `parse_wit_type` record arm.
    /// - `result(T, E)` / `result(T)` / `result(_, E)` / `result` -> `("result" <ok> <err>)` (an absent arm,
    ///   spelled `_`, is the `("none")` marker) via `Builder::wit_type_result`.
    /// - `variant(Case, Case2(T), …)` -> `("variant" (Case <T>?)…)` via `Builder::wit_type_variant`.
    /// - `enum(A, …)` -> `("enum" A …)` and `flags(A, …)` -> `("flags" A …)` (bare-NAME cases/bits) via
    ///   `Builder::wit_type_enum`/`wit_type_flags`.
    ///
    /// Any other type node is left AS-IS (still round-trips). The printer's `print_wit_type` is the inverse.
    fn wit_type_desc_of(&mut self, ty: StructId) -> StructId {
        // A bare NAME atom: `unit`/`Unit` -> the str-head `("unit")`; a WIT primitive -> `(name)`; any
        // other bare name is left as-is. (Clone the name so the read borrow releases before the mutable
        // builder call.)
        if let Some(name) = self.builder.as_name(ty).map(str::to_string) {
            return match name.as_str() {
                "unit" | "Unit" => self.builder.wit_type_unit(),
                // A bare `result` (no arms) — WIT's `result` with neither an ok nor an err type.
                "result" | "Result" => self.builder.wit_type_result(None, None),
                n if is_wit_primitive(n) => self.builder.wit_type_prim(n),
                _ => ty,
            };
        }
        // A `(head arg…)` application. Extract (head-name, args) WITHOUT holding the `get` borrow across
        // the mutable builder calls. `list`/`option` take one arg; `tuple` is variable-arity; a record TYPE
        // `(Record (: f T)…)` is variable-arity down to the empty `(Record)` (`{}`).
        let app = match self.builder.get(ty) {
            crate::ast::Struct::List(kids) if !kids.is_empty() => {
                let head = kids[0];
                let args: Vec<StructId> = kids[1..].to_vec();
                self.builder.as_name(head).map(|h| (h.to_string(), args))
            }
            _ => None,
        };
        match app {
            Some((h, args)) if (h == "list" || h == "List") && args.len() == 1 => {
                let elem = self.wit_type_desc_of(args[0]);
                self.builder.wit_type_list(elem)
            }
            Some((h, args)) if (h == "option" || h == "Option") && args.len() == 1 => {
                let inner = self.wit_type_desc_of(args[0]);
                self.builder.wit_type_option(inner)
            }
            Some((h, args)) if h == "tuple" || h == "Tuple" => {
                let elems: Vec<StructId> = args.iter().map(|&a| self.wit_type_desc_of(a)).collect();
                self.builder.wit_type_tuple(&elems)
            }
            Some((h, args)) if h == "record" || h == "Record" => {
                self.wit_type_record_desc_of(&args).unwrap_or(ty)
            }
            // `result(T, E)` / `result(T)` / `result(_, E)` — a WIT result with 0-2 arms. Each arm is a type
            // descriptor, or the WIT wildcard `_` spelling an ABSENT arm (`result<_, E>` / `result<T>`).
            Some((h, args)) if h == "result" || h == "Result" => {
                // A WIT result has at most two arms; more args is a malformed spelling — steer with a
                // message rather than silently leaving a broken member type (the reject-with-guidance
                // policy the record-field form already uses).
                if args.len() > 2 {
                    self.error(
                        "a `result` type takes at most two arguments — `result(Ok, Err)`, `result(Ok)`, \
                         `result(_, Err)`, or bare `result`",
                    );
                    return ty;
                }
                let ok = if args.is_empty() {
                    None
                } else {
                    self.wit_result_arm_of(args[0])
                };
                let err = if args.len() < 2 {
                    None
                } else {
                    self.wit_result_arm_of(args[1])
                };
                self.builder.wit_type_result(ok, err)
            }
            // `variant(Case, Case2(T), …)` — an anonymous variant type. Each case is a bare NAME
            // (payload-less) or a single-arg application `Case2(T)` (a payload case whose type is lowered).
            Some((h, args)) if h == "variant" || h == "Variant" => {
                match self.wit_type_variant_desc_of(&args) {
                    Some(v) => v,
                    None => {
                        self.error(
                            "a `variant` case is a bare name (payload-less) or `Case(T)` with a single \
                             payload type — for several fields use `Case(tuple(A, B))` or a record",
                        );
                        ty
                    }
                }
            }
            // `enum(A, B, …)` / `flags(A, B, …)` — a set of bare-NAME cases/bits. Both share the shape; the
            // head keyword selects the (distinct) WIT type. A non-name arg means it is not this spelling.
            Some((h, args)) if h == "enum" || h == "Enum" => match self.wit_case_names_of(&args) {
                Some(names) => {
                    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
                    self.builder.wit_type_enum(&refs)
                }
                None => {
                    self.error("an `enum` case is a bare name, e.g. `enum(Red, Green, Blue)`");
                    ty
                }
            },
            Some((h, args)) if h == "flags" || h == "Flags" => {
                match self.wit_case_names_of(&args) {
                    Some(names) => {
                        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
                        self.builder.wit_type_flags(&refs)
                    }
                    None => {
                        self.error("a `flags` bit is a bare name, e.g. `flags(Read, Write)`");
                        ty
                    }
                }
            }
            _ => ty,
        }
    }

    /// Collect the bare-NAME arguments of an `enum(…)` / `flags(…)` type application, in order. `None` if
    /// any arg is not a bare name — then it is not a well-formed enum/flags spelling and the caller leaves
    /// the node as-is.
    fn wit_case_names_of(&self, args: &[StructId]) -> Option<Vec<String>> {
        args.iter()
            .map(|&a| self.builder.as_name(a).map(str::to_string))
            .collect()
    }

    /// Lower a variant-TYPE node's case args to the WIT variant descriptor `("variant" (Case <T>?)…)` via
    /// `Builder::wit_type_variant`. A case arg is a bare NAME (payload-less → `(Case)`) or a single-arg
    /// application `Case(T)` (payload → `(Case <T>)`, the type lowered). `None` if any arg is neither shape
    /// (the caller then leaves the node as-is). Reads each case's (name, optional raw-payload-id) FIRST —
    /// releasing the arena borrow — before the recursive lower, since that lower mutates the builder.
    fn wit_type_variant_desc_of(&mut self, args: &[StructId]) -> Option<StructId> {
        let mut raw: Vec<(String, Option<StructId>)> = Vec::with_capacity(args.len());
        for &arg in args {
            if let Some(n) = self.builder.as_name(arg) {
                raw.push((n.to_string(), None));
                continue;
            }
            let case = match self.builder.get(arg) {
                crate::ast::Struct::List(k) if k.len() == 2 => {
                    self.builder.as_name(k[0]).map(|n| (n.to_string(), k[1]))
                }
                _ => None,
            };
            match case {
                Some((n, ty)) => raw.push((n, Some(ty))),
                None => return None,
            }
        }
        let lowered: Vec<(String, Option<StructId>)> = raw
            .into_iter()
            .map(|(n, ty)| {
                let d = ty.map(|t| self.wit_type_desc_of(t));
                (n, d)
            })
            .collect();
        let cases: Vec<(&str, Option<StructId>)> =
            lowered.iter().map(|(n, t)| (n.as_str(), *t)).collect();
        Some(self.builder.wit_type_variant(&cases))
    }

    /// Lower one arm of a `result(…)` type-application: the WIT wildcard `_` spells an ABSENT arm (→ `None`,
    /// which `Builder::wit_type_result` renders as the `("none")` marker); any other node is a present type,
    /// lowered via [`Self::wit_type_desc_of`].
    fn wit_result_arm_of(&mut self, arg: StructId) -> Option<StructId> {
        if self.builder.as_name(arg) == Some("_") {
            return None;
        }
        Some(self.wit_type_desc_of(arg))
    }

    /// Lower a record-TYPE node's field args — each a canonical `(: fname T)` ascription — to the WIT
    /// record descriptor `("record" (fname <T>)…)` via `Builder::wit_type_record`. `None` if any arg is not
    /// a well-formed `(: name T)` field (the caller then leaves the node as-is). Reads each field's (name,
    /// raw-type-id) FIRST — releasing the arena borrow — before the recursive lower, since that lower
    /// mutates the builder.
    fn wit_type_record_desc_of(&mut self, args: &[StructId]) -> Option<StructId> {
        let mut raw: Vec<(String, StructId)> = Vec::with_capacity(args.len());
        for &arg in args {
            let field = match self.builder.get(arg) {
                crate::ast::Struct::List(k)
                    if k.len() == 3 && self.builder.as_name(k[0]) == Some(":") =>
                {
                    self.builder.as_name(k[1]).map(|n| (n.to_string(), k[2]))
                }
                _ => None,
            };
            raw.push(field?);
        }
        let lowered: Vec<(String, StructId)> = raw
            .into_iter()
            .map(|(n, tid)| {
                let d = self.wit_type_desc_of(tid);
                (n, d)
            })
            .collect();
        let fields: Vec<(&str, StructId)> = lowered.iter().map(|(n, t)| (n.as_str(), *t)).collect();
        Some(self.builder.wit_type_record(&fields))
    }

    fn world_member(&mut self) -> StructId {
        let start = self.cur_span();
        let member_head = self.name("member", start);
        let member_name = self.binder();
        self.expect(Kind::Colon, "`:`");
        // Params: an optional `( name : T , … )` list. An elided list (a leading `->`) is a nullary func.
        let mut func_items = vec![self.name("func", self.cur_span())];
        if self.at(Kind::LParen) {
            self.bump(); // `(`
            if !self.at(Kind::RParen) {
                loop {
                    let before = self.pos;
                    let p_start = self.cur_span();
                    let param_head = self.name("param", p_start);
                    let p_name = self.binder();
                    self.expect(Kind::Colon, "`:`");
                    let p_ty_raw = self.type_ref();
                    let p_ty = self.wit_type_desc_of(p_ty_raw);
                    let p_span = p_start.merge(self.prev_span());
                    func_items.push(self.list(vec![param_head, p_name, p_ty], p_span));
                    if !self.sep_continue(Kind::RParen) {
                        break;
                    }
                    if self.pos == before {
                        self.bump(); // never-loop guard on a malformed param
                    }
                }
            }
            self.expect(Kind::RParen, "`)`");
        }
        // Result: `-> R` (required; the arrow separates params from the result type).
        self.expect(Kind::Arrow, "`->`");
        let result_start = self.cur_span();
        let result_ty_raw = self.type_ref();
        let result_ty = self.wit_type_desc_of(result_ty_raw);
        let result_head = self.name("result", result_start);
        let result = self.list(
            vec![result_head, result_ty],
            result_start.merge(self.prev_span()),
        );
        func_items.push(result);
        let func_span = start.merge(self.prev_span());
        let func = self.list(func_items, func_span);
        let span = start.merge(self.prev_span());
        self.list(vec![member_head, member_name, func], span)
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

    /// Iterative-reader helper: after a handle's seed, consume `with` + an optional leading `|`, and return
    /// the arms' start span (`cur_span` after the `|`). Mirrors the `with` head of `handle_expr`.
    fn handle_after_seed(&mut self) -> Span {
        self.expect_keyword(Keyword::With, "`with`");
        if self.at(Kind::Pipe) {
            self.bump(); // optional leading `|`
        }
        self.cur_span()
    }

    /// Iterative-reader helper (the head of `handle_arm`, minus the descending body expr): read one handle
    /// arm's `op(binder…, state) =>` header. Returns `(arm_start, op, params, state, saved_arm_bar)` — the
    /// caller then descends the body (`expr(0)`) with `arm_bar_terminates` forced true (set here), folding
    /// `(op params state body)` on deliver + restoring the flag via `saved_arm_bar`.
    fn handle_arm_header(&mut self) -> (Span, StructId, StructId, StructId, bool) {
        let arm_start = self.cur_span();
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
        (arm_start, op, params, state, saved)
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

    /// Two import surfaces, both ending `from "path"`, disambiguated by the SPEC after `import`:
    ///   * `import { name, … } from "path"` -> `(import "path" (name …))` — brings a sibling module's
    ///     named exports FLAT into scope (third arena element a name-LIST);
    ///   * `import alias from "path"` -> `(import "path" alias)` — binds the WHOLE module under a local
    ///     `alias` (a record of its exports), reached by projection `alias.member` (third arena element
    ///     a bare NAME). The `alias` avoids a collision when two modules export the same name.
    ///
    /// The arena is the corpus's path-first shape in both cases (a path string then either a name-list
    /// or a bare-name alias), so the surfaces agree with the sexpr surface and the linker's discriminant
    /// (list-third-element = named imports; atom-third-element = module alias). Both reuse the `from`
    /// keyword — a whole-module bind is just a BARE name where the named form has a `{ … }` list.
    fn import_expr(&mut self) -> StructId {
        let start = self.cur_span();
        let head = self.keyword_head("import", start);
        self.bump(); // `import`
        // The import SPEC: a brace name-list `{ a, b }` (named imports) OR a bare NAME (whole-module
        // alias). Both then take `from "path"`; the `{` vs bare-name opener is the sole discriminant.
        let spec = if self.at(Kind::LBrace) {
            let names_start = self.cur_span();
            let names = self.brace_name_list();
            let names_span = names_start.merge(self.prev_span());
            self.list(names, names_span)
        } else {
            self.binder()
        };
        // `from` is a CONTEXTUAL keyword — an ordinary identifier `from` in this one position, not a
        // globally-reserved word (so `from` stays usable as a variable name elsewhere).
        if self.at(Kind::Ident) && self.cur_text() == "from" {
            self.bump();
        } else {
            self.error("expected `from` after the import spec");
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
        // Arena order is path-first: `(import "path" <spec>)` (spec = name-LIST or bare-NAME alias).
        self.list(vec![head, path, spec], span)
    }

    /// A brace-delimited comma-separated name list `{ a, b, … }` -> the vector of name occurrences.
    /// Used by `import`. Each element is a bare (or backtick-escaped) name; a non-name element records
    /// an error and is skipped, so a malformed list still terminates.
    fn brace_name_list(&mut self) -> Vec<StructId> {
        // Import allows per-name renames (`{ orig as alias }`) but not `.member` postfixes.
        self.brace_list_of(false, true)
    }

    /// The `export { … }` list — a name list where each element MAY carry a member-access postfix
    /// `.A` / `.*` (a constructor-export element `(. T A)` / the wildcard `(. T *)`), since an export
    /// publishes a value/handle name OR a type's constructor(s). Import stays name-only (a member has
    /// no meaning there).
    fn brace_export_list(&mut self) -> Vec<StructId> {
        // Export allows `.member` postfixes (`{ Color.* }`) but not per-name renames.
        self.brace_list_of(true, false)
    }

    /// The shared brace-list machinery. `members` = whether an element may carry a `.member` postfix
    /// (`export` yes, `import` no) — when set, each binder runs through `postfix` so `Color.*` /
    /// `Color.Red` parse to the `(. Color …)` member form.
    fn brace_list_of(&mut self, members: bool, renames: bool) -> Vec<StructId> {
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
                // Per-name RENAME `orig as alias` (import only) -> `(as orig alias)`: bind the module's
                // export `orig` under the local name `alias`. Marker-headed (`as`) so the linker tells a
                // renamed element from a bare-name plain import (atom = plain, `as`-list = rename).
                if renames && self.at_keyword(Keyword::As) {
                    self.bump(); // `as`
                    let as_kw = self.name("as", elem_span);
                    let alias = self.binder();
                    let span = elem_span.merge(self.prev_span());
                    elem = self.list(vec![as_kw, elem, alias], span);
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
        // Desugar any `forall` binder in a parameter annotation into leading `(: a Type)` params.
        let params = self.hoist_forall_params(params, params_span);
        self.list(params, params_span)
    }

    /// DESUGAR the `forall` binder in a signature's parameter annotations into leading type-valued
    /// parameters — the arena-canonical route agreed with v-inference: a `forall a.` is PURE SUGAR for a
    /// `(: a Type)` parameter (the pinned "generics are type-valued parameters" model), so it is lowered
    /// HERE at parse time and infer NEVER sees a `(forall …)` node (it sees the exact `(: a Type)` arena a
    /// hand-written generic produces — no new ∀ engine, no infer change).
    ///
    /// For each parameter `(: x (forall (a b) T))`, prepend one `(: a Type)` / `(: b Type)` binder per
    /// forall name to the signature (source order), and rewrite the parameter itself to `(: x T)` (the
    /// bare inner type). Multiple parameters may each carry a `forall`; the prepended type-params
    /// accumulate in encounter order. A parameter with no forall passes through unchanged. Like the
    /// brace-record sugar, this is INPUT-ONLY: the printer re-emits the desugared `(: a Type)` form
    /// (`a: Type`), not `forall` — one canonical arena, both surfaces agree.
    ///
    /// `params` are the already-built parameter nodes; `span` is the signature's span (used for the
    /// synthesized `Type` binder nodes, which have no independent source span).
    fn hoist_forall_params(&mut self, params: Vec<StructId>, span: Span) -> Vec<StructId> {
        // Fast path: if no parameter's annotation carries a `forall`, return unchanged (the common case).
        if !params
            .iter()
            .any(|&p| self.param_forall_binders(p).is_some())
        {
            return params;
        }
        let mut out = Vec::with_capacity(params.len());
        for p in params {
            match self.param_forall_binders(p) {
                Some((binder, binder_names, body)) => {
                    // Prepend a `(: name Type)` param per forall binder (source order).
                    for name_text in binder_names {
                        let colon = self.name(":", span);
                        let nm = self.name(name_text, span);
                        let type_kw = self.name("Type", span);
                        out.push(self.list(vec![colon, nm, type_kw], span));
                    }
                    // Rewrite the parameter to `(: binder BODY)` — the forall stripped, bare inner type.
                    let colon = self.name(":", span);
                    out.push(self.list(vec![colon, binder, body], span));
                }
                None => out.push(p),
            }
        }
        out
    }

    /// If the built parameter `p` is `(: binder (forall (a b) T))`, return `(binder, [a, b], T)` — the
    /// binder node, the forall type-variable NAMES (owned, since we re-synthesize the atoms), and the
    /// inner type body node. `None` for any other parameter shape (a plain binder, a non-forall
    /// annotation, or a `forall` with a malformed/empty binder list).
    /// Whether `arg` is the obsolete head-application record-TYPE field spelling `field(T)` — a bare
    /// two-element application `(name Type)` whose head is a NAME (not the `:` ascription head). The
    /// canonical field is `(: name T)` (a three-element `:`-headed list), which this rejects; a positional
    /// type argument that is itself an application `(F X)` where `F` is a type constructor is NOT a record
    /// field (a record type takes no positional args), so this is only ever called on `Record(…)` args.
    /// Guards on exactly two children with a name head — a nested `(: …)`, a 3+-element app, or a
    /// bare-type arg is not flagged.
    fn is_head_app_record_field(&self, arg: StructId) -> bool {
        match self.builder.get(arg) {
            crate::ast::Struct::List(children) if children.len() == 2 => {
                // `(name Type)` — a name head that is NOT the `:` ascription marker. `(: name T)` has three
                // children, so it never matches the len-2 guard; a `(List a)` positional arg would only
                // reach here if it were a `Record` arg, which is itself the malformed case.
                self.builder.as_name(children[0]).is_some_and(|h| h != ":")
            }
            _ => false,
        }
    }

    fn param_forall_binders(&self, p: StructId) -> Option<(StructId, Vec<String>, StructId)> {
        // p == (: binder ANNOT)
        let ann = self.builder.as_form(p, ":")?;
        let [binder, annot] = *ann else { return None };
        // annot == (forall (a b) T)
        let forall = self.builder.as_form(annot, "forall")?;
        let [binder_list, body] = *forall else {
            return None;
        };
        // binder_list == (a b …) — one-or-more Name atoms.
        let names: Vec<String> = match self.builder.get(binder_list) {
            crate::ast::Struct::List(bs) if !bs.is_empty() => {
                let mut v = Vec::with_capacity(bs.len());
                for &b in bs {
                    v.push(self.builder.as_name(b)?.to_string());
                }
                v
            }
            _ => return None,
        };
        Some((binder, names, body))
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
        // Own-line `//` comment(s) leading the FIRST arm (`match x with\n  // note\n  | 0 => …`) sit in
        // the leading `|`'s slot (or the pattern's, if no leading `|`). Drain BEFORE consuming the `|` —
        // `take_comments_here` at the loop top would run AFTER the `|` bump and miss them.
        let mut pending_leading = self.take_comments_here();
        if self.at(Kind::Pipe) {
            self.bump(); // optional leading `|`
        }
        loop {
            // Wrap the arm in any own-line LEADING comment(s) drained before its `|` (the match printer
            // renders them on their own line above the arm; `is_match_shape` unwraps via
            // `strip_field_comments`). Own-line has no swallow hazard. A comment drained at the `|` slot
            // AFTER the pattern is the FIRST-arm case; subsequent arms drain at the end-of-loop `|` below.
            let leading = std::mem::take(&mut pending_leading);
            let arm = self.match_arm();
            let arm = self.wrap_comments(leading, arm);
            // A `//` trailing this arm on its line (`| pat => body  // note`) sits at the next token's
            // leading slot (the `|` or the match's end), which the arm loop never drains → dropped.
            // Attach it to THIS arm as `(comment-after …)` so it re-prints same-line. `strip_comments`
            // peels it, so the match compiler is unaffected. (Mirrors the sum-variant locus.)
            let trailing = self.take_trailing_comment_here();
            items.push(self.wrap_comment_after(trailing, arm));
            // Own-line comment(s) leading the NEXT arm sit in the upcoming `|`'s slot — drain them BEFORE
            // the bump (into `pending_leading` for the next iteration). `take_trailing_comment_here` above
            // already took any SAME-LINE trailing comment (a `l.trailing` lead); what remains here is an
            // OWN-LINE comment belonging to the next arm.
            pending_leading = self.take_comments_here();
            if self.at(Kind::Pipe) {
                self.bump(); // `|` before the next arm
            } else {
                // No more arms. Any own-line comment(s) we just drained do NOT lead a (nonexistent) next
                // arm — they lead whatever FOLLOWS the match (e.g. the next top-level `def`, or a
                // `// ---- SECTION` header between defs). Draining them here without an arm to attach to
                // would DROP them (`cdz fmt` refuses — the seq-277 reader-attachment gap). Restore them to
                // the current token's leading slot so the enclosing parser (the next `stmt`/element) picks
                // them up. (At EOF the run is empty — a trailing comment is in `self.trailing`, handled by
                // `program`.)
                if !pending_leading.is_empty() && self.pos < self.leading.len() {
                    let mut restored = std::mem::take(&mut pending_leading);
                    restored.append(&mut self.leading[self.pos]);
                    self.leading[self.pos] = restored;
                }
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
        // An own-line `//` comment leading the arm BODY (`| A() =><newline> // note<newline> body`) sits
        // at the body's first-token leading slot, which `expr` does not drain — capture + wrap
        // `(comment "text" body)` so it prints own-line above the body (round-trips; else stranded →
        // `cdz fmt` refuses). Same own-line leading-comment capture as the arm's own leading comment.
        let body_lead = self.take_comments_here();
        let saved = self.arm_bar_terminates;
        self.arm_bar_terminates = true;
        let body = self.expr(0);
        self.arm_bar_terminates = saved;
        let body = self.wrap_comments(body_lead, body);
        let span = start.merge(self.prev_span());
        self.list(vec![pat, body], span)
    }

    /// Iterative-reader helper (the head of `match_arm`, minus the descending guard/body exprs): read an
    /// arm's pattern + optional `if`-guard opener. Returns `(arm_start, pat, guard)`, where `guard` is
    /// `Some((guard_head, g_start))` iff an `if` guard is present (its `if` already consumed) — the caller
    /// then descends the guard expr on the worklist and folds `(guard pat g)` on deliver. `pattern()` is
    /// read inline (its own separate depth guard; pattern de-recursion is I4).
    fn match_arm_pat(&mut self) -> (Span, StructId, Option<(StructId, Span)>) {
        let arm_start = self.cur_span();
        let pat = self.pattern();
        if self.at_keyword(Keyword::If) {
            let g_start = self.cur_span();
            let guard_head = self.keyword_head("guard", g_start);
            self.bump(); // `if`
            (arm_start, pat, Some((guard_head, g_start)))
        } else {
            (arm_start, pat, None)
        }
    }

    /// Iterative-reader helper (the body head of `match_arm`): consume `=>`, drain the body's own-line
    /// leading comments, and force `arm_bar_terminates = true` for the upcoming body descent. Returns
    /// `(body_lead, saved_arm_bar)` so the caller restores the flag once the body delivers. Mirrors the
    /// `expect(=>)` + `arm_bar` dance in `match_arm`.
    fn match_arm_body_preamble(&mut self) -> (Vec<Lead>, bool) {
        self.expect(Kind::FatArrow, "`=>`");
        let body_lead = self.take_comments_here();
        let saved = self.arm_bar_terminates;
        self.arm_bar_terminates = true;
        (body_lead, saved)
    }

    // ---- structural pattern grammar ----

    /// A structural pattern occurrence. A pattern's tree is a plain `(head child…)` form (the same
    /// shape the pattern printer emits): a head atom — a literal, a binding/wildcard name, a
    /// backtick name, or a grouped sub-pattern — followed by an optional `.member` chain and/or a
    /// `(sub-pattern, …)` application, left-nested. It is never an infix expression. This mirrors
    /// the printer exactly, so constructor patterns (`Some(x)`), dotted constructors (`Sign.Neg`),
    /// literal-headed forms (`1(v)`), and quoted patterns (`quasiquote(…)`) all parse uniformly.
    fn pattern(&mut self) -> StructId {
        // I4: route to the iterative pattern driver when `read_ml` set the flag; the recursive body below
        // stays for `read_ml_recursive` (the frozen oracle reference). Incremental — the iterative driver
        // de-recurses one pattern family at a time, staying byte-identical (differential oracle) each step.
        if self.iterative {
            return self.pattern_iter();
        }
        let start = self.cur_span();
        // DEPTH GUARD: patterns recurse (a tuple/list/ctor sub-pattern re-enters `pattern`) on a path
        // ENTIRELY separate from `expr`, so `expr`'s guard never covers them — a pathologically deep
        // pattern (`((((…` / `[[[[…` / `C(C(C(…`) overflowed the native stack (SIGABRT). Count each
        // pattern level against the shared depth budget via `guard_prefix` (clean diagnostic). The single
        // `node` exit below decrements to keep the budget balanced.
        if let Some(err) = self.guard_prefix(start) {
            return err;
        }
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
                    // M2: `.` is a native Member leaf head (kind identity), matching `member_access` /
                    // `member_head` and the s-expr reader's `memberize` — a dotted CONSTRUCTOR pattern
                    // (`Sign.Neg`, `Id.Mk(n)`) round-trips against the same Member head every other surface
                    // produces (was `self.name(".")`, which mismatched the reader's `Leaf::Member`).
                    let dot = self.atom(Leaf::Member, dot_span);
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
        self.depth -= 1;
        node
    }

    /// Iterative pattern reader (I4) — the explicit-worklist replacement for the recursive [`Self::pattern`]
    /// (byte-identical, verified by the differential oracle). HYBRID STAGE: the head atom is read via the
    /// (recursive) [`Self::pattern_atom`] — so a `(tuple)`/`[list]`/`#{map}`/etc. sub-pattern still recurses
    /// through a nested `pattern_iter` for now — but the postfix chain (`.member` folded inline, `(args)`
    /// applications) is de-recursed onto this worklist, so a `C(C(C(… )))` constructor-application nest no
    /// longer grows the native stack. Depth accounting mirrors `pattern`'s `guard_prefix` exactly (one
    /// budget unit per pattern LEVEL — atom + its whole postfix chain — decremented when the level and all
    /// its postfix complete), so a pathological nest declines at the same point/shape. Remaining atom
    /// families (tuple/list/map/set/record/bin/raw-list) convert onto the worklist in later increments.
    /// Iterative-reader helper: advance a `{ … }` record PATTERN, appending completed SHORTHAND fields
    /// (`{ x }` -> `(= x x)`, read inline — no sub-pattern) to `items` until either the body CLOSES (the
    /// `}` is consumed here) -> `None`, or a field needs a sub-pattern DESCENT (a `.. rest` operand or a
    /// `field = <pat>` value) -> `Some(descend)` for `Cont::RecordPat` to push + read on the worklist.
    /// Mirrors the recursive `{`-pattern arm of `pattern_atom` (rest checked first, then a field-pair with
    /// shorthand pun); `rest_marker`-free non-rest fields keep the same struct-id order.
    fn advance_record_pat(&mut self, items: &mut Vec<StructId>) -> Option<RecordPatDescend> {
        loop {
            let before = self.pos;
            if self.at(Kind::DotDot) {
                let dd = self.cur_span();
                self.bump(); // `..`
                let rest_head = self.name("..", dd);
                return Some(RecordPatDescend::Rest {
                    dd_span: dd,
                    rest_head,
                    before,
                });
            }
            let f_start = self.cur_span();
            // Capture the field spelling BEFORE `binder()` consumes it (a shorthand puns it).
            let pun = self.binder_spelling();
            let field = self.binder();
            if self.at(Kind::Eq) {
                self.bump(); // `=`
                return Some(RecordPatDescend::Value {
                    f_start,
                    field,
                    before,
                });
            }
            if let Some(n) = pun {
                // Shorthand `{ x }` -> `(= x x)`: the field binds a same-named binder — no sub-pattern.
                let value = self.name(n, f_start);
                let eq = self.atom(Leaf::FieldPair, f_start);
                let f_span = f_start.merge(self.prev_span());
                items.push(self.list(vec![eq, field, value], f_span));
                if !self.sep_continue(Kind::RBrace) {
                    self.expect(Kind::RBrace, "`}`");
                    return None;
                }
                if self.pos == before {
                    self.bump(); // no field token consumed — avoid a missing-`,` spin
                }
                continue; // next field, inline
            }
            // A non-name field with no `=` — record the missing `=` (as the recursive arm does), descend.
            self.expect(Kind::Eq, "`=`");
            return Some(RecordPatDescend::Value {
                f_start,
                field,
                before,
            });
        }
    }

    /// Iterative-reader helper: read the preamble of ONE `#{ … }` map PATTERN entry — a `.. rest` spread
    /// (descend its operand) or a `key = <pat>` entry (the KEY is a value `expr(0)` read INLINE — a bounded
    /// nested `expr_iter` call — then `=`, then the value sub-pattern DESCENDS). Returns the descend for
    /// `Cont::RecordPat` (reused: its `field` slot carries the map KEY, its `f_start` the entry start, so
    /// the same `(= key value)` `FieldPair` triple is built on deliver). No shorthand (a map has none), so
    /// this always returns `Some` (the CLOSE is checked by the caller before calling). Mirrors the recursive
    /// `#{`-pattern arm; `before` is the entry-loop progress guard.
    fn advance_map_pat(&mut self) -> RecordPatDescend {
        let before = self.pos;
        if self.at(Kind::DotDot) {
            let dd = self.cur_span();
            self.bump(); // `..`
            let rest_head = self.name("..", dd);
            return RecordPatDescend::Rest {
                dd_span: dd,
                rest_head,
                before,
            };
        }
        let e_start = self.cur_span();
        let key = self.expr(0);
        self.expect(Kind::Eq, "`=`");
        RecordPatDescend::Value {
            f_start: e_start,
            field: key,
            before,
        }
    }

    fn pattern_iter(&mut self) -> StructId {
        // A pattern postfix `( arg, … )` application awaiting an argument sub-pattern. `items` = `[ base,
        // arg… ]`; `start` is the base pattern's start (the folded node's span); `entered` = whether the
        // OWNING level incremented `self.depth` (so the balanced decrement fires once, when the whole
        // pattern completes); `before` is the arg-loop missing-`,` progress guard.
        enum PCont {
            Args {
                start: Span,
                entered: bool,
                items: Vec<StructId>,
                before: usize,
            },
            // A comma-list pattern atom awaiting an element sub-pattern — the shared skeleton for `[ … ]`
            // list (head `list`, closer `]`, rest), `#( … )` set (head `#set` ctor, closer `)`, rest),
            // `#[ … ]` raw-list (NO head, closer `]`, NO rest — a bare `(p …)`), and `b[ … ]` bin (head
            // `bin` NAME, closer `]`, NO rest, WITH comment slots — the only pattern comma-list carrying
            // own-line-leading + last-segment-trailing comments). `open_span` is the opener's span;
            // `closer`/`allow_rest`/`allow_comments` select the family; `leading` holds the current segment's
            // own-line leading comments (bin only, captured before the descent); `items` the (optional) head
            // + elements; `lvl_*` the owning level (resume postfix + balance depth on close).
            List {
                lvl_start: Span,
                lvl_entered: bool,
                open_span: Span,
                closer: Kind,
                allow_rest: bool,
                allow_comments: bool,
                leading: Vec<Lead>,
                items: Vec<StructId>,
                is_rest: bool,
                rest_head: StructId,
                dd_span: Span,
                before: usize,
            },
            // A `( … )` pattern awaiting a sub-pattern (an ATOM-position form). Like the expr `Cont::Paren`:
            // `items` empty while the FIRST sub-pattern decides grouping `(p)` vs tuple `(p, …)`; once tuple
            // mode is entered it holds `[ "tuple"-head, first, … ]`. A subsequent element may be a `.. rest`
            // spread (`is_rest`). `paren_span` is the `(`'s span; `lvl_*` the owning level (resume postfix +
            // balance depth on close).
            Tuple {
                lvl_start: Span,
                lvl_entered: bool,
                paren_span: Span,
                items: Vec<StructId>,
                is_rest: bool,
                rest_head: StructId,
                dd_span: Span,
                before: usize,
            },
            // A `{ field = p, … }` record pattern awaiting a field's VALUE sub-pattern or a `.. rest`
            // operand (the field-pair analogue of `List`; SHORTHAND fields complete inline via
            // `advance_record_pat`). On value deliver: build `(= field value)`; on rest deliver: `(.. binder)`.
            // `is_rest` selects; `rest_head`/`dd_span` are the rest node's; `f_start`/`field` the value
            // field's binder (for the `(= field value)` triple built on deliver). `brace_span` is the `{`'s;
            // `lvl_*` the owning level (resume postfix + balance depth on close).
            RecordPat {
                lvl_start: Span,
                lvl_entered: bool,
                brace_span: Span,
                is_map: bool, // `#{`-map (advance via `advance_map_pat`, key=expr) vs `{`-record (shorthand)
                items: Vec<StructId>,
                is_rest: bool,
                rest_head: StructId,
                dd_span: Span,
                f_start: Span,
                field: StructId,
                before: usize,
            },
        }
        let mut pending: Vec<PCont> = Vec::new();
        let mut node: StructId = StructId(0); // placeholder; assigned before use (reading=true first)
        let mut cur_start = self.cur_span();
        let mut cur_entered = false;
        // `reading`: begin a FRESH pattern level (guard + head atom); else `node` holds a completed level
        // (either the top pattern, or an argument delivered to a suspended `PCont::Args`).
        let mut reading = true;
        loop {
            if reading {
                cur_start = self.cur_span();
                // DEPTH GUARD (mirrors `pattern`'s `guard_prefix`): on trip the level's value is a bare
                // `error_node` with NO atom/postfix + NO depth increment (as `pattern` early-returns).
                match self.guard_prefix(cur_start) {
                    Some(err) => {
                        node = err;
                        cur_entered = false;
                        reading = false;
                    }
                    None => {
                        cur_entered = true;
                        // ATOM DISPATCH: the `[ … ]` list pattern descends its elements on the worklist
                        // (so `[[[[…` de-recurses); every OTHER atom family falls back to the (recursive)
                        // `pattern_atom` for now (converted in later increments — byte-identical meanwhile).
                        if self.at(Kind::LParen) {
                            let paren_span = cur_start;
                            self.bump(); // '('
                            if self.at(Kind::RParen) {
                                let s = self.cur_span();
                                self.bump();
                                node = self.name("unit", s); // `()` -> unit
                                reading = false; // -> postfix phase
                            } else {
                                // Descend the FIRST sub-pattern (grouping-vs-tuple decided on deliver; the
                                // first element is never a `.. rest`). `items` empty = deciding mode.
                                pending.push(PCont::Tuple {
                                    lvl_start: cur_start,
                                    lvl_entered: cur_entered,
                                    paren_span,
                                    items: Vec::new(),
                                    is_rest: false,
                                    rest_head: StructId(0),
                                    dd_span: paren_span,
                                    before: 0,
                                });
                                continue; // reading stays true: read the first sub-pattern
                            }
                        } else if self.at(Kind::LBracket)
                            || self.at(Kind::BinOpen)
                            || (self.kind() == Kind::Hash
                                && matches!(self.nth_kind(1), Kind::LBracket | Kind::LParen))
                        {
                            // `[ … ]` list / `#[ … ]` raw-list / `#( … )` set / `b[ … ]` bin — the comma-list
                            // pattern atoms (mirror `pattern_atom`'s `[`/`#[`/`#(`/`b[` arms): (optional) head
                            // created BEFORE elements, then sub-patterns read on the worklist. Family selects
                            // head + closer + rest + comment slots (bin only).
                            let open_span = cur_start;
                            let (items, closer, allow_rest, allow_comments) =
                                if self.kind() == Kind::LBracket {
                                    self.bump(); // '['
                                    (
                                        vec![self.name("list", open_span)],
                                        Kind::RBracket,
                                        true,
                                        false,
                                    )
                                } else if self.kind() == Kind::BinOpen {
                                    self.bump(); // `b[`
                                    (
                                        vec![self.name("bin", open_span)],
                                        Kind::RBracket,
                                        false,
                                        true,
                                    )
                                } else if self.nth_kind(1) == Kind::LBracket {
                                    self.bump(); // '#'
                                    self.bump(); // '['
                                    (Vec::new(), Kind::RBracket, false, false) // raw-list: NO head/rest/cmnt
                                } else {
                                    self.bump(); // '#'
                                    self.bump(); // '('
                                    (
                                        vec![self.ctor_head("set", open_span)],
                                        Kind::RParen,
                                        true,
                                        false,
                                    )
                                };
                            if self.at(closer) {
                                self.expect(closer, "comma-list pattern closer");
                                node = self.list(items, open_span.merge(self.prev_span()));
                                reading = false; // -> postfix phase on the assembled node
                            } else {
                                let before = self.pos;
                                let (is_rest, dd_span, rest_head) =
                                    if allow_rest && self.at(Kind::DotDot) {
                                        let dd = self.cur_span();
                                        self.bump(); // `..`
                                        (true, dd, self.name("..", dd))
                                    } else {
                                        (false, open_span, StructId(0))
                                    };
                                // bin captures own-line leading comments before the (non-rest) segment.
                                let leading = if allow_comments && !is_rest {
                                    self.take_comments_here()
                                } else {
                                    Vec::new()
                                };
                                pending.push(PCont::List {
                                    lvl_start: cur_start,
                                    lvl_entered: cur_entered,
                                    open_span,
                                    closer,
                                    allow_rest,
                                    allow_comments,
                                    leading,
                                    items,
                                    is_rest,
                                    rest_head,
                                    dd_span,
                                    before,
                                });
                                continue; // reading stays true: read the element as a fresh level
                            }
                        } else if self.at(Kind::LBrace)
                            || (self.kind() == Kind::Hash && self.nth_kind(1) == Kind::LBrace)
                        {
                            // `{ field = p, … }` record pattern (shorthand fields inline) OR `#{ key = p, … }`
                            // map pattern (key = expr, no shorthand) — head created before entries; a value /
                            // `.. rest` descends on the worklist. Both build `(= key/field value)` triples.
                            let brace_span = cur_start;
                            let is_map = self.kind() == Kind::Hash;
                            if is_map {
                                self.bump(); // '#'
                                self.bump(); // '{'
                            } else {
                                self.bump(); // '{'
                            }
                            let head = self.name(if is_map { "map" } else { "record" }, brace_span);
                            let mut items = vec![head];
                            let step = if self.at(Kind::RBrace) {
                                self.expect(Kind::RBrace, "`}`");
                                None
                            } else if is_map {
                                Some(self.advance_map_pat())
                            } else {
                                self.advance_record_pat(&mut items)
                            };
                            match step {
                                None => {
                                    node = self.list(items, brace_span.merge(self.prev_span()));
                                    reading = false; // -> postfix phase
                                }
                                Some(descend) => {
                                    let (is_rest, rest_head, dd_span, f_start, field, before) =
                                        match descend {
                                            RecordPatDescend::Rest {
                                                dd_span,
                                                rest_head,
                                                before,
                                            } => (
                                                true,
                                                rest_head,
                                                dd_span,
                                                brace_span,
                                                StructId(0),
                                                before,
                                            ),
                                            RecordPatDescend::Value {
                                                f_start,
                                                field,
                                                before,
                                            } => (
                                                false,
                                                StructId(0),
                                                brace_span,
                                                f_start,
                                                field,
                                                before,
                                            ),
                                        };
                                    pending.push(PCont::RecordPat {
                                        lvl_start: cur_start,
                                        lvl_entered: cur_entered,
                                        brace_span,
                                        is_map,
                                        items,
                                        is_rest,
                                        rest_head,
                                        dd_span,
                                        f_start,
                                        field,
                                        before,
                                    });
                                    continue; // read the field value / rest sub-pattern as a fresh level
                                }
                            }
                        } else {
                            node = self.pattern_atom();
                            reading = false;
                        }
                    }
                }
            }
            // POSTFIX PHASE (only for a real, non-tripped level — a tripped level has no postfix): fold a
            // `.member` inline; an `( args )` application descends its args on the worklist.
            if cur_entered {
                let mut descended = false;
                loop {
                    match self.kind() {
                        Kind::Dot
                            if matches!(self.nth_kind(1), Kind::Ident | Kind::BacktickName) =>
                        {
                            self.bump(); // '.'
                            let seg_span = self.cur_span();
                            let seg_t = self.bump().unwrap();
                            let seg = match seg_t.kind {
                                Kind::BacktickName => self.name(
                                    literal::unescape_backtick_name(self.text(seg_t)),
                                    seg_span,
                                ),
                                _ => self.name(self.text(seg_t), seg_span),
                            };
                            let dot_span = cur_start.merge(self.prev_span());
                            let dot = self.atom(Leaf::Member, dot_span);
                            node = self.list(vec![dot, node, seg], dot_span);
                        }
                        Kind::LParen => {
                            self.bump(); // '('
                            let items = vec![node];
                            if self.at(Kind::RParen) {
                                // Empty application `C()` -> `(C)` (a one-element list), no arg descent.
                                self.expect(Kind::RParen, "`)`");
                                node = self.list(items, cur_start.merge(self.prev_span()));
                                continue; // fold further postfix onto the result
                            }
                            let before = self.pos;
                            pending.push(PCont::Args {
                                start: cur_start,
                                entered: cur_entered,
                                items,
                                before,
                            });
                            reading = true; // read the first argument as a fresh level
                            descended = true;
                            break;
                        }
                        _ => break, // no more postfix — the level is complete
                    }
                }
                if descended {
                    continue; // go read the argument
                }
            }
            // LEVEL COMPLETE: balance the depth budget, then deliver `node` to the parent (or return it).
            if cur_entered {
                self.depth -= 1;
            }
            match pending.pop() {
                None => return node,
                Some(PCont::Args {
                    start,
                    entered,
                    mut items,
                    before,
                }) => {
                    // `node` is the delivered argument sub-pattern.
                    items.push(node);
                    if !self.sep_continue(Kind::RParen) {
                        self.expect(Kind::RParen, "`)`");
                        node = self.list(items, start.merge(self.prev_span()));
                        // Resume the OWNING level's postfix loop on the built application node.
                        cur_start = start;
                        cur_entered = entered;
                        reading = false;
                        continue;
                    }
                    if self.pos == before {
                        self.bump(); // arg didn't consume — avoid a missing-`,` spin
                    }
                    let before = self.pos;
                    pending.push(PCont::Args {
                        start,
                        entered,
                        items,
                        before,
                    });
                    reading = true;
                    continue; // read the next argument as a fresh level
                }
                Some(PCont::List {
                    lvl_start,
                    lvl_entered,
                    open_span,
                    closer,
                    allow_rest,
                    allow_comments,
                    leading,
                    mut items,
                    is_rest,
                    rest_head,
                    dd_span,
                    before,
                }) => {
                    // `node` is the delivered element (or `.. rest` operand). Push it (bin wraps the segment
                    // with its own-line leading + a last-segment same-line trailing comment), then read the
                    // next element or close and resume the OWNING level's postfix on the assembled node.
                    if is_rest {
                        let span = dd_span.merge(self.prev_span());
                        items.push(self.list(vec![rest_head, node], span));
                    } else if allow_comments {
                        let mut seg = self.wrap_comments(leading, node);
                        if self.at(closer) {
                            let trailing = self.take_trailing_comment_here();
                            seg = self.wrap_comment_after(trailing, seg);
                        }
                        items.push(seg);
                    } else {
                        items.push(node);
                    }
                    if !self.sep_continue(closer) {
                        self.expect(closer, "comma-list pattern closer");
                        node = self.list(items, open_span.merge(self.prev_span()));
                        cur_start = lvl_start;
                        cur_entered = lvl_entered;
                        reading = false; // resume the owning level's postfix phase on the assembled node
                        continue;
                    }
                    if self.pos == before {
                        self.bump(); // element didn't consume — avoid a missing-`,` spin
                    }
                    let before = self.pos;
                    let (is_rest, dd_span, rest_head) = if allow_rest && self.at(Kind::DotDot) {
                        let dd = self.cur_span();
                        self.bump(); // `..`
                        (true, dd, self.name("..", dd))
                    } else {
                        (false, open_span, StructId(0))
                    };
                    let leading = if allow_comments && !is_rest {
                        self.take_comments_here()
                    } else {
                        Vec::new()
                    };
                    pending.push(PCont::List {
                        lvl_start,
                        lvl_entered,
                        open_span,
                        closer,
                        allow_rest,
                        allow_comments,
                        leading,
                        items,
                        is_rest,
                        rest_head,
                        dd_span,
                        before,
                    });
                    reading = true;
                    continue; // read the next element as a fresh level
                }
                Some(PCont::Tuple {
                    lvl_start,
                    lvl_entered,
                    paren_span,
                    mut items,
                    is_rest,
                    rest_head,
                    dd_span,
                    before,
                }) => {
                    // Macro-free next-element helper (grouping-vs-tuple's Comma branch + a subsequent
                    // element): a `.. rest` spread (head created before the descent) or an ordinary
                    // sub-pattern; descends it. Diverges via `continue`.
                    macro_rules! next_tuple_pat {
                        ($items:expr) => {{
                            let before = self.pos;
                            let (is_rest, dd_span, rest_head) = if self.at(Kind::DotDot) {
                                let dd = self.cur_span();
                                self.bump(); // `..`
                                (true, dd, self.name("..", dd))
                            } else {
                                (false, paren_span, StructId(0))
                            };
                            pending.push(PCont::Tuple {
                                lvl_start,
                                lvl_entered,
                                paren_span,
                                items: $items,
                                is_rest,
                                rest_head,
                                dd_span,
                                before,
                            });
                            reading = true;
                            continue; // read the next tuple element as a fresh level
                        }};
                    }
                    if items.is_empty() {
                        // `node` is the delivered FIRST sub-pattern — decide grouping `(p)` vs tuple `(p, …)`.
                        let first = node;
                        if self.at(Kind::Comma) {
                            let head = self.name("tuple", paren_span);
                            let items = vec![head, first];
                            if self.sep_continue(Kind::RParen) {
                                next_tuple_pat!(items);
                            }
                            // `(p,)` — trailing comma, no further element.
                            self.expect(Kind::RParen, "`)`");
                            node = self.list(items, paren_span.merge(self.prev_span()));
                        } else {
                            // Grouping: transparent — `first` IS the pattern.
                            self.expect(Kind::RParen, "`)`");
                            node = first;
                        }
                    } else {
                        // `node` is a delivered subsequent element (or `.. rest` operand).
                        if is_rest {
                            let span = dd_span.merge(self.prev_span());
                            items.push(self.list(vec![rest_head, node], span));
                        } else {
                            items.push(node);
                        }
                        // Missing-`,` progress guard fires AFTER each element, BEFORE the next `sep_continue`
                        // (the tuple loop's `while sep_continue { before; elem; if pos==before bump }` order,
                        // distinct from the list loop where the guard is after `sep_continue`).
                        if self.pos == before {
                            self.bump();
                        }
                        if self.sep_continue(Kind::RParen) {
                            next_tuple_pat!(items);
                        }
                        self.expect(Kind::RParen, "`)`");
                        node = self.list(items, paren_span.merge(self.prev_span()));
                    }
                    cur_start = lvl_start;
                    cur_entered = lvl_entered;
                    reading = false; // resume the owning level's postfix phase on the tuple/group node
                    continue;
                }
                Some(PCont::RecordPat {
                    lvl_start,
                    lvl_entered,
                    brace_span,
                    is_map,
                    mut items,
                    is_rest,
                    rest_head,
                    dd_span,
                    f_start,
                    field,
                    before,
                }) => {
                    // `node` is the delivered field VALUE sub-pattern (build `(= field value)`) or the
                    // `.. rest` operand (build `(.. binder)`); then advance the next field or close.
                    if is_rest {
                        let span = dd_span.merge(self.prev_span());
                        items.push(self.list(vec![rest_head, node], span));
                    } else {
                        let f_span = f_start.merge(self.prev_span());
                        let eq = self.atom(Leaf::FieldPair, f_start);
                        items.push(self.list(vec![eq, field, node], f_span));
                    }
                    if !self.sep_continue(Kind::RBrace) {
                        self.expect(Kind::RBrace, "`}`");
                        node = self.list(items, brace_span.merge(self.prev_span()));
                        cur_start = lvl_start;
                        cur_entered = lvl_entered;
                        reading = false; // resume the owning level's postfix phase on the record node
                        continue;
                    }
                    if self.pos == before {
                        self.bump(); // field didn't consume — avoid a missing-`,` spin
                    }
                    let step = if is_map {
                        Some(self.advance_map_pat())
                    } else {
                        self.advance_record_pat(&mut items)
                    };
                    match step {
                        None => {
                            node = self.list(items, brace_span.merge(self.prev_span()));
                            cur_start = lvl_start;
                            cur_entered = lvl_entered;
                            reading = false;
                            continue;
                        }
                        Some(descend) => {
                            let (is_rest, rest_head, dd_span, f_start, field, before) =
                                match descend {
                                    RecordPatDescend::Rest {
                                        dd_span,
                                        rest_head,
                                        before,
                                    } => {
                                        (true, rest_head, dd_span, brace_span, StructId(0), before)
                                    }
                                    RecordPatDescend::Value {
                                        f_start,
                                        field,
                                        before,
                                    } => (false, StructId(0), brace_span, f_start, field, before),
                                };
                            pending.push(PCont::RecordPat {
                                lvl_start,
                                lvl_entered,
                                brace_span,
                                is_map,
                                items,
                                is_rest,
                                rest_head,
                                dd_span,
                                f_start,
                                field,
                                before,
                            });
                            reading = true;
                            continue; // read the next field's value / rest sub-pattern
                        }
                    }
                }
            }
        }
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
                    Leaf::Bytes(literal::unescape_byte_string_token(self.text(t)).into()),
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
                        // A `.. rest` binds the TRAILING positional elements of the tuple to the wrapped
                        // `(.. rest)` node (the operator's `(.. v)`-everywhere canonical, the twin of the
                        // list-pattern rest at the `[…]` arm). Tuple arity is STATIC, so this is a fixed-arity
                        // trailing-positional bind — `(a, b, .. rest)` -> `(tuple a b (.. rest))`; the leading
                        // binders resolve to the first elements and `rest` to a tuple of the remainder. (The
                        // tuple-rest MATCH lowering is v-inference's co-land slice; this is the surface node it
                        // consumes.)
                        if !self.rest_marker(&mut items, |p| p.pattern()) {
                            items.push(self.pattern());
                        }
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
            Kind::Hash if self.nth_kind(1) == Kind::LBracket => {
                // `#[ p, … ]` — the RAW-LIST escape in pattern position, the twin of the expression
                // `#[ e, … ]` (`hash_list`). It reads to a BARE list of sub-patterns `(p …)` — no head
                // name — so the EMPTY case `#[]` reads to an empty-list node `()`, the exact inverse of
                // the pattern printer's `#[]` render for an empty `Struct::List`. That closes the
                // round-trip for an empty-compound QUOTE pattern (`(quote ())`), whose inner `()` has no
                // other pattern surface (`()` reads to `unit`, `[]` to `(list)`).
                self.bump(); // '#'
                self.bump(); // '['
                let mut items = Vec::new();
                if !self.at(Kind::RBracket) {
                    loop {
                        let before = self.pos;
                        items.push(self.pattern());
                        if !self.sep_continue(Kind::RBracket) {
                            break;
                        }
                        if self.pos == before {
                            self.bump(); // pattern didn't consume — avoid a missing-`,` spin
                        }
                    }
                }
                self.expect(Kind::RBracket, "`]`");
                let rlspan = span.merge(self.prev_span());
                self.list(items, rlspan)
            }
            Kind::Hash if self.nth_kind(1) == Kind::LBrace => {
                // `#{ k = p, … }` / `#{ k = p, …, .. rest }` — a map pattern, the s-expr
                // `(map (= k p) .. rest)` twin. Head is the NAME `map`; each entry is the canonical
                // `(= key sub-pattern)` `FieldPair` triple — the SAME form as a map-VALUE entry and a
                // record-pattern field (M2/M2b native ctor-leaf canonical, operator M3 ruling), so a native
                // `#map` PATTERN (`Leaf::Ctor(Map)` + `FieldPair` entries, what rcdzc #5229 resolves)
                // round-trips through this surface `structurally_eq`. The key is a value expression to look
                // up, the value slot a sub-pattern; an optional `.. rest` binds the remaining map. Was a bare
                // 2-element `(key sub-pattern)` pair, which did not `structurally_eq` a native `FieldPair`
                // map pattern (the ML map-PATTERN round-trip gap, FACE 2 / breaker cdzw45).
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
                            let eq = self.atom(Leaf::FieldPair, e_start);
                            items.push(self.list(vec![eq, key, value], e_span));
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
            Kind::Hash if self.nth_kind(1) == Kind::LParen => {
                // `#( p, … )` / `#( p, …, .. rest )` — a SET PATTERN, the pattern twin of the `#(…)` set
                // literal (`set_literal`). Head is the NATIVE set ctor leaf `Leaf::Ctor(Set)` (`#set`) —
                // v-ast-compound's ruling: the corpus's compound match-patterns are already native `#word`
                // (`#tuple`/`#list`/`#map`/`#record`/`#set`), and native `#set` matches the `#set` VALUE
                // literal + the reader's uniform Leaf::Ctor(Set) emission, so a name-head `(set …)` would be
                // the lone odd one out. Elements are sub-patterns; an optional trailing `.. rest` binds the
                // remaining set to the wrapped `(.. rest)` node — the twin of the map/list/tuple/record
                // pattern rest, the canonical form the Set rest-matcher lowering (v-ast-compound / v-inference
                // co-land) consumes. (The set-rest MATCH semantics are their slice; this is the surface.)
                self.bump(); // '#'
                self.bump(); // '('
                let head = self.ctor_head("set", span);
                let mut items = vec![head];
                if !self.at(Kind::RParen) {
                    loop {
                        let before = self.pos;
                        if !self.rest_marker(&mut items, |p| p.pattern()) {
                            items.push(self.pattern());
                        }
                        if !self.sep_continue(Kind::RParen) {
                            break;
                        }
                        if self.pos == before {
                            self.bump(); // no sub-pattern token consumed — avoid a missing-`,` spin
                        }
                    }
                }
                self.expect(Kind::RParen, "`)`");
                let sspan = span.merge(self.prev_span());
                self.list(items, sspan)
            }
            Kind::LBrace => {
                // `{ field = p, … }` — a RECORD PATTERN, destructuring a record BY FIELD (the s-expr
                // `(record (field p) …)` twin, the dual of the record-value literal). Head is the NAME
                // `record`; each entry is a `(field sub-pattern)` pair. Field SHORTHAND `{ x }` puns to
                // `{ x = x }` (the field `x` binds a same-named binder), matching the record literal. The
                // operator ruled the bare-brace surface (default, consistent with tuple/list patterns —
                // a pattern slot can't hold a value, so `{ … }` here is unambiguously a pattern, not a
                // record VALUE literal). A PARTIAL pattern (fewer fields than the type) is fine — the
                // compiler's `(record …)` binding lowering binds the named fields by projection.
                self.bump(); // '{'
                let head = self.name("record", span);
                let mut items = vec![head];
                if !self.at(Kind::RBrace) {
                    loop {
                        let before = self.pos;
                        // A `.. rest` binds the REMAINING fields (those not named by the pattern) to the
                        // wrapped `(.. rest)` node — the twin of the map/set/list-pattern rest, per the
                        // operator's `(.. v)`-everywhere canonical. A record field-set is STATIC, so `rest`
                        // is a residual record of the un-named fields (`{ a = p, .. rest }` ->
                        // `(record (= a p) (.. rest))`). (The record-rest MATCH lowering — residual-record
                        // construction — is v-inference's co-land slice; this is the surface node.)
                        if !self.rest_marker(&mut items, |p| p.pattern()) {
                            let f_start = self.cur_span();
                            // Capture the field spelling BEFORE `binder()` consumes it, so a shorthand
                            // field can pun the same name as its binder sub-pattern.
                            let pun = self.binder_spelling();
                            let field = self.binder();
                            let value = if self.at(Kind::Eq) {
                                self.bump(); // `=`
                                self.pattern()
                            } else if let Some(n) = pun {
                                // shorthand `{ x }` -> `(= x x)`: the field binds a same-named binder.
                                self.name(n, f_start)
                            } else {
                                self.expect(Kind::Eq, "`=`");
                                self.pattern()
                            };
                            let f_span = f_start.merge(self.prev_span());
                            // A record-PATTERN field is the canonical `(= name sub-pattern)` triple — the
                            // SAME form as a value-record field (RV1), so patterns and literals spell
                            // identically (operator ruling: path B, full symmetry).
                            let eq = self.atom(Leaf::FieldPair, f_start);
                            items.push(self.list(vec![eq, field, value], f_span));
                        }
                        if !self.sep_continue(Kind::RBrace) {
                            break;
                        }
                        if self.pos == before {
                            self.bump(); // no field token consumed — avoid a missing-`,` spin
                        }
                    }
                }
                self.expect(Kind::RBrace, "`}`");
                let rspan = span.merge(self.prev_span());
                self.list(items, rspan)
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
            // A tight prefix (member/call chain), no trailing infix. DEPTH GUARD: the bare form recurses
            // `prefix` → `unquote` → `prefix` directly (`,,,,,x`), bypassing `expr`'s guard — a deep run
            // overflowed the stack (SIGABRT). Count each layer against the shared depth budget via
            // `guard_prefix`, exactly like the unary-minus arm.
            if let Some(err) = self.guard_prefix(start) {
                return self.list(vec![head, err], start.merge(self.prev_span()));
            }
            let s = self.cur_span();
            let p = self.prefix();
            let inner = self.postfix(p, s);
            self.depth -= 1;
            inner
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
                    // Own-line `//` comment(s) LEADING this element (`[\n // note\n 1, …]`) sit in the
                    // element's first-token leading slot. `expr` does NOT drain that slot (only `stmt`/
                    // `body_expr` do), so without this the comment is stranded and dropped. Drain it and
                    // wrap the element in `(comment "text" elem)` (LEADING form) — the printer already
                    // renders a leading `(comment …)` as a `// …` line above the element (own-line), and
                    // `strip_comments` peels it. Distinct from the TRAILING `(comment-after …)` below.
                    let leading = self.take_comments_here();
                    let elem = self.expr(crate::token::PREC_SEQ + 1);
                    let elem = self.wrap_comments(leading, elem);
                    // A `//` comment trailing the LAST element on the same source line (`[…, x // last]`)
                    // sits in the `]` token's leading slot; capture it as `(comment-after …)` so it
                    // re-prints same-line (the printer forces `]` onto its own line so it isn't swallowed).
                    // `strip_comments` peels it, so the compiler is unaffected.
                    //
                    // GATE on `at(RBracket)`: the trailing comment is in the NEXT token's leading slot, so
                    // ONLY a last-element comment has `]` as that next token. A comment after a NON-last
                    // element (`[1 // note, 2]`) has `,` next — capturing it there would print `1 // note,
                    // 2` with the `, 2` swallowed into the comment line → invalid re-parse (PR#758 /
                    // Copilot: an unconditional capture is a round-trip BREAK, worse than the drop). So a
                    // mid-element TRAILING comment is left stranded → the comment-drop guard refuses the
                    // format (no corruption). (Mirrors `variant()`, whose capture is only well-defined at
                    // the trailing edge.) The LEADING own-line capture above has no such hazard: a `//` on
                    // its OWN line above an element re-prints on its own line and always re-reads correctly.
                    if self.at(Kind::RBracket) {
                        let trailing = self.take_trailing_comment_here();
                        items.push(self.wrap_comment_after(trailing, elem));
                    } else {
                        items.push(elem);
                    }
                }
                if !self.sep_continue(Kind::RBracket) {
                    break;
                }
                if self.pos == before {
                    self.bump(); // element didn't consume — avoid a missing-`,` spin
                }
            }
        }
        // An own-line `//` comment before the `]` (`[1, 2\n // note\n]`) is in the closer's leading slot —
        // attach it to the last element so it survives the round-trip (else the drop-guard refuses).
        self.drain_closer_comment_onto_last(&mut items, 1);
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
                // `.. r` spreads a record's fields into the literal (`{ ..base, a = 1 }`) — the record twin
                // of the list/map construction spread: a flat `Name("..")` sibling + the spread operand among
                // the `(= name value)` field triples (the SAME marker the compiler's collection lowering
                // scans for; well-formedness is the compiler's, matching the s-expr surface). An ordinary
                // field otherwise. (The construction-spread LOWERING is v-inference's slice; this is surface.)
                if self.rest_marker(&mut items, |p| p.expr(crate::token::PREC_SEQ + 1)) {
                    if !self.sep_continue(Kind::RBrace) {
                        break;
                    }
                    if self.pos == before {
                        self.bump();
                    }
                    continue;
                }
                // Own-line `//` comment(s) leading this field (`{\n // note\n a = 1, … }`) sit in the
                // field's first-token slot; drain here (before the name) and wrap the `(name value)` pair
                // below (`is_pairs`/the record printer unwrap it). Own-line has no swallow hazard.
                let leading = self.take_comments_here();
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
                // RV1 (DESIGN-record-type-syntax Phase B): a value-record field is the explicit
                // `(= name value)` node — the `=` the author writes survives into the arena (symmetric
                // to the type-side `(: name T)` ascription; operator: "much more explicit and less
                // magical"). Was the bare `(name value)` pair that dropped the `=`. Shorthand `{ x }`
                // puns to `(= x x)` (every field is `=`-headed, uniform). The `record` STRING head is
                // unchanged; only the field node gains the `=` head.
                let eq = self.atom(Leaf::FieldPair, f_start);
                let field = self.list(vec![eq, name, value], f_span);
                // Wrap any own-line LEADING comment around the field triple (printer renders it above).
                let field = self.wrap_comments(leading, field);
                // A `//` trailing the LAST field on the same line (`{ a = 1, b = 2 // last }`) sits in the
                // `}` token's leading slot; capture it as `(comment-after "text" (name value))` (gated on
                // `at(RBrace)` — only the last field, the PR#758 rule: a non-last comment would swallow the
                // following `, …`). The record printer/shape-guard unwraps the wrapper. `strip_comments`
                // peels it. A non-last same-line comment is left stranded → the comment-drop guard refuses.
                if self.at(Kind::RBrace) {
                    let trailing = self.take_trailing_comment_here();
                    items.push(self.wrap_comment_after(trailing, field));
                } else {
                    items.push(field);
                }
                if !self.sep_continue(Kind::RBrace) {
                    break;
                }
                if self.pos == before {
                    self.bump(); // no field token consumed — avoid a missing-`,` spin
                }
            }
        }
        // Own-line `//` before `}` (`{ a = 1\n // note\n }`) → attach to the last field (see the helper).
        self.drain_closer_comment_onto_last(&mut items, 1);
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
                // Own-line `//` comment(s) leading this entry (`#{\n // note\n 1 = v, … }`) — drain before
                // the entry and wrap the `(key value)` pair below (printer unwraps). No swallow hazard.
                let leading = self.take_comments_here();
                // `.. rest` spreads a tail map into the literal (`#{ 1 = v, .. rest }`); a `key = value`
                // entry otherwise. The marker is flat (`… ".." rest`), the list analogue's twin.
                // Key and value are single expressions (`PREC_SEQ + 1`); a sequence parenthesizes.
                if !self.rest_marker(&mut items, |p| p.expr(crate::token::PREC_SEQ + 1)) {
                    let e_start = self.cur_span();
                    let key = self.expr(crate::token::PREC_SEQ + 1);
                    self.expect(Kind::Eq, "`=`");
                    let value = self.expr(crate::token::PREC_SEQ + 1);
                    let e_span = e_start.merge(self.prev_span());
                    // M2: a map entry is a native `(= key value)` FieldPair (unified with record fields —
                    // operator ruling; matches the s-expr `#map((= k v))`), not a bare `(key value)` pair.
                    let eq = self.atom(Leaf::FieldPair, e_start);
                    let entry = self.list(vec![eq, key, value], e_span);
                    let entry = self.wrap_comments(leading, entry);
                    // Capture a same-line trailing `//` on the LAST entry (gated on `at(RBrace)`), like the
                    // record loop — wrap as `(comment-after "text" (key value))`; the map printer/shape-guard
                    // unwraps it. A non-last same-line comment is left to the comment-drop guard (no corruption).
                    if self.at(Kind::RBrace) {
                        let trailing = self.take_trailing_comment_here();
                        items.push(self.wrap_comment_after(trailing, entry));
                    } else {
                        items.push(entry);
                    }
                }
                if !self.sep_continue(Kind::RBrace) {
                    break;
                }
                if self.pos == before {
                    self.bump(); // no entry token consumed — avoid a missing-`,` spin
                }
            }
        }
        // Own-line `//` before `}` (`#{ a = 1\n // note\n }`) → attach to the last entry (see the helper).
        self.drain_closer_comment_onto_last(&mut items, 1);
        self.expect(Kind::RBrace, "`}`");
        let span = start.merge(self.prev_span());
        self.list(items, span)
    }

    /// The iterative field-pair driver shared by `expr_iter`'s `{ … }` record and `#{ … }` map operands
    /// (the worklist twin of the `record_literal` / `map_literal` inline field loops). Starting at a field
    /// position, it processes fields — appending each completed one to `items` — until either:
    ///   - the field list CLOSES (the closer is consumed here): returns `None`; or
    ///   - a field needs a SUB-EXPR read (a `..` rest operand, a map key, a record/map value): returns
    ///     `Some((phase, before))` so `expr_iter` can push a `Cont::Fields` for `phase` and DESCEND. On
    ///     deliver, `expr_iter` builds the field, runs the same `sep_continue` + missing-`,` progress guard
    ///     (`before` is the pos at that field's start), then calls this again for the next field.
    ///
    /// Record SHORTHAND fields (`{ x }` → `(= x x)`) read no sub-expr, so they are completed INLINE here in
    /// a loop (sibling iteration, NOT recursion — bounded by field count, never nesting depth). The two
    /// families differ only where `record_literal`/`map_literal` do: map drains own-line leading comments
    /// BEFORE the `..`-rest check (dropped on a rest), record checks rest FIRST (no leading on that path);
    /// map reads a `key` expr where record reads a flat `binder` + optional `= value` / pun. Byte-identical
    /// to the recursive bodies (arena order, span table, errors) — the `expr_iter` oracle diffs it.
    fn advance_fields(
        &mut self,
        items: &mut Vec<StructId>,
        is_map: bool,
        closer: Kind,
    ) -> Option<(FieldPhase, usize)> {
        loop {
            let before = self.pos;
            if is_map {
                // map: leading comments drained FIRST (dropped if the entry turns out to be a `..` rest).
                let leading = self.take_comments_here();
                if self.at(Kind::DotDot) {
                    let dd_span = self.cur_span();
                    self.bump(); // `..`
                    let rest_head = self.name("..", dd_span);
                    return Some((FieldPhase::RestOperand { dd_span, rest_head }, before));
                }
                let e_start = self.cur_span();
                return Some((FieldPhase::MapKey { leading, e_start }, before));
            }
            // record: `..`-rest checked FIRST (no leading comment drain on that path).
            if self.at(Kind::DotDot) {
                let dd_span = self.cur_span();
                self.bump(); // `..`
                let rest_head = self.name("..", dd_span);
                return Some((FieldPhase::RestOperand { dd_span, rest_head }, before));
            }
            let leading = self.take_comments_here();
            let f_start = self.cur_span();
            // Capture the name spelling BEFORE the binder consumes the token (a shorthand puns it).
            let pun = self.binder_spelling();
            let name = self.binder();
            if self.at(Kind::Eq) {
                self.bump(); // `=`
                return Some((
                    FieldPhase::RecordValue {
                        leading,
                        f_start,
                        name,
                    },
                    before,
                ));
            }
            if let Some(n) = pun {
                // Field SHORTHAND `{ x }` → `(= x x)`: the value is a SECOND `x` occurrence, no sub-expr —
                // completed inline (matches `record_literal`). Build + append, then advance the separator.
                let value = self.name(n, f_start);
                let eq = self.atom(Leaf::FieldPair, f_start);
                let f_span = f_start.merge(self.prev_span());
                let field = self.list(vec![eq, name, value], f_span);
                let field = self.wrap_comments(leading, field);
                if self.at(closer) {
                    let trailing = self.take_trailing_comment_here();
                    items.push(self.wrap_comment_after(trailing, field));
                } else {
                    items.push(field);
                }
                if !self.sep_continue(closer) {
                    self.drain_closer_comment_onto_last(items, 1);
                    self.expect(closer, "`}`");
                    return None;
                }
                if self.pos == before {
                    self.bump(); // no field token consumed — avoid a missing-`,` spin
                }
                continue; // next field, inline
            }
            // A non-name field with no `=` — record the missing `=` (as `record_literal` does), read value.
            self.expect(Kind::Eq, "`=`");
            return Some((
                FieldPhase::RecordValue {
                    leading,
                    f_start,
                    name,
                },
                before,
            ));
        }
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

    /// `#( e, … )`  ->  `#set(e …)` — the native `Leaf::Ctor(Set)` compound literal (M2
    /// native-compound-data), the third built-in collection surface completing the `#`-prefix family
    /// (`#{`=map, `#[`=raw-list, `#(`=set). Its head is the distinct unshadowable set ctor leaf (kind
    /// identity), uniform with `#list`/`#tuple`/`#record`/`#map` — so `#()` is the empty set `#set()` and
    /// ml-convert nativizes it through the same one route as every other ctor. (Was sugar for a
    /// `Set.of([…])` member CALL — `((. Set of) ("list" …))`; the printer still recognizes that legacy
    /// shape too, so an un-migrated corpus set still round-trips to `#(…)`.) Elements are single
    /// expressions (`PREC_SEQ + 1`), comma-separated; a sequence element parenthesizes.
    fn set_literal(&mut self) -> StructId {
        let start = self.cur_span();
        self.bump(); // '#'
        self.bump(); // '('
        let set_head = self.ctor_head("set", start);
        let mut elems = vec![set_head];
        if !self.at(Kind::RParen) {
            loop {
                let before = self.pos;
                // `.. s` spreads a set into the literal (`#(..a, x)`) — the set twin of the list/map/record
                // construction spread: a flat `Name("..")` sibling + operand in the `(set …)` ctor node (the
                // SAME marker the compiler's collection lowering scans for). An ordinary element otherwise.
                // (The set-spread LOWERING — Set.union — is v-inference's slice; this is the surface.)
                if self.rest_marker(&mut elems, |p| p.expr(crate::token::PREC_SEQ + 1)) {
                    if !self.sep_continue(Kind::RParen) {
                        break;
                    }
                    if self.pos == before {
                        self.bump();
                    }
                    continue;
                }
                // Own-line leading comment before this element (own-line has no swallow hazard), then the
                // element, then a same-line trailing comment on the LAST element (gated on `at(RParen)`).
                let leading = self.take_comments_here();
                let elem = self.expr(crate::token::PREC_SEQ + 1);
                let elem = self.wrap_comments(leading, elem);
                // A set literal `#(…)` desugars to `Set.of([…])`, so its elements ARE list elements and the
                // `#(…)` printer renders them via the shared comment-aware path. The trailing capture is
                // gated to the last element for the PR#758 reason (a non-last same-line comment would
                // swallow the following `, …`); a mid-element same-line comment is left to the drop-guard.
                if self.at(Kind::RParen) {
                    let trailing = self.take_trailing_comment_here();
                    elems.push(self.wrap_comment_after(trailing, elem));
                } else {
                    elems.push(elem);
                }
                if !self.sep_continue(Kind::RParen) {
                    break;
                }
                if self.pos == before {
                    self.bump(); // element didn't consume — avoid a missing-`,` spin
                }
            }
        }
        // Own-line `//` before `)` (`#(1, 2\n // note\n)`) → attach to the last element (see the helper).
        self.drain_closer_comment_onto_last(&mut elems, 1);
        self.expect(Kind::RParen, "`)`");
        let span = start.merge(self.prev_span());
        // Native set ctor: the `Leaf::Ctor(Set)` head + elements directly (no `Set.of`/`list` wrapper).
        self.list(elems, span)
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
                // Own-line `//` comment(s) leading this segment (`b[\n // seg\n u8(1), …]`) sit in its
                // first-token leading slot, which `expr`/`pattern` do not drain — capture + wrap
                // `(comment "text" seg)`. A same-line `//` trailing the LAST segment (`b[…, u8(2) // n]`)
                // sits in the `]` slot; capture as `(comment-after …)` gated on `at(RBracket)` (the PR#758
                // rule — a non-last segment's next token is `,`, no faithful slot). The `b[…]` printer
                // (construction) renders both via the shared comment-aware path; `strip_comments` peels
                // them so the compiler is unaffected. Same shape as `list_literal`.
                let leading = self.take_comments_here();
                let seg = segment(self);
                let seg = self.wrap_comments(leading, seg);
                let seg = if self.at(Kind::RBracket) {
                    let trailing = self.take_trailing_comment_here();
                    self.wrap_comment_after(trailing, seg)
                } else {
                    seg
                };
                items.push(seg);
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
    /// `(` tuple, `[` list, `#{` map, `#(` set, `b[` binary. These are the compound patterns
    /// [`Self::pattern`] deconstructs; a bare name/literal is NOT one (a name is an ordinary binder, a bare
    /// literal param is not a destructure). Keyed here — not by delegating every token to `pattern` — so a
    /// plain `name`/`name: Type` parameter keeps the fast [`Self::binder`] path and its diagnostics.
    fn at_pattern_param_start(&self) -> bool {
        // `{` opens a RECORD pattern (`{ x = a }`), `#{` a MAP pattern, `#(` a SET pattern — DISTINCT arenas
        // ((record …) vs (map …) vs (set …)). The bare-brace record pattern is the operator-ruled surface (a
        // pattern slot can't hold a value, so `{ … }` here is unambiguously a pattern, not a record VALUE literal).
        matches!(self.kind(), Kind::LParen | Kind::LBracket | Kind::BinOpen | Kind::LBrace)
            || (self.at(Kind::Hash) && matches!(self.nth_kind(1), Kind::LBrace | Kind::LParen))
            // A CONSTRUCTOR pattern `Ctor(binders…)` / `Mod.Ctor(binders…)` in binding position — an
            // `Ident` that HEADS a constructor pattern, i.e. immediately followed by `(` (an application,
            // `Some(x)`) or `.` (a qualified path, `Id.Mk(n)` / `W.Wrap(…)`). This is the same
            // single-constructor destructure the corpus binds in a `let`/param (`(let (((Id.Mk n) …)) …)`,
            // "binds exactly as a tuple pattern"), so `def f(Some(x)) = …` and `let C(c) = v in …` parse
            // via `pattern()` (which already deconstructs ctor patterns for match arms). A PLAIN binder
            // (`x`, `x: Type`) is an `Ident` followed by `,`/`)`/`:`/`=` — NOT `(`/`.` — so it stays on the
            // fast `binder()` path; only a ctor-application/qualified head takes the pattern route.
            || (self.at(Kind::Ident)
                && keyword(self.cur_text()).is_none()
                && matches!(self.nth_kind(1), Kind::LParen | Kind::Dot))
    }

    /// A type reference in a binder/return/payload position (a parameter annotation, a function's
    /// return type, a sum-variant payload). A type is a postfix expression — a name, a dotted or
    /// qualified name, or an application like `Option(Int64)` — extended with the RIGHT-associative
    /// function arrow `A -> B` -> `(-> A B)`, so a parameter/return type may itself be a function type
    /// (`f: Int64 -> Bool`, `-> Int64 -> Int64`). The arrow is parsed here (not via the general Pratt
    /// `expr`) so a type position admits `->` and application without also admitting arithmetic or a
    /// bare `:` re-ascription. `A -> B -> C` right-associates to `(-> A (-> B C))`.
    fn type_ref(&mut self) -> StructId {
        // I5: route to the iterative type driver when `read_ml` set the flag; the recursive body below
        // stays for `read_ml_recursive` (the frozen oracle reference). Incremental — de-recurses one type
        // layer at a time, staying byte-identical (the type differential oracle) each step.
        if self.iterative {
            return self.type_ref_iter();
        }
        let start = self.cur_span();
        // `forall a b. TYPE` — an explicit generic binder heading a type. Binds the lowercase names
        // `a`/`b` so they resolve as bound type variables inside TYPE instead of erroring CDZ0101
        // (unbound). It builds the canonical `(forall (binders…) TYPE)` node; a later lowering desugars
        // that to the pinned "generics are type-valued parameters" model — a `forall a.` becomes an
        // implicit `(: a Type)` binding — so it introduces no new ∀ engine (v-inference's I2). Contextual:
        // `forall` is only a keyword at the START of a type, so a plain name `forall` elsewhere is free.
        if self.at_keyword(Keyword::Forall) {
            return self.forall_type(start);
        }
        // A parenthesized form in TYPE position is a tuple TYPE (or a grouping), NOT the tuple VALUE
        // constructor the shared `prefix`/`paren` path would build. `(A, B)` here is `Tuple(A, B)` and
        // `(A)` is just `A` (a grouping) — the same surface `(a, b)` a tuple VALUE/pattern uses, but on
        // the RHS of a `:` the reader knows it denotes a type. Handled directly so no value ctor
        // (`("tuple" …)`) is ever emitted in type position (which resolved to a value → CDZ0203).
        let left = self.type_operand();
        // A DERIVED-UNIT type annotation composes unit factors with the infix operators `^`/`*`/`/`
        // (`Qty(Int64, meter / second ^ 2)`) — the surface the printer emits for the arena heads
        // `Unit.^`/`Unit.*`/`Unit./` (via `infix_glyph`). The value grammar reads these (in a quantity
        // literal / general expr), but type position had NO infix layer beyond `->`, so a derived-unit
        // annotation printed a form that failed re-parse (`expected ,` at the exponent — breaker's
        // report). Fold them here, tighter than the `->` below, matching the value side's bare-glyph
        // heads + `infix_prec` so the ML print→parse cycle round-trips. `->` stays the loosest type
        // constructor (folded after).
        let left = self.type_unit_infix(left, 0, start);
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

    /// Iterative type reader (I5) — the explicit-worklist replacement for the recursive [`Self::type_ref`]
    /// (byte-identical, verified by the differential oracle). Two grammar layers are de-recursed onto the
    /// worklist here:
    ///   - the `->` ARROW chain — right-associative (`A -> B -> C` = `A -> (B -> C)`), so each right operand
    ///     is read as a fresh arrow-LHS and the chain folds on the way back; and
    ///   - the postfix `(…)` APPLICATION arguments — the deep nested-generic vector `Foo(Bar(Baz(…)))` (a
    ///     type-application argument is a full [`Self::type_ref`], so it descends onto this worklist instead
    ///     of recursing `type_operand -> type_postfix -> type_arg -> type_ref`). Labeled record-type
    ///     arguments `name: T` build `(: name T)` via a [`TCont::Label`] continuation (shared node order with
    ///     the recursive [`Self::type_arg`] via [`Self::type_arg_label`]).
    ///
    /// Nodes are created in the SAME order as the recursive body (the `->` head before the right operand; a
    /// labeled arg's `label` before its type before the `:`; a `Tuple` head after the first paren element),
    /// so the arena AND the span table match. FULLY ITERATIVE (I5 complete): every recursive descent of the
    /// type grammar is on this worklist — the `->` arrow chain, the postfix `(…)` application arguments (the
    /// deep nested-generic vector `Foo(Bar(Baz(…)))`, via [`TCont::App`] + [`TCont::Label`]), the
    /// paren-tuple/grouping/unit `(…)` interior ([`TCont::Paren`]), the brace-record `{field: T, …}` interior
    /// ([`TCont::Brace`]), the `forall <binders> . body` (nested `forall`s via [`TCont::Forall`]), and the
    /// unit-composition infix `^`/`*`/`/` Pratt ([`TCont::Unit`] — so `(a * (b * …))` no longer recurses
    /// `type_unit_infix -> type_operand -> type_paren -> type_ref`). No type nesting can overflow the native
    /// stack. The `type_postfix`/`type_arg_exprs`/`type_unit_infix` depth-guard accounting (`self.depth +
    /// spine`) is mirrored exactly so the deep-nesting boundary stays byte-identical to the recursive
    /// reference (still verified by the differential oracle until I8). The recursive helpers (`type_operand`,
    /// `type_paren`, `type_brace_record`, `type_unit_infix`, `forall_type`, `type_postfix`) remain in place
    /// only for the frozen `read_ml_recursive` oracle reference (`self.iterative == false`).
    fn type_ref_iter(&mut self) -> StructId {
        // Pending continuations on the explicit worklist (replacing the native recursion of `type_ref` /
        // `type_operand` / `type_postfix` / `type_arg_exprs`). Each holds only Copy ids/spans + the
        // in-progress argument vector.
        enum TCont {
            /// A pending `LHS ->` awaiting its right type (right-associative arrow chain). `arrow` is the
            /// pre-created `->` name; `left` the arrow's left operand; `start` the LHS start span.
            Arrow {
                start: Span,
                arrow: StructId,
                left: StructId,
            },
            /// A pending LABELED type-application argument `name:` awaiting its type; on resume the completed
            /// type becomes `(: label ty)`. `start` is the label span (for the `:` name + node span).
            Label { start: Span, label: StructId },
            /// A postfix `(…)` type application mid-read: `head` (+ any member/app chain so far) is built,
            /// some `args` collected, currently descending into one argument (a fresh type). `node_start` is
            /// the operand start (app-node span + the following unit-infix/arrow start); `spine` the postfix
            /// depth counter (mirrors [`Self::type_postfix`]); `head_is_record` gates the record-field check.
            App {
                node_start: Span,
                head: StructId,
                head_is_record: bool,
                args: Vec<StructId>,
                spine: u32,
                um: u8,
            },
            /// A parenthesized type `( … )` mid-read: `start` is the `(` span; `items` the collected element
            /// nodes (empty until the first element decides grouping-vs-tuple — once a tuple, `items` holds
            /// `[Tuple-head, first, …]`); `head_made` is set once the `Tuple` head is created (i.e. a `,`
            /// confirmed a tuple, not a single-element grouping). `um` is the unit-infix min-precedence to
            /// apply to the completed paren operand (0 for a full type, >0 for a unit-infix right operand).
            Paren {
                start: Span,
                items: Vec<StructId>,
                head_made: bool,
                um: u8,
            },
            /// A brace-record type `{ field: T, … }` mid-read: `start` is the `{` span; `items` holds
            /// `[Record-head, field…]` (the `Record` head is created up front). Each field's label + `:` is
            /// read eagerly (via [`Self::read_type_record_field_label`]) and its type descends onto the
            /// worklist, rebuilt as `(: label ty)` by a [`TCont::Label`] continuation stacked above this one.
            /// `um` is the unit-infix min-precedence for the completed record operand.
            Brace {
                start: Span,
                items: Vec<StructId>,
                um: u8,
            },
            /// A `forall <binders> .` awaiting its body type. `head` is the pre-created `forall` name;
            /// `binder_list` the `(binders…)` node; `start` the `forall` span. On resume the body becomes
            /// `(forall (binders…) body)`. A nested `forall a. forall b. …` stacks these on the heap instead
            /// of recursing `forall_type -> type_ref -> forall_type`.
            Forall {
                start: Span,
                head: StructId,
                binder_list: StructId,
            },
            /// A pending unit-composition infix `left <op>` awaiting its RIGHT operand (I5 part 6). `head` is
            /// the pre-created `^`/`*`/`/` glyph name; `left` the accumulated left operand; `min_prec` the
            /// Pratt min-precedence of THIS chain (to resume after combining); `start` the chain's left-start
            /// span; `spine` the chain's op counter (mirrors `type_unit_infix`'s spine depth guard). The right
            /// operand is read as a fresh operand + a tighter Pratt (min_prec = op-prec + 1), then combined
            /// `(op left right)` on the way back — so `(a * (b * …))` no longer recurses `type_unit_infix ->
            /// type_operand -> type_paren -> type_ref -> type_unit_infix`.
            Unit {
                head: StructId,
                left: StructId,
                min_prec: u8,
                start: Span,
                spine: u32,
            },
        }
        // The driver's next action.
        enum Next {
            /// Read a fresh operand (a `forall` head ONLY when `um == 0`; else paren / brace / prefix-head),
            /// then run the unit-composition Pratt at min-precedence `um` on it. `um == 0` = a full type
            /// (arrow-checked after via `Reduce`); `um > 0` = a unit-infix RIGHT operand (combined into the
            /// pending `TCont::Unit`, no arrow) — mirroring the recursive `type_operand` + `type_unit_infix`.
            Read { um: u8 },
            /// Continue the postfix loop (member chain + `(…)` application) on a partially-built prefix-head
            /// operand, then run the unit-composition Pratt at `um`. `spine` is the postfix depth counter.
            Postfix {
                node_start: Span,
                node: StructId,
                spine: u32,
                um: u8,
            },
            /// One step of the unit-composition Pratt on `left` at `min_prec`; `spine` counts the ops folded
            /// into this chain (the `type_unit_infix` spine depth guard). `start` is the chain left-start span.
            Unit {
                left: StructId,
                min_prec: u8,
                start: Span,
                spine: u32,
            },
            /// A full type is complete: check for a trailing `->` (arrow chain) and otherwise deliver `value`
            /// to the nearest continuation. `start` is the operand start span.
            Reduce { start: Span, value: StructId },
        }
        // A min-precedence higher than any real unit-infix op (max ~11): forces the Pratt to read NO further
        // ops so it delivers `left` immediately — used after the depth guard trips (mirrors the recursive
        // `type_unit_infix` `break`, which stops that chain while ancestor chains keep folding).
        const UNIT_DONE: u8 = u8::MAX;
        let mut stack: Vec<TCont> = Vec::new();
        let mut next = Next::Read { um: 0 };
        loop {
            match next {
                Next::Read { um } => {
                    let node_start = self.cur_span();
                    if um == 0 && self.at_keyword(Keyword::Forall) {
                        // `forall <binders> . body` — de-recursed onto the worklist (I5 part 5): the preamble
                        // (head + binders + `.`) is read inline, then the BODY (a full type) descends here
                        // instead of recursing `forall_type -> type_ref`. A nested `forall a. forall b. …`
                        // stacks `TCont::Forall`s on the heap. Only at `um == 0`: a `forall` after a unit op
                        // (`a ^ forall`) is read by `prefix` as a plain name, exactly as the recursive
                        // `type_operand` (which never re-consumes `forall`). `forall` early-returns in the
                        // recursive `type_ref` (no unit-infix / no arrow at its own level — the body consumed
                        // any arrow chain), so on resume the built node reduces directly.
                        let (head, binder_list) = self.read_forall_preamble(node_start);
                        stack.push(TCont::Forall {
                            start: node_start,
                            head,
                            binder_list,
                        });
                        next = Next::Read { um: 0 };
                    } else if self.at(Kind::LParen) {
                        // Parenthesized / tuple / unit type — de-recursed onto the worklist (I5 part 3): the
                        // interior element(s) are full types, so each descends here instead of recursing
                        // `type_paren -> type_ref`. `()` is the unit type (no descent); `(A)` a transparent
                        // grouping; `(A, B, …)` a `(Tuple …)` node. The following unit-infix Pratt runs at the
                        // paren's exit points at min-prec `um` (mirroring `type_operand` + `type_unit_infix`).
                        self.expect(Kind::LParen, "`(`");
                        if self.at(Kind::RParen) {
                            self.bump();
                            let span = node_start.merge(self.prev_span());
                            let op = self.name("unit", span);
                            next = Next::Unit {
                                left: op,
                                min_prec: um,
                                start: node_start,
                                spine: 0,
                            };
                        } else {
                            stack.push(TCont::Paren {
                                start: node_start,
                                items: Vec::new(),
                                head_made: false,
                                um,
                            });
                            next = Next::Read { um: 0 };
                        }
                    } else if self.at(Kind::LBrace) {
                        // Brace record type `{ field: T, … }` — de-recursed onto the worklist (I5 part 4): the
                        // `Record` head is created up front, then each field's type descends here instead of
                        // recursing `type_brace_record -> type_ref`. Mirrors the recursive while-condition
                        // (`!} && !EOF`) so an empty / unclosed brace closes identically; the unit-infix Pratt
                        // runs at the brace exit at min-prec `um`.
                        self.expect(Kind::LBrace, "`{`");
                        let head = self.name("Record", node_start);
                        if !self.at(Kind::RBrace) && !self.at_end() {
                            let (fstart, label) = self.read_type_record_field_label();
                            stack.push(TCont::Brace {
                                start: node_start,
                                items: vec![head],
                                um,
                            });
                            stack.push(TCont::Label {
                                start: fstart,
                                label,
                            });
                            next = Next::Read { um: 0 };
                        } else {
                            self.expect(Kind::RBrace, "`}`");
                            let span = node_start.merge(self.prev_span());
                            let node = self.list(vec![head], span);
                            next = Next::Unit {
                                left: node,
                                min_prec: um,
                                start: node_start,
                                spine: 0,
                            };
                        }
                    } else {
                        // Prefix head + ITERATIVE postfix (member chain + `(…)` application): the deep
                        // nested-generic vector `Foo(Bar(Baz(…)))` descends onto this worklist.
                        let head = self.prefix();
                        next = Next::Postfix {
                            node_start,
                            node: head,
                            spine: 0,
                            um,
                        };
                    }
                }
                Next::Postfix {
                    node_start,
                    mut node,
                    mut spine,
                    um,
                } => match self.kind() {
                    Kind::Dot if self.dot_is_member() => {
                        node = self.member_access(node, node_start);
                        spine += 1;
                        if !self.depth_exceeded
                            && self.depth + spine >= crate::sexpr::MAX_NESTING_DEPTH
                        {
                            self.error("expression nests too deeply to parse");
                            self.depth_exceeded = true;
                            next = Next::Unit {
                                left: node,
                                min_prec: um,
                                start: node_start,
                                spine: 0,
                            };
                        } else {
                            next = Next::Postfix {
                                node_start,
                                node,
                                spine,
                                um,
                            };
                        }
                    }
                    Kind::LParen => {
                        self.expect(Kind::LParen, "`(`");
                        if self.at(Kind::RParen) {
                            // Empty application `Foo()`.
                            self.expect(Kind::RParen, "`)`");
                            let span = node_start.merge(self.prev_span());
                            node = self.list(vec![node], span);
                            spine += 1;
                            if !self.depth_exceeded
                                && self.depth + spine >= crate::sexpr::MAX_NESTING_DEPTH
                            {
                                self.error("expression nests too deeply to parse");
                                self.depth_exceeded = true;
                                next = Next::Unit {
                                    left: node,
                                    min_prec: um,
                                    start: node_start,
                                    spine: 0,
                                };
                            } else {
                                next = Next::Postfix {
                                    node_start,
                                    node,
                                    spine,
                                    um,
                                };
                            }
                        } else {
                            // Descend into the first argument (a fresh type, possibly labeled). Compute
                            // `head_is_record` here (at the `(`, on the current head) as the recursive
                            // `type_postfix` does, before reading any argument.
                            let head_is_record = self.builder.as_name(node) == Some("Record");
                            let label = self.type_arg_label();
                            stack.push(TCont::App {
                                node_start,
                                head: node,
                                head_is_record,
                                args: Vec::new(),
                                spine,
                                um,
                            });
                            if let Some((lstart, lbl)) = label {
                                stack.push(TCont::Label {
                                    start: lstart,
                                    label: lbl,
                                });
                            }
                            next = Next::Read { um: 0 };
                        }
                    }
                    _ => {
                        // No more postfix — run the unit-composition Pratt at `um`.
                        next = Next::Unit {
                            left: node,
                            min_prec: um,
                            start: node_start,
                            spine: 0,
                        };
                    }
                },
                Next::Unit {
                    left,
                    min_prec,
                    start,
                    spine,
                } => {
                    // The unit-composition infix Pratt (`^`/`*`/`/`), de-recursed (I5 part 6). One step:
                    // fold the next op at precedence >= `min_prec`, else deliver the chain.
                    let bind = match self.kind() {
                        Kind::Caret => Some("^"),
                        Kind::Star => Some("*"),
                        Kind::Slash => Some("/"),
                        _ => None,
                    }
                    .and_then(|o| infix_prec(o).map(|p| (o, p)))
                    .filter(|&(_, p)| p >= min_prec);
                    if let Some((op_name, prec)) = bind {
                        // Read the RIGHT operand as a fresh operand + a tighter Pratt (min-prec = prec + 1,
                        // left-associative), combined `(op left right)` when it completes via `TCont::Unit`.
                        let op_span = self.cur_span();
                        self.bump();
                        let head = self.name(op_name, op_span);
                        stack.push(TCont::Unit {
                            head,
                            left,
                            min_prec,
                            start,
                            spine,
                        });
                        next = Next::Read { um: prec + 1 };
                    } else if matches!(stack.last(), Some(TCont::Unit { .. })) {
                        // This chain's `left` is a RIGHT operand → combine into the pending `TCont::Unit`.
                        let (head, oleft, omp, ostart, ospine) = match stack.pop() {
                            Some(TCont::Unit {
                                head,
                                left,
                                min_prec,
                                start,
                                spine,
                            }) => (head, left, min_prec, start, spine),
                            _ => unreachable!("checked by matches! above"),
                        };
                        let span = ostart.merge(self.prev_span());
                        let combined = self.list(vec![head, oleft, left], span);
                        let spine2 = ospine + 1;
                        if !self.depth_exceeded
                            && self.depth + spine2 >= crate::sexpr::MAX_NESTING_DEPTH
                        {
                            // Guard trip: emit once + poison, then BREAK this chain (deliver `combined`, read
                            // no more ops via `UNIT_DONE`). Ancestor chains keep folding — their guard sees
                            // `depth_exceeded` and won't re-trip — matching the recursive `type_unit_infix`.
                            self.error("expression nests too deeply to parse");
                            self.depth_exceeded = true;
                            next = Next::Unit {
                                left: combined,
                                min_prec: UNIT_DONE,
                                start: ostart,
                                spine: spine2,
                            };
                        } else {
                            next = Next::Unit {
                                left: combined,
                                min_prec: omp,
                                start: ostart,
                                spine: spine2,
                            };
                        }
                    } else {
                        // A full type's operand-with-units is complete → arrow-check + deliver.
                        next = Next::Reduce { start, value: left };
                    }
                }
                Next::Reduce { start, value } => {
                    if self.at(Kind::Arrow) {
                        // `value` is an arrow LHS. Create the `->` head BEFORE descending the right operand
                        // (matching the recursive struct-id order), then read the right as a fresh arrow-LHS.
                        self.bump(); // `->`
                        let arrow = self.name("->", start);
                        stack.push(TCont::Arrow {
                            start,
                            arrow,
                            left: value,
                        });
                        next = Next::Read { um: 0 };
                    } else {
                        match stack.pop() {
                            None => return value,
                            // A `Reduce` only runs when the top cont is NOT a `TCont::Unit` (the `Next::Unit`
                            // no-op branch routes to `Reduce` precisely in that case), so a pending unit chain
                            // is never the delivery target here.
                            Some(TCont::Unit { .. }) => {
                                unreachable!(
                                    "Reduce never delivers into a pending unit-infix continuation"
                                )
                            }
                            Some(TCont::Arrow { start, arrow, left }) => {
                                let span = start.merge(self.prev_span());
                                let node = self.list(vec![arrow, left, value], span);
                                next = Next::Reduce { start, value: node };
                            }
                            Some(TCont::Label { start, label }) => {
                                let colon = self.name(":", start);
                                let span = start.merge(self.prev_span());
                                let node = self.list(vec![colon, label, value], span);
                                next = Next::Reduce { start, value: node };
                            }
                            Some(TCont::Paren {
                                start,
                                mut items,
                                head_made,
                                um,
                            }) => {
                                if !head_made && !self.at(Kind::Comma) {
                                    // A single parenthesized type is a transparent grouping — `(A)` is `A`.
                                    self.expect(Kind::RParen, "`)`");
                                    next = Next::Unit {
                                        left: value,
                                        min_prec: um,
                                        start,
                                        spine: 0,
                                    };
                                } else {
                                    if !head_made {
                                        // First element of a tuple: the `Tuple` head is created AFTER the
                                        // first element's subtree (matching the recursive struct-id order),
                                        // then `items` becomes `[Tuple-head, first]`.
                                        let head = self.name("Tuple", start);
                                        items.push(head);
                                    }
                                    items.push(value);
                                    if self.sep_continue(Kind::RParen) {
                                        stack.push(TCont::Paren {
                                            start,
                                            items,
                                            head_made: true,
                                            um,
                                        });
                                        next = Next::Read { um: 0 };
                                    } else {
                                        self.expect(Kind::RParen, "`)`");
                                        let span = start.merge(self.prev_span());
                                        let node = self.list(items, span);
                                        next = Next::Unit {
                                            left: node,
                                            min_prec: um,
                                            start,
                                            spine: 0,
                                        };
                                    }
                                }
                            }
                            Some(TCont::Brace {
                                start,
                                mut items,
                                um,
                            }) => {
                                // `value` is a completed `(: label ty)` field. Mirror the recursive
                                // `type_brace_record` loop: push the field, then `sep_continue` + re-check the
                                // `!} && !EOF` while-condition before reading the next field's label.
                                items.push(value);
                                if self.sep_continue(Kind::RBrace)
                                    && !self.at(Kind::RBrace)
                                    && !self.at_end()
                                {
                                    let (fstart, label) = self.read_type_record_field_label();
                                    stack.push(TCont::Brace { start, items, um });
                                    stack.push(TCont::Label {
                                        start: fstart,
                                        label,
                                    });
                                    next = Next::Read { um: 0 };
                                } else {
                                    self.expect(Kind::RBrace, "`}`");
                                    let span = start.merge(self.prev_span());
                                    let node = self.list(items, span);
                                    next = Next::Unit {
                                        left: node,
                                        min_prec: um,
                                        start,
                                        spine: 0,
                                    };
                                }
                            }
                            Some(TCont::Forall {
                                start,
                                head,
                                binder_list,
                            }) => {
                                // `value` is the forall body → `(forall (binders…) body)`. No unit-infix /
                                // arrow at the forall's own level (the body already consumed any arrow chain),
                                // matching the recursive `type_ref` early-return for `forall`.
                                let span = start.merge(self.prev_span());
                                let node = self.list(vec![head, binder_list, value], span);
                                next = Next::Reduce { start, value: node };
                            }
                            Some(TCont::App {
                                node_start,
                                head,
                                head_is_record,
                                mut args,
                                spine,
                                um,
                            }) => {
                                args.push(value);
                                if self.sep_continue(Kind::RParen) {
                                    // More arguments: descend into the next (a fresh type, possibly labeled).
                                    let label = self.type_arg_label();
                                    stack.push(TCont::App {
                                        node_start,
                                        head,
                                        head_is_record,
                                        args,
                                        spine,
                                        um,
                                    });
                                    if let Some((lstart, lbl)) = label {
                                        stack.push(TCont::Label {
                                            start: lstart,
                                            label: lbl,
                                        });
                                    }
                                    next = Next::Read { um: 0 };
                                } else {
                                    self.expect(Kind::RParen, "`)`");
                                    // RT1: a record TYPE takes only `field: T` ascriptions — flag the
                                    // obsolete head-application field spelling `Record(field(T))`.
                                    if head_is_record {
                                        for &arg in &args {
                                            if self.is_head_app_record_field(arg) {
                                                self.error(
                                                    "a record-type field is written `field: T`, not \
                                                     `field(T)` — use the colon form, e.g. \
                                                     `Record(x: Int64)`",
                                                );
                                            }
                                        }
                                    }
                                    let span = node_start.merge(self.prev_span());
                                    let mut items = Vec::with_capacity(args.len() + 1);
                                    items.push(head);
                                    items.extend(args);
                                    let node = self.list(items, span);
                                    let spine = spine + 1;
                                    if !self.depth_exceeded
                                        && self.depth + spine >= crate::sexpr::MAX_NESTING_DEPTH
                                    {
                                        self.error("expression nests too deeply to parse");
                                        self.depth_exceeded = true;
                                        next = Next::Unit {
                                            left: node,
                                            min_prec: um,
                                            start: node_start,
                                            spine: 0,
                                        };
                                    } else {
                                        // Continue the postfix loop on the new node (more `.member`/`(…)`).
                                        next = Next::Postfix {
                                            node_start,
                                            node,
                                            spine,
                                            um,
                                        };
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// A single TYPE OPERAND — the atom the `->` arrow and the unit-composition infix operators
    /// (`^`/`*`/`/`) compose over: a parenthesized/tuple type, a brace record type, or a `prefix` head
    /// extended by [`Self::type_postfix`] (member chain + `(…)` application). Factored out of
    /// [`Self::type_ref`] so [`Self::type_unit_infix`] can parse each operand the same way — WITHOUT
    /// re-consuming a `forall` (only legal at the head of a type) or an `->`.
    fn type_operand(&mut self) -> StructId {
        let start = self.cur_span();
        if self.at(Kind::LParen) {
            self.type_paren(start)
        } else if self.at(Kind::LBrace) {
            self.type_brace_record(start)
        } else {
            let head = self.prefix();
            self.type_postfix(head, start)
        }
    }

    /// Fold the unit-composition infix operators `^`/`*`/`/` in TYPE position into bare-glyph-headed
    /// nodes (`(^ base 2)`, `(/ m s)`), a Pratt climb sharing the value grammar's [`infix_prec`] so a
    /// derived-unit annotation round-trips through the ML print→parse cycle. `*`/`/` are the
    /// multiplicative tier (11) and left-associative; `^` is tier 7 (looser than `*`/`/`, matching the
    /// glyph's general-expression binding), so the printer parenthesizes `s ^ 2` under a `/` and this
    /// re-reads that parenthesized operand via [`Self::type_operand`] (→ [`Self::type_paren`] grouping).
    /// These operators are meaningless in a non-unit type, so folding them here adds no ambiguity: a
    /// type that previously reached one of them errored (`expected ,`/`)`), so nothing that parsed
    /// before changes. Bare-glyph heads (not `Unit.*`/`Unit.^`) match what the value side emits and what
    /// the printer round-trips (`has_canonicalizing_head` holds the `Unit.^` INPUT to idempotence-only).
    /// The `spine` depth guard mirrors the value infix loop (a long flat `m/s/s/…` chain deepens the
    /// arena on its left, which a recursive consumer would overflow — so bound it to a clean diagnostic).
    fn type_unit_infix(&mut self, mut left: StructId, min_prec: u8, start: Span) -> StructId {
        let mut spine: u32 = 0;
        loop {
            let op_name = match self.kind() {
                Kind::Caret => "^",
                Kind::Star => "*",
                Kind::Slash => "/",
                _ => break,
            };
            let prec = infix_prec(op_name).expect("unit infix op has a precedence");
            if prec < min_prec {
                break;
            }
            let op_span = self.cur_span();
            self.bump(); // operator
            let head = self.name(op_name, op_span);
            let rhs_start = self.cur_span();
            let right = self.type_operand();
            // Left-associative: the right operand binds one tighter, so a same-tier run (`a * b * c`)
            // groups left, and `^` (tier 7) captured on the right of `/` (tier 11) stays isolated.
            let right = self.type_unit_infix(right, prec + 1, rhs_start);
            let span = start.merge(self.prev_span());
            left = self.list(vec![head, left, right], span);
            spine += 1;
            if !self.depth_exceeded && self.depth + spine >= crate::sexpr::MAX_NESTING_DEPTH {
                self.error("expression nests too deeply to parse");
                self.depth_exceeded = true;
                break;
            }
        }
        left
    }

    /// The TYPE-position mirror of [`Self::postfix`]: a member/`.` chain plus `(…)` APPLICATION, but each
    /// application argument is parsed as a TYPE ([`Self::type_ref`]), not a value [`Self::arg_exprs`].
    /// A type-application argument may itself be any type — including a `forall` or an arrow — which the
    /// value `arg_exprs` path could NOT parse: `forall` is a contextual keyword recognized only in type
    /// position, so `Tuple(forall b. L)` routed through the value `expr` misread `forall` as a name and
    /// let the unit-suffix postfix eat the binder (`(Qty.of forall (Unit.of #"b"))` + `<error>`). Member
    /// access (`M.T` — a qualified type name) reuses the same `.`-key handling as the value postfix. The
    /// depth guard mirrors [`Self::postfix`] (a `Foo(Bar(Baz(…)))` chain deepens the tree per layer).
    fn type_postfix(&mut self, mut node: StructId, start: Span) -> StructId {
        let mut spine: u32 = 0;
        loop {
            match self.kind() {
                Kind::Dot if self.dot_is_member() => {
                    node = self.member_access(node, start);
                }
                Kind::LParen => {
                    // A `Record(…)` type-constructor takes ONLY named fields — each canonical `(: name T)`
                    // ascription (from the `name: T` label surface). RT1 (DESIGN-record-type-syntax OQ-A):
                    // reject the obsolete head-application field spelling `Record(field(T))` — where
                    // `field(T)` parsed as a positional application arg `(field T)` — and steer to the
                    // colon form `field: T`. A record type never takes a POSITIONAL type argument (unlike
                    // `List(a)`/`Tuple(A, B)`), so a non-ascription arg here is unambiguously a malformed
                    // field, not a legitimate application — no false-reject on generic type-apps.
                    let head_is_record = self.builder.as_name(node) == Some("Record");
                    let args = self.type_arg_exprs();
                    if head_is_record {
                        for &arg in &args {
                            if self.is_head_app_record_field(arg) {
                                self.error(
                                    "a record-type field is written `field: T`, not `field(T)` — \
                                     use the colon form, e.g. `Record(x: Int64)`",
                                );
                            }
                        }
                    }
                    let span = start.merge(self.prev_span());
                    let mut items = Vec::with_capacity(args.len() + 1);
                    items.push(node);
                    items.extend(args);
                    node = self.list(items, span);
                }
                _ => break,
            }
            spine += 1;
            if !self.depth_exceeded && self.depth + spine >= crate::sexpr::MAX_NESTING_DEPTH {
                self.error("expression nests too deeply to parse");
                self.depth_exceeded = true;
                return node;
            }
        }
        node
    }

    /// Parse `( arg, … )` in TYPE-application-argument position. Each argument is either:
    ///   - a LABELED field `name: T` → `(: name T)` — the shape the explicit record TYPE `Record(x: Int64,
    ///     …)` uses (the canonical `(Record (: x Int64) …)` the brace `{x: Int64}` sugar also builds), or
    ///   - a bare TYPE [`Self::type_ref`] — a positional type argument (`List(a)`, `Tuple(A, B)`), which
    ///     may itself be a `forall`/arrow/nested application.
    ///
    /// Unlike the value [`Self::arg_exprs`] (which parses each arg with the general `expr`), a bare arg
    /// here is a TYPE: `forall` is a contextual keyword recognized only in type position, so a value-`expr`
    /// arg would misread `Tuple(forall b. L)` as a name + unit-suffix. The labeled `name: T` form is kept
    /// so the explicit `Record(field: T)` application still parses (it produced `(: field T)` via the value
    /// path's infix `:`). A label is an `Ident`/backtick-name IMMEDIATELY followed by `:` — otherwise the
    /// arg is a plain type (so a bare `M.T` / `List(a)` positional arg is unaffected).
    fn type_arg_exprs(&mut self) -> Vec<StructId> {
        self.expect(Kind::LParen, "`(`");
        let mut args = Vec::new();
        if !self.at(Kind::RParen) {
            loop {
                args.push(self.type_arg());
                if !self.sep_continue(Kind::RParen) {
                    break;
                }
            }
        }
        self.expect(Kind::RParen, "`)`");
        args
    }

    /// `forall a b . TYPE`  ->  `(forall (a b) TYPE)`. The binder list is one-or-more lowercase names,
    /// terminated by `.`; TYPE is an ordinary [`Self::type_ref`] (so the arrow `forall a. a -> a` binds
    /// looser than `->` and reads as `forall a. (a -> a)`, the natural curried generic). The binder
    /// names are recorded as a nested `(binders…)` list, then TYPE — matching the canonical s-expr form.
    /// A missing binder or missing `.` records an error and recovers by treating what follows as the
    /// type (so a malformed `forall` never panics — the crate's never-panic contract).
    fn forall_type(&mut self, start: Span) -> StructId {
        let (head, binder_list) = self.read_forall_preamble(start);
        let body = self.type_ref();
        let span = start.merge(self.prev_span());
        self.list(vec![head, binder_list, body], span)
    }

    /// Read a `forall <name>+ .` preamble — the `forall` keyword, the one-or-more binder names, and the `.`
    /// terminator — returning `(forall-head-node, binder-list-node)`. A missing binder / missing `.` records
    /// an error and recovers (never-panic). Shared by the recursive [`Self::forall_type`] and the iterative
    /// [`Self::type_ref_iter`] (which then reads the body on the worklist via [`TCont::Forall`]) so the
    /// head → binders → binder-list node-creation order can't drift between the two readers.
    fn read_forall_preamble(&mut self, start: Span) -> (StructId, StructId) {
        self.expect_keyword(Keyword::Forall, "`forall`");
        let head = self.name("forall", start);
        // Binder names: one-or-more bare identifiers before the `.`. `type`/other keywords are not names.
        let binders_start = self.cur_span();
        let mut binders = Vec::new();
        while self.at(Kind::Ident) && keyword(self.cur_text()).is_none() {
            let name_span = self.cur_span();
            let text = self.cur_text().to_string();
            self.bump();
            binders.push(self.name(text, name_span));
        }
        if binders.is_empty() {
            self.error("a `forall` needs at least one type-variable name, e.g. `forall a. …`");
        }
        let binders_span = binders_start.merge(self.prev_span());
        let binder_list = self.list(binders, binders_span);
        // The `.` terminator between the binders and the type.
        if self.at(Kind::Dot) {
            self.bump();
        } else {
            self.error("expected `.` after the `forall` binders, e.g. `forall a. T`");
        }
        (head, binder_list)
    }

    /// A leading `forall a b .` clause in a DEF SIGNATURE (`def forall a b. f(x: a) = …`) — the P1
    /// ergonomic spelling for a generic def. Consumes `forall <name>+ .` and returns one synthesized
    /// `(: name Type)` parameter node PER binder (source order), ready to PREPEND to the def's parameter
    /// list. It is pure sugar: `def forall a. f(x: a) = x` produces the SAME signature as writing the
    /// leading `(: a Type)` param by hand (or as the param-annotation `def f(x: forall a. a) = x`
    /// desugar) — so infer sees the identical arena, no ∀ engine. Returns `None` (consuming nothing) when
    /// not at a `forall`. A missing binder / missing `.` records an error and recovers (never-panic).
    fn forall_sig_type_params(&mut self) -> Option<Vec<StructId>> {
        if !self.at_keyword(Keyword::Forall) {
            return None;
        }
        let start = self.cur_span();
        self.expect_keyword(Keyword::Forall, "`forall`");
        // Collect the binder names, then synthesize a `(: name Type)` param for each.
        let mut names: Vec<(String, Span)> = Vec::new();
        while self.at(Kind::Ident) && keyword(self.cur_text()).is_none() {
            names.push((self.cur_text().to_string(), self.cur_span()));
            self.bump();
        }
        if names.is_empty() {
            self.error(
                "a `forall` needs at least one type-variable name, e.g. `def forall a. f(…) = …`",
            );
        }
        if self.at(Kind::Dot) {
            self.bump();
        } else {
            self.error("expected `.` after the `forall` binders, e.g. `def forall a. f(…) = …`");
        }
        let span = start.merge(self.prev_span());
        let params = names
            .into_iter()
            .map(|(text, name_span)| {
                let colon = self.name(":", span);
                let nm = self.name(text, name_span);
                let type_kw = self.name("Type", span);
                self.list(vec![colon, nm, type_kw], span)
            })
            .collect();
        Some(params)
    }

    /// `( T, … )` in TYPE position → the tuple TYPE node `(Tuple T …)` (head is the `Tuple` type name,
    /// the same node `Tuple(A, B)` builds — one canonical type spelling). A single `( T )` is a grouping
    /// (returns `T`); `()` is the `unit` type. Each element is parsed by `type_ref`, so a nested tuple
    /// type (`(A, (B, C))`) and a function-type element (`((A) -> B, C)`) work. This makes the natural
    /// `def f(p: (Int64, Int64))` an accepted pair type instead of the CDZ0203 the value-ctor lowering
    /// produced — tuple values/patterns and tuple TYPES now share the `(…)` spelling (as lists do).
    fn type_paren(&mut self, start: Span) -> StructId {
        self.expect(Kind::LParen, "`(`");
        if self.at(Kind::RParen) {
            self.bump();
            let span = start.merge(self.prev_span());
            return self.name("unit", span);
        }
        let first = self.type_ref();
        if !self.at(Kind::Comma) {
            // A single parenthesized type is a transparent grouping — `(A)` is `A`.
            self.expect(Kind::RParen, "`)`");
            return first;
        }
        let head = self.name("Tuple", start);
        let mut items = vec![head, first];
        while self.sep_continue(Kind::RParen) {
            items.push(self.type_ref());
        }
        self.expect(Kind::RParen, "`)`");
        let span = start.merge(self.prev_span());
        self.list(items, span)
    }

    /// `{ field: T, … }` in TYPE position → the record TYPE node `(Record (: field T) …)` — the SAME
    /// canonical node the explicit `Record(field: T, …)` application builds, so the brace form is pure
    /// surface sugar for it (one type spelling, both surfaces agree). Handled directly in `type_ref`
    /// because the shared `prefix`/`paren` path would read `{ … }` as a record VALUE literal (whose
    /// fields are `name = value`), so a `field: T` there errors "expected `,`" at the `:` — the reported
    /// gap. A trailing comma is allowed. `{}` is the empty record type `(Record)`. Each field type is
    /// parsed by `type_ref`, so a field may itself be a function/tuple/nested-record type
    /// (`{f: Int64 -> Bool, p: {x: Int64}}`).
    fn type_brace_record(&mut self, start: Span) -> StructId {
        self.expect(Kind::LBrace, "`{`");
        let head = self.name("Record", start);
        let mut items = vec![head];
        while !self.at(Kind::RBrace) && !self.at_end() {
            let (field_start, label) = self.read_type_record_field_label();
            let ty = self.type_ref();
            let colon = self.name(":", field_start);
            let field_span = field_start.merge(self.prev_span());
            items.push(self.list(vec![colon, label, ty], field_span));
            if !self.sep_continue(Kind::RBrace) {
                break;
            }
        }
        self.expect(Kind::RBrace, "`}`");
        let span = start.merge(self.prev_span());
        self.list(items, span)
    }

    /// Read a brace-record TYPE field's REQUIRED label + its `:`, returning `(field-start-span, label-node)`.
    /// A field label is a bare name (or backtick name for a symbolic/reserved label); a missing name records
    /// an error and recovers with an `<error>` node (never-panic). Shared by the recursive
    /// [`Self::type_brace_record`] and the iterative [`Self::type_ref_iter`] so the label node + its `:`
    /// consumption can't drift between the two readers (the field then becomes `(: label ty)` in both, via a
    /// [`TCont::Label`] continuation on the iterative side).
    fn read_type_record_field_label(&mut self) -> (Span, StructId) {
        let field_start = self.cur_span();
        let label = match self.kind() {
            Kind::Ident => {
                let t = self.bump().unwrap();
                self.name(self.text(t), field_start)
            }
            Kind::BacktickName => {
                let t = self.bump().unwrap();
                self.name(literal::unescape_backtick_name(self.text(t)), field_start)
            }
            _ => {
                self.error("expected a record field name");
                self.error_node(field_start)
            }
        };
        self.expect(Kind::Colon, "`:` after a record field name");
        (field_start, label)
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
mod tests;
